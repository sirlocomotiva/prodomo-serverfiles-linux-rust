//! Bounded runtime character state and lifecycle management.

mod apply;
mod combat;
mod dice;
mod equip;
mod equipment;
mod grant;
mod ground;
mod inventory;
mod item_move;
mod items;
#[cfg(test)]
mod legacy_source;
mod manager;
mod model;
mod points;
mod potion;
mod quickslot;
mod shop;
mod state;

pub use apply::{
    ApplyArm, APPLY_ATTBONUS_BOSS, APPLY_ATTBONUS_HUMAN, APPLY_ATTBONUS_METIN,
    APPLY_ATTBONUS_MONSTER, APPLY_ATT_GRADE_BONUS, APPLY_CON, APPLY_COSTUME_ATTR_BONUS,
    APPLY_DEF_GRADE_BONUS, APPLY_ENCHANT_ELECT, APPLY_ENERGY, APPLY_EXTRACT_HP_PCT, APPLY_INT,
    APPLY_MAGIC_ATT_GRADE, APPLY_MAGIC_DEF_GRADE, APPLY_MAX_HP, APPLY_MAX_HP_PCT, APPLY_MAX_SP,
    APPLY_MAX_SP_PCT, APPLY_NONE, APPLY_SKILL, MAX_APPLY_NUM,
};
pub use combat::{
    attack_rating, melee_damage, normal_hit, pk_eligibility, AttackRatingInput, Damage,
    DamageOutcome, HitOutcome, MeleeDamageInput, NormalHitInput, PkActor, PkDenial, PkEligibility,
    PkMode, Vitality,
};
pub use dice::{number, Dice, Pcg32};
pub use equip::{
    find_equip_cell, is_equipable, is_female, use_item, wear_pos, worn_system_not_ported,
    CharacterLook, Gear, WornSystem,
};
pub use equipment::{
    accessory_socket_grade, is_set_item, item_applies, removal_applies, Equipment, PartChange,
    Worn, PARTS,
};
pub use grant::{grant, GrantRefused, Granted, GRANT_WINDOW};
pub use ground::{drop_item, pickup_item, DropAt, GroundItem, Picker, DROPPED_NOTICE};
pub use inventory::{
    character_cell_bound, custom_inventory_category_of, custom_inventory_position,
    inventory_page_by_pos, inventory_type_by_pos, inventory_type_of_cell,
    is_belt_inventory_position, is_custom_inventory_position, is_default_inventory_position,
    is_dragon_soul_equip_position, is_equip_position, is_switchbot_position,
    is_valid_item_position, placeholder, stored_window, CellBound, FLAT_RANGES,
    INVENTORY_PLACEHOLDER, NPOS,
};
pub use item_move::{
    move_item, GroundRecord, ItemChange, ItemRecord, MoveDone, MoveFacts, MoveKind, MoveRecord,
    MoveRefused, MoveRequest, MoveRules, Unported, AUTO_FIND_CELL,
};
pub use items::{
    belt_cell_is_available, CharacterItems, CountRefused, Lookup, Rejected, ATTR67_SLOTS,
};
pub use manager::{CharacterManager, CharacterManagerError, UpdateReport};
pub use model::{Character, CharacterKind};
pub use points::{
    apply_is_ported, race_to_job, sungma_will, PassiveBonuses, PointChangeRefused, PointRecord,
    Points, PointsRow, SungmaWill, IMMUNE_FALL, IMMUNE_SLOW, IMMUNE_STUN, SUNGMA_WILL_MAPS,
};
pub use potion::{is_recovering, update_recovery, SE_HPUP_RED, SE_SPUP_BLUE};
pub use quickslot::{
    add_from_client, chains_when_used_up, find_specify_item, sync_quickslots, Quickslot,
    QuickslotRecord, QuickslotSync, Quickslots, SyncTo, QUICKSLOT_MAX_NUM,
};
pub use shop::{
    buy_item, sale_price, sell_item, unported_creation, ShopRefused, UnportedCreation,
    GOLD_MAX_MAX, SALE_TAX_NOTICE, WORN_NOTICE,
};
pub use state::{Activity, CoreState, Posture};
