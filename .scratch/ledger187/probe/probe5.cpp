
typedef unsigned char BYTE;
typedef unsigned short WORD;
typedef unsigned int DWORD;
typedef long LONG;
typedef long time_t;
typedef float float32;
#define CHARACTER_NAME_MAX_LEN 24
#pragma pack(1)
typedef struct ctl_pvp { BYTE bHeader; DWORD dwVIDSrc; DWORD dwVIDDst; BYTE bMode; } CTL_PVP;
typedef struct ctl_pickup { BYTE bHeader; DWORD dwVID; DWORD dwVnum; } CTL_PICKUP;
typedef struct ctl_two_long { long a; long b; } CTL_TWO_LONG;
typedef struct ctl_dword { DWORD a; } CTL_DWORD;

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

typedef struct packet_main_character3_bgm
{
	enum { MUSIC_NAME_LEN = 24, };
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
	enum { MUSIC_NAME_LEN = 24, };
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

typedef struct packet_points
{
	BYTE	header;
	long	points[21];
} TPacketGCPoints;

using TPacketGCEntity = struct SPacketGCEntity
{
	BYTE bHeader;
	WORD wSize;
};
#pragma pack()

#define S(T) char sz_##T[sizeof(T)]
S(CTL_PVP); S(CTL_PICKUP); S(CTL_TWO_LONG); S(CTL_DWORD);
S(TPacketGCMainCharacter); S(TPacketGCMainCharacter3_BGM); S(TPacketGCMainCharacter4_BGM_VOL);
S(TPacketGCPoints); S(TPacketGCEntity);
