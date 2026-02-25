mod pk;

pub use pk::{pk_eligibility, PkActor, PkDenial, PkEligibility, PkMode};

/// Inputs used by the legacy `CalcAttackRating` formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackRatingInput {
    /// Attacker dexterity after polymorph stat selection.
    pub attacker_dexterity: i32,
    /// Attacker level, used for both source ratings by the legacy code.
    pub attacker_level: i32,
    /// Victim dexterity after polymorph stat selection.
    pub victim_dexterity: i32,
    /// Whether the target evasion term is omitted.
    pub ignore_target_rating: bool,
}

/// Computes the legacy attack rating with C++ `int` and `float` operation order.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn attack_rating(input: AttackRatingInput) -> f32 {
    let attacker_source = source_rating(input.attacker_dexterity, input.attacker_level);
    let victim_source = source_rating(input.victim_dexterity, input.attacker_level);
    let attack = (attacker_source as f32 + 210.0_f32) / 300.0_f32;

    if input.ignore_target_rating {
        return attack;
    }

    let evasion = ((victim_source.wrapping_mul(2).wrapping_add(5)) as f32
        / victim_source.wrapping_add(95) as f32)
        * 3.0_f32
        / 10.0_f32;
    attack - evasion
}

/// Inputs for the bounded deterministic branch of `CalcMeleeDamage`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeleeDamageInput {
    /// Attacker level.
    pub attacker_level: i32,
    /// Current attack grade.
    pub attack_grade: i32,
    /// Injected inclusive weapon or natural-damage roll before legacy doubling.
    pub weapon_damage_roll: i32,
    /// Rating returned by [`attack_rating`].
    pub attack_rating: f32,
    /// Weapon refine value before legacy doubling.
    pub refine_bonus: i32,
    /// Party attacker role bonus.
    pub party_attack_bonus: i32,
    /// Sum of attack-grade percentage bonuses.
    pub attack_bonus_percent: i32,
    /// Victim defense grade.
    pub defense_grade: i32,
    /// Victim defense percentage bonus.
    pub defense_bonus_percent: i32,
    /// Whether victim defense is ignored.
    pub ignore_defense: bool,
    /// Injected `CalcBattleDamage` roll used when computed damage is below three.
    pub low_damage_roll: i32,
}

/// Computes deterministic melee damage using the C++ integer/float operation order.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
#[must_use]
pub fn melee_damage(input: MeleeDamageInput) -> i32 {
    let level_twice = input.attacker_level.wrapping_mul(2);
    let rolled_damage = input.weapon_damage_roll.wrapping_mul(2);
    let mut attack = input
        .attack_grade
        .wrapping_add(rolled_damage)
        .wrapping_sub(level_twice);
    attack = (attack as f32 * input.attack_rating) as i32;
    attack = attack
        .wrapping_add(level_twice)
        .wrapping_add(input.refine_bonus.wrapping_mul(2))
        .wrapping_add(input.party_attack_bonus);
    attack = attack
        .wrapping_mul(100_i32.wrapping_add(input.attack_bonus_percent))
        .wrapping_div(100);

    let defense = if input.ignore_defense {
        0
    } else {
        input
            .defense_grade
            .wrapping_mul(100_i32.wrapping_add(input.defense_bonus_percent))
            .wrapping_div(100)
    };
    let damage = attack.wrapping_sub(defense).max(0);
    if damage < 3 {
        input.low_damage_roll
    } else {
        damage
    }
}

/// Current and maximum hit points owned by a Character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vitality {
    current: u32,
    maximum: u32,
}

impl Vitality {
    /// Creates vitality, clamping current hit points to the maximum.
    #[must_use]
    pub const fn new(current: u32, maximum: u32) -> Self {
        Self {
            current: if current > maximum { maximum } else { current },
            maximum,
        }
    }

    /// Returns current hit points.
    #[must_use]
    pub const fn current(self) -> u32 {
        self.current
    }

    /// Returns maximum hit points.
    #[must_use]
    pub const fn maximum(self) -> u32 {
        self.maximum
    }

    pub(crate) const fn take(self, damage: Damage) -> (Self, DamageOutcome) {
        let applied = if damage.amount > self.current {
            self.current
        } else {
            damage.amount
        };
        let remaining = self.current - applied;
        let next = Self {
            current: remaining,
            maximum: self.maximum,
        };
        if remaining == 0 {
            (
                next,
                DamageOutcome::Killed {
                    overkill: damage.amount - applied,
                },
            )
        } else {
            (next, DamageOutcome::Damaged { remaining })
        }
    }
}

/// A nonnegative amount of hit-point damage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Damage {
    amount: u32,
}

impl Damage {
    /// Creates a damage value.
    #[must_use]
    pub const fn new(amount: u32) -> Self {
        Self { amount }
    }
}

/// Result of applying typed damage to a Character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageOutcome {
    /// Damage was applied without killing the target.
    Damaged {
        /// Hit points remaining after damage.
        remaining: u32,
    },
    /// Damage made an alive target dead.
    Killed {
        /// Damage beyond the target's prior hit points.
        overkill: u32,
    },
    /// The target was already in dead posture.
    AlreadyDead,
}

/// Deterministic result of the normal-hit block check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitOutcome {
    /// The inclusive block roll succeeded.
    Blocked,
    /// The attack proceeds to damage application.
    Landed,
}

/// Inputs for the inclusive normal-hit block roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalHitInput {
    /// Victim block chance as stored by `POINT_BLOCK`.
    pub block_chance: i32,
    /// Injected inclusive roll in the legacy one-through-one-hundred range.
    pub roll: u8,
}

/// Resolves the C++ `number(1, 100) <= POINT_BLOCK` comparison.
#[must_use]
pub const fn normal_hit(input: NormalHitInput) -> HitOutcome {
    if input.block_chance != 0 && (input.roll as i32) <= input.block_chance {
        HitOutcome::Blocked
    } else {
        HitOutcome::Landed
    }
}

fn source_rating(dexterity: i32, level: i32) -> i32 {
    dexterity
        .wrapping_mul(4)
        .wrapping_add(level.wrapping_mul(2))
        .wrapping_div(6)
        .min(90)
}
