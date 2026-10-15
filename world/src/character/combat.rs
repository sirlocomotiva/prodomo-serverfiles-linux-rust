mod melee;
mod pk;

pub use melee::{
    calc_melee_damage, melee_reach, mob_attack_range, strike_npc, HitEvent, Strike, Striker,
    Struck, DAMAGE_CRITICAL, DAMAGE_NORMAL, DAMAGE_PENETRATE, FLY_HP_SMALL, FLY_SP_SMALL,
    SE_CRITICAL, SE_PENETRATE,
};
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

fn source_rating(dexterity: i32, level: i32) -> i32 {
    dexterity
        .wrapping_mul(4)
        .wrapping_add(level.wrapping_mul(2))
        .wrapping_div(6)
        .min(90)
}
