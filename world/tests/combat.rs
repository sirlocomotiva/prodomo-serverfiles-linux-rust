//! Source-parity tests for bounded Character combat rules.

use common::vid::Vid;
use world::character::{
    attack_rating, melee_damage, normal_hit, pk_eligibility, Activity, AttackRatingInput,
    Character, CharacterKind, CoreState, Damage, DamageOutcome, HitOutcome, MeleeDamageInput,
    NormalHitInput, PkActor, PkDenial, PkEligibility, PkMode, Posture, Vitality,
};

#[test]
fn combat_attack_rating_preserves_equal_stat_cpp_arithmetic() {
    let rating = attack_rating(AttackRatingInput {
        attacker_dexterity: 10,
        attacker_level: 10,
        victim_dexterity: 10,
        ignore_target_rating: false,
    });

    assert_eq!(rating.to_bits(), 0x3f29_7297);

    let low = attack_rating(AttackRatingInput {
        attacker_dexterity: 0,
        attacker_level: 0,
        victim_dexterity: 0,
        ignore_target_rating: true,
    });
    let high = attack_rating(AttackRatingInput {
        attacker_dexterity: 1_000,
        attacker_level: 1_000,
        victim_dexterity: 1_000,
        ignore_target_rating: true,
    });
    assert_eq!(low.to_bits(), 0x3f33_3333);
    assert_eq!(high.to_bits(), 1.0_f32.to_bits());
}

#[test]
fn combat_pk_policy_is_explicit_for_invalid_self_and_mode_outcomes() {
    let attacker = pk_actor(1, PkMode::Free);
    assert_eq!(
        pk_eligibility(attacker, None, false),
        PkEligibility::Denied(PkDenial::InvalidTarget)
    );
    assert_eq!(
        pk_eligibility(attacker, Some(attacker), false),
        PkEligibility::Denied(PkDenial::SelfTarget)
    );

    let target = pk_actor(2, PkMode::Peace);
    assert_eq!(
        pk_eligibility(attacker, Some(target), false),
        PkEligibility::Allowed {
            enter_killer_mode: true
        }
    );

    let peaceful = pk_actor(3, PkMode::Peace);
    assert_eq!(
        pk_eligibility(peaceful, Some(target), false),
        PkEligibility::Denied(PkDenial::PkMode)
    );

    let mut party_target = target;
    let mut party_attacker = attacker;
    party_target.party_id = Some(7);
    party_attacker.party_id = Some(7);
    assert_eq!(
        pk_eligibility(party_attacker, Some(party_target), false),
        PkEligibility::Denied(PkDenial::SameParty)
    );

    let mut foreign_target = target;
    foreign_target.empire = 2;
    assert_eq!(
        pk_eligibility(peaceful, Some(foreign_target), false),
        PkEligibility::Allowed {
            enter_killer_mode: false
        }
    );
}

#[test]
fn combat_manual_parity_scenario_hit_nonlethal_lethal_and_pk() {
    let rating = attack_rating(AttackRatingInput {
        attacker_dexterity: 10,
        attacker_level: 10,
        victim_dexterity: 10,
        ignore_target_rating: false,
    });
    assert_eq!(rating.to_bits(), 0x3f29_7297);
    assert_eq!(
        normal_hit(NormalHitInput {
            block_chance: 10,
            roll: 11,
        }),
        HitOutcome::Landed
    );

    let damage = melee_damage(MeleeDamageInput {
        attacker_level: 10,
        attack_grade: 40,
        weapon_damage_roll: 10,
        attack_rating: 1.0,
        refine_bonus: 0,
        party_attack_bonus: 0,
        attack_bonus_percent: 0,
        defense_grade: 0,
        defense_bonus_percent: 0,
        ignore_defense: false,
        low_damage_roll: 1,
    });
    assert_eq!(damage, 60);

    let mut victim = Character::new(Vid::new(30));
    victim.set_activity(Activity::Fishing);
    victim.request_state(CoreState::Battle);
    victim.set_vitality(Vitality::new(100, 100));
    assert_eq!(
        victim.apply_damage(Damage::new(40)),
        DamageOutcome::Damaged { remaining: 60 }
    );
    assert_eq!(
        victim.apply_damage(Damage::new(
            u32::try_from(damage).expect("damage must be nonnegative")
        )),
        DamageOutcome::Killed { overkill: 0 }
    );
    assert!(!victim.update(0));
    assert_eq!(victim.pending_state(), Some(CoreState::Battle));
    assert_eq!(victim.activity(), Activity::Fishing);

    assert_eq!(
        pk_eligibility(
            pk_actor(1, PkMode::Free),
            Some(pk_actor(2, PkMode::Peace)),
            false
        ),
        PkEligibility::Allowed {
            enter_killer_mode: true
        }
    );
}

fn pk_actor(id: u32, mode: PkMode) -> PkActor {
    PkActor {
        id,
        kind: CharacterKind::Player,
        posture: Posture::Standing,
        observer: false,
        ban_pk: false,
        empire: 1,
        map_empire: 0,
        mode,
        alignment: 0,
        killer_mode: false,
        party_id: None,
        guild_id: None,
        consensual_fight: false,
    }
}

#[test]
fn combat_damage_transitions_cover_hit_and_lethal_boundaries() {
    assert_eq!(
        normal_hit(NormalHitInput {
            block_chance: 25,
            roll: 25,
        }),
        HitOutcome::Blocked
    );
    assert_eq!(
        normal_hit(NormalHitInput {
            block_chance: 25,
            roll: 26,
        }),
        HitOutcome::Landed
    );

    let mut victim = Character::new(Vid::new(20));
    victim.set_vitality(Vitality::new(100, 100));
    assert_eq!(
        victim.apply_damage(Damage::new(40)),
        DamageOutcome::Damaged { remaining: 60 }
    );
    assert_eq!(
        victim.apply_damage(Damage::new(60)),
        DamageOutcome::Killed { overkill: 0 }
    );
    assert_eq!(victim.posture(), Posture::Dead);

    let mut overkill = Character::new(Vid::new(21));
    overkill.set_vitality(Vitality::new(10, 100));
    assert_eq!(
        overkill.apply_damage(Damage::new(25)),
        DamageOutcome::Killed { overkill: 15 }
    );

    let mut zero = Character::new(Vid::new(22));
    zero.set_vitality(Vitality::new(0, 100));
    assert_eq!(
        zero.apply_damage(Damage::new(0)),
        DamageOutcome::Killed { overkill: 0 }
    );
}

#[test]
fn combat_melee_damage_preserves_cpp_order_and_clamps() {
    let fixtures = [
        (40, 0, 0, 60),
        (40, 200, 0, 1),
        (30_000_000, 0, 100, 17_050_367),
    ];

    for (attack_grade, defense_grade, attack_bonus_percent, expected) in fixtures {
        let damage = melee_damage(MeleeDamageInput {
            attacker_level: 10,
            attack_grade,
            weapon_damage_roll: 10,
            attack_rating: 1.0,
            refine_bonus: 0,
            party_attack_bonus: 0,
            attack_bonus_percent,
            defense_grade,
            defense_bonus_percent: 0,
            ignore_defense: false,
            low_damage_roll: 1,
        });
        assert_eq!(damage, expected);
    }
}
