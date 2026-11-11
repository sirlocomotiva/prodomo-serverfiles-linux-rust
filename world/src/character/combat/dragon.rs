//! `BlueDragon_Damage` (`G/BlueDragon.cpp:139-196`): the dragon-stone scaling of a
//! damage `dam` already rated, after the death-blow multiple and before the flag
//! (`G/char_battle.cpp:2298`).
//!
//! Three arms, in order, each scanning the four snapshot rows in order and taking
//! the first match:
//!
//! - the attacker is the monster 2493: the first `ATK_BONUS` row's stone is
//!   counted live on the attacker's map, and the damage grows by its percent
//!   times the count;
//! - the victim is the monster 2493: the first `DEF_BONUS` row's stone is counted
//!   live on the victim's map, and the damage shrinks by its percent times the
//!   count, at least 1;
//! - the victim is a stone and the attacker rides a mount: the row whose vnum is
//!   the victim's multiplies the damage by its `enemy_val` when the attacker's
//!   mount is its `enemy`.
//!
//! The counts are the caller's: legacy's `GetMonsterCountInMap` counts the live
//! stones of the vnum on the map. A default snapshot is the identity everywhere:
//! no vnum-0 non-player exists, no arm matches effect 0, and no mounted attacker
//! rides mount 0.

use common::dragon::{
    DragonStone, BLUE_DRAGON_VNUM, DRAGON_STONE_ATK, DRAGON_STONE_DEF, DRAGON_STONE_ROWS,
};

/// The facts `BlueDragon_Damage` reads of the attacker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragonAttacker {
    /// Whether it is the monster 2493 (`IsMonster` and the vnum).
    pub is_dragon: bool,
    /// `GetMountVnum`: 0 unmounted.
    pub mount_vnum: u32,
}

/// The facts `BlueDragon_Damage` reads of the victim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragonVictim {
    /// Whether it is the monster 2493 (`IsMonster` and the vnum).
    pub is_dragon: bool,
    /// Its mob vnum.
    pub vnum: u32,
    /// Whether it is a stone (`IsStone`).
    pub is_stone: bool,
}

/// `BlueDragon_Damage` of `dam`: the three arms above. `counts` holds the live stones
/// of each row's vnum on the map, as `GetMonsterCountInMap` counts them (stones
/// only — `FCountSpecifiedMonster` requires `IsStone` — and the dead are gone from
/// the sectree). Legacy counts on the attacker's map for the attacker arm and the
/// victim's for the victim arm; combat never crosses maps, so the caller counts
/// once on the map the arms that can fire share.
#[must_use]
pub fn blue_dragon_damage(
    stones: &[DragonStone; DRAGON_STONE_ROWS],
    attacker: DragonAttacker,
    victim: DragonVictim,
    counts: [usize; DRAGON_STONE_ROWS],
    dam: i32,
) -> i32 {
    let mut dam = i64::from(dam);
    if attacker.is_dragon {
        for index in 1..=DRAGON_STONE_ROWS {
            if stone_effect(stones, index) == DRAGON_STONE_ATK {
                let val = stone_val(stones, index);
                let live = i64::from(u32::try_from(counts[index - 1]).unwrap_or(u32::MAX));
                dam += (dam * val.saturating_mul(live)) / 100;
                break;
            }
        }
    }
    if victim.is_dragon {
        for index in 1..=DRAGON_STONE_ROWS {
            if stone_effect(stones, index) == DRAGON_STONE_DEF {
                let val = stone_val(stones, index);
                let live = i64::from(u32::try_from(counts[index - 1]).unwrap_or(u32::MAX));
                dam -= (dam * val.saturating_mul(live)) / 100;
                if dam <= 0 {
                    dam = 1;
                }
                break;
            }
        }
    }
    if victim.is_stone && attacker.mount_vnum != 0 {
        for index in 1..=DRAGON_STONE_ROWS {
            if stone_vnum(stones, index) == victim.vnum
                && stone_enemy(stones, index) == attacker.mount_vnum
            {
                // `dam *= val`: the product of an `int` hit by the row's `int`
                // `enemy_val` fits in 64 bits, and the return keeps its low 32.
                dam *= i64::from(stone_enemy_val(stones, index));
                break;
            }
        }
    }
    // Legacy's `dam` is `int` on the 32-bit target, so the return keeps the low
    // 32 bits (the product of an `int` hit by a `u32` `enemy_val` cannot set the
    // high ones from valid points).
    let low = u32::try_from(dam & 0xffff_ffff).unwrap_or(u32::MAX);
    i32::from_ne_bytes(low.to_ne_bytes())
}

/// Whether the vnum is the Blue Dragon's.
#[must_use]
pub const fn is_blue_dragon(vnum: u32) -> bool {
    vnum == BLUE_DRAGON_VNUM
}

fn stone_effect(stones: &[DragonStone; DRAGON_STONE_ROWS], index: usize) -> u32 {
    stones[index - 1].effect
}

fn stone_vnum(stones: &[DragonStone; DRAGON_STONE_ROWS], index: usize) -> u32 {
    stones[index - 1].vnum
}

fn stone_val(stones: &[DragonStone; DRAGON_STONE_ROWS], index: usize) -> i64 {
    i64::from(stones[index - 1].val)
}

fn stone_enemy(stones: &[DragonStone; DRAGON_STONE_ROWS], index: usize) -> u32 {
    stones[index - 1].enemy
}

fn stone_enemy_val(stones: &[DragonStone; DRAGON_STONE_ROWS], index: usize) -> i32 {
    i32::from_ne_bytes(stones[index - 1].enemy_val.to_ne_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's stone rows (`BlueDragon.lua:129-153`).
    fn owner_stones() -> [DragonStone; DRAGON_STONE_ROWS] {
        [
            DragonStone {
                vnum: 8031,
                effect: 1,
                val: 25,
                enemy: 20110,
                enemy_val: 3,
            },
            DragonStone {
                vnum: 8032,
                effect: 2,
                val: 25,
                enemy: 20111,
                enemy_val: 3,
            },
            DragonStone {
                vnum: 8033,
                effect: 3,
                val: 20,
                enemy: 20112,
                enemy_val: 3,
            },
            DragonStone {
                vnum: 8034,
                effect: 4,
                val: 10,
                enemy: 20113,
                enemy_val: 3,
            },
        ]
    }

    const PLAIN_ATTACKER: DragonAttacker = DragonAttacker {
        is_dragon: false,
        mount_vnum: 0,
    };
    const PLAIN_VICTIM: DragonVictim = DragonVictim {
        is_dragon: false,
        vnum: 101,
        is_stone: false,
    };

    /// The three arms: the dragon's hit grows 25% per live 8032, a hit on it
    /// shrinks 25% per live 8031 with a floor of 1, and a feared mount triples a
    /// hit on its stone. Anything else is the identity.
    #[test]
    fn the_dragon_stones_scale_the_dragon_and_the_feared_mount() {
        let stones = owner_stones();
        let none = [0; DRAGON_STONE_ROWS];
        let two_attack = [0, 2, 0, 0];
        let dragon = DragonAttacker {
            is_dragon: true,
            mount_vnum: 0,
        };
        assert_eq!(
            blue_dragon_damage(&stones, dragon, PLAIN_VICTIM, two_attack, 100),
            150
        );
        assert_eq!(
            blue_dragon_damage(&stones, PLAIN_ATTACKER, PLAIN_VICTIM, two_attack, 100),
            100
        );
        let one_defence = [1, 0, 0, 0];
        let dragon_victim = DragonVictim {
            is_dragon: true,
            vnum: BLUE_DRAGON_VNUM,
            is_stone: false,
        };
        assert_eq!(
            blue_dragon_damage(&stones, PLAIN_ATTACKER, dragon_victim, one_defence, 100),
            75
        );
        let eight_defence = [8, 0, 0, 0];
        assert_eq!(
            blue_dragon_damage(&stones, PLAIN_ATTACKER, dragon_victim, eight_defence, 2),
            1
        );
        let stone_victim = DragonVictim {
            is_dragon: false,
            vnum: 8031,
            is_stone: true,
        };
        let feared = DragonAttacker {
            is_dragon: false,
            mount_vnum: 20110,
        };
        assert_eq!(
            blue_dragon_damage(&stones, feared, stone_victim, none, 100),
            300
        );
        let wrong_mount = DragonAttacker {
            is_dragon: false,
            mount_vnum: 20111,
        };
        assert_eq!(
            blue_dragon_damage(&stones, wrong_mount, stone_victim, none, 100),
            100
        );
        assert_eq!(
            blue_dragon_damage(&stones, PLAIN_ATTACKER, stone_victim, none, 100),
            100
        );
        assert_eq!(
            blue_dragon_damage(
                &[DragonStone::default(); DRAGON_STONE_ROWS],
                dragon,
                dragon_victim,
                two_attack,
                100
            ),
            100
        );
    }
}
