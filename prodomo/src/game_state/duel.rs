//! The consent duel (`do_pvp`, `CPVPManager`).
//!
//! `do_pvp` (`G/cmd_general.cpp:848-879`) past its arena gates, and the duel it
//! challenges to (`CPVP`, `CPVPManager`, `G/pvp.cpp:17-342`, `:531-637`): the
//! challenge, the accept, the fight, and the revenge after a death.
//!
//! # What legacy does
//!
//! `/pvp <vid>` on a map of no arena, by a caller in no arena, finds the VID's
//! character: a miss and an NPC are silent, a victim in an arena hears `[LS;522]`,
//! and anyone else reaches `Insert`. `Insert` refuses the dead silently; on a pair
//! it already holds it agrees for the caller, and when both agree the fight begins
//! with its two lines; else it stores the pair, fans the `GC_PVP` agree record,
//! tells the challenge lines, and notifies the victim with a `WHISPER_TYPE_SYSTEM`
//! whisper carrying the challenge. The pair's key is the FNV-1 hash of its two
//! PIDs in numeric order, and the challenger agrees at the insert.
//!
//! A death in a fight (`Dead`, called from `Dead`, `G/char_battle.cpp:1280`) wins
//! the fight for the killer: the revenge flag rises, the loser may revenge, and
//! the fan carries the revenge mode. A death outside a fight within 15 seconds of
//! the last fight touch still counts as agreed. An agreed death skips the item-drop
//! penalty (`:1333-1335`) and the killer-alignment penalty (`:1412`). `Connect`
//! re-stamps the VIDs at enter; `Disconnect` is a commented-out no-op. `SendList`
//! replays every live pair to an entrant, with the companion re-send quirk, and
//! `Process` sweeps the pairs unfought for 10 minutes with the delete fan.
//!
//! # Divergences
//!
//! - **The PK-mode head of `CanAttack` is not this slice.** The duel tail feeds
//!   the policy's `consensual_fight` flag; the modes stay the port's own.
//! - **`GiveUp` is not ported.** Legacy calls it from nowhere.
//!
//! Everything here runs on the game thread only (ADR-0002).

use std::collections::HashMap;

use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_fields::{GcPvp, HEADER_GC_PVP};
use protocol::gc_whisper::{GcWhisper, WHISPER_TYPE_SYSTEM};

use super::GameState;
use crate::chat_line::{chat_text, Arg};
use crate::save::PASSES_PER_SEC;

/// `PVP_MODE_NONE` (`G/packet.h:1774`): the delete fan's mode.
const PVP_MODE_NONE: u8 = 0;
/// `PVP_MODE_AGREE` (`:1775`): a challenge waits.
const PVP_MODE_AGREE: u8 = 1;
/// `PVP_MODE_FIGHT` (`:1776`): both agreed.
const PVP_MODE_FIGHT: u8 = 2;
/// `PVP_MODE_REVENGE` (`:1777`): the loser may revenge.
const PVP_MODE_REVENGE: u8 = 3;

/// `get_dword_time() - GetLastFightTime() <= 15000` (`Dead`, `:361`): a death
/// outside a fight counts as agreed within 15 seconds of the last fight touch.
const AGREED_DEATH_WINDOW_PULSES: u64 = 15 * PASSES_PER_SEC as u64;
/// `get_dword_time() - GetLastFightTime() > 600000` (`Process`, `:625`): a pair
/// unfought for 10 minutes is swept.
const DUEL_SWEEP_AFTER_PULSES: u64 = 600 * PASSES_PER_SEC as u64;

/// `%s requested a duel. To accept, please agree to confrontation.` (`Insert`,
/// `:207`): the victim's challenge line, in the victim's language.
const CHALLENGE_LINE: &[u8] = b"%s requested a duel. To accept, please agree to confrontation.";
/// `You challenged %s to a duel.` (`:210`): the challenger's line.
const CHALLENGED_LINE: &[u8] = b"You challenged %s to a duel.";
/// `The confrontation with %s begins!` (`:186-187`): each side's line at the accept.
const BEGINS_LINE: &[u8] = b"The confrontation with %s begins!";

/// One side of a duel (`CPVP::TPlayer`, `G/pvp.h:20-30`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Duelist {
    /// `dwPID`: the store id of the player's row.
    pid: u32,
    /// `dwVID`: the live VID, 0 while disconnected.
    vid: u32,
    /// `bAgree`: the challenge's accept.
    agree: bool,
    /// `bCanRevenge`: the loser's revenge.
    can_revenge: bool,
}

/// One consent duel (`CPVP`, `G/pvp.h:10-51`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Duel {
    /// The two sides, the higher PID first (`CPVP::CPVP`, `:17-39`).
    sides: [Duelist; 2],
    /// `m_bRevenge`: a death ended the fight.
    revenge: bool,
    /// `m_dwLastFightTime`: the Pulse of the last fight touch.
    last_fight: u64,
}

impl Duel {
    /// The new pair of `challenger` and `victim`: the challenger agrees at the
    /// insert (`CPVP::CPVP`'s `bAgree = true`), the victim does not.
    fn challenge(
        challenger: u32,
        victim: u32,
        challenger_vid: u32,
        victim_vid: u32,
        pulse: u64,
    ) -> Self {
        let (first, second) = if challenger > victim {
            (
                Duelist {
                    pid: challenger,
                    vid: challenger_vid,
                    agree: true,
                    can_revenge: false,
                },
                Duelist {
                    pid: victim,
                    vid: victim_vid,
                    agree: false,
                    can_revenge: false,
                },
            )
        } else {
            (
                Duelist {
                    pid: victim,
                    vid: victim_vid,
                    agree: false,
                    can_revenge: false,
                },
                Duelist {
                    pid: challenger,
                    vid: challenger_vid,
                    agree: true,
                    can_revenge: false,
                },
            )
        };
        Self {
            sides: [first, second],
            revenge: false,
            last_fight: pulse,
        }
    }

    /// The FNV-1 key of the pair (`m_dwCRC`, `GetFastHash`, `G/crc32.cpp:123-136`):
    /// the two PIDs in numeric order as 8 little-endian bytes.
    fn key(one: u32, other: u32) -> u32 {
        let (high, low) = if one > other {
            (one, other)
        } else {
            (other, one)
        };
        let mut hash: u32 = 0;
        for byte in high.to_le_bytes().into_iter().chain(low.to_le_bytes()) {
            hash = hash.wrapping_mul(16_777_619) ^ u32::from(byte);
        }
        hash
    }

    /// `IsFight` (`:123-126`): both sides agree.
    fn is_fight(self) -> bool {
        self.sides[0].agree && self.sides[1].agree
    }

    /// The side of `pid` (`m_players[m_players[0].dwPID != dwPID ? 1 : 0]`).
    fn side(self, pid: u32) -> usize {
        usize::from(self.sides[0].pid != pid)
    }

    /// The companion's PID of `pid` (`Dead`'s `dwCompanionPID`, `:344-349`).
    fn companion(self, pid: u32) -> u32 {
        self.sides[1 - self.side(pid)].pid
    }

    /// The record the pair fans (`CPVP::Packet`, `:56-108`): the delete mode, the
    /// fight mode, or the revenge/agree mode from the agreeing side. `None` when
    /// either VID is null, as legacy sends nothing then.
    fn record(self, delete: bool) -> Option<GcPvp> {
        let [first, second] = self.sides;
        if first.vid == 0 || second.vid == 0 {
            return None;
        }
        if delete {
            return Some(GcPvp::new(first.vid, second.vid, PVP_MODE_NONE));
        }
        if self.is_fight() {
            return Some(GcPvp::new(first.vid, second.vid, PVP_MODE_FIGHT));
        }
        let mode = if self.revenge {
            PVP_MODE_REVENGE
        } else {
            PVP_MODE_AGREE
        };
        if first.agree {
            Some(GcPvp::new(first.vid, second.vid, mode))
        } else {
            Some(GcPvp::new(second.vid, first.vid, mode))
        }
    }
}

/// The live duels (`CPVPManager::m_map_pkPVP`, `G/pvp.h:92`), keyed by the pair's
/// FNV-1 hash. The by-PID index is a scan: duels are few, and the scan keeps one
/// map to hold right.
#[derive(Debug, Default)]
pub(super) struct Duels {
    /// The live pairs by their [`Duel::key`].
    pairs: HashMap<u32, Duel>,
}

impl GameState {
    /// Runs one `/pvp` line (`do_pvp`, `G/cmd_general.cpp:848-879`): the word's VID
    /// (`str_to_number(DWORD)`), silent on a miss or an NPC, and the duel's
    /// challenge or accept. A caller with no body runs nothing.
    pub(super) fn run_pvp(&mut self, vid: Vid, rest: &[u8]) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        if self.in_arena(vid)
            || self
                .place_of(vid.raw())
                .is_some_and(|(_, map)| self.is_arena_map(map))
        {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;403]", &[]);
            return;
        }
        let (word, _) = crate::command::one_argument(rest);
        let victim = Vid::new(super::skill_admin::str_to_dword(&word));
        let Ok(character) = self.characters.find_by_vid(victim) else {
            return;
        };
        if character.kind() != world::character::CharacterKind::Player {
            return;
        }
        if self.in_arena(victim) {
            if let Some(mover) = self.own_mover(victim) {
                self.tell(victim, mover, CHAT_TYPE_INFO, b"[LS;522]", &[]);
            }
            return;
        }
        self.pvp_insert(vid, victim);
    }

    /// `CPVPManager::Insert` (`G/pvp.cpp:172-232`): the dead are silent; a held pair
    /// agrees for the caller and begins the fight with its two lines; else the pair
    /// is stored, fanned, and the challenge lines and the victim's whisper go out.
    fn pvp_insert(&mut self, challenger: Vid, victim: Vid) {
        let (challenger_pid, victim_pid) = (self.pvp_pid_of(challenger), self.pvp_pid_of(victim));
        if self.is_dead(challenger.raw()) || self.is_dead(victim.raw()) {
            return;
        }
        let key = Duel::key(challenger_pid, victim_pid);
        if self.duels.pairs.contains_key(&key) {
            if self.pvp_agree(key, challenger_pid) {
                self.tell_duel_begins(challenger, victim);
            }
            return;
        }
        let pulse = self.last_pulse;
        self.duels.pairs.insert(
            key,
            Duel::challenge(
                challenger_pid,
                victim_pid,
                challenger.raw(),
                victim.raw(),
                pulse,
            ),
        );
        self.fan_duel(key, false);
        self.tell_duel_challenge(challenger, victim);
    }

    /// `CPVP::Agree` (`:110-120`): the caller's accept, fanning when the fight begins.
    /// Returns whether the fight began.
    fn pvp_agree(&mut self, key: u32, pid: u32) -> bool {
        let fight = {
            let Some(duel) = self.duels.pairs.get_mut(&key) else {
                return false;
            };
            let side = duel.side(pid);
            duel.sides[side].agree = true;
            duel.is_fight()
        };
        if fight {
            self.fan_duel(key, false);
        }
        fight
    }

    /// The fight's two `The confrontation with %s begins!` lines (`:186-187`), each in
    /// its own language: the victim's first, as legacy tells it.
    fn tell_duel_begins(&self, challenger: Vid, victim: Vid) {
        let challenger_name = self.name_of(challenger);
        let victim_name = self.name_of(victim);
        if let Some(mover) = self.own_mover(victim) {
            self.tell(
                victim,
                mover,
                CHAT_TYPE_INFO,
                BEGINS_LINE,
                &[Arg::Text(&challenger_name)],
            );
        }
        if let Some(mover) = self.own_mover(challenger) {
            self.tell(
                challenger,
                mover,
                CHAT_TYPE_INFO,
                BEGINS_LINE,
                &[Arg::Text(&victim_name)],
            );
        }
    }

    /// The challenge's lines (`:204-210`) and the victim's `WHISPER_TYPE_SYSTEM`
    /// notify (`:212-231`): the rendered challenge with its NUL, from the challenger.
    fn tell_duel_challenge(&self, challenger: Vid, victim: Vid) {
        let challenger_name = self.name_of(challenger);
        let victim_name = self.name_of(victim);
        if let Some(mover) = self.own_mover(victim) {
            self.tell(
                victim,
                mover,
                CHAT_TYPE_INFO,
                CHALLENGE_LINE,
                &[Arg::Text(&challenger_name)],
            );
        }
        if let Some(mover) = self.own_mover(challenger) {
            self.tell(
                challenger,
                mover,
                CHAT_TYPE_INFO,
                CHALLENGED_LINE,
                &[Arg::Text(&victim_name)],
            );
        }
        let Some(mover) = self.own_mover(victim) else {
            return;
        };
        let mut text = chat_text(
            mover.recipient(&self.locale),
            CHAT_TYPE_INFO,
            CHALLENGE_LINE,
            &[Arg::Text(&challenger_name)],
        );
        text.push(0);
        let record = GcWhisper {
            whisper_type: WHISPER_TYPE_SYSTEM,
            name_from: crate::loading_phase::name_bytes_field(&challenger_name),
            can_format: true,
            text,
        };
        let Ok(bytes) = record.encode() else {
            return;
        };
        let _sent = self.write_to_client(victim, bytes);
    }

    /// `CPVP::Packet` (`:56-108`): the pair's record to every body in the world.
    /// Legacy fans to the descriptors in `PHASE_GAME` or `PHASE_DEAD`; the bodies
    /// are exactly those, as a loading descriptor has an outbox but no body yet,
    /// and a dead player keeps its body.
    fn fan_duel(&self, key: u32, delete: bool) {
        let Some(duel) = self.duels.pairs.get(&key) else {
            return;
        };
        let Some(record) = duel.record(delete) else {
            return;
        };
        let mut line = Vec::with_capacity(10);
        record.encode_into(&mut line);
        debug_assert_eq!(line[0], HEADER_GC_PVP);
        let mut vids: Vec<Vid> = self.bodies.keys().copied().collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            let _sent = self.write_to_client(vid, line.clone());
        }
    }

    /// `CPVPManager::Dead` (`:335-376`): whether the victim's death at the killer's
    /// hands is an agreed one. A death in a fight wins it for the killer (`Win`,
    /// `:128-139`: the revenge rises, the loser may revenge and unagrees, and the
    /// fan carries the revenge mode) with a fresh fight touch; a death outside a
    /// fight counts within 15 seconds of the last touch. `None` killer PID (0)
    /// never agrees: no companion holds PID 0.
    pub(super) fn pvp_dead(&mut self, victim: Vid, killer_pid: u32) -> bool {
        let victim_pid = self.pvp_pid_of(victim);
        let key = self
            .duels
            .pairs
            .iter()
            .find(|(_, duel)| {
                (duel.sides[0].pid == victim_pid || duel.sides[1].pid == victim_pid)
                    && duel.companion(victim_pid) == killer_pid
            })
            .map(|(key, _)| *key);
        let Some(key) = key else {
            return false;
        };
        let pulse = self.last_pulse;
        {
            let Some(duel) = self.duels.pairs.get_mut(&key) else {
                return false;
            };
            if !duel.is_fight() {
                return pulse.saturating_sub(duel.last_fight) <= AGREED_DEATH_WINDOW_PULSES;
            }
            duel.last_fight = pulse;
            duel.revenge = true;
            let winner = duel.side(killer_pid);
            duel.sides[winner].agree = true;
            duel.sides[1 - winner].can_revenge = true;
            duel.sides[1 - winner].agree = false;
        }
        self.fan_duel(key, false);
        true
    }

    /// Whether the two PIDs hold a live fight (`CanAttack`'s duel tail,
    /// `:508-528`): the pair exists and both agree, with a fresh fight touch.
    /// Called from the PK policy's `consensual_fight` read.
    pub(super) fn pvp_is_fight(&mut self, one: u32, other: u32) -> bool {
        let key = Duel::key(one, other);
        let fight = self
            .duels
            .pairs
            .get(&key)
            .is_some_and(|duel| duel.is_fight());
        if fight {
            let pulse = self.last_pulse;
            if let Some(duel) = self.duels.pairs.get_mut(&key) {
                duel.last_fight = pulse;
            }
        }
        fight
    }

    /// `CPVPManager::IsFighting(pid)` (`G/pvp.cpp:244-260`): any live pair holding `pid`
    /// whose both sides agreed. Read-only: unlike [`Self::pvp_is_fight`], it stamps no
    /// `last_fight`.
    pub(super) fn pvp_is_fighting(&self, pid: u32) -> bool {
        self.duels
            .pairs
            .values()
            .any(|duel| duel.is_fight() && (duel.sides[0].pid == pid || duel.sides[1].pid == pid))
    }

    /// `CPVPManager::Connect` (`:282-285`): the entrant's live pairs re-stamp its
    /// VID. `Disconnect` (`:287-290`) is a commented-out no-op, so leaving stamps
    /// nothing.
    pub(super) fn pvp_connect(&mut self, vid: Vid) {
        let pid = self.pvp_pid_of(vid);
        for duel in self.duels.pairs.values_mut() {
            if duel.sides[0].pid == pid || duel.sides[1].pid == pid {
                let side = duel.side(pid);
                duel.sides[side].vid = vid.raw();
            }
        }
    }

    /// `CPVPManager::SendList` (`:555-617`): every live pair's record to the entrant,
    /// then the companion's copy when the entrant is one side of the pair. Pairs
    /// with a null VID send nothing, as the fan sends nothing.
    pub(super) fn pvp_send_list(&self, vid: Vid) {
        let mut keys: Vec<u32> = self.duels.pairs.keys().copied().collect();
        keys.sort_unstable();
        for key in keys {
            let Some(duel) = self.duels.pairs.get(&key) else {
                continue;
            };
            let Some(record) = duel.record(false) else {
                continue;
            };
            let mut line = Vec::with_capacity(10);
            record.encode_into(&mut line);
            let _sent = self.write_to_client(vid, line.clone());
            let companion = if duel.sides[0].vid == vid.raw() {
                Some(duel.sides[1].vid)
            } else if duel.sides[1].vid == vid.raw() {
                Some(duel.sides[0].vid)
            } else {
                None
            };
            if let Some(other) = companion.filter(|other| *other != 0) {
                let _sent = self.write_to_client(Vid::new(other), line);
            }
        }
    }

    /// `CPVPManager::Process` (`:619-637`): the pairs unfought for 10 minutes are
    /// fanned deleted and dropped.
    pub(super) fn run_duel_sweep(&mut self, pulse: u64) {
        let stale: Vec<u32> = self
            .duels
            .pairs
            .iter()
            .filter(|(_, duel)| pulse.saturating_sub(duel.last_fight) > DUEL_SWEEP_AFTER_PULSES)
            .map(|(key, _)| *key)
            .collect();
        for key in stale {
            self.fan_duel(key, true);
            self.duels.pairs.remove(&key);
        }
    }

    /// The store id of the player online under `vid`, 0 when nobody is there. No
    /// row holds 0, so a miss never matches a companion.
    pub(super) fn pvp_pid_of(&self, vid: Vid) -> u32 {
        self.characters
            .find_by_vid(vid)
            .map(world::character::Character::player_id)
            .unwrap_or(0)
    }

    /// The character Name bytes of the character under `vid`, empty when nobody is there.
    fn name_of(&self, vid: Vid) -> Vec<u8> {
        self.characters
            .find_by_vid(vid)
            .map(|character| Vec::from(character.name().as_bytes()))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use common::vid::Vid;
    use tokio::sync::mpsc::UnboundedReceiver;

    use super::super::fixtures::{a_world, enter, TestClock};
    use super::super::GameState;
    use super::{Duel, AGREED_DEATH_WINDOW_PULSES, DUEL_SWEEP_AFTER_PULSES};

    const ALICE: u32 = 100;
    const BOB: u32 = 200;

    /// A world with Alice and Bob past the enter queue.
    fn a_world_with_two() -> (
        GameState,
        UnboundedReceiver<Vec<u8>>,
        UnboundedReceiver<Vec<u8>>,
    ) {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut alice = enter(&mut world, ALICE, (3200, 3200), 0);
        let mut bob = enter(&mut world, BOB, (3300, 3300), 0);
        drain(&mut alice);
        drain(&mut bob);
        (world, alice, bob)
    }

    /// Drain what one inbox holds.
    fn drain(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// The `GC_PVP` records among the lines.
    fn pvp_of(lines: &[Vec<u8>]) -> Vec<Vec<u8>> {
        lines
            .iter()
            .filter(|line| line[0] == super::HEADER_GC_PVP)
            .cloned()
            .collect()
    }

    /// The chat text of one `HEADER_GC_CHAT` frame.
    fn text_of(line: &[u8]) -> Vec<u8> {
        line[10..].to_vec()
    }
    fn install_arena(world: &mut GameState) {
        world.load_arenas(&[quest::host::Arena {
            map_index: super::super::fixtures::PLACE.1,
            start_a: (32, 32),
            start_b: (33, 33),
        }]);
    }

    fn refusal(world: &GameState, vid: Vid, text: &[u8]) -> Vec<u8> {
        crate::chat_line::chat_packet(
            world.own_mover(vid).unwrap().recipient(&world.locale),
            protocol::gc_chat::CHAT_TYPE_INFO,
            text,
            &[],
        )
    }

    #[test]
    fn arena_map_refuses_pvp_before_binding_but_allows_emotion_consent() {
        let (mut world, mut alice, mut bob) = a_world_with_two();
        install_arena(&mut world);
        let caller = Vid::new(ALICE);
        assert!(!world.in_arena(caller));
        world.run_pvp(caller, b"missing");
        assert_eq!(drain(&mut alice), [refusal(&world, caller, b"[LS;403]")]);
        assert!(drain(&mut bob).is_empty());
        assert!(world.duels.pairs.is_empty());
        world.allow_emotion(caller, b"200");
        assert!(world.emotion_consents.contains(&(ALICE, BOB)));
        assert!(drain(&mut alice).is_empty());
    }

    #[test]
    fn bound_arena_callers_refuse_emotion_party_and_messenger_without_consuming_requests() {
        let (mut world, mut alice, mut bob) = a_world_with_two();
        install_arena(&mut world);
        let caller = Vid::new(ALICE);
        let guest = Vid::new(BOB);
        world.answer_messenger_add_by_vid(guest, caller);
        assert!(world.messenger.waits(&world, caller, b"P200"));
        world.start_arena(((0, 41), 0), [caller, guest], 3, 5);
        world.run_arena_ticks(250);
        drain(&mut alice);
        drain(&mut bob);
        assert!(world.in_arena(caller));
        world.allow_emotion(caller, b"200");
        world.party_invite(caller, guest);
        world.party_invite_answer(caller, Vid::new(999), 0);
        world.party_request(caller, b"");
        world.party_remove(caller, BOB);
        let (reply, mut answer) = tokio::sync::oneshot::channel();
        world.answer_messenger_auth(caller, b"P200", false, reply);
        assert_eq!(answer.try_recv(), Ok(false));
        assert_eq!(
            drain(&mut alice),
            vec![refusal(&world, caller, b"[LS;403]"); 6]
        );
        assert!(drain(&mut bob).is_empty());
        assert!(!world.emotion_consents.contains(&(ALICE, BOB)));
        assert!(world.party_leader_of(ALICE).is_none());
        assert!(world.messenger.waits(&world, caller, b"P200"));
    }

    #[test]
    fn pvp_arena_victim_is_notified_instead_of_the_nonmember_challenger() {
        let (mut world, mut alice, mut bob) = a_world_with_two();
        install_arena(&mut world);
        world.start_arena(((0, 41), 0), [Vid::new(ALICE), Vid::new(BOB)], 3, 5);
        world.run_arena_ticks(250);
        let mut third = enter(&mut world, 300, (3400, 3400), 0);
        let challenger = Vid::new(300);
        // The caller gate only reads its arena pointer or map; the victim's map
        // is not inspected, so a remote VID still reaches the victim-pointer gate.
        world.bodies.get_mut(&challenger).unwrap().map = 42;
        drain(&mut alice);
        drain(&mut bob);
        drain(&mut third);
        world.run_pvp(challenger, b"100");
        assert!(drain(&mut third).is_empty());
        assert!(drain(&mut bob).is_empty());
        assert_eq!(
            drain(&mut alice),
            [refusal(&world, Vid::new(ALICE), b"[LS;522]")]
        );
        assert!(world.duels.pairs.is_empty());
    }

    /// The challenge stores the pair with the challenger agreed, fans the agree
    /// record from the agreeing side, and tells the challenge lines with the
    /// victim's whisper notify.
    #[test]
    fn a_challenge_stores_fans_and_tells() {
        let (mut world, mut alice, mut bob) = a_world_with_two();
        world.run_pvp(Vid::new(ALICE), BOB.to_string().as_bytes());
        let key = Duel::key(ALICE, BOB);
        let duel = world.duels.pairs.get(&key).expect("the pair is stored");
        assert!(duel.sides[duel.side(ALICE)].agree, "the challenger agrees");
        assert!(!duel.sides[duel.side(BOB)].agree, "the victim does not");
        // Bob's PID is higher, so Bob is first but Alice agrees: the fan runs
        // from the agreeing side.
        let fan = protocol::gc_fields::GcPvp::new(ALICE, BOB, super::PVP_MODE_AGREE);
        let mut want = Vec::new();
        fan.encode_into(&mut want);
        let alice_lines = drain(&mut alice);
        let bob_lines = drain(&mut bob);
        assert_eq!(pvp_of(&alice_lines), [want.clone()]);
        assert_eq!(pvp_of(&bob_lines), [want]);
        assert!(
            bob_lines.iter().any(|line| line[0] == 4
                && text_of(line)
                    == b"P100 requested a duel. To accept, please agree to confrontation."),
            "the victim hears the challenge"
        );
        assert!(
            alice_lines
                .iter()
                .any(|line| line[0] == 4 && text_of(line) == b"You challenged P200 to a duel."),
            "the challenger hears its line"
        );
        assert!(
            bob_lines
                .iter()
                .any(|line| line[0] == protocol::gc_inventory::HEADER_GC_WHISPER.value()),
            "the victim hears the whisper notify"
        );
    }

    /// The victim's `/pvp` back agrees for it: the fight begins with its fan and
    /// its two lines, the victim's first.
    #[test]
    fn an_accept_begins_the_fight() {
        let (mut world, mut alice, mut bob) = a_world_with_two();
        world.run_pvp(Vid::new(ALICE), BOB.to_string().as_bytes());
        drain(&mut alice);
        drain(&mut bob);
        world.run_pvp(Vid::new(BOB), ALICE.to_string().as_bytes());
        let key = Duel::key(ALICE, BOB);
        assert!(world.duels.pairs[&key].is_fight(), "both agree");
        let fan = protocol::gc_fields::GcPvp::new(BOB, ALICE, super::PVP_MODE_FIGHT);
        let mut want = Vec::new();
        fan.encode_into(&mut want);
        let alice_lines = drain(&mut alice);
        let bob_lines = drain(&mut bob);
        assert_eq!(pvp_of(&alice_lines), [want.clone()]);
        assert_eq!(pvp_of(&bob_lines), [want]);
        assert!(
            bob_lines.iter().any(|line| line[0] == 4 && text_of(line) == b"The confrontation with P100 begins!"),
            "the victim hears the begins line"
        );
        assert!(
            alice_lines.iter().any(|line| line[0] == 4 && text_of(line) == b"The confrontation with P200 begins!"),
            "the challenger hears the begins line"
        );
    }

    /// A miss, an NPC, a wordless line and the dead are silent: no pair, no fan,
    /// no line.
    #[test]
    fn a_miss_an_npc_and_the_dead_are_silent() {
        let (mut world, mut alice, mut bob) = a_world_with_two();
        world.run_pvp(Vid::new(ALICE), b"999");
        world.run_pvp(Vid::new(ALICE), b"");
        world.run_pvp(Vid::new(ALICE), b"nope");
        assert!(world.duels.pairs.is_empty(), "misses store nothing");
        assert!(drain(&mut alice).is_empty());
        assert!(drain(&mut bob).is_empty());
        // The dead challenge nothing.
        world.run_pvp(Vid::new(ALICE), BOB.to_string().as_bytes());
        drain(&mut alice);
        drain(&mut bob);
        world.dead_pc(Vid::new(ALICE), None);
        drain(&mut alice);
        world.run_pvp(Vid::new(ALICE), BOB.to_string().as_bytes());
        let key = Duel::key(ALICE, BOB);
        assert!(
            !world.duels.pairs[&key].is_fight(),
            "the dead agree to nothing"
        );
    }

    /// A death in a fight wins it for the killer: the revenge rises, the loser may
    /// revenge and unagrees, the fan carries revenge, and the death is agreed. A
    /// death outside a fight counts within 15 seconds of the last touch, and not
    /// after.
    #[test]
    fn a_death_in_a_fight_wins_and_counts_as_agreed() {
        let (mut world, mut alice, mut bob) = a_world_with_two();
        world.run_pvp(Vid::new(ALICE), BOB.to_string().as_bytes());
        world.run_pvp(Vid::new(BOB), ALICE.to_string().as_bytes());
        drain(&mut alice);
        drain(&mut bob);
        assert!(
            world.pvp_dead(Vid::new(BOB), ALICE),
            "the fight's death agrees"
        );
        let key = Duel::key(ALICE, BOB);
        let duel = world.duels.pairs[&key];
        assert!(duel.revenge, "the revenge rises");
        assert!(
            duel.sides[duel.side(BOB)].can_revenge,
            "the loser may revenge"
        );
        assert!(!duel.sides[duel.side(BOB)].agree, "the loser unagrees");
        // The winner agrees and the loser unagrees, so the fan runs from the
        // agreeing side: Alice first in the record, Bob second.
        let fan = protocol::gc_fields::GcPvp::new(ALICE, BOB, super::PVP_MODE_REVENGE);
        let mut want = Vec::new();
        fan.encode_into(&mut want);
        let alice_lines = drain(&mut alice);
        let bob_lines = drain(&mut bob);
        assert_eq!(pvp_of(&alice_lines), [want.clone()]);
        assert_eq!(pvp_of(&bob_lines), [want]);
        // Outside a fight: within the window it counts, past it it does not.
        world.last_pulse += AGREED_DEATH_WINDOW_PULSES;
        assert!(
            world.pvp_dead(Vid::new(BOB), ALICE),
            "within 15 seconds it counts"
        );
        world.last_pulse += 1;
        assert!(
            !world.pvp_dead(Vid::new(BOB), ALICE),
            "past 15 seconds it does not"
        );
        // No pair, and a stranger's kill, never agree.
        assert!(
            !world.pvp_dead(Vid::new(ALICE), 777),
            "a stranger never agrees"
        );
    }

    /// The sweep fans deleted and drops the pairs unfought for 10 minutes, and
    /// keeps the fresh ones.
    #[test]
    fn the_sweep_drops_stale_pairs() {
        let (mut world, mut alice, mut bob) = a_world_with_two();
        world.run_pvp(Vid::new(ALICE), BOB.to_string().as_bytes());
        let key = Duel::key(ALICE, BOB);
        let pulse = world.last_pulse + DUEL_SWEEP_AFTER_PULSES;
        world.run_duel_sweep(pulse);
        assert!(
            world.duels.pairs.contains_key(&key),
            "at 10 minutes it keeps"
        );
        world.run_duel_sweep(pulse + 1);
        assert!(world.duels.pairs.is_empty(), "past 10 minutes it drops");
        let fan = protocol::gc_fields::GcPvp::new(BOB, ALICE, super::PVP_MODE_NONE);
        let mut want = Vec::new();
        fan.encode_into(&mut want);
        assert!(pvp_of(&drain(&mut alice)).contains(&want));
        assert!(pvp_of(&drain(&mut bob)).contains(&want));
    }

    /// The enter re-stamps the entrant's VID into its pairs, and the list replays
    /// every live pair to the entrant with the companion's copy.
    #[test]
    fn connect_restamps_and_send_list_replays() {
        let (mut world, mut alice, mut bob) = a_world_with_two();
        world.run_pvp(Vid::new(ALICE), BOB.to_string().as_bytes());
        drain(&mut alice);
        drain(&mut bob);
        let key = Duel::key(ALICE, BOB);
        world.duels.pairs.get_mut(&key).expect("the pair").sides[1].vid = 0;
        world.pvp_connect(Vid::new(ALICE));
        let duel = world.duels.pairs[&key];
        assert_eq!(duel.sides[duel.side(ALICE)].vid, ALICE, "the VID re-stamps");
        world.pvp_send_list(Vid::new(BOB));
        let fan = protocol::gc_fields::GcPvp::new(ALICE, BOB, super::PVP_MODE_AGREE);
        let mut want = Vec::new();
        fan.encode_into(&mut want);
        assert_eq!(
            pvp_of(&drain(&mut bob)),
            [want.clone()],
            "the entrant hears the list"
        );
        assert_eq!(
            pvp_of(&drain(&mut alice)),
            [want],
            "the companion hears its copy"
        );
    }

    /// A live fight answers the PK policy with a fresh touch; without one it does not.
    #[test]
    fn a_live_fight_answers_the_policy() {
        let (mut world, _, _) = a_world_with_two();
        assert!(!world.pvp_is_fight(ALICE, BOB), "no pair is no fight");
        world.run_pvp(Vid::new(ALICE), BOB.to_string().as_bytes());
        assert!(!world.pvp_is_fight(ALICE, BOB), "a challenge is no fight");
        world.run_pvp(Vid::new(BOB), ALICE.to_string().as_bytes());
        let key = Duel::key(ALICE, BOB);
        world
            .duels
            .pairs
            .get_mut(&key)
            .expect("the pair")
            .last_fight = 0;
        world.last_pulse = 50;
        assert!(world.pvp_is_fight(ALICE, BOB), "the fight answers");
        assert_eq!(world.duels.pairs[&key].last_fight, 50, "the touch is fresh");
    }
}
