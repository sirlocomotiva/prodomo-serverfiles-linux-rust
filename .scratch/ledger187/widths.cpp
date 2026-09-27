
// Width probe: g++ -m32 (i386 ABI), #pragma pack(1), measured via template arg.
// Switches resolved from common/prodomodefines.h for THIS tree.
typedef unsigned char       BYTE;
typedef unsigned short      WORD;
typedef unsigned int        DWORD;
typedef int                 INT;
typedef long                LONG;
typedef unsigned long int   QWORD;

#define CHARACTER_NAME_MAX_LEN 24
#define POINT_MAX_NUM 255
#define SKILL_MAX_NUM 255
#define QUICKSLOT_MAX_NUM 36
#define ITEM_SOCKET_MAX_NUM 6      // ENABLE_EXTENDED_SOCKETS defined
#define ITEM_ATTRIBUTE_MAX_NUM 7
typedef long time_t;  // i386 glibc __TIME_T_TYPE; 32-bit glibc headers absent for -m32

// ECharacterEquipmentPart: __SASH_SYSTEM__ and __AURA_SYSTEM__ both defined
enum ECharacterEquipmentPart {
  CHR_EQUIPPART_ARMOR, CHR_EQUIPPART_WEAPON, CHR_EQUIPPART_HEAD,
  CHR_EQUIPPART_HAIR, CHR_EQUIPPART_SASH, CHR_EQUIPPART_AURA, CHR_EQUIPPART_NUM,
};

#pragma pack(1)

// ---- controls (must be 8 / 4 / 4 / 1 / 4 on i386) ----
struct TwoLong { unsigned long a; unsigned long b; };
struct OneLong  { unsigned long a; };
struct OnePtr   { void* a; };
struct OneBool  { bool a; };
struct OneTimeT { time_t a; };

// common/tables.h:426 TPlayerItemAttribute (pack 1 region 345..2291)
typedef struct TPlayerItemAttribute { BYTE bType; short sValue; } TPlayerItemAttribute;
// common/tables.h:452 SQuickslot
typedef struct SQuickslot { BYTE type; BYTE pos; } TQuickslot;
// common/tables.h:466 SPlayerSkill  (time_t == 4 on i386)
typedef struct SPlayerSkill { BYTE bMasterType; BYTE bLevel; time_t tNextRead; } TPlayerSkill;
// common/length.h:957 SItemPos (pack(push,1) region 956..1274)
typedef struct SItemPos { BYTE window_type; WORD cell; } SItemPos;

// ---- game/packet.h records (pack 1 region 274..3540) ----
typedef struct command_player_select { BYTE header; BYTE index; } TPacketCGPlayerSelect;
typedef struct packet_phase { BYTE header; BYTE phase; } TPacketGCPhase;
using TPacketGCEntity = struct SPacketGCEntity { BYTE bHeader; WORD wSize; };
using TPacketEntityInfo = struct SPacketEntityInfo {
  DWORD dwVID; DWORD dwRaceVNum; WORD wPart[CHR_EQUIPPART_NUM]; LONG xPos, yPos; };
typedef struct packet_main_character {
  BYTE header; DWORD dwVID; WORD wRaceNum; char szName[CHARACTER_NAME_MAX_LEN + 1];
  long lx, ly, lz; BYTE empire; BYTE skill_group; } TPacketGCMainCharacter;
typedef struct packet_main_character3_bgm {
  enum { MUSIC_NAME_LEN = 24, };
  BYTE header; DWORD dwVID; WORD wRaceNum; char szChrName[CHARACTER_NAME_MAX_LEN + 1];
  char szBGMName[MUSIC_NAME_LEN + 1]; long lx, ly, lz; BYTE empire; BYTE skill_group;
} TPacketGCMainCharacter3_BGM;
typedef struct packet_main_character4_bgm_vol {
  enum { MUSIC_NAME_LEN = 24, };
  BYTE header; DWORD dwVID; WORD wRaceNum; char szChrName[CHARACTER_NAME_MAX_LEN + 1];
  char szBGMName[MUSIC_NAME_LEN + 1]; float fBGMVol; long lx, ly, lz; BYTE empire; BYTE skill_group;
} TPacketGCMainCharacter4_BGM_VOL;
typedef struct packet_points { BYTE header; long long points[POINT_MAX_NUM]; } TPacketGCPoints;
typedef struct packet_skill_level { BYTE bHeader; TPlayerSkill skills[SKILL_MAX_NUM]; } TPacketGCSkillLevel;
struct packet_quickslot_add { BYTE header; BYTE pos; TQuickslot slot; };
struct packet_quickslot_del { BYTE header; BYTE pos; };
struct packet_quickslot_swap { BYTE header; BYTE pos; BYTE pos_to; };
// ENABLE_REFINE_ELEMENT and __CHANGELOOK_SYSTEM__ both defined
typedef struct packet_item_set {
  BYTE header; SItemPos Cell; DWORD vnum; WORD count;
  DWORD dwRefineElement; DWORD transmutation; DWORD flags; DWORD anti_flags;
  bool highlight; long alSockets[ITEM_SOCKET_MAX_NUM];
  TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM]; } TPacketGCItemSet;
typedef struct { DWORD dwID; long x, y; long width, height; DWORD dwGuildID; } TLandPacketElement;
typedef struct packet_land_list { BYTE header; WORD size; } TPacketGCLandList;
typedef struct packet_gold { BYTE header; unsigned long long gold; } TPacketGCGold;
// common/tables.h:928 SPlayerLoadPacket (pack 1 region)
typedef struct SPlayerLoadPacket { DWORD account_id; DWORD player_id; BYTE account_index; } TPlayerLoadPacket;
// packet.h:2710 dynamic map SDB record (packed members, stream appended by hand)
struct TPacketGCPackageSDB { BYTE bHeader; WORD uDynamicPacketSize; int iStreamLen; };

template<int N> struct Size;
Size<sizeof(TwoLong)> z01;  /*LABEL TwoLong*/
Size<sizeof(OneLong)>  z02;  /*LABEL OneLong*/
Size<sizeof(OnePtr)>   z03;  /*LABEL OnePtr*/
Size<sizeof(OneBool)>  z04;  /*LABEL OneBool*/
Size<sizeof(OneTimeT)> z05;  /*LABEL OneTimeT*/
Size<sizeof(TPlayerItemAttribute)> z06;  /*LABEL TPlayerItemAttribute*/
Size<sizeof(TQuickslot)> z07;  /*LABEL TQuickslot*/
Size<sizeof(TPlayerSkill)> z08;  /*LABEL TPlayerSkill*/
Size<sizeof(SItemPos)> z09;  /*LABEL SItemPos*/
Size<sizeof(TPacketCGPlayerSelect)> z10;  /*LABEL TPacketCGPlayerSelect*/
Size<sizeof(TPacketGCPhase)> z11;  /*LABEL TPacketGCPhase*/
Size<sizeof(TPacketGCEntity)> z12;  /*LABEL TPacketGCEntity*/
Size<sizeof(TPacketEntityInfo)> z13;  /*LABEL TPacketEntityInfo*/
Size<sizeof(TPacketGCMainCharacter)> z14;  /*LABEL TPacketGCMainCharacter*/
Size<sizeof(TPacketGCMainCharacter3_BGM)> z15;  /*LABEL TPacketGCMainCharacter3_BGM*/
Size<sizeof(TPacketGCMainCharacter4_BGM_VOL)> z16;  /*LABEL TPacketGCMainCharacter4_BGM_VOL*/
Size<sizeof(TPacketGCPoints)> z17;  /*LABEL TPacketGCPoints*/
Size<sizeof(TPacketGCSkillLevel)> z18;  /*LABEL TPacketGCSkillLevel*/
Size<sizeof(packet_quickslot_add)> z19;  /*LABEL packet_quickslot_add*/
Size<sizeof(packet_quickslot_del)> z20;  /*LABEL packet_quickslot_del*/
Size<sizeof(packet_quickslot_swap)> z21;  /*LABEL packet_quickslot_swap*/
Size<sizeof(TPacketGCItemSet)> z22;  /*LABEL TPacketGCItemSet*/
Size<sizeof(TLandPacketElement)> z23;  /*LABEL TLandPacketElement*/
Size<sizeof(TPacketGCLandList)> z24;  /*LABEL TPacketGCLandList*/
Size<sizeof(TPacketGCGold)> z25;  /*LABEL TPacketGCGold*/
Size<sizeof(TPlayerLoadPacket)> z26;  /*LABEL TPlayerLoadPacket*/
Size<sizeof(TPacketGCPackageSDB)> z27;  /*LABEL TPacketGCPackageSDB*/
