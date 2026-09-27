//! The compiled-in level and job tables of `server/server/game/constants.cpp`.
//!
//! These are not Game data files: the legacy server holds them as C++ arrays and reads
//! Game data for everything else. The Rewrite therefore carries them as constants, and
//! every value is copied from the frozen legacy source rather than invented.
//!
//! | legacy | here |
//! |---|---|
//! | `PLAYER_MAX_LEVEL_CONST` (`common/length.h:60`) | `MAX_LEVEL` |
//! | `PLAYER_CONQUEROR_EXP_TABLE_MAX` (`common/length.h:63`) | `CONQUEROR_MAX_LEVEL` |
//! | `exp_table_common` (`constants.cpp:62-313`) | `EXP_TABLE` |
//! | `conqueror_exp_table` (`constants.cpp:283-318`) | `CONQUEROR_EXP_TABLE` |
//! | `JobInitialPoints` (`constants.cpp:4-23`) | `JOB_INITIAL_POINTS` |
//!
//! # The exp table is also loaded from the database
//!
//! `__LoadExpTableFromDB` (`config.cpp:1529-1554`) replaces the whole array from an
//! `exp_table` table when that query returns rows. The owner's SQL snapshot has no
//! `exp_table` table, so the compiled-in array is what legacy uses here and what the
//! Rewrite must use. A deployment that does have the table needs a config key before the
//! Rewrite can follow it; that is a separate change, recorded in `docs/STATUS.md`, not
//! silently assumed here.
//!
//! # Signedness
//!
//! Legacy declares both tables `DWORD` and returns `DWORD`, but the point slot
//! `POINT_NEXT_EXP` is a `long long`. The values are the same positive numbers either
//! way; the Rewrite widens at the point of use and keeps the table as `i64` so no
//! conversion can lose the level-250 value of 2 500 000 000.

/// `PLAYER_MAX_LEVEL_CONST = 250`: the highest level the table has an entry for.
pub const MAX_LEVEL: u8 = 250;

/// `PLAYER_CONQUEROR_EXP_TABLE_MAX = 30`: the highest conqueror level with an entry.
pub const CONQUEROR_MAX_LEVEL: u8 = 30;

/// The experience a level needs to reach the next one (`exp_table_common`; the index is
/// the level).
pub const EXP_TABLE: [i64; 251] = [
    0,
    300,
    800,
    1_500,
    2_500,
    4_300,
    7_200,
    11_000,
    17_000,
    24_000,
    33_000,
    43_000,
    58_000,
    76_000,
    100_000,
    130_000,
    169_000,
    219_000,
    283_000,
    365_000,
    472_000,
    610_000,
    705_000,
    813_000,
    937_000,
    1_077_000,
    1_237_000,
    1_418_000,
    1_624_000,
    1_857_000,
    2_122_000,
    2_421_000,
    2_761_000,
    3_145_000,
    3_580_000,
    4_073_000,
    4_632_000,
    5_194_000,
    5_717_000,
    6_264_000,
    6_837_000,
    7_600_000,
    8_274_000,
    8_990_000,
    9_753_000,
    10_560_000,
    11_410_000,
    12_320_000,
    13_270_000,
    14_280_000,
    15_340_000,
    16_870_000,
    18_960_000,
    19_980_000,
    21_420_000,
    22_930_000,
    24_530_000,
    26_200_000,
    27_960_000,
    29_800_000,
    32_780_000,
    36_060_000,
    39_670_000,
    43_640_000,
    48_000_000,
    52_800_000,
    58_080_000,
    63_890_000,
    70_280_000,
    77_310_000,
    85_040_000,
    93_540_000,
    102_900_000,
    113_200_000,
    124_500_000,
    137_000_000,
    150_700_000,
    165_700_000,
    236_990_000,
    260_650_000,
    286_780_000,
    315_380_000,
    346_970_000,
    381_680_000,
    419_770_000,
    461_760_000,
    508_040_000,
    558_740_000,
    614_640_000,
    676_130_000,
    743_730_000,
    1_041_222_000,
    1_145_344_200,
    1_259_878_620,
    1_385_866_482,
    1_524_453_130,
    1_676_898_443,
    1_844_588_288,
    2_029_047_116,
    2_050_000_000,
    2_150_000_000,
    2_210_000_000,
    2_250_000_000,
    2_280_000_000,
    2_310_000_000,
    2_330_000_000,
    2_350_000_000,
    2_370_000_000,
    2_390_000_000,
    2_400_000_000,
    2_410_000_000,
    2_420_000_000,
    2_430_000_000,
    2_440_000_000,
    2_450_000_000,
    2_460_000_000,
    2_470_000_000,
    2_480_000_000,
    2_490_000_000,
    2_490_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
    2_500_000_000,
];

/// The experience a conqueror level needs to reach the next one (`conqueror_exp_table`).
pub const CONQUEROR_EXP_TABLE: [i64; 31] = [
    0,
    433_400,
    1_300_204,
    2_600_413,
    4_334_029,
    6_501_054,
    9_101_490,
    12_135_339,
    15_602_603,
    19_503_284,
    23_837_384,
    28_604_905,
    33_805_849,
    39_440_218,
    45_508_014,
    52_009_239,
    58_943_895,
    66_311_984,
    74_113_508,
    82_348_469,
    91_016_869,
    100_118_710,
    109_653_994,
    119_622_723,
    130_024_899,
    140_860_524,
    152_129_600,
    163_832_129,
    175_968_113,
    188_537_554,
    188_537_554,
];

/// The experience a level needs to reach the next one, as `CHARACTER::GetNextExp`
/// (`char.cpp:8816-8823`): a level past the table's last entry is the sentinel
/// 2 500 000 000, not an index into the array.
#[must_use]
pub fn next_exp(level: u8) -> i64 {
    if level > MAX_LEVEL {
        return 2_500_000_000;
    }
    EXP_TABLE[usize::from(level)]
}

/// The experience a conqueror level needs, as `CHARACTER::GetConquerorNextExp`
/// (`char.cpp:8825-8833`).
#[must_use]
pub fn conqueror_next_exp(level: u8) -> i64 {
    if level > CONQUEROR_MAX_LEVEL {
        return 2_500_000_000;
    }
    CONQUEROR_EXP_TABLE[usize::from(level)]
}

/// One row of `TJobInitialPoints` (`constants.cpp:4-23`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobInitialPoints {
    /// Strength a new character of this race starts with.
    pub st: u8,
    /// Vitality.
    pub ht: u8,
    /// Dexterity.
    pub dx: u8,
    /// Intelligence.
    pub iq: u8,
    /// The `max_hp` base, before vitality.
    pub max_hp: i32,
    /// The `max_sp` base, before intelligence.
    pub max_sp: i32,
    /// `hp_per_ht`: hit points per point of vitality.
    pub hp_per_ht: i32,
    /// `sp_per_iq`: spell points per point of intelligence.
    pub sp_per_iq: i32,
    /// The `max_stamina` base.
    pub max_stamina: i32,
    /// `stamina_per_con`: stamina per point of vitality.
    pub stamina_per_con: i32,
}

/// `JobInitialPoints` by race. `JOB_MAX_NUM` is 4, and this tree has no wolfman.
pub const JOB_INITIAL_POINTS: [JobInitialPoints; 4] = [
    // JOB_WARRIOR
    JobInitialPoints {
        st: 6,
        ht: 4,
        dx: 3,
        iq: 3,
        max_hp: 600,
        max_sp: 200,
        hp_per_ht: 40,
        sp_per_iq: 20,
        max_stamina: 800,
        stamina_per_con: 5,
    },
    // JOB_ASSASSIN
    JobInitialPoints {
        st: 4,
        ht: 3,
        dx: 6,
        iq: 3,
        max_hp: 650,
        max_sp: 200,
        hp_per_ht: 40,
        sp_per_iq: 20,
        max_stamina: 800,
        stamina_per_con: 5,
    },
    // JOB_SURA
    JobInitialPoints {
        st: 5,
        ht: 3,
        dx: 3,
        iq: 5,
        max_hp: 650,
        max_sp: 200,
        hp_per_ht: 40,
        sp_per_iq: 20,
        max_stamina: 800,
        stamina_per_con: 5,
    },
    // JOB_SHAMAN
    JobInitialPoints {
        st: 3,
        ht: 4,
        dx: 3,
        iq: 6,
        max_hp: 700,
        max_sp: 200,
        hp_per_ht: 40,
        sp_per_iq: 20,
        max_stamina: 800,
        stamina_per_con: 5,
    },
];

/// The maximum hit points of a character, from `CHARACTER::Init` (`char.cpp:2953`):
/// the job base plus vitality, without the random bonus a new character is also given
/// and without the passive-skill bonus.
#[must_use]
pub fn max_hp(job: u8, ht: i64) -> i32 {
    let row = job_row(job);
    to_int(i64::from(row.max_hp) + ht * i64::from(row.hp_per_ht))
}

/// The maximum spell points of a character (`char.cpp:2953`).
#[must_use]
pub fn max_sp(job: u8, iq: i64) -> i32 {
    let row = job_row(job);
    to_int(i64::from(row.max_sp) + iq * i64::from(row.sp_per_iq))
}

/// The maximum stamina of a character (`char.cpp:2953`).
#[must_use]
pub fn max_stamina(job: u8, ht: i64) -> i32 {
    let row = job_row(job);
    to_int(i64::from(row.max_stamina) + ht * i64::from(row.stamina_per_con))
}

/// The narrowing from the 64-bit sum to the `int` legacy stores it in (`char.h:450-470`).
///
/// Legacy computes the sum in an `int`, so a total past `i32::MAX` would be a legacy wrap,
/// and a wrap is a Defect. The Rewrite does not reproduce it. The attributes reach this
/// function from the point slots, a client attribute byte is 0 to 255, and the widest row is
/// 40 per point, so the largest reachable total is 600 + 255 * 40 = 10 800. The clamp only
/// makes that bound total; it is not a path a client can reach.
fn to_int(total: i64) -> i32 {
    i32::try_from(total).unwrap_or(if total < 0 { i32::MIN } else { i32::MAX })
}

/// The initial-points row of a race. `JOB_MAX_NUM` is 4; a race past it is not a
/// character this server can make, and a created character's job is range-checked, so
/// the fall-back only keeps this total function.
fn job_row(job: u8) -> &'static JobInitialPoints {
    &JOB_INITIAL_POINTS[usize::from(job) % JOB_INITIAL_POINTS.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values a hand sum of the table would have to match, read straight out of
    /// `constants.cpp`.
    const PINNED: [(u8, i64); 12] = [
        (1, 300),
        (2, 800),
        (3, 1500),
        (4, 2500),
        (10, 33_000),
        (20, 472_000),
        (21, 610_000),
        (50, 15_340_000),
        (99, 2_050_000_000),
        (100, 2_150_000_000),
        (249, 2_500_000_000),
        (250, 2_500_000_000),
    ];

    #[test]
    fn the_exp_table_has_one_entry_per_level_plus_zero() {
        assert_eq!(EXP_TABLE.len(), usize::from(MAX_LEVEL) + 1);
        assert_eq!(
            CONQUEROR_EXP_TABLE.len(),
            usize::from(CONQUEROR_MAX_LEVEL) + 1
        );
        assert_eq!(MAX_LEVEL, 250);
        assert_eq!(CONQUEROR_MAX_LEVEL, 30);
    }

    #[test]
    fn the_pinned_exp_values_are_the_legacy_ones() {
        for (level, value) in PINNED {
            assert_eq!(next_exp(level), value, "level {level}");
        }
    }

    #[test]
    fn level_zero_is_the_sentinel_row() {
        assert_eq!(EXP_TABLE[0], 0);
        assert_eq!(next_exp(0), 0);
        assert_eq!(conqueror_next_exp(0), 0);
    }

    #[test]
    fn a_level_past_the_table_is_the_sentinel() {
        assert_eq!(next_exp(MAX_LEVEL + 1), 2_500_000_000);
        assert_eq!(next_exp(255), 2_500_000_000);
    }

    #[test]
    fn the_pinned_conqueror_values_are_the_legacy_ones() {
        assert_eq!(conqueror_next_exp(1), 433_400);
        assert_eq!(conqueror_next_exp(2), 1_300_204);
        assert_eq!(conqueror_next_exp(30), 188_537_554);
        assert_eq!(conqueror_next_exp(31), 2_500_000_000);
        assert_eq!(conqueror_next_exp(255), 2_500_000_000);
    }

    #[test]
    fn the_exp_table_rises_monotonically() {
        for level in 1..usize::from(MAX_LEVEL) {
            assert!(
                EXP_TABLE[level + 1] >= EXP_TABLE[level],
                "level {} costs more than level {level}",
                level + 1
            );
        }
    }

    #[test]
    fn the_four_job_rows_are_the_legacy_ones() {
        assert_eq!(JOB_INITIAL_POINTS[0].st, 6);
        assert_eq!(JOB_INITIAL_POINTS[0].max_hp, 600);
        assert_eq!(JOB_INITIAL_POINTS[1].max_hp, 650);
        assert_eq!(JOB_INITIAL_POINTS[1].dx, 6);
        assert_eq!(JOB_INITIAL_POINTS[2].iq, 5);
        assert_eq!(JOB_INITIAL_POINTS[3].max_hp, 700);
        assert_eq!(JOB_INITIAL_POINTS[3].iq, 6);
        for row in JOB_INITIAL_POINTS {
            assert_eq!(row.max_sp, 200);
            assert_eq!(row.hp_per_ht, 40);
            assert_eq!(row.sp_per_iq, 20);
            assert_eq!(row.max_stamina, 800);
            assert_eq!(row.stamina_per_con, 5);
        }
    }

    #[test]
    fn a_new_warriors_totals_are_the_base_plus_vitality() {
        // 600 + 4 * 40 = 760 hit points, 200 + 3 * 20 = 260 spell points, and
        // 800 + 4 * 5 = 820 stamina.
        assert_eq!(max_hp(0, 4), 760);
        assert_eq!(max_sp(0, 3), 260);
        assert_eq!(max_stamina(0, 4), 820);
    }

    #[test]
    fn every_race_has_totals_above_its_base() {
        for (job, row) in JOB_INITIAL_POINTS.iter().enumerate() {
            let job = u8::try_from(job).expect("four races fit in a u8");
            let ht = i64::from(row.ht);
            let iq = i64::from(row.iq);
            assert_eq!(
                max_hp(job, ht),
                row.max_hp + i32::from(row.ht) * row.hp_per_ht
            );
            assert_eq!(
                max_sp(job, iq),
                row.max_sp + i32::from(row.iq) * row.sp_per_iq
            );
            assert_eq!(
                max_stamina(job, ht),
                row.max_stamina + i32::from(row.ht) * row.stamina_per_con
            );
        }
    }
}
