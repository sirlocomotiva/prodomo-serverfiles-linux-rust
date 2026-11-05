//! The two GM log lines: `/detaillog` and `/monsterlog` (`do_detaillog` and
//! `do_monsterlog`, `G/cmd_general.cpp:2242-2250`).
//!
//! `DetailLog` flips the caller's bit (`G/char.h:1735`); `ToggleMonsterLog` flips the
//! caller's bit and subscribes or unsubscribes it (`G/char.cpp:7495-7507`). Both are silent.
//! The destructor unsubscribes a subscriber (`G/char.cpp:791-792`), which the leave does here.
//!
//! # What the flags gate
//!
//! `m_bDetailLog` gates two branches. `Move` logs the point server-side (`G/char.cpp:3616-3618`),
//! which has no wire, and `Damage` tells the victim the attacker's name, VID and point
//! (`G/char_battle.cpp:2367-2371`), which is this module's `log_pc_damage`.
//!
//! `m_bMonsterLog` gates nothing by itself: `CHARACTER::MonsterLog` (`G/char.cpp:5095-5138`)
//! speaks under `test_server`, for a non-player only, as a talking `GC_CHAT` with the
//! speaker's VID whose text is `%u)` plus the line, cut to `CHAT_MAX_LEN` with its
//! terminator. `PacketMonsterLog` fans it to the subscribers within 6000 of the speaker
//! (`G/char_manager.cpp:825-841`). The call sites this build reaches are the move's chase
//! line, the walk-mode pair, the victim pair, the state pulse's line, the idle follow's
//! line, the move end's pair, the far victim's line, the regen's line, and the position's
//! pair; the summoner's call and the Meley-boss exemption wait for their systems, the idle
//! stone's draws are unlogged, and the `test_server` damage pair and the revive read no
//! port yet. `SetNextStatePulse` logs in `test_server` whatever the pulse, so the guard's
//! 12-Pulse ask logs too.
//!
//! Everything here runs on the game thread only (ADR-0002).
/// The victim `Damage`'s detail line names (`G/char_battle.cpp:2367-2371`): the attacker's
/// name, VID and point. The format is `LC_TEXT`, so the recipient's language looks it up.
const DAMAGE_DETAIL: &[u8] = b"%s[%d]\xef\xbf\xbd\xef\xbf\xbd \xef\xbf\xbd\xef\xbf\xbd\xef\xbf\xbd\xef\xbf\xbd \xef\xbf\xbd\xef\xbf\xbd\xc4\xa1: %d %d";
/// `MonsterLog("[MOVE] %s", ...)` (`G/char.cpp:3498`).
pub(super) const LOG_MOVE_TRACKING: &[u8] = b"[MOVE] target tracking";
/// `MonsterLog("[MOVE] %s", ...)` (`G/char.cpp:3498`).
pub(super) const LOG_MOVE_JUST: &[u8] = b"[MOVE] just move";
/// `MonsterLog("Let's go to the next state")` (`G/char.cpp:5356`).
pub(super) const LOG_NEXT_STATE: &[u8] = b"Let's go to the next state";
/// `MonsterLog("걷는다")` (`G/char.cpp:7373`).
pub(super) const LOG_WALKING: &[u8] = b"\xea\xb1\xb7\xeb\x8a\x94\xeb\x8b\xa4";
/// `MonsterLog("뛴다")` (`G/char.cpp:7375`).
pub(super) const LOG_RUNNING: &[u8] = b"\xeb\x9b\xb4\xeb\x8b\xa4";
/// `MonsterLog("!")` (`G/char.cpp:7530`).
pub(super) const LOG_BEGIN_EMPTY: &[u8] = b"!";
/// `MonsterLog("[BATTLE] fighting state")` (`G/char.cpp:1455`).
pub(super) const LOG_BATTLE_FIGHTING: &[u8] = b"[BATTLE] fighting state";
/// `MonsterLog("[IDLE] restarting state")` (`G/char.cpp:1462`).
pub(super) const LOG_IDLE_RESTARTING: &[u8] = b"[IDLE] restarting state";
/// `MonsterLog("???????????")` (`G/char_battle.cpp:3416`).
pub(super) const LOG_VICTIM_LOST: &[u8] = b"???????????";
/// `MonsterLog("Set attack target: %s", ...)` (`G/char_battle.cpp:3424`).
pub(super) const LOG_SET_ATTACK_TARGET: &[u8] = b"Set attack target: ";
/// `MonsterLog("[IDLE] ...")` (`G/char_state.cpp:713`).
pub(super) const LOG_IDLE_TOO_FAR: &[u8] = b"[IDLE] \xb8\xae\xb4\xf5\xb7\xce\xba\xce\xc5\xcd \xb3\xca\xb9\xab \xb8\xd6\xb8\xae \xb6\xb3\xbe\xee\xc1\xb3\xb4\xd9! \xba\xb9\xb1\xcd\xc7\xd1\xb4\xd9.";
/// `MonsterLog("[BATTLE] ... %s", ...)` (`G/char_state.cpp:910`).
pub(super) const LOG_BATTLE_APPROACH: &[u8] =
    b"[BATTLE] \xb1\xd9\xc3\xb3\xbf\xa1 \xbf\xd4\xc0\xb8\xb4\xcf \xb0\xf8\xb0\xdd\xbd\xc3\xc0\xdb ";
/// `MonsterLog("[IDLE] ...")` (`G/char_state.cpp:918`).
pub(super) const LOG_IDLE_LOITERING: &[u8] =
    b"[IDLE] \xb4\xeb\xbb\xf3\xc0\xcc \xbe\xf8\xc0\xb8\xb4\xcf \xbd\xac\xc0\xda";
/// `MonsterLog("...")` (`G/char_state.cpp:1081`).
pub(super) const LOG_TARGET_LOST_FAR: &[u8] =
    b"\xc5\xb8\xb0\xd9\xc0\xcc \xb8\xd6\xbe\xee\xbc\xad \xc6\xf7\xb1\xe2";

use common::vid::Vid;
use protocol::gc_chat::{GcChat, CHAT_MAX_LEN, CHAT_TYPE_INFO, CHAT_TYPE_TALKING};

use super::GameState;
use crate::chat_line::Arg;

/// `6000`: the fan's reach (`G/char_manager.cpp:833`).
const MONSTER_LOG_REACH: i32 = 6000;

impl GameState {
    /// `DetailLog` (`G/char.h:1735`): the caller's detail flag flips. Silent.
    pub(super) fn toggle_detail_log(&mut self, vid: Vid) {
        if !self.detail_logs.remove(&vid) {
            self.detail_logs.insert(vid);
        }
    }

    /// `ToggleMonsterLog` (`G/char.cpp:7495-7507`): the caller's monster subscription flips.
    /// Silent.
    pub(super) fn toggle_monster_log(&mut self, vid: Vid) {
        if !self.monster_logs.remove(&vid) {
            self.monster_logs.insert(vid);
        }
    }
    /// Forgets the leaving character's log flags, as the destructor unsubscribes it
    /// (`G/char.cpp:791-792`).
    pub(super) fn forget_logs(&mut self, vid: Vid) {
        let _detail = self.detail_logs.remove(&vid);
        let _monster = self.monster_logs.remove(&vid);
    }

    /// `CHARACTER::MonsterLog` (`G/char.cpp:5095-5138`) of the non-player `speaker`, fanned
    /// by `PacketMonsterLog` (`G/char_manager.cpp:825-841`): under `test_server`, a talking
    /// `GC_CHAT` with the speaker's VID and `%u)` plus `line` reaches every subscriber within
    /// 6000 of it. A subscriber on another map, or at a point no sectree holds, hears nothing:
    /// legacy reads both characters' points from the same sectree manager.
    pub(super) fn monster_log(&self, speaker: u32, line: &[u8]) {
        if !self.test_server {
            return;
        }
        let Some(at) = self.npc_spot(speaker) else {
            return;
        };
        let mut text = format!("{speaker})").into_bytes();
        text.extend_from_slice(line);
        if text.len() > CHAT_MAX_LEN {
            text.truncate(CHAT_MAX_LEN);
        }
        // `\0 문자 포함`: the text goes with its terminator.
        text.push(0);
        let chat = GcChat {
            chat_type: CHAT_TYPE_TALKING,
            id: speaker,
            empire: 0,
            can_format: true,
            text,
        };
        let Ok(record) = chat.encode() else {
            return;
        };
        let mut hearers: Vec<Vid> = self.monster_logs.iter().copied().collect();
        hearers.sort_by_key(|hearer| hearer.raw());
        for hearer in hearers {
            let Some(spot) = self.spot_of(hearer.raw()) else {
                continue;
            };
            let away = crate::sync_position::distance_approx(
                spot.x.saturating_sub(at.0),
                spot.y.saturating_sub(at.1),
            );
            if away > MONSTER_LOG_REACH {
                continue;
            }
            let _sent = self.write_to_client(hearer, record.clone());
        }
    }

    /// `Damage`'s detail line (`G/char_battle.cpp:2367-2371`) to the player `victim`: the
    /// attacker's name, VID and point. Only a flagged victim hears it.
    pub(super) fn log_pc_damage(&self, attacker: Vid, victim: Vid) {
        if !self.detail_logs.contains(&victim) {
            return;
        }
        let (Some(mover), Ok(character), Some(spot)) = (
            self.own_mover(victim),
            self.characters.find_by_vid(attacker),
            self.spot_of(attacker.raw()),
        ) else {
            return;
        };
        let name = character.name().as_bytes();
        self.tell(
            victim,
            mover,
            CHAT_TYPE_INFO,
            DAMAGE_DETAIL,
            &[
                Arg::Text(name),
                Arg::Int(i64::from(attacker.raw())),
                Arg::Int(i64::from(spot.x)),
                Arg::Int(i64::from(spot.y)),
            ],
        );
    }

    /// `Damage`'s detail line (`G/char_battle.cpp:2367-2371`) to the player `attacker`, whose
    /// victim is the non-player `victim`: the attacker's own name, VID and point. Only a
    /// flagged attacker hears it: the flag lives on the victim, and a non-player has none,
    /// so the hitter's own flag gates it.
    pub(super) fn log_npc_damage(&self, attacker: Vid, _victim: u32) {
        if !self.detail_logs.contains(&attacker) {
            return;
        }
        self.log_pc_damage(attacker, attacker);
    }
}

#[cfg(test)]
mod tests {
    use common::point_slot as point;
    use gamedata::mob_proto::CHAR_TYPE_MONSTER;
    use protocol::gc_chat::{GcChat, CHAT_TYPE_INFO};
    use protocol::gc_inventory::HEADER_GC_CHAT;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::npc::FIRST_NPC_VID;

    use super::super::fixtures::{a_fighter, enter, open_cells, stand_field, TestClock, PLACE};
    use super::*;
    use crate::game_loop::PulseProcessor;

    /// The monster every test watches.
    const ME: u32 = FIRST_NPC_VID;

    /// Where it stands.
    const SPOT: (i32, i32) = (3200, 3200);

    /// 100 west of it: within the fan's reach.
    const NEAR: (i32, i32) = (3100, 3200);

    /// Past the fan's reach (`DISTANCE_APPROX` reads 7000 as 6718).
    const FAR: (i32, i32) = (3200 + 7000, 3200);

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// `a_fighter` 101 at [`SPOT`] and the player 1 [`NEAR`], subscribed, after Pulse 1;
    /// the inbox is the player's, drained.
    fn a_watch() -> (TestClock, GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        clock.set(1_000);
        let npc = a_fighter(ME, 101, 0, (175, 2000), 0, SPOT, 10_000);
        let mut world = stand_field(&clock, vec![npc], open_cells(), true);
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(1);
        world.toggle_monster_log(Vid::new(1));
        drained(&mut inbox);
        (clock, world, inbox)
    }

    /// The toggle is silent and flips both ways; the leave forgets it.
    #[test]
    fn the_monster_toggle_is_silent_and_the_leave_forgets_it() {
        let (_clock, mut world, mut alpha) = a_watch();
        world.toggle_monster_log(Vid::new(1));
        assert!(drained(&mut alpha).is_empty(), "toggling off is silent");
        world.toggle_monster_log(Vid::new(1));
        assert!(drained(&mut alpha).is_empty(), "toggling on is silent");
        world.monster_log(ME, b"!");
        assert_eq!(drained(&mut alpha).len(), 1, "subscribed again");
        world.leave_world(Vid::new(1));
        let mut bravo = enter(&mut world, 1, NEAR, 12_345);
        drained(&mut bravo);
        world.monster_log(ME, b"!");
        assert!(drained(&mut bravo).is_empty(), "the leave unsubscribed");
    }

    /// The fan's bytes: a talking chat with the speaker's VID, `%u)` plus the line and its
    /// terminator; a far subscriber hears nothing.
    #[test]
    fn the_fan_carries_the_speaker_and_its_line_within_6000() {
        let (_clock, mut world, mut alpha) = a_watch();
        world.monster_log(ME, b"Let's go to the next state");
        let heard = drained(&mut alpha);
        assert_eq!(heard.len(), 1);
        let chat = GcChat::decode(&heard[0]).expect("a chat record");
        assert_eq!(heard[0][0], HEADER_GC_CHAT.value());
        assert_eq!(chat.chat_type, CHAT_TYPE_TALKING);
        assert_eq!(chat.id, ME);
        assert_eq!(chat.empire, 0);
        let mut want = format!("{ME})").into_bytes();
        want.extend_from_slice(b"Let's go to the next state");
        want.push(0);
        assert_eq!(chat.text, want);
        let mut bravo = enter(&mut world, 2, FAR, 12_345);
        drained(&mut bravo);
        drained(&mut alpha);
        world.toggle_monster_log(Vid::new(2));
        world.monster_log(ME, b"!");
        assert_eq!(drained(&mut alpha).len(), 1, "the near one hears");
        assert!(drained(&mut bravo).is_empty(), "the far one hears nothing");
    }

    /// A dead hit begins the fight with its three lines in legacy's order: the victim, the
    /// stand, the pulse.
    #[test]
    fn a_begin_logs_the_victim_the_stand_and_the_pulse_in_order() {
        let (_clock, mut world, mut alpha) = a_watch();
        world.begin_fight(ME, Vid::new(1));
        let heard = drained(&mut alpha);
        assert_eq!(heard.len(), 3, "three lines: {heard:02x?}");
        let texts: Vec<Vec<u8>> = heard
            .iter()
            .map(|record| GcChat::decode(record).expect("a chat record").text)
            .collect();
        let prefix = format!("{ME})").into_bytes();
        let mut victim = prefix.clone();
        victim.extend_from_slice(b"Set attack target: P1");
        victim.push(0);
        let mut fighting = prefix.clone();
        fighting.extend_from_slice(b"[BATTLE] fighting state");
        fighting.push(0);
        let mut pulse = prefix.clone();
        pulse.extend_from_slice(b"Let's go to the next state");
        pulse.push(0);
        assert_eq!(texts[0], victim);
        assert_eq!(texts[1], fighting);
        assert_eq!(texts[2], pulse);
    }

    /// `SetVictim(None)` on a held victim logs the loss; on none it stays silent.
    #[test]
    fn clearing_a_held_victim_logs_and_clearing_none_is_silent() {
        let (_clock, mut world, mut alpha) = a_watch();
        world.set_victim(ME, None);
        // `begin_fight`'s three lines plus the clear's one are drained together below;
        // first the silent clear on no victim.
        world.begin_fight(ME, Vid::new(1));
        world.set_victim(ME, None);
        let heard = drained(&mut alpha);
        assert_eq!(heard.len(), 4, "three to begin, one to clear: {heard:02x?}");
        let last = GcChat::decode(&heard[3]).expect("a chat record");
        let mut want = format!("{ME})").into_bytes();
        want.extend_from_slice(b"???????????");
        want.push(0);
        assert_eq!(last.text, want);
    }

    /// The detail line names the attacker's name, VID and point; unflagged it stays silent.
    #[test]
    fn the_detail_line_names_the_attacker_and_needs_the_flag() {
        let (_clock, mut world, mut alpha) = a_watch();
        world.log_pc_damage(Vid::new(1), Vid::new(1));
        assert!(drained(&mut alpha).is_empty(), "unflagged is silent");
        world.toggle_detail_log(Vid::new(1));
        assert!(drained(&mut alpha).is_empty(), "the toggle is silent");
        world.log_pc_damage(Vid::new(1), Vid::new(1));
        let heard = drained(&mut alpha);
        assert_eq!(heard.len(), 1);
        let chat = GcChat::decode(&heard[0]).expect("a chat record");
        assert_eq!(chat.chat_type, CHAT_TYPE_INFO);
        assert_eq!(chat.id, 0);
        let mut want = b"P1[1]".to_vec();
        want.extend_from_slice(b"\xef\xbf\xbd\xef\xbf\xbd \xef\xbf\xbd\xef\xbf\xbd\xef\xbf\xbd\xef\xbf\xbd \xef\xbf\xbd\xef\xbf\xbd\xc4\xa1: 3100 3200");
        assert_eq!(chat.text, want);
        let _ = (PLACE, point::POINT_HP, CHAR_TYPE_MONSTER);
    }
}
