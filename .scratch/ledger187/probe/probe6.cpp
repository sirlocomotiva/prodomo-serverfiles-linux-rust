
typedef unsigned char BYTE;
typedef unsigned short WORD;
typedef unsigned int DWORD;
typedef long time_t;
#define POINT_MAX_NUM 255
#define SKILL_MAX_NUM 255
#pragma pack(1)
typedef struct ctl_pvp { BYTE bHeader; DWORD dwVIDSrc; DWORD dwVIDDst; BYTE bMode; } CTL_PVP;
typedef struct ctl_two_long { long a; long b; } CTL_TWO_LONG;
typedef struct
{
	BYTE	header;
	long long	points[POINT_MAX_NUM];
} TPacketGCPoints;
typedef struct SPlayerSkill
{
	BYTE	bMasterType;
	BYTE	bLevel;
	time_t	tNextRead;
} TPlayerSkill;
typedef struct packet_skill_level
{
	BYTE		bHeader;
	TPlayerSkill	skills[SKILL_MAX_NUM];
} TPacketGCSkillLevel;
#pragma pack()

#define S(T) char sz_##T[sizeof(T)]
S(CTL_PVP); S(CTL_TWO_LONG); S(TPlayerSkill); S(TPacketGCPoints); S(TPacketGCSkillLevel);
