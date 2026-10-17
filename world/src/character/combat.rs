mod melee;
mod mob_melee;
mod pk;

pub use melee::{
    calc_melee_damage, melee_reach, mob_attack_range, strike_npc, HitEvent, Strike, Striker,
    Struck, DAMAGE_CRITICAL, DAMAGE_NORMAL, DAMAGE_PENETRATE, FLY_HP_SMALL, FLY_SP_SMALL,
    SE_CRITICAL, SE_PENETRATE,
};
pub use mob_melee::{mob_reach, pc_hp_pct, strike_pc, MobStriker, PcHitEnd, PcStrike, PcVictim};
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

fn source_rating(dexterity: i32, level: i32) -> i32 {
    dexterity
        .wrapping_mul(4)
        .wrapping_add(level.wrapping_mul(2))
        .wrapping_div(6)
        .min(90)
}
