//! Bounded runtime character state and lifecycle management.

mod combat;
mod grant;
mod inventory;
mod item_move;
mod items;
mod manager;
mod model;
mod state;

pub use combat::{
    attack_rating, melee_damage, normal_hit, pk_eligibility, AttackRatingInput, Damage,
    DamageOutcome, HitOutcome, MeleeDamageInput, NormalHitInput, PkActor, PkDenial, PkEligibility,
    PkMode, Vitality,
};
pub use grant::{grant, GrantRefused, Granted, GRANT_WINDOW};
pub use inventory::{
    character_cell_bound, custom_inventory_category_of, custom_inventory_position,
    inventory_page_by_pos, inventory_type_by_pos, inventory_type_of_cell,
    is_belt_inventory_position, is_custom_inventory_position, is_default_inventory_position,
    is_dragon_soul_equip_position, is_equip_position, is_switchbot_position,
    is_valid_item_position, placeholder, stored_window, CellBound, FLAT_RANGES,
    INVENTORY_PLACEHOLDER, NPOS,
};
pub use item_move::{
    move_item, ItemChange, ItemRecord, MoveDone, MoveFacts, MoveKind, MoveRefused, MoveRequest,
    MoveRules, Unported, AUTO_FIND_CELL,
};
pub use items::{
    belt_cell_is_available, CharacterItems, CountRefused, Lookup, Rejected, ATTR67_SLOTS,
};
pub use manager::{CharacterManager, CharacterManagerError, UpdateReport};
pub use model::{Character, CharacterKind};
pub use state::{Activity, CoreState, Posture};
