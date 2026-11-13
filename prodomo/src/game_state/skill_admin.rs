//! The skill-admin lines: `/setskill`, `/setskillother`, `/setskillpoint`,
//! `/setjob`, `/advance_skill_select` and `/do_click_special_inv`
//! (`ACMD(do_setskill)`, `do_setskillother`, `do_set_skill_point`,
//! `do_set_skill_group`, `G/cmd_gm.cpp:2528-2572`, `:2574-2608`, `:2610-2622`,
//! `:2624-2637`, and `do_advance_skill_select`, `do_click_special_inv`,
//! `G/cmd_general.cpp:3982-4002`, `:4006-4009`).
//!
//! `setskill` (`GM_LOW_WIZARD`) answers `Syntax: setskill <name> <lev>` when a
//! word is missing or the level's first byte is no digit; else the word is a
//! vnum when its first byte is a digit, else a name through
//! `CSkillManager::Get` (`G/skill.cpp:327-350`), and an unknown one answers
//! `No such a skill by that name`. Else `SetSkillLevel` lands, `ComputePoints`
//! runs, byte 76 goes out, and the caller hears nothing more. `setskillother`
//! (`GM_HIGH_WIZARD`) is the same with a target first: `Syntax: setskillother
//! <target> <skillname> <lev>`, then `There is no such character` for an
//! offline name (`FindPC`'s lowered lookup, `G/char_manager.cpp:244-260`), and
//! the target hears the compute and byte 76 while the caller hears nothing.
//!
//! `SetSkillLevel` (`G/char_skill.cpp:183-213`) refuses a vnum past 254, refuses
//! a nonzero level for a 221-243 vnum whose familyneck fails (`SkillCanUp`,
//! `:909-1145`, behind `__7AND8TH_SKILLS__`, which the owner's build defines:
//! every other member of the 221-228 or 236-243 family must stand at 0, and
//! 229-235 hit the `default` arm and refuse any nonzero level), and otherwise
//! plants `MIN(40, level)`, with the master ladder riding byte 76's encode.
//! `setskillpoint` (`GM_IMPLEMENTOR`) moves both skill-counter arrays to the
//! line's number, 0 when the line is empty, and sends the kind-28 change with
//! amount 0 (`SetRealPoint` + `SetPoint` + `PointChange(POINT_SKILL, 0)`).
//! `setjob` (`GM_IMPLEMENTOR`, the table's name for `do_set_skill_group`)
//! narrows the line's number to a byte, 0 when empty; a group past 2 or a
//! caller below level 5 skips the set and byte 112, and then `ClearSkill` and
//! `skill group to %d.` with the full number always run.
//! `advance_skill_select` (`GM_PLAYER`) answers the syntax on an empty line
//! and `YOU_CAN_NOT_DO_THAT` to a grouped caller or one below level 5; else
//! the same set and clear run with no confirmation line.
//! `do_click_special_inv` (`GM_PLAYER`) tells `ShowSpecialInventory`.
//!
//! The numbers are `strtoul`/`strtol` ones: the level wraps mod 256, the vnum
//! mod 2^32, and the group and the counter wrap mod 2^32 (the `(int)` cast of
//! the clamped `long`). What the step stores rides the answers: the vnum, the
//! landed level and the untouched next-read for the two set lines, the counter
//! for the point line, and the group with the clear's zeroed skills and the
//! counter for the job line.

use common::point_slot as point;
use common::vid::Vid;

use super::pc_skill::{
    SKILL_HELP_FIRST, SKILL_HELP_LAST, SKILL_MAX_NUM, SKILL_NEW_FIRST, SKILL_NEW_LAST,
};
use super::GameState;
use crate::chat_line::chat_packet;
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_inventory::HEADER_GC_CHANGE_SKILL_GROUP;
use protocol::gc_small::GcHeaderAndByte;

/// `SkillList` (`G/char_skill.cpp:4108-4114`): the six group skills per job
/// and group, which `CanUseSkill` reads (`:4139-4147`).
const SKILL_LIST: [[u8; 6]; 8] = [
    [1, 2, 3, 4, 5, 6],
    [16, 17, 18, 19, 20, 21],
    [31, 32, 33, 34, 35, 36],
    [46, 47, 48, 49, 50, 51],
    [61, 62, 63, 64, 65, 66],
    [76, 77, 78, 79, 80, 81],
    [91, 92, 93, 94, 95, 96],
    [106, 107, 108, 109, 110, 111],
];

/// The skills `CanUseSkill` always grants (`G/char_skill.cpp:4173-4180`):
/// leadership, combo, mining, the three tongues, polymorph, horse, horse
/// summon, and the nine guild skills.
const ALWAYS_USABLE: [u8; 18] = [
    121, 122, 124, 126, 127, 128, 129, 130, 131, 151, 152, 153, 154, 155, 156, 157, 158, 159,
];

/// The horse skills the `do_all_skill_master` else-switch grants
/// (`G/cmd_gm.cpp:4745-4753`): wild attack, charge, escape, and the ranged
/// wild attack. The switch (not `CanUseSkill`'s riding arm) is what grants them.
const HORSE_USABLE: [u8; 4] = [137, 138, 139, 140];

/// `SKILL_MAX_LEVEL` (`C/length.h:75`): the level the master line plants.
const SKILL_MAX_LEVEL: u8 = 40;

/// What `/setskill` landed, for the store's `store_skill`: the vnum, the
/// landed level, and the next-read the set left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetSkillLanded {
    /// The skill vnum the line named.
    pub vnum: u8,
    /// The level the set planted (`MIN(40, line)`).
    pub level: u8,
    /// The slot's next-read, which `SetSkillLevel` never touches.
    pub next_read: u32,
}

/// What `/setskillother` landed: the target's shown name with the set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetSkillOtherLanded {
    /// The target's name as the world shows it, for the store's ID lookup.
    pub target: String,
    /// The set itself.
    pub set: SetSkillLanded,
}

/// What `/setjob` landed: the group for the held row, the clear's zeroed
/// skills for the store's deletes, and the skill counter for the held row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetJobLanded {
    /// The group byte the line narrowed to, whether the set ran or not.
    pub group: u8,
    /// Whether the set ran: the group is at most 2 and the caller is past
    /// level 4.
    pub set: bool,
    /// The skill vnums the clear zeroed that held a level.
    pub cleared_skills: Vec<u8>,
    /// The skill counter after the clear.
    pub skill: i32,
}

/// `(unsigned char) strtoul(text, NULL, 10)`: the leading digits mod 256. The
/// callers gate on a leading digit, so at least one digit always runs.
pub(super) fn str_to_byte(text: &[u8]) -> u8 {
    let mut value: u64 = 0;
    for byte in text {
        let Some(digit) = byte.checked_sub(b'0').filter(|digit| *digit < 10) else {
            break;
        };
        value = value.saturating_mul(10).saturating_add(u64::from(digit));
    }
    value.to_le_bytes()[0]
}

/// `(DWORD) strtoul(text, NULL, 10)`: the leading digits mod 2^32. The caller
/// gates on a leading digit, so at least one digit always runs.
pub(super) fn str_to_dword(text: &[u8]) -> u32 {
    let mut value: u64 = 0;
    for byte in text {
        let Some(digit) = byte.checked_sub(b'0').filter(|digit| *digit < 10) else {
            break;
        };
        value = value.saturating_mul(10).saturating_add(u64::from(digit));
    }
    u32::try_from(value & 0xffff_ffff).expect("a masked word fits")
}

/// `(int) strtol(text, NULL, 10)`: the leading digits as a signed 64-bit
/// number, clamped, then the low 32 bits, as the `(int)` cast of the clamped
/// `long` reads them.
pub(super) fn str_to_wrapping_int(text: &[u8]) -> i32 {
    let mut bytes = text.iter().peekable();
    while bytes.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
        bytes.next();
    }
    let mut negative = false;
    match bytes.peek() {
        Some(b'+') => {
            bytes.next();
        }
        Some(b'-') => {
            negative = true;
            bytes.next();
        }
        _ => {}
    }
    let mut value: i64 = 0;
    for byte in bytes {
        let Some(digit) = byte.checked_sub(b'0').filter(|digit| *digit < 10) else {
            break;
        };
        value = value.saturating_mul(10).saturating_add(i64::from(digit));
    }
    let value = if negative { -value } else { value };
    i32::from_le_bytes(value.to_le_bytes()[..4].try_into().expect("four bytes fit"))
}

impl GameState {
    /// `ACMD(do_setskill)`: the caller's skill becomes the line's level.
    /// Answers the landed set for the store, or `None` for a line that runs
    /// nothing. `SetSkillLevel`'s 7/8 gate refuses silently: the caller heard
    /// nothing either way, and legacy sends no compute then.
    pub(super) fn run_setskill(&mut self, vid: Vid, rest: &[u8]) -> Option<SetSkillLanded> {
        let mover = self.own_mover(vid)?;
        let (first, back) = crate::command::one_argument(rest);
        let (second, _) = crate::command::one_argument(back);
        if first.is_empty() || second.is_empty() || !second[0].is_ascii_digit() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Syntax: setskill <name> <lev>",
                &[],
            );
            return None;
        }
        let Some(vnum) = self.skill_vnum(&first) else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"No such a skill by that name",
                &[],
            );
            return None;
        };
        let level = str_to_byte(&second);
        self.set_skill_level(vid, vnum, level)
    }

    /// `ACMD(do_setskillother)`: the named character's skill becomes the
    /// line's level. Answers the target's shown name with the set for the
    /// store, or `None` for a line that runs nothing.
    pub(super) fn run_setskillother(
        &mut self,
        vid: Vid,
        rest: &[u8],
    ) -> Option<SetSkillOtherLanded> {
        let mover = self.own_mover(vid)?;
        let (first, back) = crate::command::one_argument(rest);
        let (second, back) = crate::command::one_argument(back);
        let (third, _) = crate::command::one_argument(back);
        if first.is_empty() || second.is_empty() || third.is_empty() || !third[0].is_ascii_digit() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Syntax: setskillother <target> <skillname> <lev>",
                &[],
            );
            return None;
        }
        let target = std::str::from_utf8(&first)
            .ok()
            .and_then(|name| self.whisper_target(name));
        let Some(target) = target else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"There is no such character",
                &[],
            );
            return None;
        };
        let Some(vnum) = self.skill_vnum(&second) else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"No such a skill by that name",
                &[],
            );
            return None;
        };
        let level = str_to_byte(&third);
        let set = self.set_skill_level(target, vnum, level)?;
        let shown = self
            .characters
            .find_by_vid(target)
            .map_or_else(|_| String::new(), |found| found.name().to_string());
        Some(SetSkillOtherLanded { target: shown, set })
    }

    /// `ACMD(do_set_skill_point)`: both skill-counter arrays move to the
    /// line's number, 0 when empty, and the kind-28 change with amount 0 goes
    /// out. Answers the counter for the held row.
    pub(super) fn run_setskillpoint(&mut self, vid: Vid, rest: &[u8]) -> Option<i32> {
        let (word, _) = crate::command::one_argument(rest);
        let value = if word.is_empty() {
            0
        } else {
            str_to_wrapping_int(&word)
        };
        let points = self.pc_points_mut(vid.raw())?;
        points.set_both_points(point::POINT_SKILL, value);
        let changed = points
            .point_change(point::POINT_SKILL, 0, true, false)
            .unwrap_or_default();
        self.send_point_records(vid, &changed);
        Some(value)
    }

    /// `ACMD(do_set_skill_group)`: the group byte sets when it is at most 2
    /// and the caller stands past level 4 (then byte 112 goes out), and then
    /// `ClearSkill` and the `skill group to %d.` line with the full number
    /// always run. Answers the group, whether the set ran, the clear's zeroed
    /// skills, and the counter.
    pub(super) fn run_setjob(&mut self, vid: Vid, rest: &[u8]) -> Option<SetJobLanded> {
        let mover = self.own_mover(vid)?;
        let (word, _) = crate::command::one_argument(rest);
        let value = if word.is_empty() {
            0
        } else {
            str_to_wrapping_int(&word)
        };
        let line = format!("skill group to {value}.");
        Some(self.set_group_and_clear(vid, mover, value, Some(line.as_bytes())))
    }
    /// `ACMD(do_advance_skill_select)`: an empty line answers the syntax, a
    /// caller already grouped or below level 5 hears
    /// `YOU_CAN_NOT_DO_THAT`, else the set and the clear run exactly as
    /// `/setjob` runs them, minus the confirmation line. Answers the same
    /// landed row `/setjob` answers, or `None` for a line that runs nothing.
    pub(super) fn run_advance_skill(&mut self, vid: Vid, rest: &[u8]) -> Option<SetJobLanded> {
        let mover = self.own_mover(vid)?;
        let (word, _) = crate::command::one_argument(rest);
        if word.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"@@(cmd_general)tradus:Wrong Syntax",
                &[],
            );
            return None;
        }
        let grouped = self
            .bodies
            .get(&vid)
            .is_some_and(|body| body.card.skill_group != 0);
        let level = self.level_of(vid)?;
        if grouped || level < 5 {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"@@(cmd_general)tradus:YOU_CAN_NOT_DO_THAT",
                &[],
            );
            return None;
        }
        let value = str_to_wrapping_int(&word);
        Some(self.set_group_and_clear(vid, mover, value, None))
    }

    /// `CHARACTER::AdvanceSkillSelect` (`G/char.cpp:11328-11359`,
    /// `__ENABLE_ADVANCE_SKILL_SELECT__`): a grouped caller or one below level 5
    /// runs nothing; else the `AdvanceSkillSelectWindow <JOB>` command line goes
    /// out, with the job word of the caller's job (an unknown job logs nothing
    /// and the line carries the empty word, as legacy's `sys_err` + empty word).
    pub(super) fn advance_skill_select(&mut self, vid: Vid) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let Some(word) = self.advance_skill_word(vid) else {
            return;
        };
        let mut format = b"AdvanceSkillSelectWindow ".to_vec();
        format.extend_from_slice(word);
        self.tell(
            vid,
            mover,
            protocol::gc_chat::CHAT_TYPE_COMMAND,
            &format,
            &[],
        );
    }

    /// The job word `AdvanceSkillSelect` offers with, or `None` for a grouped caller
    /// or one below level 5. The burst's frame and the level-up hook share it.
    fn advance_skill_word(&mut self, vid: Vid) -> Option<&'static [u8]> {
        let (grouped, job) = self.bodies.get(&vid).map_or((true, 0), |body| {
            (body.card.skill_group != 0, body.card.job)
        });
        if grouped {
            return None;
        }
        let level = self.level_of(vid).unwrap_or(0);
        if level < 5 {
            return None;
        }
        Some(match job {
            0 => b"JOB_WARRIOR",
            1 => b"JOB_ASSASSIN",
            2 => b"JOB_SURA",
            3 => b"JOB_SHAMAN",
            _ => b"",
        })
    }

    /// The `AdvanceSkillSelectWindow <JOB>` command frame, or `None` for a grouped
    /// caller or one below level 5. The login burst carries it.
    pub(super) fn advance_skill_window(&mut self, vid: Vid) -> Option<Vec<u8>> {
        let word = self.advance_skill_word(vid)?;
        let mover = self.own_mover(vid)?;
        let mut format = b"AdvanceSkillSelectWindow ".to_vec();
        format.extend_from_slice(word);
        Some(chat_packet(
            mover.recipient(&self.locale),
            protocol::gc_chat::CHAT_TYPE_COMMAND,
            &format,
            &[],
        ))
    }

    /// `ACMD(do_click_special_inv)`: the `ShowSpecialInventory` command line.
    pub(super) fn run_click_special_inv(&self, vid: Vid) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        self.tell(
            vid,
            mover,
            protocol::gc_chat::CHAT_TYPE_COMMAND,
            b"ShowSpecialInventory",
            &[],
        );
    }

    /// `SetSkillGroup` + `ClearSkill`: the group byte sets when it is at most
    /// 2 and the caller stands past level 4 (then byte 112 goes out), and
    /// then the clear runs. `confirm` is the line `/setjob` tells with the
    /// full number; the skill-select tells none. Answers the group, whether
    /// the set ran, the clear's zeroed skills, and the counter.
    fn set_group_and_clear(
        &mut self,
        vid: Vid,
        mover: crate::item_move::Mover,
        value: i32,
        confirm: Option<&[u8]>,
    ) -> SetJobLanded {
        let group = value.to_le_bytes()[0];
        let level = self.level_of(vid).unwrap_or(0);
        let set = group <= 2 && level >= 5;
        if set {
            if let Some(body) = self.bodies.get_mut(&vid) {
                body.card.skill_group = group;
            }
            let mut out = Vec::new();
            GcHeaderAndByte::new(HEADER_GC_CHANGE_SKILL_GROUP.value(), group).encode_into(&mut out);
            let _sent = self.write_to_client(vid, out);
        }
        let cleared_skills = self.clear_skills(vid, level);
        let skill = self
            .pc_points_mut(vid.raw())
            .map_or(0, |points| points.get_point(point::POINT_SKILL));
        if let Some(line) = confirm {
            self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
        }
        SetJobLanded {
            group,
            set,
            cleared_skills,
            skill,
        }
    }

    /// `CSkillManager::Get`: the vnum when the word starts with a digit
    /// (`strtoul` mod 2^32), else the name's lowest vnum. A vnum no row
    /// carries, or past 254, is no skill, as the NULL proto is.
    fn skill_vnum(&self, word: &[u8]) -> Option<u8> {
        if word.first().is_some_and(u8::is_ascii_digit) {
            let vnum = str_to_dword(word);
            if vnum >= SKILL_MAX_NUM || !self.skill_names.contains(vnum) {
                return None;
            }
            return u8::try_from(vnum).ok();
        }
        let vnum = self.skill_names.by_name(word)?;
        if vnum >= SKILL_MAX_NUM {
            return None;
        }
        u8::try_from(vnum).ok()
    }

    /// `SetSkillLevel` + `ComputePoints` + `SkillLevelPacket`: the level
    /// plants (past the 7/8 gate), then the compute and byte 76 go out.
    /// Answers the landed set for the store, or `None` when the gate refuses.
    fn set_skill_level(&mut self, vid: Vid, vnum: u8, level: u8) -> Option<SetSkillLanded> {
        if level != 0 && !self.skill_can_up(vid, u32::from(vnum)) {
            return None;
        }
        let landed = level.min(40);
        let state = self.pc_skills.entry(vid.raw()).or_default();
        state.levels[usize::from(vnum)] = landed;
        let next_read = state.next_read[usize::from(vnum)];
        let points = self.pc_points_mut(vid.raw())?;
        let computed = points.compute_points();
        self.send_point_records(vid, &computed);
        self.send_skill_levels(vid);
        Some(SetSkillLanded {
            vnum,
            level: landed,
            next_read,
        })
    }

    /// `SkillCanUp` (`G/char_skill.cpp:909-1145`): outside 221-243 anything
    /// climbs; inside, every other member of the 221-228 or 236-243 family
    /// must stand at 0, and 229-235 refuse (the `default` arm). The climber's
    /// own level is never read, so a re-set of the standing skill passes.
    pub(super) fn skill_can_up(&self, vid: Vid, vnum: u32) -> bool {
        let family = if (SKILL_NEW_FIRST..=SKILL_NEW_LAST).contains(&vnum) {
            SKILL_NEW_FIRST..=SKILL_NEW_LAST
        } else if (SKILL_HELP_FIRST..=SKILL_HELP_LAST).contains(&vnum) {
            SKILL_HELP_FIRST..=SKILL_HELP_LAST
        } else {
            return !(SKILL_NEW_FIRST..=SKILL_HELP_LAST).contains(&vnum);
        };
        family
            .filter(|sibling| *sibling != vnum)
            .all(|sibling| self.pc_skill_level(vid.raw(), sibling) == 0)
    }

    /// `ACMD(do_all_skill_master)` (`G/cmd_gm.cpp:4734-4758`): every usable
    /// skill becomes 40. Answers what the line planted for the store's
    /// `store_skill`: the vnum, 40, and the next-read the set left alone.
    ///
    /// `SetHorseLevel(40)` runs first: the horse clamps to 30 and refills, and
    /// horse skill 130 is planted at 30, which the loop then plants at 40. The
    /// loop is
    /// `i = 0..255`: usable (`CanUseSkill`, `G/char_skill.cpp:4135-4181` —
    /// the caller's six group skills past group 0, the eighteen always
    /// usable, and the four horse skills through the else-switch) plants
    /// `MIN(40, 40)` past the 7/8 gate; then one compute and one byte 76.
    pub(super) fn run_all_skill_master(&mut self, vid: Vid) -> Vec<SetSkillLanded> {
        let Some(body) = self.bodies.get(&vid) else {
            return Vec::new();
        };
        let (job, group) = (body.card.job, body.card.skill_group);
        let _horse = self.plant_horse_level(vid, i32::from(SKILL_MAX_LEVEL));
        let mut usable = Vec::new();
        if group >= 1 {
            if let Some(row) = usize::from(job)
                .checked_mul(2)
                .and_then(|base| base.checked_add(usize::from(group) - 1))
                .and_then(|row| SKILL_LIST.get(row))
            {
                usable.extend_from_slice(row);
            }
        }
        usable.extend_from_slice(&ALWAYS_USABLE);
        usable.extend_from_slice(&HORSE_USABLE);
        usable.sort_unstable();
        let mut planted = Vec::new();
        for vnum in usable {
            if u32::from(vnum) >= SKILL_MAX_NUM || !self.skill_can_up(vid, u32::from(vnum)) {
                continue;
            }
            let state = self.pc_skills.entry(vid.raw()).or_default();
            state.levels[usize::from(vnum)] = SKILL_MAX_LEVEL;
            planted.push(SetSkillLanded {
                vnum,
                level: SKILL_MAX_LEVEL,
                next_read: state.next_read[usize::from(vnum)],
            });
        }
        if planted.is_empty() {
            return planted;
        }
        if let Some(points) = self.pc_points_mut(vid.raw()) {
            let computed = points.compute_points();
            self.send_point_records(vid, &computed);
        }
        self.send_skill_levels(vid);
        planted
    }
}
#[cfg(test)]
mod tests {
    use super::{str_to_byte, str_to_dword, str_to_wrapping_int, SetSkillLanded};
    use crate::game_state::fixtures::{a_world, enter, TestClock};
    use common::point_slot as point;
    use common::vid::Vid;
    use std::path::Path;
    use std::sync::Arc;

    const VID: u32 = 0x6001;
    const OTHER: u32 = 0x6002;

    fn names() -> Arc<gamedata::pc_skill_data::SkillNames> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/sql/gamedata");
        let player = std::fs::read(root.join("player.sql")).unwrap();
        Arc::new(gamedata::pc_skill_data::SkillNames::from_dump(&player).unwrap())
    }

    fn entered(
        world: &mut crate::game_state::GameState,
    ) -> tokio::sync::mpsc::UnboundedReceiver<Vec<u8>> {
        let mut inbox = enter(world, VID, (3200, 3200), 0);
        while inbox.try_recv().is_ok() {}
        inbox
    }

    fn says(line: &[u8], text: &[u8]) -> bool {
        line.windows(text.len()).any(|window| window == text)
    }

    /// A missing word or a level that starts with no digit hears the syntax
    /// and sets nothing.
    #[test]
    fn a_setskill_answers_its_syntax_and_changes_nothing() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_skill_names(names());
        let mut inbox = entered(&mut world);
        for line in [b"".as_slice(), b"1", b"1 ", b"1 x", b"1 +2", b"Ambuscada"] {
            assert!(
                world.run_setskill(Vid::new(VID), line).is_none(),
                "no set for {line:?}"
            );
            let heard = inbox.try_recv().expect("the syntax");
            assert!(
                says(&heard, b"Syntax: setskill <name> <lev>"),
                "the syntax line"
            );
            assert!(inbox.try_recv().is_err(), "nothing else");
        }
        assert_eq!(world.pc_skill_level(VID, 1), 0, "nothing set");
    }

    /// An unknown vnum or name hears the miss and sets nothing.
    #[test]
    fn a_setskill_misses_an_unknown_skill() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_skill_names(names());
        let mut inbox = entered(&mut world);
        for line in [b"7 20".as_slice(), b"999 20", b"NoSuchSkill 20"] {
            assert!(
                world.run_setskill(Vid::new(VID), line).is_none(),
                "no set for {line:?}"
            );
            let heard = inbox.try_recv().expect("the miss");
            assert!(
                says(&heard, b"No such a skill by that name"),
                "the miss line"
            );
            assert!(inbox.try_recv().is_err(), "nothing else");
        }
    }

    /// A set by vnum or by name plants the level, computes, sends byte 76,
    /// and answers the landed set with the untouched next-read.
    #[test]
    fn a_setskill_sets_by_vnum_and_by_name() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_skill_names(names());
        let mut inbox = entered(&mut world);
        world.pc_skills.entry(VID).or_default().next_read[31] = 1234;
        let set = world
            .run_setskill(Vid::new(VID), b"31 20")
            .expect("the set lands");
        assert_eq!(
            set,
            SetSkillLanded {
                vnum: 31,
                level: 20,
                next_read: 1234,
            }
        );
        assert_eq!(world.pc_skill_level(VID, 31), 20);
        let mut saw_compute = false;
        let mut saw_76 = false;
        while let Ok(frame) = inbox.try_recv() {
            assert!(!says(&frame, b"Syntax"), "no syntax: {frame:?}");
            if frame.len() == 25 && frame[0..4] == 17i32.to_le_bytes() {
                saw_compute = true;
            }
            if frame.first() == Some(&76) {
                saw_76 = true;
                assert_eq!(frame[1 + 31 * 6 + 1], 20, "slot 31 stands at 20");
            }
        }
        assert!(saw_compute, "the compute rides");
        assert!(saw_76, "byte 76 rides");
        let set = world
            .run_setskill(Vid::new(VID), b"aMbUsCaDa 5")
            .expect("the name lands");
        assert_eq!(set.vnum, 31);
        assert_eq!(set.level, 5);
        assert_eq!(world.pc_skill_level(VID, 31), 5);
    }

    /// The level wraps mod 256 (`strtoul` into a byte) and then clamps to 40;
    /// the vnum wraps mod 2^32 and misses when no row carries it.
    #[test]
    fn a_setskill_wraps_and_clamps() {
        assert_eq!(str_to_byte(b"300"), 44);
        assert_eq!(str_to_byte(b"20x"), 20);
        assert_eq!(str_to_dword(b"4294967296"), 0);
        assert_eq!(str_to_wrapping_int(b"2147483648"), i32::MIN);
        assert_eq!(str_to_wrapping_int(b"-1"), -1);
        assert_eq!(str_to_wrapping_int(b""), 0);
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_skill_names(names());
        let mut inbox = entered(&mut world);
        let set = world
            .run_setskill(Vid::new(VID), b"1 300")
            .expect("the set lands");
        assert_eq!(set.level, 40, "44 clamps to 40");
        assert_eq!(world.pc_skill_level(VID, 1), 40);
        while inbox.try_recv().is_ok() {}
        assert!(
            world
                .run_setskill(Vid::new(VID), b"4294967296 20")
                .is_none(),
            "vnum 0 has no row"
        );
        let heard = inbox.try_recv().expect("the miss");
        assert!(says(&heard, b"No such a skill by that name"));
    }

    /// The 7/8 gate: 221 climbs while its family stands at 0, 222 refuses
    /// beside it, the re-set of 221 passes, 0 always lands, and 229 refuses
    /// any nonzero level through the `default` arm.
    #[test]
    fn a_setskill_holds_the_new_family_line() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_skill_names(names());
        let mut inbox = entered(&mut world);
        let set = world
            .run_setskill(Vid::new(VID), b"221 20")
            .expect("221 climbs alone");
        assert_eq!((set.vnum, set.level), (221, 20));
        while inbox.try_recv().is_ok() {}
        assert!(
            world.run_setskill(Vid::new(VID), b"222 20").is_none(),
            "222 refuses beside 221"
        );
        assert!(inbox.try_recv().is_err(), "the refusal is silent");
        let set = world
            .run_setskill(Vid::new(VID), b"221 30")
            .expect("221 re-sets");
        assert_eq!(set.level, 30);
        let set = world
            .run_setskill(Vid::new(VID), b"222 0")
            .expect("0 always lands");
        assert_eq!(set.level, 0);
        while inbox.try_recv().is_ok() {}
        assert!(
            world.run_setskill(Vid::new(VID), b"229 20").is_none(),
            "229 refuses through default"
        );
        assert!(inbox.try_recv().is_err(), "the refusal is silent");
        assert_eq!(world.pc_skill_level(VID, 222), 0);
        assert_eq!(world.pc_skill_level(VID, 229), 0);
    }

    /// The other line: the syntax, the offline miss, the unknown skill, and
    /// the target's set with its shown name answered.
    #[test]
    fn a_setskillother_sets_the_target() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_skill_names(names());
        let mut inbox = entered(&mut world);
        let mut other = enter(&mut world, OTHER, (3200, 3200), 0);
        while other.try_recv().is_ok() {}
        while inbox.try_recv().is_ok() {}
        assert!(
            world.run_setskillother(Vid::new(VID), b"P1 1").is_none(),
            "a short line runs nothing"
        );
        let heard = inbox.try_recv().expect("the syntax");
        assert!(says(
            &heard,
            b"Syntax: setskillother <target> <skillname> <lev>"
        ));
        assert!(
            world
                .run_setskillother(Vid::new(VID), b"Nobody 1 20")
                .is_none(),
            "an offline name runs nothing"
        );
        let heard = inbox.try_recv().expect("the miss");
        assert!(says(&heard, b"There is no such character"));
        let shown = world
            .characters
            .find_by_vid(Vid::new(OTHER))
            .map(|found| found.name().to_string())
            .expect("the other stands");
        let line = format!("{shown} 1 20");
        let set = world
            .run_setskillother(Vid::new(VID), line.as_bytes())
            .expect("the set lands");
        assert_eq!(set.target, shown);
        assert_eq!((set.set.vnum, set.set.level), (1, 20));
        assert_eq!(world.pc_skill_level(OTHER, 1), 20);
        assert_eq!(world.pc_skill_level(VID, 1), 0, "the caller keeps none");
        assert!(inbox.try_recv().is_err(), "the caller hears nothing");
        let mut saw_76 = false;
        while let Ok(frame) = other.try_recv() {
            if frame.first() == Some(&76) {
                saw_76 = true;
            }
        }
        assert!(saw_76, "the target hears byte 76");
    }

    /// The point line: an empty line zeroes the counter, a number sets it,
    /// and the kind-28 change with amount 0 goes out either way.
    #[test]
    fn a_setskillpoint_moves_the_counter() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_skill_names(names());
        let mut inbox = entered(&mut world);
        let counter = world
            .run_setskillpoint(Vid::new(VID), b"17")
            .expect("the set lands");
        assert_eq!(counter, 17);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        assert_eq!(points.get_point(point::POINT_SKILL), 17);
        let frame = inbox.try_recv().expect("the change");
        assert_eq!(frame.len(), 25);
        assert_eq!(&frame[0..4], &17i32.to_le_bytes());
        assert_eq!(frame[8], 28);
        assert_eq!(&frame[9..17], &0i64.to_le_bytes(), "the amount is 0");
        assert_eq!(&frame[17..25], &17i64.to_le_bytes(), "the value is 17");
        assert!(inbox.try_recv().is_err(), "nothing else");
        let counter = world
            .run_setskillpoint(Vid::new(VID), b"")
            .expect("the empty lands");
        assert_eq!(counter, 0);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        assert_eq!(points.get_point(point::POINT_SKILL), 0);
    }

    /// The job line: the group sets with byte 112, the clear zeroes the
    /// non-sub skills and moves the counter, and the line always prints the
    /// full number, even when the set skips.
    #[test]
    fn a_setjob_sets_clears_and_always_confirms() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_skill_names(names());
        let mut inbox = entered(&mut world);
        world.set_pc_skill_level(VID, 1, 20);
        world.set_pc_skill_level(VID, 121, 5);
        let landed = world
            .run_setjob(Vid::new(VID), b"2")
            .expect("the line lands");
        assert!(landed.set);
        assert_eq!(landed.group, 2);
        assert_eq!(landed.cleared_skills, vec![1]);
        assert_eq!(landed.skill, 4 + (10 - 5), "the clear's counter");
        assert_eq!(world.pc_skill_level(VID, 1), 0, "the skill zeroes");
        assert_eq!(world.pc_skill_level(VID, 121), 5, "the sub-skill stands");
        let mut saw_112 = false;
        let mut saw_line = false;
        let mut saw_76 = false;
        while let Ok(frame) = inbox.try_recv() {
            if frame == vec![112, 2] {
                saw_112 = true;
            }
            if says(&frame, b"skill group to 2.") {
                saw_line = true;
            }
            if frame.first() == Some(&76) {
                saw_76 = true;
            }
        }
        assert!(saw_112, "byte 112 rides first");
        assert!(saw_line, "the line confirms");
        assert!(saw_76, "the clear sends byte 76");
        let body = world.bodies.get(&Vid::new(VID)).expect("the card");
        assert_eq!(body.card.skill_group, 2);
        // A group past 2 skips the set but still clears and confirms.
        world.set_pc_skill_level(VID, 2, 7);
        while inbox.try_recv().is_ok() {}
        let landed = world
            .run_setjob(Vid::new(VID), b"5")
            .expect("the line lands");
        assert!(!landed.set);
        assert_eq!(landed.group, 5);
        assert_eq!(landed.cleared_skills, vec![2]);
        assert_eq!(world.pc_skill_level(VID, 2), 0);
        let mut saw_112 = false;
        let mut saw_line = false;
        while let Ok(frame) = inbox.try_recv() {
            if frame.first() == Some(&112) {
                saw_112 = true;
            }
            if says(&frame, b"skill group to 5.") {
                saw_line = true;
            }
        }
        assert!(!saw_112, "no set, no byte 112");
        assert!(saw_line, "the line still confirms");
        let body = world.bodies.get(&Vid::new(VID)).expect("the card");
        assert_eq!(body.card.skill_group, 2, "the card keeps 2");
    }

    /// The master line plants the caller's six group skills, the eighteen
    /// always usable, and the four horse skills at 40, with one compute and
    /// one byte 76; a groupless caller plants no group six.
    #[test]
    fn a_master_line_plants_every_usable_skill_at_once() {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_skill_names(names());
        let mut inbox = entered(&mut world);
        world
            .bodies
            .get_mut(&Vid::new(VID))
            .expect("the card")
            .card
            .skill_group = 1;
        world.pc_skills.entry(VID).or_default().next_read[3] = 777;
        let planted = world.run_all_skill_master(Vid::new(VID));
        assert_eq!(planted.len(), 28, "six plus eighteen plus four");
        assert!(
            planted.iter().all(|set| set.level == 40),
            "every plant stands at 40"
        );
        let third = planted.iter().find(|set| set.vnum == 3).expect("skill 3");
        assert_eq!(third.next_read, 777, "the next-read rides along");
        assert_eq!(world.pc_skill_level(VID, 1), 40);
        assert_eq!(world.pc_skill_level(VID, 121), 40);
        assert_eq!(world.pc_skill_level(VID, 130), 40, "horse 130 lands at 40");
        assert_eq!(world.pc_skill_level(VID, 137), 40, "the horse four land");
        assert_eq!(world.pc_skill_level(VID, 7), 0, "the other group keeps 0");
        assert_eq!(world.pc_skill_level(VID, 221), 0, "no 7/8 skill is usable");
        let mut seen_76 = 0;
        while let Ok(frame) = inbox.try_recv() {
            if frame.first() == Some(&76) {
                seen_76 += 1;
                assert_eq!(frame[1 + 6], 3, "slot 1 rides PERFECT");
                assert_eq!(frame[1 + 6 + 1], 40, "slot 1 rides 40");
                assert_eq!(frame[1 + 7 * 6 + 1], 0, "slot 7 rides 0");
            }
        }
        assert_eq!(seen_76, 1, "one byte 76");

        let mut other = enter(&mut world, OTHER, (3200, 3200), 0);
        while other.try_recv().is_ok() {}
        let planted = world.run_all_skill_master(Vid::new(OTHER));
        assert_eq!(planted.len(), 22, "no group, no group six");
        assert_eq!(world.pc_skill_level(OTHER, 1), 0);
        assert_eq!(world.pc_skill_level(OTHER, 151), 40, "the guild nine land");
    }
}
