#ifndef __INC_TABLES_H__
#define __INC_TABLES_H__

#include "length.h"
#include "item_length.h"
#include "prodomodefines.h"
#ifdef __PREMIUM_PRIVATE_SHOP__
#include <boost/functional/hash.hpp>
#endif

typedef	DWORD IDENT;


enum
{
	HEADER_GD_LOGIN				= 1,
	HEADER_GD_LOGOUT			= 2,

	HEADER_GD_PLAYER_LOAD		= 3,
	HEADER_GD_PLAYER_SAVE		= 4,
	HEADER_GD_PLAYER_CREATE		= 5,
	HEADER_GD_PLAYER_DELETE		= 6,

	HEADER_GD_LOGIN_KEY			= 7,
	// 8 empty
	HEADER_GD_BOOT				= 9,
	HEADER_GD_PLAYER_COUNT		= 10,
	HEADER_GD_QUEST_SAVE		= 11,
	HEADER_GD_SAFEBOX_LOAD		= 12,
	HEADER_GD_SAFEBOX_SAVE		= 13,
	HEADER_GD_SAFEBOX_CHANGE_SIZE	= 14,
	HEADER_GD_EMPIRE_SELECT		= 15,

	HEADER_GD_SAFEBOX_CHANGE_PASSWORD		= 16,
	HEADER_GD_SAFEBOX_CHANGE_PASSWORD_SECOND	= 17, // Not really a packet, used internal
	HEADER_GD_DIRECT_ENTER		= 18,

	HEADER_GD_GUILD_SKILL_UPDATE	= 19,
	HEADER_GD_GUILD_EXP_UPDATE		= 20,
	HEADER_GD_GUILD_ADD_MEMBER		= 21,
	HEADER_GD_GUILD_REMOVE_MEMBER	= 22,
	HEADER_GD_GUILD_CHANGE_GRADE	= 23,
	HEADER_GD_GUILD_CHANGE_MEMBER_DATA	= 24,
	HEADER_GD_GUILD_DISBAND		= 25,
	HEADER_GD_GUILD_WAR			= 26,
	HEADER_GD_GUILD_WAR_SCORE		= 27,
	HEADER_GD_GUILD_CREATE		= 28,

	HEADER_GD_ITEM_SAVE			= 30,
	HEADER_GD_ITEM_DESTROY		= 31,

	HEADER_GD_ADD_AFFECT		= 32,
	HEADER_GD_REMOVE_AFFECT		= 33,

	HEADER_GD_HIGHSCORE_REGISTER	= 34,
	HEADER_GD_ITEM_FLUSH		= 35,

	HEADER_GD_PARTY_CREATE		= 36,
	HEADER_GD_PARTY_DELETE		= 37,
	HEADER_GD_PARTY_ADD			= 38,
	HEADER_GD_PARTY_REMOVE		= 39,
	HEADER_GD_PARTY_STATE_CHANGE	= 40,
	HEADER_GD_PARTY_HEAL_USE		= 41,

	HEADER_GD_FLUSH_CACHE		= 42,
	HEADER_GD_RELOAD_PROTO		= 43,

	HEADER_GD_CHANGE_NAME		= 44,
	HEADER_GD_SMS				= 45,

	HEADER_GD_GUILD_CHANGE_LADDER_POINT	= 46,
	HEADER_GD_GUILD_USE_SKILL		= 47,

	HEADER_GD_REQUEST_EMPIRE_PRIV	= 48,
	HEADER_GD_REQUEST_GUILD_PRIV	= 49,

	HEADER_GD_MONEY_LOG				= 50,

	HEADER_GD_GUILD_DEPOSIT_MONEY				= 51,
	HEADER_GD_GUILD_WITHDRAW_MONEY				= 52,
	HEADER_GD_GUILD_WITHDRAW_MONEY_GIVE_REPLY	= 53,

	HEADER_GD_REQUEST_CHARACTER_PRIV	= 54,

	HEADER_GD_SET_EVENT_FLAG			= 55,

	HEADER_GD_PARTY_SET_MEMBER_LEVEL	= 56,

	HEADER_GD_GUILD_WAR_BET		= 57,

	HEADER_GD_CREATE_OBJECT		= 60,
	HEADER_GD_DELETE_OBJECT		= 61,
	HEADER_GD_UPDATE_LAND		= 62,

	HEADER_GD_MARRIAGE_ADD		= 70,
	HEADER_GD_MARRIAGE_UPDATE	= 71,
	HEADER_GD_MARRIAGE_REMOVE	= 72,

	HEADER_GD_WEDDING_REQUEST	= 73,
	HEADER_GD_WEDDING_READY		= 74,
	HEADER_GD_WEDDING_END		= 75,

	HEADER_GD_AUTH_LOGIN		= 100,
	HEADER_GD_LOGIN_BY_KEY		= 101,
	HEADER_GD_MALL_LOAD			= 107,

	HEADER_GD_MYSHOP_PRICELIST_UPDATE	= 108,
	HEADER_GD_MYSHOP_PRICELIST_REQ		= 109,

	HEADER_GD_BLOCK_CHAT				= 110,
	
	HEADER_GD_RELOAD_ADMIN			= 115,
	HEADER_GD_BREAK_MARRIAGE		= 116,
	HEADER_GD_ELECT_MONARCH			= 117,
	HEADER_GD_CANDIDACY				= 118,
	HEADER_GD_ADD_MONARCH_MONEY		= 119,
	HEADER_GD_TAKE_MONARCH_MONEY	= 120,
	HEADER_GD_COME_TO_VOTE			= 121,
	HEADER_GD_RMCANDIDACY			= 122,
	HEADER_GD_SETMONARCH			= 123,
	HEADER_GD_RMMONARCH			= 124,
	HEADER_GD_DEC_MONARCH_MONEY = 125,

	HEADER_GD_CHANGE_MONARCH_LORD = 126,
	HEADER_GD_BLOCK_COUNTRY_IP		= 127,
	HEADER_GD_BLOCK_EXCEPTION		= 128,

	HEADER_GD_REQ_CHANGE_GUILD_MASTER	= 129,

	HEADER_GD_REQ_SPARE_ITEM_ID_RANGE	= 130,

	HEADER_GD_UPDATE_HORSE_NAME		= 131,
	HEADER_GD_REQ_HORSE_NAME		= 132,

	HEADER_GD_DC					= 133,

	HEADER_GD_VALID_LOGOUT			= 134,

#ifdef ENABLE_MOVE_CHANNEL
	HEADER_GD_FIND_CHANNEL 			= 135,
#endif

	HEADER_GD_REQUEST_CHARGE_CASH	= 137,

	HEADER_GD_DELETE_AWARDID	= 138,	// delete gift notify icon

	HEADER_GD_UPDATE_CHANNELSTATUS	= 139,
	HEADER_GD_REQUEST_CHANNELSTATUS	= 140,
#ifdef __EVENT_MANAGER__
	HEADER_GD_UPDATE_EVENT_STATUS	= 141,
	HEADER_GD_EVENT_NOTIFICATION	= 142,
#endif
#ifdef ENABLE_TOP_PLAYERS_EFFECT
	HEADER_GD_REMOVE_TOP_PLAYER_INFO	= 145,
#endif
#ifdef __PREMIUM_PRIVATE_SHOP__
	HEADER_GD_PRIVATE_SHOP,
#endif
#if defined(OFFLINE_MESSAGE_REWORKED)
	HEADER_GD_SEND_OFFLINE_MESSAGE = 147,
#endif
#ifdef __DUNGEON_FOR_GUILD__
	HEADER_GD_GUILD_DUNGEON			= 150,
	HEADER_GD_GUILD_DUNGEON_CD 		= 151,
#endif
#ifdef __MULTI_LANGUAGE_SYSTEM__
	HEADER_GD_REQUEST_CHANGE_LANGUAGE = 152,
#endif
	HEADER_GD_REQUEST_OFFLINE_MESSAGES = 153,
#if defined(__WORLD_BOSS_EVENT__)
	HEADER_GD_ADD_TEMP_WORLD_BOSS_RK,
	HEADER_GD_GET_TEMP_WORLD_BOSS_RK,
	HEADER_GD_CLR_TEMP_WORLD_BOSS_RK,
	HEADER_GD_WORLD_BOSS_RANKING,
#endif


	HEADER_GD_SETUP			= 0xff,

	///////////////////////////////////////////////
	HEADER_DG_NOTICE			= 1,

	HEADER_DG_LOGIN_SUCCESS			= 30,
	HEADER_DG_LOGIN_NOT_EXIST		= 31,
	HEADER_DG_LOGIN_WRONG_PASSWD	= 33,
	HEADER_DG_LOGIN_ALREADY			= 34,

	HEADER_DG_PLAYER_LOAD_SUCCESS	= 35,
	HEADER_DG_PLAYER_LOAD_FAILED	= 36,
	HEADER_DG_PLAYER_CREATE_SUCCESS	= 37,
	HEADER_DG_PLAYER_CREATE_ALREADY	= 38,
	HEADER_DG_PLAYER_CREATE_FAILED	= 39,
	HEADER_DG_PLAYER_DELETE_SUCCESS	= 40,
	HEADER_DG_PLAYER_DELETE_FAILED	= 41,

	HEADER_DG_ITEM_LOAD			= 42,

	HEADER_DG_BOOT				= 43,
	HEADER_DG_QUEST_LOAD		= 44,

	HEADER_DG_SAFEBOX_LOAD					= 45,
	HEADER_DG_SAFEBOX_CHANGE_SIZE			= 46,
	HEADER_DG_SAFEBOX_WRONG_PASSWORD		= 47,
	HEADER_DG_SAFEBOX_CHANGE_PASSWORD_ANSWER = 48,

	HEADER_DG_EMPIRE_SELECT		= 49,

	HEADER_DG_AFFECT_LOAD		= 50,
	HEADER_DG_MALL_LOAD			= 51,

	HEADER_DG_DIRECT_ENTER		= 55,

	HEADER_DG_GUILD_SKILL_UPDATE	= 56,
	HEADER_DG_GUILD_SKILL_RECHARGE	= 57,
	HEADER_DG_GUILD_EXP_UPDATE		= 58,

	HEADER_DG_PARTY_CREATE		= 59,
	HEADER_DG_PARTY_DELETE		= 60,
	HEADER_DG_PARTY_ADD			= 61,
	HEADER_DG_PARTY_REMOVE		= 62,
	HEADER_DG_PARTY_STATE_CHANGE	= 63,
	HEADER_DG_PARTY_HEAL_USE		= 64,
	HEADER_DG_PARTY_SET_MEMBER_LEVEL	= 65,

	HEADER_DG_TIME			= 90,
	HEADER_DG_ITEM_ID_RANGE		= 91,

	HEADER_DG_GUILD_ADD_MEMBER		= 92,
	HEADER_DG_GUILD_REMOVE_MEMBER	= 93,
	HEADER_DG_GUILD_CHANGE_GRADE	= 94,
	HEADER_DG_GUILD_CHANGE_MEMBER_DATA	= 95,
	HEADER_DG_GUILD_DISBAND		= 96,
	HEADER_DG_GUILD_WAR			= 97,
	HEADER_DG_GUILD_WAR_SCORE		= 98,
	HEADER_DG_GUILD_TIME_UPDATE		= 99,
	HEADER_DG_GUILD_LOAD		= 100,
	HEADER_DG_GUILD_LADDER		= 101,
	HEADER_DG_GUILD_SKILL_USABLE_CHANGE	= 102,
	HEADER_DG_GUILD_MONEY_CHANGE	= 103,
	HEADER_DG_GUILD_WITHDRAW_MONEY_GIVE	= 104,

	HEADER_DG_SET_EVENT_FLAG		= 105,

	HEADER_DG_GUILD_WAR_RESERVE_ADD	= 106,
	HEADER_DG_GUILD_WAR_RESERVE_DEL	= 107,
	HEADER_DG_GUILD_WAR_BET		= 108,

	HEADER_DG_RELOAD_PROTO		= 120,
#ifdef ENABLE_ITEMSHOP
	HEADER_DG_ITEMSHOP = 76,
	HEADER_GD_ITEMSHOP = 76,
#endif
	HEADER_DG_CHANGE_NAME		= 121,

	HEADER_DG_AUTH_LOGIN		= 122,

	HEADER_DG_CHANGE_EMPIRE_PRIV	= 124,
	HEADER_DG_CHANGE_GUILD_PRIV		= 125,

	HEADER_DG_MONEY_LOG			= 126,

	HEADER_DG_CHANGE_CHARACTER_PRIV	= 127,

	HEADER_DG_CREATE_OBJECT		= 140,
	HEADER_DG_DELETE_OBJECT		= 141,
	HEADER_DG_UPDATE_LAND		= 142,

	HEADER_DG_MARRIAGE_ADD		= 150,
	HEADER_DG_MARRIAGE_UPDATE		= 151,
	HEADER_DG_MARRIAGE_REMOVE		= 152,

	HEADER_DG_WEDDING_REQUEST		= 153,
	HEADER_DG_WEDDING_READY		= 154,
	HEADER_DG_WEDDING_START		= 155,
	HEADER_DG_WEDDING_END		= 156,

	HEADER_DG_MYSHOP_PRICELIST_RES	= 157,		///< �������� ����Ʈ ����
	HEADER_DG_RELOAD_ADMIN = 158, 				///< ��� ���� ���ε�
	HEADER_DG_BREAK_MARRIAGE = 159,				///< ��ȥ �ı�
	HEADER_DG_ELECT_MONARCH			= 160,			///< ���� ��ǥ
	HEADER_DG_CANDIDACY				= 161,			///< ���� ���
	HEADER_DG_ADD_MONARCH_MONEY		= 162,			///< ���� �� ����
	HEADER_DG_TAKE_MONARCH_MONEY	= 163,			///< ���� �� ����
	HEADER_DG_COME_TO_VOTE			= 164,			///< ǥ��
	HEADER_DG_RMCANDIDACY			= 165,			///< �ĺ� ���� (���)
	HEADER_DG_SETMONARCH			= 166,			///<���ּ��� (���)
	HEADER_DG_RMMONARCH			= 167,			///<���ֻ���
	HEADER_DG_DEC_MONARCH_MONEY = 168,

	HEADER_DG_CHANGE_MONARCH_LORD_ACK = 169,
	HEADER_DG_UPDATE_MONARCH_INFO	= 170,
	HEADER_DG_BLOCK_COUNTRY_IP		= 171,		// ���뿪 IP-Block
	HEADER_DG_BLOCK_EXCEPTION		= 172,		// ���뿪 IP-Block ���� account

	HEADER_DG_ACK_CHANGE_GUILD_MASTER = 173,

	HEADER_DG_ACK_SPARE_ITEM_ID_RANGE = 174,

	HEADER_DG_UPDATE_HORSE_NAME 	= 175,
	HEADER_DG_ACK_HORSE_NAME		= 176,

	HEADER_DG_NEED_LOGIN_LOG		= 177,
	HEADER_DG_RESULT_CHARGE_CASH	= 179,
	HEADER_DG_ITEMAWARD_INFORMER	= 180,	//gift notify
	HEADER_DG_RESPOND_CHANNELSTATUS		= 181,
#ifdef __EVENT_MANAGER__
	HEADER_DG_UPDATE_EVENT_STATUS	= 182,
	HEADER_DG_EVENT_NOTIFICATION	= 186,
#endif
#if defined(OFFLINE_MESSAGE_REWORKED)
	HEADER_DG_RESPOND_OFFLINE_MESSAGES = 183,
#endif

#ifdef __PREMIUM_PRIVATE_SHOP__
	HEADER_DG_PRIVATE_SHOP	= 184,
#endif

#ifdef ENABLE_MOVE_CHANNEL
	HEADER_DG_CHANNEL_RESULT 		= 185,
#endif

#ifdef __DUNGEON_FOR_GUILD__
	HEADER_DG_GUILD_DUNGEON			 = 195,
	HEADER_DG_GUILD_DUNGEON_CD 		 = 196,
#endif
#ifdef ENABLE_GLOBAL_RANK
	HEADER_DG_RANKGLOBAL_LOAD_PLAYER,
	HEADER_DG_RANKGLOBAL_LOAD_ITEM,
	HEADER_DG_RANKGLOBAL_LOAD_STATE,
	HEADER_DG_RANKGLOBAL_ADD_POINT,
#endif
#if defined(__WORLD_BOSS_EVENT__)
	HEADER_DG_GET_TEMP_WORLD_BOSS_RK,
#endif
	HEADER_DG_MAP_LOCATIONS		= 0xfe,
	HEADER_DG_P2P			= 0xff,
};

/* ----------------------------------------------
 * table
 * ----------------------------------------------
 */

/* game Server -> DB Server */
#pragma pack(1)
enum ERequestChargeType
{
	ERequestCharge_Cash = 0,
	ERequestCharge_Mileage,
};

#if defined(OFFLINE_MESSAGE_REWORKED)
typedef struct
{
	char 	szName[CHARACTER_NAME_MAX_LEN + 1];
} TPacketGDReadOfflineMessage;

typedef struct
{
	char	szFrom[CHARACTER_NAME_MAX_LEN + 1];
	char	szMessage[CHAT_MAX_LEN + 1];
} TPacketDGReadOfflineMessage;

typedef struct
{
	char	szFrom[CHARACTER_NAME_MAX_LEN + 1];
	char	szTo[CHARACTER_NAME_MAX_LEN + 1];
	char	szMessage[CHAT_MAX_LEN + 1];
} TPacketGDSendOfflineMessage;
#endif

typedef struct SRequestChargeCash
{
	DWORD		dwAID;		// id(primary key) - Account Table
	DWORD		dwAmount;
	ERequestChargeType	eChargeType;

} TRequestChargeCash;

typedef struct SSimplePlayer
{
	DWORD		dwID;
	char		szName[CHARACTER_NAME_MAX_LEN + 1];
	BYTE		byJob;
	BYTE		byLevel;
	DWORD		dwPlayMinutes;
	BYTE		byST, byHT, byDX, byIQ;
	WORD		wMainPart;
	BYTE		bChangeName;
	WORD		wHairPart;
#ifdef __SASH_SYSTEM__
	WORD	wSashPart;
#endif
	BYTE		bDummy[4];
	long		x, y;
	long		lAddr;
	WORD		wPort;
	BYTE		skill_group;
#if defined(__CONQUEROR_LEVEL__)
	BYTE byConquerorLevel;
	BYTE bySungmaStr, bySungmaHp, bySungmaMove, bySungmaImmune;
#endif
} TSimplePlayer;

typedef struct SAccountTable
{
	DWORD		id;
	char		login[LOGIN_MAX_LEN + 1];
	char		passwd[PASSWD_MAX_LEN + 1];
	char		social_id[SOCIAL_ID_MAX_LEN + 1];
	char		status[ACCOUNT_STATUS_MAX_LEN + 1];
	BYTE		bEmpire;
	TSimplePlayer	players[PLAYER_PER_ACCOUNT];
#ifdef __MULTI_LANGUAGE_SYSTEM__
	BYTE bLanguage;
#endif

} TAccountTable;

typedef struct SPacketDGCreateSuccess
{
	BYTE		bAccountCharacterIndex;
	TSimplePlayer	player;
} TPacketDGCreateSuccess;

typedef struct TPlayerItemAttribute
{
	BYTE	bType;
	short	sValue;
} TPlayerItemAttribute;

typedef struct SPlayerItem
{
	DWORD	id;
	BYTE	window;
	WORD	pos;
	DWORD	count;

	DWORD	vnum;
	long	alSockets[ITEM_SOCKET_MAX_NUM];	// ���Ϲ�ȣ

	TPlayerItemAttribute    aAttr[ITEM_ATTRIBUTE_MAX_NUM];
	DWORD	owner;
#ifdef ENABLE_REFINE_ELEMENT
	DWORD	dwRefineElement;
#endif
#ifdef __CHANGELOOK_SYSTEM__
	DWORD	transmutation;
#endif
} TPlayerItem;

typedef struct SQuickslot
{
	BYTE	type;
	BYTE	pos;
} TQuickslot;

#ifdef ENABLE_FISH_EVENT
typedef struct SPlayerFishEventSlot
{
	bool	bIsMain;
	BYTE	bShape;
} TPlayerFishEventSlot;
#endif

typedef struct SPlayerSkill
{
	BYTE	bMasterType;
	BYTE	bLevel;
	time_t	tNextRead;
} TPlayerSkill;

struct	THorseInfo
{
	BYTE	bLevel;
	BYTE	bRiding;
	short	sStamina;
	short	sHealth;
	DWORD	dwHorseHealthDropTime;
};


#ifdef ENABLE_BATTLE_PASS
enum
{
	BATTLEPASS_MISSIONS_PER_PLAYER = 10,
};
enum EBattlePassMissionTypes
{
	//Example Specific - kill a dog (101)
	//Example global - kill mobs with +- levels

	MISSION_TYPE_KILL, // specific mob / global
	MISSION_TYPE_SELL, // specific item / global

	MISSION_TYPE_ITEM_DESTROY, // specific item / global
	MISSION_TYPE_ITEM_DROP, // specific item / global

	MISSION_TYPE_CRAFT, // specific item / global
	MISSION_TYPE_CHEST_OPEN, // specific chest / global

	MISSION_TYPE_USE_ITEM, // specific item / globa	l
	MISSION_TYPE_FINISH_DUNGEON, // specific dungeon / global

	//Others
	MISSION_TYPE_POLYMORPH,
	MISSION_TYPE_MESSAGES,

	MISSION_TYPE_PLAYTIME,
};

enum EBattlePassMissionTypeSpecific
{
	MISSION_TYPE_SPECIFIC,
	MISSION_TYPE_GLOBAL,
};

enum EBattlePassMissionTime
{
	MISSION_TIME_DAILY,
	MISSION_TIME_WEEKLY,
};

typedef struct BattlePassParser
{
	uint16_t id;

	uint8_t missionType;
	uint8_t missionSpecific;

	std::vector<uint32_t> vnums;
	uint32_t maxProgress;

	uint8_t timeInfo; // Dalily or weekly

	uint16_t pointsReward;

	std::string name;

} TBattlePassParser;

typedef struct PlayerBattlePass
{
	uint16_t missionID;
	uint32_t progress;

	uint8_t type;

	uint32_t endTime;

	PlayerBattlePass() : missionID(0), progress(0), type(0), endTime(0) {}
} TPlayerBattlePass;


typedef struct BattlePassStruct
{
	uint8_t maxDailyMissions;
	uint8_t maxWeeklyMissions;

	uint32_t endTime;
	std::vector<std::pair<int32_t, int32_t>> rewardsFree;
	std::vector<std::pair<int32_t, int32_t>> rewardsPremium;
} TBattlePassSettings;
#endif


typedef struct SPlayerTable
{
	DWORD	id;

	char	name[CHARACTER_NAME_MAX_LEN + 1];
	char	ip[IP_ADDRESS_LENGTH + 1];

	WORD	job;
	BYTE	voice;

	BYTE	level;
	BYTE	level_step;
	short	st, ht, dx, iq;

	DWORD	exp;
#ifdef ENABLE_REMOVE_LIMIT_GOLD
	unsigned long long		gold;
#else
	INT		gold;
#endif

#ifdef ENABLE_GAYA_SYSTEM
INT 	gaya;
#endif

	BYTE	dir;
	INT		x, y, z;
	INT		lMapIndex;

	long	lExitX, lExitY;
	long	lExitMapIndex;

	// @fixme301
	int		hp;
	int		sp;

	short	sRandomHP;
	short	sRandomSP;

	int         playtime;

	short	stat_point;
	short	skill_point;
	short	sub_skill_point;
	short	horse_skill_point;
	TPlayerSkill skills[SKILL_MAX_NUM];


	TQuickslot  quickslot[QUICKSLOT_MAX_NUM];

	BYTE	part_base;
	WORD	parts[PART_MAX_NUM];

	short	stamina;

	BYTE	skill_group;
	long	lAlignment;

	short	stat_reset_count;

	THorseInfo	horse;

	DWORD	logoff_interval;

	int		aiPremiumTimes[PREMIUM_MAX_NUM];
#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	INT 	envanter;
#endif
#ifdef ENABLE_FISH_EVENT
	DWORD	fishEventUseCount;
	TPlayerFishEventSlot fishSlots[FISH_EVENT_SLOTS_NUM];
#endif
#ifdef __HIDE_COSTUME_SYSTEM__
	DWORD	dwCostumeFlag;
#endif
#ifdef __ENABLE_PREMIUM_PLAYERS__
	BYTE premium;
	long int premium_time;
#endif
#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	BYTE secured;
	int secured_password;
#endif
#ifdef __ENABLE_BIOLOGIST_RENEWAL_SYSTEM__
	DWORD biologist_state;
	DWORD biologist_items_taken;
	DWORD biologist_completed;
#endif
#if defined(__CONQUEROR_LEVEL__)
	BYTE conqueror_level;
	BYTE conqueror_level_step;
	short sungma_str, sungma_hp, sungma_move, sungma_immune;
	DWORD conqueror_exp;
	short conqueror_point;
#endif
#ifdef ENABLE_BATTLE_PASS
	TPlayerBattlePass battlePass[BATTLEPASS_MISSIONS_PER_PLAYER];
#endif
#ifdef __PREMIUM_PRIVATE_SHOP__
	WORD	wPrivateShopUnlockedSlot;
#endif
} TPlayerTable;

typedef struct SMobSkillLevel
{
	DWORD	dwVnum;
	BYTE	bLevel;
} TMobSkillLevel;

typedef struct SEntityTable
{
	DWORD dwVnum;
} TEntityTable;

typedef struct SMobTable : public SEntityTable
{
	char	szName[CHARACTER_NAME_MAX_LEN + 1];
	char	szLocaleName[CHARACTER_NAME_MAX_LEN + 1];

	BYTE	bType;			// Monster, NPC
	BYTE	bRank;			// PAWN, KNIGHT, KING
	BYTE	bBattleType;		// MELEE, etc..
	BYTE	bLevel;			// Level
	BYTE	bSize;

	DWORD	dwGoldMin;
	DWORD	dwGoldMax;
	DWORD	dwExp;
	DWORD	dwMaxHP;
	BYTE	bRegenCycle;
	BYTE	bRegenPercent;
	WORD	wDef;

	DWORD	dwAIFlag;
	DWORD	dwRaceFlag;
	DWORD	dwImmuneFlag;

	BYTE	bStr, bDex, bCon, bInt;
	DWORD	dwDamageRange[2];

	short	sAttackSpeed;
	short	sMovingSpeed;
	BYTE	bAggresiveHPPct;
	WORD	wAggressiveSight;
	WORD	wAttackRange;

	char	cEnchants[MOB_ENCHANTS_MAX_NUM];
	char	cResists[MOB_RESISTS_MAX_NUM];

	DWORD	dwResurrectionVnum;
	DWORD	dwDropItemVnum;

	BYTE	bMountCapacity;
	BYTE	bOnClickType;

	BYTE	bEmpire;
	char	szFolder[64 + 1];

	float	fDamMultiply;

	DWORD	dwSummonVnum;
	DWORD	dwDrainSP;
	DWORD	dwMobColor;
	DWORD	dwPolymorphItemVnum;

	TMobSkillLevel Skills[MOB_SKILL_MAX_NUM];

	BYTE	bBerserkPoint;
	BYTE	bStoneSkinPoint;
	BYTE	bGodSpeedPoint;
	BYTE	bDeathBlowPoint;
	BYTE	bRevivePoint;
} TMobTable;

typedef struct SSkillTable
{
	DWORD	dwVnum;
	char	szName[32 + 1];
	BYTE	bType;
	BYTE	bMaxLevel;
	DWORD	dwSplashRange;

	char	szPointOn[64];
	char	szPointPoly[100 + 1];
	char	szSPCostPoly[100 + 1];
	char	szDurationPoly[100 + 1];
	char	szDurationSPCostPoly[100 + 1];
	char	szCooldownPoly[100 + 1];
	char	szMasterBonusPoly[100 + 1];
	//char	szAttackGradePoly[100 + 1];
	char	szGrandMasterAddSPCostPoly[100 + 1];
	DWORD	dwFlag;
	DWORD	dwAffectFlag;

	// Data for secondary skill
	char 	szPointOn2[64];
	char 	szPointPoly2[100 + 1];
	char 	szDurationPoly2[100 + 1];
	DWORD 	dwAffectFlag2;

	// Data for grand master point
	char 	szPointOn3[64];
	char 	szPointPoly3[100 + 1];
	char 	szDurationPoly3[100 + 1];

	BYTE	bLevelStep;
	BYTE	bLevelLimit;
	DWORD	preSkillVnum;
	BYTE	preSkillLevel;

	long	lMaxHit;
	char	szSplashAroundDamageAdjustPoly[100 + 1];

	BYTE	bSkillAttrType;

	DWORD	dwTargetRange;
} TSkillTable;

#if defined(ENABLE_RENEWAL_SHOPEX)
enum STableExTypes
{
	SHOPEX_GOLD = 1,
	SHOPEX_SECONDARY,
	SHOPEX_ITEM,
	SHOPEX_EXP,
	SHOPEX_MAX,
};
#endif

typedef struct SShopItemTable
{
	DWORD		vnum;
	WORD		count;

    TItemPos	pos;			// PC �������� �̿�
#ifdef ENABLE_REMOVE_LIMIT_GOLD
	unsigned long long		price;
#else
	DWORD		price;
#endif
	BYTE		display_pos; // PC, shop_table_ex.txt �������� �̿�, ���� ��ġ.
#if defined(ENABLE_RENEWAL_SHOPEX)
	long	alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute	aAttr[ITEM_ATTRIBUTE_MAX_NUM];
	BYTE 	price_type;
	DWORD 	price_vnum;
	SShopItemTable() : price_type(SHOPEX_GOLD), price_vnum(0) {
		memset(&alSockets, 0, sizeof(alSockets));
		memset(&aAttr, 0, sizeof(aAttr));
	}
#endif
} TShopItemTable;

typedef struct SShopTable
{
	DWORD		dwVnum;
	DWORD		dwNPCVnum;

	BYTE		byItemCount;
	TShopItemTable	items[SHOP_HOST_ITEM_MAX_NUM];
#if defined(ENABLE_RENEWAL_SHOPEX)
	char szShopName[SHOP_TAB_NAME_MAX + 1];
#endif
} TShopTable;

#define QUEST_NAME_MAX_LEN	32
#define QUEST_STATE_MAX_LEN	64

typedef struct SQuestTable
{
	DWORD		dwPID;
	char		szName[QUEST_NAME_MAX_LEN + 1];
	char		szState[QUEST_STATE_MAX_LEN + 1];
	long		lValue;
} TQuestTable;

typedef struct SItemLimit
{
	BYTE	bType;
	long	lValue;
} TItemLimit;

typedef struct SItemApply
{
	BYTE	bType;
	long	lValue;
} TItemApply;

typedef struct SItemTable : public SEntityTable
{
	DWORD		dwVnumRange;
	char        szName[ITEM_NAME_MAX_LEN + 1];
	char	szLocaleName[ITEM_NAME_MAX_LEN + 1];
	BYTE	bType;
	BYTE	bSubType;

	BYTE        bWeight;
	BYTE	bSize;

	DWORD	dwAntiFlags;
	DWORD	dwFlags;
	DWORD	dwWearFlags;
	DWORD	dwImmuneFlag;

#ifdef ENABLE_REMOVE_LIMIT_GOLD
	unsigned long long       dwGold;
	unsigned long long       dwShopBuyPrice;
#else
	DWORD       dwGold;
	DWORD       dwShopBuyPrice;
#endif

	TItemLimit	aLimits[ITEM_LIMIT_MAX_NUM];
	TItemApply	aApplies[ITEM_APPLY_MAX_NUM];
	long        alValues[ITEM_VALUES_MAX_NUM];
	long	alSockets[ITEM_SOCKET_MAX_NUM];
	DWORD	dwRefinedVnum;
	WORD	wRefineSet;
	BYTE	bAlterToMagicItemPct;
	BYTE	bSpecular;
	BYTE	bGainSocketPct;

	short int	sAddonType;



	char		cLimitRealTimeFirstUseIndex;
	char		cLimitTimerBasedOnWearIndex;

} TItemTable;

struct TItemAttrTable
{
	TItemAttrTable() :
		dwApplyIndex(0),
		dwProb(0)
	{
		szApply[0] = 0;
		memset(&lValues, 0, sizeof(lValues));
		memset(&bMaxLevelBySet, 0, sizeof(bMaxLevelBySet));
	}

	char    szApply[APPLY_NAME_MAX_LEN + 1];
	DWORD   dwApplyIndex;
	DWORD   dwProb;
	long    lValues[ITEM_ATTRIBUTE_MAX_LEVEL];
	BYTE    bMaxLevelBySet[ATTRIBUTE_SET_MAX_NUM];
};

typedef struct SConnectTable
{
	char	login[LOGIN_MAX_LEN + 1];
	IDENT	ident;
} TConnectTable;

typedef struct SLoginPacket
{
	char	login[LOGIN_MAX_LEN + 1];
	char	passwd[PASSWD_MAX_LEN + 1];
} TLoginPacket;

typedef struct SPlayerLoadPacket
{
	DWORD	account_id;
	DWORD	player_id;
	BYTE	account_index;	/* account ������ ��ġ */
} TPlayerLoadPacket;

typedef struct SPlayerCreatePacket
{
	char		login[LOGIN_MAX_LEN + 1];
	char		passwd[PASSWD_MAX_LEN + 1];
	DWORD		account_id;
	BYTE		account_index;
	TPlayerTable	player_table;
} TPlayerCreatePacket;

typedef struct SPlayerDeletePacket
{
	char	login[LOGIN_MAX_LEN + 1];
	DWORD	player_id;
	BYTE	account_index;
	//char	name[CHARACTER_NAME_MAX_LEN + 1];
	char	private_code[8];
} TPlayerDeletePacket;

typedef struct SLogoutPacket
{
	char	login[LOGIN_MAX_LEN + 1];
	char	passwd[PASSWD_MAX_LEN + 1];
} TLogoutPacket;

typedef struct SPlayerCountPacket
{
	DWORD	dwCount;
} TPlayerCountPacket;

#if !defined(__EXTENDED_SAFEBOX__)
#define SAFEBOX_MAX_NUM			135
#endif
#define SAFEBOX_PASSWORD_MAX_LEN	6

typedef struct SSafeboxTable
{
	DWORD	dwID;
	BYTE	bSize;
	DWORD	dwGold;
	WORD	wItemCount;
} TSafeboxTable;

typedef struct SSafeboxChangeSizePacket
{
	DWORD	dwID;
	BYTE	bSize;
} TSafeboxChangeSizePacket;

typedef struct SSafeboxLoadPacket
{
	DWORD	dwID;
	char	szLogin[LOGIN_MAX_LEN + 1];
	char	szPassword[SAFEBOX_PASSWORD_MAX_LEN + 1];
} TSafeboxLoadPacket;

typedef struct SSafeboxChangePasswordPacket
{
	DWORD	dwID;
	char	szOldPassword[SAFEBOX_PASSWORD_MAX_LEN + 1];
	char	szNewPassword[SAFEBOX_PASSWORD_MAX_LEN + 1];
} TSafeboxChangePasswordPacket;

typedef struct SSafeboxChangePasswordPacketAnswer
{
	BYTE	flag;
} TSafeboxChangePasswordPacketAnswer;

typedef struct SEmpireSelectPacket
{
	DWORD	dwAccountID;
	BYTE	bEmpire;
} TEmpireSelectPacket;

typedef struct SPacketGDSetup
{
	char	szPublicIP[16];	// Public IP which listen to users
	BYTE	bChannel;
	WORD	wListenPort;	// Ŭ���̾�Ʈ�� �����ϴ� ��Ʈ ��ȣ
	WORD	wP2PPort;	// �������� ���� ��Ű�� P2P ��Ʈ ��ȣ
	long	alMaps[MAP_ALLOW_LIMIT];
	DWORD	dwLoginCount;
	BYTE	bAuthServer;
} TPacketGDSetup;

typedef struct SPacketDGMapLocations
{
	BYTE	bCount;
} TPacketDGMapLocations;

typedef struct SMapLocation
{
	long	alMaps[MAP_ALLOW_LIMIT];
	char	szHost[MAX_HOST_LENGTH + 1];
	WORD	wPort;
} TMapLocation;

typedef struct SPacketDGP2P
{
	char	szHost[MAX_HOST_LENGTH + 1];
	WORD	wPort;
	BYTE	bChannel;
} TPacketDGP2P;

typedef struct SPacketGDDirectEnter
{
	char	login[LOGIN_MAX_LEN + 1];
	char	passwd[PASSWD_MAX_LEN + 1];
	BYTE	index;
} TPacketGDDirectEnter;

typedef struct SPacketDGDirectEnter
{
	TAccountTable accountTable;
	TPlayerTable playerTable;
} TPacketDGDirectEnter;

typedef struct SPacketGuildSkillUpdate
{
	DWORD guild_id;
	int amount;
	BYTE skill_levels[12];
	BYTE skill_point;
	BYTE save;
} TPacketGuildSkillUpdate;

typedef struct SPacketGuildExpUpdate
{
	DWORD guild_id;
	int amount;
} TPacketGuildExpUpdate;

typedef struct SPacketGuildChangeMemberData
{
	DWORD guild_id;
	DWORD pid;
	DWORD offer;
	BYTE level;
	BYTE grade;
} TPacketGuildChangeMemberData;


typedef struct SPacketDGLoginAlready
{
	char	szLogin[LOGIN_MAX_LEN + 1];
} TPacketDGLoginAlready;

typedef struct TPacketAffectElement
{
	DWORD	dwType;
	BYTE	bApplyOn;
	long	lApplyValue;
	DWORD	dwFlag;
	long	lDuration;
	long	lSPCost;
} TPacketAffectElement;

typedef struct SPacketGDAddAffect
{
	DWORD			dwPID;
	TPacketAffectElement	elem;
} TPacketGDAddAffect;

typedef struct SPacketGDRemoveAffect
{
	DWORD	dwPID;
	DWORD	dwType;
	BYTE	bApplyOn;
} TPacketGDRemoveAffect;

#ifdef ENABLE_TOP_PLAYERS_EFFECT
typedef struct SPacketGDRemoveTopPlayerInfo
{
	DWORD	dwPID;
} TPacketGDRemoveTopPlayerInfo;
#endif


typedef struct SPacketGDHighscore
{
	DWORD	dwPID;
	long	lValue;
	char	cDir;
	char	szBoard[21];
} TPacketGDHighscore;

typedef struct SPacketPartyCreate
{
	DWORD	dwLeaderPID;
} TPacketPartyCreate;

typedef struct SPacketPartyDelete
{
	DWORD	dwLeaderPID;
} TPacketPartyDelete;

typedef struct SPacketPartyAdd
{
	DWORD	dwLeaderPID;
	DWORD	dwPID;
	BYTE	bState;
} TPacketPartyAdd;

typedef struct SPacketPartyRemove
{
	DWORD	dwLeaderPID;
	DWORD	dwPID;
} TPacketPartyRemove;

typedef struct SPacketPartyStateChange
{
	DWORD	dwLeaderPID;
	DWORD	dwPID;
	BYTE	bRole;
	BYTE	bFlag;
} TPacketPartyStateChange;

typedef struct SPacketPartySetMemberLevel
{
	DWORD	dwLeaderPID;
	DWORD	dwPID;
	BYTE	bLevel;
} TPacketPartySetMemberLevel;

typedef struct SPacketGDBoot
{
    DWORD	dwItemIDRange[2];
	char	szIP[16];
} TPacketGDBoot;

typedef struct SPacketGuild
{
	DWORD	dwGuild;
	DWORD	dwInfo;
} TPacketGuild;

typedef struct SPacketGDGuildAddMember
{
	DWORD	dwPID;
	DWORD	dwGuild;
	BYTE	bGrade;
} TPacketGDGuildAddMember;

typedef struct SPacketDGGuildMember
{
	DWORD	dwPID;
	DWORD	dwGuild;
	BYTE	bGrade;
	BYTE	isGeneral;
	BYTE	bJob;
	BYTE	bLevel;
	DWORD	dwOffer;
	char	szName[CHARACTER_NAME_MAX_LEN + 1];
} TPacketDGGuildMember;

typedef struct SPacketGuildWar
{
	BYTE	bType;
	BYTE	bWar;
	DWORD	dwGuildFrom;
	DWORD	dwGuildTo;
	long	lWarPrice;
	long	lInitialScore;
} TPacketGuildWar;

// Game -> DB : ����� ��ȭ��
// DB -> Game : ��Ż�� ������
typedef struct SPacketGuildWarScore
{
	DWORD dwGuildGainPoint;
	DWORD dwGuildOpponent;
	long lScore;
	long lBetScore;
} TPacketGuildWarScore;

typedef struct SRefineMaterial
{
	DWORD vnum;
	int count;
} TRefineMaterial;

typedef struct SRefineTable
{
	//DWORD src_vnum;
	//DWORD result_vnum;
	DWORD id;
	BYTE material_count;
	int cost; // �ҿ� ���
	int prob; // Ȯ��
	TRefineMaterial materials[REFINE_MATERIAL_MAX_NUM];
} TRefineTable;

typedef struct SBanwordTable
{
	char szWord[BANWORD_MAX_LEN + 1];
} TBanwordTable;

typedef struct SPacketGDChangeName
{
	DWORD pid;
	char name[CHARACTER_NAME_MAX_LEN + 1];
} TPacketGDChangeName;

typedef struct SPacketDGChangeName
{
	DWORD pid;
	char name[CHARACTER_NAME_MAX_LEN + 1];
} TPacketDGChangeName;

typedef struct SPacketGuildLadder
{
	DWORD dwGuild;
	long lLadderPoint;
	long lWin;
	long lDraw;
	long lLoss;
} TPacketGuildLadder;

typedef struct SPacketGuildLadderPoint
{
	DWORD dwGuild;
	long lChange;
} TPacketGuildLadderPoint;

typedef struct SPacketGuildUseSkill
{
	DWORD dwGuild;
	DWORD dwSkillVnum;
	DWORD dwCooltime;
} TPacketGuildUseSkill;

typedef struct SPacketGuildSkillUsableChange
{
	DWORD dwGuild;
	DWORD dwSkillVnum;
	BYTE bUsable;
} TPacketGuildSkillUsableChange;

typedef struct SPacketGDLoginKey
{
	DWORD dwAccountID;
	DWORD dwLoginKey;
} TPacketGDLoginKey;

typedef struct SPacketGDAuthLogin
{
	DWORD	dwID;
	DWORD	dwLoginKey;
	char	szLogin[LOGIN_MAX_LEN + 1];
	char	szSocialID[SOCIAL_ID_MAX_LEN + 1];
	DWORD	adwClientKey[4];
	int		iPremiumTimes[PREMIUM_MAX_NUM];
#ifdef __MULTI_LANGUAGE_SYSTEM__
	BYTE bLanguage;
#endif
} TPacketGDAuthLogin;

typedef struct SPacketGDLoginByKey
{
	char	szLogin[LOGIN_MAX_LEN + 1];
	DWORD	dwLoginKey;
	DWORD	adwClientKey[4];
	char	szIP[MAX_HOST_LENGTH + 1];
} TPacketGDLoginByKey;

/**
 * @version 05/06/08	Bang2ni - ���ӽð� �߰�
 */
typedef struct SPacketGiveGuildPriv
{
	BYTE type;
	int value;
	DWORD guild_id;
	time_t duration_sec;	///< ���ӽð�
} TPacketGiveGuildPriv;
typedef struct SPacketGiveEmpirePriv
{
	BYTE type;
	int value;
	BYTE empire;
	time_t duration_sec;
} TPacketGiveEmpirePriv;
typedef struct SPacketGiveCharacterPriv
{
	BYTE type;
	int value;
	DWORD pid;
} TPacketGiveCharacterPriv;
typedef struct SPacketRemoveGuildPriv
{
	BYTE type;
	DWORD guild_id;
} TPacketRemoveGuildPriv;
typedef struct SPacketRemoveEmpirePriv
{
	BYTE type;
	BYTE empire;
} TPacketRemoveEmpirePriv;

typedef struct SPacketDGChangeCharacterPriv
{
	BYTE type;
	int value;
	DWORD pid;
	BYTE bLog;
} TPacketDGChangeCharacterPriv;

/**
 * @version 05/06/08	Bang2ni - ���ӽð� �߰�
 */
typedef struct SPacketDGChangeGuildPriv
{
	BYTE type;
	int value;
	DWORD guild_id;
	BYTE bLog;
	time_t end_time_sec;	///< ���ӽð�
} TPacketDGChangeGuildPriv;

typedef struct SPacketDGChangeEmpirePriv
{
	BYTE type;
	int value;
	BYTE empire;
	BYTE bLog;
	time_t end_time_sec;
} TPacketDGChangeEmpirePriv;

typedef struct SPacketMoneyLog
{
	BYTE type;
	DWORD vnum;
#ifdef ENABLE_REMOVE_LIMIT_GOLD
	unsigned long long gold;
#else
	INT gold;
#endif
} TPacketMoneyLog;

typedef struct SPacketGDGuildMoney
{
	DWORD dwGuild;
	INT iGold;
} TPacketGDGuildMoney;

typedef struct SPacketDGGuildMoneyChange
{
	DWORD dwGuild;
	INT iTotalGold;
} TPacketDGGuildMoneyChange;

typedef struct SPacketDGGuildMoneyWithdraw
{
	DWORD dwGuild;
	INT iChangeGold;
} TPacketDGGuildMoneyWithdraw;

typedef struct SPacketGDGuildMoneyWithdrawGiveReply
{
	DWORD dwGuild;
	INT iChangeGold;
	BYTE bGiveSuccess;
} TPacketGDGuildMoneyWithdrawGiveReply;

typedef struct SPacketSetEventFlag
{
	char	szFlagName[EVENT_FLAG_NAME_MAX_LEN + 1];
	long	lValue;
} TPacketSetEventFlag;

typedef struct SPacketLoginOnSetup
{
	DWORD   dwID;
	char    szLogin[LOGIN_MAX_LEN + 1];
	char    szSocialID[SOCIAL_ID_MAX_LEN + 1];
	char    szHost[MAX_HOST_LENGTH + 1];
	DWORD   dwLoginKey;
	DWORD   adwClientKey[4];
#ifdef __MULTI_LANGUAGE_SYSTEM__
	BYTE bLanguage;
#endif
#ifdef __PREMIUM_PRIVATE_SHOP__
	DWORD	dwPID;
	DWORD	dwHandle;
	bool	bHasPrivateShop;
#endif
} TPacketLoginOnSetup;

typedef struct SPacketGDCreateObject
{
	DWORD	dwVnum;
	DWORD	dwLandID;
	INT		lMapIndex;
	INT	 	x, y;
	float	xRot;
	float	yRot;
	float	zRot;
} TPacketGDCreateObject;

typedef struct SPacketGDHammerOfTor
{
	DWORD 	key;
	DWORD	delay;
} TPacketGDHammerOfTor;

typedef struct SGuildReserve
{
	DWORD       dwID;
	DWORD       dwGuildFrom;
	DWORD       dwGuildTo;
	DWORD       dwTime;
	BYTE        bType;
	long        lWarPrice;
	long        lInitialScore;
	bool        bStarted;
	DWORD	dwBetFrom;
	DWORD	dwBetTo;
	long	lPowerFrom;
	long	lPowerTo;
	long	lHandicap;
} TGuildWarReserve;

typedef struct
{
	DWORD	dwWarID;
	char	szLogin[LOGIN_MAX_LEN + 1];
	DWORD	dwGold;
	DWORD	dwGuild;
} TPacketGDGuildWarBet;

// Marriage

typedef struct
{
	DWORD dwPID1;
	DWORD dwPID2;
	time_t tMarryTime;
	char szName1[CHARACTER_NAME_MAX_LEN + 1];
	char szName2[CHARACTER_NAME_MAX_LEN + 1];
} TPacketMarriageAdd;

typedef struct
{
	DWORD dwPID1;
	DWORD dwPID2;
	INT  iLovePoint;
	BYTE  byMarried;
} TPacketMarriageUpdate;

typedef struct
{
	DWORD dwPID1;
	DWORD dwPID2;
} TPacketMarriageRemove;

typedef struct
{
	DWORD dwPID1;
	DWORD dwPID2;
} TPacketWeddingRequest;

typedef struct
{
	DWORD dwPID1;
	DWORD dwPID2;
	DWORD dwMapIndex;
} TPacketWeddingReady;

typedef struct
{
	DWORD dwPID1;
	DWORD dwPID2;
} TPacketWeddingStart;

typedef struct
{
	DWORD dwPID1;
	DWORD dwPID2;
} TPacketWeddingEnd;

/// ���λ��� ���������� ���. ���� ��Ŷ���� �� �ڿ� byCount ��ŭ�� TItemPriceInfo �� �´�.
typedef struct SPacketMyshopPricelistHeader
{
	DWORD	dwOwnerID;	///< ���������� ���� �÷��̾� ID
	BYTE	byCount;	///< �������� ����
} TPacketMyshopPricelistHeader;

/// ���λ����� ���� �����ۿ� ���� ��������
typedef struct SItemPriceInfo
{
	DWORD	dwVnum;		///< ������ vnum
#ifdef ENABLE_REMOVE_LIMIT_GOLD
	unsigned long long	dwPrice;
#else
	DWORD	dwPrice;
#endif
} TItemPriceInfo;

/// ���λ��� ������ �������� ����Ʈ ���̺�
typedef struct SItemPriceListTable
{
	DWORD	dwOwnerID;	///< ���������� ���� �÷��̾� ID
	BYTE	byCount;	///< �������� ����Ʈ�� ����

	TItemPriceInfo	aPriceInfo[SHOP_PRICELIST_MAX_NUM];	///< �������� ����Ʈ
} TItemPriceListTable;

typedef struct
{
	char szName[CHARACTER_NAME_MAX_LEN + 1];
	long lDuration;
} TPacketBlockChat;

//ADMIN_MANAGER
typedef struct TAdminInfo
{
	int m_ID;				//����ID
	char m_szAccount[32];	//����
	char m_szName[32];		//ĳ�����̸�
	char m_szContactIP[16];	//���پ�����
	char m_szServerIP[16];  //����������
	int m_Authority;		//����
} tAdminInfo;
//END_ADMIN_MANAGER

//BOOT_LOCALIZATION
struct tLocale
{
	char szValue[32];
	char szKey[32];
};
//BOOT_LOCALIZATION

//RELOAD_ADMIN
typedef struct SPacketReloadAdmin
{
	char szIP[16];
} TPacketReloadAdmin;
//END_RELOAD_ADMIN

typedef struct TMonarchInfo
{
	DWORD pid[4];  // ������ PID
	int64_t money[4];  // ������ ���� ��
	char name[4][32];  // ������ �̸�
	char date[4][32];  // ���� ��� ��¥
} MonarchInfo;

typedef struct TMonarchElectionInfo
{
	DWORD pid;  // ��ǥ �ѻ�� PID
	DWORD selectedpid; // ��ǥ ���� PID ( ���� ������ )
	char date[32]; // ��ǥ ��¥
} MonarchElectionInfo;

// ���� �⸶��
typedef struct tMonarchCandidacy
{
	DWORD pid;
	char name[32];
	char date[32];
} MonarchCandidacy;

typedef struct tChangeMonarchLord
{
	BYTE bEmpire;
	DWORD dwPID;
} TPacketChangeMonarchLord;

typedef struct tChangeMonarchLordACK
{
	BYTE bEmpire;
	DWORD dwPID;
	char szName[32];
	char szDate[32];
} TPacketChangeMonarchLordACK;

// Block Country Ip
typedef struct tBlockCountryIp
{
	DWORD	ip_from;
	DWORD	ip_to;
} TPacketBlockCountryIp;

enum EBlockExceptionCommand
{
	BLOCK_EXCEPTION_CMD_ADD = 1,
	BLOCK_EXCEPTION_CMD_DEL = 2,
};

// Block Exception Account
typedef struct tBlockException
{
	BYTE	cmd;	// 1 == add, 2 == delete
	char	login[LOGIN_MAX_LEN + 1];
}TPacketBlockException;

typedef struct tChangeGuildMaster
{
	DWORD dwGuildID;
	DWORD idFrom;
	DWORD idTo;
} TPacketChangeGuildMaster;

typedef struct tItemIDRange
{
	DWORD dwMin;
	DWORD dwMax;
	DWORD dwUsableItemIDMin;
} TItemIDRangeTable;

typedef struct tUpdateHorseName
{
	DWORD dwPlayerID;
	char szHorseName[CHARACTER_NAME_MAX_LEN + 1];
} TPacketUpdateHorseName;

typedef struct tDC
{
	char	login[LOGIN_MAX_LEN + 1];
} TPacketDC;

typedef struct tNeedLoginLogInfo
{
	DWORD dwPlayerID;
#ifdef __MULTI_LANGUAGE_SYSTEM__
	BYTE bLanguage;
#endif
} TPacketNeedLoginLogInfo;

//���� ���� �˸� ��� �׽�Ʈ�� ��Ŷ ����
typedef struct tItemAwardInformer
{
	char	login[LOGIN_MAX_LEN + 1];
	char	command[20];		//���ɾ�
	unsigned int vnum;			//������
} TPacketItemAwardInfromer;
// ���� �˸� ��� ������ ��Ŷ ����
typedef struct tDeleteAwardID
{
	DWORD dwID;
} TPacketDeleteAwardID;

typedef struct SChannelStatus
{
	short nPort;
	BYTE bStatus;
} TChannelStatus;

#ifdef ENABLE_MOVE_CHANNEL
typedef struct
{
	long lMapIndex;
	int iChannel;
} TPacketChangeChannel;

typedef struct
{
	long lAddr;
	WORD wPort;
} TPacketReturnChannel;
#endif

#ifdef OFFLINE_SHOP
typedef struct SShopPrice
{
	int days;
	int time;
	DWORD price;
} TShopCost;

typedef struct command_shop_name
{
	DWORD shop_id;
	char szSign[SHOP_SIGN_MAX_LEN + 1];
} TPacketShopName;
typedef struct command_shop_close
{
	DWORD shop_id;
	DWORD pid;
	bool error;
	bool reload;
} TPacketShopClose;

typedef struct command_shop_update_item
{
	DWORD shop_id;
	bool	tick;
	bool	shop_locked;
	bool	refresh;
} TPacketShopUpdateItem;
#endif
#ifdef ENABLE_MULTI_FARM_BLOCK
typedef struct SMultiFarm
{
	uint32_t playerID;
	bool farmStatus;
	uint8_t affectType;
	int affectTime;
	char playerName[CHARACTER_NAME_MAX_LEN + 1];
	SMultiFarm(uint32_t id_, const char* playerName_, bool status_, uint8_t type_, int time_) : playerID(id_), farmStatus(status_), affectType(type_), affectTime(time_) {
		strlcpy(playerName, playerName_, sizeof(playerName));
	}
}TMultiFarm;
#endif
#if defined(__ATTR_6TH_7TH__)
typedef struct SAttr67AddData
{
	SAttr67AddData() : wRegistItemPos(0), byMaterialCount(0), wSupportItemPos(0), bySupportItemCount(0) {}
	WORD wRegistItemPos;
	BYTE byMaterialCount;
	WORD wSupportItemPos;
	BYTE bySupportItemCount;
} TAttr67AddData;
#endif
#ifdef __DUNGEON_FOR_GUILD__
typedef struct SPacketGDGuildDungeon
{
	DWORD	dwGuildID;
	BYTE	bChannel;
	long	lMapIndex;
} TPacketGDGuildDungeon;

typedef struct SPacketDGGuildDungeon
{
	DWORD	dwGuildID;
	BYTE	bChannel;
	long	lMapIndex;
} TPacketDGGuildDungeon;

typedef struct SPacketGDGuildDungeonCD
{
	DWORD	dwGuildID;
	DWORD	dwTime;
} TPacketGDGuildDungeonCD;

typedef struct SPacketDGGuildDungeonCD
{
	DWORD	dwGuildID;
	DWORD	dwTime;
} TPacketDGGuildDungeonCD;
#endif

#ifdef ENABLE_ITEMSHOP
enum
{
	ITEMSHOP_LOAD,
	ITEMSHOP_LOG,
	ITEMSHOP_BUY,
	ITEMSHOP_DRAGONCOIN,
	ITEMSHOP_RELOAD,
	ITEMSHOP_LOG_ADD,
	ITEMSHOP_UPDATE_ITEM,
};
typedef struct SIShopData
{
	DWORD	id;
	DWORD	itemVnum;
	long long	itemPrice;
	int		topSellingIndex;
	BYTE	discount;
	int		offerTime;
	int		addedTime;
	long long	sellCount;
	int	week_limit;
	int	month_limit;
	int maxSellCount;
}TIShopData;
typedef struct SIShopLogData
{
	DWORD	accountID;
	char	playerName[CHARACTER_NAME_MAX_LEN+1];
	char	buyDate[21];
	int		buyTime;
	char	ipAdress[16];
	DWORD	itemID;
	DWORD	itemVnum;
	int		itemCount;
	long long	itemPrice;
}TIShopLogData;
#endif

#ifdef __MULTI_LANGUAGE_SYSTEM__
typedef struct SRequestChangeLanguage
{
	DWORD dwAID;
	BYTE bLanguage;
} TRequestChangeLanguage;
#endif

#ifdef __STAGE_EVENT__
typedef struct TStageInfo
{
	BYTE	missionIdx, rewardIdx;
	long long	argument[3], rewardData[3];
	TStageInfo() : missionIdx(0), rewardIdx(0) {
		memset(&argument, 0, sizeof(argument));
		memset(&rewardData, 0, sizeof(rewardData));
	}
}TStageInfo;

typedef struct SStageEvent
{
	BYTE	id;
	BYTE	stageType;
	TStageInfo	stage[4];
	//int	start, end;
}TStageEvent;

#endif

#ifdef __DUNGEON_INFO__
typedef struct SDungeonRank
{
	char name[CHARACTER_NAME_MAX_LEN + 1];
	BYTE level;
	int	value;
}TDungeonRank;
#endif

#ifdef ENABLE_GLOBAL_RANK
struct stRankGlobal_player
{
	DWORD pid;
	char name[CHARACTER_NAME_MAX_LEN + 1];
	int count[9];
	int lv;
	BYTE empire;

	stRankGlobal_player()
	{
		memset(this, 0, sizeof(stRankGlobal_player));
	}
};

struct stRankGlobal_player_sort
{
	DWORD pid;
	char name[CHARACTER_NAME_MAX_LEN + 1];
	BYTE type;
	int count;
	int lv;
	BYTE empire;
};

struct stRankGlobal_item
{
	BYTE type;
	int pos;
	int item_id;
	int item_count;
};

struct stRankGlobal_state
{
	BYTE option;
};
#endif

#ifdef __PREMIUM_PRIVATE_SHOP__
typedef struct SPrivateShop
{
	DWORD				dwOwner;
	char				szTitle[TITLE_MAX_LEN + 1];
	char				szOwnerName[CHARACTER_NAME_MAX_LEN + 1];
	BYTE				bState;

	DWORD				dwVnum;
	BYTE				bTitleType;

	long				lX;
	long				lY;
	long				lMapIndex;
	BYTE				bChannel;
	WORD				wPort;

	long long			llGold;
	DWORD				dwCheque;
	BYTE				bPageCount;
	time_t				tPremiumTime;
	WORD				wUnlockedSlots;
} TPrivateShop;

typedef struct SItemPrice
{
	long long	llGold;
	DWORD		dwCheque;
} TItemPrice;

typedef struct SPlayerPrivateShopItem
{
	DWORD					dwID;
	WORD					wPos;
	DWORD					dwCount;

	DWORD					dwVnum;
	long					alSockets[ITEM_SOCKET_MAX_NUM];

	TPlayerItemAttribute    aAttr[ITEM_ATTRIBUTE_MAX_NUM];
	time_t					tCheckin;

	TItemPrice				TPrice;
	DWORD					dwOwner;

#ifdef ENABLE_PET_GROWTH_SYSTEM
	TPetGrowthCache pet;
#endif

#ifdef ENABLE_PRIVATE_SHOP_CHANGE_LOOK
    DWORD dwTransmutationVnum;
#endif
#ifdef ENABLE_PRIVATE_SHOP_REFINE_ELEMENT
    DWORD dwRefineElement;
#endif
#ifdef ENABLE_PRIVATE_SHOP_APPLY_RANDOM
    TPlayerItemAttribute aApplyRandom[ITEM_APPLY_MAX_NUM];
#endif
} TPlayerPrivateShopItem;

typedef struct SPrivateShopSale
{
	DWORD		dwID;
	DWORD		dwOwner;
	DWORD		dwCustomer;
	char		szCustomerName[CHARACTER_NAME_MAX_LEN + 1];
	time_t		tTime;
	TPlayerPrivateShopItem	TItem;

	void AssignID()
	{
		size_t seededID = 0;

		// Both time of sale and sale item's id are a unique pair
		boost::hash_combine(seededID, tTime);
		boost::hash_combine(seededID, TItem.dwID);

		dwID = seededID;
	}
} TPrivateShopSale;

typedef struct SMarketItemPrice
{
	DWORD dwVnum;
	TItemPrice TPrice;
} TMarketItemPrice;

/* Game -> Database */
enum EPrivateShopGDSubheader
{
	PRIVATE_SHOP_GD_SUBHEADER_LOGOUT,
	PRIVATE_SHOP_GD_SUBHEADER_CREATE,
	PRIVATE_SHOP_GD_SUBHEADER_CLOSE,
	PRIVATE_SHOP_GD_SUBHEADER_DELETE,
	PRIVATE_SHOP_GD_SUBHEADER_DESPAWN,
	PRIVATE_SHOP_GD_SUBHEADER_WITHDRAW_REQUEST,
	PRIVATE_SHOP_GD_SUBHEADER_MODIFY_REQUEST,
	PRIVATE_SHOP_GD_SUBHEADER_BUY_REQUEST,
	PRIVATE_SHOP_GD_SUBHEADER_ITEM_PRICE_CHANGE_REQUEST,
	PRIVATE_SHOP_GD_SUBHEADER_ITEM_MOVE_REQUEST,
	PRIVATE_SHOP_GD_SUBHEADER_ITEM_CHECKIN_REQUEST,
	PRIVATE_SHOP_GD_SUBHEADER_ITEM_CHECKOUT_REQUEST,
	PRIVATE_SHOP_GD_SUBHEADER_TITLE_CHANGE_REQUEST,
	PRIVATE_SHOP_GD_SUBHEADER_WARP_REQUEST,
	PRIVATE_SHOP_GD_SUBHEADER_SLOT_UNLOCK_REQUEST,
	PRIVATE_SHOP_GD_SUBHEADER_WITHDRAW,
	PRIVATE_SHOP_GD_SUBHEADER_BUY,
	PRIVATE_SHOP_GD_SUBHEADER_FAILED_BUY,
	PRIVATE_SHOP_GD_SUBHEADER_ITEM_CHECKIN_UPDATE,
	PRIVATE_SHOP_GD_SUBHEADER_ITEM_CHECKOUT_UPDATE,
	PRIVATE_SHOP_GD_SUBHEADER_ITEM_TRANSFER,
	PRIVATE_SHOP_GD_SUBHEADER_ITEM_DELETE,
	PRIVATE_SHOP_GD_SUBHEADER_ITEM_EXPIRE,
	PRIVATE_SHOP_GD_SUBHEADER_PREMIUM_TIME_UPDATE,
	PRIVATE_SHOP_GD_SUBHEADER_INIT,
};

typedef struct SSelectedItem
{
	DWORD		dwShopID;
	WORD		wPos;
	TItemPrice	TPrice;
} TSelectedItem;

typedef struct SPacketGDPrivateShopBuyRequest
{
	DWORD			dwCustomerPID;
	long long		llGoldBalance;
	DWORD			dwChequeBalance;
	TSelectedItem	aSelectedItems[SELECTED_ITEM_MAX_NUM];
} TPacketGDPrivateShopBuyRequest;

typedef struct SPacketGDPrivateShopItemCheckin
{
	DWORD					dwShopID;
	TPlayerPrivateShopItem	TItem;
	int						iPos;
} TPacketGDPrivateShopItemCheckin;

typedef struct SPacketGDPrivateShopItemCheckout
{
	DWORD		dwPID;
	WORD		wSrcPos;
	TItemPos	TDstPos;
	TPlayerPrivateShopItem	TItem;
} TPacketGDPrivateShopItemCheckout;

typedef struct SPacketGDPrivateShopBuy
{
	TPlayerPrivateShopItem	TItem;
	DWORD					dwCustomer;
	char					szCustomerName[CHARACTER_NAME_MAX_LEN + 1];
	time_t					tTime;
} TPacketGDPrivateShopBuy;

typedef struct SPacketGDPrivateShopFailedBuy
{
	DWORD		dwShopID;
	WORD		wPos;
} TPacketGDPrivateShopFailedBuy;

typedef struct SPacketGDPrivateShopItemDelete
{
	DWORD		dwShopID;
	DWORD		dwItemID;
} TPacketGDPrivateShopItemDelete;

typedef struct SPacketGDPrivateShopItemExpire
{
	DWORD		dwShopID;
	WORD		wPos;
} TPacketGDPrivateShopItemExpire;


typedef struct SPacketGDPrivateShopPremiumTimeUpdate
{
	DWORD		dwAID;
	DWORD		dwPID;
	time_t		tPremiumTime;
} TPacketGDPrivateShopPremiumTimeUpdate;

/* Database -> Game */
enum EPrivateShopDGSubheader
{
	PRIVATE_SHOP_DG_SUBHEADER_CREATE_RESULT,
	PRIVATE_SHOP_DG_SUBHEADER_NO_SHOP,
	PRIVATE_SHOP_DG_SUBHEADER_CLOSE_RESULT_BALANCE_AVAILABLE,
	PRIVATE_SHOP_DG_SUBHEADER_CLOSE,
	PRIVATE_SHOP_DG_SUBHEADER_SPAWN,
	PRIVATE_SHOP_DG_SUBHEADER_DESTROY,
	PRIVATE_SHOP_DG_SUBHEADER_DESPAWN,
	PRIVATE_SHOP_DG_SUBHEADER_LOAD,
	PRIVATE_SHOP_DG_SUBHEADER_ITEM_LOAD,
	PRIVATE_SHOP_DG_SUBHEADER_SALE_LOAD,
	PRIVATE_SHOP_DG_SUBHEADER_BUY_RESULT_FALSE_ITEM,
	PRIVATE_SHOP_DG_SUBHEADER_BUY_RESULT_FALSE_PRICE,
	PRIVATE_SHOP_DG_SUBHEADER_BUY_RESULT_MODIFY_STATE,
	PRIVATE_SHOP_DG_SUBHEADER_BUY_RESULT_NO_GOLD,
	PRIVATE_SHOP_DG_SUBHEADER_BUY_RESULT_NO_CHEQUE,
	PRIVATE_SHOP_DG_SUBHEADER_BUY_REQUEST,
	PRIVATE_SHOP_DG_SUBHEADER_REMOVE_ITEM,
	PRIVATE_SHOP_DG_SUBHEADER_ADD_ITEM,
	PRIVATE_SHOP_DG_SUBHEADER_SALE_UPDATE,
	PRIVATE_SHOP_DG_SUBHEADER_STATE_UPDATE,
	PRIVATE_SHOP_DG_SUBHEADER_WITHDRAW_RESULT_NO_BALANCE,
	PRIVATE_SHOP_DG_SUBHEADER_WITHDRAW,
	PRIVATE_SHOP_DG_SUBHEADER_NOT_MODIFY_STATE,
	PRIVATE_SHOP_DG_SUBHEADER_ITEM_PRICE_CHANGE,
	PRIVATE_SHOP_DG_SUBHEADER_ITEM_MOVE,
	PRIVATE_SHOP_DG_SUBHEADER_CANNOT_MOVE_ITEM,
	PRIVATE_SHOP_DG_SUBHEADER_ITEM_CHECKIN_REQ,
	PRIVATE_SHOP_DG_SUBHEADER_ITEM_CHECKIN_FALSE_ITEM,
	PRIVATE_SHOP_DG_SUBHEADER_ITEM_CHECKOUT_REQ,
	PRIVATE_SHOP_DG_SUBHEADER_ITEM_EXPIRE,
	PRIVATE_SHOP_DG_SUBHEADER_SHOP_NOT_AVAILABLE,
	PRIVATE_SHOP_DG_SUBHEADER_TITLE_CHANGE,
	PRIVATE_SHOP_DG_SUBHEADER_WARP,
	PRIVATE_SHOP_DG_SUBHEADER_UNLOCK_SLOT_RES,
	PRIVATE_SHOP_DG_SUBHEADER_NO_AVAILABLE_SPACE,
	PRIVATE_SHOP_DG_SUBHEADER_MARKET_ITEM_PRICE_DATA_UPDATE,
};

typedef struct SPacketDGPrivateShopCreateResult
{
	TPrivateShop		privateShopTable;
	bool				bSuccess;
} TPacketDGPrivateShopCreateResult;

typedef struct SPacketDGPrivateShopBuyRequest
{
	DWORD					dwCustomerPID;
	TPlayerPrivateShopItem	arRequestedItems[SELECTED_ITEM_MAX_NUM];
} TPacketDGPrivateShopBuyRequest;

typedef struct SPacketDGPrivateShopStateUpdate
{
	DWORD		dwPID;
	BYTE		bState;
} TPacketDGPrivateShopStateUpdate;

typedef struct SPacketDGPrivateShopWithdraw
{
	long long	llGold;
	DWORD		dwCheque;
} TPacketDGPrivateShopWithdraw;

typedef struct SPacketDGPrivateShopItemCheckin
{
	DWORD		dwPID;
	TPlayerPrivateShopItem	TItem;
} TPacketDGPrivateShopItemCheckin;

typedef struct SPacketDGPrivateShopItemCheckout
{
	DWORD		dwPID;
	WORD		wSrcPos;
	TItemPos	TDstPos;
} TPacketDGPrivateShopItemCheckout;

/* Database <-> Game */
typedef struct SPacketPrivateShopItemMove
{
	DWORD		dwShopID;
	WORD		wPos;
	WORD		wChangePos;
} TPacketPrivateShopItemMove;

typedef struct SPacketPrivateShopItemPriceChange
{
	DWORD		dwShopID;
	WORD		wPos;
	TItemPrice	TPrice;
} TPacketPrivateShopItemPriceChange;

typedef struct SPacketPrivateShopTitleChange
{
	DWORD		dwPID;
	char		szTitle[SHOP_SIGN_MAX_LEN + 1];
} TPacketPrivateShopTitleChange;

typedef struct SPacketGDPrivateShopWarpReq
{
	DWORD		dwPID;
	DWORD		dwMapIndex;
	WORD		wListenPort;
	BYTE		bChannel;
} TPacketGDPrivateShopWarpReq;

typedef struct SPacketDGPrivateShopWarp
{
	WORD		wListenPort;
	long		lAddr;
} TPacketDGPrivateShopWarp;

typedef struct SPacketGDPrivateShopSlotUnlockReq
{
	DWORD		dwPID;
	WORD		wCount;
} TPacketGDPrivateShopSlotUnlockReq;

typedef struct SPacketGDPrivateShopSlotUnlockRes
{
	DWORD		dwShopID;
	WORD		wUnlockedSlots;
} TPacketGDPrivateShopSlotUnlockRes;
#endif


#ifdef ENABLE_SWITCHBOT
struct TSwitchbotAttributeAlternativeTable
{
	TPlayerItemAttribute attributes[MAX_NORM_ATTR_NUM];

	bool IsConfigured() const
	{
		for (const auto& it : attributes)
		{
			if (it.bType && it.sValue)
			{
				return true;
			}
		}

		return false;
	}
};

struct TSwitchbotTable
{
	DWORD player_id;
	bool active[SWITCHBOT_SLOT_COUNT];
	bool finished[SWITCHBOT_SLOT_COUNT];
	DWORD items[SWITCHBOT_SLOT_COUNT];
	TSwitchbotAttributeAlternativeTable alternatives[SWITCHBOT_SLOT_COUNT][SWITCHBOT_ALTERNATIVE_COUNT];

	TSwitchbotTable() : player_id(0)
	{
		memset(&items, 0, sizeof(items));
		memset(&alternatives, 0, sizeof(alternatives));
		memset(&active, false, sizeof(active));
		memset(&finished, false, sizeof(finished));
	}
};

struct TSwitchbottAttributeTable
{
	BYTE attribute_set;
	int apply_num;
	long max_value;
};
#endif

#ifdef __EVENT_MANAGER__
typedef struct SEventTable
{
	DWORD	dwID;
	char	szType[64];
	long	startTime;
	long	endTime;
	int		iValue0;
	int		iValue1;
	bool	bCompleted;
} TEventTable;
#endif

#if defined(__WORLD_BOSS_EVENT__)
typedef struct SPacketGDTempWorldBossRanking
{
	char szPlayerName[CHARACTER_NAME_MAX_LEN + 1];
	char szGuildName[GUILD_NAME_MAX_LEN + 1];
	BYTE bEmpire;
	DWORD dwRecord;
} TPacketGDTempWorldBossRanking;

typedef struct SPacketGDWorldBossRanking
{
	SPacketGDWorldBossRanking() :
		dwPID(0),
		dwGuildID(0),
		bEmpire(0),
		dwRecord(0),
		dwStartTime(0),
		bFlush(false)
	{}
	DWORD dwPID;
	DWORD dwGuildID;
	BYTE bEmpire;
	DWORD dwRecord;
	DWORD dwStartTime;
	bool bFlush;
} TPacketGDWorldBossRanking;
#endif






#pragma pack()
#endif
