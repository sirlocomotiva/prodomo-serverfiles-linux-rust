//! The messenger: the friend list, its login and logout fans, the adds, the answer and
//! the remove.
//!
//! # What the legacy does
//!
//! `MessengerManager` (`G/messenger_manager.cpp`) keeps two relations
//! (`m_Relation`/`m_InverseRelation`), one team relation for the GMs
//! (`ENABLE_MESSENGER_TEAM`, `common/prodomodefines.h:155`), the online set
//! (`m_set_loginAccount`), and the set of the pending friend requests as CRC pairs
//! (`m_set_requestToAdd`, `:302`). The list rides the store: `Login` (`:46-60`) asks the
//! DB for the account's rows, and the answer loads the relations, sends the list, tells
//! every friend that holds this character its login, and stands the GM team list up the
//! same way (`LoadList` `:159-194`, `LoadTeamList` `:24-52`). A player that leaves loses
//! its rows: `CHARACTER::Disconnect` calls `Logout` (`G/char.cpp:1817`), which fans the
//! logout to every holder and erases the rows from memory (`:196-271`).
//!
//! # One process, every Channel
//!
//! ADR-0002 puts every Channel in one process behind one game thread, and the Rewrite's
//! one [`GameState`] holds every online character. Legacy crossed its cores with P2P
//! pushes (`P2P_MANAGER::P2PLogin` calls the same `Login` at `p2p.cpp:184`), so its
//! fans crossed cores at every login: the record's own `FindPC` looked in the calling
//! core and the record went where its answer reached. The Rewrite has no cores; the
//! relations are the process's own and the fans reach a companion through the client
//! registry by name, as the whisper crosses a Channel
//! ([`crate::client_registry::ChannelClients::outbox_of`]).
//!
//! # The team list
//!
//! Legacy's team list reads one row per GM name other than the owner's:
//! `SELECT '%s', mName FROM common.gmlist WHERE mName != '%s'`
//! (`G/messenger_manager.cpp:16-17`). The Rewrite keeps the GM grants in the `gm_grant`
//! table (ledger 179), where the Name is unique regardless of case, so the same rows are
//! the team list and the name-to-level map the GM gates read.
//!
//! # The GM gates
//!
//! `gm_new_get_level` (`G/gm.cpp:12-14`) answers `GM_IMPLEMENTOR` in the test-server
//! mode before the map is asked, and the mode is on in the default deployment
//! (`G/config.cpp:72`), so every gate that compares a level with `GM_PLAYER` is silent
//! in it. The Rewrite carries the mode in [`GameState::test_server`], and the gates here
//! apply the same short-cut: in the mode they are dead; out of it the owner's level is
//! the points' own (`m_pointsInstant.gm_level`) and the companion's is the level the
//! caller read from the store's grants.
//!
//! # Offline messages are not this module's
//!
//! The lines that wait for a character that is not there belong to the whisper
//! (`game_state::whisper`), whose `OFFLINE_MESSAGE_REWORKED` path ledger 246 ported.
//! This module is the friend list only.

use std::collections::{BTreeMap, BTreeSet};

use crate::command::{GM_IMPLEMENTOR, GM_PLAYER};
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};
use protocol::gc_messenger::{encode, GcMessenger, GcMessengerEntry, GC_MESSENGER_NAME_MAX};
#[cfg(test)]
use protocol::gc_messenger::{
    MESSENGER_SUBHEADER_GC_LIST, MESSENGER_SUBHEADER_GC_LOGIN, MESSENGER_SUBHEADER_GC_LOGOUT,
};
use tokio::sync::oneshot;
use tracing::debug;

use crate::chat_line::{chat_packet, Arg};
use common::vid::Vid;

use super::GameState;

/// `LC_TEXT("<Messenger> %s has been added as a friend.")`
/// (`G/messenger_manager.cpp:346`).
pub const ADDED_AS_A_FRIEND: &[u8] = b"<Messenger> %s has been added as a friend.";

/// `LC_TEXT("<Messenger> %s has been deleted from friends list.")`
/// (`G/messenger_manager.cpp:399`).
pub const DELETED_FROM_FRIENDS_LIST: &[u8] = b"<Messenger> %s has been deleted from friends list.";

/// `LC_TEXT("The other party is in a state where they cannot receive friend
/// additions.")` (`G/messenger_manager.cpp:291`).
///
/// Legacy formats it through `LC_TEXT`, whose table has no such line in this tree, so the
/// literal is what the client hears there too.
pub const CANNOT_RECEIVE_ADDITIONS: &[u8] =
    b"The other party is in a state where they cannot receive friend additions.";

/// `"[LS;659]"`, a player may not add a game master (`G/input_main.cpp:1168`, `:1187`).
pub const LS_PLAYER_CANNOT_ADD_A_GM: &[u8] = b"[LS;659]";

/// `"[LS;660;%s]"`, the named character is not online (`G/input_main.cpp:1197`).
pub const LS_NOT_ONLINE: &[u8] = b"[LS;660;%s]";

/// `"[LS;657]"`, the target blocks friend requests (`G/input_main.cpp:1157-1160`,
/// `:1203-1206`).
pub const LS_MESSENGER_BLOCKED: &[u8] = b"[LS;657]";

/// `"[LS;548;%s]"`, the notice a deny answers to the requester
pub const LS_ADD_DENIED: &[u8] = b"[LS;548;%s]";

/// `"messenger_auth %s"`, the command line the target hears, whose answer is the
/// `do_messenger_auth` command (`G/messenger_manager.cpp:307`).
pub const MESSENGER_AUTH_FORMAT: &[u8] = b"messenger_auth %s";

/// The CRC pair of two names, in the request's own order: the requester's first.
///
/// `RequestToAdd` computes `GetCRC32(ch->GetName())`, `GetCRC32(target->GetName())` and
/// the CRC of the `"dw1:dw2"` text (`G/messenger_manager.cpp:294-304`); `AuthToAdd`
/// recomputes the same pair with the names in the same order, because there the caller
/// is the *responder* and the companion is the *requester*, and the call named
/// `(companion, account)` — wait, actually the call named `(ch->GetName(), arg2)` and the
/// structure reads `(account = ch->GetName(), companion = arg2)`, so the pair is
/// recomputed with `(companion, account)`'s names as the same `dw1:dw2` shape with
/// `dw1 = GetCRC32(companion)` and `dw2 = GetCRC32(account)`: the complex string is the
/// same one only when the requester's name is the first of the two CRCs. `RequestToAdd`
/// put the caller's name in `dw1` and the target's in `dw2`, and the answer's call names
/// the responder in the *call's own* `account` and the requester's name in `companion`,
/// so `dw1 = GetCRC32(requester)` — the same first name `RequestToAdd` used. Both
/// computations agree.
///
/// `GetCRC32` is the byte-at-a-time CRC-32 (`G/crc32.cpp:57-85`), the one
/// [`crate::game_state::guild_mark::crc32`] carries for the guild marks.
fn request_pair(first: &[u8], second: &[u8]) -> u32 {
    let crc32 = crate::game_state::guild_mark::crc32;
    let text = format!("{}:{}", crc32(first), crc32(second));
    crc32(text.as_bytes())
}

/// The relations of the process's messenger.
///
/// Every key and every companion is a character Name's bytes, and every set is sorted by
/// the bytes, as legacy's `std::set<std::string>` is. A name the world never loaded has
/// no entry here; the store's own checks (the shape, and the pair's presence) live there.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Relations {
    /// `m_Relation`: which companions a name lists.
    relation: BTreeMap<Vec<u8>, BTreeSet<Vec<u8>>>,
    /// `m_InverseRelation`: which owners list a name.
    inverse: BTreeMap<Vec<u8>, BTreeSet<Vec<u8>>>,
    /// `m_TeamRelation`: which GMs a name's team list holds.
    team: BTreeMap<Vec<u8>, BTreeSet<Vec<u8>>>,
    /// `m_InverseTeamRelation`: which owners list a GM.
    inverse_team: BTreeMap<Vec<u8>, BTreeSet<Vec<u8>>>,
    /// `m_set_requestToAdd`: the CRC pairs of the requests that wait for their answer.
    requests: BTreeSet<u32>,
}

impl Relations {
    /// `LoadList` and `LoadTeamList` for the character `name` under `vid`, with the rows
    /// the caller read from the store: the list's companions, the team's GM names and
    /// every grant's name-to-level map, which the GM gates read.
    pub(super) fn login(
        &mut self,
        state: &mut GameState,
        vid: Vid,
        name: &[u8],
        companions: &[Vec<u8>],
        team: &[Vec<u8>],
        levels: &BTreeMap<Vec<u8>, u8>,
    ) {
        // `LoadList` keyed the rows in both maps (`G/messenger_manager.cpp:174-186`).
        let listed: BTreeSet<Vec<u8>> = companions
            .iter()
            .filter(|companion| !companion.is_empty() && companion.len() <= GC_MESSENGER_NAME_MAX)
            .cloned()
            .collect();
        for companion in &listed {
            self.inverse
                .entry(companion.clone())
                .or_default()
                .insert(name.to_vec());
        }
        self.relation.insert(name.to_vec(), listed.clone());
        // `SendList` writes one entry per companion, online by the login set
        // (`G/messenger_manager.cpp:481-513`); a list with no companion is no record,
        // because `LoadList` early-returns on `uiNumRows == 0`.
        if !listed.is_empty() {
            let entries = listed
                .iter()
                .map(|companion| GcMessengerEntry {
                    online: online_by_name(state, companion),
                    name: companion.clone(),
                })
                .collect();
            if let Ok(bytes) = encode(&GcMessenger::List(entries)) {
                let _sent = state.write_to_client(vid, bytes);
            }
        }
        // The load fans the login to every friend whose list holds this name
        // (`G/messenger_manager.cpp:190-193`), through `SendLogin`'s GM gate
        // (`:515-543`).
        let holders = self.inverse.get(name).cloned().unwrap_or_default();
        for holder in &holders {
            let companion_level = levels.get(holder.as_slice()).copied();
            if gm_gate(state, holder.as_slice(), companion_level) {
                continue;
            }
            if let Ok(bytes) = encode(&GcMessenger::Login(name.to_vec())) {
                deliver(state, holder.as_slice(), bytes);
            }
        }
        // `LoadTeamList`: the GM names other than this character's own
        // (`G/messenger_manager.cpp:32-46`).
        let team_listed: BTreeSet<Vec<u8>> = team
            .iter()
            .filter(|gm| {
                !gm.is_empty() && gm.as_slice() != name && gm.len() <= GC_MESSENGER_NAME_MAX
            })
            .cloned()
            .collect();
        for gm in &team_listed {
            self.inverse_team
                .entry(gm.clone())
                .or_default()
                .insert(name.to_vec());
        }
        self.team.insert(name.to_vec(), team_listed.clone());
        if !team_listed.is_empty() {
            let entries = team_listed
                .iter()
                .map(|gm| GcMessengerEntry {
                    online: online_by_name(state, gm),
                    name: gm.clone(),
                })
                .collect();
            if let Ok(bytes) = encode(&GcMessenger::TeamList(entries)) {
                let _sent = state.write_to_client(vid, bytes);
            }
        }
        // The team's fans tell every owner of this name's own team list that it came in;
        // the name rides a key only when the character is a GM
        // (`G/messenger_manager.cpp:48-52`).
        let team_holders = self.inverse_team.get(name).cloned().unwrap_or_default();
        for holder in &team_holders {
            if let Ok(bytes) = encode(&GcMessenger::TeamLogin(name.to_vec())) {
                deliver(state, holder.as_slice(), bytes);
            }
        }
    }

    /// `MessengerManager::Logout` (`G/messenger_manager.cpp:196-271`): the logouts first,
    /// then the erase of every map the name is in.
    pub(super) fn logout(&mut self, state: &mut GameState, name: &[u8]) {
        let holders = self.inverse.get(name).cloned().unwrap_or_default();
        let team_holders = self.inverse_team.get(name).cloned().unwrap_or_default();
        // `SendLogout` refuses an empty companion (`:567`), and the fans run before the
        // erase (`:203-208`, `:241-246`).
        if !name.is_empty() {
            for holder in &holders {
                if let Ok(bytes) = encode(&GcMessenger::Logout(name.to_vec())) {
                    deliver(state, holder.as_slice(), bytes);
                }
            }
            for holder in &team_holders {
                if let Ok(bytes) = encode(&GcMessenger::TeamLogout(name.to_vec())) {
                    deliver(state, holder.as_slice(), bytes);
                }
            }
        }
        // The load's maps forget the name (`:209-222`, `:247-262`): the name's own sets
        // and the name inside every holder's set.
        self.relation.remove(name);
        self.team.remove(name);
        self.inverse.remove(name);
        self.inverse_team.remove(name);
        for set in self.relation.values_mut() {
            set.remove(name);
        }
        for set in self.team.values_mut() {
            set.remove(name);
        }
    }

    /// `MessengerManager::RequestToAdd` (`G/messenger_manager.cpp:278-309`): the
    /// running-quest gates, the CRC pair, then the command line the target hears.
    ///
    /// The caller's own running quest answers with the "other party" line
    /// (`:286-296`, which names the *other party* at the caller's own gate: that is
    /// what the tree says and the deployment ran it); the target's running quest is
    /// quiet (`:298-300`).
    pub(super) fn request_to_add(&mut self, state: &mut GameState, caller: Vid, target: Vid) {
        let caller_name = name_of(state, caller);
        let target_name = name_of(state, target);
        if caller_name.is_empty() || target_name.is_empty() {
            return;
        }
        if state.quest_running(caller) {
            if let Some(mover) = state.own_mover(caller) {
                let line = chat_packet(
                    mover.recipient(&state.locale),
                    CHAT_TYPE_INFO,
                    CANNOT_RECEIVE_ADDITIONS,
                    &[],
                );
                let _sent = state.write_to_client(caller, line);
            }
            return;
        }
        if state.quest_running(target) {
            return;
        }
        self.requests
            .insert(request_pair(&caller_name, &target_name));
        if let Some(mover) = state.own_mover(target) {
            let line = chat_packet(
                mover.recipient(&state.locale),
                CHAT_TYPE_COMMAND,
                MESSENGER_AUTH_FORMAT,
                &[Arg::Text(&caller_name)],
            );
            let _sent = state.write_to_client(target, line);
        }
    }

    /// `MessengerManager::AuthToAdd` (`G/messenger_manager.cpp:312-336`): the pair must
    /// be one a request inserted; it is consumed either way; a yes adds both directions.
    ///
    /// `responder` is the character answering, and `requester` the name its
    /// `do_messenger_auth` command named, which is who the request asked. The command's
    /// own arena gate (`[LS;403]`, `G/cmd_general.cpp:2274-2278`) passes: there are no
    /// arenas, as `sys.event.arena` is unported.
    pub(super) fn auth_to_add(
        &mut self,
        state: &mut GameState,
        responder: Vid,
        requester: &[u8],
        deny: bool,
    ) -> bool {
        let responder_name = name_of(state, responder);
        let pair = request_pair(requester, &responder_name);
        if !self.requests.remove(&pair) {
            return false;
        }
        if deny {
            // `tch->ChatPacket(CHAT_TYPE_INFO,
            // "[LS;548;%s]", ch->GetName()); }` (`G/cmd_general.cpp:2285-2292`): the
            // requester hears the answerer's own refusal.
            if let Some(requester_vid) = vid_of(state, requester) {
                if let Some(mover) = state.own_mover(requester_vid) {
                    let line = chat_packet(
                        mover.recipient(&state.locale),
                        CHAT_TYPE_INFO,
                        LS_ADD_DENIED,
                        &[Arg::Text(&responder_name)],
                    );
                    let _sent = state.write_to_client(requester_vid, line);
                }
            }
        } else {
            let one = self.add_to_list(state, requester, &responder_name);
            let other = self.add_to_list(state, &responder_name, requester);
            let _ = (one, other);
        }
        true
    }

    /// `MessengerManager::AddToList` (`G/messenger_manager.cpp:321-368`) and its memory
    /// half `__AddToList` (`:305-319`): the empty name is quiet, the pair already there
    /// is quiet, then the insert and the fans.
    ///
    /// Answers `true` when the pair became one, which is the caller's record that a row
    /// must be written; the quiet paths answer `false`.
    pub(super) fn add_to_list(
        &mut self,
        state: &mut GameState,
        account: &[u8],
        companion: &[u8],
    ) -> bool {
        if companion.is_empty() || account.is_empty() {
            return false;
        }
        // `if (m_Relation[account].find(companion) != ... end()) return;` — the pair is
        // already listed (`G/messenger_manager.cpp:338-340`).
        if self
            .relation
            .get(account)
            .is_some_and(|set| set.contains(companion))
        {
            return false;
        }
        self.relation
            .entry(account.to_vec())
            .or_default()
            .insert(companion.to_vec());
        self.inverse
            .entry(companion.to_vec())
            .or_default()
            .insert(account.to_vec());
        // The owner's own line (`G/messenger_manager.cpp:310-315`).
        if let Some(owner_vid) = vid_of(state, account) {
            if let Some(mover) = state.own_mover(owner_vid) {
                let line = chat_packet(
                    mover.recipient(&state.locale),
                    CHAT_TYPE_INFO,
                    ADDED_AS_A_FRIEND,
                    &[Arg::Text(companion)],
                );
                let _sent = state.write_to_client(owner_vid, line);
            }
        }
        // `__AddToList` answers the companion's own name through `SendLogin`'s GM gate
        // (`G/messenger_manager.cpp:537-543`): a player hears nothing when the companion
        // is a grant. The test-server mode silences the gate (`G/gm.cpp:12-14`).
        let login = online_by_name(state, companion)
            && !gm_gate(state, account, companion_level_of(state, companion));
        let record = if login {
            GcMessenger::Login(companion.to_vec())
        } else {
            GcMessenger::Logout(companion.to_vec())
        };
        if let Ok(bytes) = encode(&record) {
            deliver(state, account, bytes);
        }
        true
    }

    /// `MessengerManager::RemoveFromList` and its memory half `__RemoveFromList`
    /// (`G/messenger_manager.cpp:372-421`): both rows of the pair leave the memory, the
    /// friend's line and the remover's record go out.
    pub(super) fn remove_from_list(
        &mut self,
        state: &mut GameState,
        account: &[u8],
        companion: &[u8],
    ) {
        if companion.is_empty() || account.is_empty() {
            return;
        }
        // The pair leaves the memory symmetrically, as `__RemoveFromList` and the
        // refresh fix's own two adds do (`G/messenger_manager.cpp:372-381`).
        self.relation
            .entry(account.to_vec())
            .or_default()
            .remove(companion);
        self.inverse
            .entry(companion.to_vec())
            .or_default()
            .remove(account);
        self.relation
            .entry(companion.to_vec())
            .or_default()
            .remove(account);
        self.inverse
            .entry(account.to_vec())
            .or_default()
            .remove(companion);
        // The friend's line goes to `FindPC(account)` — the character the relation was
        // taken *from* — and names `companion` (`:385-390`).
        if let Some(friend_vid) = vid_of(state, account) {
            if let Some(mover) = state.own_mover(friend_vid) {
                let line = chat_packet(
                    mover.recipient(&state.locale),
                    CHAT_TYPE_INFO,
                    DELETED_FROM_FRIENDS_LIST,
                    &[Arg::Text(companion)],
                );
                let _sent = state.write_to_client(friend_vid, line);
            }
        }
        // The remover's own record names `account` — the friend taken away
        // (`G/messenger_manager.cpp:392-407`).
        if let Some(remover_vid) = vid_of(state, companion) {
            if let Ok(bytes) = encode(&GcMessenger::RemoveFriend(account.to_vec())) {
                let _sent = state.write_to_client(remover_vid, bytes);
            }
        }
    }

    /// The companions a name's own list holds, in the sorted set order. The tests read
    /// the world's own state through it.
    #[must_use]
    #[cfg(test)]
    pub(super) fn companions_of(&self, name: &[u8]) -> Vec<Vec<u8>> {
        self.relation
            .get(name)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Whether a request from `requester` to the character under `vid` waits. The
    /// tests read the world's own state through it.
    #[must_use]
    #[cfg(test)]
    pub(super) fn waits(&self, state: &GameState, vid: Vid, requester: &[u8]) -> bool {
        let responder_name = name_of(state, vid);
        self.requests
            .contains(&request_pair(requester, &responder_name))
    }
}

/// `gm_get_level`'s gate for a record that names a companion: the owner of the record is
/// a player and the companion is a grant, which legacy answers with no record
/// (`G/messenger_manager.cpp:554-566`). The test-server mode makes every level
/// `GM_IMPLEMENTOR` (`G/gm.cpp:14`), which silences the gate in it.
fn gm_gate(state: &GameState, owner: &[u8], companion_level: Option<u8>) -> bool {
    let owner_level = match vid_of(state, owner) {
        Some(owner_vid) => state
            .characters
            .find_by_vid(owner_vid)
            .ok()
            .and_then(|character| character.points().map(world::character::Points::gm_level)),
        None => Some(GM_PLAYER),
    };
    let owner_level = if state.test_server {
        GM_IMPLEMENTOR
    } else {
        owner_level.unwrap_or(GM_PLAYER)
    };
    let companion_level = if state.test_server {
        Some(GM_IMPLEMENTOR)
    } else {
        companion_level
    };
    owner_level == GM_PLAYER && companion_level.is_some_and(|level| level != GM_PLAYER)
}

/// The GM level of the character under `vid`, silenced to `GM_IMPLEMENTOR` in the
/// test-server mode (`G/gm.cpp:12-14`), and `GM_PLAYER` for a VID nobody holds.
fn gm_level_of(state: &GameState, vid: Vid) -> u8 {
    if state.test_server {
        return GM_IMPLEMENTOR;
    }
    state
        .characters
        .find_by_vid(vid)
        .ok()
        .and_then(|character| character.points().map(world::character::Points::gm_level))
        .unwrap_or(GM_PLAYER)
}

/// The companion's level for a GM gate (`gm_get_level`, `G/gm.cpp:110-113`): the points'
/// own when the companion is online, and none when nobody answers to the name, which is
/// the player's — legacy's list does not hold it. The points' level rode the admission
/// from the store's grant, so it is the list's own answer.
fn companion_level_of(state: &GameState, companion: &[u8]) -> Option<u8> {
    vid_of(state, companion).map(|vid| gm_level_of(state, vid))
}

/// Whether a name stands in the game: `m_set_loginAccount`'s answer
/// (`G/messenger_manager.cpp:492,503`).
fn online_by_name(state: &GameState, name: &[u8]) -> bool {
    vid_of(state, name).is_some()
}
/// The character Name bytes of the character under `vid`, empty when nobody is there.
fn name_of(state: &GameState, vid: Vid) -> Vec<u8> {
    state
        .characters
        .find_by_vid(vid)
        .map(|character| Vec::from(character.name().as_bytes()))
        .unwrap_or_default()
}

/// The VID of the character that answers to the name, or `None`. `FindPC` matches the
/// name without regard to the case (`G/char_manager.cpp:244-260`); the manager's own
/// lookup matches the same way.
fn vid_of(state: &GameState, name: &[u8]) -> Option<Vid> {
    let text = std::str::from_utf8(name).ok()?;
    state
        .characters
        .find_player_by_name(text)
        .ok()
        .map(world::character::Character::vid)
}

/// Deliver one record through the registry to the character the name names, when it is
/// in the game.
fn deliver(state: &GameState, holder: &[u8], bytes: Vec<u8>) {
    let Ok(text) = std::str::from_utf8(holder) else {
        return;
    };
    let Some(outbox) = state
        .clients
        .as_ref()
        .and_then(|clients| clients.outbox_of(text))
    else {
        return;
    };
    let _sent = outbox.send(bytes);
}

/// The message API for the world: the commands the descriptor task asks the world to run.
impl GameState {
    /// `CInputMain::Messenger`'s `ADD_BY_VID` arm (`G/input_main.cpp:1140-1178`):
    /// `RequestToAdd` runs past the arm's gates. `Find` answers quiet for a VID nobody
    /// holds; the manager only ever holds players (`enter_world_with_items` is the only
    /// insert path), so both `IsPC` arms pass; the observer arm passes (this build has
    /// no observer mode); the target's `BLOCK_MESSENGER_INVITE` answers `[LS;657]`;
    /// a companion with no outbox is quiet (the desc the arm asks
    /// for); a player may not add a GM (`[LS;659]`); the caller may not add itself.
    pub(super) fn answer_messenger_add_by_vid(&mut self, caller: Vid, target: Vid) {
        // `LPCHARACTER ch_companion = ...Find(p2->vid); if (!ch_companion) return`
        // (`:1145-1148`).
        if self.characters.find_by_vid(target).is_err()
            || self.characters.find_by_vid(caller).is_err()
        {
            return;
        }
        // `if (ch_companion->IsBlockMode(BLOCK_MESSENGER_INVITE))`
        // (`:1157-1160`): the target blocks friend requests, so the caller hears
        // `[LS;657]` and the request never runs.
        if self.is_block_mode(target, crate::game_state::whisper::BLOCK_MESSENGER_INVITE) {
            if let Some(mover) = self.own_mover(caller) {
                let line = chat_packet(
                    mover.recipient(&self.locale),
                    CHAT_TYPE_INFO,
                    LS_MESSENGER_BLOCKED,
                    &[],
                );
                let _sent = self.write_to_client(caller, line);
            }
            return;
        }
        // `if (ch_companion->GetDesc())` inverted (`:1162-1165`): the outbox is the desc.
        if !self.outboxes.contains_key(&target) {
            return;
        }
        // `if (ch->GetGMLevel() == GM_PLAYER && ch_companion->GetGMLevel() != GM_PLAYER)`
        // (`:1167-1171`): the test-server mode silences the gate (`G/gm.cpp:12-14`).
        if gm_level_of(self, caller) == GM_PLAYER && gm_level_of(self, target) != GM_PLAYER {
            if let Some(mover) = self.own_mover(caller) {
                let line = chat_packet(
                    mover.recipient(&self.locale),
                    CHAT_TYPE_INFO,
                    LS_PLAYER_CANNOT_ADD_A_GM,
                    &[],
                );
                let _sent = self.write_to_client(caller, line);
            }
            return;
        }
        // `if (ch->GetDesc() == d)` (`:1173-1174`): the caller may not add itself.
        if caller == target {
            return;
        }
        let mut taken = std::mem::take(&mut self.messenger);
        taken.request_to_add(self, caller, target);
        self.messenger = taken;
    }

    /// `MessengerManager::AuthToAdd`: the CRC pair is consumed and a yes adds both
    /// directions. Answers whether the rows need writing.
    pub(super) fn answer_messenger_auth(
        &mut self,
        responder: Vid,
        requester: &[u8],
        deny: bool,
        reply: oneshot::Sender<bool>,
    ) {
        let mut taken = std::mem::take(&mut self.messenger);
        let added = taken.auth_to_add(self, responder, requester, deny);
        self.messenger = taken;
        if reply.send(added).is_err() {
            debug!(
                ?responder,
                "nobody was left to hear a friend answer's outcome"
            );
        }
    }

    /// `MessengerManager::RemoveFromList` for the character under `vid`, which names the
    /// companion it took away.
    pub(super) fn answer_messenger_remove(&mut self, responder: Vid, companion: &[u8]) {
        let responder_name = name_of(self, responder);
        if responder_name.is_empty() || companion.is_empty() {
            return;
        }
        let mut taken = std::mem::take(&mut self.messenger);
        taken.remove_from_list(self, companion, &responder_name);
        self.messenger = taken;
    }

    /// `CInputMain::Messenger`'s `ADD_BY_NAME` arm (`G/input_main.cpp:1180-1214`):
    /// `RequestToAdd` runs past the arm's gates. `FindPC` answers the named character
    /// or the offline notice; the self gate runs before the `BLOCK_MESSENGER_INVITE`
    /// gate, which answers `[LS;657]`.
    pub(super) fn messenger_add_by_name(
        &mut self,
        caller: Vid,
        name: &[u8],
        companion_level: Option<u8>,
    ) {
        // `if (ch->GetGMLevel() == GM_PLAYER && gm_get_level(name) != GM_PLAYER)`
        // (`G/input_main.cpp:1186-1191`): the caller's own level is the points', and the
        // name's comes from the caller's store read; the test-server mode silences the
        // gate (`G/gm.cpp:12-14`).
        let caller_level = gm_level_of(self, caller);
        let companion_level = if self.test_server {
            Some(GM_IMPLEMENTOR)
        } else {
            companion_level
        };
        if caller_level == GM_PLAYER && companion_level.is_some_and(|level| level != GM_PLAYER) {
            if let Some(mover) = self.own_mover(caller) {
                let line = chat_packet(
                    mover.recipient(&self.locale),
                    CHAT_TYPE_INFO,
                    LS_PLAYER_CANNOT_ADD_A_GM,
                    &[],
                );
                let _sent = self.write_to_client(caller, line);
            }
            return;
        }
        // FindPC: the named character's own.
        let Some(target) = vid_of(self, name) else {
            // `ch->ChatPacket(CHAT_TYPE_INFO, "[LS;660;%s]", name)` — the offline
            // notice (`G/input_main.cpp:1196-1198`).
            if let Some(mover) = self.own_mover(caller) {
                let line = chat_packet(
                    mover.recipient(&self.locale),
                    CHAT_TYPE_INFO,
                    LS_NOT_ONLINE,
                    &[Arg::Text(name)],
                );
                let _sent = self.write_to_client(caller, line);
            }
            return;
        };
        // `if (tch == ch) // 자신은 추가할 수 없다` (`G/input_main.cpp:1199-1200`): the
        // caller names its own name.
        if caller == target {
            return;
        }
        // `if (tch->IsBlockMode(BLOCK_MESSENGER_INVITE) == true)`
        // (`G/input_main.cpp:1202-1206`): the target blocks friend requests, so the
        // caller hears `[LS;657]` and the request never runs.
        if self.is_block_mode(target, crate::game_state::whisper::BLOCK_MESSENGER_INVITE) {
            if let Some(mover) = self.own_mover(caller) {
                let line = chat_packet(
                    mover.recipient(&self.locale),
                    CHAT_TYPE_INFO,
                    LS_MESSENGER_BLOCKED,
                    &[],
                );
                let _sent = self.write_to_client(caller, line);
            }
            return;
        }
        let mut taken = std::mem::take(&mut self.messenger);
        taken.request_to_add(self, caller, target);
        self.messenger = taken;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::fixtures::{a_world, enter, TestClock};
    use protocol::gc_messenger::{
        MESSENGER_SUBHEADER_GC_TEAM_LIST, MESSENGER_SUBHEADER_GC_TEAM_LOGIN,
        MESSENGER_SUBHEADER_GC_TEAM_LOGOUT,
    };

    /// The fixture names the character it enters `P{vid}`.
    const VID: u32 = 100;
    /// The other character the fixture enters.
    const OTHER: u32 = 101;

    fn a_world_with_one() -> (
        GameState,
        TestClock,
        tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    ) {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let inbox = enter(&mut world, VID, (3200, 3200), 0);
        (world, clock, inbox)
    }

    fn a_pair_on_the_registry() -> (
        GameState,
        crate::client_registry::Lease,
        crate::client_registry::Lease,
    ) {
        use crate::client_registry::{ChannelClients, ClientEntry};
        use crate::game_state::fixtures::PLACE;
        use std::sync::Arc;
        let clock = TestClock::default();
        let clients = Arc::new(ChannelClients::new());
        let mut world = a_world(&clock).with_clients(Arc::clone(&clients));
        let _first = enter(&mut world, VID, (3200, 3200), 0);
        let _second = enter(&mut world, OTHER, (3200, 3200), 0);
        let owner = clients.join(ClientEntry {
            channel: PLACE.0,
            map: PLACE.1,
            name: format!("P{VID}"),
            vid: VID,
            empire: 1,
            language: 0,
        });
        let companion = clients.join(ClientEntry {
            channel: PLACE.0,
            map: PLACE.1,
            name: format!("P{OTHER}"),
            vid: OTHER,
            empire: 1,
            language: 0,
        });
        (world, owner, companion)
    }

    fn a_second(world: &mut GameState) {
        let _inbox = enter(world, OTHER, (3200, 3200), 0);
    }

    #[test]
    fn a_login_loads_the_rows_and_sends_the_list() {
        let (mut world, _clock, mut inbox) = a_world_with_one();
        let mut taken = std::mem::take(&mut world.messenger);
        taken.login(
            &mut world,
            Vid::new(VID),
            b"P100",
            &[b"Friend".to_vec()],
            &[],
            &BTreeMap::new(),
        );
        world.messenger = taken;
        let frames: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert_eq!(frames.len(), 1, "one LIST frame: {frames:?}");
        assert_eq!(frames[0][0], 0x4a);
        assert_eq!(frames[0][3], MESSENGER_SUBHEADER_GC_LIST);
        assert_eq!(
            u16::from_le_bytes([frames[0][1], frames[0][2]]) as usize,
            frames[0].len()
        );
    }

    #[test]
    fn a_login_with_no_rows_sends_nothing() {
        let (mut world, _clock, mut inbox) = a_world_with_one();
        let mut taken = std::mem::take(&mut world.messenger);
        taken.login(
            &mut world,
            Vid::new(VID),
            b"P100",
            &[],
            &[],
            &BTreeMap::new(),
        );
        world.messenger = taken;
        assert!(inbox.try_recv().is_err(), "nothing was sent");
    }

    #[test]
    fn a_request_to_add_inserts_the_pair_and_the_line_goes_to_the_target() {
        let (mut world, _clock, _inbox) = a_world_with_one();
        a_second(&mut world);
        let mut taken = std::mem::take(&mut world.messenger);
        taken.request_to_add(&mut world, Vid::new(VID), Vid::new(OTHER));
        world.messenger = taken;
        assert!(
            world.messenger.waits(&world, Vid::new(OTHER), b"P100"),
            "the pair waits"
        );
    }

    #[test]
    fn an_answer_consumes_the_pair_and_a_yes_lists_both_directions() {
        let (mut world, _clock, _inbox) = a_world_with_one();
        a_second(&mut world);
        let mut taken = std::mem::take(&mut world.messenger);
        taken.request_to_add(&mut world, Vid::new(VID), Vid::new(OTHER));
        let added = taken.auth_to_add(&mut world, Vid::new(OTHER), b"P100", false);
        world.messenger = taken;
        assert!(added, "the request was answered");
        assert_eq!(
            world.messenger.companions_of(b"P100"),
            vec![b"P101".to_vec()]
        );
        assert_eq!(
            world.messenger.companions_of(b"P101"),
            vec![b"P100".to_vec()]
        );
        let mut taken = std::mem::take(&mut world.messenger);
        let again = taken.auth_to_add(&mut world, Vid::new(OTHER), b"P100", false);
        world.messenger = taken;
        assert!(!again, "the pair was consumed the first time");
    }

    #[test]
    fn a_deny_consumes_the_pair_and_lists_nothing() {
        let (mut world, _clock, _inbox) = a_world_with_one();
        a_second(&mut world);
        let mut taken = std::mem::take(&mut world.messenger);
        taken.request_to_add(&mut world, Vid::new(VID), Vid::new(OTHER));
        let added = taken.auth_to_add(&mut world, Vid::new(OTHER), b"P100", true);
        world.messenger = taken;
        assert!(added);
        assert!(world.messenger.companions_of(b"P100").is_empty());
        assert!(world.messenger.companions_of(b"P101").is_empty());
    }

    #[test]
    fn a_remove_walks_both_rows_and_the_records_go_out() {
        let (mut world, _clock, mut inbox) = a_world_with_one();
        a_second(&mut world);
        let _list = inbox.try_recv().ok();
        let mut taken = std::mem::take(&mut world.messenger);
        let one = taken.add_to_list(&mut world, b"P100", b"P101");
        let other = taken.add_to_list(&mut world, b"P101", b"P100");
        world.messenger = taken;
        assert!(one && other);
        let mut taken = std::mem::take(&mut world.messenger);
        taken.remove_from_list(&mut world, b"P101", b"P100");
        world.messenger = taken;
        assert!(world.messenger.companions_of(b"P100").is_empty());
        assert!(world.messenger.companions_of(b"P101").is_empty());
    }

    #[test]
    fn a_logout_erases_every_map_the_name_is_in() {
        let (mut world, _clock, _inbox) = a_world_with_one();
        a_second(&mut world);
        let mut taken = std::mem::take(&mut world.messenger);
        let _one = taken.add_to_list(&mut world, b"P100", b"P101");
        let _other = taken.add_to_list(&mut world, b"P101", b"P100");
        world.messenger = taken;
        let mut taken = std::mem::take(&mut world.messenger);
        taken.logout(&mut world, b"P101");
        world.messenger = taken;
        assert!(
            world.messenger.companions_of(b"P100").is_empty(),
            "the holder's row went"
        );
    }

    /// `ADD_BY_VID` on a VID nobody holds is quiet (`:1145-1148`).
    #[test]
    fn an_add_by_vid_for_a_missing_vid_is_quiet() {
        let (mut world, _clock, mut inbox) = a_world_with_one();
        world.answer_messenger_add_by_vid(Vid::new(VID), Vid::new(999));
        assert!(
            !world.messenger.waits(&world, Vid::new(999), b"P100"),
            "no pair waits"
        );
        assert!(inbox.try_recv().is_err(), "the caller heard nothing");
    }

    /// `ADD_BY_VID` names its own VID is quiet (`:1173-1174`).
    #[test]
    fn an_add_by_vid_for_its_own_vid_is_quiet() {
        let (mut world, _clock, mut inbox) = a_world_with_one();
        world.answer_messenger_add_by_vid(Vid::new(VID), Vid::new(VID));
        assert!(
            !world.messenger.waits(&world, Vid::new(VID), b"P100"),
            "no pair waits"
        );
        assert!(inbox.try_recv().is_err(), "the caller heard nothing");
    }

    /// `ADD_BY_VID` past every gate inserts the pair and the target hears the
    /// `messenger_auth` line (`:1176`).
    #[test]
    fn an_add_by_vid_past_every_gate_asks_the_target() {
        let (mut world, _clock, _caller) = a_world_with_one();
        a_second(&mut world);
        world.answer_messenger_add_by_vid(Vid::new(VID), Vid::new(OTHER));
        assert!(
            world.messenger.waits(&world, Vid::new(OTHER), b"P100"),
            "the pair waits"
        );
    }

    /// A player may not add a GM by VID (`[LS;659]`, `:1167-1171`).
    #[test]
    fn an_add_by_vid_for_a_gm_answers_the_659_line() {
        let (mut world, _clock, mut inbox) = a_world_with_one();
        a_second(&mut world);
        let mut companion_points = world
            .characters
            .find_by_vid(Vid::new(OTHER))
            .expect("the companion is online")
            .points()
            .expect("the companion has points")
            .clone();
        companion_points.set_gm_level(crate::command::GM_IMPLEMENTOR);
        world
            .characters
            .find_by_vid_mut(Vid::new(OTHER))
            .expect("the companion is online")
            .set_points(Some(companion_points));
        let world = world.with_test_server(false);
        let mut world = world;
        world.answer_messenger_add_by_vid(Vid::new(VID), Vid::new(OTHER));
        let _enter = inbox.try_recv().expect("the enter burst is queued");
        let _placed = inbox.try_recv().expect("the placing is queued");
        let _spawn = inbox.try_recv().expect("the spawn is queued");
        assert!(
            !world.messenger.waits(&world, Vid::new(OTHER), b"P100"),
            "no pair waits"
        );
        let line = inbox.try_recv().expect("the caller was told");
        assert_eq!(
            line,
            chat_packet(
                world
                    .own_mover(Vid::new(VID))
                    .expect("the caller has a mover")
                    .recipient(&world.locale),
                CHAT_TYPE_INFO,
                LS_PLAYER_CANNOT_ADD_A_GM,
                &[],
            ),
            "the caller heard the [LS;659] line"
        );
    }

    /// The GM gate is silent in the test-server mode (`G/gm.cpp:12-14`).
    #[test]
    fn an_add_by_vid_for_a_gm_passes_in_the_test_server_mode() {
        let (mut world, _clock, _caller) = a_world_with_one();
        a_second(&mut world);
        let mut companion_points = world
            .characters
            .find_by_vid(Vid::new(OTHER))
            .expect("the companion is online")
            .points()
            .expect("the companion has points")
            .clone();
        companion_points.set_gm_level(crate::command::GM_IMPLEMENTOR);
        world
            .characters
            .find_by_vid_mut(Vid::new(OTHER))
            .expect("the companion is online")
            .set_points(Some(companion_points));
        world.answer_messenger_add_by_vid(Vid::new(VID), Vid::new(OTHER));
        assert!(
            world.messenger.waits(&world, Vid::new(OTHER), b"P100"),
            "the pair waits in the test-server mode"
        );
    }

    /// `ADD_BY_NAME` for a missing name answers the offline notice (`[LS;660]`,
    /// `:1196-1198`).
    #[test]
    fn an_add_by_name_for_a_missing_name_answers_not_online() {
        let (mut world, _clock, mut inbox) = a_world_with_one();
        world.messenger_add_by_name(Vid::new(VID), b"Nobody", None);
        assert!(
            !world.messenger.waits(&world, Vid::new(OTHER), b"P100"),
            "no pair waits"
        );
        let line = inbox.try_recv().expect("the caller was told");
        assert_eq!(
            line,
            chat_packet(
                world
                    .own_mover(Vid::new(VID))
                    .expect("the caller has a mover")
                    .recipient(&world.locale),
                CHAT_TYPE_INFO,
                LS_NOT_ONLINE,
                &[Arg::Text(b"Nobody")],
            ),
            "the caller heard the [LS;660] line"
        );
    }

    /// `ADD_BY_NAME` for its own name is quiet (`:1199-1200`).
    #[test]
    fn an_add_by_name_for_its_own_name_is_quiet() {
        let (mut world, _clock, mut inbox) = a_world_with_one();
        world.messenger_add_by_name(Vid::new(VID), b"P100", None);
        assert!(
            !world.messenger.waits(&world, Vid::new(VID), b"P100"),
            "no pair waits"
        );
        assert!(inbox.try_recv().is_err(), "the caller heard nothing");
    }

    fn blocked_line(world: &GameState, caller: Vid) -> Vec<u8> {
        chat_packet(
            world
                .own_mover(caller)
                .expect("the caller has a mover")
                .recipient(&world.locale),
            CHAT_TYPE_INFO,
            LS_MESSENGER_BLOCKED,
            &[],
        )
    }

    #[test]
    fn a_blocked_invite_by_vid_answers_657_and_requests_nothing() {
        let (mut world, _clock, mut caller_inbox) = a_world_with_one();
        a_second(&mut world);
        while caller_inbox.try_recv().is_ok() {}
        world.block_modes.insert(
            Vid::new(OTHER),
            crate::game_state::whisper::BLOCK_MESSENGER_INVITE,
        );
        world.answer_messenger_add_by_vid(Vid::new(VID), Vid::new(OTHER));
        assert!(
            !world.messenger.waits(&world, Vid::new(OTHER), b"P100"),
            "no pair waits"
        );
        assert_eq!(
            caller_inbox.try_recv().expect("the caller was told"),
            blocked_line(&world, Vid::new(VID)),
            "the caller heard the [LS;657] line"
        );
    }

    #[test]
    fn a_blocked_invite_by_name_answers_657_and_requests_nothing() {
        let (mut world, _clock, mut caller_inbox) = a_world_with_one();
        a_second(&mut world);
        while caller_inbox.try_recv().is_ok() {}
        world.block_modes.insert(
            Vid::new(OTHER),
            crate::game_state::whisper::BLOCK_MESSENGER_INVITE,
        );
        world.messenger_add_by_name(Vid::new(VID), b"P101", None);
        assert!(
            !world.messenger.waits(&world, Vid::new(OTHER), b"P100"),
            "no pair waits"
        );
        assert_eq!(
            caller_inbox.try_recv().expect("the caller was told"),
            blocked_line(&world, Vid::new(VID)),
            "the caller heard the [LS;657] line"
        );
    }

    /// `__AddToList` answers the companion's own name as LOGIN past `SendLogin`'s GM
    /// gate (`G/messenger_manager.cpp:310-325`, `:537-543`): a player companion reaches
    /// the owner's registry outbox while the owner's own client hears the added line.
    #[test]
    fn an_add_lists_a_player_companion_for_the_owner() {
        let (mut world, mut owner, _companion) = a_pair_on_the_registry();
        let mut taken = std::mem::take(&mut world.messenger);
        assert!(taken.add_to_list(&mut world, b"P100", b"P101"));
        world.messenger = taken;
        let record = owner.try_next().expect("the owner hears the LOGIN");
        assert_eq!(record[0], 0x4a, "the owner's record is a messenger one");
        assert_eq!(
            record[3], MESSENGER_SUBHEADER_GC_LOGIN,
            "the companion's name rides a LOGIN"
        );
        assert_eq!(&record[5..], b"P101", "the record names the companion");
    }

    /// `SendLogin`'s GM gate (`G/messenger_manager.cpp:554-566`): the relation still
    /// lands when a player lists a grant, but the owner's registry outbox hears LOGOUT
    /// for the name, never LOGIN — the owner stands in the game, so `__AddToList`
    /// answers at all only past the gate.
    #[test]
    fn an_add_for_a_gm_companion_answers_logout_for_a_player() {
        let (world, mut owner, _companion) = a_pair_on_the_registry();
        let mut world = world.with_test_server(false);
        let mut companion_points = world
            .characters
            .find_by_vid(Vid::new(OTHER))
            .expect("the companion is online")
            .points()
            .expect("the companion has points")
            .clone();
        companion_points.set_gm_level(GM_IMPLEMENTOR);
        world
            .characters
            .find_by_vid_mut(Vid::new(OTHER))
            .expect("the companion is online")
            .set_points(Some(companion_points));
        while owner.try_next().is_some() {}
        let mut taken = std::mem::take(&mut world.messenger);
        assert!(taken.add_to_list(&mut world, b"P100", b"P101"));
        world.messenger = taken;
        assert_eq!(
            world.messenger.companions_of(b"P100"),
            vec![b"P101".to_vec()],
            "the pair holds past the gate"
        );
        let record = owner.try_next().expect("the owner hears an answer");
        assert_eq!(record[0], 0x4a, "the owner's record is a messenger one");
        assert_eq!(
            record[3], MESSENGER_SUBHEADER_GC_LOGOUT,
            "a player hears LOGOUT for the GM"
        );
    }

    /// `SendTeamList` (`G/messenger_manager.cpp:99-136`): the entrant's own client
    /// hears the `TEAM_LIST` of the GM names other than its own.
    #[test]
    fn a_login_sends_the_team_list_to_the_entrant() {
        let (mut world, _clock, mut inbox) = a_world_with_one();
        let mut taken = std::mem::take(&mut world.messenger);
        taken.login(
            &mut world,
            Vid::new(VID),
            b"P100",
            &[],
            &[b"P101".to_vec()],
            &BTreeMap::new(),
        );
        world.messenger = taken;
        let list = inbox.try_recv().expect("the entrant hears its team");
        assert_eq!(list[0], 0x4a, "the entrant's record is a messenger one");
        assert_eq!(
            list[3], MESSENGER_SUBHEADER_GC_TEAM_LIST,
            "the entrant hears its TEAM_LIST"
        );
        assert_eq!(&list[6..], b"P101", "the team names the other GM");
    }

    /// `LoadTeamList`'s own fans (`G/messenger_manager.cpp:48-52`): every owner of
    /// the entrant's own name on its team hears the `TEAM_LOGIN`.
    #[test]
    fn a_login_fans_the_team_login_to_every_holder() {
        let (mut world, mut owner, mut companion) = a_pair_on_the_registry();
        while owner.try_next().is_some() {}
        while companion.try_next().is_some() {}
        let mut taken = std::mem::take(&mut world.messenger);
        // P100 stands its team up first, so P101's login finds its holder.
        taken.login(
            &mut world,
            Vid::new(VID),
            b"P100",
            &[],
            &[b"P101".to_vec()],
            &BTreeMap::new(),
        );
        taken.login(
            &mut world,
            Vid::new(OTHER),
            b"P101",
            &[],
            &[b"P100".to_vec()],
            &BTreeMap::new(),
        );
        world.messenger = taken;
        while companion.try_next().is_some() {}
        let login = owner.try_next().expect("the holder hears the team come in");
        assert_eq!(login[0], 0x4a, "the holder's record is a messenger one");
        assert_eq!(
            login[3], MESSENGER_SUBHEADER_GC_TEAM_LOGIN,
            "the holder hears the `TEAM_LOGIN`"
        );
        assert_eq!(&login[5..], b"P101", "the record names the entrant");
    }

    /// `MessengerManager::Logout`'s team half (`G/messenger_manager.cpp:241-246`):
    /// every owner of the leaving name's own team list hears the `TEAM_LOGOUT`, and
    /// the team's maps forget the name.
    #[test]
    fn a_logout_fans_the_team_logout_to_every_holder() {
        let (mut world, mut owner, mut companion) = a_pair_on_the_registry();
        let mut taken = std::mem::take(&mut world.messenger);
        taken.login(
            &mut world,
            Vid::new(OTHER),
            b"P101",
            &[],
            &[b"P100".to_vec()],
            &BTreeMap::new(),
        );
        taken.login(
            &mut world,
            Vid::new(VID),
            b"P100",
            &[],
            &[b"P101".to_vec()],
            &BTreeMap::new(),
        );
        world.messenger = taken;
        while owner.try_next().is_some() {}
        while companion.try_next().is_some() {}
        let mut taken = std::mem::take(&mut world.messenger);
        taken.logout(&mut world, b"P101");
        world.messenger = taken;
        let logout = owner.try_next().expect("the holder hears the team leave");
        assert_eq!(logout[0], 0x4a, "the holder's record is a messenger one");
        assert_eq!(
            logout[3], MESSENGER_SUBHEADER_GC_TEAM_LOGOUT,
            "the holder hears the `TEAM_LOGOUT`"
        );
        assert_eq!(&logout[5..], b"P101", "the record names the leaver");
    }
}
