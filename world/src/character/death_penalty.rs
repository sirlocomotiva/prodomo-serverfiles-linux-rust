//! The experience a death costs, as `DeathPenalty` computes it (`char_battle.cpp:378-386`).
//!
//! The rest of the penalty (the flag, the town arm, the level and the roll that spare a
//! character, and the record) is the world's: see `prodomo::game_state::death`.

/// `aiExpLossPercents` (`constants.cpp:885-899`): the share of the next level's cost a death
/// takes, by level, for levels 1 to 120.
const EXP_LOSS_PERCENTS: [i64; 121] = [
    0, //
    5, 5, 5, 5, 5, 5, 5, 5, 5, 4, // 1 - 10
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, // 11 - 20
    4, 4, 4, 4, 4, 4, 4, 3, 3, 3, // 21 - 30
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, // 31 - 40
    3, 3, 3, 3, 2, 2, 2, 2, 2, 2, // 41 - 50
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // 51 - 60
    2, 2, 1, 1, 1, 1, 1, 1, 1, 1, // 61 - 70
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 71 - 80
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 81 - 90
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 91 - 100
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 101 - 110
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 111 - 120
];

/// The most experience one death takes (`char_battle.cpp:380`).
const MAX_LOSS: i64 = 800_000;

/// `__GetExpLossPerc` (`char_battle.cpp:77-82`): level 0 and every level past the table lose 1
/// per cent.
#[must_use]
pub fn exp_loss_percent(level: u8) -> i64 {
    EXP_LOSS_PERCENTS
        .get(usize::from(level))
        .copied()
        .filter(|_| level != 0)
        .unwrap_or(1)
}

/// The loss of a death outside town: the level's share of `next_exp`, at most 800 000, and
/// half of that while the Teardrop of Godness (70012) is worn.
#[must_use]
pub fn death_penalty_loss(level: u8, next_exp: i64, teardrop: bool) -> i64 {
    let loss = (next_exp * exp_loss_percent(level) / 100).min(MAX_LOSS);
    if teardrop {
        loss / 2
    } else {
        loss
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loss_percent_follows_the_legacy_table_at_each_band_edge() {
        for (level, percent) in [
            (0, 1),
            (1, 5),
            (9, 5),
            (10, 4),
            (27, 4),
            (28, 3),
            (44, 3),
            (45, 2),
            (62, 2),
            (63, 1),
            (120, 1),
            (121, 1),
            (250, 1),
        ] {
            assert_eq!(exp_loss_percent(level), percent, "level {level}");
        }
    }

    #[test]
    fn every_level_from_0_to_255_loses_the_percent_of_its_legacy_band() {
        // The bands of `aiExpLossPercents` (`constants.cpp:885-899`); level 0 and the levels
        // past `PLAYER_EXP_TABLE_MAX` lose 1 per cent (`char_battle.cpp:79-80`).
        for level in 0..=u8::MAX {
            let percent = match level {
                1..=9 => 5,
                10..=27 => 4,
                28..=44 => 3,
                45..=62 => 2,
                0 | 63..=u8::MAX => 1,
            };
            assert_eq!(exp_loss_percent(level), percent, "level {level}");
        }
    }

    #[test]
    fn a_loss_is_capped_at_800000_and_halved_by_the_teardrop() {
        assert_eq!(death_penalty_loss(10, 33_000, false), 1_320);
        assert_eq!(death_penalty_loss(10, 33_000, true), 660);
        // The division comes after the product: 4 % of 1 299 is 51, not 4 * 12.
        assert_eq!(death_penalty_loss(10, 1_299, false), 51);
        assert_eq!(death_penalty_loss(10, 1_299, true), 25);
        assert_eq!(death_penalty_loss(10, 20_000_025, false), 800_000);
        assert_eq!(death_penalty_loss(10, 20_000_025, true), 400_000);
        assert_eq!(death_penalty_loss(200, 2_500_000_000, false), 800_000);
        assert_eq!(death_penalty_loss(90, 79_999_999, false), 799_999);
    }

    #[test]
    fn the_teardrop_halves_the_capped_loss_not_the_full_share() {
        // Level 90's next level costs 676 130 000, whose 1 per cent is 6 761 300: capped to
        // 800 000 (`char_battle.cpp:380`), then halved (`:385-386`), it is 400 000.
        assert_eq!(death_penalty_loss(90, 676_130_000, false), 800_000);
        assert_eq!(death_penalty_loss(90, 676_130_000, true), 400_000);
    }
}
