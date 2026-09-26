//! The Channel login: the rules legacy applies to `HEADER_CG_LOGIN2`, the registry of logins
//! held by Channel descriptors, and the character list the select screen receives.
//!
//! Legacy spreads the login over two processes. `CInputLogin::LoginByKey`
//! (`G/input_login.cpp:153-219`) refuses a full or closing server under the handshake key, then
//! installs the client's TEA key and asks the DB server (`QUERY_LOGIN_BY_KEY`,
//! `D/ClientManagerLogin.cpp:82-149`), which judges the login key and reads the characters.
//! `CInputDB::LoginSuccess` (`G/input_db.cpp:107-200`) then fills in each character's server
//! location (`GetServerLocation`, `G/input_db.cpp:54-103`) and sends `GC_EMPIRE` and
//! `GC_LOGIN_SUCCESS` before the select phase.
//!
//! The DB server checks `FindLogonAccount` (`ALREADY`) before it compares the login and the
//! client key, and the game then kicks the descriptor holding the login the client *typed*
//! (`CInputDB::LoginAlready`). A client holding any live login key whose account is logged on
//! could therefore disconnect any other logged-on account by name. That is a Defect; the Rewrite
//! compares the login and the client key first, so only the key's own login can be kicked.

use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use db::credentials::Login;
use db::players::{Lobby, LobbyPlayer, PLAYER_SLOTS};
use gamedata::map_atlas::MapAtlas;
use protocol::gc::{GcLoginPlayer, GcLoginSuccess};
use tokio::sync::Notify;

use crate::auth_login::LoginGrant;

/// Legacy `g_start_position`: where a character whose map no Channel hosts is placed, by empire
/// (`G/start_position.cpp:25-31`). Empire 0 has no start position.
pub const EMPIRE_START: [(i32, i32); 4] = [
    (0, 0),
    (469_300, 964_200),
    (55_700, 157_900),
    (969_600, 278_400),
];

/// A reason the Channel login refuses `LOGIN2`. Each is sent as `HEADER_GC_LOGIN_FAILURE`, and
/// the descriptor stays open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelRefusal {
    /// The login key is unknown, or the login or client key does not match its grant.
    NoId,
    /// Another Channel descriptor holds the login; it is kicked.
    Already,
    /// The server takes no more clients.
    Shutdown,
    /// The user limit is reached.
    Full,
}

impl ChannelRefusal {
    /// The `szStatus` bytes legacy sends for this refusal.
    #[must_use]
    pub const fn status(self) -> &'static [u8] {
        match self {
            Self::NoId => b"NOID",
            Self::Already => b"ALREADY",
            Self::Shutdown => b"SHUTDOWN",
            Self::Full => b"FULL",
        }
    }
}

/// The checks `CInputLogin::LoginByKey` makes before it asks for the account, in legacy order:
/// `g_bNoMoreClient`, then the user limit (`G/input_login.cpp:174-202`). Both answers go out
/// under the handshake key, because the client key is installed only after them.
///
/// `user_limit` at or below zero is unlimited; `online` counts every character in game.
///
/// # Errors
///
/// Returns the refusal legacy sends first.
pub fn admit(no_more_clients: bool, user_limit: i32, online: u32) -> Result<(), ChannelRefusal> {
    if no_more_clients {
        return Err(ChannelRefusal::Shutdown);
    }
    if let Ok(limit) = u32::try_from(user_limit) {
        if limit > 0 && limit <= online {
            return Err(ChannelRefusal::Full);
        }
    }
    Ok(())
}

/// The login-key checks of `QUERY_LOGIN_BY_KEY` except `ALREADY`: the key is granted, the login
/// matches without regard to case, and the client key is the one sent with `LOGIN3`
/// (`D/ClientManagerLogin.cpp:85-123`).
///
/// `login` is the result of `login_from_field`; a malformed login matches nothing. [`Login`]
/// is already lower case, so equality is legacy's `strcasecmp`.
///
/// # Errors
///
/// Returns [`ChannelRefusal::NoId`].
pub fn judge_key(
    grant: Option<LoginGrant>,
    login: Option<&Login>,
    client_key: [u32; 4],
) -> Result<LoginGrant, ChannelRefusal> {
    let grant = grant.ok_or(ChannelRefusal::NoId)?;
    if login != Some(&grant.login) {
        return Err(ChannelRefusal::NoId);
    }
    if grant.client_key != client_key {
        return Err(ChannelRefusal::NoId);
    }
    Ok(grant)
}

#[derive(Debug, Default)]
struct Logons {
    held: HashMap<Login, (u64, Arc<Notify>)>,
    next_claim: u64,
}

/// The logins held by Channel descriptors (legacy DB `m_map_kLogonAccount`).
///
/// A login stays held until its descriptor is gone, across the select screen and in game, as the
/// DB server keeps it until the game Core reports the logout.
#[derive(Debug, Default)]
pub struct LogonRegistry {
    inner: Mutex<Logons>,
}

impl LogonRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> MutexGuard<'_, Logons> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Hold `login` for one descriptor until the claim is dropped. When another descriptor holds
    /// it, that descriptor is told to close (legacy `CInputDB::LoginAlready`,
    /// `G/input_db.cpp:1219-1240`) and the login is refused with `ALREADY`; it stays held until
    /// the kicked descriptor is gone.
    ///
    /// # Errors
    ///
    /// Returns [`ChannelRefusal::Already`].
    pub fn claim(self: &Arc<Self>, login: &Login) -> Result<LogonClaim, ChannelRefusal> {
        let mut logons = self.lock();
        if let Some((_, kick)) = logons.held.get(login) {
            kick.notify_one();
            return Err(ChannelRefusal::Already);
        }
        logons.next_claim += 1;
        let id = logons.next_claim;
        let kick = Arc::new(Notify::new());
        logons.held.insert(login.clone(), (id, Arc::clone(&kick)));
        Ok(LogonClaim {
            registry: Arc::clone(self),
            login: login.clone(),
            id,
            kick,
        })
    }

    /// Whether a Channel descriptor holds `login`.
    #[must_use]
    pub fn is_held(&self, login: &Login) -> bool {
        self.lock().held.contains_key(login)
    }
}

/// One descriptor's hold on a login. Dropping it releases the login.
#[derive(Debug)]
pub struct LogonClaim {
    registry: Arc<LogonRegistry>,
    login: Login,
    id: u64,
    kick: Arc<Notify>,
}

impl LogonClaim {
    /// The held login.
    #[must_use]
    pub fn login(&self) -> &Login {
        &self.login
    }

    /// Resolves once another descriptor has tried to log in with this login. A kick that
    /// arrives while nobody waits is kept for the next call.
    pub async fn kicked(&self) {
        self.kick.notified().await;
    }
}

impl Drop for LogonClaim {
    fn drop(&mut self) {
        let mut logons = self.registry.lock();
        if logons.held.get(&self.login).map(|(id, _)| *id) == Some(self.id) {
            logons.held.remove(&self.login);
        }
    }
}

/// Which maps a Channel and the Shared Channel host, and where the client reaches them: the
/// Rewrite's `CMapLocation` (`G/map_location.cpp`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapLocations {
    /// `lAddr`: the public address as the client reads four bytes.
    addr: i32,
    /// The port the client used to reach this Channel, and the maps the Channel hosts.
    channel: (u16, HashSet<u32>),
    /// The Shared Channel's first port and its maps, when one is configured.
    shared: Option<(u16, HashSet<u32>)>,
}

impl MapLocations {
    /// Locations for a client connected to `channel_port` of a Channel hosting `channel_maps`.
    #[must_use]
    pub fn new(
        public_ip: Ipv4Addr,
        channel_port: u16,
        channel_maps: impl IntoIterator<Item = u32>,
        shared: Option<(u16, Vec<u32>)>,
    ) -> Self {
        Self {
            // `inet_addr` gives the address in network order; read as a little-endian `long`
            // it goes out as the four octets in order.
            addr: i32::from_le_bytes(public_ip.octets()),
            channel: (channel_port, channel_maps.into_iter().collect()),
            shared: shared.map(|(port, maps)| (port, maps.into_iter().collect())),
        }
    }

    /// `CMapLocation::Get(index, ...)`: the address and port of the Channel hosting a map,
    /// preferring the client's own Channel.
    #[must_use]
    pub fn get(&self, index: i32) -> Option<(i32, u16)> {
        let index = u32::try_from(index).ok().filter(|&index| index != 0)?;
        if self.channel.1.contains(&index) {
            return Some((self.addr, self.channel.0));
        }
        match &self.shared {
            Some((port, maps)) if maps.contains(&index) => Some((self.addr, *port)),
            _ => None,
        }
    }
}

/// One character's position and server location after `GetServerLocation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Located {
    /// The position, moved to the empire start when its map is hosted nowhere.
    pub x: i32,
    /// See `x`.
    pub y: i32,
    /// `lAddr`, zero when no Channel hosts even the empire start.
    pub addr: i32,
    /// `wPort`, zero with `addr`.
    pub port: u16,
}

/// `GetServerLocation` for one character (`G/input_db.cpp:54-103`): the map at its position,
/// else the empire start, else no location.
#[must_use]
pub fn locate(atlas: &MapAtlas, locations: &MapLocations, empire: u8, x: i32, y: i32) -> Located {
    let find = |x, y| atlas.index_at(x, y).and_then(|index| locations.get(index));
    if let Some((addr, port)) = find(x, y) {
        return Located { x, y, addr, port };
    }
    let (x, y) = EMPIRE_START
        .get(usize::from(empire))
        .copied()
        .unwrap_or_default();
    let (addr, port) = find(x, y).unwrap_or_default();
    Located { x, y, addr, port }
}

/// `CMapLocation::Get(x, y, ...)` alone, as a new character is placed
/// (`CInputDB::PlayerCreateSuccess`, `G/input_db.cpp:241-253`): no empire-start fallback, and no
/// location when no Channel hosts the map.
#[must_use]
pub fn locate_here(atlas: &MapAtlas, locations: &MapLocations, x: i32, y: i32) -> Located {
    let (addr, port) = atlas
        .index_at(x, y)
        .and_then(|index| locations.get(index))
        .unwrap_or_default();
    Located { x, y, addr, port }
}

/// The `GC_EMPIRE` value: the account's empire once it has a character, else 0
/// (`FIX_SELECT_EMPIRE_PHASE`, `G/input_db.cpp:163-177`).
#[must_use]
pub fn empire_shown(lobby: &Lobby) -> u8 {
    if lobby.players.is_empty() {
        0
    } else {
        lobby.empire
    }
}

/// A character's `TSimplePlayer`, located.
pub(crate) fn player_record(player: &LobbyPlayer, located: Located) -> GcLoginPlayer {
    let mut name = [0; protocol::simple_player::CHARACTER_NAME_BYTES];
    let bytes = player.name.as_bytes();
    let kept = bytes.len().min(name.len() - 1);
    name[..kept].copy_from_slice(&bytes[..kept]);
    GcLoginPlayer {
        id: player.id,
        name,
        job: player.job,
        level: player.level,
        play_minutes: player.play_minutes,
        st: player.st,
        ht: player.ht,
        dx: player.dx,
        iq: player.iq,
        main_part: player.main_part,
        change_name: u8::from(player.change_name),
        hair_part: player.hair_part,
        sash_part: player.sash_part,
        dummy: [0; 4],
        x: located.x,
        y: located.y,
        addr: located.addr,
        port: located.port,
        skill_group: player.skill_group,
        conqueror_level: player.conqueror_level,
        sungma_str: player.sungma_str,
        sungma_hp: player.sungma_hp,
        sungma_move: player.sungma_move,
        sungma_immune: player.sungma_immune,
    }
}

/// The `GC_LOGIN_SUCCESS` record (`DESC::SendLoginSuccessPacket`, `G/desc.cpp:874`): one summary per
/// slot, located, with an empty slot left zero. Guilds are not ported yet, so every guild ID and
/// name is zero.
#[must_use]
pub fn login_success(
    lobby: &Lobby,
    atlas: &MapAtlas,
    locations: &MapLocations,
    handle: u32,
    random_key: u32,
) -> GcLoginSuccess {
    let mut players = [GcLoginPlayer::default(); PLAYER_SLOTS];
    for (slot, record) in (0u8..).zip(players.iter_mut()) {
        if let Some(player) = lobby.in_slot(slot) {
            let located = locate(atlas, locations, lobby.empire, player.x, player.y);
            *record = player_record(player, located);
        }
    }
    GcLoginSuccess::new(
        players,
        [0; PLAYER_SLOTS],
        [[0; 13]; PLAYER_SLOTS],
        handle,
        random_key,
    )
}

/// A random `random_key` (legacy `DESC_MANAGER::MakeRandomKey`, a `thecore_random()`).
#[must_use]
pub fn random_key() -> u32 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(NEXT.fetch_add(1, Ordering::Relaxed));
    let [a, b, c, d, ..] = hasher.finish().to_le_bytes();
    u32::from_le_bytes([a, b, c, d])
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::accounts::AccountId;
    use gamedata::map_atlas::MapRegion;

    fn login(text: &str) -> Login {
        Login::new(text).unwrap()
    }

    fn grant() -> LoginGrant {
        LoginGrant {
            account: AccountId::new(7),
            login: login("hero"),
            client_key: [0x0102_0304, 0x0506_0708, 0x090a_0b0c, 0x0d0e_0f10],
            language: 1,
        }
    }

    fn region(index: i32, sx: i32, sy: i32) -> MapRegion {
        MapRegion {
            index,
            name: b"test".to_vec(),
            sx,
            sy,
            ex: sx + 25_600,
            ey: sy + 25_600,
            spawn: (sx, sy),
            empire_spawns: None,
        }
    }

    #[test]
    fn admission_refuses_shutdown_before_full() {
        assert_eq!(admit(true, 1, 5), Err(ChannelRefusal::Shutdown));
        assert_eq!(admit(false, 5, 5), Err(ChannelRefusal::Full));
        assert_eq!(admit(false, 5, 6), Err(ChannelRefusal::Full));
        assert_eq!(admit(false, 5, 4), Ok(()));
        assert_eq!(admit(false, 0, u32::MAX), Ok(()));
        assert_eq!(admit(false, -1, u32::MAX), Ok(()));
    }

    #[test]
    fn refusal_statuses_are_the_legacy_bytes() {
        assert_eq!(ChannelRefusal::NoId.status(), b"NOID");
        assert_eq!(ChannelRefusal::Already.status(), b"ALREADY");
        assert_eq!(ChannelRefusal::Shutdown.status(), b"SHUTDOWN");
        assert_eq!(ChannelRefusal::Full.status(), b"FULL");
    }

    #[test]
    fn the_key_must_be_granted_to_the_same_login_and_client_key() {
        let key = grant().client_key;
        assert_eq!(
            judge_key(Some(grant()), Some(&login("hero")), key),
            Ok(grant())
        );
        assert_eq!(
            judge_key(None, Some(&login("hero")), key),
            Err(ChannelRefusal::NoId)
        );
        assert_eq!(
            judge_key(Some(grant()), Some(&login("other")), key),
            Err(ChannelRefusal::NoId)
        );
        assert_eq!(
            judge_key(Some(grant()), None, key),
            Err(ChannelRefusal::NoId)
        );
        for word in 0..4 {
            let mut wrong = key;
            wrong[word] ^= 1;
            assert_eq!(
                judge_key(Some(grant()), Some(&login("hero")), wrong),
                Err(ChannelRefusal::NoId)
            );
        }
    }

    #[tokio::test]
    async fn a_second_logon_is_refused_and_kicks_the_holder_until_it_is_gone() {
        let registry = LogonRegistry::new();
        let first = registry.claim(&login("hero")).unwrap();
        assert_eq!(first.login(), &login("hero"));
        assert_eq!(
            registry.claim(&login("hero")).unwrap_err(),
            ChannelRefusal::Already
        );
        // The kick was stored while nobody waited.
        tokio::time::timeout(std::time::Duration::from_secs(1), first.kicked())
            .await
            .unwrap();
        assert!(registry.claim(&login("other")).is_ok());
        assert!(registry.is_held(&login("hero")));
        drop(first);
        assert!(!registry.is_held(&login("hero")));
        let second = registry.claim(&login("hero")).unwrap();
        // An earlier claim's drop never releases a later holder.
        assert!(registry.is_held(second.login()));
    }

    #[test]
    fn a_stale_claim_does_not_release_a_newer_holder() {
        let registry = LogonRegistry::new();
        let first = registry.claim(&login("hero")).unwrap();
        let first_id = first.id;
        drop(first);
        let second = registry.claim(&login("hero")).unwrap();
        assert_ne!(first_id, second.id);
        let stale = LogonClaim {
            registry: Arc::clone(&registry),
            login: login("hero"),
            id: first_id,
            kick: Arc::new(Notify::new()),
        };
        drop(stale);
        assert!(registry.is_held(&login("hero")));
    }

    #[test]
    fn a_map_is_found_on_the_channel_first_then_the_shared_channel() {
        let locations = MapLocations::new(
            Ipv4Addr::new(10, 1, 2, 3),
            30003,
            [1, 3],
            Some((30019, vec![3, 113])),
        );
        let addr = i32::from_le_bytes([10, 1, 2, 3]);
        assert_eq!(locations.get(1), Some((addr, 30003)));
        assert_eq!(locations.get(3), Some((addr, 30003)));
        assert_eq!(locations.get(113), Some((addr, 30019)));
        assert_eq!(locations.get(21), None);
        assert_eq!(locations.get(0), None);
        assert_eq!(locations.get(-1), None);
        let alone = MapLocations::new(Ipv4Addr::LOCALHOST, 30003, [1], None);
        assert_eq!(alone.get(113), None);
        // `CMapLocation::Get` refuses index 0 before its lookup, even if a map list held it.
        let zero = MapLocations::new(Ipv4Addr::LOCALHOST, 30003, [0, 1], Some((30019, vec![0])));
        assert_eq!(zero.get(0), None);
        assert_eq!(
            zero.get(1),
            Some((i32::from_le_bytes([127, 0, 0, 1]), 30003))
        );
    }

    #[test]
    fn a_character_on_an_unhosted_map_moves_to_its_empire_start() {
        let atlas = MapAtlas::from_regions(vec![
            region(1, 460_800, 947_200),
            region(21, 51_200, 153_600),
            region(113, 0, 0),
        ]);
        let locations = MapLocations::new(Ipv4Addr::LOCALHOST, 30003, [1], None);
        let addr = i32::from_le_bytes([127, 0, 0, 1]);
        // Hosted: kept.
        assert_eq!(
            locate(&atlas, &locations, 2, 470_000, 950_000),
            Located {
                x: 470_000,
                y: 950_000,
                addr,
                port: 30003
            }
        );
        // Unhosted map: empire 1 start, which is hosted.
        assert_eq!(
            locate(&atlas, &locations, 1, 100, 100),
            Located {
                x: 469_300,
                y: 964_200,
                addr,
                port: 30003
            }
        );
        // No map at all, and the empire 2 start is not hosted: moved, with no location.
        assert_eq!(
            locate(&atlas, &locations, 2, -5, -5),
            Located {
                x: 55_700,
                y: 157_900,
                addr: 0,
                port: 0
            }
        );
        // Empire 0 and an impossible empire have no start.
        assert_eq!(
            locate(&atlas, &locations, 0, 100, 100),
            Located {
                x: 0,
                y: 0,
                addr: 0,
                port: 0
            }
        );
        assert_eq!(
            locate(&atlas, &locations, 9, 100, 100),
            Located {
                x: 0,
                y: 0,
                addr: 0,
                port: 0
            }
        );
    }

    fn lobby_player(slot: u8, name: &str) -> LobbyPlayer {
        LobbyPlayer {
            slot,
            id: 0x0102_0304 + u32::from(slot),
            name: name.to_owned(),
            job: 3,
            level: 0x42,
            play_minutes: 0x0a0b_0c0d,
            st: 11,
            ht: 12,
            dx: 13,
            iq: 14,
            conqueror_level: 15,
            sungma_str: 16,
            sungma_hp: 17,
            sungma_move: 18,
            sungma_immune: 19,
            main_part: 0x1122,
            hair_part: 0x3344,
            sash_part: 0x5566,
            x: 470_000,
            y: 950_000,
            skill_group: 2,
            change_name: true,
        }
    }

    #[test]
    fn the_character_list_fills_each_slot_and_leaves_empty_ones_zero() {
        let atlas = MapAtlas::from_regions(vec![region(1, 460_800, 947_200)]);
        let locations = MapLocations::new(Ipv4Addr::new(10, 1, 2, 3), 30005, [1], None);
        let lobby = Lobby {
            empire: 1,
            players: vec![
                lobby_player(0, "Alpha"),
                lobby_player(2, "Gamma"),
                lobby_player(3, "Abcdefghijklmnopqrstuvwx"),
            ],
        };
        let record = login_success(&lobby, &atlas, &locations, 0x1234_5678, 0x9abc_def0);
        // A 24-byte Name fills `CHARACTER_NAME_MAX_LEN` and keeps its terminating NUL.
        assert_eq!(&record.players[3].name, b"Abcdefghijklmnopqrstuvwx\0");
        assert_eq!(record.players[1], GcLoginPlayer::default());
        let first = record.players[0];
        assert_eq!(&first.name[..6], b"Alpha\0");
        assert_eq!(first.id, 0x0102_0304);
        assert_eq!(record.players[2].id, 0x0102_0306);
        assert_eq!(first.change_name, 1);
        assert_eq!(first.addr, i32::from_le_bytes([10, 1, 2, 3]));
        assert_eq!(first.port, 30005);
        assert_eq!((first.x, first.y), (470_000, 950_000));
        assert_eq!(
            (first.main_part, first.hair_part, first.sash_part),
            (0x1122, 0x3344, 0x5566)
        );
        assert_eq!(
            (first.st, first.ht, first.dx, first.iq, first.skill_group),
            (11, 12, 13, 14, 2)
        );
        assert_eq!(
            (
                first.conqueror_level,
                first.sungma_str,
                first.sungma_hp,
                first.sungma_move,
                first.sungma_immune
            ),
            (15, 16, 17, 18, 19)
        );
        assert_eq!(
            (first.job, first.level, first.play_minutes),
            (3, 0x42, 0x0a0b_0c0d)
        );
        let bytes = record.encode();
        assert_eq!(bytes.len(), 357);
        assert_eq!(bytes[0], 32);
        assert_eq!(
            &bytes[349..],
            &[0x78, 0x56, 0x34, 0x12, 0xf0, 0xde, 0xbc, 0x9a]
        );
    }

    #[test]
    fn the_empire_is_shown_only_once_the_account_has_a_character() {
        let mut lobby = Lobby {
            empire: 2,
            players: Vec::new(),
        };
        assert_eq!(empire_shown(&lobby), 0);
        lobby.players.push(lobby_player(0, "Alpha"));
        assert_eq!(empire_shown(&lobby), 2);
    }
}
