//! The level line: `/level` (`ACMD(do_level)`, `G/cmd_gm.cpp:2740-2758`).
//!
//! [`GameState::run_level`] is `do_level`: an empty line answers `Syntax: level
//! <level>`; else the caller hears nothing and `ResetPoint`, `ClearSkill` and
//! `ClearSubSkill` run in that order. The level is `str_to_number(int)` clamped
//! to `1..=gPlayerMaxLevel` (the configured `game.max_level`, 120 on the
//! owner's data).
//!
//! `ResetPoint` (`G/char.cpp:7424-7452`) runs the `POINT_LEVEL` change, plants
//! the job's four starts in both arrays, rolls the random pools once each
//! (`(level - 1) * number(begin, end)`, the hit roll before the spell roll),
//! moves the stat counter to `(clamped level * 3) + level step`, computes,
//! refills both pools, and sends the points pair. The level change itself is
//! [`Points::reset_level`]: the cap refusal, the level, the conqueror-off gate
//! (a champion lowered below the max loses the level, the experience and the
//! sungma, with the gate's own compute and points pair riding the stream
//! first), the nested next-experience, and the level record. The gate's quest
//! `LevelUp` raise, the guild and party member levels, and the top-120 affect
//! are this module's: the reward system's tables are empty on the owner's data
//! (`legacy/README.md` drops `reward_global/` and `reward_solo/`), so its
//! `DoReward` runs nothing.
//!
//! `ClearSkill` (`G/char_skill.cpp:3942-3947`) moves the skill counter to `4 +
//! (level - 5)` and zeroes every skill but the eleven sub-skills, then computes
//! and sends byte 76. `ClearSubSkill` (`:3949-3976`) moves the sub-skill counter
//! to `level - 9` (a level below 10 moves nothing: the counter stays), zeroes
//! the eleven, then computes and sends byte 76 again. Both sends ride the
//! caller's outbox, so a `/level` always sends two skill-level records.
//!
//! [`LevelHeld`] is what the held row takes: the level, the four starts, the
//! random pools, the three counters, the full pools, the conqueror fields and
//! the sungma, the skills the clears zeroed (whose store rows the step
//! deletes), and whether the caller reached 120 on a moved level, which the
//! step answers with the top-player affect past the store's `player_top` rows.

use common::point_slot as point;
use common::vid::Vid;

use super::affect::Affect;
use super::whisper::str_to_int;
use super::GameState;
use crate::command::one_argument;
use crate::item_move::Mover;
use protocol::gc_chat::CHAT_TYPE_INFO;
use world::character::number;

/// The eighteen sub-skill vnums `ResetSkill` keeps and `ClearSubSkill` zeroes
/// (`s_adwSubSkillVnums`, `G/char_skill.cpp:35-54`, with the `ENABLE_NEW_PASSIVE_SKILL`
/// tail: 141 142 and the 164-166 new passives).
const SUB_SKILLS: [u8; 18] = [
    121, 122, 124, 126, 127, 128, 129, 130, 131, 137, 138, 139, 140, 141, 142, 164, 165, 166,
];

/// `TOP_PLAYER_MAX_LEVEL` (`common/prodomodefines.h:127`): the level whose
/// reaching grants the top-player affect, and below which the held rows fall.
/// Shared with the strip in `affect.rs`.
pub(super) const TOP_LEVEL: u8 = 120;

/// What the line reset: the held numbers the save writes, the skills the step
/// deletes, and whether the caller reached the top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelHeld {
    /// The new level.
    pub level: u8,
    /// The four starts, real.
    pub st: i32,
    /// The four starts, real.
    pub ht: i32,
    /// The four starts, real.
    pub dx: i32,
    /// The four starts, real.
    pub iq: i32,
    /// The random hit points the rolls set.
    pub random_hp: i32,
    /// The random spell points the rolls set.
    pub random_sp: i32,
    /// `POINT_STAT`.
    pub stat: i32,
    /// `POINT_SKILL`.
    pub skill: i32,
    /// `POINT_SUB_SKILL`.
    pub sub_skill: i32,
    /// The refilled hit points.
    pub hp: i32,
    /// The refilled spell points.
    pub sp: i32,
    /// `POINT_CONQUEROR_LEVEL`, 0 when the gate switched it off.
    pub conqueror_level: u8,
    /// The conqueror experience, 0 when the gate switched it off.
    pub conqueror_exp: i64,
    /// The four sungma, 0 when the gate switched them off.
    pub sungma: [i32; 4],
    /// The skill vnums the clears zeroed that held a level, whose store rows go.
    pub cleared_skills: Vec<u8>,
    /// The level moved and landed on 120 (`TOP_PLAYER_MAX_LEVEL`): the step
    /// grants the top-player affect past the store's rows.
    pub reached_top: bool,
    /// The player IDs whose `player_top` rows fell: the level moved off 120 and
    /// the strip below took their held top rows (`ComputeAffect`,
    /// `G/char_affect.cpp:811-826`), one per kind in kind order. The step
    /// deletes them from the store.
    pub fell_top: Vec<u32>,
}

/// What `/reset` refilled: the filled pools the held row takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetHeld {
    /// The refilled hit points.
    pub hp: i32,
    /// The refilled spell points.
    pub sp: i32,
}

/// What `/advance` landed: the target's shown name with the reset, for the
/// store's ID lookup and row write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvanceLanded {
    /// The target's name as the world shows it.
    pub target: String,
    /// The held numbers the save writes.
    pub held: LevelHeld,
}

impl GameState {
    /// `ACMD(do_advance)`: the named character's level becomes the line's,
    /// `ResetPoint` and the two clears in the same order as [`GameState::run_level`].
    /// Answers the target's shown name with the held numbers, or `None` for a
    /// line that runs nothing. No name or no level answers `Syntax: advance
    /// <name> <level>`; an offline name answers `<name> not exist`. The level
    /// clamps to `0..=max`, and a 0 lands the caller on 0: legacy's `MINMAX(0,
    /// level, max)`, whose 0 `do_level`'s 1 never plants. `gold` and `voice`
    /// are the caller's row's, which the points pair carries; the target's
    /// own row never crosses the world boundary.
    pub(super) fn run_advance(
        &mut self,
        vid: Vid,
        rest: &[u8],
        mover: Mover,
        gold: i64,
        voice: u8,
    ) -> Option<AdvanceLanded> {
        let (first, back) = one_argument(rest);
        let (second, _) = one_argument(back);
        if first.is_empty() || second.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Syntax: advance <name> <level>",
                &[],
            );
            return None;
        }
        let target = std::str::from_utf8(&first)
            .ok()
            .and_then(|name| self.whisper_target(name));
        let Some(target) = target else {
            let mut line = first.clone();
            line.extend_from_slice(b" not exist");
            self.tell(vid, mover, CHAT_TYPE_INFO, &line, &[]);
            return None;
        };
        let max = self.exp_limits.max_level;
        let new = str_to_int(&second).clamp(0, max);
        let Ok(new) = u8::try_from(new) else {
            return None;
        };
        let old = self.level_of(target)?;
        let (records, conqu_off) = self.pc_reset_level(target, new, max)?;
        self.send_point_records(target, &records);
        if conqu_off {
            let points = self.pc_points_mut(target.raw())?;
            let computed = points.compute_points();
            self.send_point_records(target, &computed);
            let points = self.pc_points_mut(target.raw())?;
            for frame in crate::loading_phase::mid_game_points(points, gold, voice) {
                let _sent = self.write_to_client(target, frame);
            }
        }
        if new != old {
            self.raise_quest_levelup(target);
            self.set_member_levels(target, new);
        }
        self.reset_stats(target, new, gold, voice);
        let mut cleared_skills = self.clear_skills(target, new);
        if let Some(body) = self.bodies.get_mut(&target) {
            body.card.level = new;
        }
        cleared_skills.extend(self.clear_sub_skills(target, new));
        // Below the top a held top row falls with the move (`ComputeAffect`,
        // `G/char_affect.cpp:811-826`); the strip's answer names the store rows
        // the step deletes.
        let fell_top = if new != old && new != TOP_LEVEL {
            self.strip_top_affects(target)
        } else {
            Vec::new()
        };
        let held = self.level_held(
            target,
            new != old && new == TOP_LEVEL,
            cleared_skills,
            fell_top,
        );
        let shown = self
            .characters
            .find_by_vid(target)
            .map_or_else(|_| String::new(), |found| found.name().to_string());
        Some(AdvanceLanded {
            target: shown,
            held,
        })
    }

    /// `ACMD(do_level)`: the caller's level becomes the line's, clamped to the
    /// max, with the stats, skills and records `ResetPoint` and the two clears
    /// write. Answers the held numbers, or `None` for a line that runs nothing
    /// (no word, which hears the syntax, or no character under `vid`). `gold`
    /// and `voice` are the row's, which the points pair carries and the world
    /// does not hold.
    pub(super) fn run_level(
        &mut self,
        vid: Vid,
        rest: &[u8],
        mover: Mover,
        gold: i64,
        voice: u8,
    ) -> Option<LevelHeld> {
        let (word, _) = one_argument(rest);
        if word.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Syntax: level <level>", &[]);
            return None;
        }
        let max = self.exp_limits.max_level;
        let new = str_to_int(&word).clamp(1, max);
        let Ok(new) = u8::try_from(new) else {
            return None;
        };
        let old = self.level_of(vid)?;
        let (records, conqu_off) = self.pc_reset_level(vid, new, max)?;
        self.send_point_records(vid, &records);
        if conqu_off {
            let points = self.pc_points_mut(vid.raw())?;
            let computed = points.compute_points();
            self.send_point_records(vid, &computed);
            let points = self.pc_points_mut(vid.raw())?;
            for frame in crate::loading_phase::mid_game_points(points, gold, voice) {
                let _sent = self.write_to_client(vid, frame);
            }
        }
        if new != old {
            self.raise_quest_levelup(vid);
            self.set_member_levels(vid, new);
        }
        self.reset_stats(vid, new, gold, voice);
        let mut cleared_skills = self.clear_skills(vid, new);
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.card.level = new;
        }
        cleared_skills.extend(self.clear_sub_skills(vid, new));
        let fell_top = if new != old && new != TOP_LEVEL {
            self.strip_top_affects(vid)
        } else {
            Vec::new()
        };
        Some(self.level_held(
            vid,
            new != old && new == TOP_LEVEL,
            cleared_skills,
            fell_top,
        ))
    }

    /// `ACMD(do_reset)`: the caller's hit and spell pools refill, the filled
    /// pools bank, and the party takes the member's info when the hit points
    /// moved. Answers the filled pools, or `None` for no character under
    /// `vid`. The line takes no arguments and says nothing.
    pub(super) fn run_reset(&mut self, vid: Vid) -> Option<ResetHeld> {
        let (hit, hit_max) = self
            .pc_points_mut(vid.raw())
            .map(|points| (points.hp(), points.max_hp()))?;
        let (spell, spell_max) = self
            .pc_points_mut(vid.raw())
            .map(|points| (points.sp(), points.max_sp()))?;
        self.change_pc_point(vid, point::POINT_HP, hit_max.saturating_sub(hit), false);
        self.broadcast_pc_target(vid);
        self.change_pc_point(vid, point::POINT_SP, spell_max.saturating_sub(spell), false);
        let filled = self
            .pc_points_mut(vid.raw())
            .map(|points| (points.hp(), points.sp()))?;
        if filled.0 != hit {
            if let Ok(character) = self.characters.find_by_vid(vid) {
                let pid = character.player_id();
                if let Some(leader) = self.party_leader_of(pid) {
                    self.send_party_info_one_to_all(leader, pid);
                }
            }
        }
        Some(ResetHeld {
            hp: filled.0,
            sp: filled.1,
        })
    }

    /// The level of the player under `vid`.
    pub(super) fn level_of(&mut self, vid: Vid) -> Option<u8> {
        self.pc_points_mut(vid.raw()).map(|points| points.level())
    }

    /// The `POINT_LEVEL` change of `ResetPoint` on the player under `vid`.
    fn pc_reset_level(
        &mut self,
        vid: Vid,
        new: u8,
        max: i32,
    ) -> Option<(Vec<world::character::PointRecord>, bool)> {
        self.pc_points_mut(vid.raw())?.reset_level(new, max)
    }

    /// The guild and party member rows of the player under `vid` carry `level`.
    fn set_member_levels(&mut self, vid: Vid, level: u8) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        self.set_guild_member_level(pid, level);
        self.set_party_member_level(pid, level);
    }

    /// `ResetPoint` past the level change: the four starts, the random rolls,
    /// the stat counter, the compute, the refill, and the points pair.
    fn reset_stats(&mut self, vid: Vid, new: u8, gold: i64, voice: u8) {
        let job = self.pc_job_starts(vid);
        let Some([st, ht, dx, iq]) = job else {
            return;
        };
        let bounds = self.pc_roll_bounds(vid);
        let Some((hp_begin, hp_end, sp_begin, sp_end)) = bounds else {
            return;
        };
        let hp = number(&mut self.dice, hp_begin, hp_end);
        let sp = number(&mut self.dice, sp_begin, sp_end);
        let limit = self.exp_limits.status_point_get_level_limit;
        let points = self.pc_points_mut(vid.raw());
        let Some(points) = points else {
            return;
        };
        points.set_both_points(point::POINT_ST, st);
        points.set_both_points(point::POINT_HT, ht);
        points.set_both_points(point::POINT_DX, dx);
        points.set_both_points(point::POINT_IQ, iq);
        let times = i32::from(new) - 1;
        points.set_random_hp(times.wrapping_mul(hp));
        points.set_random_sp(times.wrapping_mul(sp));
        let clamped = i32::from(new).clamp(1, limit);
        let step = points.get_point(point::POINT_LEVEL_STEP);
        let stat = clamped * 3 + step - points.get_point(point::POINT_STAT);
        let changed = points
            .point_change(point::POINT_STAT, stat, false, false)
            .unwrap_or_default();
        self.send_point_records(vid, &changed);
        let points = self.pc_points_mut(vid.raw());
        let Some(points) = points else {
            return;
        };
        let computed = points.compute_points();
        self.send_point_records(vid, &computed);
        let points = self.pc_points_mut(vid.raw());
        let Some(points) = points else {
            return;
        };
        let hp_refill = points
            .point_change(point::POINT_HP, points.max_hp() - points.hp(), false, false)
            .unwrap_or_default();
        self.send_point_records(vid, &hp_refill);
        let points = self.pc_points_mut(vid.raw());
        let Some(points) = points else {
            return;
        };
        let sp_refill = points
            .point_change(point::POINT_SP, points.max_sp() - points.sp(), false, false)
            .unwrap_or_default();
        self.send_point_records(vid, &sp_refill);
        let points = self.pc_points_mut(vid.raw());
        let Some(points) = points else {
            return;
        };
        for frame in crate::loading_phase::mid_game_points(points, gold, voice) {
            let _sent = self.write_to_client(vid, frame);
        }
    }

    /// The job's four starts of the player under `vid`.
    fn pc_job_starts(&mut self, vid: Vid) -> Option<[i32; 4]> {
        let points = self.pc_points_mut(vid.raw())?;
        Some([
            i32::from(points.job_start(point::POINT_ST)),
            i32::from(points.job_start(point::POINT_HT)),
            i32::from(points.job_start(point::POINT_DX)),
            i32::from(points.job_start(point::POINT_IQ)),
        ])
    }

    /// The hit and spell roll bounds of the job of the player under `vid`.
    fn pc_roll_bounds(&mut self, vid: Vid) -> Option<(i32, i32, i32, i32)> {
        let points = self.pc_points_mut(vid.raw())?;
        let job = common::levels::JOB_INITIAL_POINTS[usize::from(points.race() % 4)];
        Some((
            job.hp_per_lv_begin,
            job.hp_per_lv_end,
            job.sp_per_lv_begin,
            job.sp_per_lv_end,
        ))
    }

    /// `ClearSkill`: the skill counter moves and every skill but the eleven
    /// sub-skills zeroes, then the compute and byte 76. Answers the vnums the
    /// clear zeroed that held a level.
    pub(super) fn clear_skills(&mut self, vid: Vid, new: u8) -> Vec<u8> {
        let points = self.pc_points_mut(vid.raw());
        let Some(points) = points else {
            return Vec::new();
        };
        let delta = 4 + (i32::from(new) - 5) - points.get_point(point::POINT_SKILL);
        let changed = points
            .point_change(point::POINT_SKILL, delta, false, false)
            .unwrap_or_default();
        self.send_point_records(vid, &changed);
        let mut cleared = Vec::new();
        let state = self.pc_skills.entry(vid.raw()).or_default();
        for (slot, level) in state.levels.iter_mut().enumerate() {
            let Ok(vnum) = u8::try_from(slot) else {
                continue;
            };
            if SUB_SKILLS.contains(&vnum) {
                continue;
            }
            if *level != 0 {
                cleared.push(vnum);
            }
            *level = 0;
        }
        let points = self.pc_points_mut(vid.raw());
        let Some(points) = points else {
            return cleared;
        };
        let computed = points.compute_points();
        self.send_point_records(vid, &computed);
        self.send_skill_levels(vid);
        cleared
    }

    /// `ClearSubSkill`: the sub-skill counter moves past level 9 and the eighteen
    /// zero, then the compute and byte 76. Answers the vnums the two clears
    /// zeroed that held a level.
    pub(super) fn clear_sub_skills(&mut self, vid: Vid, new: u8) -> Vec<u8> {
        let points = self.pc_points_mut(vid.raw());
        let Some(points) = points else {
            return Vec::new();
        };
        let delta = if new < 10 {
            0
        } else {
            i32::from(new) - 9 - points.get_point(point::POINT_SUB_SKILL)
        };
        let changed = points
            .point_change(point::POINT_SUB_SKILL, delta, false, false)
            .unwrap_or_default();
        self.send_point_records(vid, &changed);
        let mut cleared = Vec::new();
        let state = self.pc_skills.entry(vid.raw()).or_default();
        for vnum in SUB_SKILLS {
            if state.levels[usize::from(vnum)] != 0 {
                cleared.push(vnum);
            }
            state.levels[usize::from(vnum)] = 0;
        }
        let points = self.pc_points_mut(vid.raw());
        let Some(points) = points else {
            return cleared;
        };
        let computed = points.compute_points();
        self.send_point_records(vid, &computed);
        self.send_skill_levels(vid);
        cleared
    }

    /// `SetTopPlayerEffect` (`G/char.cpp:11255-11284`): the `rank`th top-player
    /// affect (1 to 3) lands with override. The caller counted the store's
    /// `player_top` rows and inserted this caller's.
    pub(super) fn grant_top_affect(&mut self, vid: Vid, rank: u8) {
        let (kind, flag) = match rank {
            1 => (
                crate::loading_phase::AFFECT_PLAYER_TOP_1,
                crate::loading_phase::AFF_PLAYER_TOP_1,
            ),
            2 => (
                crate::loading_phase::AFFECT_PLAYER_TOP_2,
                crate::loading_phase::AFF_PLAYER_TOP_2,
            ),
            _ => (
                crate::loading_phase::AFFECT_PLAYER_TOP_3,
                crate::loading_phase::AFF_PLAYER_TOP_3,
            ),
        };
        self.add_pc_affect(
            vid,
            Affect {
                kind,
                apply_on: u8::try_from(point::POINT_NONE).unwrap_or(u8::MAX),
                value: 0,
                flag,
                remaining: crate::loading_phase::INFINITE_AFFECT_DURATION,
                sp_cost: 0,
            },
            true,
        );
    }

    /// The held numbers of the player under `vid` after the reset.
    fn level_held(
        &mut self,
        vid: Vid,
        reached_top: bool,
        cleared: Vec<u8>,
        fell_top: Vec<u32>,
    ) -> LevelHeld {
        let points = self.pc_points_mut(vid.raw());
        let Some(points) = points else {
            return LevelHeld {
                level: 1,
                st: 0,
                ht: 0,
                dx: 0,
                iq: 0,
                random_hp: 0,
                random_sp: 0,
                stat: 0,
                skill: 0,
                sub_skill: 0,
                hp: 0,
                sp: 0,
                conqueror_level: 0,
                conqueror_exp: 0,
                sungma: [0; 4],
                cleared_skills: cleared,
                reached_top,
                fell_top: Vec::new(),
            };
        };
        LevelHeld {
            level: points.level(),
            st: points.real_point(point::POINT_ST),
            ht: points.real_point(point::POINT_HT),
            dx: points.real_point(point::POINT_DX),
            iq: points.real_point(point::POINT_IQ),
            random_hp: points.random_hp(),
            random_sp: points.random_sp(),
            stat: points.get_point(point::POINT_STAT),
            skill: points.get_point(point::POINT_SKILL),
            sub_skill: points.get_point(point::POINT_SUB_SKILL),
            hp: points.hp(),
            sp: points.sp(),
            conqueror_level: points.conqueror_level(),
            conqueror_exp: points.conqueror_exp(),
            sungma: [
                points.get_point(point::POINT_SUNGMA_STR),
                points.get_point(point::POINT_SUNGMA_HP),
                points.get_point(point::POINT_SUNGMA_MOVE),
                points.get_point(point::POINT_SUNGMA_IMMUNE),
            ],
            cleared_skills: cleared,
            reached_top,
            fell_top,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LevelHeld, SUB_SKILLS, TOP_LEVEL};
    use crate::game_state::fixtures::{a_world, enter, TestClock};
    use common::point_slot as point;
    use common::vid::Vid;

    const VID: u32 = 0x6001;

    /// The point-change kinds of the inbox after one line: a `/level` sends
    /// point changes, the points pair, and two skill-level records, and only
    /// the 25-byte header-17 frames carry kinds.
    fn change_kinds(line: &[Vec<u8>]) -> Vec<u8> {
        line.iter()
            .filter(|frame| {
                frame.len() == 25
                    && frame[0..4] == 17i32.to_le_bytes()
                    && frame[4..8] == VID.to_le_bytes()
            })
            .map(|frame| frame[8])
            .collect()
    }

    fn entered(
        world: &mut crate::game_state::GameState,
    ) -> tokio::sync::mpsc::UnboundedReceiver<Vec<u8>> {
        let mut inbox = enter(world, VID, (3200, 3200), 0);
        while inbox.try_recv().is_ok() {}
        inbox
    }

    /// An empty line hears the syntax and resets nothing.
    #[test]
    fn a_level_answers_its_syntax_and_changes_nothing() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = entered(&mut world);
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        assert!(world.run_level(Vid::new(VID), b"", mover, 0, 0).is_none());
        let line = inbox.try_recv().expect("the syntax");
        assert!(
            line.windows(b"Syntax: level <level>".len())
                .any(|window| window == b"Syntax: level <level>"),
            "the syntax line"
        );
        assert!(inbox.try_recv().is_err(), "nothing else");
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        assert_eq!(points.level(), 10, "the level stays");
    }

    /// A warrior to 30: the starts, the rolls' ranges, the three counters, the
    /// full pools, and the cleared skills named.
    #[test]
    fn a_level_resets_the_stats_and_skills() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_max_level(120);
        let mut inbox = entered(&mut world);
        world.set_pc_skill_level(VID, 16, 20);
        world.set_pc_skill_level(VID, 121, 5);
        while inbox.try_recv().is_ok() {}
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        let held = world
            .run_level(Vid::new(VID), b"30", mover, 1000, 0)
            .expect("30 resets");
        assert_eq!(held.level, 30);
        assert_eq!(
            [held.st, held.ht, held.dx, held.iq],
            [6, 4, 3, 3],
            "the warrior starts"
        );
        assert!(
            (29 * 36..=29 * 44).contains(&held.random_hp),
            "the hit roll: {}",
            held.random_hp
        );
        assert!(
            (29 * 18..=29 * 22).contains(&held.random_sp),
            "the spell roll: {}",
            held.random_sp
        );
        assert_eq!(held.stat, 90, "30 levels of 3");
        assert_eq!(held.skill, 29, "4 + (30 - 5)");
        assert_eq!(held.sub_skill, 21, "30 - 9");
        assert_eq!(
            held.hp,
            world.pc_points_mut(VID).expect("points").max_hp(),
            "the refill"
        );
        assert_eq!(
            held.sp,
            world.pc_points_mut(VID).expect("points").max_sp(),
            "the refill"
        );
        assert_eq!(held.conqueror_level, 0);
        assert_eq!(held.sungma, [0; 4]);
        assert!(!held.reached_top);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        assert_eq!(points.level(), 30);
        assert_eq!(points.real_point(point::POINT_ST), 6);
        assert_eq!(points.get_point(point::POINT_STAT), 90);
        assert_eq!(points.get_point(point::POINT_SKILL), 29);
        assert_eq!(points.get_point(point::POINT_SUB_SKILL), 21);
        assert_eq!(world.pc_skill_level(VID, 16), 0, "the skill zeroes");
        assert_eq!(world.pc_skill_level(VID, 121), 0, "the sub-skill zeroes");
        let body = world.bodies.get(&Vid::new(VID)).expect("the body");
        assert_eq!(body.card.level, 30, "the card follows");
        let mut frames = Vec::new();
        while let Ok(frame) = inbox.try_recv() {
            frames.push(frame);
        }
        let kinds = change_kinds(&frames);
        for slot in [
            point::POINT_LEVEL,
            point::POINT_NEXT_EXP,
            point::POINT_STAT,
            point::POINT_SKILL,
            point::POINT_SUB_SKILL,
            point::POINT_HP,
            point::POINT_SP,
        ] {
            assert!(
                kinds.contains(&u8::try_from(slot).expect("a slot")),
                "slot {slot}: {kinds:?}"
            );
        }
        assert_eq!(
            frames.iter().filter(|frame| frame[0] == 16).count(),
            1,
            "one points record"
        );
        assert_eq!(
            frames.iter().filter(|frame| frame[0] == 76).count(),
            2,
            "two skill records"
        );
    }

    /// The line clamps to the max and the floor.
    #[test]
    fn a_level_clamps_to_the_max_and_the_floor() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_max_level(120);
        let mut inbox = entered(&mut world);
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        let held = world
            .run_level(Vid::new(VID), b"999", mover, 0, 0)
            .expect("999 clamps");
        assert_eq!(held.level, 120);
        assert!(held.reached_top, "the clamp reaches the top");
        assert_eq!(held.skill, 119, "4 + (120 - 5)");
        assert_eq!(held.stat, 270, "90 levels of 3: the get limit clamps");
        while inbox.try_recv().is_ok() {}
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        let held = world
            .run_level(Vid::new(VID), b"-5", mover, 0, 0)
            .expect("-5 clamps");
        assert_eq!(held.level, 1);
        assert_eq!(held.stat, 3, "one level of 3");
        assert_eq!(held.skill, 0, "4 + (1 - 5)");
        assert_eq!(held.sub_skill, 111, "below 10 the counter stays: 120 - 9");
        assert_eq!(held.random_hp, 0, "no level, no roll");
        assert_eq!(held.random_sp, 0, "no level, no roll");
    }

    /// The same level still resets and clears, but raises no quest scripts and
    /// names no top.
    #[test]
    fn a_same_level_still_clears_but_raises_nothing() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_max_level(120);
        let mut inbox = entered(&mut world);
        world.set_pc_skill_level(VID, 16, 20);
        while inbox.try_recv().is_ok() {}
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        let held = world
            .run_level(Vid::new(VID), b"10", mover, 0, 0)
            .expect("the same level runs");
        assert_eq!(held.level, 10);
        assert!(!held.reached_top);
        assert_eq!(world.pc_skill_level(VID, 16), 0, "the skill still zeroes");
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        assert_eq!(
            points.get_point(point::POINT_STAT),
            30,
            "the stat still moves"
        );
        let mut frames = Vec::new();
        while let Ok(frame) = inbox.try_recv() {
            frames.push(frame);
        }
        assert!(!frames.is_empty(), "the records still ride");
    }

    /// A champion lowered below the max loses the conqueror: the level, the
    /// experience, the sungma, and their records.
    #[test]
    fn a_lowered_champion_loses_the_conqueror() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_max_level(120);
        let mut inbox = entered(&mut world);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _records = points.become_conqueror(30);
        points.set_both_points(point::POINT_SUNGMA_STR, 5);
        points.set_both_points(point::POINT_CONQUEROR_POINT, 3);
        while inbox.try_recv().is_ok() {}
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        let held = world
            .run_level(Vid::new(VID), b"119", mover, 0, 0)
            .expect("119 runs");
        assert_eq!(held.level, 119);
        assert_eq!(held.conqueror_level, 0);
        assert_eq!(held.conqueror_exp, 0);
        assert_eq!(held.sungma, [0; 4], "the sungma zeroes");
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        assert_eq!(points.conqueror_level(), 0);
        assert_eq!(points.get_point(point::POINT_CONQUEROR_POINT), 0);
        let mut frames = Vec::new();
        while let Ok(frame) = inbox.try_recv() {
            frames.push(frame);
        }
        let kinds = change_kinds(&frames);
        assert!(
            kinds.contains(&u8::try_from(point::POINT_CONQUEROR_LEVEL).expect("a slot")),
            "the conqueror change: {kinds:?}"
        );
        assert!(
            kinds.contains(&u8::try_from(point::POINT_CONQUEROR_EXP).expect("a slot")),
            "the conqueror experience: {kinds:?}"
        );
        assert_eq!(
            frames.iter().filter(|frame| frame[0] == 16).count(),
            2,
            "the gate's pair and the reset's"
        );
    }

    /// At the max a champion keeps the conqueror: the gate fires only below it.
    #[test]
    fn at_the_max_a_champion_keeps_the_conqueror() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_max_level(120);
        let mut inbox = entered(&mut world);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _records = points.become_conqueror(30);
        while inbox.try_recv().is_ok() {}
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        let held = world
            .run_level(Vid::new(VID), b"120", mover, 0, 0)
            .expect("120 runs");
        assert_eq!(held.conqueror_level, 1, "the level stays");
        assert!(held.reached_top);
    }

    /// A reach of 120 on a moved level names the top; below it and on the same
    /// level it stays quiet.
    #[test]
    fn a_reach_of_120_names_the_top() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_max_level(120);
        let mut inbox = entered(&mut world);
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        let held = world
            .run_level(Vid::new(VID), b"119", mover, 0, 0)
            .expect("119 runs");
        assert!(!held.reached_top, "below the top");
        while inbox.try_recv().is_ok() {}
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        let held = world
            .run_level(Vid::new(VID), b"120", mover, 0, 0)
            .expect("120 runs");
        assert!(held.reached_top, "the reach");
        while inbox.try_recv().is_ok() {}
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        let held = world
            .run_level(Vid::new(VID), b"120", mover, 0, 0)
            .expect("120 again runs");
        assert!(!held.reached_top, "the same level raises nothing");
    }

    /// A move off 120 fells the held top row (`ComputeAffect`,
    /// `G/char_affect.cpp:811-826`): the 321 row's remove rides and the held
    /// answer names the fallen player for the store.
    #[test]
    fn a_move_off_120_fells_the_held_top_row() {
        use crate::loading_phase::AFFECT_PLAYER_TOP_1;
        use protocol::gc_vid::HEADER_GC_AFFECT_REMOVE;
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_max_level(120);
        let mut inbox = entered(&mut world);
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        world
            .run_level(Vid::new(VID), b"120", mover, 0, 0)
            .expect("120 runs");
        world.grant_top_affect(Vid::new(VID), 1);
        while inbox.try_recv().is_ok() {}
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        let held = world
            .run_level(Vid::new(VID), b"119", mover, 0, 0)
            .expect("119 runs");
        assert!(!held.reached_top, "below the top");
        let heard: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        let removes: Vec<u32> = heard
            .iter()
            .filter(|frame| frame.first() == Some(&HEADER_GC_AFFECT_REMOVE))
            .map(|frame| u32::from_le_bytes(frame[1..5].try_into().unwrap()))
            .collect();
        assert_eq!(removes, [AFFECT_PLAYER_TOP_1]);
        assert_eq!(held.fell_top.len(), 1, "one row fell");
    }

    /// The eighteen sub-skills are the ones legacy keeps through `ResetSkill`.
    #[test]
    fn the_sub_skills_are_the_eighteen() {
        assert_eq!(
            SUB_SKILLS,
            [
                121, 122, 124, 126, 127, 128, 129, 130, 131, 137, 138, 139, 140, 141, 142, 164,
                165, 166,
            ]
        );
        assert_eq!(TOP_LEVEL, 120);
        let _ = LevelHeld {
            level: 1,
            st: 0,
            ht: 0,
            dx: 0,
            iq: 0,
            random_hp: 0,
            random_sp: 0,
            stat: 0,
            skill: 0,
            sub_skill: 0,
            hp: 0,
            sp: 0,
            conqueror_level: 0,
            conqueror_exp: 0,
            sungma: [0; 4],
            cleared_skills: Vec::new(),
            reached_top: false,
            fell_top: Vec::new(),
        };
    }

    /// A missing word hears the syntax; an offline name hears the miss with
    /// the name as the line spelled it; and neither resets anyone.
    #[test]
    fn an_advance_answers_its_syntax_and_its_miss() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_max_level(120);
        let mut inbox = entered(&mut world);
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        assert!(world.run_advance(Vid::new(VID), b"", mover, 0, 0).is_none());
        let line = inbox.try_recv().expect("the syntax");
        assert!(
            line.windows(b"Syntax: advance <name> <level>".len())
                .any(|window| window == b"Syntax: advance <name> <level>"),
            "the syntax line"
        );
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        assert!(world
            .run_advance(Vid::new(VID), b"P24577", mover, 0, 0)
            .is_none());
        let line = inbox.try_recv().expect("the syntax");
        assert!(
            line.windows(b"Syntax: advance <name> <level>".len())
                .any(|window| window == b"Syntax: advance <name> <level>"),
            "the syntax line"
        );
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        assert!(world
            .run_advance(Vid::new(VID), b"Nobody 30", mover, 0, 0)
            .is_none());
        let line = inbox.try_recv().expect("the miss");
        assert!(
            line.windows(b"Nobody not exist".len())
                .any(|window| window == b"Nobody not exist"),
            "the miss line"
        );
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        assert_eq!(points.level(), 10, "the level stays");
    }

    /// The named character resets and answers the shown name with the held
    /// numbers; the caller hears nothing more. A 0 lands: `do_advance`
    /// clamps to `0..=max` where `do_level` starts at 1.
    #[test]
    fn an_advance_resets_the_named_character() {
        const OTHER: u32 = 0x6002;
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_max_level(120);
        let mut inbox = entered(&mut world);
        let mut other = enter(&mut world, OTHER, (3300, 3300), 0);
        while inbox.try_recv().is_ok() {}
        while other.try_recv().is_ok() {}
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        let landed = world
            .run_advance(Vid::new(VID), b"p24578 30", mover, 0, 0)
            .expect("30 runs");
        assert_eq!(landed.target, format!("P{OTHER}"), "the shown name");
        assert_eq!(landed.held.level, 30);
        assert_eq!(landed.held.stat, 90, "30 levels of 3");
        let points = world.pc_points_mut(OTHER).expect("the target has points");
        assert_eq!(points.level(), 30);
        let points = world.pc_points_mut(VID).expect("the caller has points");
        assert_eq!(points.level(), 10, "the caller stays");
        assert!(inbox.try_recv().is_err(), "the caller hears nothing more");
        let mut frames = Vec::new();
        while let Ok(frame) = other.try_recv() {
            frames.push(frame);
        }
        assert!(!frames.is_empty(), "the target hears the reset");
        while other.try_recv().is_ok() {}
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        let landed = world
            .run_advance(Vid::new(VID), format!("P{OTHER} 0").as_bytes(), mover, 0, 0)
            .expect("0 runs");
        assert_eq!(landed.held.level, 0, "a 0 lands");
        let points = world.pc_points_mut(OTHER).expect("the target has points");
        assert_eq!(points.level(), 0);
    }

    /// A wounded caller refills both pools and answers the filled numbers; the
    /// line says nothing. The hit change rides first, then the spell change.
    #[test]
    fn a_reset_refills_the_pools_and_answers_the_filled_numbers() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = entered(&mut world);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let (hit_max, spell_max) = (points.max_hp(), points.max_sp());
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _wound = points
            .point_change(point::POINT_HP, 1 - hit_max, false, false)
            .expect("the wound lands");
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _drain = points
            .point_change(point::POINT_SP, 1 - spell_max, false, false)
            .expect("the drain lands");
        while inbox.try_recv().is_ok() {}
        let filled = world.run_reset(Vid::new(VID)).expect("the line runs");
        assert_eq!(filled.hp, hit_max, "the hit points fill");
        assert_eq!(filled.sp, spell_max, "the spell points fill");
        let line: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        let hit = u8::try_from(point::POINT_HP).expect("a point byte");
        let spell = u8::try_from(point::POINT_SP).expect("a point byte");
        assert_eq!(
            change_kinds(&line),
            vec![hit, spell],
            "hit first, then spell"
        );
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        assert_eq!(points.hp(), hit_max);
        assert_eq!(points.sp(), spell_max);
        assert!(
            world.run_reset(Vid::new(0x1)).is_none(),
            "no character, no run"
        );
    }
}
