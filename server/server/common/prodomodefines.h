#ifndef __M2_PRODOMO_DEFINES__
#define __M2_PRODOMO_DEFINES__

enum eCommonDefines {
	MAP_ALLOW_LIMIT = 32, // __PSVF_Map_Allow_Limit__
};

//////////////////////////////////////////////////////////////////////////
// ### General Features ###
#define ENABLE_D_NJGUILD // __PSVF_Guild_Features__
#define ENABLE_NEWSTUFF // new stuff from martysama
#define ENABLE_PORT_SECURITY // new 3306 port security
#define ENABLE_BELT_INVENTORY_EX // refactorized belt invetory
// ### General Features ###
#define __SASH_SYSTEM__ // __GF Sash / Acce__
#define __CHANGELOOK_SYSTEM__ // __GF Transmutation__
#define __ENABLE_CHANGELOOK_SYSTEM__ // __GF Transmutation__
#define __QUEST_RENEWAL__ // __GF_Quest_Renewal__
#ifdef __QUEST_RENEWAL__
	#define _QR_MS_
#endif
#define ENABLE_CUBE_RENEWAL_WORLDARD // __GF_Cube_Renewal__
#define __BL_ENABLE_PICKUP_ITEM_EFFECT__ // __GF_Pickup_SlotEffect__
#define ENABLE_FISH_EVENT // __GF_JigSaw_Event__
#define WJ_ENABLE_TRADABLE_ICON // __GF_Tradable_Icon__
#define __NEW_EXCHANGE_WINDOW__ // __GF_Exchange_Window__
#define ENABLE_DRAGONSOUL_ALCHEMY_PLUS // __GF_Mythic_Alchemy_Set_Bonus__
#define ENABLE_REFINE_ELEMENT // __GF_Refine_Element_Fire&Ice&Lightning&Darkness&Earth&Wind__
#define ENABLE_PET_COSTUME_SYSTEM // __GF_Pet_Slot__
#define __AURA_SYSTEM__ // __GF_Aura__
#define __ATTR_6TH_7TH__ // __GF_6&7Attr__
#define LOCALE_STRING_RENEWAL // __GF_Locale_String_Renewal&Adapted_for_MultiLanguage
#define __NEW_SET_BONUS__ // __GF_Costume&Sash&Aura&Item_BonusSet__
// ### // __Reworked_Bonuses__ ####
#define NEW_BONUS // __GF_Metin&Boss_Bonus__
#define BONUS_PCT // __GF__Proto_New_Bonuses__
// ### // __Reworked_Bonuses__ ####
#define __SHIP_DEFENSE__ // __GF_Hydra_Dungeon__
#define __VERSION_162__ // __GF_Hydra_Dungeon__
#ifdef __VERSION_162__// __GF_Hydra_Dungeon__ 
	#define HEALING_SKILL_VNUM 265// __GF_Hydra_Dungeon__
#endif
#define __DUNGEON_FOR_GUILD__ // __GF_Meley_Lair__
#ifdef __DUNGEON_FOR_GUILD__// __GF_Meley_Lair__
	#define __MELEY_LAIR_DUNGEON__// __GF_Meley_Lair__
	#ifdef __MELEY_LAIR_DUNGEON__// __GF_Meley_Lair__
		#define __DESTROY_INFINITE_STATUES_GM__// __GF_Meley_Lair__
		#define __LASER_EFFECT_ON_75HP__// __GF_Meley_Lair__
		#define __LASER_EFFECT_ON_50HP__// __GF_Meley_Lair__
	#endif
#endif
#define ENABLE_GAYA_SYSTEM // __GF_Gaya_System__
#define __CONQUEROR_LEVEL__ // __GF_Conqueror(Level&Map&Attr&Items&Bonuses)__
#define ENABLE_GLOVE_SYSTEM	// __GF_Glove&Yohara__
#ifdef ENABLE_GLOVE_SYSTEM
	#define ENABLE_GLOVE_ITEM_ATTR	// __GF_ItemAttrTable_For_Gloves
#endif
#define ELEMENT_TARGET  //__GF_17.5_ELEMENT_ON_TARGET__
// ### // __Reworked_Inventory__ ####
#define ENABLE_EXTEND_INVEN_SYSTEM // __GF_Inventory__
#define __EXTENDED_SAFEBOX__ // __GF_Safebox__
#define ENABLE_EXTEND_INVEN_SYSTEM // __GF_3&4_Blocked_Inventory__
// ### // __Reworked_Inventory__ ####
// ### General Features ###
//////////////////////////////////////////////////////////////////////////
// ### // __Reworked__ ####
#define ENABLE_FLOOD_PRETECTION // __Fix__
#define IS_FIX_POISON_RISIPA_AFFECT(type) ((type) == AFFECT_MOV_SPEED || (type) == AFFECT_ATT_SPEED || (type) == AFFECT_STR || (type) == AFFECT_DEX || (type) == AFFECT_INT || (type) == AFFECT_CON || (type) == AFFECT_CHINA_FIREWORK || (type) == SKILL_JEONGWI || (type) == SKILL_GEOMKYUNG || (type) == SKILL_CHUNKEON || (type) == SKILL_EUNHYUNG || (type) == SKILL_GYEONGGONG || (type) == SKILL_GWIGEOM || (type) == SKILL_TERROR || (type) == SKILL_JUMAGAP || (type) == SKILL_MANASHILED || (type) == SKILL_HOSIN || (type) == SKILL_REFLECT || (type) == SKILL_KWAESOK || (type) == SKILL_JEUNGRYEOK || (type) == SKILL_GICHEON) // __Fix__
#define __FIX_CHANGE_SEX_WITHOUT_RELOG__ // __Fix__
#define ENABLE_FLY_FIX // __Fix__
#define ENABLE_EXTENDED_RELOAD // __Fix__
#define ENABLE_BOSS_KICK_INTO_WALLS_FIX // __Fix__
#define ENABLE_KICK_MULTI_IP_OX // __Fix__
#define ENABLE_HEADER_100_FIX // __Fix__
#define __FIX_UPDATE_LEVEL__ // __Fix__
#define ENABLE_EXTENDED_SOCKETS // __Fix__
#define __FIX_UPDATE_ALIGNMENT__ // __Fix__
#define __FIX_COSTUM_NUNTA_PESTE_COSTUM_NORMAL__ // __Fix__
#define FIX_BLOCK_MOB_SAFEZONE // __Fix__
#define __FIX_BREASLA_LA_SCHIMBARE_REGAT__ // __Fix__
#define __FIX_UPDATE_PLAYTIME_AND_ITEMS__ // __Fix__
#define __FIX_DELETE_FRIEND_REFRESH__ // __Fix__
#define __FIX_INFO_REFINE_DRAGONSOUL__ // __Fix__
#define __FIX_DUNGEON_PARTY__ // __Fix__
#define ENABLE_FIX_READ_ETC_DROP_ITEM﻿_FILE_BY_VNUM // __Fix__
#define __FIX_EXPLOIT_QUEST__ // __Fix__
#define FLUSH_AT_SHUTDOWN // __Fix__
#define FIX_SELECT_EMPIRE_PHASE // __Fix__
#define __FIX_EXP_GRUP__ // __Fix__
#define __FIX_ITEMS_TYPE_33__ // __Fix__
#define __FIX_KICK_HACK__ // __Fix__
#define ENABLE_COUNT_MONSTER_FIX // __Fix__
#define ENABLE_DUNGEON_NOTICE_FIX // __Fix__
#define FIX_LOAD_MOBS_WITH_MOUNT // __Fix__
#define __FIX_NIVEL_CAL__ // __Fix__
#define __FIX_PC_SELECT_QUEST__ // __Fix__
#define ENABLE_FIX_QUICK_SLOT // __Fix__
#define __FIX_SECONDARY_SKILL__ // __Fix__
#define __FIX_TIMER_EVENT__ // __Fix__
#define ENABLE_DESTROY_GUILD_IF_WAR_IS_ACTIVE_FIX // __Fix__
#define __FIX_BUG_AURA_TAIS_FARA_ARMA__ // __Fix__
#define __FIX_HP_GROUP__ // __Fix__
#define __FIX_COMANDA_RAZBOI__ // __Fix__
#define __FIX_CAMPFIRE__ // __Fix__
#define __FIX_CAMPFIRE__SEC 60 // __Fix__
#define __FIX_BUG_IMMUNE__ // __Fix__
#define __FIX_CHANGE_SKILL_VISUAL_BUG__ // __Fix__
#define FIX_DUNGEON_MUSIC // __Fix__
#define POTIUNI_PERMANENTE_RELUCRATE // __Fix__
#define FIX_STICLA_CUNOASTERII // __Fix__
#define FIX_AURA_BUG_VIZUAL // __Fix__
#define PRODOMO_EMOTIE_FIX // __Fix__
#define PRODOMO_PIATRA_FIX // __Fix__
#define PRODOMO_SOME_FIX // __Fix__
#define lib_essex_prodomo // __Fix__
#define ENABLE_WEDDING_FIX // __Fix__
#define ENABLE_CLEAR_OLD_GUILDS_LANDS_BY_INACTIVITY // __Fix__
#define ENABLE_UPDATE_LASTPLAY_REAL_TIME // __Fix__
//////////////////////////////////////////////////////////////////////////
// ### // __Reworked__ ####


// __Top_System__

#define ENABLE_TOP_PLAYERS_EFFECT
#ifdef ENABLE_TOP_PLAYERS_EFFECT // __Top_Player_Effect__
	#define TOP_PLAYER_MAX_LEVEL 120 // __Top_Player_Effect__
#endif
#define ENABLE_SHOW_LIDER_AND_GENERAL_GUILD // __Leader&CoLeader_Show__
#define __ENABLE_PREMIUM_PLAYERS__

// ### Multi_Language Systems ###
#define __MULTI_LANGUAGE_SYSTEM__ // __Renewal_11Languages_MultiLanguage__
#define __EXTENDED_WHISPER_DETAILS__ // __Renewal_11Languages_MultiLanguage__&&__Whisper_Details_For_MultiLanguage__
// ### Multi_Language Systems ###

// ### CommonDefines Systems ### -- test_git2
//////////////////////////////////////////////////////////////////////////
#endif

#define ENABLE_MOVE_CHANNEL // __Move_Channel_1&2&3&4__
#define __SEND_TARGET_INFO__ // __Renewal_Target_Info
#define __SORT_INVENTORY_ITEMS__ // __Sort_Inventory&Special_Inventory_Items__
#define __VIEW_TARGET_PLAYER_HP__ // __Target_Info__
#define __VIEW_TARGET_DECIMAL_HP__ // __Target_Info__
#define ENABLE_SEND_TARGET_INFO_EXTENDED //__Target_Info__
#define __DAILY_GIFT_SYSTEM__ /// __New_Daily_Gift__
#define __ENABLE_BIOLOGIST_RENEWAL_SYSTEM__ // __Renewal_Biologist__
#define PRODOMO_HIDE_COSTUME // __Renewal_Hide_System__
#define ENABLE_MULTI_FARM_BLOCK // __Renewal_Hwid_Multi_Farm_Block__
#define RENEWAL_PICKUP_AFFECT // __Renewal_PickUp_Instant__
#define __GM_PE_N__ // __Renewal_n_GameMaster_Name__
#define ENABLE_ITEMSHOP // __Renewal_InGame_ItemShop__
#define ENABLE_ITEMSHOP_TO_INVENTORY // __Renewal_InGame_ItemShop__
#define ENABLE_MESSENGER_TEAM // __Renewal_Friend_List
#define ENABLE_RENEWAL_SHOPEX // __Renewal_ShopEx__
#define ENABLE_REMOVE_LIMIT_GOLD // __Unsigned_Long_Long_Limit_Gold__
#define ENABLE_AFFECT_RENEWAL // __Reworked_Affect_Renewal__
#define __ENABLE_ADVANCE_SKILL_SELECT__ // __Renewal_Skill_Select__
#define NEW_ATTR_RANFORSARI // __Renewal_Reinforcements__
#define OFFLINE_MESSAGE_REWORKED // __Renewal_OfflineMessages__
#define ENABLE_MOUNT_COSTUME_SYSTEM // __Renewal_Mount_Costume__
#define ENABLE_WEAPON_COSTUME_SYSTEM //__Renewal_Weapon_Costume__
#define __GUILD_BONUSES__ // __Renewal_Bonuses_For_Guild__
#define __7AND8TH_SKILLS__ //
#define ENABLE_NEW_PASSIVE_SKILL
#define ENABLE_EXPRESSING_EMOTION
#define __DUNGEON_INFO__
#define __ENABLE_INVENTORY_PROTECTED_SYSTEM__
#define ENABLE_GLOBAL_RANK
#ifdef ENABLE_GLOBAL_RANK
	#define RANKGLOBAL_DB_SAVE_TIME (int)60*20
	#define RANKGLOBAL_DB_FLUSH_TIME (int)10
	#define RANKGLOBAL_DB_FLUSH_COUNT (int)2000
#endif
#define __PREMIUM_PRIVATE_SHOP__
#ifdef __PREMIUM_PRIVATE_SHOP__
	#define ENABLE_PRIVATE_SHOP_PREMIUM_TIME
	// #define ENABLE_PRIVATE_SHOP_CHEQUE
	//#define ENABLE_PRIVATE_SHOP_BUNDLE_REQ
	//#define ENABLE_PRIVATE_SHOP_BUILD_LIMITATIONS
	// #define ENABLE_PRIVATE_SHOP_SPECIAL_INV
	// #define ENABLE_PRIVATE_SHOP_EXTEND_INV
	// #define ENABLE_PRIVATE_SHOP_EXTEND_SPECIAL_INV
	#define ENABLE_PRIVATE_SHOP_LOCKED_SLOTS
	// #define ENABLE_PRIVATE_SHOP_CHANGE_LOOK
	// #define ENABLE_PRIVATE_SHOP_REFINE_ELEMENT
	// #define ENABLE_PRIVATE_SHOP_APPLY_RANDOM
	// #define ENABLE_PRIVATE_SHOP_SOCKET5
#endif
#define ENABLE_SWITCHBOT
#define __EVENT_MANAGER__
#define __ENABLE_SHAMAN_SYSTEM__
#define __PET_SYSTEM__
#define __UDP_BLOCK__
#define ENABLE_RENEWAL_SHOPEX
#define __CASKET_PREVIEW_ENABLE__
#define __SKIN_SYSTEM__
#define ENABLE_BATTLE_PASS
#define ENABLE_REWARD_SYSTEM
#define __WORLD_BOSS_EVENT__
#define ENABLE_CUSTOM_INVENTORY