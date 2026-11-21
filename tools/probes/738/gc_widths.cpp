// Package C8 (ledger 738): i686 packed width probe for every game-to-client record
// the Rewrite sends, compiled directly against the verbatim legacy headers
// (`server/server/libthecore/typedef.h`, `server/server/common/prodomodefines.h`,
// `length.h`, `item_length.h`, `tables.h`, `server/server/game/packet.h`).
//
// Build and run (from the repository root):
//   mkdir -p .scratch/tmp && i686-linux-gnu-g++-12 -std=c++17 -static \
//     -I .scratch/probes/738/inc -I server/server/libthecore \
//     -I server/server/common -I server/server/game \
//     -o .scratch/tmp/gc_widths .scratch/probes/738/gc_widths.cpp \
//     && .scratch/tmp/gc_widths
//
// Negative controls:
//   1. Add `-DNEGATIVE_CONTROL`: must fail to compile (`TPacketGCPhase == 3`).
//   2. Compile with host 64-bit `g++`: must fail (`ControlTwoLong == 8`).

#include "stdafx.h"
#include "packet.h"
#include <cstdio>

#pragma pack(1)
struct ControlTwoLong {
    long a;
    long b;
};
#pragma pack()

// Controls required by AGENTS.md and .scratch/probes/README.md.
static_assert(sizeof(ControlTwoLong) == 8, "packed two-long control must be 8 on i686");
static_assert(sizeof(long) == 4, "i686 long must be 4");
static_assert(sizeof(time_t) == 4, "i686 time_t must be 4");
static_assert(sizeof(QWORD) == 4, "i686 QWORD (unsigned long) must be 4");
static_assert(sizeof(bool) == 1, "i686 bool must be 1");
static_assert(sizeof(TPacketGCPhase) == 2, "known control: TPacketGCPhase");
static_assert(sizeof(TPacketGCHandshake) == 13, "known control: TPacketGCHandshake");
static_assert(sizeof(TPacketGCLoginFailure) == 10, "known control: TPacketGCLoginFailure");

#ifdef NEGATIVE_CONTROL
static_assert(sizeof(TPacketGCPhase) == 3, "negative control must fail");
#endif

#define CHECK(byte, name, stype, expected, kind)                                              \
    static_assert(sizeof(stype) == (expected), "width mismatch for " #stype);                 \
    printf("%3d\t%-40s\t%-40s\t%4zu\t%s\n", (byte), (name), #stype, sizeof(stype), (kind))

#define CHECK1(byte, name)                                                                    \
    printf("%3d\t%-40s\t%-40s\t%4zu\t%s\n", (byte), (name), "(1-byte header)", (size_t)1, "fix")

int main() {
    printf("byte\trecord                                  \tstruct                                  \twidth\tkind\n");
    CHECK(1,   "HEADER_GC_CHARACTER_ADD",                TPacketGCCharacterAdd,            35, "fix");
    CHECK(2,   "HEADER_GC_CHARACTER_DEL",                TPacketGCCharacterDelete,          5, "fix");
    CHECK(3,   "HEADER_GC_CHARACTER_MOVE",               TPacketGCMove,                    24, "fix");
    CHECK(4,   "HEADER_GC_CHAT",                         TPacketGCChat,                    10, "dyn-hdr");
    CHECK(5,   "HEADER_GC_SYNC_POSITION",                TPacketGCSyncPosition,             3, "dyn-hdr");
    CHECK(5,   "  (elem) TPacketGCSyncPositionElement",  TPacketGCSyncPositionElement,     12, "dyn-elem");
    CHECK(7,   "HEADER_GC_LOGIN_FAILURE",                TPacketGCLoginFailure,            10, "fix");
    CHECK(8,   "HEADER_GC_PLAYER_CREATE_SUCCESS",        TPacketGCPlayerCreateSuccess,     72, "fix");
    CHECK(9,   "HEADER_GC_PLAYER_CREATE_FAILURE",        TPacketGCCreateFailure,            2, "fix");
    CHECK1(10, "HEADER_GC_PLAYER_DELETE_SUCCESS");
    CHECK1(11, "HEADER_GC_PLAYER_DELETE_WRONG_SOCIAL_ID");
    CHECK(13,  "HEADER_GC_STUN",                         TPacketGCStun,                     5, "fix");
    CHECK(14,  "HEADER_GC_DEAD",                         TPacketGCDead,                     5, "fix");
    CHECK(16,  "HEADER_GC_PLAYER_POINTS",                TPacketGCPoints,                2041, "fix");
    CHECK(17,  "HEADER_GC_PLAYER_POINT_CHANGE",          TPacketGCPointChange,             25, "fix");
    CHECK(18,  "HEADER_GC_CHANGE_SPEED",                 TPacketGCChangeSpeed,              7, "fix");
    CHECK(19,  "HEADER_GC_CHARACTER_UPDATE",             TPacketGCCharacterUpdate,         55, "fix");
    CHECK(20,  "HEADER_GC_ITEM_DEL (client SET)",        TPacketGCItemDelDeprecated,       62, "fix");
    CHECK(21,  "HEADER_GC_ITEM_SET (client SET2)",       TPacketGCItemSet,                 72, "fix");
    CHECK(25,  "HEADER_GC_ITEM_UPDATE",                  TPacketGCItemUpdate,              59, "fix");
    CHECK(26,  "HEADER_GC_ITEM_GROUND_ADD",              TPacketGCItemGroundAdd,           21, "fix");
    CHECK(27,  "HEADER_GC_ITEM_GROUND_DEL",              TPacketGCItemGroundDel,            5, "fix");
    CHECK(28,  "HEADER_GC_QUICKSLOT_ADD",                struct packet_quickslot_add,       4, "fix");
    CHECK(29,  "HEADER_GC_QUICKSLOT_DEL",                struct packet_quickslot_del,       2, "fix");
    CHECK(30,  "HEADER_GC_QUICKSLOT_SWAP",               struct packet_quickslot_swap,      3, "fix");
    CHECK(31,  "HEADER_GC_ITEM_OWNERSHIP",               TPacketGCItemOwnership,           30, "fix");
    CHECK(32,  "HEADER_GC_LOGIN_SUCCESS_NEWSLOT",        TPacketGCLoginSuccess,           357, "fix");
    CHECK(34,  "HEADER_GC_WHISPER",                      TPacketGCWhisper,                 30, "dyn-hdr");
    CHECK(36,  "HEADER_GC_MOTION",                       struct packet_motion,             11, "fix");
    CHECK(38,  "HEADER_GC_SHOP",                         TPacketGCShop,                     4, "dyn-hdr");
    CHECK(38,  "  (elem) packet_shop_item",              struct packet_shop_item,          73, "dyn-elem");
    CHECK(38,  "  (sub) packet_shop_start",              struct packet_shop_start,       2924, "dyn-sub");
    CHECK(38,  "  (sub) packet_shop_update_item",        struct packet_shop_update_item,   74, "dyn-sub");
    CHECK(39,  "HEADER_GC_SHOP_SIGN",                    TPacketGCShopSign,                38, "fix");
    CHECK(40,  "HEADER_GC_DUEL_START",                   TPacketGCDuelStart,                3, "dyn-hdr");
    CHECK(41,  "HEADER_GC_PVP",                          TPacketGCPVP,                     10, "fix");
    CHECK(42,  "HEADER_GC_EXCHANGE",                     struct packet_exchange,           74, "fix");
    CHECK(43,  "HEADER_GC_CHARACTER_POSITION",           struct packet_position,            6, "fix");
    CHECK(44,  "HEADER_GC_PING",                         TPacketGCPing,                     1, "fix");
    CHECK(45,  "HEADER_GC_SCRIPT",                       struct packet_script,              6, "dyn-hdr");
    CHECK(46,  "HEADER_GC_QUEST_CONFIRM",                TPacketGCQuestConfirm,            74, "fix");
    CHECK(57,  "HEADER_GC_AUTO_SHAMAN_SKILL",            TPacketGCShamanUseSkill,          10, "fix");
    CHECK(58,  "HEADER_GC_TARGET_INFO",                  TPacketGCTargetInfo,              19, "fix");
    CHECK(62,  "HEADER_GC_OWNERSHIP",                    TPacketGCOwnership,                9, "fix");
    CHECK(63,  "HEADER_GC_TARGET",                       TPacketGCTarget,                  32, "fix");
    CHECK(65,  "HEADER_GC_WARP",                         TPacketGCWarp,                    15, "fix");
    CHECK(69,  "HEADER_GC_ADD_FLY_TARGETING",            TPacketGCFlyTargeting,            17, "fix");
    CHECK(70,  "HEADER_GC_CREATE_FLY",                   TPacketGCCreateFly,               10, "fix");
    CHECK(71,  "HEADER_GC_FLY_TARGETING",                TPacketGCFlyTargeting,            17, "fix");
    CHECK(74,  "HEADER_GC_MESSENGER",                    TPacketGCMessenger,                4, "dyn-hdr");
    CHECK(75,  "HEADER_GC_GUILD",                        TPacketGCGuild,                    4, "dyn-hdr");
    CHECK(75,  "  (sub) TPacketGCGuildName",             TPacketGCGuildName,               20, "dyn-sub");
    CHECK(75,  "  (sub) TPacketGCGuildWar",              TPacketGCGuildWar,                10, "dyn-sub");
    CHECK(76,  "HEADER_GC_SKILL_LEVEL_NEW",              TPacketGCSkillLevel,            1531, "fix");
    CHECK(77,  "HEADER_GC_PARTY_INVITE",                 TPacketGCPartyInvite,              5, "fix");
    CHECK(78,  "HEADER_GC_PARTY_ADD",                    TPacketGCPartyAdd,                30, "fix");
    CHECK(79,  "HEADER_GC_PARTY_UPDATE",                 TPacketGCPartyUpdate,             21, "fix");
    CHECK(80,  "HEADER_GC_PARTY_REMOVE",                 TPacketGCPartyRemove,              5, "fix");
    CHECK(81,  "HEADER_GC_QUEST_INFO",                   struct packet_quest_info,          8, "dyn-hdr");
    CHECK1(82, "HEADER_GC_REQUEST_MAKE_GUILD");
    CHECK(83,  "HEADER_GC_PARTY_PARAMETER",              TPacketGCPartyParameter,           2, "fix");
    CHECK(84,  "HEADER_GC_SAFEBOX_MONEY_CHANGE",         TPacketGCSafeboxMoneyChange,       5, "fix");
    CHECK(85,  "HEADER_GC_SAFEBOX_SET",                  TPacketGCItemSet,                 72, "fix");
    CHECK(86,  "HEADER_GC_SAFEBOX_DEL",                  TPacketGCItemDel,                  5, "fix");
    CHECK(87,  "HEADER_GC_SAFEBOX_WRONG_PASSWORD",       TPacketCGSafeboxWrongPassword,     1, "fix");
    CHECK(88,  "HEADER_GC_SAFEBOX_SIZE",                 TPacketCGSafeboxSize,              2, "fix");
    CHECK(89,  "HEADER_GC_FISHING",                      TPacketGCFishing,                  7, "fix");
    CHECK(90,  "HEADER_GC_EMPIRE",                       TPacketGCEmpire,                   2, "fix");
    CHECK(91,  "HEADER_GC_PARTY_LINK",                   TPacketGCPartyLink,                9, "fix");
    CHECK(92,  "HEADER_GC_PARTY_UNLINK",                 TPacketGCPartyUnlink,              9, "fix");
    CHECK(99,  "HEADER_GC_VIEW_EQUIP",                   struct pakcet_view_equip,       1303, "fix");
    CHECK(100, "HEADER_GC_MARK_BLOCK",                   TPacketGCMarkBlock,               10, "dyn-hdr");
    CHECK(102, "HEADER_GC_MARK_IDXLIST",                 TPacketGCMarkIDXList,              7, "dyn-hdr");
    CHECK(106, "HEADER_GC_TIME",                         TPacketGCTime,                     5, "fix");
    CHECK(107, "HEADER_GC_CHANGE_NAME",                  TPacketGCChangeName,              30, "fix");
    CHECK(111, "HEADER_GC_WALK_MODE",                    TPacketGCWalkMode,                 6, "fix");
    CHECK(112, "HEADER_GC_CHANGE_SKILL_GROUP",           TPacketGCChangeSkillGroup,         2, "fix");
    CHECK(113, "HEADER_GC_MAIN_CHARACTER2_EMPIRE",       TPacketGCMainCharacter,           46, "fix");
    CHECK(114, "HEADER_GC_SEPCIAL_EFFECT",               TPacketGCSpecialEffect,            6, "fix");
    CHECK(115, "HEADER_GC_NPC_POSITION",                 TPacketGCNPCPosition,              5, "dyn-hdr");
    CHECK(115, "  (elem) TNPCPosition",                  TNPCPosition,                     34, "dyn-elem");
    CHECK(118, "HEADER_GC_LOGIN_KEY",                    TPacketGCLoginKey,                 5, "fix");
    CHECK(119, "HEADER_GC_REFINE_INFORMATION_NEW",       TPacketGCRefineInformation,       60, "fix");
    CHECK(121, "HEADER_GC_CHANNEL",                      TPacketGCChannel,                  2, "fix");
    CHECK(122, "HEADER_GC_MALL_OPEN",                    TPacketCGSafeboxSize,              2, "fix");
    CHECK(123, "HEADER_GC_TARGET_UPDATE",                TPacketGCTargetUpdate,            13, "fix");
    CHECK(124, "HEADER_GC_TARGET_DELETE",                TPacketGCTargetDelete,             5, "fix");
    CHECK(125, "HEADER_GC_TARGET_CREATE_NEW",            TPacketGCTargetCreate,            43, "fix");
    CHECK(126, "HEADER_GC_AFFECT_ADD",                   TPacketGCAffectAdd,               22, "fix");
    CHECK(127, "HEADER_GC_AFFECT_REMOVE",                TPacketGCAffectRemove,             6, "fix");
    CHECK(128, "HEADER_GC_MALL_SET",                     TPacketGCItemSet,                 72, "fix");
    CHECK(129, "HEADER_GC_MALL_DEL",                     TPacketGCItemDel,                  5, "fix");
    CHECK(130, "HEADER_GC_LAND_LIST",                    TPacketGCLandList,                 3, "dyn-hdr");
    CHECK(130, "  (elem) TLandPacketElement",            TLandPacketElement,               24, "dyn-elem");
    CHECK(131, "HEADER_GC_LOVER_INFO",                   TPacketGCLoverInfo,               27, "fix");
    CHECK(132, "HEADER_GC_LOVE_POINT_UPDATE",            TPacketGCLovePointUpdate,          2, "fix");
    CHECK(133, "HEADER_GC_SYMBOL_DATA",                  TPacketGCGuildSymbolData,          7, "dyn-hdr");
    CHECK(134, "HEADER_GC_DIG_MOTION",                   TPacketGCDigMotion,               10, "fix");
    CHECK(135, "HEADER_GC_DAMAGE_INFO",                  TPacketGCDamageInfo,              10, "fix");
    CHECK(136, "HEADER_GC_CHAR_ADDITIONAL_INFO",         TPacketGCCharacterAdditionalInfo, 70, "fix");
    CHECK(137, "HEADER_GC_MAIN_CHARACTER3_BGM",          TPacketGCMainCharacter3_BGM,      71, "fix");
    CHECK(138, "HEADER_GC_MAIN_CHARACTER4_BGM_VOL",      TPacketGCMainCharacter4_BGM_VOL,  75, "fix");
    CHECK(140, "HEADER_GC_BIOLOGIST",                    TPacketGCBiologist,              115, "fix");
    CHECK(141, "HEADER_GC_PREMIUM_PLAYERS",              TPacketGCPremiumPlayers,          28, "fix");
    CHECK(148, "HEADER_GC_WORLD_BOSS",                   TPacketGCWorldBoss,                4, "dyn-hdr");
    CHECK(148, "  (sub) TPacketGCWorldBossProcess",      TPacketGCWorldBossProcess,        20, "dyn-sub");
    CHECK(148, "  (sub) TPacketGDTempWorldBossRanking",  TPacketGDTempWorldBossRanking,    43, "dyn-sub");
    CHECK(150, "HEADER_GC_AUTH_SUCCESS",                 TPacketGCAuthSuccess,              6, "fix");
    CHECK(155, "HEADER_GC_EVENT_INFO",                   TPacketGCEventInfo,                3, "dyn-hdr");
    CHECK(155, "  (elem) TPacketEventData",              TPacketEventData,                 22, "dyn-elem");
    CHECK(156, "HEADER_GC_EVENT_RELOAD",                 TPacketGCEventReload,              1, "fix");
    CHECK(157, "HEADER_GC_EVENT_KW_SCORE",               TPacketGCEventKWScore,             7, "fix");
    CHECK(171, "HEADER_GC_SWITCHBOT",                    TPacketGCSwitchbot,                7, "dyn-hdr");
    CHECK(175, "HEADER_GC_PRIVATE_SHOP",                 TPacketGCPrivateShop,              4, "dyn-hdr");
    CHECK(175, "  (sub) TPacketGCPrivateShopLoad",       TPacketGCPrivateShopLoad,         56, "dyn-sub");
    CHECK(175, "  (sub) TPacketGCPrivateShopAddEntity",  TPacketGCPrivateShopAddEntity,    79, "dyn-sub");
    CHECK(175, "  (sub) TPrivateShopItemData",           TPrivateShopItemData,             71, "dyn-sub");
    CHECK(175, "  (sub) TPacketGCPrivateShopOpen",       TPacketGCPrivateShopOpen,       9129, "dyn-sub");
    CHECK(175, "  (sub) TPrivateShopSaleData",           TPrivateShopSaleData,            100, "dyn-sub");
    CHECK(175, "  (sub) TPrivateShopSearchData",         TPrivateShopSearchData,          101, "dyn-sub");
    CHECK(180, "HEADER_GC_DAILY_GIFT",                   TPacketGCDailyGift,              111, "fix");
    CHECK(208, "HEADER_GC_SPECIFIC_EFFECT",              TPacketGCSpecificEffect,         133, "fix");
    CHECK(209, "HEADER_GC_DRAGON_SOUL_REFINE",           TPacketGCDragonSoulRefine,         5, "fix");
    CHECK(210, "HEADER_GC_RESPOND_CHANNELSTATUS",        TChannelStatus,                    3, "dyn-elem");
    CHECK(211, "HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN",     TPacketGCOpenDragonSoulChangeAttr, 1, "fix");
    CHECK(212, "HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT",   TPacketGCDragonSoulChangeAttrResult, 2, "fix");
    CHECK(214, "HEADER_GC_AURA",                         TPacketGCAura,                     4, "dyn-hdr");
    CHECK(221, "HEADER_GC_CUBE_RENEWAL",                 TPacketGCCubeRenewalReceive,     171, "fix");
    CHECK(223, "HEADER_GC_FISH_EVENT_INFO",              TPacketGCFishEventInfo,           10, "fix");
    CHECK(224, "HEADER_GC_CHARACTER_GOLD",               TPacketGCGold,                     9, "fix");
    CHECK(225, "HEADER_GC_CHARACTER_GOLD_CHANGE",        TPacketGCGoldChange,              24, "fix");
    CHECK(228, "HEADER_GC_REFINE_ELEMENT",               TPacketGCRefineElement,            6, "fix");
    CHECK(231, "HEADER_GC_SASH",                         TPacketSash,                      23, "fix");
    CHECK(234, "HEADER_GC_CL",                           TPacketChangeLook,                10, "fix");
    CHECK(238, "HEADER_GC_BATTLE_PASS",                  TPacketBattlePass,                 9, "dyn-hdr");
    CHECK(238, "  (elem) TPlayerBattlePass",             TPlayerBattlePass,                11, "dyn-elem");
    CHECK(238, "  (sub) TPacketSubBattlePassUpdate",     TPacketSubBattlePassUpdate,       10, "dyn-sub");
    CHECK(245, "HEADER_GC_REQUEST_CHANGE_LANGUAGE",      TPacketChangeLanguage,             2, "fix");
    CHECK(246, "HEADER_GC_WHISPER_DETAILS",              TPacketGCWhisperDetails,          27, "fix");
    CHECK(249, "HEADER_GC_ENTITY",                       TPacketGCEntity,                   3, "dyn-hdr");
    CHECK(249, "  (elem) TPacketEntityInfo",             TPacketEntityInfo,                28, "dyn-elem");
    CHECK1(252, "HEADER_GC_HANDSHAKE_OK (TIME_SYNC)");
    CHECK(253, "HEADER_GC_PHASE",                        TPacketGCPhase,                    2, "fix");
    CHECK(254, "HEADER_GC_BINDUDP",                      TPacketGCBindUDP,                  7, "fix");
    CHECK(255, "HEADER_GC_HANDSHAKE",                    TPacketGCHandshake,               13, "fix");
    return 0;
}
