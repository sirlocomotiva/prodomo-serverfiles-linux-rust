//! Bounded runtime character state and lifecycle management.

mod combat;
mod manager;
mod model;
mod state;

pub use combat::{
    attack_rating, melee_damage, normal_hit, pk_eligibility, AttackRatingInput, Damage,
    DamageOutcome, HitOutcome, MeleeDamageInput, NormalHitInput, PkActor, PkDenial, PkEligibility,
    PkMode, Vitality,
};
pub use manager::{CharacterManager, CharacterManagerError, UpdateReport};
pub use model::{Character, CharacterKind};
pub use state::{Activity, CoreState, Posture};
