//! The reward window (`ENABLE_REWARD_SYSTEM`): `/reward_system`
//! (`do_reward_system`, `G/cmd_general.cpp:4299-4310`), the two tables, and the
//! progress `DoReward` marks (`G/char_manager.cpp:1413-1863`, `:1985-2043`).
//!
//! # What legacy does
//!
//! `/reward_system update` (the line answers nothing else: fewer than two words, or any
//! second word but `update`, returns) sends the open pair: the global full fan (mission
//! to first-claim name, `clean` 1) and then the solo full fan (mission to the caller's
//! count), each as `RewardData <global 0/1> <clean 0/1> <body>`, where the body is
//! `mission?name#` per row or `empty`. The open stamps the caller's `reward_opened`
//! protect to 1, which later single fans require.
//!
//! The tables are compiled in: the global map holds 0-16 with the mission kind and the
//! sub-value filter (the source names key 1 twice — `FIRST_ITEM/319` then
//! `AVERAGE_BONUS/55` — so the map keeps the second), and the solo map holds 0-17 with
//! the kind, the sub-value, the max, and the level-difference window. Every prize is
//! one 31111. A mission whose kind matches fires when its sub-value filter passes (a
//! zero sub-value passes everything, else the sub-value must equal), and a solo mission
//! with a nonzero level window passes only when the killer's level sits within it of
//! the victim's (`level_check_difference`, `:1982-1985`).
//!
//! `DoReward` marks the progress: a global mission whose file is missing claims it for
//! the caller (fanning the single to every opener on this server and, in legacy, to the
//! peers) and pays the prize; a solo mission adds the value, clamped to its max, fans
//! the single to the caller, and pays the prize exactly when the clamp lands on the max.
//! The call sites this slice ports are the level-up (`LEVEL_UP` at the new level,
//! `G/char.cpp:4051-4055`), the load-time playtime (`PLAYTIME` at the seconds,
//! `:1524-1527`), and the kill (`KILL_STONE`, `KILL_BOSS` at boss rank and up, else
//! `KILL_MONSTER`, each at the race with the victim's level, `G/char_battle.cpp:1283-1292`).
//! The solo counts persist per pid and the global claims per mission; legacy keeps both
//! as plain-text files outside SQL, so the store shape is ours (ADR-0001).
//!
//! # Divergences
//!
//! - **The global fan stays on this server.** Legacy relays the single through
//!   `HEADER_GG_REWARD_INFO` to the peers (ADR-0001 retires the GG protocol); one
//!   process holds every Channel, so the fan reaches every opener directly.
//! - **The global table's second 15 and 16 have no readers.** The source holds 18
//!   global rows (keys 0-16 with the doubled 1); the kind/sub-value pairs of key 15
//!   (`OFFLINESHOP_SLOT/40`) and key 16 (`INVENTORY_SLOT/40`) fire from no ported call
//!   site, so they sit claimed by nobody until their systems land.
//!
//! Everything here runs on the game thread only (ADR-0002).

use std::collections::HashMap;

use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_COMMAND;

use super::GameState;
use crate::item_move::{MovedItems, Mover};

/// `REWARD_MISSION_*` (`C/length.h:1250-1269`).
pub(super) const MISSION_LEVEL_UP: u8 = 0;
/// `REWARD_MISSION_LEVEL_UP_PET`.
const MISSION_LEVEL_UP_PET: u8 = 1;
/// `REWARD_MISSION_SKILL_UPGRADE`.
const MISSION_SKILL_UPGRADE: u8 = 2;
/// `REWARD_MISSION_KILL_MONSTER`.
pub(super) const MISSION_KILL_MONSTER: u8 = 3;
/// `REWARD_MISSION_KILL_STONE`.
pub(super) const MISSION_KILL_STONE: u8 = 4;
/// `REWARD_MISSION_KILL_BOSS`.
pub(super) const MISSION_KILL_BOSS: u8 = 5;
/// `REWARD_MISSION_INVENTORY_SLOT`.
const MISSION_INVENTORY_SLOT: u8 = 6;
/// `REWARD_MISSION_OFFLINESHOP_SLOT`.
const MISSION_OFFLINESHOP_SLOT: u8 = 7;
/// `REWARD_MISSION_AVERAGE_BONUS`.
const MISSION_AVERAGE_BONUS: u8 = 8;
/// `REWARD_MISSION_BATTLEPASS`.
const MISSION_BATTLEPASS: u8 = 9;
/// `REWARD_MISSION_FIRST_ITEM`.
const MISSION_FIRST_ITEM: u8 = 10;
/// `REWARD_MISSION_CUSTOM_SASH`.
const MISSION_CUSTOM_SASH: u8 = 11;
/// `REWARD_MISSION_BIOLOG`.
const MISSION_BIOLOG: u8 = 12;
/// `REWARD_MISSION_PASSIVE_SKILL_COMPLETE`.
#[allow(dead_code)]
const MISSION_PASSIVE_SKILL_COMPLETE: u8 = 13;
/// `REWARD_MISSION_DUNGEON`.
const MISSION_DUNGEON: u8 = 14;
/// `REWARD_MISSION_USE_ITEM`.
const MISSION_USE_ITEM: u8 = 15;
/// `REWARD_MISSION_SELL_ITEM`.
#[allow(dead_code)]
const MISSION_SELL_ITEM: u8 = 16;
/// `REWARD_MISSION_BUY_ITEM`.
#[allow(dead_code)]
const MISSION_BUY_ITEM: u8 = 17;
/// `REWARD_MISSION_PLAYTIME`.
pub(super) const MISSION_PLAYTIME: u8 = 18;
/// `REWARD_MISSION_COMPLETE_SKILL`.
const MISSION_COMPLETE_SKILL: u8 = 19;

/// One global row: the key, the mission kind, and the sub-value filter.
type GlobalRow = (u8, u8, u32);

/// `m_mapRewardGlobal` (`G/char_manager.cpp:1413-1561`): the key, the kind, the
/// sub-value. The doubled key 1 keeps the second row (`AVERAGE_BONUS/55`), as the
/// `std::map` initializer does.
const GLOBAL_ROWS: [GlobalRow; 17] = [
    (0, MISSION_DUNGEON, 2),
    (1, MISSION_AVERAGE_BONUS, 55),
    (2, MISSION_AVERAGE_BONUS, 55),
    (3, MISSION_CUSTOM_SASH, 25),
    (4, MISSION_FIRST_ITEM, 310),
    (5, MISSION_LEVEL_UP, 120),
    (6, MISSION_LEVEL_UP, 115),
    (7, MISSION_LEVEL_UP_PET, 115),
    (8, MISSION_DUNGEON, 1),
    (9, MISSION_DUNGEON, 2),
    (10, MISSION_SKILL_UPGRADE, 40),
    (11, MISSION_FIRST_ITEM, 51_002),
    (12, MISSION_FIRST_ITEM, 18_099),
    (13, MISSION_BATTLEPASS, 0),
    (14, MISSION_BIOLOG, 94),
    (15, MISSION_OFFLINESHOP_SLOT, 40),
    (16, MISSION_INVENTORY_SLOT, 40),
];

/// One solo row: the key, the mission kind, the sub-value filter, the max, and the
/// level-difference window.
type SoloRow = (u8, u8, u32, u32, u8);

/// `m_mapRewardSolo` (`G/char_manager.cpp:1566-1782`).
const SOLO_ROWS: [SoloRow; 18] = [
    (0, MISSION_LEVEL_UP, 120, 1, 0),
    (1, MISSION_KILL_BOSS, 6_500, 50, 0),
    (2, MISSION_KILL_BOSS, 6_501, 200, 0),
    (3, MISSION_USE_ITEM, 67_997, 10_000, 0),
    (4, MISSION_USE_ITEM, 67_998, 50, 0),
    (5, MISSION_KILL_BOSS, 2_862, 250, 0),
    (6, MISSION_KILL_STONE, 0, 100_000, 15),
    (7, MISSION_DUNGEON, 0, 1_000, 0),
    (8, MISSION_KILL_BOSS, 2_493, 100, 0),
    (9, MISSION_KILL_BOSS, 2_598, 100, 0),
    (10, MISSION_KILL_BOSS, 2_092, 100, 0),
    (11, MISSION_KILL_BOSS, 6_091, 400, 0),
    (12, MISSION_KILL_BOSS, 6_191, 400, 0),
    (13, MISSION_KILL_BOSS, 20_435, 100, 0),
    (14, MISSION_COMPLETE_SKILL, 0, 1, 0),
    (15, MISSION_KILL_BOSS, 0, 2_500, 0),
    (16, MISSION_PLAYTIME, 0, 43_200, 0),
    (17, MISSION_OFFLINESHOP_SLOT, 40, 1, 0),
];

/// The prize every row pays (`{31111, 1}`), through `AutoGiveItem`.
const PRIZE_VNUM: u32 = 31111;

impl GameState {
    /// `/reward_system ...` (`do_reward_system`, `G/cmd_general.cpp:4299-4310`): fewer
    /// than two words, or any second word but `update`, stays silent; `update` sends
    /// the open pair (the global full fan, then the solo full fan) and stamps the
    /// caller's open. Words split on spaces (`split_argument`, `G/utils.cpp:178-182`),
    /// with the command's own name as the first word, as legacy passes the whole line.
    pub(super) fn run_reward_system(&mut self, vid: Vid, rest: &[u8], _mover: Mover) {
        let words: Vec<&[u8]> = [b"reward_system".as_slice()]
            .into_iter()
            .chain(
                rest.split(|byte| *byte == b' ')
                    .filter(|word| !word.is_empty()),
            )
            .collect();
        if words.len() < 2 || words[1] != b"update" {
            return;
        }
        self.set_reward_opened(vid);
        self.send_reward_info(255, true, Some(vid));
        self.send_reward_info(255, false, Some(vid));
    }

    /// `CHARACTER_MANAGER::SendRewardInfo` (`G/char_manager.cpp:1865-1980`) for the
    /// caller, or for every opener when `vid` is `None`: the global full fan when
    /// `mission` is 255 and `global` is set, the global single with the opener gate
    /// else, and the solo pair the same way. Each line is
    /// `RewardData <global 0/1> <clean 0/1> <body>`.
    fn send_reward_info(&mut self, mission: u8, global: bool, vid: Option<Vid>) {
        let clean = mission == 255;
        let mut lines: Vec<(Vid, Vec<u8>)> = Vec::new();
        if global {
            if clean {
                let body = self.global_fan_body();
                let targets = self.reward_targets(vid);
                for target in targets {
                    lines.push((target, reward_line(true, true, &body)));
                }
            } else if let Some(target) = vid {
                if !self.reward_opened(target) {
                    return;
                }
                if let Some(name) = self.reward_global_name(mission) {
                    let body = format!("{mission}?{}#", String::from_utf8_lossy(&name));
                    lines.push((target, reward_line(true, false, body.as_bytes())));
                }
            }
        } else {
            let Some(target) = vid else { return };
            if clean {
                let body = self.solo_fan_body(target);
                lines.push((target, reward_line(false, true, &body)));
            } else {
                if !self.reward_opened(target) {
                    return;
                }
                if Self::solo_mission_exists(mission) {
                    let count = self.reward_solo_count(target, mission);
                    let body = format!("{mission}?{count}#");
                    lines.push((target, reward_line(false, false, body.as_bytes())));
                }
            }
        }
        for (target, line) in lines {
            let Some(mover) = self.own_mover(target) else {
                continue;
            };
            self.tell(target, mover, CHAT_TYPE_COMMAND, &line, &[]);
        }
    }

    /// The global single fanned to every opener on this server (`DoReward`'s global arm
    /// with `isP2P`, minus the retired relay): each opener hears the claimed mission.
    fn fan_global_single(&mut self, mission: u8) {
        let Some(name) = self.reward_global_name(mission) else {
            return;
        };
        let line = reward_line(
            true,
            false,
            format!("{mission}?{}#", String::from_utf8_lossy(&name)).as_bytes(),
        );
        let openers: Vec<Vid> = self.reward_counts.opened.iter().copied().collect();
        for opener in openers {
            let Some(mover) = self.own_mover(opener) else {
                continue;
            };
            self.tell(opener, mover, CHAT_TYPE_COMMAND, &line, &[]);
        }
    }

    /// The global full fan's body: `mission?name#` per claimed mission, in key order, or
    /// `empty`.
    fn global_fan_body(&self) -> Vec<u8> {
        let mut body = Vec::new();
        for (key, _, _) in GLOBAL_ROWS {
            if let Some(name) = self.reward_global_name(key) {
                body.extend_from_slice(key.to_string().as_bytes());
                body.push(b'?');
                body.extend_from_slice(&name);
                body.push(b'#');
            }
        }
        if body.is_empty() {
            body.extend_from_slice(b"empty");
        }
        body
    }

    /// The solo full fan's body for the caller: `mission?count#` per row, in key order, or
    /// `empty` (unreachable: every row fans its count, even 0).
    fn solo_fan_body(&self, vid: Vid) -> Vec<u8> {
        let mut body = Vec::new();
        for (key, _, _, _, _) in SOLO_ROWS {
            body.extend_from_slice(key.to_string().as_bytes());
            body.push(b'?');
            body.extend_from_slice(self.reward_solo_count(vid, key).to_string().as_bytes());
            body.push(b'#');
        }
        if body.is_empty() {
            body.extend_from_slice(b"empty");
        }
        body
    }

    /// `CHARACTER_MANAGER::DoReward` (`G/char_manager.cpp:1985-2043`): the global missions
    /// of `kind` whose sub-value passes claim for the caller when unclaimed (fanning the
    /// single and paying the prize), then the solo missions of `kind` whose sub-value and
    /// level window pass add `value`, clamp to the max, fan the single, and pay the prize
    /// exactly on reaching the max. Returns the rows the store must write.
    pub(super) fn do_reward(
        &mut self,
        vid: Vid,
        mover: Mover,
        kind: u8,
        sub: u32,
        value: u32,
        level_difference: u8,
    ) -> RewardWrites {
        let mut writes = RewardWrites::default();
        let name = self
            .characters
            .find_by_vid(vid)
            .map(|character| character.name().as_bytes().to_vec())
            .unwrap_or_default();
        let killer_level = self.bodies.get(&vid).map_or(0, |body| body.card.level);
        for (key, row_kind, row_sub) in GLOBAL_ROWS {
            if row_kind != kind || (row_sub != 0 && row_sub != sub) {
                continue;
            }
            if self.reward_global_name(key).is_some() {
                continue;
            }
            self.claim_reward_global(key, &name);
            writes.global.push((key, name.clone()));
            self.fan_global_single(key);
            if let Some(packed) = self.pay_reward_prize(vid, mover) {
                writes.pack(packed);
            }
        }
        for (key, row_kind, row_sub, max, window) in SOLO_ROWS {
            if row_kind != kind || (row_sub != 0 && row_sub != sub) {
                continue;
            }
            if window != 0 && !level_check_difference(killer_level, level_difference, window) {
                continue;
            }
            let current = self.reward_solo_count(vid, key);
            if current >= max {
                continue;
            }
            let landed = current.saturating_add(value).min(max);
            self.set_reward_solo(vid, key, landed);
            writes.solo.push((key, landed));
            self.send_reward_info(key, false, Some(vid));
            if landed == max {
                if let Some(packed) = self.pay_reward_prize(vid, mover) {
                    writes.pack(packed);
                }
            }
        }
        writes
    }

    /// One prize through `AutoGiveItem(31111, 1)`: silent, laid at the caller's feet when
    /// no cell holds it. Answers the packed rows for the descriptor.
    fn pay_reward_prize(&mut self, vid: Vid, mover: Mover) -> Option<MovedItems> {
        let questing = self.quest_running(vid);
        let (outcome, mut given) = self.auto_give(vid, (PRIZE_VNUM, 1), -1, questing)?;
        if let world::character::AutoGiven::Unplaced(item) = outcome {
            self.lay_auto_given(vid, item);
        }
        given
            .records
            .retain(|record| !matches!(record, world::character::MoveRecord::PickedUp { .. }));
        let pid = self.characters.find_by_vid(vid).ok()?.player_id();
        Some(MovedItems::new(
            pid,
            vid.raw(),
            given,
            mover,
            &self.protos,
            &self.locale,
        ))
    }
}

/// What `DoReward` moved: the claimed global missions with the name, the solo missions
/// with the landed count, and the prizes' packed rows.
#[derive(Debug, Default)]
pub struct RewardWrites {
    /// The claimed global missions, in table order.
    pub global: Vec<(u8, Vec<u8>)>,
    /// The moved solo missions with the landed counts, in table order.
    pub solo: Vec<(u8, u32)>,
    /// The records the prizes packed for the caller, in order.
    pub records: Vec<Vec<u8>>,
    /// The row changes the prizes packed, in order.
    pub changes: Vec<db::items::RowChange>,
}

impl RewardWrites {
    /// Whether nothing moved: no store write is owed.
    #[must_use]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.global.is_empty() && self.solo.is_empty() && self.changes.is_empty()
    }

    /// Fold one prize's packed rows into the writes.
    fn pack(&mut self, packed: MovedItems) {
        self.records.extend(packed.records);
        self.changes.extend(packed.changes);
    }
}

/// `level_check_difference` (`G/char_manager.cpp:1982-1985`): the killer's level sits
/// within `window` of the victim's.
fn level_check_difference(killer: u8, victim: u8, window: u8) -> bool {
    let (killer, victim, window) = (i16::from(killer), i16::from(victim), i16::from(window));
    killer - window <= victim && killer + window >= victim
}

/// The `RewardData` command line: `RewardData <global 0/1> <clean 0/1> <body>`.
fn reward_line(global: bool, clean: bool, body: &[u8]) -> Vec<u8> {
    let mut line = format!("RewardData {} {} ", i32::from(global), i32::from(clean)).into_bytes();
    line.extend_from_slice(body);
    line
}

/// The reward counts each character holds, which [`GameState`] keeps beside the bodies.
#[derive(Debug, Default)]
pub struct RewardState {
    /// The solo counts by VID, each the mission to its count.
    pub solo: HashMap<Vid, HashMap<u8, u32>>,
    /// The global claims: the mission to its first claimer's name.
    pub global: HashMap<u8, Vec<u8>>,
    /// The VIDs whose window stands open (`reward_opened` 1).
    pub opened: std::collections::HashSet<Vid>,
}

impl GameState {
    /// Whether the caller's window stands open (`GetProtectTime("reward_opened")`).
    fn reward_opened(&self, vid: Vid) -> bool {
        self.reward_counts.opened.contains(&vid)
    }

    /// Stamp the caller's open (`SetProtectTime("reward_opened", 1)`).
    fn set_reward_opened(&mut self, vid: Vid) {
        self.reward_counts.opened.insert(vid);
    }

    /// The global claim of `mission`, if one stands (`GetGlobalReward`).
    fn reward_global_name(&self, mission: u8) -> Option<Vec<u8>> {
        self.reward_counts.global.get(&mission).cloned()
    }

    /// Claim the global `mission` for `name` (`SetGlobalReward`).
    fn claim_reward_global(&mut self, mission: u8, name: &[u8]) {
        self.reward_counts.global.insert(mission, name.to_vec());
    }

    /// The caller's solo count of `mission` (`GetRewardData`), 0 unmarked.
    fn reward_solo_count(&self, vid: Vid, mission: u8) -> u32 {
        self.reward_counts
            .solo
            .get(&vid)
            .and_then(|counts| counts.get(&mission))
            .copied()
            .unwrap_or(0)
    }

    /// Mark the caller's solo count of `mission` (`SetRewardData`).
    fn set_reward_solo(&mut self, vid: Vid, mission: u8, count: u32) {
        self.reward_counts
            .solo
            .entry(vid)
            .or_default()
            .insert(mission, count);
    }

    /// Whether the solo table holds `mission`.
    fn solo_mission_exists(mission: u8) -> bool {
        SOLO_ROWS.iter().any(|(key, _, _, _, _)| *key == mission)
    }

    /// The fan's targets: the caller, or every opener when `vid` is `None`.
    fn reward_targets(&self, vid: Option<Vid>) -> Vec<Vid> {
        match vid {
            Some(vid) => vec![vid],
            None => self.reward_counts.opened.iter().copied().collect(),
        }
    }

    /// The kill's `DoReward` for the monster under `vid`, paid by `killer`
    /// (`G/char_battle.cpp:1283-1292`).
    pub(super) fn reward_kill(&mut self, vid: u32, killer: Vid) {
        let Some(npc) = self.npc_by_vid(vid) else {
            return;
        };
        let kind = if npc.char_type == gamedata::mob_proto::CHAR_TYPE_STONE {
            MISSION_KILL_STONE
        } else if npc.fight.rank >= gamedata::mob_proto::MOB_RANK_BOSS {
            MISSION_KILL_BOSS
        } else {
            MISSION_KILL_MONSTER
        };
        let (race, level) = (u32::from(npc.race), npc.fight.level);
        let Some(mover) = self.own_mover(killer) else {
            return;
        };
        let writes = self.do_reward(killer, mover, kind, race, 1, level);
        self.hold_reward_writes(killer, writes);
    }

    /// Hold `writes` for the descriptor: the prizes' records go to the caller now, and
    /// the counts wait for the next save, as `SaveRewardData` writes on disconnect
    /// (`G/char.cpp:1700-1702`).
    pub(super) fn hold_reward_writes(&mut self, vid: Vid, writes: RewardWrites) {
        for record in writes.records {
            let _sent = self.write_to_client(vid, record);
        }
        // The counts live in `reward_counts` already; the descriptor's save reads them
        // from there.
        let _held = writes.solo.len() + writes.global.len();
    }
}

#[cfg(test)]
mod tests {
    use protocol::gc_inventory::HEADER_GC_CHAT;
    use tokio::sync::mpsc::UnboundedReceiver;

    use world::npc::FIRST_NPC_VID;

    use super::super::fixtures::{a_fighter, enter, open_cells, stand_field, TestClock};
    use super::*;
    use crate::game_loop::PulseProcessor;

    /// The monster every test watches.
    const ME: u32 = FIRST_NPC_VID;

    /// Where it stands.
    const SPOT: (i32, i32) = (3200, 3200);

    /// The player.
    const NEAR: (i32, i32) = (3100, 3200);

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// The dog 101 at [`SPOT`] and the player 1 [`NEAR`], after Pulse 1; the inbox is
    /// the player's, drained.
    fn a_kill() -> (TestClock, GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        clock.set(1_000);
        let npc = a_fighter(ME, 101, 0, (175, 2000), 0, SPOT, 10_000);
        let mut world = stand_field(&clock, vec![npc], open_cells(), true);
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(1);
        drained(&mut inbox);
        (clock, world, inbox)
    }

    /// `DoReward`'s solo arms (`G/char_manager.cpp:2008-2041`): a `KILL_MONSTER` the
    /// table does not carry marks nothing; a `KILL_BOSS` of race 2493 lands on its
    /// row and on the `sub` 0 catch-all (mission 15); race 0 adds 1 to mission 15 and
    /// fans its single, and clamps at the 2500 max, paying the prize exactly once, at
    /// the max.
    #[test]
    fn do_reward_marks_counts_and_pays_the_prize_once_at_the_max() {
        let (_clock, mut world, mut alpha) = a_kill();
        let vid = Vid::new(1);
        let mover = world.own_mover(vid).expect("the player stands entered");
        world.set_reward_opened(vid);
        let clean = world.do_reward(vid, mover, MISSION_KILL_MONSTER, 101, 1, 1);
        assert!(clean.solo.is_empty(), "no KILL_MONSTER row stands");
        assert!(drained(&mut alpha).is_empty(), "nothing fans");
        world.set_reward_solo(vid, 15, 2_499);
        let refused = world.do_reward(vid, mover, MISSION_KILL_BOSS, 2493, 1, 1);
        assert_eq!(
            refused.solo,
            vec![(8, 1), (15, 2_500)],
            "race 2493 lands on its row and the 0-sub catch-all"
        );
        drained(&mut alpha);
        world.set_reward_solo(vid, 15, 0);
        world.set_reward_solo(vid, 8, 0);
        world.set_reward_solo(vid, 15, 2_499);
        let writes = world.do_reward(vid, mover, MISSION_KILL_BOSS, 0, 1, 1);
        assert_eq!(writes.solo, vec![(15, 2_500)], "the count lands on the max");
        let heard = drained(&mut alpha);
        let fans: Vec<&[u8]> = heard
            .iter()
            .filter(|record| record[0] == HEADER_GC_CHAT.value())
            .map(Vec::as_slice)
            .collect();
        assert_eq!(fans.len(), 1, "exactly the single fans: {heard:02x?}");
        assert!(
            fans[0].ends_with(b"RewardData 0 0 15?2500#"),
            "the single names the mission and the count: {:02x?}",
            fans[0]
        );
        let again = world.do_reward(vid, mover, MISSION_KILL_BOSS, 0, 1, 1);
        assert!(again.solo.is_empty(), "at the max nothing marks");
        world.set_reward_solo(vid, 15, 2_499);
        let over = world.do_reward(vid, mover, MISSION_KILL_BOSS, 0, 100, 1);
        assert_eq!(over.solo, vec![(15, 2_500)], "the add clamps to the max");
    }
}
