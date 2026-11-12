//! Bounded runtime character state and lifecycle management.

mod apply;
mod auto_give_new;
mod bait;
mod blend;
mod campfire;
mod combat;
mod death_penalty;
mod dice;
mod drop_item;
mod equip;
mod equipment;
mod give;
mod grant;
mod ground;
mod horse;
mod inventory;
mod item_move;
mod items;
#[cfg(test)]
pub(crate) mod legacy_source;
mod manager;
mod model;
mod points;
mod polymorph;
mod portal;
mod potion;
mod premium;
mod purge;
mod quickslot;
mod recovery;
mod safebox;
mod shop;
mod smith_refine;
mod sort_inventory;
mod specify_item;
mod state;
mod toggle;
mod tool_refine;
mod trade;
mod use_to_item;

pub use apply::{
    point_type, ApplyArm, APPLY_ATTBONUS_BOSS, APPLY_ATTBONUS_HUMAN, APPLY_ATTBONUS_METIN,
    APPLY_ATTBONUS_MONSTER, APPLY_ATT_GRADE_BONUS, APPLY_CON, APPLY_COSTUME_ATTR_BONUS,
    APPLY_DEF_GRADE_BONUS, APPLY_ENCHANT_ELECT, APPLY_ENERGY, APPLY_EXTRACT_HP_PCT, APPLY_INT,
    APPLY_MAGIC_ATT_GRADE, APPLY_MAGIC_DEF_GRADE, APPLY_MAX_HP, APPLY_MAX_HP_PCT, APPLY_MAX_SP,
    APPLY_MAX_SP_PCT, APPLY_NONE, APPLY_SKILL, MAX_APPLY_NUM,
};
pub use auto_give_new::{auto_give_item_new, dragon_soul_buy_cell, GivenNew};
pub use bait::{is_bait, use_bait, Baited, ROD_BAIT_SOCKET, WEAPON_CELL};
pub use blend::spend_blend;
pub use campfire::{
    is_campfire, use_campfire, CampfireGround, Fired, CAMPFIRE_BARRED_MAP, CAMPFIRE_COOLDOWN_SECS,
};
pub use combat::{
    attack_rating, battle_damage_floor, calc_arrow_damage, calc_arrow_skill_atk, calc_melee_damage,
    calc_melee_damage_on_player, calc_melee_skill_atk, damage_magic, damage_melee,
    damage_normal_range, damage_range, damage_special, melee_reach, mob_att_bonus_self,
    mob_attack_range, mob_calc_attack, mob_calc_attack_npc, mob_reach, normal_attack_affect,
    normal_attack_affect_on_player, pc_hp_pct, pc_poison_rate, pk_eligibility, reflect_damage,
    shoot_npc, shoot_pc, skill_damage_npc, skill_damage_pc, skill_magic_attr, skill_melee_attr,
    skill_range_attr, special_on_mob, strike_npc, strike_npc_by_monster, strike_npc_by_npc,
    strike_pc, strike_pc_by_pc, AttackAffect, AttackRatingInput, HitEvent, MeleeVictim, MobStriker,
    NpcVictim, PcHitEnd, PcStrike, PcVictim, PkActor, PkDenial, PkEligibility, PkMode,
    PlayerVictim, PvAction, PvAttacker, PvEnd, PvKind, PvStrike, PvVictim, Strike, Striker, Struck,
    DAMAGE_CRITICAL, DAMAGE_DODGE, DAMAGE_NORMAL, DAMAGE_PENETRATE, DAMAGE_POISON,
    FLY_CONQUEROR_EXP, FLY_EXP, FLY_HP_SMALL, FLY_SP_SMALL, IMMUNE_REFLECT, SE_CRITICAL,
    SE_EFFECT_HEALER, SE_PENETRATE,
};
pub use death_penalty::{death_penalty_loss, exp_loss_percent};
pub use dice::{number, Dice, FixedDraw, Pcg32};
pub use drop_item::{
    add_rare_attribute, add_rolled_attribute, attribute_set_index, change_attribute,
    change_attribute_on, change_rare_attribute, gauss_random, golden_change_levels, make_drop_item,
    sig_vnum_of, stamp_created_sockets, unported_drop, DropAsk, DropMade, DropRefused, Gauss,
    UnportedDrop, ITEM_FLAG_MAKECOUNT,
};
pub use equip::{
    consume_giftbox, find_equip_cell, is_equipable, is_female, is_ride_item, is_skill_book,
    look_of, recompute_points, unequip_points, use_checks, use_item, use_source, wear_full_set,
    wear_of, wear_pos, worn_system_not_ported, CharacterLook, Gear, WornSystem, PICKUP_AFFECT_ITEM,
};
pub use equipment::{
    accessory_socket_grade, counted_attrs, energy_worn_attrs, is_accessory_for_socket,
    is_energy_wear, is_set_item, item_applies, removal_applies, Equipment, PartChange, Worn, PARTS,
};
pub use give::{
    can_receive, unported_of, GiveAnswer, GiveUnported, ALCHEMIST_MOB, BLACKSMITH2_MOB,
    BLACKSMITH_ACCESSORY_MOB, BLACKSMITH_ARMOR_MOB, BLACKSMITH_MOB, BLACKSMITH_WEAPON_MOB,
    CAMPFIRE_MOB, DEVILTOWER_BLACKSMITH_ACCESSORY_MOB, DEVILTOWER_BLACKSMITH_ARMOR_MOB,
    DEVILTOWER_BLACKSMITH_WEAPON_MOB, FISHER_MOB, GIVE_DISTANCE, HORSE_FEED_LINE,
    HORSE_REVIVE_LINE, HORSE_VNUMS, ITEM_HORSE_FOOD_1, ITEM_HORSE_FOOD_2, ITEM_HORSE_FOOD_3,
    ITEM_REVIVE_HORSE_1, ITEM_REVIVE_HORSE_2, ITEM_REVIVE_HORSE_3, MELEY_STATUE_VNUM,
};
pub use grant::{grant, GrantRefused, Granted, GRANT_WINDOW};
pub use ground::{
    auto_give_item, destroy_item, drop_gold, drop_item, drop_penalty_item, pickup_gold,
    pickup_item, AutoGiven, DropAt, GroundItem, Picker, DROPPED_NOTICE, GOLD_DROPPED_NOTICE,
};
pub use horse::{
    Horse, HorseInfo, HorseInfoLine, HorseLine, HorseStat, HorseTimer, HORSE_HEALTH_DROP_INTERVAL,
    HORSE_MAX_LEVEL, HORSE_STAMINA_CONSUME_INTERVAL, HORSE_STAMINA_REGEN_INTERVAL, HORSE_STATS,
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
    MoveRefused, MoveRequest, MoveRules, Unported, AUTO_FIND_CELL, MOVE_BELT_ELIGIBLE,
    MOVE_CHAINS_QUICKSLOTS, MOVE_DRAGON_SOUL, MOVE_SWITCHBOT_ITEM,
};
pub use items::{
    belt_cell_is_available, AttributeRefused, CharacterItems, CountRefused, Lookup, Rejected,
    SocketRefused, ATTR67_SLOTS,
};
pub use manager::{CharacterManager, CharacterManagerError, UpdateReport};
pub use model::{Character, CharacterKind};
pub use points::{
    apply_is_ported, quarter_cost, race_to_job, sungma_will, ConquerorEvent, ExpLimits,
    GainOutcome, PassiveBonuses, PointChangeRefused, PointRecord, Points, PointsRow, SungmaWill,
    IMMUNE_FALL, IMMUNE_SLOW, IMMUNE_STUN, SUNGMA_WILL_MAPS,
};
pub use polymorph::{
    polymorph_power, POLYMORPH_MAX_LEVEL, POLYMORPH_POWER_BY_LEVEL, SKILL_POLYMORPH,
};
pub use portal::{
    calculate_consume, empire_of_map_index, is_portal, use_portal, PortalGround, Portaled,
    BARRED_MAPS, MEMORY_PORTAL, PVP_ALLOWED, TOWN_PORTAL, WARP_MIN_LIFE_PERCENT,
    WARP_NEED_LIFE_PERCENT,
};
pub use potion::{
    is_recovering, toggle_auto_potion, update_recovery, AutoBranch, SE_HPUP_RED, SE_SPUP_BLUE,
};
pub use premium::{is_premium, use_premium, PREMIUM_VNUMS};
pub use purge::{
    book_give, purge_all, slow_give, BOOK_SOCKET, BOOK_VNUM, DRAGON_SOUL_SLOTS, SLOW_COUNT,
    SLOW_SOCKET, SLOW_VNUM,
};
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
    buy_item, sale_price, sell_item, unported_creation, ShopRefused, UnportedCreation, GAYA_MAX,
    GOLD_MAX_MAX, SALE_TAX_NOTICE, WORN_NOTICE,
};
pub use smith_refine::{refined_item, swap_refined};
pub use sort_inventory::{sort_inventory, Sorted, SORT_KEPT_WHOLE, SUMMON_ITEMS};
pub use specify_item::{
    count_specify_item, count_specify_type_item, force_worn_attribute, refine_level,
    remove_quest_item, remove_specify_item, remove_specify_type_item, set_item_socket,
};
pub use state::{Activity, CoreState, Posture};
pub use toggle::{is_shaman_toggle, use_toggle_item, Toggled};
pub use tool_refine::{refine_cell, refine_tool, ToolOutcome, ToolRefine};
pub use trade::{
    settle, Accepted, Offered, Said, Settled, Side, Trade, TradeRecord, Trader, Unsettled,
    COMPLETED_NOTICE, EXCHANGE_ITEM_MAX_NUM, EXCHANGE_MAX_DISTANCE, FULL_NOTICE,
    GIVE_REFUSED_NOTICE, OTHER_TRANSACTION_NOTICE, OUT_OF_PLACE_NOTICE, PARTNER_BUSY_NOTICE,
    PARTNER_FULL_NOTICE, PARTNER_OUT_OF_PLACE_NOTICE, PARTNER_QUESTING_NOTICE, QUESTING_NOTICE,
    YANG_LIMIT_NOTICE,
};
pub use use_to_item::{
    change_target_attributes, clean_sash_attr, clean_transmutation, detach_stones, put_stone,
    spend_one, use_target, DetachedStones, ATTR_ADDED_NOTICE, ATTR_ADD_FAILED_NOTICE,
    ATTR_CHANGED_NOTICE, ATTR_CHANGE_WAIT_NOTICE, ATTR_FIVE_NOTICE, ATTR_FULL_NOTICE,
    ATTR_NEED_FOUR_NOTICE, ATTR_SHADE_LEVEL_NOTICE, ATTR_SHADE_TYPE_NOTICE, BROKEN_STONE_SOCKET,
    CLEAN_LOCKED_NOTICE, NOT_BODY_STONE_NOTICE, NOT_WEAPON_STONE_NOTICE, NO_ATTR_SET_NOTICE,
    NO_ATTR_TO_CHANGE_NOTICE, NO_SOCKET_NOTICE, NO_STONE_NOTICE, RARE_CHANGE_FAILED_NOTICE,
    RARE_UNCHANGED_NOTICE, SAME_STONE_NOTICE, STONE_SET_NOTICE, STONE_SOCKETS, TALISMAN_ADD_NOTICE,
    TALISMAN_ATTR_NOTICE,
};
