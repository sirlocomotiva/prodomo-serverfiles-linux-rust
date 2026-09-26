//! The select screen: choosing an empire and creating, deleting, and renaming characters.
//!
//! `CInputLogin::Analyze` serves these four records in the login, select, and loading phases
//! (`G/input_login.cpp:1185-1244`). Each game-side handler checks the descriptor's account table
//! and asks the DB server, whose answer the game applies to the table and relays
//! (`G/input_db.cpp`). Here the store answers directly and [`SelectAccount`] is the account
//! table: the account a Channel login accepted and the characters its select screen shows.
//!
//! The rules below are pure. What each handler sends is decided here; the descriptor only
//! sends it and asks the store.
//!
//! Where the Rewrite differs from legacy:
//!
//! - A Name must end with a NUL inside its 25-byte field. Legacy reads it as a C string and runs
//!   past the field when there is none (a Defect).
//! - A creation refused for its Name or shape is answered with the 2-byte
//!   `HEADER_GC_CHARACTER_CREATE_FAILURE` record, type 0. Legacy sends a zeroed 10-byte
//!   `TPacketGCLoginFailure` under that header (`G/input_login.cpp:463-481`), 8 bytes the
//!   client's 2-byte record does not have (a Defect).
//! - The race is checked as the whole 16-bit `job` word. Legacy truncates it to a byte first
//!   (`NewPlayerTable2(..., pinfo->job, ...)` takes a `BYTE`), so job 256 created a warrior (a
//!   Divergence).
//! - A character is created only for an account with an empire. Legacy places a character of
//!   an account without one near (0, 0) (a Divergence).
//! - Empire 0 closes the descriptor like an empire past 3. Legacy stores it and moves the
//!   account's characters to (0, 0) (a Divergence).
//! - A rename naming a slot past 3 closes the descriptor at once; legacy closes it 5 seconds
//!   later (`DelayedDisconnect(5)`, a Divergence).

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use db::accounts::{AccountId, Name};
use db::credentials::Login;
use db::players::{Lobby, LobbyPlayer, NewPlayer, PLAYER_SLOTS};
use gamedata::banword::{holds_banword, BanwordRecord};
use gamedata::map_atlas::MapAtlas;
use gamedata::mob_names::MobNames;
use protocol::cg_account::CgPlayerCreate;
use protocol::cg_name::CgChangeName;
use protocol::gc::{
    GcCreateFailure, GcPlayerCreateSuccess, GcPlayerDeleteSuccess, GcPlayerDeleteWrongSocialId,
};
use protocol::gc_fields::{GcNamed, GC_NAME_FIELD_SIZE, HEADER_GC_CHANGE_NAME};

use crate::channel_login::{locate_here, player_record, MapLocations, EMPIRE_START};

/// Legacy `g_create_position`: where a new character is placed, by empire
/// (`G/start_position.cpp:43-49`). Empire 0 has none.
pub const CREATE_START: [(i32, i32); 4] = [
    (0, 0),
    (459_800, 953_900),
    (52_070, 166_600),
    (957_300, 255_200),
];

/// A new character lands up to this far from the create start on each axis
/// (`number(-300, 300)`, inclusive).
pub const CREATE_SPREAD: i32 = 300;

/// After a character is created, the account creates no other for this long
/// (`s_createTimeByAccountID`, `D/ClientManagerPlayer.cpp:1067-1078`).
pub const CREATE_COOLDOWN: Duration = Duration::from_secs(30);

/// `HEADER_DG_PLAYER_CREATE_FAILED` and every game-side refusal: the create failure type 0.
pub const CREATE_REFUSED: u8 = 0;

/// `HEADER_DG_PLAYER_CREATE_ALREADY`, and a Name equal to the login: the create failure type 1.
pub const CREATE_TAKEN: u8 = 1;

/// `MAIN_RACE_MAX_NUM`: races 0 to 7.
const RACES: u16 = 8;

/// The empires a client may choose (`EMPIRE_MAX_NUM` is 4, and 0 is no empire).
const EMPIRES: std::ops::RangeInclusive<u8> = 1..=3;

/// The `JobInitialPoints` a character starts with (`G/constants.cpp:17-22`), by job.
struct InitialPoints {
    st: u8,
    ht: u8,
    dx: u8,
    iq: u8,
    max_hp: i32,
}

const INITIAL_POINTS: [InitialPoints; 4] = [
    // Warrior.
    InitialPoints {
        st: 6,
        ht: 4,
        dx: 3,
        iq: 3,
        max_hp: 600,
    },
    // Assassin.
    InitialPoints {
        st: 4,
        ht: 3,
        dx: 6,
        iq: 3,
        max_hp: 650,
    },
    // Sura.
    InitialPoints {
        st: 5,
        ht: 3,
        dx: 3,
        iq: 5,
        max_hp: 650,
    },
    // Shaman.
    InitialPoints {
        st: 3,
        ht: 4,
        dx: 3,
        iq: 6,
        max_hp: 700,
    },
];

/// `hp_per_ht`, `max_sp`, `sp_per_iq`, and `max_stamina`, the same for every job.
const HP_PER_HT: i32 = 40;
const MAX_SP: i32 = 200;
const SP_PER_IQ: i32 = 20;
const MAX_STAMINA: i32 = 800;

/// The account a Channel descriptor logged in, and its characters: legacy `TAccountTable`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectAccount {
    /// The account.
    pub id: AccountId,
    /// Its login, as stored.
    pub login: Login,
    /// Its empire and characters, kept up to date by the select-screen handlers.
    pub lobby: Lobby,
}

/// The Name rules of the `europe` Locale: legacy `check_name_alphabet`
/// (`G/locale_service.cpp:317-337`) and `check_name_independent` (`:86-99`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameRules {
    banwords: Vec<BanwordRecord>,
    mobs: MobNames,
}

impl NameRules {
    /// Rules refusing the banned words and the mob names.
    #[must_use]
    pub const fn new(banwords: Vec<BanwordRecord>, mobs: MobNames) -> Self {
        Self { banwords, mobs }
    }

    /// The Name in a client's 25-byte field, when the rules accept it: the bytes before the
    /// first NUL, holding no banned word, not a mob's name in lowercase, and a [`Name`] (2 to 24
    /// ASCII letters and digits).
    #[must_use]
    pub fn check(&self, field: &[u8]) -> Option<Name> {
        let end = field.iter().position(|&byte| byte == 0)?;
        let name = &field[..end];
        if holds_banword(&self.banwords, name) || self.mobs.refuses(name) {
            return None;
        }
        Name::new(std::str::from_utf8(name).ok()?).ok()
    }
}

/// When each account last created a character: the DB server's `s_createTimeByAccountID`,
/// shared by every Channel.
#[derive(Debug, Default)]
pub struct CreateCooldown {
    last: Mutex<HashMap<AccountId, Instant>>,
}

impl CreateCooldown {
    /// Whether the account created a character less than [`CREATE_COOLDOWN`] before `now`.
    #[must_use]
    pub fn cooling(&self, account: AccountId, now: Instant) -> bool {
        self.last
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&account)
            .is_some_and(|&at| now.saturating_duration_since(at) < CREATE_COOLDOWN)
    }

    /// Record that the account created a character at `now`.
    pub fn created(&self, account: AccountId, now: Instant) {
        self.last
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(account, now);
    }
}

/// The offset `number(-300, 300)` gives for a random 32-bit value.
#[must_use]
pub fn create_offset(random: u32) -> i32 {
    let width = CREATE_SPREAD.unsigned_abs() * 2 + 1;
    i32::try_from(random % width).unwrap_or_default() - CREATE_SPREAD
}

/// The create failure record of a type.
#[must_use]
pub fn create_failure(failure_type: u8) -> Vec<u8> {
    GcCreateFailure::new(failure_type).encode()
}

/// The checks the game makes before a creation reaches the store
/// (`CInputLogin::CharacterCreate` and `NewPlayerTable2`, `G/input_login.cpp:403-516`), and the
/// character they build. `offset` is where the character lands from the create start. The error
/// is the create failure type.
///
/// # Errors
///
/// Returns [`CREATE_REFUSED`] when creation is blocked, for a Name the rules refuse, a shape
/// past 1, a slot past 3, a race past 7, or an account without an empire, and [`CREATE_TAKEN`]
/// when the Name is the login.
pub fn judge_create(
    rules: &NameRules,
    blocked: bool,
    account: &SelectAccount,
    record: &CgPlayerCreate,
    offset: (i32, i32),
) -> Result<NewPlayer, u8> {
    if blocked {
        return Err(CREATE_REFUSED);
    }
    let name = rules
        .check(&record.name)
        .filter(|_| record.shape <= 1)
        .ok_or(CREATE_REFUSED)?;
    // `strcmp(login, name)`: case-sensitive.
    if name.as_str() == account.login.as_str() {
        return Err(CREATE_TAKEN);
    }
    // The DB server finds no `pid5` column for a slot past 3 and answers FAILED.
    if usize::from(record.index) >= PLAYER_SLOTS {
        return Err(CREATE_REFUSED);
    }
    let race = u8::try_from(record.job)
        .ok()
        .filter(|race| u16::from(*race) < RACES)
        .ok_or(CREATE_REFUSED)?;
    let empire = account.lobby.empire;
    if !EMPIRES.contains(&empire) {
        return Err(CREATE_REFUSED);
    }
    let (x, y) = CREATE_START[usize::from(empire)];
    let points = &INITIAL_POINTS[usize::from(race % 4)];
    Ok(NewPlayer {
        slot: record.index,
        name,
        job: race,
        st: points.st,
        ht: points.ht,
        dx: points.dx,
        iq: points.iq,
        hp: points.max_hp + i32::from(points.ht) * HP_PER_HT,
        sp: MAX_SP + i32::from(points.iq) * SP_PER_IQ,
        stamina: MAX_STAMINA,
        part_base: record.shape,
        x: x + offset.0,
        y: y + offset.1,
    })
}

/// Apply a created character to the account table and build `GC_PLAYER_CREATE_SUCCESS`
/// (`CInputDB::PlayerCreateSuccess`, `G/input_db.cpp:228-275`). The DB server's `TSimplePlayer`
/// carries the base part as the main part, no hair part, and no play time.
#[must_use]
pub fn created(
    lobby: &mut Lobby,
    player: &NewPlayer,
    id: u32,
    atlas: &MapAtlas,
    locations: &MapLocations,
) -> Vec<u8> {
    let entry = LobbyPlayer {
        slot: player.slot,
        id,
        name: player.name.as_str().to_owned(),
        job: player.job,
        level: 1,
        play_minutes: 0,
        st: player.st,
        ht: player.ht,
        dx: player.dx,
        iq: player.iq,
        conqueror_level: 0,
        sungma_str: 0,
        sungma_hp: 0,
        sungma_move: 0,
        sungma_immune: 0,
        main_part: u16::from(player.part_base),
        hair_part: 0,
        sash_part: 0,
        x: player.x,
        y: player.y,
        skill_group: 0,
        change_name: false,
    };
    let located = locate_here(atlas, locations, entry.x, entry.y);
    let record = player_record(&entry, located);
    lobby.players.retain(|held| held.slot != player.slot);
    lobby.players.push(entry);
    GcPlayerCreateSuccess::new(player.slot, record).encode()
}

/// What a `CHARACTER_DELETE` leads to (`CInputLogin::CharacterDelete`,
/// `G/input_login.cpp:518-554`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteVerdict {
    /// No account, or a slot past 3: nothing is sent.
    Ignore,
    /// The slot is empty: `GC_PLAYER_DELETE_WRONG_SOCIAL_ID`.
    Refuse,
    /// Ask the store to delete this character.
    Ask {
        /// The character in the slot.
        player: u32,
    },
}

/// Judge a `CHARACTER_DELETE` against the account table.
#[must_use]
pub fn judge_delete(account: Option<&SelectAccount>, slot: u8) -> DeleteVerdict {
    let Some(account) = account else {
        return DeleteVerdict::Ignore;
    };
    if usize::from(slot) >= PLAYER_SLOTS {
        return DeleteVerdict::Ignore;
    }
    match account.lobby.in_slot(slot) {
        Some(player) => DeleteVerdict::Ask { player: player.id },
        None => DeleteVerdict::Refuse,
    }
}

/// The answer to a delete the store decided, applied to the account table
/// (`CInputDB::PlayerDeleteSuccess` and `PlayerDeleteFail`, `G/input_db.cpp:277-296`).
#[must_use]
pub fn deleted(lobby: &mut Lobby, slot: u8, done: bool) -> Vec<u8> {
    if done {
        lobby.players.retain(|player| player.slot != slot);
        GcPlayerDeleteSuccess::new(slot).encode()
    } else {
        GcPlayerDeleteWrongSocialId.encode()
    }
}

/// What an `EMPIRE` leads to (`CInputLogin::Empire`, `G/input_login.cpp:961-986`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmpireVerdict {
    /// Empire 0 or past 3: the descriptor closes.
    Close,
    /// No account, or an account that already has an empire and a character.
    Ignore,
    /// Ask the store to set the empire and move the characters to its start.
    Select {
        /// The empire.
        empire: u8,
        /// Its start, `g_start_position`.
        start: (i32, i32),
    },
}

/// Judge an `EMPIRE` against the account table.
#[must_use]
pub fn judge_empire(account: Option<&SelectAccount>, empire: u8) -> EmpireVerdict {
    if !EMPIRES.contains(&empire) {
        return EmpireVerdict::Close;
    }
    let Some(account) = account else {
        return EmpireVerdict::Ignore;
    };
    if account.lobby.empire != 0 && !account.lobby.players.is_empty() {
        return EmpireVerdict::Ignore;
    }
    EmpireVerdict::Select {
        empire,
        start: EMPIRE_START[usize::from(empire)],
    }
}

/// Apply a chosen empire to the account table (`CInputDB::EmpireSelect`,
/// `G/input_db.cpp:1243-1266`). The caller then sends `GC_EMPIRE` and the character list again.
pub fn empire_selected(lobby: &mut Lobby, empire: u8, start: (i32, i32)) {
    lobby.empire = empire;
    for player in &mut lobby.players {
        (player.x, player.y) = start;
    }
}

/// What a `CHANGE_NAME` leads to (`CInputLogin::ChangeName`, `G/input_login.cpp:221-263`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameVerdict {
    /// No account, or a character not asked to choose a new Name: nothing is sent.
    Ignore,
    /// A slot past 3, or an empty slot: the descriptor closes.
    Close,
    /// The rules refuse the Name: the create failure type 0.
    Refuse,
    /// Ask the store to rename this character.
    Ask {
        /// The character in the slot.
        player: u32,
        /// The new Name.
        name: Name,
    },
}

/// Judge a `CHANGE_NAME` against the account table.
#[must_use]
pub fn judge_rename(
    rules: &NameRules,
    account: Option<&SelectAccount>,
    record: &CgChangeName,
) -> RenameVerdict {
    let Some(account) = account else {
        return RenameVerdict::Ignore;
    };
    if usize::from(record.index) >= PLAYER_SLOTS {
        return RenameVerdict::Close;
    }
    let Some(player) = account.lobby.in_slot(record.index) else {
        return RenameVerdict::Close;
    };
    if !player.change_name {
        return RenameVerdict::Ignore;
    }
    match rules.check(&record.name) {
        Some(name) => RenameVerdict::Ask {
            player: player.id,
            name,
        },
        None => RenameVerdict::Refuse,
    }
}

/// Apply a new Name to the account table and build `GC_CHANGE_NAME` (`CInputDB::ChangeName`,
/// `G/input_db.cpp:298-324`); `None` when no character of the table has the ID.
#[must_use]
pub fn renamed(lobby: &mut Lobby, player: u32, name: &Name) -> Option<Vec<u8>> {
    let entry = lobby.players.iter_mut().find(|held| held.id == player)?;
    name.as_str().clone_into(&mut entry.name);
    entry.change_name = false;
    let mut field = [0; GC_NAME_FIELD_SIZE];
    field[..name.as_str().len()].copy_from_slice(name.as_str().as_bytes());
    let mut record = Vec::new();
    GcNamed::new(HEADER_GC_CHANGE_NAME, player, field).encode_into(&mut record);
    Some(record)
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::path::Path;

    use gamedata::map_atlas::MapRegion;

    use super::*;

    fn rules() -> NameRules {
        let mobs = MobNames::from_files(
            Path::new("mob_proto.txt"),
            b"VNUM\tNAME\n101\twolf\n",
            Path::new("mob_names.txt"),
            b"VNUM\tLOCALE_NAME\n",
        )
        .unwrap();
        NameRules::new(vec![BanwordRecord::from_value(b"bad")], mobs)
    }

    fn field(name: &[u8]) -> [u8; 25] {
        let mut field = [0; 25];
        field[..name.len()].copy_from_slice(name);
        field
    }

    fn account(empire: u8, players: Vec<LobbyPlayer>) -> SelectAccount {
        SelectAccount {
            id: AccountId::new(7),
            login: Login::new("hero").unwrap(),
            lobby: Lobby { empire, players },
        }
    }

    fn lobby_player(slot: u8, id: u32, change_name: bool) -> LobbyPlayer {
        LobbyPlayer {
            slot,
            id,
            name: format!("Char{slot}"),
            job: 0,
            level: 5,
            play_minutes: 0,
            st: 1,
            ht: 1,
            dx: 1,
            iq: 1,
            conqueror_level: 0,
            sungma_str: 0,
            sungma_hp: 0,
            sungma_move: 0,
            sungma_immune: 0,
            main_part: 0,
            hair_part: 0,
            sash_part: 0,
            x: 1,
            y: 2,
            skill_group: 0,
            change_name,
        }
    }

    fn create(index: u8, name: &[u8], job: u16, shape: u8) -> CgPlayerCreate {
        CgPlayerCreate {
            index,
            name: field(name),
            job,
            shape,
            con: 0xff,
            int_: 0xff,
            str_: 0xff,
            dex: 0xff,
        }
    }

    #[test]
    fn a_name_is_two_to_24_letters_and_digits_before_a_nul() {
        let rules = rules();
        assert_eq!(rules.check(&field(b"Al9")).unwrap().as_str(), "Al9");
        assert_eq!(
            rules.check(&field(&[b'a'; 24])).unwrap().as_str(),
            "a".repeat(24)
        );
        assert!(rules.check(&[b'a'; 25]).is_none(), "no NUL in the field");
        assert!(rules.check(&field(b"A")).is_none());
        assert!(rules.check(&field(b"")).is_none());
        assert!(rules.check(&field(b"Al_x")).is_none());
        assert!(rules.check(&field(b"Al x")).is_none());
        assert!(rules.check(&field(b"Al\xe9")).is_none());
        let mut after_nul = field(b"Ab");
        after_nul[5] = b'!';
        assert_eq!(
            rules.check(&after_nul).unwrap().as_str(),
            "Ab",
            "bytes after the NUL are not the Name"
        );
    }

    #[test]
    fn a_name_holding_a_banned_word_or_naming_a_mob_is_refused() {
        let rules = rules();
        assert!(rules.check(&field(b"xbadx")).is_none());
        assert!(rules.check(&field(b"bad")).is_none());
        assert!(
            rules.check(&field(b"xBADx")).is_some(),
            "banned words match case-sensitively"
        );
        assert!(rules.check(&field(b"Wolf")).is_none());
        assert!(
            rules.check(&field(b"Wolfy")).is_some(),
            "a mob name matches whole"
        );
        assert!(NameRules::default().check(&field(b"wolf")).is_some());
    }

    #[test]
    fn creation_is_checked_in_the_legacy_order() {
        let rules = rules();
        let hero = account(1, Vec::new());
        let judge = |blocked, account: &SelectAccount, record| {
            judge_create(&rules, blocked, account, &record, (0, 0)).map(|player| player.slot)
        };
        assert_eq!(judge(false, &hero, create(2, b"Alpha", 0, 1)), Ok(2));
        assert_eq!(judge(true, &hero, create(2, b"Alpha", 0, 1)), Err(0));
        assert_eq!(judge(false, &hero, create(2, b"A", 0, 1)), Err(0));
        assert_eq!(judge(false, &hero, create(2, b"Alpha", 0, 2)), Err(0));
        assert_eq!(judge(false, &hero, create(2, b"hero", 0, 1)), Err(1));
        assert_eq!(
            judge(false, &hero, create(2, b"Hero", 0, 1)),
            Ok(2),
            "the login comparison is case-sensitive"
        );
        assert_eq!(
            judge(false, &hero, create(9, b"hero", 0, 1)),
            Err(1),
            "the login is compared before the slot"
        );
        assert_eq!(judge(false, &hero, create(4, b"Alpha", 0, 1)), Err(0));
        assert_eq!(judge(false, &hero, create(3, b"Alpha", 7, 1)), Ok(3));
        assert_eq!(judge(false, &hero, create(3, b"Alpha", 8, 1)), Err(0));
        assert_eq!(
            judge(false, &hero, create(3, b"Alpha", 256, 1)),
            Err(0),
            "the job word is not truncated to a byte"
        );
        for empire in [0, 4, 255] {
            assert_eq!(
                judge(
                    false,
                    &account(empire, Vec::new()),
                    create(0, b"Alpha", 0, 0)
                ),
                Err(0)
            );
        }
    }

    #[test]
    fn a_new_character_starts_with_its_job_points_near_the_create_start() {
        let rules = rules();
        let expected = [
            (6, 4, 3, 3, 760, 260),
            (4, 3, 6, 3, 770, 260),
            (5, 3, 3, 5, 770, 300),
            (3, 4, 3, 6, 860, 320),
        ];
        for race in 0..8_u8 {
            for empire in 1..=3_u8 {
                let player = judge_create(
                    &rules,
                    false,
                    &account(empire, Vec::new()),
                    &create(1, b"Alpha", u16::from(race), 1),
                    (-300, 299),
                )
                .unwrap();
                let (st, ht, dx, iq, hp, sp) = expected[usize::from(race % 4)];
                assert_eq!(
                    (player.st, player.ht, player.dx, player.iq, player.hp, player.sp),
                    (st, ht, dx, iq, hp, sp),
                    "race {race}"
                );
                assert_eq!(player.job, race);
                assert_eq!(player.stamina, 800);
                assert_eq!(player.part_base, 1);
                assert_eq!(player.name.as_str(), "Alpha");
                let (x, y) = CREATE_START[usize::from(empire)];
                assert_eq!((player.x, player.y), (x - 300, y + 299));
            }
        }
    }

    #[test]
    fn the_create_offset_covers_minus_300_to_300() {
        assert_eq!(create_offset(0), -300);
        assert_eq!(create_offset(300), 0);
        assert_eq!(create_offset(600), 300);
        assert_eq!(create_offset(601), -300);
        assert_eq!(
            create_offset(u32::MAX),
            i32::try_from(u32::MAX % 601).unwrap() - 300
        );
    }

    #[test]
    fn the_create_cooldown_lasts_30_seconds_per_account() {
        let cooldown = CreateCooldown::default();
        let start = Instant::now();
        let hero = AccountId::new(7);
        assert!(!cooldown.cooling(hero, start));
        cooldown.created(hero, start);
        assert!(cooldown.cooling(hero, start));
        assert!(cooldown.cooling(hero, start + Duration::from_millis(29_999)));
        assert!(!cooldown.cooling(hero, start + CREATE_COOLDOWN));
        assert!(!cooldown.cooling(AccountId::new(8), start));
    }

    #[test]
    fn a_created_character_is_listed_and_sent_located() {
        let atlas = MapAtlas::from_regions(vec![MapRegion {
            index: 1,
            name: b"test".to_vec(),
            sx: 435_200,
            sy: 921_600,
            ex: 460_800,
            ey: 972_800,
            spawn: (0, 0),
            empire_spawns: None,
        }]);
        let locations = MapLocations::new(Ipv4Addr::new(10, 1, 2, 3), 0x7531, [1], None);
        let mut hero = account(1, vec![lobby_player(0, 5, false)]);
        let player =
            judge_create(&rules(), false, &hero, &create(2, b"Alpha", 5, 1), (-1, 2)).unwrap();
        let bytes = created(&mut hero.lobby, &player, 0x0102_0304, &atlas, &locations);
        assert_eq!(&bytes[..6], [8, 2, 0x04, 0x03, 0x02, 0x01]);
        let record = GcPlayerCreateSuccess::decode(&bytes).unwrap().player;
        assert_eq!(&record.name, &field(b"Alpha"));
        assert_eq!((record.job, record.level, record.play_minutes), (5, 1, 0));
        assert_eq!((record.st, record.ht, record.dx, record.iq), (4, 3, 6, 3));
        assert_eq!((record.main_part, record.hair_part), (1, 0));
        assert_eq!((record.x, record.y), (459_799, 953_902));
        assert_eq!(record.addr, i32::from_le_bytes([10, 1, 2, 3]));
        assert_eq!(record.port, 0x7531);
        let listed = hero.lobby.in_slot(2).unwrap();
        assert_eq!((listed.id, listed.name.as_str()), (0x0102_0304, "Alpha"));
        assert_eq!(hero.lobby.players.len(), 2);

        // Off every hosted map: no location and no empire-start fallback.
        let far = NewPlayer {
            x: 1,
            y: 1,
            ..player
        };
        let bytes = created(&mut hero.lobby, &far, 9, &atlas, &locations);
        let record = GcPlayerCreateSuccess::decode(&bytes).unwrap().player;
        assert_eq!((record.x, record.y, record.addr, record.port), (1, 1, 0, 0));
        assert_eq!(hero.lobby.players.len(), 2, "the slot is replaced");
        assert_eq!(hero.lobby.in_slot(2).unwrap().id, 9);
    }

    #[test]
    fn a_delete_names_a_character_the_table_holds() {
        let hero = account(1, vec![lobby_player(1, 0x0a0b_0c0d, false)]);
        assert_eq!(judge_delete(None, 1), DeleteVerdict::Ignore);
        assert_eq!(judge_delete(Some(&hero), 4), DeleteVerdict::Ignore);
        assert_eq!(judge_delete(Some(&hero), 0), DeleteVerdict::Refuse);
        assert_eq!(
            judge_delete(Some(&hero), 1),
            DeleteVerdict::Ask {
                player: 0x0a0b_0c0d
            }
        );
        let mut lobby = hero.lobby;
        assert_eq!(deleted(&mut lobby, 1, false), [11]);
        assert_eq!(lobby.players.len(), 1);
        assert_eq!(deleted(&mut lobby, 1, true), [10, 1]);
        assert!(lobby.players.is_empty());
    }

    #[test]
    fn an_empire_is_chosen_once_the_account_has_a_character() {
        let fresh = account(0, vec![lobby_player(0, 1, false)]);
        let settled = account(2, vec![lobby_player(0, 1, false)]);
        let empty = account(2, Vec::new());
        for empire in [0, 4, 255] {
            assert_eq!(judge_empire(Some(&settled), empire), EmpireVerdict::Close);
            assert_eq!(judge_empire(None, empire), EmpireVerdict::Close);
        }
        assert_eq!(judge_empire(None, 1), EmpireVerdict::Ignore);
        assert_eq!(judge_empire(Some(&settled), 1), EmpireVerdict::Ignore);
        for (account, empire) in [(&fresh, 3), (&empty, 1)] {
            assert_eq!(
                judge_empire(Some(account), empire),
                EmpireVerdict::Select {
                    empire,
                    start: EMPIRE_START[usize::from(empire)]
                }
            );
        }
        let mut lobby = fresh.lobby;
        empire_selected(&mut lobby, 3, (969_600, 278_400));
        assert_eq!(lobby.empire, 3);
        assert_eq!((lobby.players[0].x, lobby.players[0].y), (969_600, 278_400));
    }

    #[test]
    fn a_rename_needs_a_character_asked_to_choose_a_name() {
        let rules = rules();
        let hero = account(
            1,
            vec![
                lobby_player(0, 0x0102_0304, true),
                lobby_player(1, 2, false),
            ],
        );
        let judge = |account, index, name: &[u8]| {
            judge_rename(&rules, account, &CgChangeName::new(index, field(name)))
        };
        assert_eq!(judge(None, 0, b"Alpha"), RenameVerdict::Ignore);
        assert_eq!(judge(Some(&hero), 4, b"Alpha"), RenameVerdict::Close);
        assert_eq!(judge(Some(&hero), 2, b"Alpha"), RenameVerdict::Close);
        assert_eq!(judge(Some(&hero), 1, b"Alpha"), RenameVerdict::Ignore);
        assert_eq!(judge(Some(&hero), 0, b"wolf"), RenameVerdict::Refuse);
        assert_eq!(
            judge(Some(&hero), 0, b"hero"),
            RenameVerdict::Ask {
                player: 0x0102_0304,
                name: Name::new("hero").unwrap()
            },
            "a rename may take the login"
        );
        let mut lobby = hero.lobby;
        let name = Name::new("Alpha").unwrap();
        assert_eq!(renamed(&mut lobby, 99, &name), None);
        let bytes = renamed(&mut lobby, 0x0102_0304, &name).unwrap();
        let mut expected = vec![0x6b, 0x04, 0x03, 0x02, 0x01];
        expected.extend_from_slice(&field(b"Alpha"));
        assert_eq!(bytes, expected);
        assert_eq!(lobby.players[0].name, "Alpha");
        assert!(!lobby.players[0].change_name);
    }

    #[test]
    fn a_create_failure_is_two_bytes() {
        assert_eq!(create_failure(CREATE_REFUSED), [9, 0]);
        assert_eq!(create_failure(CREATE_TAKEN), [9, 1]);
    }
}
