//! The polymorph power table (`aiPolymorphPowerByLevel`,
//! `G/constants.cpp:1089-1131`, indexed by the `SKILL_POLYMORPH` (129) level clamped to 0..40).
//!
//! `GetPolymorphPower` (`G/char.cpp:7669-7678`) reads the `poly` event flag first on the
//! test server and falls back to this table. The caller owns the flag, so this module is
//! the table only.

/// `SKILL_MAX_LEVEL` (`C/length.h:75`): the table's top climb.
pub const POLYMORPH_MAX_LEVEL: usize = 40;

/// `SKILL_POLYMORPH` (`G/skill.h:150`): the skill whose level climbs the table.
pub const SKILL_POLYMORPH: u32 = 129;

/// `aiPolymorphPowerByLevel` (`G/constants.cpp:1089-1131`): the power at each climb 0..40.
/// Level 0 is legacy's zeroed slot (the array is `SKILL_MAX_LEVEL + 1` wide and the
/// initialiser names climbs 1..40); `GetSkillLevel` never returns it for an unlearned
/// skill, but the index is clamped, not gated, so it stays part of the table.
pub const POLYMORPH_POWER_BY_LEVEL: [i32; POLYMORPH_MAX_LEVEL + 1] = [
    0, 10, 11, 11, 12, 13, 13, 14, 15, 16, 17, 18, 19, 20, 22, 23, 24, 26, 27, 29, 31, 33, 35, 37,
    39, 41, 44, 46, 49, 52, 55, 59, 62, 66, 70, 74, 79, 84, 89, 94, 100,
];

/// The power at `level`, clamped to 0..40 (`MINMAX`, `G/char.cpp:7677`).
#[must_use]
pub const fn polymorph_power(level: u8) -> i32 {
    let level = if level as usize > POLYMORPH_MAX_LEVEL {
        POLYMORPH_MAX_LEVEL
    } else {
        level as usize
    };
    POLYMORPH_POWER_BY_LEVEL[level]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table's shape and its pinned climbs: the zeroed slot, the doubled 11 and 13,
    /// the 20s' steps, and the perfect 100.
    #[test]
    fn the_table_holds_41_climbs_with_the_legacy_steps() {
        assert_eq!(POLYMORPH_POWER_BY_LEVEL.len(), 41);
        assert_eq!(POLYMORPH_POWER_BY_LEVEL[0], 0);
        assert_eq!(polymorph_power(1), 10);
        assert_eq!(polymorph_power(2), 11);
        assert_eq!(polymorph_power(3), 11);
        assert_eq!(polymorph_power(13), 20);
        assert_eq!(polymorph_power(25), 41);
        assert_eq!(polymorph_power(40), 100);
        assert_eq!(polymorph_power(41), 100);
        assert_eq!(polymorph_power(u8::MAX), 100);
    }
}
