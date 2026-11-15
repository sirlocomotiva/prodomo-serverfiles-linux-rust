
// Width probe for ledger 187 records. Compile with: g++ -m32 -c -o /dev/null probe.cpp
// Every assert that survives is a measured width. The controls (known widths and the packed
// two-long struct) must also survive, or the probe itself is untrustworthy.
typedef unsigned char  BYTE;
typedef unsigned short WORD;
typedef unsigned int   DWORD;
typedef unsigned long  QWORD;
typedef int            LONG;

#pragma pack(1)

// --- controls: widths already known and pinned elsewhere in this repository ---
struct control_two_long { long a; long b; };            // must be 8
struct control_packet_gc_ping { BYTE header; };         // must be 1
struct control_packet_quickslot_add { BYTE header; BYTE pos; BYTE type; BYTE pos2; }; // must be 4

// --- HEADER_GC_ENTITY: server/server/game/packet.h:3365-3379 ---
struct TPacketGCEntity { BYTE bHeader; WORD wSize; };   // 3
struct TPacketEntityInfo { DWORD dwVID; DWORD dwRaceVNum; WORD wPart[6]; LONG xPos, yPos; };

// --- HEADER_GC_CHAT: server/server/game/packet.h:979-990 (LOCALE_STRING_RENEWAL is defined) ---
struct packet_chat { BYTE header; WORD size; BYTE type; DWORD id; BYTE bEmpire; bool bCanFormat; };

// --- HEADER_GC_GREET path uses the same struct; also check the client shape ---
struct client_packet_chat { BYTE header; WORD size; BYTE type; DWORD id; BYTE bEmpire; bool bCanFormat; };

// --- TPacketGCTime: packet.h (search) ---
struct TPacketGCTime { BYTE bHeader; DWORD time; };

// --- TPacketGCChannel ---
struct TPacketGCChannel { BYTE header; BYTE channel; };

// --- TPacketGCSkillLevel (76): SKILL_MAX_NUM entries of TPlayerSkill ---
struct TPlayerSkill { BYTE bSkill; BYTE bLevel; BYTE bOn; };
#ifndef SKILL_MAX_NUM
#define SKILL_MAX_NUM 255
#endif
struct TPacketGCSkillLevel { BYTE bHeader; TPlayerSkill skills[SKILL_MAX_NUM]; };

// --- TPacketGCGold (224) under ENABLE_REMOVE_LIMIT_GOLD ---
typedef unsigned long long ULONGLONG;
struct TPacketGCGold { BYTE header; ULONGLONG gold; };

static_assert(sizeof(control_two_long) == 8, "packed two long must be 8");
static_assert(sizeof(control_packet_gc_ping) == 1, "ping must be 1");
static_assert(sizeof(control_packet_quickslot_add) == 4, "quickslot add must be 4");

static_assert(sizeof(TPacketGCEntity) == 3, "entity header must be 3");
static_assert(sizeof(TPacketEntityInfo) == 32, "MUTANT: entity info 32");
static_assert(sizeof(packet_chat) == 10, "chat header must be 10");
static_assert(sizeof(client_packet_chat) == 10, "client chat header must be 10");
static_assert(sizeof(TPacketGCTime) == 5, "time must be 5");
static_assert(sizeof(TPacketGCChannel) == 2, "channel must be 2");
static_assert(sizeof(TPacketGCSkillLevel) == 1 + 3 * SKILL_MAX_NUM, "skill level width");
static_assert(sizeof(TPacketGCGold) == 9, "gold must be 9");
