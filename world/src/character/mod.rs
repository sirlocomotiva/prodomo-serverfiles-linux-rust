//! Bounded runtime character state and lifecycle management.

mod apply;
mod combat;
mod death_penalty;
mod dice;
mod drop_item;
mod equip;
mod equipment;
mod grant;
mod ground;
mod inventory;
mod item_move;
mod items;
#[cfg(test)]
pub(crate) mod legacy_source;
mod manager;
mod model;
mod points;
mod potion;
mod quickslot;
mod recovery;
mod safebox;
mod shop;
mod state;
mod trade;

pub use apply::{
    ApplyArm, APPLY_ATTBONUS_BOSS, APPLY_ATTBONUS_HUMAN, APPLY_ATTBONUS_METIN,
    APPLY_ATTBONUS_MONSTER, APPLY_ATT_GRADE_BONUS, APPLY_CON, APPLY_COSTUME_ATTR_BONUS,
    APPLY_DEF_GRADE_BONUS, APPLY_ENCHANT_ELECT, APPLY_ENERGY, APPLY_EXTRACT_HP_PCT, APPLY_INT,
    APPLY_MAGIC_ATT_GRADE, APPLY_MAGIC_DEF_GRADE, APPLY_MAX_HP, APPLY_MAX_HP_PCT, APPLY_MAX_SP,
    APPLY_MAX_SP_PCT, APPLY_NONE, APPLY_SKILL, MAX_APPLY_NUM,
};
pub use combat::{
    attack_rating, battle_damage_floor, calc_arrow_damage, calc_arrow_skill_atk, calc_melee_damage,
    calc_melee_damage_on_player, calc_melee_skill_atk, damage_magic, damage_melee,
    damage_normal_range, damage_range, damage_special, melee_reach, mob_att_bonus_self,
    mob_attack_range, mob_calc_attack, mob_reach, normal_attack_affect,
    normal_attack_affect_on_player, pc_hp_pct, pc_poison_rate, pk_eligibility, reflect_damage,
    shoot_pc, skill_damage_pc, skill_magic_attr, skill_melee_attr, skill_range_attr, strike_npc,
    strike_npc_by_npc, strike_pc, strike_pc_by_pc, AttackAffect, AttackRatingInput, HitEvent,
    MeleeVictim, MobStriker, NpcVictim, PcHitEnd, PcStrike, PcVictim, PkActor, PkDenial,
    PkEligibility, PkMode, PlayerVictim, PvAction, PvAttacker, PvEnd, PvKind, PvStrike, PvVictim,
    Strike, Striker, Struck, DAMAGE_CRITICAL, DAMAGE_DODGE, DAMAGE_NORMAL, DAMAGE_PENETRATE,
    DAMAGE_POISON, FLY_EXP, FLY_HP_SMALL, FLY_SP_SMALL, IMMUNE_REFLECT, SE_CRITICAL,
    SE_EFFECT_HEALER, SE_PENETRATE,
};
pub use death_penalty::{death_penalty_loss, exp_loss_percent};
pub use dice::{number, Dice, Pcg32};
pub use drop_item::{
    attribute_set_index, gauss_random, make_drop_item, unported_drop, DropAsk, DropMade,
    DropRefused, Gauss, UnportedDrop,
};
pub use equip::{
    find_equip_cell, is_equipable, is_female, is_skill_book, look_of, recompute_points,
    unequip_points, use_item, wear_of, wear_pos, worn_system_not_ported, CharacterLook, Gear,
    WornSystem,
};
pub use equipment::{
    accessory_socket_grade, is_set_item, item_applies, removal_applies, Equipment, PartChange,
    Worn, PARTS,
};
pub use grant::{grant, GrantRefused, Granted, GRANT_WINDOW};
pub use ground::{
    drop_gold, drop_item, drop_penalty_item, pickup_gold, pickup_item, DropAt, GroundItem, Picker,
    DROPPED_NOTICE, GOLD_DROPPED_NOTICE,
};
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
    apply_is_ported, quarter_cost, race_to_job, sungma_will, ExpLimits, GainOutcome,
    PassiveBonuses, PointChangeRefused, PointRecord, Points, PointsRow, SungmaWill, IMMUNE_FALL,
    IMMUNE_SLOW, IMMUNE_STUN, SUNGMA_WILL_MAPS,
};
pub use potion::{is_recovering, update_recovery, SE_HPUP_RED, SE_SPUP_BLUE};
pub use quickslot::{
    add_from_client, chains_when_used_up, find_specify_item, sync_quickslots, Quickslot,
    QuickslotRecord, QuickslotSync, Quickslots, SyncTo, QUICKSLOT_MAX_NUM,
};
pub use recovery::{distribute_sp, hp_recovery, RECOVERY_PERCENTS};
pub use safebox::{
    checkin, checkout, move_stored, Safebox, SafeboxRefused, StoreChange, StoreMove, StoreRecord,
    IRREMOVABLE_NOTICE, MALL_ROWS, SAFEBOX_PAGE_SIZE, SAFEBOX_ROWS, SAFEBOX_WIDTH,
    UNIQUE_ITEM_SAFEBOX_EXPAND, WRONG_BANK_NOTICE,
};
pub use shop::{
    buy_item, sale_price, sell_item, unported_creation, ShopRefused, UnportedCreation,
    GOLD_MAX_MAX, SALE_TAX_NOTICE, WORN_NOTICE,
};
pub use state::{Activity, CoreState, Posture};
pub use trade::{
    settle, Accepted, Offered, Said, Settled, Side, Trade, TradeRecord, Trader, Unsettled,
    COMPLETED_NOTICE, EXCHANGE_ITEM_MAX_NUM, EXCHANGE_MAX_DISTANCE, FULL_NOTICE,
    GIVE_REFUSED_NOTICE, OTHER_TRANSACTION_NOTICE, OUT_OF_PLACE_NOTICE, PARTNER_BUSY_NOTICE,
    PARTNER_FULL_NOTICE, PARTNER_OUT_OF_PLACE_NOTICE, PARTNER_QUESTING_NOTICE, QUESTING_NOTICE,
    YANG_LIMIT_NOTICE,
};
