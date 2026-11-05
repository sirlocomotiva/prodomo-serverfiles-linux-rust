//! `/set`: the GM's attribute line (`ACMD(do_set)`, `G/cmd_gm.cpp:1862-2074`).
//!
//! `set <name> <field> <value>` finds the online PC case-insensitively (`FindPC`) and
//! matches the field by prefix against the table in order — gold, race, sex, job, exp, the
//! maxima, skill, alignment, align, gaya — so `g` is gold and `align` meets
//! alignment first. A missing word tells the usage with the numbered field list (and the
//! race/sex/job help lines, under `ENABLE_NEWSTUFF`); an offline name tells
//! `<name> not exist`; an unknown field is silent (the `\n` row is `MISC`, so no
//! confirmation rides). A set field answers `<name>'s <field> set to [<value>]` with the
//! canonical field and the `long long` value — even when the change refused (the gold
//! cap) or is a gap (conqueror experience).
//!
//! This slice ports the gold, exp, maxima and skill arms. Gold is `ChangeGold`
//! (`G/char.cpp:3823-3850`): the `i128` total refuses below 0 and at `GOLD_MAX_MAX`
//! (the unsigned comparison refuses negatives), else `set_gold` with the
//! `GC_CHARACTER_GOLD_CHANGE` whose amount is 0 for a non-positive delta. The store
//! writes the delta as a Transfer, so the answer carries it. Exp is conqueror-aware
//! (`__CONQUEROR_LEVEL__`): a target past conqueror 0 takes `POINT_CONQUEROR_EXP` with
//! its chat line and steps (`game_state::conqueror`); a 0 takes
//! `gain_pc_exp` up and `lose_exp` down. The maxima and the skill point are `PointChange`
//! with the amount shown, sent to the target. Race, sex, job, alignment and gaya wait
//! for ledger 352 with their systems (polymorph, the fallthrough ruling, the gaya
//! column); their words resolve but change nothing yet.

use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_inventory::HEADER_GC_CHANGE_SKILL_GROUP;
use protocol::gc_small::GcHeaderAndByte;

use super::view::EntityKey;
use super::GameState;
use crate::chat_line::Arg;
use crate::command::one_argument;
use crate::game_state::whisper::str_to_int;

/// One `/set` field, in `set_fields` order (`G/cmd_gm.cpp:1843-1860`): the canonical
/// name with its help line, if it has one.
const FIELDS: [(&str, Option<&str>); 11] = [
    ("gold", None),
    ("race", Some("0. Warrior, 1. Ninja, 2. Sura, 3. Shaman")),
    ("sex", Some("0. Male, 1. Female")),
    ("job", Some("0. None, 1. First, 2. Second")),
    ("exp", None),
    ("max_hp", None),
    ("max_sp", None),
    ("skill", None),
    ("alignment", None),
    ("align", None),
    ("gaya", None),
];

/// The race the race arm picks for a job and a sex, male then female
/// (`MAIN_RACE_*`, `G/char.h:63-72`): warrior 0/4, assassin 5/1, sura 2/6, shaman 7/3.
const RACE_OF_JOB: [[u8; 2]; 4] = [[0, 4], [5, 1], [2, 6], [7, 3]];

/// What `/set` stored on the target's row, which the save would not reach: the gold
/// arm's delta (a Transfer) and the race and job arms' identity. Exp, the points, the
/// alignment and the gaya ride the target's own save through its held copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetAttrStored {
    /// The target's row id.
    pub pid: u32,
    /// The gold delta the arm applied, for the Transfer the step writes.
    pub gold: Option<i64>,
    /// The race and group the race or job arm left, for the identity the step writes.
    pub identity: Option<(u8, u8)>,
    /// The skill vnums the race or job arm's clear zeroed that held a level.
    pub cleared_skills: Vec<u8>,
}

impl GameState {
    /// Run `/set` for the caller online under `vid`: the usage, the miss, the arm, and
    /// the confirmation. Answers the gold Transfer for the store, or `None` when no arm
    /// stored one.
    pub(super) fn run_set_attr(&mut self, vid: Vid, rest: &[u8]) -> Option<SetAttrStored> {
        let mover = self.own_mover(vid)?;
        let (first, back) = one_argument(rest);
        let (second, back) = one_argument(back);
        let (third, _) = one_argument(back);
        if first.is_empty() || second.is_empty() || third.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Usage: set <name> <field> <value>",
                &[],
            );
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"List of the fields available:",
                &[],
            );
            for (index, (field, help)) in FIELDS.iter().enumerate() {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b" %d. %s",
                    &[
                        Arg::Int(i64::try_from(index).unwrap_or(i64::MAX) + 1),
                        Arg::Text(field.as_bytes()),
                    ],
                );
                if let Some(help) = help {
                    self.tell(
                        vid,
                        mover,
                        CHAT_TYPE_INFO,
                        b"  Help: %s",
                        &[Arg::Text(help.as_bytes())],
                    );
                }
            }
            return None;
        }
        let field = FIELDS
            .iter()
            .position(|(name, _)| name.as_bytes().starts_with(&second));
        let target = self.find_pc_vid(&first);
        let (Some(index), Some(target)) = (field, target) else {
            if target.is_none() {
                let mut line = first.clone();
                line.extend_from_slice(b" not exist");
                self.tell(vid, mover, CHAT_TYPE_INFO, &line, &[]);
            }
            return None;
        };
        let stored = match index {
            0 => self.set_gold(target, &third),
            1 => self.set_race(target, &third),
            2 => self.set_sex(target, &third),
            3 => self.set_job_field(target, &third),
            4 => {
                self.set_exp(target, &third);
                None
            }
            5 => {
                self.set_point(target, common::point_slot::POINT_MAX_HP, &third);
                None
            }
            6 => {
                self.set_point(target, common::point_slot::POINT_MAX_SP, &third);
                None
            }
            7 => {
                self.set_point(target, common::point_slot::POINT_SKILL, &third);
                None
            }
            // `alignment` and `align` both run the same arm; the gaya that legacy's
            // missing `break` also changes is a Defect and stays unchanged.
            8 | 9 => {
                self.set_alignment(target, &third);
                None
            }
            _ => {
                self.set_gaya_field(target, &third);
                None
            }
        };
        let amount = str_to_long_long(&third);
        if let Ok(shown) = self.characters.find_by_vid(target) {
            let name = shown.name().to_owned();
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"%s's %s set to [%lld]",
                &[
                    Arg::Text(name.as_bytes()),
                    Arg::Text(FIELDS[index].0.as_bytes()),
                    Arg::Int(amount),
                ],
            );
        }
        stored
    }

    /// The gold arm: `ChangeGold` on the target, answered for the store's Transfer.
    fn set_gold(&mut self, target: Vid, word: &[u8]) -> Option<SetAttrStored> {
        let amount = str_to_long_long(word);
        let Ok(character) = self.characters.find_by_vid(target) else {
            return None;
        };
        let total = i128::from(character.gold()) + i128::from(amount);
        if total < 0 || total >= i128::from(world::character::GOLD_MAX_MAX) {
            return None;
        }
        let Ok(gold) = u64::try_from(total) else {
            return None;
        };
        let pid = character.player_id();
        if let Ok(character) = self.characters.find_by_vid_mut(target) {
            character.set_gold(gold);
        } else {
            return None;
        }
        let mut record = Vec::new();
        protocol::gc_actors::GcCharacterGoldChange::new(
            target.raw(),
            if amount <= 0 { 0 } else { amount },
            gold,
        )
        .encode_into(&mut record);
        let _sent = self.write_to_client(target, record);
        Some(SetAttrStored {
            pid,
            gold: Some(amount),
            identity: None,
            cleared_skills: Vec::new(),
        })
    }

    /// The exp arm: `gain_pc_exp` up and `lose_exp` down, or the conqueror experience arm
    /// past conqueror 0, which takes either sign.
    fn set_exp(&mut self, target: Vid, word: &[u8]) {
        let amount = str_to_int(word);
        let conqueror = self
            .characters
            .find_by_vid(target)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, world::character::Points::conqueror_level);
        if conqueror > 0 {
            self.grant_conqueror_exp(target, i64::from(amount));
            return;
        }
        if amount >= 0 {
            self.gain_pc_exp(target, i64::from(amount));
        } else {
            let max = self.exp_limits.max_level;
            let loss = -i64::from(amount);
            let record = self
                .pc_points_mut(target.raw())
                .and_then(|points| points.lose_exp(loss, max));
            if let Some(record) = record {
                self.send_point_records(target, &[record]);
            }
        }
    }

    /// A `PointChange(kind, amount, true)` arm: the change shows its amount on the
    /// target's client.
    fn set_point(&mut self, target: Vid, kind: usize, word: &[u8]) {
        let amount = str_to_int(word);
        let records = self
            .pc_points_mut(target.raw())
            .and_then(|points| points.point_change(kind, amount, true, false).ok());
        if let Some(records) = records {
            self.send_point_records(target, &records);
        }
    }

    /// The race arm: the job byte picks the target's job's race in the sex it holds,
    /// clamped to the four jobs; a changed race sets, clears the skills, drops the
    /// group, and reencodes twice through the polymorph workaround.
    fn set_race(&mut self, target: Vid, word: &[u8]) -> Option<SetAttrStored> {
        let job = str_to_int(word).clamp(0, 3);
        let female = self
            .characters
            .find_by_vid(target)
            .ok()
            .and_then(|character| character.points())
            .is_some_and(|points| world::character::is_female(points.race()));
        let race = RACE_OF_JOB[usize::try_from(job).unwrap_or(0)][usize::from(female)];
        let changed = self
            .characters
            .find_by_vid(target)
            .ok()
            .and_then(|character| character.points())
            .is_some_and(|points| points.race() != race);
        if !changed {
            return None;
        }
        if let Some(points) = self.pc_points_mut(target.raw()) {
            points.set_race(race);
        }
        if let Some(body) = self.bodies.get_mut(&target) {
            body.card.job = race;
            body.card.skill_group = 0;
        }
        let level = self.level_of(target).unwrap_or(1);
        let cleared_skills = self.clear_skills(target, level);
        self.set_polymorph_mesh(target);
        let pid = self.characters.find_by_vid(target).ok()?.player_id();
        Some(SetAttrStored {
            pid,
            gold: None,
            identity: Some((race, 0)),
            cleared_skills,
        })
    }

    /// The sex arm: the clamped sex flips the target's race when it differs, then
    /// reencodes twice through the polymorph workaround.
    fn set_sex(&mut self, target: Vid, word: &[u8]) -> Option<SetAttrStored> {
        let female = str_to_int(word).clamp(0, 1) == 1;
        let race = self
            .characters
            .find_by_vid(target)
            .ok()
            .and_then(|character| character.points())
            .map(world::character::Points::race);
        let changed =
            race.is_some_and(|race| world::character::is_female(race) != female && race < 8);
        if !changed {
            return None;
        }
        let flipped = race.map_or(0, |race| race ^ 4);
        if let Some(points) = self.pc_points_mut(target.raw()) {
            points.set_race(flipped);
        }
        let group = if let Some(body) = self.bodies.get_mut(&target) {
            body.card.job = flipped;
            body.card.skill_group
        } else {
            0
        };
        self.set_polymorph_mesh(target);
        let pid = self.characters.find_by_vid(target).ok()?.player_id();
        Some(SetAttrStored {
            pid,
            gold: None,
            identity: Some((flipped, group)),
            cleared_skills: Vec::new(),
        })
    }

    /// The job arm (`SetSkillGroup`): the clamped group byte sets past level 4 with
    /// byte 112, and the skills always clear. Unlike `/setjob`, no line rides.
    fn set_job_field(&mut self, target: Vid, word: &[u8]) -> Option<SetAttrStored> {
        let group = str_to_int(word).clamp(0, 2);
        let level = self.level_of(target).unwrap_or(1);
        if let Ok(byte) = u8::try_from(group) {
            if level >= 5 {
                if let Some(body) = self.bodies.get_mut(&target) {
                    body.card.skill_group = byte;
                }
                let mut out = Vec::new();
                GcHeaderAndByte::new(HEADER_GC_CHANGE_SKILL_GROUP.value(), byte)
                    .encode_into(&mut out);
                let _sent = self.write_to_client(target, out);
            }
        }
        let cleared_skills = self.clear_skills(target, level);
        let pid = self.characters.find_by_vid(target).ok()?.player_id();
        let (race, group) = self
            .bodies
            .get(&target)
            .map_or((0, 0), |body| (body.card.job, body.card.skill_group));
        Some(SetAttrStored {
            pid,
            gold: None,
            identity: Some((race, group)),
            cleared_skills,
        })
    }

    /// The alignment arm: the real alignment moves to the word through
    /// `UpdateAlignment` (`__FIX_UPDATE_ALIGNMENT__`).
    fn set_alignment(&mut self, target: Vid, word: &[u8]) {
        let amount = str_to_int(word);
        let real = self
            .bodies
            .get(&target)
            .map_or(0, |body| body.card.alignment);
        self.update_alignment(target, amount.saturating_sub(real));
    }

    /// The gaya arm: `PointChange(POINT_GAYA, gaya, true)` on the target, which banks
    /// for the save.
    fn set_gaya_field(&mut self, target: Vid, word: &[u8]) {
        self.set_point(target, common::point_slot::POINT_GAYA, word);
    }

    /// The mesh-change workaround both identity arms run (`:1952-1956`, `:1976-1980`):
    /// `SetPolymorph(101)` then `SetPolymorph(0)`, each a reencode, four zero-amount
    /// changes through the attributes, and the battle compute. The polymorph point and
    /// the combo resets are gaps: no ported path reads them, so both calls change
    /// nothing but what this function sends.
    fn set_polymorph_mesh(&mut self, target: Vid) {
        for _ in 0..2 {
            let place = self.place_of(target.raw());
            let spot = self.spot_of(target.raw());
            let mut effects = Vec::new();
            if let (Some(place), Some(spot)) = (place, spot) {
                if let Some(index) = self.maps.get_mut(&place) {
                    let _shown = index.show(
                        EntityKey::Character(target.raw()),
                        (spot.x, spot.y, spot.z),
                        self.view_radius,
                        &mut effects,
                    );
                }
            }
            self.deliver(&effects, None);
            for kind in [
                common::point_slot::POINT_ST,
                common::point_slot::POINT_DX,
                common::point_slot::POINT_IQ,
                common::point_slot::POINT_HT,
            ] {
                let records = self
                    .pc_points_mut(target.raw())
                    .and_then(|points| points.point_change(kind, 0, false, false).ok());
                if let Some(records) = records {
                    self.send_point_records(target, &records);
                }
            }
            let computed = self
                .pc_points_mut(target.raw())
                .map(world::character::Points::compute_battle_points)
                .unwrap_or_default();
            self.send_point_records(target, &computed);
        }
    }

    /// `FindPC`: the VID of the online character whose shown name matches `name`
    /// ASCII case-insensitively, or `None`.
    fn find_pc_vid(&self, name: &[u8]) -> Option<Vid> {
        self.characters
            .player_names()
            .into_iter()
            .find(|(shown, _)| shown.as_bytes().eq_ignore_ascii_case(name))
            .map(|(_, vid)| vid)
    }
}

/// `str_to_number(long long)` (`C/utils.h:70-76`): `strtoull` base 10 — leading spaces
/// and one sign skipped, digits to the first non-digit, clamped at `ULLONG_MAX` — read
/// back as the `long long` (negatives wrap mod 2^64, as the cast does).
fn str_to_long_long(text: &[u8]) -> i64 {
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
    let mut value: u64 = 0;
    for byte in bytes {
        let Some(digit) = byte.checked_sub(b'0').filter(|digit| *digit < 10) else {
            break;
        };
        value = value.saturating_mul(10).saturating_add(u64::from(digit));
    }
    // The `strtoull` to `long long` cast: the bits read back signed.
    let bits = if negative {
        0u64.wrapping_sub(value)
    } else {
        value
    };
    i64::from_ne_bytes(bits.to_ne_bytes())
}

#[cfg(test)]
mod tests {
    use super::{str_to_long_long, FIELDS};

    #[test]
    fn the_long_long_wraps_signs_and_clamps() {
        assert_eq!(str_to_long_long(b"12"), 12);
        assert_eq!(str_to_long_long(b"  -5x"), -5);
        assert_eq!(str_to_long_long(b"+7"), 7);
        assert_eq!(str_to_long_long(b""), 0);
        assert_eq!(str_to_long_long(b"9223372036854775807"), i64::MAX);
        // Past `u64::MAX` clamps, then reads back as -1, as the `strtoull` cast does.
        assert_eq!(str_to_long_long(b"99999999999999999999999"), -1);
        assert_eq!(str_to_long_long(b"-9223372036854775809"), i64::MAX);
    }

    #[test]
    fn the_prefix_match_follows_the_table_order() {
        let at = |word: &[u8]| {
            FIELDS
                .iter()
                .position(|(name, _)| name.as_bytes().starts_with(word))
        };
        assert_eq!(at(b"g"), Some(0));
        assert_eq!(at(b"gold"), Some(0));
        assert_eq!(at(b"Gold"), None, "the match is case-sensitive");
        assert_eq!(at(b"goldx"), None, "past the end matches nothing");
        assert_eq!(at(b"align"), Some(8), "align meets alignment first");
        assert_eq!(at(b"a"), Some(8));
        assert_eq!(at(b"max_"), Some(5));
        assert_eq!(at(b"nosuch"), None);
        assert_eq!(FIELDS.len(), 11);
    }
}
