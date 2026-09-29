//! The `APPLY_*` bonus types an item carries, and the point slot each one changes.
//!
//! An item's proto has three applies, and an item has seven attributes and six sockets that can
//! hold more; each is an apply type and a value. `CHARACTER::ApplyPoint` (`char.cpp:4826-5040`)
//! turns one into point changes. Most types change one point slot by the value, and the slot is
//! the type's row in `aApplyInfo` (`constants.cpp:717-864`). Seven types have an arm of their
//! own, and one has none. [`ApplyArm::of`] sorts a type into those arms and
//! [`crate::character::Points::apply_point`] carries them out.
//!
//! The numbering is `EApplyTypes` (`server/server/common/length.h:495-640`) as this build
//! compiles it: `__CONQUEROR_LEVEL__` and `BONUS_PCT` are on, so there are 130 types. The
//! tests read the legacy source and pin the table here to it.
//!
//! # Where the Rewrite differs from legacy, and why
//!
//! - **`APPLY_ATTBONUS_BOSS` raises the boss bonus.** `aApplyInfo` lists
//!   `POINT_ATTBONUS_METIN` in row 90 and `POINT_ATTBONUS_BOSS` in row 91
//!   (`constants.cpp:820-821`), while `EApplyTypes` numbers `APPLY_ATTBONUS_BOSS` 90 and
//!   `APPLY_ATTBONUS_METIN` 91. The battle code reads `POINT_ATTBONUS_METIN` against a stone
//!   and `POINT_ATTBONUS_BOSS` against a boss (`battle.cpp:366-369`), so in legacy an item
//!   that says it is strong against bosses is strong against metin stones instead, and the
//!   reverse. Wearing the item is enough to trigger it, so it is a Defect. The Rewrite maps each
//!   type to the point of its own name; the set bonuses in `constants.cpp:204-275` grant the
//!   boss bonus they name.
//! - **An unknown type logs nothing.** Legacy's `default` arm writes a `sys_err` line
//!   for `APPLY_EXTRACT_HP_PCT` (75) and for any type at or above 130. The Rewrite does nothing
//!   for them, as legacy does, without the log line.

use common::point_slot as point;

/// `APPLY_NONE`.
pub const APPLY_NONE: u8 = 0;
/// `APPLY_MAX_HP`.
pub const APPLY_MAX_HP: u8 = 1;
/// `APPLY_MAX_SP`.
pub const APPLY_MAX_SP: u8 = 2;
/// `APPLY_CON`.
pub const APPLY_CON: u8 = 3;
/// `APPLY_INT`.
pub const APPLY_INT: u8 = 4;
/// `APPLY_ATTBONUS_HUMAN`.
pub const APPLY_ATTBONUS_HUMAN: u8 = 17;
/// `APPLY_SKILL`: a skill-damage bonus packed into the value.
pub const APPLY_SKILL: u8 = 51;
/// `APPLY_ATT_GRADE_BONUS`.
pub const APPLY_ATT_GRADE_BONUS: u8 = 53;
/// `APPLY_DEF_GRADE_BONUS`.
pub const APPLY_DEF_GRADE_BONUS: u8 = 54;
/// `APPLY_MAGIC_ATT_GRADE`.
pub const APPLY_MAGIC_ATT_GRADE: u8 = 55;
/// `APPLY_MAGIC_DEF_GRADE`.
pub const APPLY_MAGIC_DEF_GRADE: u8 = 56;
/// `APPLY_ATTBONUS_MONSTER`.
pub const APPLY_ATTBONUS_MONSTER: u8 = 63;
/// `APPLY_MAX_HP_PCT`.
pub const APPLY_MAX_HP_PCT: u8 = 69;
/// `APPLY_MAX_SP_PCT`.
pub const APPLY_MAX_SP_PCT: u8 = 70;
/// `APPLY_EXTRACT_HP_PCT`, which `ApplyPoint` has no arm for.
pub const APPLY_EXTRACT_HP_PCT: u8 = 75;
/// `APPLY_ENERGY`.
pub const APPLY_ENERGY: u8 = 80;
/// `APPLY_COSTUME_ATTR_BONUS`.
pub const APPLY_COSTUME_ATTR_BONUS: u8 = 82;
/// `APPLY_ATTBONUS_BOSS`.
pub const APPLY_ATTBONUS_BOSS: u8 = 90;
/// `APPLY_ATTBONUS_METIN`.
pub const APPLY_ATTBONUS_METIN: u8 = 91;
/// `APPLY_ENCHANT_ELECT`, the first of the six element enchantments.
pub const APPLY_ENCHANT_ELECT: u8 = 92;

/// `MAX_APPLY_NUM`: the number of apply types this build compiles.
pub const MAX_APPLY_NUM: usize = 130;

/// The point slot each apply type changes: `aApplyInfo` with rows 90 and 91 exchanged.
///
/// Rows 0 (`APPLY_NONE`), 51 (`APPLY_SKILL`) and 75 (`APPLY_EXTRACT_HP_PCT`) are
/// `POINT_NONE`: none of them changes a point through this table.
const APPLY_POINT: [usize; MAX_APPLY_NUM] = [
    point::POINT_NONE,                      // 0 APPLY_NONE
    point::POINT_MAX_HP,                    // 1 APPLY_MAX_HP
    point::POINT_MAX_SP,                    // 2 APPLY_MAX_SP
    point::POINT_HT,                        // 3 APPLY_CON
    point::POINT_IQ,                        // 4 APPLY_INT
    point::POINT_ST,                        // 5 APPLY_STR
    point::POINT_DX,                        // 6 APPLY_DEX
    point::POINT_ATT_SPEED,                 // 7 APPLY_ATT_SPEED
    point::POINT_MOV_SPEED,                 // 8 APPLY_MOV_SPEED
    point::POINT_CASTING_SPEED,             // 9 APPLY_CAST_SPEED
    point::POINT_HP_REGEN,                  // 10 APPLY_HP_REGEN
    point::POINT_SP_REGEN,                  // 11 APPLY_SP_REGEN
    point::POINT_POISON_PCT,                // 12 APPLY_POISON_PCT
    point::POINT_STUN_PCT,                  // 13 APPLY_STUN_PCT
    point::POINT_SLOW_PCT,                  // 14 APPLY_SLOW_PCT
    point::POINT_CRITICAL_PCT,              // 15 APPLY_CRITICAL_PCT
    point::POINT_PENETRATE_PCT,             // 16 APPLY_PENETRATE_PCT
    point::POINT_ATTBONUS_HUMAN,            // 17 APPLY_ATTBONUS_HUMAN
    point::POINT_ATTBONUS_ANIMAL,           // 18 APPLY_ATTBONUS_ANIMAL
    point::POINT_ATTBONUS_ORC,              // 19 APPLY_ATTBONUS_ORC
    point::POINT_ATTBONUS_MILGYO,           // 20 APPLY_ATTBONUS_MILGYO
    point::POINT_ATTBONUS_UNDEAD,           // 21 APPLY_ATTBONUS_UNDEAD
    point::POINT_ATTBONUS_DEVIL,            // 22 APPLY_ATTBONUS_DEVIL
    point::POINT_STEAL_HP,                  // 23 APPLY_STEAL_HP
    point::POINT_STEAL_SP,                  // 24 APPLY_STEAL_SP
    point::POINT_MANA_BURN_PCT,             // 25 APPLY_MANA_BURN_PCT
    point::POINT_DAMAGE_SP_RECOVER,         // 26 APPLY_DAMAGE_SP_RECOVER
    point::POINT_BLOCK,                     // 27 APPLY_BLOCK
    point::POINT_DODGE,                     // 28 APPLY_DODGE
    point::POINT_RESIST_SWORD,              // 29 APPLY_RESIST_SWORD
    point::POINT_RESIST_TWOHAND,            // 30 APPLY_RESIST_TWOHAND
    point::POINT_RESIST_DAGGER,             // 31 APPLY_RESIST_DAGGER
    point::POINT_RESIST_BELL,               // 32 APPLY_RESIST_BELL
    point::POINT_RESIST_FAN,                // 33 APPLY_RESIST_FAN
    point::POINT_RESIST_BOW,                // 34 APPLY_RESIST_BOW
    point::POINT_RESIST_FIRE,               // 35 APPLY_RESIST_FIRE
    point::POINT_RESIST_ELEC,               // 36 APPLY_RESIST_ELEC
    point::POINT_RESIST_MAGIC,              // 37 APPLY_RESIST_MAGIC
    point::POINT_RESIST_WIND,               // 38 APPLY_RESIST_WIND
    point::POINT_REFLECT_MELEE,             // 39 APPLY_REFLECT_MELEE
    point::POINT_REFLECT_CURSE,             // 40 APPLY_REFLECT_CURSE
    point::POINT_POISON_REDUCE,             // 41 APPLY_POISON_REDUCE
    point::POINT_KILL_SP_RECOVER,           // 42 APPLY_KILL_SP_RECOVER
    point::POINT_EXP_DOUBLE_BONUS,          // 43 APPLY_EXP_DOUBLE_BONUS
    point::POINT_GOLD_DOUBLE_BONUS,         // 44 APPLY_GOLD_DOUBLE_BONUS
    point::POINT_ITEM_DROP_BONUS,           // 45 APPLY_ITEM_DROP_BONUS
    point::POINT_POTION_BONUS,              // 46 APPLY_POTION_BONUS
    point::POINT_KILL_HP_RECOVERY,          // 47 APPLY_KILL_HP_RECOVER
    point::POINT_IMMUNE_STUN,               // 48 APPLY_IMMUNE_STUN
    point::POINT_IMMUNE_SLOW,               // 49 APPLY_IMMUNE_SLOW
    point::POINT_IMMUNE_FALL,               // 50 APPLY_IMMUNE_FALL
    point::POINT_NONE,                      // 51 APPLY_SKILL
    point::POINT_BOW_DISTANCE,              // 52 APPLY_BOW_DISTANCE
    point::POINT_ATT_GRADE_BONUS,           // 53 APPLY_ATT_GRADE_BONUS
    point::POINT_DEF_GRADE_BONUS,           // 54 APPLY_DEF_GRADE_BONUS
    point::POINT_MAGIC_ATT_GRADE_BONUS,     // 55 APPLY_MAGIC_ATT_GRADE
    point::POINT_MAGIC_DEF_GRADE_BONUS,     // 56 APPLY_MAGIC_DEF_GRADE
    point::POINT_CURSE_PCT,                 // 57 APPLY_CURSE_PCT
    point::POINT_MAX_STAMINA,               // 58 APPLY_MAX_STAMINA
    point::POINT_ATTBONUS_WARRIOR,          // 59 APPLY_ATTBONUS_WARRIOR
    point::POINT_ATTBONUS_ASSASSIN,         // 60 APPLY_ATTBONUS_ASSASSIN
    point::POINT_ATTBONUS_SURA,             // 61 APPLY_ATTBONUS_SURA
    point::POINT_ATTBONUS_SHAMAN,           // 62 APPLY_ATTBONUS_SHAMAN
    point::POINT_ATTBONUS_MONSTER,          // 63 APPLY_ATTBONUS_MONSTER
    point::POINT_ATT_BONUS,                 // 64 APPLY_MALL_ATTBONUS
    point::POINT_MALL_DEFBONUS,             // 65 APPLY_MALL_DEFBONUS
    point::POINT_MALL_EXPBONUS,             // 66 APPLY_MALL_EXPBONUS
    point::POINT_MALL_ITEMBONUS,            // 67 APPLY_MALL_ITEMBONUS
    point::POINT_MALL_GOLDBONUS,            // 68 APPLY_MALL_GOLDBONUS
    point::POINT_MAX_HP_PCT,                // 69 APPLY_MAX_HP_PCT
    point::POINT_MAX_SP_PCT,                // 70 APPLY_MAX_SP_PCT
    point::POINT_SKILL_DAMAGE_BONUS,        // 71 APPLY_SKILL_DAMAGE_BONUS
    point::POINT_NORMAL_HIT_DAMAGE_BONUS,   // 72 APPLY_NORMAL_HIT_DAMAGE_BONUS
    point::POINT_SKILL_DEFEND_BONUS,        // 73 APPLY_SKILL_DEFEND_BONUS
    point::POINT_NORMAL_HIT_DEFEND_BONUS,   // 74 APPLY_NORMAL_HIT_DEFEND_BONUS
    point::POINT_NONE,                      // 75 APPLY_EXTRACT_HP_PCT
    point::POINT_RESIST_WARRIOR,            // 76 APPLY_RESIST_WARRIOR
    point::POINT_RESIST_ASSASSIN,           // 77 APPLY_RESIST_ASSASSIN
    point::POINT_RESIST_SURA,               // 78 APPLY_RESIST_SURA
    point::POINT_RESIST_SHAMAN,             // 79 APPLY_RESIST_SHAMAN
    point::POINT_ENERGY,                    // 80 APPLY_ENERGY
    point::POINT_DEF_GRADE,                 // 81 APPLY_DEF_GRADE
    point::POINT_COSTUME_ATTR_BONUS,        // 82 APPLY_COSTUME_ATTR_BONUS
    point::POINT_MAGIC_ATT_BONUS_PER,       // 83 APPLY_MAGIC_ATTBONUS_PER
    point::POINT_MELEE_MAGIC_ATT_BONUS_PER, // 84 APPLY_MELEE_MAGIC_ATTBONUS_PER
    point::POINT_RESIST_ICE,                // 85 APPLY_RESIST_ICE
    point::POINT_RESIST_EARTH,              // 86 APPLY_RESIST_EARTH
    point::POINT_RESIST_DARK,               // 87 APPLY_RESIST_DARK
    point::POINT_RESIST_CRITICAL,           // 88 APPLY_ANTI_CRITICAL_PCT
    point::POINT_RESIST_PENETRATE,          // 89 APPLY_ANTI_PENETRATE_PCT
    point::POINT_ATTBONUS_BOSS,             // 90 APPLY_ATTBONUS_BOSS
    point::POINT_ATTBONUS_METIN,            // 91 APPLY_ATTBONUS_METIN
    point::POINT_ENCHANT_ELECT,             // 92 APPLY_ENCHANT_ELECT
    point::POINT_ENCHANT_FIRE,              // 93 APPLY_ENCHANT_FIRE
    point::POINT_ENCHANT_ICE,               // 94 APPLY_ENCHANT_ICE
    point::POINT_ENCHANT_WIND,              // 95 APPLY_ENCHANT_WIND
    point::POINT_ENCHANT_EARTH,             // 96 APPLY_ENCHANT_EARTH
    point::POINT_ENCHANT_DARK,              // 97 APPLY_ENCHANT_DARK
    point::POINT_SUNGMA_STR,                // 98 APPLY_SUNGMA_STR
    point::POINT_SUNGMA_HP,                 // 99 APPLY_SUNGMA_HP
    point::POINT_SUNGMA_MOVE,               // 100 APPLY_SUNGMA_MOVE
    point::POINT_SUNGMA_IMMUNE,             // 101 APPLY_SUNGMA_IMMUNE
    point::POINT_ATTBONUS_ANIMAL_PCT,       // 102 APPLY_ATTBONUS_ANIMAL_PCT
    point::POINT_ATTBONUS_UNDEAD_PCT,       // 103 APPLY_ATTBONUS_UNDEAD_PCT
    point::POINT_ATTBONUS_DEVIL_PCT,        // 104 APPLY_ATTBONUS_DEVIL_PCT
    point::POINT_ATTBONUS_ORC_PCT,          // 105 APPLY_ATTBONUS_ORC_PCT
    point::POINT_ATTBONUS_MILGYO_PCT,       // 106 APPLY_ATTBONUS_MILGYO_PCT
    point::POINT_ATTBONUS_DESERT_PCT,       // 107 APPLY_ATTBONUS_DESERT_PCT
    point::POINT_ATTBONUS_INSECT_PCT,       // 108 APPLY_ATTBONUS_INSECT_PCT
    point::POINT_ATTBONUS_TREE_PCT,         // 109 APPLY_ATTBONUS_TREE_PCT
    point::POINT_ATTBONUS_BOSS_PCT,         // 110 APPLY_ATTBONUS_BOSS_PCT
    point::POINT_ATTBONUS_METIN_PCT,        // 111 APPLY_ATTBONUS_METIN_PCT
    point::POINT_ATTBONUS_CZ_PCT,           // 112 APPLY_ATTBONUS_CZ_PCT
    point::POINT_ATTBONUS_HUMAN_PCT,        // 113 APPLY_ATTBONUS_HUMAN_PCT
    point::POINT_ATTBONUS_MONSTER_PCT,      // 114 APPLY_ATTBONUS_MONSTER_PCT
    point::POINT_ENCHANT_ELECT_PCT,         // 115 APPLY_ENCHANT_ELECT_PCT
    point::POINT_ENCHANT_FIRE_PCT,          // 116 APPLY_ENCHANT_FIRE_PCT
    point::POINT_ENCHANT_ICE_PCT,           // 117 APPLY_ENCHANT_ICE_PCT
    point::POINT_ENCHANT_WIND_PCT,          // 118 APPLY_ENCHANT_WIND_PCT
    point::POINT_ENCHANT_EARTH_PCT,         // 119 APPLY_ENCHANT_EARTH_PCT
    point::POINT_ENCHANT_DARK_PCT,          // 120 APPLY_ENCHANT_DARK_PCT
    point::POINT_RESIST_ELECT_PCT,          // 121 APPLY_RESIST_ELECT_PCT
    point::POINT_RESIST_FIRE_PCT,           // 122 APPLY_RESIST_FIRE_PCT
    point::POINT_RESIST_ICE_PCT,            // 123 APPLY_RESIST_ICE_PCT
    point::POINT_RESIST_WIND_PCT,           // 124 APPLY_RESIST_WIND_PCT
    point::POINT_RESIST_EARTH_PCT,          // 125 APPLY_RESIST_EARTH_PCT
    point::POINT_RESIST_DARK_PCT,           // 126 APPLY_RESIST_DARK_PCT
    point::POINT_RESIST_HUMAN_PCT,          // 127 APPLY_RESIST_HUMAN_PCT
    point::POINT_RESIST_FALL,               // 128 APPLY_RESIST_FALL
    point::POINT_RESIST_COMBAT,             // 129 APPLY_RESIST_COMBAT
];

/// What `ApplyPoint` does with one apply type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyArm {
    /// `APPLY_NONE`, `APPLY_EXTRACT_HP_PCT`, and every type at or above
    /// [`MAX_APPLY_NUM`]: nothing changes.
    Nothing,
    /// `APPLY_CON`: the vitality, and the maximum hit points and stamina it gives.
    Con,
    /// `APPLY_INT`: the intelligence, and the maximum spell points it gives.
    Int,
    /// `APPLY_SKILL`: the skill-damage bonus of one skill.
    Skill,
    /// `APPLY_MAX_HP` or `APPLY_MAX_HP_PCT`: the slot, and the current hit points keep
    /// their share of the maximum.
    MaxHp(usize),
    /// `APPLY_MAX_SP` or `APPLY_MAX_SP_PCT`: the slot, and the current spell points keep
    /// their share of the maximum.
    MaxSp(usize),
    /// Every other type: this one point slot changes by the value.
    Point(usize),
}

impl ApplyArm {
    /// The arm `ApplyPoint` takes for `apply`.
    pub fn of(apply: u8) -> Self {
        match apply {
            APPLY_NONE | APPLY_EXTRACT_HP_PCT => Self::Nothing,
            APPLY_CON => Self::Con,
            APPLY_INT => Self::Int,
            APPLY_SKILL => Self::Skill,
            APPLY_MAX_HP | APPLY_MAX_HP_PCT => Self::MaxHp(APPLY_POINT[usize::from(apply)]),
            APPLY_MAX_SP | APPLY_MAX_SP_PCT => Self::MaxSp(APPLY_POINT[usize::from(apply)]),
            _ => APPLY_POINT
                .get(usize::from(apply))
                .map_or(Self::Nothing, |slot| Self::Point(*slot)),
        }
    }

    /// The arm for an apply type read from an item proto, where the reader keeps it as an
    /// `i32`.
    ///
    /// The reader refuses a name outside the table, so the value is always below
    /// [`MAX_APPLY_NUM`]; anything else takes no arm.
    pub fn of_proto(apply: i32) -> Self {
        u8::try_from(apply).map_or(Self::Nothing, Self::of)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use gamedata::item_proto_value::{self, APPLY_TYPE};

    use super::*;
    use crate::character::legacy_source::{
        legacy, names, preprocess, span, strip_comments, switches,
    };

    /// `POINT_*` name to slot, from `common::point_slot`, whose own tests pin it to `char.h`.
    fn point_slots() -> BTreeMap<String, usize> {
        let text = std::fs::read_to_string("../common/src/point_slot.rs").expect("point_slot.rs");
        text.lines()
            .filter_map(|line| line.trim().strip_prefix("pub const "))
            .filter_map(|rest| {
                let (name, value) = rest.split_once(": usize = ")?;
                let value = value.strip_suffix(';')?.parse().ok()?;
                Some((name.to_owned(), value))
            })
            .collect()
    }

    fn apply_index(name: &str) -> usize {
        APPLY_TYPE
            .iter()
            .position(|entry| *entry == name)
            .unwrap_or_else(|| panic!("{name} is not an apply type"))
    }

    fn at(apply: usize) -> u8 {
        u8::try_from(apply).expect("an apply type fits a byte")
    }

    #[test]
    fn the_enum_compiles_to_the_readers_table() {
        let text = strip_comments(&legacy("common/length.h"));
        let body = span(&text, "enum EApplyTypes", "};").join("\n");
        let members: Vec<String> = preprocess(&body, &switches())
            .iter()
            .flat_map(|line| names(line, "APPLY_"))
            .map(str::to_owned)
            .collect();
        assert!(!body.contains('='), "every member is auto-numbered");
        assert_eq!(members, APPLY_TYPE);
        assert_eq!(members.len(), MAX_APPLY_NUM);
        assert_eq!(
            u32::try_from(MAX_APPLY_NUM).expect("small"),
            item_proto_value::MAX_APPLY_NUM
        );
    }

    #[test]
    fn each_named_apply_is_its_enum_member() {
        for (value, name) in [
            (APPLY_NONE, "APPLY_NONE"),
            (APPLY_MAX_HP, "APPLY_MAX_HP"),
            (APPLY_MAX_SP, "APPLY_MAX_SP"),
            (APPLY_CON, "APPLY_CON"),
            (APPLY_INT, "APPLY_INT"),
            (APPLY_ATTBONUS_HUMAN, "APPLY_ATTBONUS_HUMAN"),
            (APPLY_SKILL, "APPLY_SKILL"),
            (APPLY_ATT_GRADE_BONUS, "APPLY_ATT_GRADE_BONUS"),
            (APPLY_DEF_GRADE_BONUS, "APPLY_DEF_GRADE_BONUS"),
            (APPLY_MAGIC_ATT_GRADE, "APPLY_MAGIC_ATT_GRADE"),
            (APPLY_MAGIC_DEF_GRADE, "APPLY_MAGIC_DEF_GRADE"),
            (APPLY_ATTBONUS_MONSTER, "APPLY_ATTBONUS_MONSTER"),
            (APPLY_MAX_HP_PCT, "APPLY_MAX_HP_PCT"),
            (APPLY_MAX_SP_PCT, "APPLY_MAX_SP_PCT"),
            (APPLY_EXTRACT_HP_PCT, "APPLY_EXTRACT_HP_PCT"),
            (APPLY_ENERGY, "APPLY_ENERGY"),
            (APPLY_COSTUME_ATTR_BONUS, "APPLY_COSTUME_ATTR_BONUS"),
            (APPLY_ATTBONUS_BOSS, "APPLY_ATTBONUS_BOSS"),
            (APPLY_ATTBONUS_METIN, "APPLY_ATTBONUS_METIN"),
            (APPLY_ENCHANT_ELECT, "APPLY_ENCHANT_ELECT"),
        ] {
            assert_eq!(usize::from(value), apply_index(name), "{name}");
        }
    }

    /// Every row of `aApplyInfo` is the table's row, except 90 and 91, which the Rewrite
    /// exchanges so that each type raises the point of its own name.
    #[test]
    fn the_point_table_is_legacys_with_boss_and_metin_exchanged() {
        let text = strip_comments(&legacy("game/constants.cpp"));
        let body = span(&text, "const TApplyInfo aApplyInfo[MAX_APPLY_NUM]", "};").join("\n");
        let slots = point_slots();
        let legacy_rows: Vec<usize> = preprocess(&body, &switches())
            .iter()
            .filter(|line| line.trim_start().starts_with('{'))
            .flat_map(|line| names(line, "POINT_"))
            .map(|name| slots[name])
            .collect();
        assert_eq!(legacy_rows.len(), MAX_APPLY_NUM);
        let mut repaired = legacy_rows.clone();
        repaired.swap(
            usize::from(APPLY_ATTBONUS_BOSS),
            usize::from(APPLY_ATTBONUS_METIN),
        );
        assert_eq!(APPLY_POINT.to_vec(), repaired);
        // The exchange is the repair: legacy's row for the boss type is the metin point.
        assert_eq!(legacy_rows[90], point::POINT_ATTBONUS_METIN);
        assert_eq!(legacy_rows[91], point::POINT_ATTBONUS_BOSS);
        assert_eq!(APPLY_POINT[90], point::POINT_ATTBONUS_BOSS);
        assert_eq!(APPLY_POINT[91], point::POINT_ATTBONUS_METIN);
    }

    /// The case groups of `ApplyPoint`'s switch, each with the statements it runs.
    fn apply_point_cases() -> Vec<(Vec<String>, String)> {
        let text = strip_comments(&legacy("game/char.cpp"));
        let body = span(
            &text,
            "void CHARACTER::ApplyPoint(BYTE bApplyType, int iVal)",
            "}",
        );
        let lines = preprocess(&body.join("\n"), &switches());
        let mut groups: Vec<(Vec<String>, String)> = Vec::new();
        let mut in_body = false;
        for line in lines.iter().map(|line| line.trim()) {
            let label = line.strip_prefix("case ").or(if line == "default:" {
                Some("default:")
            } else {
                None
            });
            match label {
                Some(label) => {
                    if in_body || groups.is_empty() {
                        groups.push((Vec::new(), String::new()));
                        in_body = false;
                    }
                    let name = label.trim_end_matches(':').trim().to_owned();
                    groups.last_mut().expect("a group").0.push(name);
                }
                None if !line.is_empty() && !groups.is_empty() => {
                    in_body = true;
                    let statements = &mut groups.last_mut().expect("a group").1;
                    statements.push_str(line);
                    statements.push(' ');
                }
                None => {}
            }
        }
        groups
    }

    #[test]
    fn apply_point_has_an_arm_for_every_type_but_extract_hp() {
        let labelled: BTreeSet<usize> = apply_point_cases()
            .iter()
            .flat_map(|(labels, _)| labels.iter())
            .filter(|label| label.as_str() != "default")
            .map(|label| apply_index(label))
            .collect();
        let expected: BTreeSet<usize> = (0..MAX_APPLY_NUM)
            .filter(|apply| *apply != usize::from(APPLY_EXTRACT_HP_PCT))
            .collect();
        assert_eq!(labelled, expected);
        assert_eq!(ApplyArm::of(APPLY_EXTRACT_HP_PCT), ApplyArm::Nothing);
    }

    #[test]
    fn each_arm_is_the_one_the_switch_takes() {
        let cases = apply_point_cases();
        // NONE, CON, INT, SKILL, the two hit-point types, the two spell-point types, the rest,
        // and the default.
        assert_eq!(cases.len(), 8);
        let mut seen = 0;
        for (labels, statements) in &cases {
            let group: Vec<&str> = labels.iter().map(String::as_str).collect();
            let arms: Vec<ApplyArm> = group
                .iter()
                .filter(|label| **label != "default")
                .map(|label| ApplyArm::of(at(apply_index(label))))
                .collect();
            match group.as_slice() {
                ["APPLY_NONE"] => {
                    assert!(statements.starts_with("break;"), "{statements}");
                    assert_eq!(arms, [ApplyArm::Nothing]);
                }
                ["APPLY_CON"] => {
                    assert!(statements.contains("PointChange(POINT_HT, iVal)"));
                    assert!(statements.contains("hp_per_ht"));
                    assert!(statements.contains("stamina_per_con"));
                    assert_eq!(arms, [ApplyArm::Con]);
                }
                ["APPLY_INT"] => {
                    assert!(statements.contains("PointChange(POINT_IQ, iVal)"));
                    assert!(statements.contains("sp_per_iq"));
                    assert_eq!(arms, [ApplyArm::Int]);
                }
                ["APPLY_SKILL"] => {
                    assert!(statements.contains("m_SkillDamageBonus"));
                    assert_eq!(arms, [ApplyArm::Skill]);
                }
                ["APPLY_MAX_HP", "APPLY_MAX_HP_PCT"] => {
                    assert!(statements.contains("GetMaxHP()"));
                    assert!(statements.contains("PointChange(POINT_HP,"));
                    let hp = ApplyArm::MaxHp(point::POINT_MAX_HP);
                    let pct = ApplyArm::MaxHp(point::POINT_MAX_HP_PCT);
                    assert_eq!(arms, [hp, pct]);
                }
                ["APPLY_MAX_SP", "APPLY_MAX_SP_PCT"] => {
                    assert!(statements.contains("GetMaxSP()"));
                    assert!(statements.contains("PointChange(POINT_SP,"));
                    let sp = ApplyArm::MaxSp(point::POINT_MAX_SP);
                    let pct = ApplyArm::MaxSp(point::POINT_MAX_SP_PCT);
                    assert_eq!(arms, [sp, pct]);
                }
                ["default"] => assert!(statements.starts_with("sys_err("), "{statements}"),
                _ => {
                    assert_eq!(
                        statements.trim(),
                        "PointChange(aApplyInfo[bApplyType].bPointType, iVal); break;",
                        "{group:?}"
                    );
                    for (label, arm) in group.iter().zip(&arms) {
                        let slot = APPLY_POINT[apply_index(label)];
                        assert_eq!(*arm, ApplyArm::Point(slot), "{label}");
                        assert_ne!(slot, point::POINT_NONE, "{label}");
                    }
                }
            }
            seen += group.len();
        }
        // 129 labelled types and the default.
        assert_eq!(seen, MAX_APPLY_NUM);
    }

    #[test]
    fn a_type_past_the_table_takes_no_arm() {
        for apply in 130..=u8::MAX {
            assert_eq!(ApplyArm::of(apply), ApplyArm::Nothing, "{apply}");
        }
        assert_eq!(ApplyArm::of_proto(-1), ApplyArm::Nothing);
        assert_eq!(ApplyArm::of_proto(256 + 5), ApplyArm::Nothing);
        assert_eq!(ApplyArm::of_proto(5), ApplyArm::Point(point::POINT_ST));
        assert_eq!(
            ApplyArm::of_proto(129),
            ApplyArm::Point(point::POINT_RESIST_COMBAT)
        );
    }
}
