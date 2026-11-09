//! The battle pass (`CBattlePass`, `G/BattlePassManager.cpp:1-345`) with its two
//! lines (`do_battle_pass`, `do_battle_pass_admin`,
//! `G/cmd_general.cpp:4176-4294`, `G/cmd.cpp:522-523`).
//!
//! Legacy is a process-wide singleton holding the JSON table, while each
//! character holds its own mission map (`std::map<uint8_t,
//! std::vector<TPlayerBattlePass>>`, `G/char.h:2644`): key 0 is the daily
//! bucket, key 1 the weekly bucket. The Rewrite keeps the table on the state
//! (`with_battle_pass_table`, installed at boot) and one such map per
//! character VID.
//!
//! The wire is `HEADER_GC_BATTLE_PASS` (238, `G/packet.h:3509`): a 9-byte
//! fixed head (`bHeader`, `wSize`, `bSubHeader`, `isPremium`, `points`) with
//! the mission rows (`TPlayerBattlePass`, 11 bytes each: `missionID u16`,
//! `progress u32`, `type u8`, `endTime u32`) appended in map order, daily
//! before weekly (`CHARACTER::UpdateBattlePass`, `G/char.cpp:12914-12947`).
//! The client never registered byte 238 in its decode table
//! (`PythonNetworkStream.cpp` has no such `Set`), so on a live client the
//! send is unobservable; the port still sends the bytes.
//!
//! # Divergences
//!
//! - **Byte 238 is client-unhandled.** `protocol/src/gc_inventory.rs` holds
//!   the 134 registered game-to-client bytes (positive controls 0xe7/0xea
//!   present, negative control: no row near 0xee); no new inventory row is
//!   invented. The record below is byte-exact to `G/packet.h:3510-3518`.
//! - **`GetMissions` draws from the world's dice.** Legacy shuffles with
//!   `std::mt19937` from `random_device` (`:256-258`); the Rewrite draws with
//!   Fisher-Yates over the world's dice, so the mission picks differ.
//! - **Day math in UTC.** The mission end times count UTC midnights and weeks
//!   (see `gamedata::battle_pass`), while legacy's `localtime`/`mktime`
//!   counts the host's zone.
//! - **Boot failure.** A missing or malformed `battlepass.json` stops the
//!   server, as for every other Game data file. Legacy only logs and boots
//!   with an empty (or half-cleared) map.
//! - **No quest-flag persistence gap.** Quest flags (`battlepass.points`,
//!   `battlepass.isPremium`) ride the existing `quest_flag` rows; the mission
//!   rows ride the new `battle_pass` store below.
//!
//! # Waiting hooks
//!
//! Three legacy sites have no Rewrite path yet, so their missions wait:
//! `MESSAGES` (the shout fan-out lives in `main.rs`, never reaching the
//! world), `CHEST_OPEN` (no box-open flow is ported), and `PLAYTIME` (no
//! per-minute play tick exists). Their `Register*` arms land with the paths
//! that carry them.

use std::sync::Arc;

use common::vid::Vid;

use super::GameState;
use crate::chat_line::Arg;
use crate::command::two_arguments;
use crate::game_state::gm_lines::{int_word, GmRows};
use crate::item_move::Mover;
use gamedata::battle_pass::{
    BattlePassMission, BattlePassTable, MISSION_TIME_DAILY, MISSION_TIME_WEEKLY,
    MISSION_TYPE_CRAFT, MISSION_TYPE_ITEM_DROP, MISSION_TYPE_KILL, MISSION_TYPE_SELL,
    MISSION_TYPE_SPECIFIC, MISSION_TYPE_USE_ITEM,
};
use protocol::gc_chat::{CHAT_TYPE_INFO, CHAT_TYPE_NOTICE};

/// `HEADER_GC_BATTLE_PASS` (`G/packet.h:3509`).
pub(super) const HEADER_GC_BATTLE_PASS: u8 = 238;
/// `BATTLEPASS_GC_SUB_INIT` (`G/packet.h:3508`).
pub(super) const SUB_INIT: u8 = 0;
/// `BATTLEPASS_GC_SUB_UPDATE` (`G/packet.h:3508`).
pub(super) const SUB_UPDATE: u8 = 1;
/// `BATTLEPASS_MISSIONS_PER_PLAYER` (`C/tables.h:486`).
pub(super) const MISSIONS_PER_PLAYER: usize = 10;
/// The missions each draw deals (`GetMissions`, `G/BattlePassManager.cpp:265`).
pub(super) const DRAWN_MISSIONS: usize = 5;
/// The premium ticket (`CountSpecifyItem(30018)`, `G/char.cpp:12954`).
pub(super) const PREMIUM_TICKET: u32 = 30018;
/// The premium item (`GetVnum() == 40004`, `G/char_item.cpp:2827`).
pub(super) const PREMIUM_ITEM: u32 = 40004;
/// The level gate every `Register*Mission` checks (`me->GetLevel() < 30`).
pub(super) const MIN_LEVEL: u8 = 30;
/// The points per level (`GetQuestFlag("battlepass.points") / 10000`).
pub(super) const POINTS_PER_LEVEL: i32 = 10_000;
/// `battlepass.points`.
pub(super) const POINTS_FLAG: &[u8] = b"battlepass.points";
/// `battlepass.isPremium`.
pub(super) const PREMIUM_FLAG: &[u8] = b"battlepass.isPremium";
/// `ITEM_BLEND` (`G/char_item.cpp:2824`): blends never count as used items.
pub(super) const ITEM_BLEND: i32 = 27;

/// One `TPlayerBattlePass` row (`C/tables.h:542-552`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PlayerMission {
    /// `missionID`.
    pub(crate) mission_id: u16,
    /// `progress`.
    pub(crate) progress: u32,
    /// `type`: 0 daily, 1 weekly.
    pub(crate) kind: u8,
    /// `endTime`.
    pub(crate) end_time: u32,
}

impl PlayerMission {
    /// The 11 wire bytes (`missionID u16 | progress u32 | type u8 | endTime u32`).
    fn encode(self) -> [u8; 11] {
        let mut out = [0_u8; 11];
        out[0..2].copy_from_slice(&self.mission_id.to_le_bytes());
        out[2..6].copy_from_slice(&self.progress.to_le_bytes());
        out[6] = self.kind;
        out[7..11].copy_from_slice(&self.end_time.to_le_bytes());
        out
    }
}

/// Encode `UpdateBattlePass` (`G/char.cpp:12914-12947`): the 9-byte head with
/// the rows in map order (daily bucket before weekly).
pub(super) fn encode_update(
    sub: u8,
    premium: bool,
    points: i32,
    daily: &[PlayerMission],
    weekly: &[PlayerMission],
) -> Vec<u8> {
    let rows = daily.len() + weekly.len();
    let size = 9 + rows * 11;
    let mut out = Vec::with_capacity(size);
    out.push(HEADER_GC_BATTLE_PASS);
    out.extend_from_slice(&u16::try_from(size).unwrap_or(u16::MAX).to_le_bytes());
    out.push(sub);
    out.push(u8::from(premium));
    out.extend_from_slice(&points.to_le_bytes());
    for mission in daily.iter().chain(weekly.iter()) {
        out.extend_from_slice(&mission.encode());
    }
    out
}

impl GameState {
    /// The battle-pass table, defaulting to empty before boot installs it.
    fn pass_table(&self) -> Arc<BattlePassTable> {
        self.battle_pass_table.clone()
    }

    /// The caller's level, through its points.
    fn pass_level(&self, vid: Vid) -> Option<u8> {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map(world::character::Points::level)
    }

    /// Read a quest flag of the caller    /// Read a quest flag of the caller's player.
    fn pass_flag(&self, vid: Vid, name: &[u8]) -> i32 {
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        self.quests.as_ref().map_or(0, |quests| {
            quests.manager().quest_flag(pid, name).unwrap_or(0)
        })
    }

    /// Write a quest flag of the caller's player.
    fn set_pass_flag(&mut self, vid: Vid, name: &[u8], value: i32) {
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        if pid == 0 {
            return;
        }
        if let Some(quests) = self.quests.as_mut() {
            quests.manager_mut().set_quest_flag(pid, name, value);
        }
    }

    /// The mission rows the caller holds, daily then weekly.
    fn pass_rows(&self, vid: Vid) -> (Vec<PlayerMission>, Vec<PlayerMission>) {
        let raw = vid.raw();
        let daily = self
            .battle_pass
            .get(&raw)
            .and_then(|missions| missions.get(&MISSION_TIME_DAILY))
            .cloned()
            .unwrap_or_default();
        let weekly = self
            .battle_pass
            .get(&raw)
            .and_then(|missions| missions.get(&MISSION_TIME_WEEKLY))
            .cloned()
            .unwrap_or_default();
        (daily, weekly)
    }

    /// `UpdateBattlePass` (`G/char.cpp:12914-12947`): the record rides when
    /// the caller holds any mission; silent with none.
    pub(super) fn send_battle_pass(&self, vid: Vid, _mover: Mover, sub: u8) {
        let (daily, weekly) = self.pass_rows(vid);
        if daily.is_empty() && weekly.is_empty() {
            return;
        }
        let premium = self.pass_flag(vid, PREMIUM_FLAG) != 0;
        let points = self.pass_flag(vid, POINTS_FLAG);
        let record = encode_update(sub, premium, points, &daily, &weekly);
        let _sent = self.write_to_client(vid, record);
    }

    /// `CheckBattlePassMissions` (`G/char.cpp:12762-12823`): deal the bucket
    /// when missing, or re-deal it when empty or any row expired.
    pub(super) fn check_pass_missions(&mut self, vid: Vid, time_info: u8) {
        let now = self.clock.now();
        // `battlepass.endTime - get_global_time()`, as legacy's signed 32-bit
        // comparison on the x86 `time_t` sees it.
        let wall = wall_now();
        let stale = match self
            .battle_pass
            .get(&vid.raw())
            .and_then(|missions| missions.get(&time_info))
        {
            None => true,
            Some(rows) => {
                rows.is_empty()
                    || rows.iter().any(|row| {
                        i32::from_ne_bytes(row.end_time.wrapping_sub(unsigned(wall)).to_ne_bytes())
                            <= 0
                    })
            }
        };
        if stale {
            let dealt = self.deal_pass_missions(time_info, now);
            self.battle_pass
                .entry(vid.raw())
                .or_default()
                .insert(time_info, dealt);
        }
    }

    /// Deal up to five missions of `time_info` from the table.
    fn deal_pass_missions(&mut self, time_info: u8, _now: u32) -> Vec<PlayerMission> {
        let wall = wall_now();
        let table = self.pass_table();
        let mut ids: Vec<u16> = table
            .missions
            .values()
            .filter(|mission| mission.time_info == time_info)
            .map(|mission| mission.id)
            .collect();
        ids.sort_unstable();
        let mut dealt = Vec::new();
        while dealt.len() < DRAWN_MISSIONS && !ids.is_empty() {
            let at = if ids.len() == 1 {
                0
            } else {
                let top = i32::try_from(ids.len() - 1).unwrap_or(i32::MAX);
                let drawn = world::character::number(&mut self.dice, 0, top).max(0);
                usize::try_from(drawn).unwrap_or(usize::MAX) % ids.len()
            };
            let id = ids.swap_remove(at);
            dealt.push(PlayerMission {
                mission_id: id,
                progress: 0,
                kind: time_info,
                end_time: mission_end(time_info, wall),
            });
        }
        dealt
    }

    /// `GetBattlePass` (`G/char.cpp:12826-12850`): split the ten stored rows
    /// by nonzero end time into the buckets, then refresh both.
    pub(super) fn install_pass_missions(&mut self, vid: Vid, rows: &[PlayerMission]) {
        let mut daily = Vec::new();
        let mut weekly = Vec::new();
        for row in rows {
            if row.end_time == 0 {
                continue;
            }
            if row.kind == MISSION_TIME_DAILY {
                daily.push(*row);
            } else {
                weekly.push(*row);
            }
        }
        if !daily.is_empty() {
            self.battle_pass
                .entry(vid.raw())
                .or_default()
                .insert(MISSION_TIME_DAILY, daily);
        }
        if !weekly.is_empty() {
            self.battle_pass
                .entry(vid.raw())
                .or_default()
                .insert(MISSION_TIME_WEEKLY, weekly);
        }
        self.check_pass_missions(vid, MISSION_TIME_WEEKLY);
        self.check_pass_missions(vid, MISSION_TIME_DAILY);
    }

    /// The ten stored rows (`CreatePlayerProto`, `G/char.cpp:1635-1650`):
    /// daily then weekly, and only when both buckets hold at least five;
    /// else the blob stays zeroed.
    pub(super) fn kept_pass_missions(&self, vid: Vid) -> [PlayerMission; MISSIONS_PER_PLAYER] {
        let blank = [PlayerMission::default(); MISSIONS_PER_PLAYER];
        let (daily, weekly) = self.pass_rows(vid);
        if daily.len() < DRAWN_MISSIONS || weekly.len() < DRAWN_MISSIONS {
            return blank;
        }
        let mut kept = blank;
        for (slot, row) in daily.iter().take(DRAWN_MISSIONS).enumerate() {
            kept[slot] = *row;
        }
        for (slot, row) in weekly.iter().take(DRAWN_MISSIONS).enumerate() {
            kept[slot + DRAWN_MISSIONS] = *row;
        }
        kept
    }

    /// `BattlePassAction` (`G/char.cpp:12852-12911`): count `points` on the
    /// mission in its bucket, completing it with its reward when it fills.
    /// Past the pass `endTime` nothing counts; a filled mission stays filled;
    /// an unknown mission id answers the zeroed row and stays silent.
    pub(super) fn battle_pass_action(
        &mut self,
        vid: Vid,
        mover: Mover,
        mission: u16,
        kind: u8,
        points: u32,
    ) -> Option<GmRows> {
        let wall = wall_now();
        let table = self.pass_table();
        if expiration_left(table.settings.end_time, unsigned(wall)) <= 0 {
            return None;
        }
        let data: BattlePassMission = table.mission(mission);
        let rows = self
            .battle_pass
            .entry(vid.raw())
            .or_default()
            .entry(kind)
            .or_default();
        let slot = rows.iter().position(|row| row.mission_id == mission)?;
        if rows[slot].progress == data.max_progress {
            return None;
        }
        let pid = self.characters.find_by_vid(vid).ok()?.player_id();
        if rows[slot].progress.saturating_add(points) >= data.max_progress {
            rows[slot].progress = data.max_progress;
            let held = self.pass_flag(vid, POINTS_FLAG);
            let before = held / POINTS_PER_LEVEL;
            self.set_pass_flag(
                vid,
                POINTS_FLAG,
                held.saturating_add(i32::from(data.points)),
            );
            let held = self.pass_flag(vid, POINTS_FLAG);
            let after = held / POINTS_PER_LEVEL;
            self.tell(
                vid,
                mover,
                CHAT_TYPE_NOTICE,
                b"[BattlePass] Misiunea: %s a fost finalizata.",
                &[Arg::Text(&data.name)],
            );
            // The reward arms index the tier lists with no bounds check
            // (`:12893-12899`): a level past the lists reads past them. The
            // port grants only the tiers the table holds.
            for level in before..after {
                if let Some(tier) = tier_of(&table.settings.rewards_free, level) {
                    self.pass_give(vid, mover, tier);
                }
                if self.pass_flag(vid, PREMIUM_FLAG) != 0 {
                    if let Some(tier) = tier_of(&table.settings.rewards_premium, level) {
                        self.pass_give(vid, mover, tier);
                    }
                }
            }
            if before != after {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_NOTICE,
                    b"[BattlePass] Ai colectat un premiu.",
                    &[],
                );
            }
        } else {
            rows[slot].progress = rows[slot].progress.saturating_add(points);
        }
        let (daily, weekly) = self.pass_rows(vid);
        let premium = self.pass_flag(vid, PREMIUM_FLAG) != 0;
        let held = self.pass_flag(vid, POINTS_FLAG);
        let record = encode_update(SUB_UPDATE, premium, held, &daily, &weekly);
        let _sent = self.write_to_client(vid, record);
        // `test_server`'s info line (`:12908-12909`): the build is never a
        // test server, so it stays silent.
        let character = self.characters.find_by_vid(vid).ok()?;
        Some(empty_pass_rows(pid, character.points().cloned()))
    }

    /// Grant one reward tier as `AutoGiveItem` does, silently.
    fn pass_give(&mut self, vid: Vid, mover: Mover, (vnum, count): (u32, u32)) {
        let questing = self.quest_running(vid);
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        let Some((given, done)) = self.auto_give(
            vid,
            (vnum, u16::try_from(count).unwrap_or(u16::MAX)),
            -1,
            questing,
        ) else {
            return;
        };
        if let world::character::AutoGiven::Unplaced(item) = given {
            self.lay_auto_given(vid, item);
        }
        let packed = crate::item_move::MovedItems::new(
            pid,
            vid.raw(),
            done,
            mover,
            &self.protos,
            &self.locale,
        );
        for record in packed.records {
            let _sent = self.write_to_client(vid, record);
        }
    }

    /// `RegisterTargetMission` (`G/BattlePassManager.cpp:275-297`): count a
    /// kill on every mission of `mission` the buckets hold, matching the race
    /// for `SPECIFIC` missions.
    pub(super) fn register_pass_kill(&mut self, killer: Vid, mover: Mover, race: u32) {
        let Some(level) = self.pass_level(killer) else {
            return;
        };
        if level < MIN_LEVEL {
            return;
        }
        let table = self.pass_table();
        let mut hits = Vec::new();
        if let Some(missions) = self.battle_pass.get(&killer.raw()) {
            for (kind, rows) in missions {
                for row in rows {
                    let data = table.mission(row.mission_id);
                    if data.mission_type != MISSION_TYPE_KILL {
                        continue;
                    }
                    if data.specific == MISSION_TYPE_SPECIFIC && !data.vnums.contains(&race) {
                        continue;
                    }
                    hits.push((row.mission_id, *kind));
                }
            }
        }
        for (mission, kind) in hits {
            let _rows = self.battle_pass_action(killer, mover, mission, kind, 1);
        }
    }

    /// `RegisterItemMission` (`:299-324`): count an item on every mission of
    /// `mission` the buckets hold, matching the vnum for `SPECIFIC` missions.
    pub(super) fn register_pass_item(
        &mut self,
        vid: Vid,
        mover: Mover,
        mission: u8,
        points: u32,
        vnum: u32,
    ) {
        let Some(level) = self.pass_level(vid) else {
            return;
        };
        if level < MIN_LEVEL {
            return;
        }
        let table = self.pass_table();
        let mut hits = Vec::new();
        if let Some(missions) = self.battle_pass.get(&vid.raw()) {
            for (kind, rows) in missions {
                for row in rows {
                    let data = table.mission(row.mission_id);
                    if data.mission_type != mission {
                        continue;
                    }
                    if data.specific == MISSION_TYPE_SPECIFIC && !data.vnums.contains(&vnum) {
                        continue;
                    }
                    hits.push((row.mission_id, *kind));
                }
            }
        }
        for (id, kind) in hits {
            let _rows = self.battle_pass_action(vid, mover, id, kind, points);
        }
    }

    /// `do_battle_pass` (`G/cmd_general.cpp:4184-4223`): fewer than two words
    /// tells the usage; `open` opens once (the parsed `value` is unused) and
    /// sends the init record; `premium` runs the premium arm; any other word
    /// falls out of the `switch` silently. The match is a prefix of the table
    /// word (`strncmp(arg1, battlepass[i], len)`): an overlong word never
    /// matches, and an empty word returns at the usage.
    pub(super) fn run_battle_pass_gm_line(
        &mut self,
        vid: Vid,
        rest: &[u8],
        mover: Mover,
    ) -> Option<GmRows> {
        let (first, second) = two_arguments(rest);
        if first.is_empty() || second.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Usage: battle_pass <field> <value>",
                &[],
            );
            return None;
        }
        if sub_matches(b"open", &first) {
            let _value = int_word(&second).unwrap_or(0);
            if self.battle_pass_open.contains(&vid.raw()) {
                return None;
            }
            self.battle_pass_open.insert(vid.raw());
            self.check_pass_missions(vid, MISSION_TIME_WEEKLY);
            self.check_pass_missions(vid, MISSION_TIME_DAILY);
            self.send_battle_pass(vid, mover, SUB_INIT);
            return None;
        }
        if sub_matches(b"premium", &first) {
            return self.set_premium_pass(vid, mover);
        }
        None
    }

    /// `SetPremiumBattlePass` (`G/char.cpp:12949-12974`): without the 30018
    /// ticket the caller hears the refusal; else the premium flag lands, the
    /// update rides, the retro tiers (guarded by the list length) land, and
    /// one ticket leaves. A caller already premium keeps its ticket.
    pub(super) fn set_premium_pass(&mut self, vid: Vid, mover: Mover) -> Option<GmRows> {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"You don't have premium battle pass ticket.",
                &[],
            );
            return None;
        };
        let usable = usable_cells_of(character.inven_point());
        let held = world::character::count_specify_item(character.items(), usable, PREMIUM_TICKET);
        if held < 1 {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"You don't have premium battle pass ticket.",
                &[],
            );
            return None;
        }
        if self.pass_flag(vid, PREMIUM_FLAG) != 0 {
            return None;
        }
        self.set_pass_flag(vid, PREMIUM_FLAG, 1);
        self.send_battle_pass(vid, mover, SUB_UPDATE);
        let levels = self.pass_flag(vid, POINTS_FLAG) / POINTS_PER_LEVEL;
        let tiers: Vec<(u32, u32)> = {
            let table = self.pass_table();
            table
                .settings
                .rewards_premium
                .iter()
                .take(usize::try_from(levels.max(0)).unwrap_or(usize::MAX))
                .map(|(vnum, count)| (bits(*vnum), bits(*count)))
                .collect()
        };
        for tier in tiers {
            self.pass_give(vid, mover, tier);
        }
        let character = self.characters.find_by_vid_mut(vid).ok()?;
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let done = world::character::remove_specify_item(
            items,
            slots,
            &self.protos,
            usable,
            (PREMIUM_TICKET, 1),
        );
        let pid = self.characters.find_by_vid(vid).ok()?.player_id();
        let packed = crate::item_move::MovedItems::new(
            pid,
            vid.raw(),
            done,
            mover,
            &self.protos,
            &self.locale,
        );
        for record in packed.records {
            let _sent = self.write_to_client(vid, record);
        }
        let character = self.characters.find_by_vid(vid).ok()?;
        Some(empty_pass_rows(pid, character.points().cloned()))
    }

    /// `do_battle_pass_admin` (`G/cmd_general.cpp:4232-4294`): fewer than two
    /// words tells the usage; `completemission` tops every held row of the
    /// named id up; `getmissions` lists every held row; `resetMissions` clears
    /// and re-deals, then sends the init record. Any other word falls out of
    /// the `switch` silently. The second word is required even for
    /// `getmissions` and `resetMissions`.
    pub(super) fn run_battle_pass_admin_gm_line(
        &mut self,
        vid: Vid,
        rest: &[u8],
        mover: Mover,
    ) -> Option<GmRows> {
        let (first, second) = two_arguments(rest);
        if first.is_empty() || second.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Usage: battle_pass_admin <field> <value>",
                &[],
            );
            return None;
        }
        if sub_matches(b"completemission", &first) {
            let wanted = int_word(&second).unwrap_or(0);
            let table = self.pass_table();
            let mut hits = Vec::new();
            if let Some(missions) = self.battle_pass.get(&vid.raw()) {
                for (kind, rows) in missions {
                    for row in rows {
                        if row.mission_id == bits16(wanted) {
                            let data = table.mission(row.mission_id);
                            let left = data.max_progress.saturating_sub(row.progress);
                            hits.push((row.mission_id, *kind, left));
                        }
                    }
                }
            }
            for (mission, kind, left) in hits {
                let _rows = self.battle_pass_action(vid, mover, mission, kind, left);
            }
            return None;
        }
        if sub_matches(b"getmissions", &first) {
            let (daily, weekly) = self.pass_rows(vid);
            for row in daily.iter().chain(weekly.iter()) {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"MissionID: %d, Type: %d, Progress: %d",
                    &[
                        Arg::Int(i64::from(row.mission_id)),
                        Arg::Int(i64::from(row.kind)),
                        Arg::Int(i64::from(row.progress)),
                    ],
                );
            }
            return None;
        }
        if sub_matches(b"resetMissions", &first) {
            self.clear_pass_missions(vid);
            self.send_battle_pass(vid, mover, SUB_INIT);
        }
        None
    }

    /// `ClearBattlePass` (`G/char.cpp:12976-12987`): clear both buckets and
    /// re-deal them, weekly before daily. Legacy returns early with no
    /// descriptor; the caller is always described here.
    pub(super) fn clear_pass_missions(&mut self, vid: Vid) {
        self.battle_pass.remove(&vid.raw());
        self.check_pass_missions(vid, MISSION_TIME_WEEKLY);
        self.check_pass_missions(vid, MISSION_TIME_DAILY);
    }

    /// The stored mission rows a save writes (`CreatePlayerProto`,
    /// `G/char.cpp:1635-1650`): the kept ten, in slot order.
    pub(super) fn kept_pass_stored(&self, vid: Vid) -> Vec<db::battle_pass::StoredPassMission> {
        self.kept_pass_missions(vid)
            .iter()
            .enumerate()
            .filter(|(_, row)| row.end_time != 0)
            .map(|(slot, row)| db::battle_pass::StoredPassMission {
                slot: u8::try_from(slot).unwrap_or(u8::MAX),
                mission_id: row.mission_id,
                progress: row.progress,
                kind: row.kind,
                end_time: row.end_time,
            })
            .collect()
    }

    /// Install the loaded mission rows (`GetBattlePass`, `G/char.cpp:2434-2436`).
    pub(super) fn install_pass_loaded(
        &mut self,
        vid: Vid,
        stored: &[db::battle_pass::StoredPassMission],
    ) {
        let rows: Vec<PlayerMission> = stored
            .iter()
            .map(|row| PlayerMission {
                mission_id: row.mission_id,
                progress: row.progress,
                kind: row.kind,
                end_time: row.end_time,
            })
            .collect();
        self.install_pass_missions(vid, &rows);
    }
}

/// `strncmp(arg1, word, len)` with `len = strlen(arg1)`: the word starts with
/// the whole first argument.
fn sub_matches(word: &[u8], arg: &[u8]) -> bool {
    arg.len() <= word.len() && word[..arg.len()] == *arg
}

/// The mission end for a fresh row (`CheckBattlePassMissions`,
/// `G/char.cpp:12777-12780`): the wall-clock end second.
fn mission_end(time_info: u8, now: i32) -> u32 {
    let stamp = unsigned(now);
    if time_info == MISSION_TIME_DAILY {
        stamp.saturating_add(gamedata::battle_pass::seconds_until_midnight(stamp))
    } else {
        stamp.saturating_add(gamedata::battle_pass::seconds_until_end_of_week(stamp))
    }
}

/// `battlepassSettings.endTime - get_global_time()`: signed 32-bit, as the x86
/// `time_t` subtraction sees it.
fn expiration_left(end_time: u32, now: u32) -> i32 {
    i32::from_ne_bytes(end_time.wrapping_sub(now).to_ne_bytes())
}

/// A signed wall second as the unsigned stamp it names.
fn unsigned(wall: i32) -> u32 {
    u32::from_ne_bytes(wall.to_ne_bytes())
}

/// `time(0)` as the 32-bit `long` legacy's `get_global_time()` returns.
fn wall_now() -> i32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u32::try_from(since.as_secs()).unwrap_or(u32::MAX)
        });
    i32::from_ne_bytes(secs.to_ne_bytes())
}

/// One reward tier as unsigned item facts: the bits as legacy's
/// `std::pair<int32_t, int32_t>` leaves them.
fn tier_of(tiers: &[(i32, i32)], level: i32) -> Option<(u32, u32)> {
    let (vnum, count) = tiers.get(usize::try_from(level).unwrap_or(usize::MAX))?;
    Some((bits(*vnum), bits(*count)))
}

/// Reinterpret signed bits unsigned.
fn bits(value: i32) -> u32 {
    u32::from_ne_bytes(value.to_ne_bytes())
}

/// `str_to_number(mission)` into the `uint8_t` comparison: the low 16 bits,
/// as the `missionInfo.missionID == mission` comparison sees them.
fn bits16(value: i32) -> u16 {
    u16::from_ne_bytes(value.to_ne_bytes()[..2].try_into().unwrap_or([0, 0]))
}

/// The usable base-inventory cells of an `inven_point` word.
fn usable_cells_of(inven_point: u16) -> u16 {
    common::item_slots::usable_inventory_cells(inven_point)
}

/// An empty row answer for a pass move that changed no item row.
fn empty_pass_rows(pid: u32, points: Option<world::character::Points>) -> GmRows {
    GmRows {
        pid,
        changes: Vec::new(),
        cleared_skills: Vec::new(),
        gold: 0,
        monarch: None,
        points,
        quickslots: None,
        buildings: Vec::new(),
        account: None,
    }
}

/// The pass hooks below run at the same order position as their legacy sites.
impl GameState {
    /// The kill hook (`Dead`, `G/char_battle.cpp:1270-1272`): the killer's
    /// race match counts one kill point.
    pub(super) fn pass_on_kill(&mut self, killer: Vid, mover: Mover, victim_race: u32) {
        self.register_pass_kill(killer, mover, victim_race);
    }

    /// The use-item hook (`UseItem`, `G/char_item.cpp:2822-2831`): a
    /// non-blend use counts one point, and vnum 40004 runs the premium arm
    /// and ends the use before the type switch.
    pub(super) fn pass_on_use_item(
        &mut self,
        vid: Vid,
        mover: Mover,
        vnum: u32,
        item_type: i32,
    ) -> bool {
        if item_type != ITEM_BLEND {
            self.register_pass_item(vid, mover, MISSION_TYPE_USE_ITEM, 1, vnum);
        }
        if vnum == PREMIUM_ITEM {
            let _rows = self.set_premium_pass(vid, mover);
            return true;
        }
        false
    }

    /// The sell hook (`CShopManager::Sell`, `G/shop_manager.cpp:573-574`):
    /// the sold count counts on the sold item.
    pub(super) fn pass_on_sell(&mut self, vid: Vid, mover: Mover, count: u16, vnum: u32) {
        self.register_pass_item(vid, mover, MISSION_TYPE_SELL, u32::from(count), vnum);
    }

    /// The craft hook (`CCubeManager`, `G/cuberenewal.cpp:458-459`): the made
    /// count counts on the made item.
    pub(super) fn pass_on_craft(&mut self, vid: Vid, mover: Mover, count: u16, vnum: u32) {
        self.register_pass_item(vid, mover, MISSION_TYPE_CRAFT, u32::from(count), vnum);
    }

    /// The drop hook (`CreateQuestDropItem` loop, `G/item_manager.cpp:1326-1327`):
    /// each drawn item's count counts on the killer.
    pub(super) fn pass_on_drop(&mut self, killer: Vid, mover: Mover, drops: &[(u32, u16)]) {
        for (vnum, count) in drops {
            self.register_pass_item(
                killer,
                mover,
                MISSION_TYPE_ITEM_DROP,
                u32::from(*count),
                *vnum,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tokio::sync::mpsc::UnboundedReceiver;

    use crate::game_state::fixtures::{a_world, enter, points_at, TestClock};
    use gamedata::battle_pass::MISSION_TYPE_MESSAGES;

    /// A table with one daily KILL mission and one weekly MESSAGES mission.
    fn wired() -> Arc<BattlePassTable> {
        use gamedata::battle_pass::BattlePassSettings;
        let mut missions = BTreeMap::new();
        missions.insert(
            0,
            BattlePassMission {
                id: 0,
                mission_type: MISSION_TYPE_KILL,
                specific: MISSION_TYPE_SPECIFIC,
                vnums: vec![101],
                max_progress: 2,
                time_info: MISSION_TIME_DAILY,
                points: 1300,
                name: b"Kill.".to_vec(),
            },
        );
        missions.insert(
            1,
            BattlePassMission {
                id: 1,
                mission_type: MISSION_TYPE_MESSAGES,
                specific: 1,
                vnums: Vec::new(),
                max_progress: 1,
                time_info: MISSION_TIME_WEEKLY,
                points: 7000,
                name: b"Shout.".to_vec(),
            },
        );
        Arc::new(BattlePassTable {
            settings: BattlePassSettings {
                max_daily: 5,
                max_weekly: 10,
                end_time: 2_000_000_000,
                rewards_free: vec![(27005, 1)],
                rewards_premium: vec![(27006, 1)],
            },
            missions,
        })
    }

    /// An entered level-30 caller with the table installed.
    fn caller(clock: &TestClock) -> (GameState, Vid, Mover, UnboundedReceiver<Vec<u8>>) {
        let mut world = a_world(clock).with_battle_pass_table(wired());
        let inbox = enter(&mut world, 0x6001, (3200, 3200), 10_000);
        world
            .characters
            .find_by_vid_mut(Vid::new(0x6001))
            .expect("entered")
            .set_points(Some(points_at(30, 10_000, 0)));
        let mover = world.own_mover(Vid::new(0x6001)).expect("entered");
        (world, Vid::new(0x6001), mover, inbox)
    }

    /// The update record's head and rows.
    fn head(bytes: &[u8]) -> (u8, bool, i32, Vec<PlayerMission>) {
        decode_update(bytes).expect("the record decodes")
    }

    /// Decode an `encode_update` record back into its head and rows.
    /// Test-only: production only sends.
    fn decode_update(bytes: &[u8]) -> Option<(u8, bool, i32, Vec<PlayerMission>)> {
        if bytes.len() < 9 || bytes[0] != HEADER_GC_BATTLE_PASS {
            return None;
        }
        let mut size = [0_u8; 2];
        size.copy_from_slice(&bytes[1..3]);
        if usize::from(u16::from_le_bytes(size)) != bytes.len() {
            return None;
        }
        let mut points = [0_u8; 4];
        points.copy_from_slice(&bytes[5..9]);
        let rest = &bytes[9..];
        if rest.len() % 11 != 0 {
            return None;
        }
        let mut rows = Vec::new();
        for chunk in rest.chunks_exact(11) {
            rows.push(PlayerMission::decode(chunk)?);
        }
        Some((bytes[3], bytes[4] != 0, i32::from_le_bytes(points), rows))
    }

    /// Whether `line` is a `GC_CHAT` record.
    fn is_chat(line: &[u8]) -> bool {
        line.first() == Some(&4)
    }

    impl PlayerMission {
        /// Decode 11 wire bytes.
        fn decode(bytes: &[u8]) -> Option<Self> {
            if bytes.len() != 11 {
                return None;
            }
            let mut id = [0_u8; 2];
            let mut progress = [0_u8; 4];
            let mut end = [0_u8; 4];
            id.copy_from_slice(&bytes[0..2]);
            progress.copy_from_slice(&bytes[2..6]);
            end.copy_from_slice(&bytes[7..11]);
            Some(Self {
                mission_id: u16::from_le_bytes(id),
                progress: u32::from_le_bytes(progress),
                kind: bytes[6],
                end_time: u32::from_le_bytes(end),
            })
        }
    }

    #[test]
    fn the_wire_is_byte_exact() {
        let bytes = encode_update(
            SUB_INIT,
            true,
            0x0102_0407,
            &[PlayerMission {
                mission_id: 0x0203,
                progress: 0x0506_0709,
                kind: 0,
                end_time: 0x0B0C_0D0E,
            }],
            &[],
        );
        assert_eq!(
            bytes,
            vec![
                238, 0x14, 0x00, 0x00, 0x01, 0x07, 0x04, 0x02, 0x01, 0x03, 0x02, 0x09, 0x07, 0x06,
                0x05, 0x00, 0x0E, 0x0D, 0x0C, 0x0B,
            ]
        );
        let (sub, premium, points, rows) = head(&bytes);
        assert_eq!((sub, premium, points), (SUB_INIT, true, 0x0102_0407));
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn open_deals_and_sends_the_init_record() {
        let clock = TestClock::default();
        let (mut world, vid, mover, mut inbox) = caller(&clock);
        world.run_battle_pass_gm_line(vid, b"open 1", mover);
        assert!(world.battle_pass_open.contains(&vid.raw()));
        let line = inbox.try_recv().expect("the init record");
        let (sub, _, _, rows) = head(&line);
        assert_eq!(sub, SUB_INIT);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.kind == MISSION_TIME_DAILY));
        assert!(rows.iter().any(|row| row.kind == MISSION_TIME_WEEKLY));
        world.run_battle_pass_gm_line(vid, b"open 1", mover);
        assert!(inbox.try_recv().is_err(), "no second record");
    }

    #[test]
    fn a_kill_counts_and_completes_with_its_notice() {
        let clock = TestClock::default();
        let (mut world, vid, mover, mut inbox) = caller(&clock);
        world.run_battle_pass_gm_line(vid, b"open 1", mover);
        while inbox.try_recv().is_ok() {}
        world.register_pass_kill(vid, mover, 101);
        let first = inbox.try_recv().expect("the progress record");
        let (_, _, _, rows) = head(&first);
        assert_eq!(
            rows.iter()
                .find(|row| row.mission_id == 0)
                .map(|row| row.progress),
            Some(1)
        );
        world.register_pass_kill(vid, mover, 101);
        let mut updates = 0;
        let mut chats = 0;
        while let Ok(line) = inbox.try_recv() {
            if line.first() == Some(&HEADER_GC_BATTLE_PASS) {
                let (_, _, _, rows) = head(&line);
                assert_eq!(
                    rows.iter()
                        .find(|row| row.mission_id == 0)
                        .map(|row| row.progress),
                    Some(2)
                );
                updates += 1;
            } else if is_chat(&line) {
                chats += 1;
            }
        }
        assert_eq!(updates, 1);
        // 1300 points never cross a 10000 tier, so only the completion notice
        // rides: the prize notice would make two.
        assert_eq!(chats, 1, "only the completion notice rides");
        world.register_pass_kill(vid, mover, 101);
        assert!(inbox.try_recv().is_err(), "a filled mission stays silent");
    }

    #[test]
    fn a_wrong_race_never_counts() {
        let clock = TestClock::default();
        let (mut world, vid, mover, mut inbox) = caller(&clock);
        world.run_battle_pass_gm_line(vid, b"open 1", mover);
        while inbox.try_recv().is_ok() {}
        world.register_pass_kill(vid, mover, 999);
        assert!(inbox.try_recv().is_err(), "nothing counts");
    }

    #[test]
    fn an_expired_pass_counts_nothing() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut table = wired();
        Arc::get_mut(&mut table)
            .expect("only holder")
            .settings
            .end_time = 100;
        world = world.with_battle_pass_table(table);
        let mut inbox = enter(&mut world, 0x6001, (3200, 3200), 10_000);
        world
            .characters
            .find_by_vid_mut(Vid::new(0x6001))
            .expect("entered")
            .set_points(Some(points_at(30, 10_000, 0)));
        let mover = world.own_mover(Vid::new(0x6001)).expect("entered");
        world.run_battle_pass_gm_line(Vid::new(0x6001), b"open 1", mover);
        while inbox.try_recv().is_ok() {}
        clock.set(1_000);
        world.register_pass_kill(Vid::new(0x6001), mover, 101);
        assert!(inbox.try_recv().is_err(), "the expired pass is silent");
    }

    #[test]
    fn expired_rows_are_redealt() {
        let clock = TestClock::default();
        let (mut world, vid, mover, mut inbox) = caller(&clock);
        world.run_battle_pass_gm_line(vid, b"open 1", mover);
        while inbox.try_recv().is_ok() {}
        clock.set(200_000);
        world.check_pass_missions(vid, MISSION_TIME_DAILY);
        let (daily, _) = world.pass_rows(vid);
        assert_eq!(daily.len(), 1);
        assert!(daily[0].end_time > 200_000);
        let _ = inbox;
    }

    #[test]
    fn the_admin_lines_list_complete_and_reset() {
        let clock = TestClock::default();
        let (mut world, vid, mover, mut inbox) = caller(&clock);
        world.run_battle_pass_gm_line(vid, b"open 1", mover);
        while inbox.try_recv().is_ok() {}
        world.run_battle_pass_admin_gm_line(vid, b"getmissions x", mover);
        let mut listed = 0;
        while inbox.try_recv().is_ok() {
            listed += 1;
        }
        assert_eq!(listed, 2);
        world.run_battle_pass_admin_gm_line(vid, b"completemission 0", mover);
        let mut update = None;
        let mut chats = 0;
        while let Ok(line) = inbox.try_recv() {
            if line.first() == Some(&HEADER_GC_BATTLE_PASS) {
                update = Some(line);
            } else {
                chats += 1;
            }
        }
        assert!(chats >= 1, "the completion notice rides");
        let (_, _, _, rows) = head(&update.expect("the update"));
        assert_eq!(
            rows.iter()
                .find(|row| row.mission_id == 0)
                .map(|row| row.progress),
            Some(2)
        );
        while inbox.try_recv().is_ok() {}
        world.run_battle_pass_admin_gm_line(vid, b"resetMissions x", mover);
        let line = inbox.try_recv().expect("the init record");
        let (sub, _, _, rows) = head(&line);
        assert_eq!(sub, SUB_INIT);
        assert!(rows.iter().all(|row| row.progress == 0));
    }

    #[test]
    fn the_usage_lines_tell_the_usage() {
        let clock = TestClock::default();
        let (mut world, vid, mover, mut inbox) = caller(&clock);
        world.run_battle_pass_gm_line(vid, b"", mover);
        let line = inbox.try_recv().expect("the usage");
        assert!(is_chat(&line), "the usage is a chat line");
        assert!(line.windows(26).any(|w| w == b"Usage: battle_pass <field>"));
        world.run_battle_pass_admin_gm_line(vid, b"getmissions", mover);
        let line = inbox.try_recv().expect("the usage");
        assert!(is_chat(&line), "the usage is a chat line");
        world.run_battle_pass_gm_line(vid, b"bogus 1", mover);
        assert!(inbox.try_recv().is_err(), "nothing unknown speaks");
    }
}
