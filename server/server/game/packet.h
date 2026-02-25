#ifndef __INC_PACKET_H__
#define __INC_PACKET_H__
#include "stdafx.h"
#include <vector>

enum
{
	HEADER_CG_HANDSHAKE							= 0xff,
	HEADER_CG_PONG								= 0xfe,
	HEADER_CG_TIME_SYNC							= 0xfc,
	HEADER_CG_KEY_AGREEMENT						= 0xfb, 
	HEADER_CG_LOGIN								= 1,
	HEADER_CG_ATTACK							= 2,
	HEADER_CG_CHAT								= 3,
	HEADER_CG_CHARACTER_CREATE					= 4,
	HEADER_CG_CHARACTER_DELETE					= 5,
	HEADER_CG_CHARACTER_SELECT					= 6,
	HEADER_CG_MOVE								= 7,
	HEADER_CG_SYNC_POSITION						= 8,
	HEADER_CG_ENTERGAME							= 10,
	HEADER_CG_ITEM_USE							= 11,
	HEADER_CG_ITEM_DROP							= 12,
	HEADER_CG_ITEM_MOVE							= 13,
	HEADER_CG_ITEM_PICKUP						= 15,
	HEADER_CG_QUICKSLOT_ADD						= 16,
	HEADER_CG_QUICKSLOT_DEL						= 17,
	HEADER_CG_QUICKSLOT_SWAP					= 18,
	HEADER_CG_WHISPER							= 19,
	HEADER_CG_ITEM_DROP2						= 20,
	HEADER_CG_ITEM_DESTROY						= 21,
#ifdef ENABLE_SELL_ITEM
	HEADER_CG_ITEM_SELL							= 22,
#endif	
	HEADER_CG_ON_CLICK							= 26,
	HEADER_CG_EXCHANGE							= 27,
	HEADER_CG_CHARACTER_POSITION				= 28,
	HEADER_CG_SCRIPT_ANSWER						= 29,
	HEADER_CG_QUEST_INPUT_STRING				= 30,
	HEADER_CG_QUEST_CONFIRM						= 31,
	HEADER_CG_REQUEST_EVENT_QUEST				= 32,
	HEADER_CG_SHOP								= 50,
	HEADER_CG_FLY_TARGETING						= 51,
	HEADER_CG_USE_SKILL							= 52,
	HEADER_CG_ADD_FLY_TARGETING					= 53,
	HEADER_CG_SHOOT								= 54,
	HEADER_CG_MYSHOP							= 55,
	HEADER_CG_ITEM_USE_TO_ITEM					= 60,
	HEADER_CG_TARGET			 				= 61,
	HEADER_CG_TEXT								= 64,
	HEADER_CG_WARP								= 65,
	HEADER_CG_SCRIPT_BUTTON						= 66,
	HEADER_CG_MESSENGER							= 67,
	HEADER_CG_MALL_CHECKOUT						= 69,
	HEADER_CG_SAFEBOX_CHECKIN					= 70,
	HEADER_CG_SAFEBOX_CHECKOUT					= 71,
	HEADER_CG_PARTY_INVITE						= 72,
	HEADER_CG_PARTY_INVITE_ANSWER				= 73,
	HEADER_CG_PARTY_REMOVE						= 74,
	HEADER_CG_PARTY_SET_STATE                   = 75,
	HEADER_CG_PARTY_USE_SKILL					= 76,
	HEADER_CG_SAFEBOX_ITEM_MOVE					= 77,
	HEADER_CG_PARTY_PARAMETER					= 78,
	HEADER_CG_GUILD								= 80,
	HEADER_CG_ANSWER_MAKE_GUILD					= 81,
	HEADER_CG_FISHING							= 82,
	HEADER_CG_ITEM_GIVE							= 83,
	HEADER_CG_EMPIRE							= 90,
	HEADER_CG_REFINE							= 96,
	HEADER_CG_MARK_LOGIN						= 100,
	HEADER_CG_MARK_CRCLIST						= 101,
	HEADER_CG_MARK_UPLOAD						= 102,
	HEADER_CG_MARK_IDXLIST						= 104,
	HEADER_CG_HACK								= 105,
	HEADER_CG_CHANGE_NAME						= 106,
	HEADER_CG_LOGIN2							= 109,
	HEADER_CG_DUNGEON							= 110,
	HEADER_CG_LOGIN3							= 111,
	HEADER_CG_GUILD_SYMBOL_UPLOAD				= 112,
	HEADER_CG_SYMBOL_CRC						= 113,
	HEADER_CG_SCRIPT_SELECT_ITEM				= 114,
	HEADER_CG_REQUEST_EVENT_DATA				= 117,
	HEADER_CG_SWITCHBOT							= 171,
	HEADER_CG_AURA								= 172,
	HEADER_CG_DAILY_GIFT 						= 180,
	HEADER_CG_DRAGON_SOUL_REFINE				= 205,
	HEADER_CG_STATE_CHECKER						= 206,
	HEADER_CG_CUBE_RENEWAL 						= 220,
	ENVANTER_BLACK		            			= 226,
	HEADER_CG_PRIVATE_SHOP						= 236,
	HEADER_CG_CHANGE_LANGUAGE 					= 238,
	HEADER_CG_WHISPER_DETAILS 					= 239,
    HEADER_CG_GAYA_SYSTEM 						= 241,
	HEADER_CG_CLIENT_VERSION					= 0xfd,
	HEADER_CG_CLIENT_VERSION2					= 0xf1,
	HEADER_GC_KEY_AGREEMENT_COMPLETED 			= 0xfa, 
	HEADER_GC_KEY_AGREEMENT						= 0xfb,
	HEADER_GC_TIME_SYNC							= 0xfc,
	HEADER_GC_PHASE								= 0xfd,
	HEADER_GC_BINDUDP							= 0xfe,
	HEADER_GC_HANDSHAKE							= 0xff,
	HEADER_GC_CHARACTER_ADD						= 1,
	HEADER_GC_CHARACTER_DEL						= 2,
	HEADER_GC_MOVE								= 3,
	HEADER_GC_CHAT								= 4,
	HEADER_GC_SYNC_POSITION						= 5,
	HEADER_GC_LOGIN_SUCCESS						= 6,
	HEADER_GC_LOGIN_FAILURE						= 7,
	HEADER_GC_CHARACTER_CREATE_SUCCESS			= 8,
	HEADER_GC_CHARACTER_CREATE_FAILURE			= 9,
	HEADER_GC_CHARACTER_DELETE_SUCCESS			= 10,
	HEADER_GC_CHARACTER_DELETE_WRONG_SOCIAL_ID	= 11,
	HEADER_GC_ATTACK							= 12,
	HEADER_GC_STUN								= 13,
	HEADER_GC_DEAD								= 14,
	HEADER_GC_MAIN_CHARACTER_OLD				= 15,
	HEADER_GC_CHARACTER_POINTS					= 16,
	HEADER_GC_CHARACTER_POINT_CHANGE			= 17,
	HEADER_GC_CHANGE_SPEED						= 18,
	HEADER_GC_CHARACTER_UPDATE					= 19,
	HEADER_GC_CHARACTER_UPDATE_NEW				= 24,
	HEADER_GC_ITEM_DEL							= 20,
	HEADER_GC_ITEM_SET							= 21,
	HEADER_GC_ITEM_USE							= 22,
	HEADER_GC_ITEM_DROP							= 23,
	HEADER_GC_ITEM_UPDATE						= 25,
	HEADER_GC_ITEM_GROUND_ADD					= 26,
	HEADER_GC_ITEM_GROUND_DEL					= 27,
	HEADER_GC_QUICKSLOT_ADD						= 28,
	HEADER_GC_QUICKSLOT_DEL						= 29,
	HEADER_GC_QUICKSLOT_SWAP					= 30,
	HEADER_GC_ITEM_OWNERSHIP					= 31,
	HEADER_GC_LOGIN_SUCCESS_NEWSLOT				= 32,
	HEADER_GC_WHISPER							= 34,
	HEADER_GC_MOTION							= 36,
	HEADER_GC_PARTS								= 37,
	HEADER_GC_SHOP								= 38,
	HEADER_GC_SHOP_SIGN							= 39,
	HEADER_GC_DUEL_START						= 40,
	HEADER_GC_PVP                               = 41,
	HEADER_GC_EXCHANGE							= 42,
	HEADER_GC_CHARACTER_POSITION				= 43,
	HEADER_GC_PING								= 44,
	HEADER_GC_SCRIPT							= 45,
	HEADER_GC_QUEST_CONFIRM						= 46,
#ifdef __ENABLE_SHAMAN_SYSTEM__
	HEADER_GC_AUTO_SHAMAN_SKILL 				= 57,
#endif
	HEADER_GC_TARGET_INFO						= 58,
	HEADER_CG_TARGET_INFO_LOAD					= 59,
	HEADER_GC_MOUNT								= 61,
	HEADER_GC_OWNERSHIP							= 62,
	HEADER_GC_TARGET			 				= 63,
#ifdef ENABLE_SPECIAL_DROP_CHAT_RENEWAL
	HEADER_GC_PICKUP_ITEM_SC					= 64,
#endif
	HEADER_GC_WARP								= 65,
	HEADER_GC_ADD_FLY_TARGETING					= 69,
	HEADER_GC_CREATE_FLY						= 70,
	HEADER_GC_FLY_TARGETING						= 71,
	HEADER_GC_SKILL_LEVEL_OLD					= 72,
	HEADER_GC_SKILL_LEVEL						= 76,
	HEADER_GC_MESSENGER							= 74,
	HEADER_GC_GUILD								= 75,
	HEADER_GC_PARTY_INVITE						= 77,
	HEADER_GC_PARTY_ADD							= 78,
	HEADER_GC_PARTY_UPDATE						= 79,
	HEADER_GC_PARTY_REMOVE						= 80,
	HEADER_GC_QUEST_INFO						= 81,
	HEADER_GC_REQUEST_MAKE_GUILD				= 82,
	HEADER_GC_PARTY_PARAMETER					= 83,
	HEADER_GC_SAFEBOX_SET						= 85,
	HEADER_GC_SAFEBOX_DEL						= 86,
	HEADER_GC_SAFEBOX_WRONG_PASSWORD			= 87,
	HEADER_GC_SAFEBOX_SIZE						= 88,
	HEADER_GC_FISHING							= 89,
	HEADER_GC_EMPIRE							= 90,
	HEADER_GC_PARTY_LINK						= 91,
	HEADER_GC_PARTY_UNLINK						= 92,
	HEADER_GC_REFINE_INFORMATION_OLD			= 95,
	HEADER_GC_VIEW_EQUIP						= 99,
	HEADER_GC_MARK_BLOCK						= 100,
	HEADER_GC_MARK_IDXLIST						= 102,
	HEADER_GC_TIME								= 106,
	HEADER_GC_CHANGE_NAME						= 107,
	HEADER_GC_DUNGEON							= 110,
	HEADER_GC_WALK_MODE							= 111,
	HEADER_GC_SKILL_GROUP						= 112,
	HEADER_GC_MAIN_CHARACTER					= 113,
	HEADER_GC_SEPCIAL_EFFECT					= 114,
	HEADER_GC_NPC_POSITION						= 115,
	HEADER_GC_LOGIN_KEY							= 118,
	HEADER_GC_REFINE_INFORMATION				= 119,
	HEADER_GC_CHANNEL							= 121,
	HEADER_GC_TARGET_UPDATE						= 123,
	HEADER_GC_TARGET_DELETE						= 124,
	HEADER_GC_TARGET_CREATE						= 125,
	HEADER_GC_AFFECT_ADD						= 126,
	HEADER_GC_AFFECT_REMOVE						= 127,
	HEADER_GC_MALL_OPEN							= 122,
	HEADER_GC_MALL_SET							= 128,
	HEADER_GC_MALL_DEL							= 129,
	HEADER_GC_LAND_LIST							= 130,
	HEADER_GC_LOVER_INFO						= 131,
	HEADER_GC_LOVE_POINT_UPDATE					= 132,
	HEADER_GC_SYMBOL_DATA						= 133,
	HEADER_GC_DIG_MOTION						= 134,
	HEADER_GC_DAMAGE_INFO           			= 135,
	HEADER_GC_CHAR_ADDITIONAL_INFO				= 136,
	HEADER_GC_MAIN_CHARACTER3_BGM				= 137,
	HEADER_GC_MAIN_CHARACTER4_BGM_VOL			= 138,
	HEADER_GC_AUTH_SUCCESS						= 150,
	HEADER_GC_PANAMA_PACK						= 151,
	HEADER_GC_HYBRIDCRYPT_KEYS					= 152,
	HEADER_GC_HYBRIDCRYPT_SDB					= 153,
	HEADER_GC_EVENT_INFO						= 155,
	HEADER_GC_EVENT_RELOAD						= 156,
	HEADER_GC_EVENT_KW_SCORE					= 157,
	HEADER_GC_SWITCHBOT							= 171,
	HEADER_GC_PRIVATE_SHOP 						= 175,
	HEADER_GC_DAILY_GIFT 						= 180,
	HEADER_GC_ROULETTE							= 200,
	HEADER_GC_SPECIFIC_EFFECT					= 208,
	HEADER_GC_DRAGON_SOUL_REFINE				= 209,
	HEADER_GC_RESPOND_CHANNELSTATUS				= 210,
	HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN			= 211,
	HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT		= 212,
	HEADER_GC_AURA								= 214,
	HEADER_GC_CUBE_RENEWAL 						= 221,
	HEADER_GC_REQUEST_CHANGE_LANGUAGE 			= 245,
	HEADER_GC_WHISPER_DETAILS					= 246,
	HEADER_GG_LOGIN								= 1,
	HEADER_GG_LOGOUT							= 2,
	HEADER_GG_RELAY								= 3,
	HEADER_GG_NOTICE							= 4,
	HEADER_GG_SHUTDOWN							= 5,
	HEADER_GG_GUILD								= 6,
	HEADER_GG_DISCONNECT						= 7,
	HEADER_GG_SHOUT								= 8,
	HEADER_GG_SETUP								= 9,
	HEADER_GG_MESSENGER_ADD                     = 10,
	HEADER_GG_MESSENGER_REMOVE                  = 11,
	HEADER_GG_FIND_POSITION						= 12,
	HEADER_GG_WARP_CHARACTER					= 13,
	HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX			= 15,
	HEADER_GG_TRANSFER							= 16,
	HEADER_GG_XMAS_WARP_SANTA					= 17,
	HEADER_GG_XMAS_WARP_SANTA_REPLY				= 18,
	HEADER_GG_RELOAD_CRC_LIST					= 19,
	HEADER_GG_LOGIN_PING						= 20,
	HEADER_GG_CHECK_CLIENT_VERSION				= 21,
	HEADER_GG_BLOCK_CHAT						= 22,
	HEADER_GG_BLOCK_EXCEPTION					= 24,
	HEADER_GG_SIEGE								= 25,
	HEADER_GG_MONARCH_NOTICE					= 26,
	HEADER_GG_MONARCH_TRANSFER					= 27,
	HEADER_GG_CHECK_AWAKENESS					= 29,
#ifdef ENABLE_FULL_NOTICE
	HEADER_GG_BIG_NOTICE						= 30,
#endif
	HEADER_GG_SWITCHBOT							= 31,
#ifdef __EVENT_MANAGER__
	HEADER_GG_EVENT_RELOAD						= 32,
	HEADER_GG_EVENT								= 33,
	HEADER_GG_EVENT_HIDE_AND_SEEK				= 34,
#endif
	HEADER_GG_REWARD_INFO 						= 45,
	HEADER_GG_MULTI_FARM						= 46,
	HEADER_GG_LOCALE_NOTICE 					= 47,
	HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH_RESULT,
	HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH,
	HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH_UPDATE,
};

#pragma pack(1)
typedef struct SPacketGGSetup
{
	BYTE	bHeader;
	WORD	wPort;
	BYTE	bChannel;
} TPacketGGSetup;

typedef struct SPacketGGLogin
{
	BYTE	bHeader;
	char	szName[CHARACTER_NAME_MAX_LEN + 1];
	DWORD	dwPID;
	BYTE	bEmpire;
	long	lMapIndex;
	BYTE	bChannel;
#ifdef __MULTI_LANGUAGE_SYSTEM__
	BYTE bLanguage;
#endif
} TPacketGGLogin;

typedef struct SPacketGGLogout
{
	BYTE	bHeader;
	char	szName[CHARACTER_NAME_MAX_LEN + 1];
} TPacketGGLogout;

typedef struct SPacketGGRelay
{
	BYTE	bHeader;
	char	szName[CHARACTER_NAME_MAX_LEN + 1];
	long	lSize;
} TPacketGGRelay;

typedef struct SPacketGGNotice
{
	BYTE	bHeader;
	long	lSize;
} TPacketGGNotice;

typedef struct SPacketGGMonarchNotice
{
	BYTE	bHeader;
	BYTE	bEmpire;
	long	lSize;
} TPacketGGMonarchNotice;

//FORKED_ROAD
typedef struct SPacketGGForkedMapInfo
{
	BYTE	bHeader;
	BYTE	bPass;
	BYTE	bSungzi;
} TPacketGGForkedMapInfo;
//END_FORKED_ROAD
typedef struct SPacketGGShutdown
{
	BYTE	bHeader;
} TPacketGGShutdown;

typedef struct SPacketGGGuild
{
	BYTE	bHeader;
	BYTE	bSubHeader;
	DWORD	dwGuild;
} TPacketGGGuild;

enum
{
	GUILD_SUBHEADER_GG_CHAT,
	GUILD_SUBHEADER_GG_SET_MEMBER_COUNT_BONUS,
};

typedef struct SPacketGGGuildChat
{
	BYTE	bHeader;
	BYTE	bSubHeader;
	DWORD	dwGuild;
	char	szText[CHAT_MAX_LEN + 1];
} TPacketGGGuildChat;

typedef struct SPacketGGParty
{
	BYTE	header;
	BYTE	subheader;
	DWORD	pid;
	DWORD	leaderpid;
} TPacketGGParty;

enum
{
	PARTY_SUBHEADER_GG_CREATE,
	PARTY_SUBHEADER_GG_DESTROY,
	PARTY_SUBHEADER_GG_JOIN,
	PARTY_SUBHEADER_GG_QUIT,
};

typedef struct SPacketGGDisconnect
{
	BYTE	bHeader;
	char	szLogin[LOGIN_MAX_LEN + 1];
} TPacketGGDisconnect;

typedef struct SPacketGGShout
{
	BYTE	bHeader;
	BYTE	bEmpire;
	char	szText[CHAT_MAX_LEN + 1];
} TPacketGGShout;

typedef struct SPacketGGXmasWarpSanta
{
	BYTE	bHeader;
	BYTE	bChannel;
	long	lMapIndex;
} TPacketGGXmasWarpSanta;

typedef struct SPacketGGXmasWarpSantaReply
{
	BYTE	bHeader;
	BYTE	bChannel;
} TPacketGGXmasWarpSantaReply;

typedef struct SPacketGGMessenger
{
	BYTE        bHeader;
	char        szAccount[CHARACTER_NAME_MAX_LEN + 1];
	char        szCompanion[CHARACTER_NAME_MAX_LEN + 1];
} TPacketGGMessenger;

typedef struct SPacketGGFindPosition
{
	BYTE header;
	DWORD dwFromPID; // �� ��ġ�� �����Ϸ��� ���
	DWORD dwTargetPID; // ã�� ���
} TPacketGGFindPosition;

typedef struct SPacketGGWarpCharacter
{
	BYTE header;
	DWORD pid;
	long x;
	long y;
} TPacketGGWarpCharacter;

//  HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX	    = 15,

typedef struct SPacketGGGuildWarMapIndex
{
	BYTE bHeader;
	DWORD dwGuildID1;
	DWORD dwGuildID2;
	long lMapIndex;
} TPacketGGGuildWarMapIndex;

typedef struct SPacketGGTransfer
{
	BYTE	bHeader;
	char	szName[CHARACTER_NAME_MAX_LEN + 1];
	long	lX, lY;
} TPacketGGTransfer;

typedef struct SPacketGGLoginPing
{
	BYTE	bHeader;
	char	szLogin[LOGIN_MAX_LEN + 1];
} TPacketGGLoginPing;

typedef struct SPacketGGBlockChat
{
	BYTE	bHeader;
	char	szName[CHARACTER_NAME_MAX_LEN + 1];
	long	lBlockDuration;
} TPacketGGBlockChat;

/* Ŭ���̾�Ʈ ������ ������ ��Ŷ */

typedef struct command_text
{
	BYTE	bHeader;
} TPacketCGText;

/* �α��� (1) */
typedef struct command_handshake
{
	BYTE	bHeader;
	DWORD	dwHandshake;
	DWORD	dwTime;
	long	lDelta;
} TPacketCGHandshake;

typedef struct command_login
{
	BYTE	header;
	char	login[LOGIN_MAX_LEN + 1];
	char	passwd[PASSWD_MAX_LEN + 1];
} TPacketCGLogin;

typedef struct command_login2
{
	BYTE	header;
	char	login[LOGIN_MAX_LEN + 1];
	DWORD	dwLoginKey;
	DWORD	adwClientKey[4];
} TPacketCGLogin2;

typedef struct command_login3
{
	BYTE	header;
	char	login[LOGIN_MAX_LEN + 1];
	char	passwd[PASSWD_MAX_LEN + 1];
	DWORD	adwClientKey[4];
#ifdef __MULTI_LANGUAGE_SYSTEM__
	BYTE bLanguage;
#endif
} TPacketCGLogin3;

typedef struct packet_login_key
{
	BYTE	bHeader;
	DWORD	dwLoginKey;
} TPacketGCLoginKey;

typedef struct command_player_select
{
	BYTE	header;
	BYTE	index;
} TPacketCGPlayerSelect;

typedef struct command_player_delete
{
	BYTE	header;
	BYTE	index;
	char	private_code[8];
} TPacketCGPlayerDelete;

typedef struct command_player_create
{
	BYTE        header;
	BYTE        index;
	char        name[CHARACTER_NAME_MAX_LEN + 1];
	WORD        job;
	BYTE	shape;
	BYTE	Con;
	BYTE	Int;
	BYTE	Str;
	BYTE	Dex;
} TPacketCGPlayerCreate;

typedef struct command_player_create_success
{
	BYTE		header;
	BYTE		bAccountCharacterIndex;
	TSimplePlayer	player;
} TPacketGCPlayerCreateSuccess;

// ����
typedef struct command_attack
{
	BYTE	bHeader;
	BYTE	bType;
	DWORD	dwVID;
	BYTE	bCRCMagicCubeProcPiece;
	BYTE	bCRCMagicCubeFilePiece;
} TPacketCGAttack;

enum EMoveFuncType
{
	FUNC_WAIT,
	FUNC_MOVE,
	FUNC_ATTACK,
	FUNC_COMBO,
	FUNC_MOB_SKILL,
	_FUNC_SKILL,
	FUNC_MAX_NUM,
	FUNC_SKILL = 0x80,
};

// �̵�
typedef struct command_move
{
	BYTE	bHeader;
	BYTE	bFunc;
	BYTE	bArg;
	BYTE	bRot;
	long	lX;
	long	lY;
	DWORD	dwTime;
} TPacketCGMove;

typedef struct command_sync_position_element
{
	DWORD	dwVID;
	long	lX;
	long	lY;
} TPacketCGSyncPositionElement;

// ��ġ ����ȭ
typedef struct command_sync_position	// ���� ��Ŷ
{
	BYTE	bHeader;
	WORD	wSize;
} TPacketCGSyncPosition;

/* ä�� (3) */
typedef struct command_chat	// ���� ��Ŷ
{
	BYTE	header;
	WORD	size;
	BYTE	type;
} TPacketCGChat;

/* �ӼӸ� */
typedef struct command_whisper
{
	BYTE	bHeader;
	WORD	wSize;
	char 	szNameTo[CHARACTER_NAME_MAX_LEN + 1];
} TPacketCGWhisper;

typedef struct command_entergame
{
	BYTE	header;
} TPacketCGEnterGame;

typedef struct command_item_use
{
	BYTE 	header;
	TItemPos 	Cell;
} TPacketCGItemUse;

typedef struct command_item_use_to_item
{
	BYTE	header;
	TItemPos	Cell;
	TItemPos	TargetCell;
} TPacketCGItemUseToItem;

typedef struct command_item_drop
{
	BYTE 	header;
	TItemPos 	Cell;
	DWORD	gold;
} TPacketCGItemDrop;

typedef struct command_item_drop2
{
	BYTE 	header;
	TItemPos 	Cell;
	DWORD	gold;
	WORD	count;
} TPacketCGItemDrop2;

typedef struct command_item_destroy
{
	BYTE		header;
	TItemPos	Cell;
} TPacketCGItemDestroy;

typedef struct command_item_move
{
	BYTE 	header;
	TItemPos	Cell;
	TItemPos	CellTo;
	WORD	count;
} TPacketCGItemMove;


#ifdef ENABLE_EXTEND_INVEN_SYSTEM
typedef struct envanter_paketi
{
	BYTE	header;
} TPacketCGEnvanter;
#endif

#ifdef ENABLE_SELL_ITEM
typedef struct command_item_sell
{
    BYTE        header;
    TItemPos    Cell;
	DWORD		gold;
} TPacketCGItemSell;
#endif

typedef struct command_item_pickup
{
	BYTE 	header;
	DWORD	vid;
} TPacketCGItemPickup;

typedef struct command_quickslot_add
{
	BYTE	header;
	BYTE	pos;
	TQuickslot	slot;
} TPacketCGQuickslotAdd;

typedef struct command_quickslot_del
{
	BYTE	header;
	BYTE	pos;
} TPacketCGQuickslotDel;

typedef struct command_quickslot_swap
{
	BYTE	header;
	BYTE	pos;
	BYTE	change_pos;
} TPacketCGQuickslotSwap;
enum
{
	SHOP_SUBHEADER_CG_END,
	SHOP_SUBHEADER_CG_BUY,
	SHOP_SUBHEADER_CG_SELL,
	SHOP_SUBHEADER_CG_SELL2
};

typedef struct command_shop_buy
{
	BYTE	count;
} TPacketCGShopBuy;

typedef struct command_shop_sell
{
	BYTE	pos;
	BYTE	count;
} TPacketCGShopSell;

typedef struct command_shop
{
	BYTE	header;
	BYTE	subheader;
} TPacketCGShop;

typedef struct command_on_click
{
	BYTE	header;
	DWORD	vid;
} TPacketCGOnClick;

enum
{
	EXCHANGE_SUBHEADER_CG_START,	/* arg1 == vid of target character */
	EXCHANGE_SUBHEADER_CG_ITEM_ADD,	/* arg1 == position of item */
	EXCHANGE_SUBHEADER_CG_ITEM_DEL,	/* arg1 == position of item */
	EXCHANGE_SUBHEADER_CG_ELK_ADD,	/* arg1 == amount of gold */
	EXCHANGE_SUBHEADER_CG_ACCEPT,	/* arg1 == not used */
	EXCHANGE_SUBHEADER_CG_CANCEL,	/* arg1 == not used */
};

typedef struct command_exchange
{
	BYTE	header;
	BYTE	sub_header;
#ifdef ENABLE_REMOVE_LIMIT_GOLD
	unsigned long long	arg1;
#else
	DWORD	arg1;
#endif
	BYTE	arg2;
	TItemPos	Pos;
} TPacketCGExchange;

typedef struct command_position
{
	BYTE	header;
	BYTE	position;
} TPacketCGPosition;

typedef struct command_script_answer
{
	BYTE	header;
	BYTE	answer;
	//char	file[32 + 1];
	//BYTE	answer[16 + 1];
} TPacketCGScriptAnswer;


typedef struct command_script_button
{
	BYTE        header;
	unsigned int	idx;
} TPacketCGScriptButton;

typedef struct command_quest_input_string
{
	BYTE header;
	char msg[64+1];
} TPacketCGQuestInputString;

typedef struct command_quest_confirm
{
	BYTE header;
	BYTE answer;
	DWORD requestPID;
} TPacketCGQuestConfirm;

/*
 * ���� ������ ������ ��Ŷ
 */
typedef struct packet_quest_confirm
{
	BYTE header;
	char msg[64+1];
	long timeout;
	DWORD requestPID;
} TPacketGCQuestConfirm;

typedef struct packet_handshake
{
	BYTE	bHeader;
	DWORD	dwHandshake;
	DWORD	dwTime;
	long	lDelta;
} TPacketGCHandshake;

enum EPhase
{
	PHASE_CLOSE,
	PHASE_HANDSHAKE,
	PHASE_LOGIN,
	PHASE_SELECT,
	PHASE_LOADING,
	PHASE_GAME,
	PHASE_DEAD,

	PHASE_CLIENT_CONNECTING,
	PHASE_DBCLIENT,
	PHASE_P2P,
	PHASE_AUTH,
};

typedef struct packet_phase
{
	BYTE	header;
	BYTE	phase;
} TPacketGCPhase;

typedef struct packet_bindudp
{
	BYTE	header;
	DWORD	addr;
	WORD	port;
} TPacketGCBindUDP;

enum
{
	LOGIN_FAILURE_ALREADY	= 1,
	LOGIN_FAILURE_ID_NOT_EXIST	= 2,
	LOGIN_FAILURE_WRONG_PASS	= 3,
	LOGIN_FAILURE_FALSE		= 4,
	LOGIN_FAILURE_NOT_TESTOR	= 5,
	LOGIN_FAILURE_NOT_TEST_TIME	= 6,
	LOGIN_FAILURE_FULL		= 7
};

typedef struct packet_login_success
{
	BYTE		bHeader;
	TSimplePlayer	players[PLAYER_PER_ACCOUNT];
	DWORD		guild_id[PLAYER_PER_ACCOUNT];
	char		guild_name[PLAYER_PER_ACCOUNT][GUILD_NAME_MAX_LEN+1];

	DWORD		handle;
	DWORD		random_key;
} TPacketGCLoginSuccess;

typedef struct packet_auth_success
{
	BYTE	bHeader;
	DWORD	dwLoginKey;
	BYTE	bResult;
} TPacketGCAuthSuccess;

typedef struct packet_login_failure
{
	BYTE	header;
	char	szStatus[ACCOUNT_STATUS_MAX_LEN + 1];
} TPacketGCLoginFailure;

typedef struct packet_create_failure
{
	BYTE	header;
	BYTE	bType;
} TPacketGCCreateFailure;

enum
{
	ADD_CHARACTER_STATE_DEAD		= (1 << 0),
	ADD_CHARACTER_STATE_SPAWN		= (1 << 1),
	ADD_CHARACTER_STATE_GUNGON		= (1 << 2),
	ADD_CHARACTER_STATE_KILLER		= (1 << 3),
	ADD_CHARACTER_STATE_PARTY		= (1 << 4),
};

enum ECharacterEquipmentPart
{
	CHR_EQUIPPART_ARMOR,
	CHR_EQUIPPART_WEAPON,
	CHR_EQUIPPART_HEAD,
	CHR_EQUIPPART_HAIR,
#ifdef __SASH_SYSTEM__
	CHR_EQUIPPART_SASH,
#endif
#ifdef __AURA_SYSTEM__
	CHR_EQUIPPART_AURA,
#endif
	CHR_EQUIPPART_NUM,
};

typedef struct packet_add_char
{
	BYTE	header;
	DWORD	dwVID;

	float	angle;
	long	x;
	long	y;
	long	z;

	BYTE	bType;
	WORD	wRaceNum;
	BYTE	bMovingSpeed;
	BYTE	bAttackSpeed;

	BYTE	bStateFlag;
	DWORD	dwAffectFlag[2];	// ȿ��
} TPacketGCCharacterAdd;

typedef struct packet_char_additional_info
{
	BYTE    header;
	DWORD   dwVID;
	char    name[CHARACTER_NAME_MAX_LEN + 1];
	WORD    awPart[CHR_EQUIPPART_NUM];
	BYTE	bEmpire;
	DWORD   dwGuildID;
	DWORD   dwLevel;
#if defined(__CONQUEROR_LEVEL__)
	DWORD dwConquerorLevel;
#endif
	short	sAlignment;
	BYTE	bPKMode;
	DWORD	dwMountVnum;
#ifdef ENABLE_REFINE_ELEMENT
	BYTE	bRefineElementType;
#endif
#ifdef ENABLE_SHOW_LIDER_AND_GENERAL_GUILD
	BYTE	dwNewIsGuildName;
#endif
#ifdef __ENABLE_PREMIUM_PLAYERS__
	BYTE byPremium;
	long int iPremiumTime;
#endif
#ifdef __MULTI_LANGUAGE_SYSTEM__
	BYTE bLanguage;
#endif
} TPacketGCCharacterAdditionalInfo;

typedef struct packet_update_char
{
	BYTE	header;
	DWORD	dwVID;

	WORD        awPart[CHR_EQUIPPART_NUM];
	BYTE	bMovingSpeed;
	BYTE	bAttackSpeed;

	BYTE	bStateFlag;
	DWORD	dwAffectFlag[2];

	DWORD	dwGuildID;
	short	sAlignment;
#ifdef __FIX_UPDATE_LEVEL__
	DWORD	dwLevel;
#endif
#if defined(__CONQUEROR_LEVEL__)
	DWORD	dwConquerorLevel;
#endif
	BYTE	bPKMode;

	DWORD	dwMountVnum;
#ifdef ENABLE_REFINE_ELEMENT
	BYTE	bRefineElementType;
#endif

#ifdef ENABLE_SHOW_LIDER_AND_GENERAL_GUILD
	BYTE	dwNewIsGuildName;
#endif
#ifdef __ENABLE_PREMIUM_PLAYERS__
	BYTE byPremium;
	long int iPremiumTime;
#endif
#ifdef __MULTI_LANGUAGE_SYSTEM__
	BYTE bLanguage;
#endif
} TPacketGCCharacterUpdate;

typedef struct packet_del_char
{
	BYTE	header;
	DWORD	id;
} TPacketGCCharacterDelete;

typedef struct packet_chat	// ���� ��Ŷ
{
	BYTE	header;
	WORD	size;
	BYTE	type;
	DWORD	id;
	BYTE	bEmpire;
#if defined(LOCALE_STRING_RENEWAL)
	bool	bCanFormat;
	packet_chat() : bCanFormat(true) {}
#endif

} TPacketGCChat;

typedef struct packet_whisper	// ���� ��Ŷ
{
	BYTE	bHeader;
	WORD	wSize;
	BYTE	bType;
	char	szNameFrom[CHARACTER_NAME_MAX_LEN + 1];
#if defined(LOCALE_STRING_RENEWAL)
	bool	bCanFormat;
	packet_whisper() : bCanFormat(true) {}
#endif
} TPacketGCWhisper;

typedef struct packet_main_character
{
	BYTE        header;
	DWORD	dwVID;
	WORD	wRaceNum;
	char	szName[CHARACTER_NAME_MAX_LEN + 1];
	long	lx, ly, lz;
	BYTE	empire;
	BYTE	skill_group;
} TPacketGCMainCharacter;

// SUPPORT_BGM
typedef struct packet_main_character3_bgm
{
	enum
	{
		MUSIC_NAME_LEN = 24,
	};

	BYTE    header;
	DWORD	dwVID;
	WORD	wRaceNum;
	char	szChrName[CHARACTER_NAME_MAX_LEN + 1];
	char	szBGMName[MUSIC_NAME_LEN + 1];
	long	lx, ly, lz;
	BYTE	empire;
	BYTE	skill_group;
} TPacketGCMainCharacter3_BGM;

typedef struct packet_main_character4_bgm_vol
{
	enum
	{
		MUSIC_NAME_LEN = 24,
	};

	BYTE    header;
	DWORD	dwVID;
	WORD	wRaceNum;
	char	szChrName[CHARACTER_NAME_MAX_LEN + 1];
	char	szBGMName[MUSIC_NAME_LEN + 1];
	float	fBGMVol;
	long	lx, ly, lz;
	BYTE	empire;
	BYTE	skill_group;
} TPacketGCMainCharacter4_BGM_VOL;
// END_OF_SUPPORT_BGM

typedef struct packet_points
{
	BYTE	header;
	long long		points[POINT_MAX_NUM];
} TPacketGCPoints;

typedef struct packet_skill_level
{
	BYTE		bHeader;
	TPlayerSkill	skills[SKILL_MAX_NUM];
} TPacketGCSkillLevel;

typedef struct packet_point_change
{
	int		header;
	DWORD	dwVID;
	BYTE	type;
	long long	amount;
	long long	value;
} TPacketGCPointChange;

typedef struct packet_stun
{
	BYTE	header;
	DWORD	vid;
} TPacketGCStun;

typedef struct packet_dead
{
	BYTE	header;
	DWORD	vid;
} TPacketGCDead;

struct TPacketGCItemDelDeprecated
{
	BYTE	header;
	TItemPos Cell;
	DWORD	vnum;
	BYTE	count;
#ifdef ENABLE_REFINE_ELEMENT
	DWORD	dwRefineElement;
#endif
#ifdef __CHANGELOOK_SYSTEM__
	DWORD	transmutation;
#endif
	long	alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
};

#ifdef __PREMIUM_PRIVATE_SHOP__
enum EPrivateShopGCSubheader
{
	SUBHEADER_GC_PRIVATE_SHOP_ADD_ENTITY,
	SUBHEADER_GC_PRIVATE_SHOP_DEL_ENTITY,
	SUBHEADER_GC_PRIVATE_SHOP_TITLE,
	SUBHEADER_GC_PRIVATE_SHOP_LOAD,
	SUBHEADER_GC_PRIVATE_SHOP_SET_ITEM,
	SUBHEADER_GC_PRIVATE_SHOP_SET_SALE,
	SUBHEADER_GC_PRIVATE_SHOP_BALANCE_UPDATE,
	SUBHEADER_GC_PRIVATE_SHOP_OPEN_PANEL,
	SUBHEADER_GC_PRIVATE_SHOP_CLOSE_PANEL,
	SUBHEADER_GC_PRIVATE_SHOP_CLOSE,
	SUBHEADER_GC_PRIVATE_SHOP_START,
	SUBHEADER_GC_PRIVATE_SHOP_END,
	SUBHEADER_GC_PRIVATE_SHOP_REMOVE_ITEM,
	SUBHEADER_GC_PRIVATE_SHOP_REMOVE_MY_ITEM,
	SUBHEADER_GC_PRIVATE_SHOP_ADD_ITEM,
	SUBHEADER_GC_PRIVATE_SHOP_STATE_UPDATE,
	SUBHEADER_GC_PRIVATE_SHOP_WITHDRAW,
	SUBHEADER_GC_PRIVATE_SHOP_ITEM_PRICE_CHANGE,
	SUBHEADER_GC_PRIVATE_SHOP_ITEM_MOVE,
	SUBHEADER_GC_PRIVATE_SHOP_TITLE_CHANGE,
	SUBHEADER_GC_PRIVATE_SHOP_UNLOCKED_SLOTS_CHANGE,

	SUBHEADER_GC_PRIVATE_SHOP_SEARCH_OPEN_LOOK_MODE,
	SUBHEADER_GC_PRIVATE_SHOP_SEARCH_OPEN_TRADE_MODE,
	SUBHEADER_GC_PRIVATE_SHOP_SEARCH_RESULT,
	SUBHEADER_GC_PRIVATE_SHOP_SEARCH_UPDATE,

	SUBHEADER_GC_PRIVATE_SHOP_MARKET_ITEM_PRICE_DATA_RESULT,
	SUBHEADER_GC_PRIVATE_SHOP_MARKET_ITEM_PRICE_RESULT,
};

enum EPrivateShopCGSubheader
{
	SUBHEADER_CG_PRIVATE_SHOP_BUILD,
	SUBHEADER_CG_PRIVATE_SHOP_CLOSE,
	SUBHEADER_CG_PRIVATE_SHOP_PANEL_OPEN,
	SUBHEADER_CG_PRIVATE_SHOP_PANEL_CLOSE,
	SUBHEADER_CG_PRIVATE_SHOP_START,
	SUBHEADER_CG_PRIVATE_SHOP_END,
	SUBHEADER_CG_PRIVATE_SHOP_BUY,
	SUBHEADER_CG_PRIVATE_SHOP_WITHDRAW,
	SUBHEADER_CG_PRIVATE_SHOP_MODIFY,
	SUBHEADER_CG_PRIVATE_SHOP_STATE_UPDATE,
	SUBHEADER_CG_PRIVATE_SHOP_ITEM_PRICE_CHANGE,
	SUBHEADER_CG_PRIVATE_SHOP_ITEM_MOVE,
	SUBHEADER_CG_PRIVATE_SHOP_ITEM_CHECKIN,
	SUBHEADER_CG_PRIVATE_SHOP_ITEM_CHECKOUT,
	SUBHEADER_CG_PRIVATE_SHOP_TITLE_CHANGE,
	SUBHEADER_CG_PRIVATE_SHOP_WARP_REQUEST,
	SUBHEADER_CG_PRIVATE_SHOP_SLOT_UNLOCK_REQUEST,

	SUBHEADER_CG_PRIVATE_SHOP_SEARCH_CLOSE,
	SUBHEADER_CG_PRIVATE_SHOP_SEARCH,
	SUBHEADER_CG_PRIVATE_SHOP_SEARCH_BUY,

	SUBHEADER_CG_PRIVATE_SHOP_MARKET_ITEM_PRICE_DATA_REQUEST,
	SUBHEADER_CG_PRIVATE_SHOP_MARKET_ITEM_PRICE_REQUEST,
};

typedef struct SPrivateShopItem
{
	TItemPos	TPos;
	TItemPrice	TPrice;
	WORD		wDisplayPos;
} TPrivateShopItem;

typedef struct SPrivateShopItemData
{
	DWORD					dwVnum;
	TItemPrice				TPrice;
	time_t					tCheckin;
	DWORD					dwCount;
	WORD					wPos;
	long					alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute	aAttr[ITEM_ATTRIBUTE_MAX_NUM];
#ifdef ENABLE_PET_GROWTH_SYSTEM
	//TPetGrowthCache pet = {};
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
} TPrivateShopItemData;

typedef struct SPrivateShopSaleData
{
	char					szCustomer[CHARACTER_NAME_MAX_LEN + 1];
	time_t					tTime;
	TPrivateShopItemData	TItem;
} TPrivateShopSaleData;

typedef struct SPrivateShopSearchData
{
	DWORD					dwShopID;
	char					szOwnerName[CHARACTER_NAME_MAX_LEN + 1];
	DWORD					dwVnum;
	TItemPrice				TPrice;
	DWORD					dwCount;
	WORD					wPos;
	long					alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute	aAttr[ITEM_ATTRIBUTE_MAX_NUM];
	time_t					tCheckin;
	BYTE					bState;
#ifdef ENABLE_PET_GROWTH_SYSTEM
	//TPetGrowthCache pet = {};
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
} TPrivateShopSearchData;

typedef struct SPacketCGPrivateShop
{
	BYTE	bHeader;
	BYTE	bSubHeader;
} TPacketCGPrivateShop;

typedef struct SPacketGCPrivateShopAddEntity
{
	long		lX;
	long		lY;
	long		lZ;
	DWORD		dwVID;
	DWORD		dwVnum;
	char		szName[CHARACTER_NAME_MAX_LEN + 1];
	BYTE		bTitleType;
	char		szTitle[TITLE_MAX_LEN + 1];
} TPacketGCPrivateShopAddEntity;

typedef struct SPacketGCPrivateShopDelEntity
{
	DWORD		dwVID;
} TPacketGCPrivateShopDelEntity;

typedef struct SPacketGCPrivateShopTitle
{
	DWORD		dwVID;
	char		szTitle[TITLE_MAX_LEN + 1];
	BYTE		bTitleType;
} TPacketGCPrivateShopTitle;

typedef struct SPacketCGPrivateShopBuild
{
	char	szTitle[TITLE_MAX_LEN + 1];
	DWORD	dwPolyVnum;
	BYTE	bTitleType;
	BYTE	bPageCount;
	WORD	wItemCount;
} TPacketCGPrivateShopBuild;

typedef struct SPacketCGPrivateShopItemPriceChange
{
	WORD		wPos;
	TItemPrice	TPrice;
} TPacketCGPrivateShopItemPriceChange;

typedef struct SPacketCGPrivateShopItemMove
{
	WORD		wPos;
	WORD		wChangePos;
} TPacketCGPrivateShopItemMove;

typedef struct SPrivateShopSearchFilter
{
	DWORD	dwVnum;
	char	szOwnerName[CHARACTER_NAME_MAX_LEN + 1];

	int		iItemType;
	int		iItemSubType;

	int		iJob;
	int		iGender;

	WORD	wMinStack;
	WORD	wMaxStack;

	BYTE	bMinRefine;
	BYTE	bMaxRefine;

	DWORD	dwMinLevel;
	DWORD	dwMaxLevel;

	TPlayerItemAttribute	aAttr[ITEM_ATTRIBUTE_MAX_NUM];
	WORD					wSashAbsorption;
	BYTE					bAlchemyLevel;
	BYTE					bAlchemyClarity;
} TPrivateShopSearchFilter;

typedef struct SPacketCGPrivateShopSearch
{
	TPrivateShopSearchFilter	Filter;
	bool						bUseFilter;
} TPacketCGPrivateShopSearch;

typedef struct SPrivateShopSearchSelectedItem
{
	DWORD		dwShopID;
	WORD		wPos;
	TItemPrice	TPrice;
} SPrivateShopSearchSelectedItem;

typedef struct SPacketCGPrivateShopSearchBuy
{
	SPrivateShopSearchSelectedItem aSelectedItems[SELECTED_ITEM_MAX_NUM];
} TPacketCGPrivateShopSearchBuy;

typedef struct SPacketGCPrivateShopSearchUpdate
{
	DWORD	dwShopID;
	int		iSpecificItemPos;
	BYTE	bState;
} TPacketGCPrivateShopSearchUpdate;

typedef struct SPacketGCPrivateShop
{
	BYTE	bHeader;
	WORD	wSize;
	BYTE	bSubHeader;
} TPacketGCPrivateShop;

typedef struct SPacketGCPrivateShopLoad
{
	char		szTitle[SHOP_SIGN_MAX_LEN + 1];
	long long	llGold;
	DWORD		dwCheque;
	long		lX;
	long		lY;
	BYTE		bChannel;
	BYTE		bState;
	BYTE		bPageCount;
} TPacketGCPrivateShopLoad;

typedef struct SPacketGCPrivateShopOpen
{
	char					szTitle[SHOP_SIGN_MAX_LEN + 1];
	BYTE					bState;
	BYTE					bPageCount;
	WORD					wUnlockedSlots;
	DWORD					dwVID;
	TPrivateShopItemData	aItems[PRIVATE_SHOP_HOST_ITEM_MAX_NUM];
} TPacketGCPrivateShopOpen;

typedef struct SPacketGCPrivateStateUpdate
{
	BYTE	bState;
	bool	bIsMainPlayerPrivateShop;
} TPacketGCPrivateStateUpdate;

typedef struct SPacketGCPrivateShopItemPriceChange
{
	WORD		wPos;
	TItemPrice	TPrice;
} TPacketGCPrivateShopItemPriceChange;

typedef struct SPacketGCPrivateShopItemMove
{
	WORD		wPos;
	WORD		wChangePos;
} TPacketGCPrivateShopItemMove;

typedef struct SPacketGCPrivateShopBalanceUpdate
{
	TItemPrice	TPrice;
} TPacketGCPrivateShopBalanceUpdate;

typedef struct SPacketCGPrivateShopItemCheckin
{
	TItemPos	TSrcPos;
	long long	llGold;
	DWORD		dwCheque;
	int			iDstPos;
} TPacketCGPrivateShopItemCheckin;

typedef struct SPacketCGPrivateShopItemCheckout
{
	WORD		wSrcPos;
	int			iDstPos;
} TPacketCGPrivateShopItemCheckout;

typedef struct SPacketGGPrivateShopItemRemove
{
	BYTE					bHeader;
	DWORD					dwShopID;
	WORD					wPos;
} TPacketGGPrivateShopItemRemove;

typedef struct SPacketGGPrivateShopItemSearch
{
	BYTE						bHeader;
	DWORD						dwCustomerID;
	DWORD						dwCustomerPort;
	bool						bUseFilter;
	TPrivateShopSearchFilter	Filter;
} TPacketGGPrivateShopItemSearch;

typedef struct SPacketGGPrivateShopItemSearchUpdate
{
	BYTE					bHeader;
	DWORD					dwShopID;
	int						iSpecificItemPos;
	BYTE					bState;
} TPacketGGPrivateShopItemSearchUpdate;

typedef struct SPacketGGPrivateShopItemSearchResult
{
	BYTE					bHeader;
	WORD					wSize;
	DWORD					dwCustomerID;
} TPacketGGPrivateShopItemSearchResult;
#endif


typedef struct packet_item_set
{
	BYTE	header;
	TItemPos Cell;
	DWORD	vnum;
	WORD	count;
#ifdef ENABLE_REFINE_ELEMENT
	DWORD	dwRefineElement;
#endif
#ifdef __CHANGELOOK_SYSTEM__
	DWORD	transmutation;
#endif
	DWORD	flags;
	DWORD	anti_flags;
	bool	highlight;
	long	alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
} TPacketGCItemSet;

typedef struct packet_item_del
{
	BYTE	header;
#if defined(__EXTENDED_SAFEBOX__)
	DWORD	pos;
#else
	BYTE	pos;
#endif
} TPacketGCItemDel;

struct packet_item_use
{
	BYTE	header;
	TItemPos Cell;
	DWORD	ch_vid;
	DWORD	victim_vid;
	DWORD	vnum;
};

struct packet_item_move
{
	BYTE	header;
	TItemPos Cell;
	TItemPos CellTo;
};

typedef struct packet_item_update
{
	BYTE	header;
	TItemPos Cell;
	WORD	count;
#ifdef ENABLE_REFINE_ELEMENT
	DWORD	dwRefineElement;
#endif
#ifdef __CHANGELOOK_SYSTEM__
	DWORD	transmutation;
#endif
	long	alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
} TPacketGCItemUpdate;

typedef struct packet_item_ground_add
{
	BYTE	bHeader;
	long 	x, y, z;
	DWORD	dwVID;
	DWORD	dwVnum;
} TPacketGCItemGroundAdd;

typedef struct packet_item_ownership
{
	BYTE	bHeader;
	DWORD	dwVID;
	char	szName[CHARACTER_NAME_MAX_LEN + 1];
} TPacketGCItemOwnership;

typedef struct packet_item_ground_del
{
	BYTE	bHeader;
	DWORD	dwVID;
} TPacketGCItemGroundDel;

struct packet_quickslot_add
{
	BYTE	header;
	BYTE	pos;
	TQuickslot	slot;
};

struct packet_quickslot_del
{
	BYTE	header;
	BYTE	pos;
};

struct packet_quickslot_swap
{
	BYTE	header;
	BYTE	pos;
	BYTE	pos_to;
};

struct packet_motion
{
	BYTE	header;
	DWORD	vid;
	DWORD	victim_vid;
	WORD	motion;
};

enum EPacketShopSubHeaders
{
	SHOP_SUBHEADER_GC_START,
	SHOP_SUBHEADER_GC_END,
	SHOP_SUBHEADER_GC_UPDATE_ITEM,
	SHOP_SUBHEADER_GC_UPDATE_PRICE,
	SHOP_SUBHEADER_GC_OK,
	SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY,
	SHOP_SUBHEADER_GC_SOLDOUT,
	SHOP_SUBHEADER_GC_INVENTORY_FULL,
	SHOP_SUBHEADER_GC_INVALID_POS,
	SHOP_SUBHEADER_GC_SOLD_OUT,
	SHOP_SUBHEADER_GC_START_EX,
	SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY_EX,
#if defined(ENABLE_RENEWAL_SHOPEX)
	SHOP_SUBHEADER_GC_NOT_ENOUGH_ITEM,
	SHOP_SUBHEADER_GC_NOT_ENOUGH_EXP,
#endif
};

struct packet_shop_item
{
	DWORD       vnum;
#ifdef ENABLE_REMOVE_LIMIT_GOLD
	unsigned long long       price;
#else
	DWORD		price;
#endif
	WORD        count;
	BYTE		display_pos;
#ifdef ENABLE_REFINE_ELEMENT
	DWORD		dwRefineElement;
#endif
	long	alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
#ifdef __CHANGELOOK_SYSTEM__
	DWORD	transmutation;
#endif
#if defined(ENABLE_RENEWAL_SHOPEX)
	BYTE 	price_type;
	DWORD 	price_vnum;
	packet_shop_item() : price_type(SHOPEX_GOLD), price_vnum(0) {
		memset(&alSockets, 0, sizeof(alSockets));
		memset(&aAttr, 0, sizeof(aAttr));
	}
#endif
};

typedef struct packet_shop_start
{
	DWORD   owner_vid;
	struct packet_shop_item	items[SHOP_HOST_ITEM_MAX_NUM];
#ifdef ENABLE_DISTANCE_SHOPPING
	bool	isDistanceShop;
#endif
} TPacketGCShopStart;

typedef struct packet_shop_start_ex // ������ TSubPacketShopTab* shop_tabs �� �����.
{
	typedef struct sub_packet_shop_tab
	{
		char name[SHOP_TAB_NAME_MAX];
		BYTE coin_type;
		packet_shop_item items[SHOP_HOST_ITEM_MAX_NUM];
	} TSubPacketShopTab;
	DWORD owner_vid;
	BYTE shop_tab_count;
} TPacketGCShopStartEx;

typedef struct packet_shop_update_item
{
	BYTE			pos;
	struct packet_shop_item	item;
} TPacketGCShopUpdateItem;

typedef struct packet_shop_update_price
{
#ifdef ENABLE_REMOVE_LIMIT_GOLD
	unsigned long long				iPrice;
#else
	int				iPrice;
#endif
} TPacketGCShopUpdatePrice;

typedef struct packet_shop	// ���� ��Ŷ
{
	BYTE        header;
	WORD	size;
	BYTE        subheader;
} TPacketGCShop;

struct packet_exchange
{
	BYTE	header;
	BYTE	sub_header;
	BYTE	is_me;
#ifdef ENABLE_REMOVE_LIMIT_GOLD
	unsigned long long	arg1;	// vnum
#else
	DWORD	arg1;	// vnum
#endif
	TItemPos	arg2;	// cell
	DWORD	arg3;	// count
#ifdef WJ_ENABLE_TRADABLE_ICON
	TItemPos	arg4;	// srccell
#endif
	long	alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
#ifdef ENABLE_REFINE_ELEMENT
	DWORD	dwRefineElement;
#endif
#ifdef __CHANGELOOK_SYSTEM__
	DWORD	dwTransmutation;
#endif

};

enum EPacketTradeSubHeaders
{
	EXCHANGE_SUBHEADER_GC_START,	/* arg1 == vid */
	EXCHANGE_SUBHEADER_GC_ITEM_ADD,	/* arg1 == vnum  arg2 == pos  arg3 == count */
	EXCHANGE_SUBHEADER_GC_ITEM_DEL,
	EXCHANGE_SUBHEADER_GC_GOLD_ADD,	/* arg1 == gold */
	EXCHANGE_SUBHEADER_GC_ACCEPT,	/* arg1 == accept */
	EXCHANGE_SUBHEADER_GC_END,		/* arg1 == not used */
	EXCHANGE_SUBHEADER_GC_ALREADY,	/* arg1 == not used */
	EXCHANGE_SUBHEADER_GC_LESS_GOLD,	/* arg1 == not used */
};

struct packet_position
{
	BYTE	header;
	DWORD	vid;
	BYTE	position;
};

typedef struct packet_ping
{
	BYTE	header;
} TPacketGCPing;

struct packet_script
{
	BYTE	header;
	WORD	size;
	BYTE	skin;
	WORD	src_size;
};

typedef struct packet_change_speed
{
	BYTE		header;
	DWORD		vid;
	WORD		moving_speed;
} TPacketGCChangeSpeed;

struct packet_mount
{
	BYTE	header;
	DWORD	vid;
	DWORD	mount_vid;
	BYTE	pos;
	DWORD	x, y;
};

typedef struct packet_move
{
	BYTE		bHeader;
	BYTE		bFunc;
	BYTE		bArg;
	BYTE		bRot;
	DWORD		dwVID;
	long		lX;
	long		lY;
	DWORD		dwTime;
	DWORD		dwDuration;
} TPacketGCMove;

// ������
typedef struct packet_ownership
{
	BYTE		bHeader;
	DWORD		dwOwnerVID;
	DWORD		dwVictimVID;
} TPacketGCOwnership;

// ��ġ ����ȭ ��Ŷ�� bCount ��ŭ �ٴ� ����
typedef struct packet_sync_position_element
{
	DWORD	dwVID;
	long	lX;
	long	lY;
} TPacketGCSyncPositionElement;

// ��ġ ����ȭ
typedef struct packet_sync_position	// ���� ��Ŷ
{
	BYTE	bHeader;
	WORD	wSize;	// ���� = (wSize - sizeof(TPacketGCSyncPosition)) / sizeof(TPacketGCSyncPositionElement)
} TPacketGCSyncPosition;

typedef struct packet_fly
{
	BYTE	bHeader;
	BYTE	bType;
	DWORD	dwStartVID;
	DWORD	dwEndVID;
} TPacketGCCreateFly;

typedef struct command_fly_targeting
{
	BYTE		bHeader;
	DWORD		dwTargetVID;
	long		x, y;
} TPacketCGFlyTargeting;

typedef struct packet_fly_targeting
{
	BYTE		bHeader;
	DWORD		dwShooterVID;
	DWORD		dwTargetVID;
	long		x, y;
} TPacketGCFlyTargeting;

typedef struct packet_shoot
{
	BYTE		bHeader;
	BYTE		bType;
} TPacketCGShoot;

typedef struct packet_duel_start
{
	BYTE	header;
	WORD	wSize;	// DWORD�� �? ���� = (wSize - sizeof(TPacketGCPVPList)) / 4
} TPacketGCDuelStart;

enum EPVPModes
{
	PVP_MODE_NONE,
	PVP_MODE_AGREE,
	PVP_MODE_FIGHT,
	PVP_MODE_REVENGE
};

typedef struct packet_pvp
{
	BYTE        bHeader;
	DWORD       dwVIDSrc;
	DWORD       dwVIDDst;
	BYTE        bMode;	// 0 �̸� ��, 1�̸� ��
} TPacketGCPVP;

typedef struct command_use_skill
{
	BYTE	bHeader;
	DWORD	dwVnum;
	DWORD	dwVID;
} TPacketCGUseSkill;

typedef struct command_target
{
	BYTE	header;
	DWORD	dwVID;
} TPacketCGTarget;

typedef struct packet_target
{
	BYTE	header;
	DWORD	dwVID;
	BYTE	bHPPercent;
#ifdef __VIEW_TARGET_DECIMAL_HP__
	int		iMinHP;
	int		iMaxHP;
#endif
#if defined(__SHIP_DEFENSE__)
	bool bAlliance;
	int64_t iAllianceMinHP, iAllianceMaxHP;
#endif

#ifdef ELEMENT_TARGET
	BYTE	bElement;
#endif

} TPacketGCTarget;

#ifdef __SEND_TARGET_INFO__
typedef struct packet_target_info
{
	BYTE	header;
	DWORD	dwVID;
	DWORD	race;
	DWORD	dwVnum;
	WORD	count;
#ifdef ENABLE_SEND_TARGET_INFO_EXTENDED
	DWORD	rarity;
#endif
} TPacketGCTargetInfo;

typedef struct packet_target_info_load
{
	BYTE header;
	DWORD dwVID;
} TPacketCGTargetInfoLoad;
#endif

#ifdef ENABLE_DRAGONSOUL_ALCHEMY_PLUS
typedef struct SPacketGCOpenDragonSoulAttrChange
{
	BYTE header;
} TPacketGCOpenDragonSoulChangeAttr;

typedef struct SPacketGCDragonSoulChangeAttrResult
{
	BYTE header;
	bool result;
} TPacketGCDragonSoulChangeAttrResult;
#endif

typedef struct packet_warp
{
	BYTE	bHeader;
	long	lX;
	long	lY;
	long	lAddr;
	WORD	wPort;
} TPacketGCWarp;

typedef struct command_warp
{
	BYTE	bHeader;
} TPacketCGWarp;

struct packet_quest_info
{
	BYTE header;
	WORD size;
	WORD index;
#ifdef __QUEST_RENEWAL__
	WORD c_index;
#endif
	BYTE flag;
};

enum
{
#ifdef ENABLE_MESSENGER_TEAM
	MESSENGER_SUBHEADER_GC_TEAM_LIST,
	MESSENGER_SUBHEADER_GC_TEAM_LOGIN,
	MESSENGER_SUBHEADER_GC_TEAM_LOGOUT,
#endif
	MESSENGER_SUBHEADER_GC_LIST,
	MESSENGER_SUBHEADER_GC_LOGIN,
	MESSENGER_SUBHEADER_GC_LOGOUT,
	MESSENGER_SUBHEADER_GC_INVITE,
#ifdef __FIX_DELETE_FRIEND_REFRESH__
	MESSENGER_SUBHEADER_GC_REMOVE_FRIEND,
#endif
};

typedef struct packet_messenger
{
	BYTE header;
	WORD size;
	BYTE subheader;
} TPacketGCMessenger;

typedef struct packet_messenger_guild_list
{
	BYTE connected;
	BYTE length;
	//char login[LOGIN_MAX_LEN+1];
} TPacketGCMessengerGuildList;

typedef struct packet_messenger_guild_login
{
	BYTE length;
	//char login[LOGIN_MAX_LEN+1];
} TPacketGCMessengerGuildLogin;

typedef struct packet_messenger_guild_logout
{
	BYTE length;

	//char login[LOGIN_MAX_LEN+1];
} TPacketGCMessengerGuildLogout;

typedef struct packet_messenger_list_offline
{
	BYTE connected; // always 0
	BYTE length;
} TPacketGCMessengerListOffline;

#ifdef ENABLE_MESSENGER_TEAM
typedef struct packet_messenger_team_list_offline
{
	BYTE connected;
	BYTE length;
} TPacketGCMessengerTeamListOffline;

typedef struct packet_messenger_team_list_online
{
	BYTE connected;
	BYTE length;
} TPacketGCMessengerTeamListOnline;
#endif

typedef struct packet_messenger_list_online
{
	BYTE connected; // always 1
	BYTE length;
} TPacketGCMessengerListOnline;

enum
{
	MESSENGER_SUBHEADER_CG_ADD_BY_VID,
	MESSENGER_SUBHEADER_CG_ADD_BY_NAME,
	MESSENGER_SUBHEADER_CG_REMOVE,
	MESSENGER_SUBHEADER_CG_INVITE_ANSWER,
};

typedef struct command_messenger
{
	BYTE header;
	BYTE subheader;
} TPacketCGMessenger;

typedef struct command_messenger_add_by_vid
{
	DWORD vid;
} TPacketCGMessengerAddByVID;

typedef struct command_messenger_add_by_name
{
	BYTE length;
	//char login[LOGIN_MAX_LEN+1];
} TPacketCGMessengerAddByName;

typedef struct command_messenger_remove
{
	char login[LOGIN_MAX_LEN+1];
	//DWORD account;
} TPacketCGMessengerRemove;

typedef struct command_safebox_checkout
{
	BYTE	bHeader;
#if defined(__EXTENDED_SAFEBOX__)
	DWORD	bSafePos;
#else
	BYTE	bSafePos;
#endif
	TItemPos	ItemPos;
} TPacketCGSafeboxCheckout;

typedef struct command_safebox_checkin
{
	BYTE	bHeader;
#if defined(__EXTENDED_SAFEBOX__)
	DWORD	bSafePos;
#else
	BYTE	bSafePos;
#endif
	TItemPos	ItemPos;
} TPacketCGSafeboxCheckin;

///////////////////////////////////////////////////////////////////////////////////
// Party

typedef struct command_party_parameter
{
	BYTE	bHeader;
	BYTE	bDistributeMode;
} TPacketCGPartyParameter;

typedef struct paryt_parameter
{
	BYTE	bHeader;
	BYTE	bDistributeMode;
} TPacketGCPartyParameter;

typedef struct packet_party_add
{
	BYTE	header;
	DWORD	pid;
	char	name[CHARACTER_NAME_MAX_LEN+1];
} TPacketGCPartyAdd;

typedef struct command_party_invite
{
	BYTE	header;
	DWORD	vid;
} TPacketCGPartyInvite;

typedef struct packet_party_invite
{
	BYTE	header;
	DWORD	leader_vid;
} TPacketGCPartyInvite;

typedef struct command_party_invite_answer
{
	BYTE	header;
	DWORD	leader_vid;
	BYTE	accept;
} TPacketCGPartyInviteAnswer;

typedef struct packet_party_update
{
	BYTE	header;
	DWORD	pid;
	BYTE	role;
	BYTE	percent_hp;
	short	affects[7];
} TPacketGCPartyUpdate;

typedef struct packet_party_remove
{
	BYTE header;
	DWORD pid;
} TPacketGCPartyRemove;

typedef struct packet_party_link
{
	BYTE header;
	DWORD pid;
	DWORD vid;
} TPacketGCPartyLink;

typedef struct packet_party_unlink
{
	BYTE header;
	DWORD pid;
	DWORD vid;
} TPacketGCPartyUnlink;

typedef struct command_party_remove
{
	BYTE header;
	DWORD pid;
} TPacketCGPartyRemove;

typedef struct command_party_set_state
{
	BYTE header;
	DWORD pid;
	BYTE byRole;
	BYTE flag;
} TPacketCGPartySetState;

enum
{
	PARTY_SKILL_HEAL = 1,
	PARTY_SKILL_WARP = 2
};

typedef struct command_party_use_skill
{
	BYTE header;
	BYTE bySkillIndex;
	DWORD vid;
} TPacketCGPartyUseSkill;

typedef struct packet_safebox_size
{
	BYTE bHeader;
	BYTE bSize;
} TPacketCGSafeboxSize;

typedef struct packet_safebox_wrong_password
{
	BYTE	bHeader;
} TPacketCGSafeboxWrongPassword;

typedef struct command_empire
{
	BYTE	bHeader;
	BYTE	bEmpire;
} TPacketCGEmpire;

typedef struct packet_empire
{
	BYTE	bHeader;
	BYTE	bEmpire;
} TPacketGCEmpire;

enum
{
	SAFEBOX_MONEY_STATE_SAVE,
	SAFEBOX_MONEY_STATE_WITHDRAW,
};

typedef struct command_safebox_money
{
	BYTE        bHeader;
	BYTE        bState;
	long	lMoney;
} TPacketCGSafeboxMoney;

typedef struct packet_safebox_money_change
{
	BYTE	bHeader;
	long	lMoney;
} TPacketGCSafeboxMoneyChange;

// Guild

enum
{
	GUILD_SUBHEADER_GC_LOGIN,
	GUILD_SUBHEADER_GC_LOGOUT,
	GUILD_SUBHEADER_GC_LIST,
	GUILD_SUBHEADER_GC_GRADE,
	GUILD_SUBHEADER_GC_ADD,
	GUILD_SUBHEADER_GC_REMOVE,
	GUILD_SUBHEADER_GC_GRADE_NAME,
	GUILD_SUBHEADER_GC_GRADE_AUTH,
	GUILD_SUBHEADER_GC_INFO,
	GUILD_SUBHEADER_GC_COMMENTS,
	GUILD_SUBHEADER_GC_CHANGE_EXP,
	GUILD_SUBHEADER_GC_CHANGE_MEMBER_GRADE,
	GUILD_SUBHEADER_GC_SKILL_INFO,
	GUILD_SUBHEADER_GC_CHANGE_MEMBER_GENERAL,
	GUILD_SUBHEADER_GC_GUILD_INVITE,
	GUILD_SUBHEADER_GC_WAR,
	GUILD_SUBHEADER_GC_GUILD_NAME,
	GUILD_SUBHEADER_GC_GUILD_WAR_LIST,
	GUILD_SUBHEADER_GC_GUILD_WAR_END_LIST,
	GUILD_SUBHEADER_GC_WAR_SCORE,
	GUILD_SUBHEADER_GC_MONEY_CHANGE,
};

enum GUILD_SUBHEADER_CG
{
	GUILD_SUBHEADER_CG_ADD_MEMBER,
	GUILD_SUBHEADER_CG_REMOVE_MEMBER,
	GUILD_SUBHEADER_CG_CHANGE_GRADE_NAME,
	GUILD_SUBHEADER_CG_CHANGE_GRADE_AUTHORITY,
	GUILD_SUBHEADER_CG_OFFER,
	GUILD_SUBHEADER_CG_POST_COMMENT,
	GUILD_SUBHEADER_CG_DELETE_COMMENT,
	GUILD_SUBHEADER_CG_REFRESH_COMMENT,
	GUILD_SUBHEADER_CG_CHANGE_MEMBER_GRADE,
	GUILD_SUBHEADER_CG_USE_SKILL,
	GUILD_SUBHEADER_CG_CHANGE_MEMBER_GENERAL,
	GUILD_SUBHEADER_CG_GUILD_INVITE_ANSWER,
	GUILD_SUBHEADER_CG_CHARGE_GSP,
	GUILD_SUBHEADER_CG_DEPOSIT_MONEY,
	GUILD_SUBHEADER_CG_WITHDRAW_MONEY,
};

typedef struct packet_guild
{
	BYTE header;
	WORD size;
	BYTE subheader;
} TPacketGCGuild;

typedef struct packet_guild_name_t
{
	BYTE header;
	WORD size;
	BYTE subheader;
	DWORD	guildID;
	char	guildName[GUILD_NAME_MAX_LEN];
} TPacketGCGuildName;

typedef struct packet_guild_war
{
	DWORD	dwGuildSelf;
	DWORD	dwGuildOpp;
	BYTE	bType;
	BYTE 	bWarState;
} TPacketGCGuildWar;

typedef struct command_guild
{
	BYTE header;
	BYTE subheader;
} TPacketCGGuild;

typedef struct command_guild_answer_make_guild
{
	BYTE header;
	char guild_name[GUILD_NAME_MAX_LEN+1];
} TPacketCGAnswerMakeGuild;

typedef struct command_guild_use_skill
{
	DWORD	dwVnum;
	DWORD	dwPID;
} TPacketCGGuildUseSkill;

// Guild Mark
typedef struct command_mark_login
{
	BYTE    header;
	DWORD   handle;
	DWORD   random_key;
} TPacketCGMarkLogin;

typedef struct command_mark_upload
{
	BYTE	header;
	DWORD	gid;
	BYTE	image[16*12*4];
} TPacketCGMarkUpload;

typedef struct command_mark_idxlist
{
	BYTE	header;
} TPacketCGMarkIDXList;

typedef struct command_mark_crclist
{
	BYTE	header;
	BYTE	imgIdx;
	DWORD	crclist[80];
} TPacketCGMarkCRCList;

typedef struct packet_mark_idxlist
{
	BYTE    header;
	DWORD	bufSize;
	WORD	count;
	//�ڿ� size * (WORD + WORD)��ŭ ������ ����
} TPacketGCMarkIDXList;

typedef struct packet_mark_block
{
	BYTE	header;
	DWORD	bufSize;
	BYTE	imgIdx;
	DWORD	count;
	// �ڿ� 64 x 48 x �ȼ�ũ��(4����Ʈ) = 12288��ŭ ������ ����
} TPacketGCMarkBlock;

typedef struct command_symbol_upload
{
	BYTE	header;
	WORD	size;
	DWORD	guild_id;
} TPacketCGGuildSymbolUpload;

typedef struct command_symbol_crc
{
	BYTE header;
	DWORD guild_id;
	DWORD crc;
	DWORD size;
} TPacketCGSymbolCRC;

typedef struct packet_symbol_data
{
	BYTE header;
	WORD size;
	DWORD guild_id;
} TPacketGCGuildSymbolData;

// Fishing

typedef struct command_fishing
{
	BYTE header;
	BYTE dir;
} TPacketCGFishing;

typedef struct packet_fishing
{
	BYTE header;
	BYTE subheader;
	DWORD info;
	BYTE dir;
} TPacketGCFishing;

enum
{
	FISHING_SUBHEADER_GC_START,
	FISHING_SUBHEADER_GC_STOP,
	FISHING_SUBHEADER_GC_REACT,
	FISHING_SUBHEADER_GC_SUCCESS,
	FISHING_SUBHEADER_GC_FAIL,
	FISHING_SUBHEADER_GC_FISH,
};

typedef struct command_give_item
{
	BYTE byHeader;
	DWORD dwTargetVID;
	TItemPos ItemPos;
	BYTE byItemCount;
} TPacketCGGiveItem;

typedef struct SPacketCGHack
{
	BYTE	bHeader;
	char	szBuf[255 + 1];
} TPacketCGHack;

// SubHeader - Dungeon
enum
{
	DUNGEON_SUBHEADER_GC_TIME_ATTACK_START = 0,
	DUNGEON_SUBHEADER_GC_DESTINATION_POSITION = 1,
};

typedef struct packet_dungeon
{
	BYTE bHeader;
	WORD size;
	BYTE subheader;
} TPacketGCDungeon;

typedef struct packet_dungeon_dest_position
{
	long x;
	long y;
} TPacketGCDungeonDestPosition;

typedef struct SPacketGCShopSign
{
	BYTE	bHeader;
	DWORD	dwVID;
	char	szSign[SHOP_SIGN_MAX_LEN + 1];
} TPacketGCShopSign;

typedef struct SPacketCGMyShop
{
	BYTE	bHeader;
	char	szSign[SHOP_SIGN_MAX_LEN + 1];
	BYTE	bCount;
#ifdef OFFLINE_SHOP
	BYTE	days;
#endif
} TPacketCGMyShop;

typedef struct SPacketGCTime
{
	BYTE	bHeader;
	time_t	time;
} TPacketGCTime;

enum
{
	WALKMODE_RUN,
	WALKMODE_WALK,
};

typedef struct SPacketGCWalkMode
{
	BYTE	header;
	DWORD	vid;
	BYTE	mode;
} TPacketGCWalkMode;

typedef struct SPacketGCChangeSkillGroup
{
	BYTE        header;
	BYTE        skill_group;
} TPacketGCChangeSkillGroup;

typedef struct SPacketCGRefine
{
	BYTE	header;
// #ifdef ENABLE_CUSTOM_INVENTORY
	// WORD	pos;
// #else
	BYTE	pos;
// #endif
	BYTE	type;
} TPacketCGRefine;

typedef struct SPacketCGRequestRefineInfo
{
	BYTE	header;
	BYTE	pos;
} TPacketCGRequestRefineInfo;

typedef struct SPacketGCRefineInformaion
{
	BYTE	header;
	BYTE	type;
	BYTE	pos;
	DWORD	src_vnum;
	DWORD	result_vnum;
	BYTE	material_count;
	int		cost; // �ҿ� ���
	int		prob; // Ȯ��
	TRefineMaterial materials[REFINE_MATERIAL_MAX_NUM];
} TPacketGCRefineInformation;

struct TNPCPosition
{
	BYTE bType;
	char name[CHARACTER_NAME_MAX_LEN+1];
	long x;
	long y;
};

typedef struct SPacketGCNPCPosition
{
	BYTE header;
	WORD size;
	WORD count;

	// array of TNPCPosition
} TPacketGCNPCPosition;

typedef struct SPacketGCSpecialEffect
{
	BYTE header;
	BYTE type;
	DWORD vid;
} TPacketGCSpecialEffect;

typedef struct SPacketCGChangeName
{
	BYTE header;
	BYTE index;
	char name[CHARACTER_NAME_MAX_LEN+1];
} TPacketCGChangeName;

typedef struct SPacketGCChangeName
{
	BYTE header;
	DWORD pid;
	char name[CHARACTER_NAME_MAX_LEN+1];
} TPacketGCChangeName;


typedef struct command_client_version
{
	BYTE header;
	char filename[32+1];
	char timestamp[32+1];
} TPacketCGClientVersion;

typedef struct command_client_version2
{
	BYTE header;
	char filename[32+1];
	char timestamp[32+1];
} TPacketCGClientVersion2;

typedef struct packet_channel
{
	BYTE header;
	BYTE channel;
} TPacketGCChannel;

typedef struct SEquipmentItemSet
{
	DWORD   vnum;
	BYTE    count;
	long    alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
} TEquipmentItemSet;

typedef struct pakcet_view_equip
{
	BYTE	header;
	DWORD	vid;
	struct {
		DWORD	vnum;
		WORD	count;
#ifdef __CHANGELOOK_SYSTEM__
		DWORD	transmutation;
#endif
		DWORD   dwRefineElement;
		long	alSockets[ITEM_SOCKET_MAX_NUM];
		TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
	} equips[22];
} TPacketViewEquip;

typedef struct
{
	DWORD	dwID;
	long	x, y;
	long	width, height;
	DWORD	dwGuildID;
} TLandPacketElement;

typedef struct packet_land_list
{
	BYTE	header;
	WORD	size;
} TPacketGCLandList;

typedef struct
{
	BYTE	bHeader;
	long	lID;
	char	szName[32+1];
	DWORD	dwVID;
	BYTE	bType;
} TPacketGCTargetCreate;

typedef struct
{
	BYTE	bHeader;
	long	lID;
	long	lX, lY;
} TPacketGCTargetUpdate;

typedef struct
{
	BYTE	bHeader;
	long	lID;
} TPacketGCTargetDelete;

typedef struct
{
	BYTE		bHeader;
	TPacketAffectElement elem;
} TPacketGCAffectAdd;

typedef struct
{
	BYTE	bHeader;
	DWORD	dwType;
	BYTE	bApplyOn;
} TPacketGCAffectRemove;

typedef struct packet_lover_info
{
	BYTE header;
	char name[CHARACTER_NAME_MAX_LEN + 1];
	BYTE love_point;
} TPacketGCLoverInfo;

typedef struct packet_love_point_update
{
	BYTE header;
	BYTE love_point;
} TPacketGCLovePointUpdate;

// MINING
typedef struct packet_dig_motion
{
	BYTE header;
	DWORD vid;
	DWORD target_vid;
	BYTE count;
} TPacketGCDigMotion;
// END_OF_MINING

// SCRIPT_SELECT_ITEM
typedef struct command_script_select_item
{
	BYTE header;
	DWORD selection;
} TPacketCGScriptSelectItem;
// END_OF_SCRIPT_SELECT_ITEM

typedef struct packet_damage_info
{
	BYTE header;
	DWORD dwVID;
	BYTE flag;
	int damage;
} TPacketGCDamageInfo;

typedef struct tag_GGSiege
{
	BYTE	bHeader;
	BYTE	bEmpire;
	BYTE	bTowerCount;
} TPacketGGSiege;

typedef struct SPacketGGMonarchTransfer
{
	BYTE	bHeader;
	DWORD	dwTargetPID;
	long	x;
	long	y;
} TPacketMonarchGGTransfer;

typedef struct SPacketGGCheckAwakeness
{
	BYTE bHeader;
} TPacketGGCheckAwakeness;

typedef struct SPacketGCPanamaPack
{
	BYTE	bHeader;
	char	szPackName[256];
	BYTE	abIV[32];
} TPacketGCPanamaPack;

//TODO :  �ƿ� ¯��..������Ŷ ������ �޾Ƶ��ϼ� �ְ� ��Ŷ �ڵ鷯 Refactoring ����.
typedef struct SPacketGCHybridCryptKeys
{
	SPacketGCHybridCryptKeys() : m_pStream(NULL) {}
	~SPacketGCHybridCryptKeys()
	{
		//GCC ���� NULL delete �ص� ������? �ϴ� �����ϰ� NULL üũ ����. ( �ٵ� �̰� C++ ǥ�ؾƴϾ��� --a )
		if( m_pStream )
		{
			delete[] m_pStream;
			m_pStream = NULL;
		}
	}

	DWORD GetStreamSize()
	{
		return sizeof(bHeader) + sizeof(WORD) + sizeof(int) + KeyStreamLen;
	}

	BYTE* GetStreamData()
	{
		if( m_pStream )
			delete[] m_pStream;

		uDynamicPacketSize = (WORD)GetStreamSize();

		m_pStream = new BYTE[ uDynamicPacketSize ];

		memcpy( m_pStream, &bHeader, 1 );
		memcpy( m_pStream+1, &uDynamicPacketSize, 2 );
		memcpy( m_pStream+3, &KeyStreamLen, 4 );

		if( KeyStreamLen > 0 )
			memcpy( m_pStream+7, pDataKeyStream, KeyStreamLen );

		return m_pStream;
	}

	BYTE	bHeader;
	WORD    uDynamicPacketSize; // ������� Ŭ��  DynamicPacketHeader ���������� ��������Ѵ� -_-;
	int		KeyStreamLen;
	BYTE*   pDataKeyStream;

private:
	BYTE* m_pStream;


} TPacketGCHybridCryptKeys;


typedef struct SPacketGCPackageSDB
{
	SPacketGCPackageSDB() : m_pDataSDBStream(NULL), m_pStream(NULL) {}
	~SPacketGCPackageSDB()
	{
		if( m_pStream )
		{
			delete[] m_pStream;
			m_pStream = NULL;
		}
	}

	DWORD GetStreamSize()
	{
		return sizeof(bHeader) + sizeof(WORD) + sizeof(int) + iStreamLen;
	}

	BYTE* GetStreamData()
	{
		if( m_pStream )
			delete[] m_pStream;

		uDynamicPacketSize =  GetStreamSize();

		m_pStream = new BYTE[ uDynamicPacketSize ];

		memcpy( m_pStream, &bHeader, 1 );
		memcpy( m_pStream+1, &uDynamicPacketSize, 2 );
		memcpy( m_pStream+3, &iStreamLen, 4 );

		if( iStreamLen > 0 )
			memcpy( m_pStream+7, m_pDataSDBStream, iStreamLen );

		return m_pStream;
	}

	BYTE	bHeader;
	WORD    uDynamicPacketSize; // ������� Ŭ��  DynamicPacketHeader ���������� ��������Ѵ� -_-;
	int		iStreamLen;
	BYTE*   m_pDataSDBStream;

private:
	BYTE* m_pStream;


} TPacketGCPackageSDB;

#ifdef _IMPROVED_PACKET_ENCRYPTION_
struct TPacketKeyAgreement
{
	static const int MAX_DATA_LEN = 256;
	BYTE bHeader;
	WORD wAgreedLength;
	WORD wDataLength;
	BYTE data[MAX_DATA_LEN];
};

struct TPacketKeyAgreementCompleted
{
	BYTE bHeader;
	BYTE data[3]; // dummy (not used)
};

#endif // _IMPROVED_PACKET_ENCRYPTION_

#define MAX_EFFECT_FILE_NAME 128
typedef struct SPacketGCSpecificEffect
{
	BYTE header;
	DWORD vid;
	char effect_file[MAX_EFFECT_FILE_NAME];
} TPacketGCSpecificEffect;

// ��ȥ��
enum EDragonSoulRefineWindowRefineType
{
	DragonSoulRefineWindow_UPGRADE,
	DragonSoulRefineWindow_IMPROVEMENT,
	DragonSoulRefineWindow_REFINE,
};

enum EPacketCGDragonSoulSubHeaderType
{
	DS_SUB_HEADER_OPEN,
	DS_SUB_HEADER_CLOSE,
	DS_SUB_HEADER_DO_REFINE_GRADE,
	DS_SUB_HEADER_DO_REFINE_STEP,
	DS_SUB_HEADER_DO_REFINE_STRENGTH,
	DS_SUB_HEADER_REFINE_FAIL,
	DS_SUB_HEADER_REFINE_FAIL_MAX_REFINE,
	DS_SUB_HEADER_REFINE_FAIL_INVALID_MATERIAL,
	DS_SUB_HEADER_REFINE_FAIL_NOT_ENOUGH_MONEY,
	DS_SUB_HEADER_REFINE_FAIL_NOT_ENOUGH_MATERIAL,
	DS_SUB_HEADER_REFINE_FAIL_TOO_MUCH_MATERIAL,
	DS_SUB_HEADER_REFINE_SUCCEED,
	DS_SUB_HEADER_REFINE_ALL,
};
typedef struct SPacketCGDragonSoulRefine
{
	SPacketCGDragonSoulRefine() : header (HEADER_CG_DRAGON_SOUL_REFINE)
	{}
	BYTE header;
	BYTE bSubType;
	TItemPos ItemGrid[DRAGON_SOUL_REFINE_GRID_SIZE];
} TPacketCGDragonSoulRefine;

typedef struct SPacketGCDragonSoulRefine
{
	SPacketGCDragonSoulRefine() : header(HEADER_GC_DRAGON_SOUL_REFINE)
	{}
	BYTE header;
	BYTE bSubType;
	TItemPos Pos;
} TPacketGCDragonSoulRefine;

typedef struct SPacketCGStateCheck
{
	BYTE header;
	unsigned long key;
	unsigned long index;
} TPacketCGStateCheck;

typedef struct SPacketGCStateCheck
{
	BYTE header;
	unsigned long key;
	unsigned long index;
	unsigned char state;
} TPacketGCStateCheck;

#ifdef __SASH_SYSTEM__
enum
{
	HEADER_CG_SASH = 230,
	HEADER_GC_SASH = 231,
	SASH_SUBHEADER_GC_OPEN = 0,
	SASH_SUBHEADER_GC_CLOSE,
	SASH_SUBHEADER_GC_ADDED,
	SASH_SUBHEADER_GC_REMOVED,
	SASH_SUBHEADER_CG_REFINED,
	SASH_SUBHEADER_CG_CLOSE = 0,
	SASH_SUBHEADER_CG_ADD,
	SASH_SUBHEADER_CG_REMOVE,
	SASH_SUBHEADER_CG_REFINE,
};

typedef struct SPacketSash
{
	BYTE	header;
	BYTE	subheader;
	bool	bWindow;
	DWORD	dwPrice;
	BYTE	bPos;
	TItemPos	tPos;
	DWORD	dwItemVnum;
	DWORD	dwMinAbs;
	DWORD	dwMaxAbs;
} TPacketSash;
#endif

#ifdef __CHANGELOOK_SYSTEM__
enum
{
	HEADER_CG_CL = 233,
	HEADER_GC_CL = 234,
	CL_SUBHEADER_OPEN = 0,
	CL_SUBHEADER_CLOSE,
	CL_SUBHEADER_ADD,
	CL_SUBHEADER_REMOVE,
	CL_SUBHEADER_REFINE,
};

typedef struct SPacketChangeLook
{
	BYTE	header;
	BYTE	subheader;
	DWORD	dwCost;
	BYTE	bPos;
	TItemPos	tPos;
} TPacketChangeLook;
#endif

#ifdef ENABLE_FISH_EVENT
enum
{
	HEADER_CG_FISH_EVENT_SEND = 219,
	HEADER_GC_FISH_EVENT_INFO = 223,
};

enum
{
	FISH_EVENT_SUBHEADER_BOX_USE,
	FISH_EVENT_SUBHEADER_SHAPE_ADD,
	FISH_EVENT_SUBHEADER_GC_REWARD,
	FISH_EVENT_SUBHEADER_GC_ENABLE,
};

typedef struct SPacketGCFishEvent {
	BYTE	bHeader;
	BYTE	bSubheader;
} TPacketCGFishEvent;

typedef struct SPacketGCFishEventInfo {
	BYTE	bHeader;
	BYTE	bSubheader;
	DWORD	dwFirstArg;
	DWORD	dwSecondArg;
} TPacketGCFishEventInfo;
#endif

#ifdef ENABLE_CUBE_RENEWAL_WORLDARD

enum
{
	CUBE_RENEWAL_SUB_HEADER_OPEN_RECEIVE,
	CUBE_RENEWAL_SUB_HEADER_CLEAR_DATES_RECEIVE,
	CUBE_RENEWAL_SUB_HEADER_DATES_RECEIVE,
	CUBE_RENEWAL_SUB_HEADER_DATES_LOADING,

	CUBE_RENEWAL_SUB_HEADER_MAKE_ITEM,
	CUBE_RENEWAL_SUB_HEADER_CLOSE,
};

typedef struct  packet_send_cube_renewal
{
	BYTE header;
	BYTE subheader;
	DWORD index_item;
	DWORD count_item;
	DWORD index_item_improve;
}TPacketCGCubeRenewalSend;


typedef struct dates_cube_renewal
{
	DWORD npc_vnum;
	DWORD index;

	DWORD	vnum_reward;
	int		count_reward;

	bool 	item_reward_stackable;

	DWORD	vnum_material_1;
	int		count_material_1;

	DWORD	vnum_material_2;
	int		count_material_2;

	DWORD	vnum_material_3;
	int		count_material_3;

	DWORD	vnum_material_4;
	int		count_material_4;

	DWORD	vnum_material_5;
	int		count_material_5;

	unsigned long long 	gold;
	int 	percent;

	char 	category[100];
}TInfoDateCubeRenewal;

typedef struct packet_receive_cube_renewal
{
	packet_receive_cube_renewal(): header(HEADER_GC_CUBE_RENEWAL)
	{}

	BYTE header;
	BYTE subheader;
	TInfoDateCubeRenewal	date_cube_renewal;
}TPacketGCCubeRenewalReceive;
#endif


#ifdef ENABLE_REMOVE_LIMIT_GOLD
enum {
	HEADER_GC_CHARACTER_GOLD 				= 224,
	HEADER_GC_CHARACTER_GOLD_CHANGE 		= 225,	
};

typedef struct packet_gold
{
	BYTE	header;
	unsigned long long		gold;
} TPacketGCGold;

typedef struct packet_gold_change
{
	int		header;
	DWORD	dwVID;
	long long	amount;
	unsigned long long	value;
} TPacketGCGoldChange;
#endif

typedef struct SItemData
{
    DWORD       vnum;
    WORD        count;
#ifdef ENABLE_REFINE_ELEMENT
	DWORD		dwRefineElement;
#endif
#ifdef __CHANGELOOK_SYSTEM__
	DWORD		transmutation;
#endif
	DWORD		flags;
	DWORD		anti_flags;
	long		alSockets[ITEM_SOCKET_MAX_NUM];
    TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
} TItemData;

#ifdef __AURA_SYSTEM__
enum EPacketGCAuraSubHeader
{
	AURA_SUBHEADER_GC_OPEN,
	AURA_SUBHEADER_GC_CLOSE,
	AURA_SUBHEADER_GC_SET_ITEM,
	AURA_SUBHEADER_GC_CLEAR_SLOT,
	AURA_SUBHEADER_GC_CLEAR_ALL,
	AURA_SUBHEADER_GC_CLEAR_RIGHT,
	AURA_SUBHEADER_GC_REFINE_INFO,
};

typedef struct SSubPacketGCAuraOpenClose
{
	BYTE	bAuraWindowType;
} TSubPacketGCAuraOpenClose;

typedef struct SSubPacketGCAuraSetItem
{
	TItemPos	Cell;
	TItemPos	AuraCell;
	TItemData	pItem;
} TSubPacketGCAuraSetItem;

typedef struct SSubPacketGCAuraClearSlot
{
	BYTE	bAuraSlotPos;
} TSubPacketGCAuraClearSlot;

typedef struct SSubPacketGCAuraRefineInfo
{
	BYTE	bAuraRefineInfoType;
	BYTE	bAuraRefineInfoLevel;
	BYTE	bAuraRefineInfoExpPercent;
} TSubPacketGCAuraRefineInfo;

enum EPacketCGAuraSubHeader
{
	AURA_SUBHEADER_CG_REFINE_CHECKIN,
	AURA_SUBHEADER_CG_REFINE_CHECKOUT,
	AURA_SUBHEADER_CG_REFINE_ACCEPT,
	AURA_SUBHEADER_CG_REFINE_CANCEL,
};

typedef struct SSubPacketCGAuraRefineCheckIn
{
	TItemPos	ItemCell;
	TItemPos	AuraCell;
	BYTE		byAuraRefineWindowType;
} TSubPacketCGAuraRefineCheckIn;

typedef struct SSubPacketCGAuraRefineCheckOut
{
	TItemPos	AuraCell;
	BYTE		byAuraRefineWindowType;
} TSubPacketCGAuraRefineCheckOut;

typedef struct SSubPacketCGAuraRefineAccept
{
	BYTE		byAuraRefineWindowType;
} TSubPacketCGAuraRefineAccept;

typedef struct SPacketGCAura
{
	SPacketGCAura() : bHeader(HEADER_GC_AURA) {}
	BYTE bHeader;
	WORD wSize;
	BYTE bSubHeader;
} TPacketGCAura;

typedef struct SPacketCGAura
{
	SPacketCGAura() : bHeader(HEADER_CG_AURA) {}
	BYTE bHeader;
	WORD wSize;
	BYTE bSubHeader;
} TPacketCGAura;
#endif

#ifdef ENABLE_REFINE_ELEMENT
enum {
	HEADER_CG_REFINE_ELEMENT 					= 227,
	HEADER_GC_REFINE_ELEMENT 					= 228,
};

typedef struct SPacketCGRefineElement
{
	BYTE	bHeader;
	BYTE	bArg;
} TPacketCGRefineElement;

typedef struct SPacketGCRefineElement
{
	BYTE	bHeader;
	WORD	wSrcCell;
	WORD	wDstCell;
	BYTE	bType;
} TPacketGCRefineElement;
#endif
#if defined(__ATTR_6TH_7TH__)
enum EAttr67AddHeader
{
	HEADER_CG_ATTR67_ADD = 169,
};

enum EAttr67AddSubHeader
{
	SUBHEADER_CG_ATTR67_ADD_CLOSE,
	SUBHEADER_CG_ATTR67_ADD_OPEN,
	SUBHEADER_CG_ATTR67_ADD_REGIST,
};

typedef struct SPacketCGAttr67Add
{
	BYTE byHeader;
	BYTE bySubHeader;
	TAttr67AddData Attr67AddData;
} TPacketCGAttr67Add;
#endif
#ifdef ENABLE_MULTI_FARM_BLOCK
enum
{
	MULTI_FARM_SET,
	MULTI_FARM_REMOVE,
};

typedef struct SPacketGGMultiFarm
{
	uint8_t header;
	uint32_t size;
	uint8_t subHeader;
	char playerIP[IP_ADDRESS_LENGTH + 1];
	uint32_t playerID;
	char playerName[CHARACTER_NAME_MAX_LEN + 1];
	bool farmStatus;
	uint8_t affectType;
	int affectTime;
} TPacketGGMultiFarm;
#endif

#if defined(__DAILY_GIFT_SYSTEM__)
typedef struct SPacketGCDailyGift
{
	BYTE bHeader;

	DWORD dwCash;
	BYTE bWeek;

	DWORD dwVnum[DAILY_GIFT_WEEK_DAYS];
	DWORD dwCount[DAILY_GIFT_WEEK_DAYS];
	DWORD dwCollectTime[DAILY_GIFT_WEEK_DAYS];
	WORD wCost[DAILY_GIFT_WEEK_DAYS];
	BYTE bStatus[DAILY_GIFT_WEEK_DAYS];
} TPacketGCDailyGift;

typedef struct SPacketCGDailyGift
{
	BYTE bHeader;
	BYTE bAction;
	BYTE bSlotIndex;
} TPacketCGDailyGift;
#endif
#ifdef __ENABLE_BIOLOGIST_RENEWAL_SYSTEM__
enum BiologistHeaders
{
	HEADER_CG_BIOLOGIST = 175,
	HEADER_GC_BIOLOGIST = 140,

	BIOLOGIST_SUBHEADER_CG_OPEN = 0,
	BIOLOGIST_SUBHEADER_CG_CLOSE = 1,
	BIOLOGIST_SUBHEADER_CG_PROVIDES = 2,
	BIOLOGIST_SUBHEADER_CG_SELECTIVE = 3,
	BIOLOGIST_SUBHEADER_CG_TIME = 4,

	BIOLOGIST_SUBHEADER_GC_OPEN = 0,
	BIOLOGIST_SUBHEADER_GC_CLOSE = 1,
	BIOLOGIST_SUBHEADER_GC_PROVIDES = 2,
	BIOLOGIST_SUBHEADER_GC_SELECTIVE = 3,
	BIOLOGIST_SUBHEADER_GC_TIME = 4,
};

typedef struct SPacketCGBiologist
{
	SPacketCGBiologist() :
		byHeader(HEADER_CG_BIOLOGIST)
	{}
	BYTE byHeader;
	BYTE bySubHeader;
	BYTE byChosenAffect;
	BYTE byDecreaseTimeIndex;
	BYTE byIsElixirUse;
	BYTE byIsBookTimeUse;
} TPacketCGBiologist;

typedef struct SPacketGCBiologist
{
	SPacketGCBiologist() :
		byHeader(HEADER_GC_BIOLOGIST)
	{}
	BYTE byHeader;
	BYTE bySubHeader;
	bool bIsSelective;

	char szTitle[64];
	DWORD dwLevelLimit;
	DWORD dwItemReq;
	DWORD dwItemReqCount;
	DWORD dwItemIncreaseRate;
	DWORD dwItemForgetTime;
	DWORD dwRewardBonusVnum[3];
	DWORD dwRewardBonusValue[3];
	int iTime;
} TPacketGCBiologist;
#endif



#ifdef __ENABLE_PREMIUM_PLAYERS__
enum PremiumPlayersHeader
{
	HEADER_CG_PREMIUM_PLAYERS = 176,
	HEADER_GC_PREMIUM_PLAYERS = 141,

	PREMIUM_PLAYERS_SUBHEADER_CG_OPEN = 0,
	PREMIUM_PLAYERS_SUBHEADER_CG_LIST = 1,
	PREMIUM_PLAYERS_SUBHEADER_CG_ACTIVATE = 2,

	PREMIUM_PLAYERS_SUBHEADER_GC_OPEN = 0,
	PREMIUM_PLAYERS_SUBHEADER_GC_LIST = 1,
	PREMIUM_PLAYERS_SUBHEADER_GC_ACTIVATE = 2,
};

typedef struct SPacketCGPremiumPlayers
{
	SPacketCGPremiumPlayers() : 
		byHeader(HEADER_CG_PREMIUM_PLAYERS)
	{}
	BYTE byHeader;
	BYTE bySubHeader;
} TPacketCGPremiumPlayers;

typedef struct SPacketGCPremiumPlayers
{
	SPacketGCPremiumPlayers() :
		byHeader(HEADER_GC_PREMIUM_PLAYERS)
	{}
	BYTE byHeader;
	BYTE bySubHeader;
	BYTE byPos;
	char szName[CHARACTER_NAME_MAX_LEN + 1];
} TPacketGCPremiumPlayers;
#endif

#ifdef __MULTI_LANGUAGE_SYSTEM__
typedef struct SPacketGGLocaleNotice
{
	BYTE bHeader;
	bool bBigFont;
	char szNotice[MAX_QUEST_NOTICE_ARGS + 1][CHAT_MAX_LEN + 1];
} TPacketGGLocaleNotice;

typedef struct SPacketChangeLanguage
{
	BYTE bHeader;
	BYTE bLanguage;
} TPacketChangeLanguage;
#endif

#ifdef __EXTENDED_WHISPER_DETAILS__
typedef struct SPacketCGWhisperDetails
{
	BYTE header;
	char name[CHARACTER_NAME_MAX_LEN + 1];
} TPacketCGWhisperDetails;

typedef struct SPacketGCWhisperDetails
{
	BYTE header;
	char name[CHARACTER_NAME_MAX_LEN + 1];
#ifdef __MULTI_LANGUAGE_SYSTEM__
	BYTE bLanguage;
#endif
} TPacketGCWhisperDetails;
#endif

#ifdef ENABLE_GAYA_SYSTEM
enum
{
	GAYA_SYSTEM_SUB_HEADER_CRAFT,
	GAYA_SYSTEM_SUB_HEADER_MARKET,
	GAYA_SYSTEM_SUB_HEADER_REFRESH,
};

typedef struct packet_gaya_system
{
	BYTE header;
	BYTE subheader;
	int  pos;

} TPacketCGGayaSystem;
#endif

#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
enum EInventoryProtectedEnum
{
	HEADER_CG_INVENTORY_PROTECTED = 144,

	SUBHEADER_CG_INVENTORY_PROTECTED_ACTIVATE = 0,
	SUBHEADER_CG_INVENTORY_PROTECTED_PASSWORD_CHANGE = 1
};

typedef struct SPacketCGInventoryProtected
{
	SPacketCGInventoryProtected() :
		byHeader(HEADER_CG_INVENTORY_PROTECTED)
	{}

	BYTE byHeader;
	BYTE bySubHeader;
	char szPasswordNow[INVENTORY_PROTECTED_PASSWORD_MAX_LEN + 1];
	char szPasswordNew[INVENTORY_PROTECTED_PASSWORD_MAX_LEN + 1];
	bool bActivated;

} TPacketCGInventoryProtected;
#endif

#ifdef ENABLE_SPECIAL_DROP_CHAT_RENEWAL
typedef struct pickup_item_packet
{
	BYTE bHeader;
	int  item_vnum;
	int  item_count;
} TPacketGCPickupItemSC;
#endif


#if defined(__WORLD_BOSS_EVENT__)
enum EWorldBossHeader
{
	// NOTE : Make sure you are not already using these
	// header numbers on `HEADER_GC` or `HEADER_CG`.
	HEADER_CG_WORLD_BOSS = 148,
	HEADER_GC_WORLD_BOSS = 148,
};

enum EWorldBossSubHeader
{
	WORLD_BOSS_SUBHEADER_GC_INFO = 0,
	WORLD_BOSS_SUBHEADER_GC_DAMAGE,
	WORLD_BOSS_SUBHEADER_GC_RANKING,

	WORLD_BOSS_SUBHEADER_CG_INFO = 0,
	WORLD_BOSS_SUBHEADER_CG_REWARD,
	WORLD_BOSS_SUBHEADER_CG_RANKING,
};

typedef struct SPacketGCWorldBoss
{
	BYTE bHeader;
	WORD wSize;
	BYTE bSubHeader;
} TPacketGCWorldBoss;

typedef struct SPacketGCWorldBossProcess
{
	SPacketGCWorldBossProcess()
		: bState(0), dwRunTimeLeft(0), dwCooldownTimeLeft(0), dwTotalDamage(0), dwMinDamage(0) {}
	DWORD bState, dwRunTimeLeft, dwCooldownTimeLeft, dwTotalDamage, dwMinDamage;
} TPacketGCWorldBossProcess;

typedef struct SPacketCGWorldBoss
{
	BYTE bHeader;
	BYTE bSubHeader;
} TPacketCGWorldBoss;
#endif

enum EntityHeader {
	HEADER_GC_ENTITY = 249,
};

using TPacketGCEntity = struct SPacketGCEntity
{
	BYTE bHeader;
	WORD wSize;
};
using TPacketEntityInfo = struct SPacketEntityInfo
{
	DWORD dwVID;
	DWORD dwRaceVNum;
	WORD wPart[CHR_EQUIPPART_NUM];
	LONG xPos, yPos;
};

#ifdef ENABLE_SWITCHBOT
struct TPacketGGSwitchbot
{
	BYTE bHeader;
	WORD wPort;
	TSwitchbotTable table;

	TPacketGGSwitchbot() : bHeader(HEADER_GG_SWITCHBOT), wPort(0)
	{
		table = {};
	}
};

enum ECGSwitchbotSubheader
{
	SUBHEADER_CG_SWITCHBOT_START,
	SUBHEADER_CG_SWITCHBOT_STOP,
};

struct TPacketCGSwitchbot
{
	BYTE header;
	int size;
	BYTE subheader;
	BYTE slot;
};

enum EGCSwitchbotSubheader
{
	SUBHEADER_GC_SWITCHBOT_UPDATE,
	SUBHEADER_GC_SWITCHBOT_UPDATE_ITEM,
	SUBHEADER_GC_SWITCHBOT_SEND_ATTRIBUTE_INFORMATION,
};

struct TPacketGCSwitchbot
{
	BYTE header;
	int size;
	BYTE subheader;
	BYTE slot;
};

struct TSwitchbotUpdateItem
{
	BYTE	slot;
	BYTE	vnum;
	BYTE	count;
	long	alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
};
#endif

#ifdef __EVENT_MANAGER__
typedef struct SPacketGGReloadEvent
{
	BYTE bHeader;
} TPacketGGReloadEvent;

typedef struct SPacketGGEvent
{
	BYTE			bHeader;
	TEventTable		table;
	bool			bState;
} TPacketGGEvent;

typedef struct SPacketGGEventHideAndSeek
{
	BYTE			bHeader;
	int				iPosition;
	int				iRound;
} TPacketGGEventHideAndSeek;

typedef struct SPacketCGRequestEventData
{
	BYTE bHeader;
	BYTE bMonth;
} TPacketCGRequestEventData;

typedef struct SPacketGCEventInfo
{
	BYTE	bHeader;
	WORD	wSize;
} TPacketGCEventInfo;

typedef struct SPacketGCEventReload
{
	BYTE	bHeader;
} TPacketGCEventReload;

typedef struct SPacketGCEventKWScore
{
	BYTE	bHeader;
	WORD	wKingdomScores[3];
} TPacketGCEventKWScore;

typedef struct SPacketEventData
{
	DWORD	dwID;
	BYTE	bType;
	long	startTime;
	long	endTime;
	int		iValue0;
	int		iValue1;
	bool	bCompleted;
} TPacketEventData;
#endif

typedef struct command_request_event_quest
{
	BYTE		bHeader;
	char		szName[QUEST_NAME_MAX_NUM + 1];
} TPacketCGRequestEventQuest;


#ifdef __ENABLE_SHAMAN_SYSTEM__
typedef struct shaman_use_skill
{
	BYTE	bHeader;
	DWORD	dwVnum;
	DWORD	dwVid;
	BYTE	dwLevel;
}TPacketGCShamanUseSkill;
#endif



#ifdef ENABLE_BATTLE_PASS
enum { BATTLEPASS_GC_SUB_INIT, BATTLEPASS_GC_SUB_UPDATE };
enum { HEADER_GC_BATTLE_PASS = 238 };
typedef struct SPacketBattlePass
{
	BYTE	bHeader;
	WORD	wSize;
	BYTE	bSubHeader;

	bool isPremium;
	int points;
} TPacketBattlePass;
typedef struct SPacketSubBattlePassUpdate
{
	uint8_t type;
	uint8_t missionVnum;

	uint32_t progress;
	uint32_t endTime;
} TPacketSubBattlePassUpdate;
#endif


#ifdef ENABLE_REWARD_SYSTEM
typedef struct SPacketGGRewardInfo
{
	BYTE	bHeader;
	BYTE	bType;
} TPacketGGRewardInfo;
#endif



#pragma pack()
#endif
