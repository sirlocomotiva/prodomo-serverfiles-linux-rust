
typedef unsigned char BYTE;
typedef unsigned short WORD;
typedef unsigned int DWORD;
typedef long time_t;
#define CHARACTER_NAME_MAX_LEN 24
#define LOCALE_STRING_RENEWAL
#define __SASH_SYSTEM__
#define __AURA_SYSTEM__
#define __CONQUEROR_LEVEL__
#define ENABLE_REFINE_ELEMENT
#define ENABLE_SHOW_LIDER_AND_GENERAL_GUILD
#define __ENABLE_PREMIUM_PLAYERS__
#define __MULTI_LANGUAGE_SYSTEM__
#define __FIX_UPDATE_LEVEL__
#define LONG long

enum ECharacterEquipmentPart { CHR_EQUIPPART_ARMOR, CHR_EQUIPPART_WEAPON, CHR_EQUIPPART_HEAD, CHR_EQUIPPART_HAIR,
#ifdef __SASH_SYSTEM__
 CHR_EQUIPPART_SASH,
#endif
#ifdef __AURA_SYSTEM__
 CHR_EQUIPPART_AURA,
#endif
 CHR_EQUIPPART_NUM };

#pragma pack(1)
/* ---- controls: already-settled widths from protocol/src/gc_fields.rs ---- */
typedef struct ctl_pvp { BYTE bHeader; DWORD dwVIDSrc; DWORD dwVIDDst; BYTE bMode; } CTL_PVP;   /* 10 */
typedef struct ctl_pickup { BYTE bHeader; DWORD dwVID; DWORD dwVnum; } CTL_PICKUP;              /* 9 */
typedef struct ctl_motion { BYTE bHeader; DWORD dwVID; DWORD dwJob; WORD wFunc; } CTL_MOTION;  /* 11 */
typedef struct ctl_tu { BYTE bHeader; DWORD dwID; DWORD dwTargetID; DWORD dwSkillID; } CTL_TU;  /* 13 */
typedef struct ctl_two_long { long a; long b; } CTL_TWO_LONG;                                  /* 8 */

typedef struct
{
	DWORD	dwType;
	BYTE	bApplyOn;
	long	lApplyValue;
	DWORD	dwFlag;
	long	lDuration;
	long	lSPCost;
} TPacketAffectElement;

typedef struct
{
	BYTE		bHeader;
	TPacketAffectElement elem;
} TPacketGCAffectAdd;

typedef struct packet_pvp
{
	BYTE        bHeader;
	DWORD       dwVIDSrc;
	DWORD       dwVIDDst;
	BYTE        bMode;
} TPacketGCPVP;

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

typedef struct SPacketGCWalkMode
{
	BYTE	header;
	DWORD	vid;
	BYTE	mode;
} TPacketGCWalkMode;

typedef struct SPacketGCShopSign
{
	BYTE	bHeader;
	DWORD	dwVID;
	char	szSign[32+1];
} TPacketGCShopSign;

using TPacketEntityInfo = struct SPacketEntityInfo
{
	DWORD dwVID;
	DWORD dwRaceVNum;
	WORD wPart[CHR_EQUIPPART_NUM];
	LONG xPos, yPos;
};
#pragma pack()

#define S(T) char sz_##T[sizeof(T)]
S(CTL_PVP); S(CTL_PICKUP); S(CTL_MOTION); S(CTL_TU); S(CTL_TWO_LONG);
S(TPacketAffectElement); S(TPacketGCAffectAdd); S(TPacketGCPVP); S(TLandPacketElement);
S(TPacketGCLandList); S(TPacketGCWalkMode); S(TPacketGCShopSign); S(TPacketEntityInfo);
