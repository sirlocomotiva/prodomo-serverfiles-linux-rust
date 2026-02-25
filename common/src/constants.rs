//! Game constants ported from C++ length.h and item_length.h
//!
//! All values match the original C++ definitions exactly.

// ============================================================================
// EMisc constants from length.h
// ============================================================================

/// Maximum length for host/IP address strings
pub const MAX_HOST_LENGTH: u32 = 15;
/// IP address string length
pub const IP_ADDRESS_LENGTH: u32 = 15;
/// Maximum login name length
pub const LOGIN_MAX_LEN: u32 = 30;
/// Maximum password length
pub const PASSWD_MAX_LEN: u32 = 16;
/// Number of characters per account
pub const PLAYER_PER_ACCOUNT: u32 = 4;
/// Maximum account status string length
pub const ACCOUNT_STATUS_MAX_LEN: u32 = 8;
/// Maximum character name length
pub const CHARACTER_NAME_MAX_LEN: u32 = 24;
/// Maximum shop sign length
pub const SHOP_SIGN_MAX_LEN: u32 = 32;

// Inventory layout
/// Inventory page column count
pub const INVENTORY_PAGE_COLUMN: u32 = 5;
/// Inventory page row count
pub const INVENTORY_PAGE_ROW: u32 = 9;
/// Inventory page size (columns * rows)
pub const INVENTORY_PAGE_SIZE: u32 = INVENTORY_PAGE_COLUMN * INVENTORY_PAGE_ROW; // 45
/// Number of inventory pages (default)
pub const INVENTORY_PAGE_COUNT: u32 = 2;
/// Total inventory slots
pub const INVENTORY_MAX_NUM: u32 = INVENTORY_PAGE_SIZE * INVENTORY_PAGE_COUNT; // 90

/// Maximum ability count
pub const ABILITY_MAX_NUM: u32 = 50;
/// Maximum empire count
pub const EMPIRE_MAX_NUM: u32 = 4;
/// Maximum banword length
pub const BANWORD_MAX_LEN: u32 = 24;
/// Maximum social ID length
pub const SOCIAL_ID_MAX_LEN: u32 = 18;
/// Maximum guild name length
pub const GUILD_NAME_MAX_LEN: u32 = 12;
/// Maximum quest name count
pub const QUEST_NAME_MAX_NUM: u32 = 64;
/// Maximum shop host items
pub const SHOP_HOST_ITEM_MAX_NUM: u32 = 40;
/// Maximum shop guest items
pub const SHOP_GUEST_ITEM_MAX_NUM: u32 = 18;
/// Maximum shop pricelist entries
pub const SHOP_PRICELIST_MAX_NUM: u32 = 40;
/// Maximum chat message length
pub const CHAT_MAX_LEN: u32 = 512;
/// Maximum quickslot count
pub const QUICKSLOT_MAX_NUM: u32 = 36;
/// Maximum journal entries
pub const JOURNAL_MAX_NUM: u32 = 2;
/// Maximum query length
pub const QUERY_MAX_LEN: u32 = 8192;
/// Maximum file path length
pub const FILE_MAX_LEN: u32 = 128;
/// Player experience table max level
pub const PLAYER_EXP_TABLE_MAX: u32 = 120;
/// Maximum player level constant
pub const PLAYER_MAX_LEVEL_CONST: u32 = 250;
/// Maximum guild level
pub const GUILD_MAX_LEVEL: u32 = 20;
/// Maximum mob level
pub const MOB_MAX_LEVEL: u32 = 100;
/// Maximum attribute value
pub const ATTRIBUTE_MAX_VALUE: u32 = 20;
/// Maximum character path points
pub const CHARACTER_PATH_MAX_NUM: u32 = 64;
/// Maximum skill count
pub const SKILL_MAX_NUM: u32 = 255;
/// Minimum skillbook delay (seconds)
pub const SKILLBOOK_DELAY_MIN: u32 = 64800;
/// Maximum skillbook delay (seconds)
pub const SKILLBOOK_DELAY_MAX: u32 = 108_000;
/// Maximum skill level
pub const SKILL_MAX_LEVEL: u32 = 40;
/// Maximum apply name length
pub const APPLY_NAME_MAX_LEN: u32 = 32;
/// Maximum event flag name length
pub const EVENT_FLAG_NAME_MAX_LEN: u32 = 32;
/// Maximum mob skill count
pub const MOB_SKILL_MAX_NUM: u32 = 5;
/// Maximum point count
pub const POINT_MAX_NUM: u32 = 255;

// Dragon Soul system
/// Dragon soul box size
pub const DRAGON_SOUL_BOX_SIZE: u32 = 32;
/// Dragon soul box column count
pub const DRAGON_SOUL_BOX_COLUMN_NUM: u32 = 8;
/// Dragon soul box row count
pub const DRAGON_SOUL_BOX_ROW_NUM: u32 = DRAGON_SOUL_BOX_SIZE / DRAGON_SOUL_BOX_COLUMN_NUM; // 4
/// Dragon soul refine grid size
pub const DRAGON_SOUL_REFINE_GRID_SIZE: u32 = 15;
/// Maximum mall bonus count
pub const MAX_AMOUNT_OF_MALL_BONUS: u32 = 20;
/// Maximum wear positions
pub const WEAR_MAX_NUM: u32 = 64;

// Shop tabs
/// Maximum shop tab name length
pub const SHOP_TAB_NAME_MAX: u32 = 32;
/// Maximum shop tab count
pub const SHOP_TAB_COUNT_MAX: u32 = 7;

// Belt inventory
/// Belt inventory slot width
pub const BELT_INVENTORY_SLOT_WIDTH: u32 = 4;
/// Belt inventory slot height
pub const BELT_INVENTORY_SLOT_HEIGHT: u32 = 4;
/// Belt inventory slot count
pub const BELT_INVENTORY_SLOT_COUNT: u32 = BELT_INVENTORY_SLOT_WIDTH * BELT_INVENTORY_SLOT_HEIGHT; // 16

// Word max (from #define)
/// Maximum WORD value (0xFFFF)
pub const WORD_MAX: u32 = 0xFFFF;

// Ability system
/// Maximum ability level
pub const ABILITY_MAX_LEVEL: u32 = 10;

// Dragon Soul strength
/// Maximum dragon soul strength
pub const DRAGON_SOUL_STRENGTH_MAX: u32 = 7;

// ============================================================================
// Item constants from item_length.h
// ============================================================================

/// Maximum item name length
pub const ITEM_NAME_MAX_LEN: u32 = 36;
/// Maximum item values count
pub const ITEM_VALUES_MAX_NUM: u32 = 6;
/// Maximum item small description length
pub const ITEM_SMALL_DESCR_MAX_LEN: u32 = 256;
/// Maximum item limit count
pub const ITEM_LIMIT_MAX_NUM: u32 = 2;
/// Maximum item apply count
pub const ITEM_APPLY_MAX_NUM: u32 = 3;
/// Maximum item socket count (default)
pub const ITEM_SOCKET_MAX_NUM: u32 = 3;
/// Maximum item stack count
pub const ITEM_MAX_COUNT: u32 = 5000;
/// Normal attribute count
pub const ITEM_ATTRIBUTE_NORM_NUM: u32 = 5;
/// Rare attribute count
pub const ITEM_ATTRIBUTE_RARE_NUM: u32 = 2;
/// Normal attribute start index
pub const ITEM_ATTRIBUTE_NORM_START: u32 = 0;
/// Normal attribute end index
pub const ITEM_ATTRIBUTE_NORM_END: u32 = ITEM_ATTRIBUTE_NORM_START + ITEM_ATTRIBUTE_NORM_NUM; // 5
/// Rare attribute start index
pub const ITEM_ATTRIBUTE_RARE_START: u32 = ITEM_ATTRIBUTE_NORM_END; // 5
/// Rare attribute end index
pub const ITEM_ATTRIBUTE_RARE_END: u32 = ITEM_ATTRIBUTE_RARE_START + ITEM_ATTRIBUTE_RARE_NUM; // 7
/// Maximum attribute count
pub const ITEM_ATTRIBUTE_MAX_NUM: u32 = ITEM_ATTRIBUTE_RARE_END; // 7
/// Maximum attribute level
pub const ITEM_ATTRIBUTE_MAX_LEVEL: u32 = 5;
/// Maximum award reason length
pub const ITEM_AWARD_WHY_MAX_LEN: u32 = 50;
/// Maximum refine material count
pub const REFINE_MATERIAL_MAX_NUM: u32 = 5;
/// Elk item vnum
pub const ITEM_ELK_VNUM: u32 = 50026;
/// Toggle item group value index
pub const ITEM_VALUE_TOGGLE_GROUP: u32 = 5;
/// Socket index for unique save time
pub const ITEM_SOCKET_UNIQUE_SAVE_TIME: u32 = ITEM_SOCKET_MAX_NUM - 2; // 1
/// Socket index for unique remain time
pub const ITEM_SOCKET_UNIQUE_REMAIN_TIME: u32 = ITEM_SOCKET_MAX_NUM - 1; // 2
/// Socket index for toggle time
pub const ITEM_SOCKET_TOGGLE_TIME: u32 = 0;
/// Socket index for toggle active state
pub const ITEM_SOCKET_TOGGLE_ACTIVE: u32 = 3;
/// Socket index for toggle riding state
pub const ITEM_SOCKET_TOGGLE_RIDING: u32 = 4;

/// Item socket remain seconds constant
pub const ITEM_SOCKET_REMAIN_SEC: u8 = 0;

// Item value indices
/// Dragon soul poll out bonus value index
pub const ITEM_VALUE_DRAGON_SOUL_POLL_OUT_BONUS_IDX: u32 = 0;
/// Charging amount value index
pub const ITEM_VALUE_CHARGING_AMOUNT_IDX: u32 = 0;
/// Secondary coin unit value index
pub const ITEM_VALUE_SECONDARY_COIN_UNIT_IDX: u32 = 0;

// Dragon soul sockets
/// Dragon soul active socket index
pub const ITEM_SOCKET_DRAGON_SOUL_ACTIVE_IDX: u32 = 2;
/// Charging amount socket index
pub const ITEM_SOCKET_CHARGING_AMOUNT_IDX: u32 = 2;

// Battle pass
/// Battle pass item vnum
pub const ITEM_BATTLE_PASS: u32 = 50027;

// Dragon Soul inventory
/// Dragon soul inventory max slots (calculated from `DS_SLOT_MAX` * `DRAGON_SOUL_GRADE_MAX` * `DRAGON_SOUL_BOX_SIZE`)
pub const DRAGON_SOUL_INVENTORY_MAX_NUM: u32 = 6 * 6 * 32; // 1152

// ============================================================================
// Slot position constants from EMisc2
// ============================================================================

/// Dragon soul equip slot start position
pub const DRAGON_SOUL_EQUIP_SLOT_START: u32 = INVENTORY_MAX_NUM + WEAR_MAX_NUM; // 154
/// Dragon soul equip slot end position
pub const DRAGON_SOUL_EQUIP_SLOT_END: u32 = DRAGON_SOUL_EQUIP_SLOT_START + (6 * 2); // 166 (DS_SLOT_MAX * DRAGON_SOUL_DECK_MAX_NUM)
/// Dragon soul equip reserved slot end position
pub const DRAGON_SOUL_EQUIP_RESERVED_SLOT_END: u32 = DRAGON_SOUL_EQUIP_SLOT_END + (6 * 3); // 184 (DS_SLOT_MAX * DRAGON_SOUL_DECK_RESERVED_MAX_NUM)

/// Belt inventory slot start position
pub const BELT_INVENTORY_SLOT_START: u32 = DRAGON_SOUL_EQUIP_RESERVED_SLOT_END; // 184
/// Belt inventory slot end position
pub const BELT_INVENTORY_SLOT_END: u32 = BELT_INVENTORY_SLOT_START + BELT_INVENTORY_SLOT_COUNT; // 200

/// Maximum inventory and equipment slot count
pub const INVENTORY_AND_EQUIP_SLOT_MAX: u32 = BELT_INVENTORY_SLOT_END; // 200

// ============================================================================
// Guild war constants
// ============================================================================

/// Guild war duration in seconds (30 minutes)
pub const GUILD_WAR_DURATION: u32 = 30 * 60; // 1800
/// Guild war win point threshold
pub const GUILD_WAR_WIN_POINT: u32 = 1000;
/// Guild war ladder half penalty time in seconds (12 hours)
pub const GUILD_WAR_LADDER_HALF_PENALTY_TIME: u32 = 12 * 60 * 60; // 43200

// ============================================================================
// Alignment constants
// ============================================================================

/// Alignment threshold 1
pub const NAME_ALIGNMENT_1: u32 = 10000;
/// Alignment threshold 2
pub const NAME_ALIGNMENT_2: u32 = 40000;
/// Alignment threshold 3
pub const NAME_ALIGNMENT_3: u32 = 80000;
/// Alignment threshold 4
pub const NAME_ALIGNMENT_4: u32 = 120_000;
/// Alignment time constant (1 year in seconds)
pub const ALIGNMENT_TIME: u32 = 60 * 60 * 24 * 365; // 31536000

// ============================================================================
// Aura system constants
// ============================================================================

/// Maximum aura level
pub const AURA_MAX_LEVEL: u32 = 250;
/// Maximum aura refine distance
pub const AURA_REFINE_MAX_DISTANCE: u32 = 1000;

// ============================================================================
// Refine element constants
// ============================================================================

/// Maximum refine element count
pub const REFINE_ELEMENT_MAX: u32 = 3;
/// Minimum refine level for element
pub const ELEMENT_MIN_REFINE_LEVEL: u32 = 7;
/// Yang cost for element upgrade
pub const REFINE_ELEMENT_UPGRADE_YANG: u32 = 3_000_000;
/// Yang cost for element downgrade
pub const REFINE_ELEMENT_DOWNGRADE_YANG: u32 = 10_000_000;
/// Yang cost for element change
pub const REFINE_ELEMENT_CHANGE_YANG: u32 = 10_000_000;
/// Element upgrade probability
pub const REFINE_ELEMENT_UPGRADE_PROBABILITY: u32 = 100;
/// Element downgrade probability
pub const REFINE_ELEMENT_DOWNGRADE_PROBABILITY: u32 = 100;
/// Element change probability
pub const REFINE_ELEMENT_CHANGE_PROBABILITY: u32 = 100;
/// Element upgrade type
pub const REFINE_ELEMENT_TYPE_UPGRADE: u32 = 0;
/// Element downgrade type
pub const REFINE_ELEMENT_TYPE_DOWNGRADE: u32 = 1;
/// Element change type
pub const REFINE_ELEMENT_TYPE_CHANGE: u32 = 2;
/// Element upgrade success type
pub const REFINE_ELEMENT_TYPE_UPGRADE_SUCCES: u32 = 10;
/// Element upgrade fail type
pub const REFINE_ELEMENT_TYPE_UPGRADE_FAIL: u32 = 11;
/// Element downgrade success type
pub const REFINE_ELEMENT_TYPE_DOWNGRADE_SUCCES: u32 = 12;
/// Element change success type
pub const REFINE_ELEMENT_TYPE_CHANGE_SUCCES: u32 = 13;
/// Minimum random element value
pub const REFINE_ELEMENT_RANDOM_VALUE_MIN: u32 = 1;
/// Maximum random element value
pub const REFINE_ELEMENT_RANDOM_VALUE_MAX: u32 = 8;
/// Minimum random bonus element value
pub const REFINE_ELEMENT_RANDOM_BONUS_VALUE_MIN: u32 = 2;
/// Maximum random bonus element value
pub const REFINE_ELEMENT_RANDOM_BONUS_VALUE_MAX: u32 = 12;

// ============================================================================
// Daily gift system constants
// ============================================================================

/// Daily gift day count
pub const DAILY_GIFT_DAY: u32 = 1;
/// Daily gift week days
pub const DAILY_GIFT_WEEK_DAYS: u32 = 7;
/// Daily gift ticket item vnum
pub const DAILY_GIFT_TICKET_ITEM: u32 = 72319;

// ============================================================================
// Multi-language system constants
// ============================================================================

/// Maximum quest notice arguments
pub const MAX_QUEST_NOTICE_ARGS: u32 = 5;

// ============================================================================
// Stage event constants
// ============================================================================

/// Stage event count
pub const STAGE_COUNT: u32 = 4;

// ============================================================================
// Map constants
// ============================================================================

/// Map allow limit
pub const MAP_ALLOW_LIMIT: u32 = 32;

// ============================================================================
// Healing skill vnum
// ============================================================================

/// Healing skill vnum
pub const HEALING_SKILL_VNUM: u32 = 265;

// ============================================================================
// Global rank constants
// ============================================================================

/// Global rank DB save time (seconds)
pub const RANKGLOBAL_DB_SAVE_TIME: u32 = 60 * 20; // 1200
/// Global rank DB flush time (seconds)
pub const RANKGLOBAL_DB_FLUSH_TIME: u32 = 10;
/// Global rank DB flush count
pub const RANKGLOBAL_DB_FLUSH_COUNT: u32 = 2000;

// ============================================================================
// Top player constants
// ============================================================================

/// Top player max level for effect
pub const TOP_PLAYER_MAX_LEVEL: u32 = 120;

// ============================================================================
// Campfire fix constant
// ============================================================================

/// Campfire fix timeout in seconds
pub const CAMPFIRE_FIX_SEC: u32 = 60;

// ============================================================================
// Sash system constants
// ============================================================================

/// Sash grade 1 absorption value
pub const SASH_GRADE_1_ABS: u32 = 1;
/// Sash grade 2 absorption value
pub const SASH_GRADE_2_ABS: u32 = 5;
/// Sash grade 3 absorption value
pub const SASH_GRADE_3_ABS: u32 = 10;
/// Sash grade 4 minimum absorption
pub const SASH_GRADE_4_ABS_MIN: u32 = 11;
/// Sash grade 4 maximum absorption
pub const SASH_GRADE_4_ABS_MAX: u32 = 25;
/// Sash grade 4 max combination absorption
pub const SASH_GRADE_4_ABS_MAX_COMB: u32 = 19;
/// Sash grade 4 absorption range
pub const SASH_GRADE_4_ABS_RANGE: u32 = 5;
/// Sash effect from absorption threshold
pub const SASH_EFFECT_FROM_ABS: u32 = 19;
/// Sash clean attribute value
pub const SASH_CLEAN_ATTR_VALUE0: u32 = 7;
/// Sash window max materials
pub const SASH_WINDOW_MAX_MATERIALS: u32 = 2;
/// Sash grade 1 price
pub const SASH_GRADE_1_PRICE: u32 = 100_000;
/// Sash grade 2 price
pub const SASH_GRADE_2_PRICE: u32 = 200_000;
/// Sash grade 3 price
pub const SASH_GRADE_3_PRICE: u32 = 300_000;
/// Sash grade 4 price
pub const SASH_GRADE_4_PRICE: u32 = 500_000;
/// Sash combine grade 1 probability
pub const SASH_COMBINE_GRADE_1: u32 = 80;
/// Sash combine grade 2 probability
pub const SASH_COMBINE_GRADE_2: u32 = 70;
/// Sash combine grade 3 probability
pub const SASH_COMBINE_GRADE_3: u32 = 50;
/// Sash combine grade 4 probability
pub const SASH_COMBINE_GRADE_4: u32 = 30;

// ============================================================================
// Change look system constants
// ============================================================================

/// Change look window max materials
pub const CL_WINDOW_MAX_MATERIALS: u32 = 2;
/// Change look clean attribute value
pub const CL_CLEAN_ATTR_VALUE0: u32 = 8;
/// Change look transmutation price
pub const CL_TRANSMUTATION_PRICE: u32 = 50_000_000;

// ============================================================================
// Attribute 6th/7th constants
// ============================================================================

/// Attribute 6/7 add slot max
pub const ATTR67_ADD_SLOT_MAX: u32 = 1;
/// Attribute 6/7 material max count
pub const ATTR67_MATERIAL_MAX_COUNT: u32 = 10;
/// Attribute 6/7 support max count
pub const ATTR67_SUPPORT_MAX_COUNT: u32 = 5;
/// Attribute 6/7 success per material
pub const ATTR67_SUCCESS_PER_MATERIAL: u32 = 2;
/// Attribute 6/7 add wait time (seconds)
pub const ATTR67_ADD_WAIT_TIME: u32 = 60 * 60 * 24; // 86400

// ============================================================================
// Private shop constants
// ============================================================================

/// Private shop page max count
pub const PRIVATE_SHOP_PAGE_MAX_NUM: u32 = 2;
/// Private shop width
pub const PRIVATE_SHOP_WIDTH: u32 = 8;
/// Private shop height
pub const PRIVATE_SHOP_HEIGHT: u32 = 8;
/// Private shop page item max count
pub const PRIVATE_SHOP_PAGE_ITEM_MAX_NUM: u32 = PRIVATE_SHOP_WIDTH * PRIVATE_SHOP_HEIGHT; // 64
/// Private shop host item max count
pub const PRIVATE_SHOP_HOST_ITEM_MAX_NUM: u32 =
    PRIVATE_SHOP_PAGE_ITEM_MAX_NUM * PRIVATE_SHOP_PAGE_MAX_NUM; // 128
/// Private shop locked slot max count
pub const PRIVATE_SHOP_LOCKED_SLOT_MAX_NUM: u32 = PRIVATE_SHOP_HOST_ITEM_MAX_NUM / 2; // 64
/// Private shop slot unlock item vnum
pub const PRIVATE_SHOP_SLOT_UNLOCK_ITEM: u32 = 72357;
/// Private shop max premium time (seconds)
pub const PRIVATE_SHOP_MAX_PREMIUM_TIME: u32 = 3600 * 24 * 7; // 604800
/// Selected item max count
pub const SELECTED_ITEM_MAX_NUM: u32 = 10;
/// Valid sale day interval
pub const VALID_SALE_DAY_INTERVAL: u32 = 30;
/// Valid market price day interval
pub const VALID_MARKET_PRICE_DAY_INTERVAL: u32 = 3;
/// Market item price update interval (seconds)
pub const MARKET_ITEM_PRICE_UPDATE_SEC_INTERVAL: u32 = 3600;
/// Private shop title max length
pub const TITLE_MAX_LEN: u32 = 32;
/// Private shop title min length
pub const TITLE_MIN_LEN: u32 = 3;

// ============================================================================
// Switchbot constants
// ============================================================================

/// Switchbot slot count
pub const SWITCHBOT_SLOT_COUNT: u32 = 5;
/// Switchbot alternative count
pub const SWITCHBOT_ALTERNATIVE_COUNT: u32 = 2;
/// Switchbot price type (1 = switching item, 2 = yang)
pub const SWITCHBOT_PRICE_TYPE: u32 = 1;
/// Switchbot price amount
pub const SWITCHBOT_PRICE_AMOUNT: u32 = 1;

// ============================================================================
// Fish event constants
// ============================================================================

/// Fish event slots count
pub const FISH_EVENT_SLOTS_NUM: u32 = 24;
/// Fish event box item vnum
pub const ITEM_FISH_EVENT_BOX: u32 = 25106;
/// Fish event special box item vnum
pub const ITEM_FISH_EVENT_BOX_SPECIAL: u32 = 25107;

// ============================================================================
// Gold system constants
// ============================================================================

/// Maximum gold with limit removed
pub const GOLD_MAX_MAX: u64 = 1_200_000_000_000_000_000;

// ============================================================================
// Gaya system constants
// ============================================================================

/// Maximum gaya currency
pub const GAYA_MAX: u32 = 999_999;

// ============================================================================
// Conqueror level constants
// ============================================================================

/// Conqueror experience table max
pub const PLAYER_CONQUEROR_EXP_TABLE_MAX: u32 = 30;
/// Maximum conqueror level
pub const PLAYER_MAX_CONQUEROR_LEVEL_CONST: u32 = 30;

// ============================================================================
// Extended safebox constants
// ============================================================================

/// Safebox max page count
pub const SAFEBOX_MAX_PAGE_COUNT: u32 = 6;
/// Safebox max item count
pub const SAFEBOX_MAX_NUM: u32 = 45 * SAFEBOX_MAX_PAGE_COUNT; // 270

// ============================================================================
// Inventory protected system constants
// ============================================================================

/// Inventory protected password max length
pub const INVENTORY_PROTECTED_PASSWORD_MAX_LEN: u32 = 6;

// ============================================================================
// Custom inventory constants
// ============================================================================

/// Custom inventory page size
pub const CUSTOM_INVENTORY_PAGE_SIZE: u32 = 45;
/// Custom inventory page count
pub const CUSTOM_INVENTORY_PAGE_COUNT: u32 = 4;
/// Custom inventory max items
pub const CUSTOM_INVENTORY_MAX_NUM: u32 = CUSTOM_INVENTORY_PAGE_SIZE * CUSTOM_INVENTORY_PAGE_COUNT; // 180
/// Custom inventory category count
pub const CUSTOM_INVENTORY_CATEGORY_NUM: u32 = 6;

// ============================================================================
// Extended inventory system constants
// ============================================================================

/// Inventory open page count
pub const INVENTORY_OPEN_PAGE_COUNT: u32 = 2;
/// Inventory open key vnum
pub const INVENTORY_OPEN_KEY_VNUM: u32 = 72319;
/// Inventory open key vnum 2
pub const INVENTORY_OPEN_KEY_VNUM2: u32 = 72320;
/// Inventory start delete vnum
pub const INVENTORY_START_DELETE_VNUM: u32 = INVENTORY_OPEN_KEY_VNUM;
/// Inventory need key start page
pub const INVENTORY_NEED_KEY_START: u32 = 2;
/// Inventory need key increase interval
pub const INVENTORY_NEED_KEY_INCREASE: u32 = 3;
/// Inventory width
pub const INVENTORY_WIDTH: u32 = 5;
/// Inventory height
pub const INVENTORY_HEIGHT: u32 = 9;
/// Inventory open page size
pub const INVENTORY_OPEN_PAGE_SIZE: u32 = INVENTORY_OPEN_PAGE_COUNT * INVENTORY_PAGE_SIZE; // 90
/// Inventory locked page count
pub const INVENTORY_LOCKED_PAGE_COUNT: u32 = INVENTORY_PAGE_COUNT - INVENTORY_OPEN_PAGE_COUNT; // 0
/// Inventory lock cover count
pub const INVENTORY_LOCK_COVER_COUNT: u32 = INVENTORY_LOCKED_PAGE_COUNT * INVENTORY_HEIGHT; // 0

// ============================================================================
// Custom inventory slot positions
// ============================================================================

/// Custom inventory slot start position
pub const CUSTOM_INVENTORY_SLOT_START: u32 = BELT_INVENTORY_SLOT_END; // 200
/// Custom inventory slot end position
pub const CUSTOM_INVENTORY_SLOT_END: u32 =
    CUSTOM_INVENTORY_SLOT_START + (CUSTOM_INVENTORY_MAX_NUM * CUSTOM_INVENTORY_CATEGORY_NUM); // 1280
