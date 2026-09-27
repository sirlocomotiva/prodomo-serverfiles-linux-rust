
typedef unsigned char BYTE;
typedef unsigned short WORD;
typedef unsigned int DWORD;
typedef unsigned int time_t_probe_dummy;
typedef long time_t;   // 4 bytes on i386
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

#pragma pack(1)
typedef struct command_entergame
{
	BYTE	header;
} TPacketCGEnterGame;

typedef struct packet_phase
{
	BYTE	header;
	BYTE	phase;
} TPacketGCPhase;

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
	DWORD	dwAffectFlag[2];
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

typedef struct packet_chat
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

typedef struct SPacketGCTime
{
	BYTE	bHeader;
	time_t	time;
} TPacketGCTime;

typedef struct SPacketGCNPCPosition
{
	BYTE header;
	WORD size;
	WORD count;
} TPacketGCNPCPosition;

struct TNPCPosition
{
	BYTE bType;
	char name[CHARACTER_NAME_MAX_LEN+1];
	long x;
	long y;
};

typedef struct packet_channel
{
	BYTE header;
	BYTE channel;
} TPacketGCChannel;
#pragma pack()

#define SIZEOF(T) static_assert(sizeof(T) == 0, "SIZEOF " #T " = " )

#define SIZEOF(T) enum { probe_##T = sizeof(T) }
SIZEOF(TPacketCGEnterGame);
SIZEOF(TPacketGCPhase);
SIZEOF(TPacketGCCharacterAdd);
SIZEOF(TPacketGCCharacterAdditionalInfo);
SIZEOF(TPacketGCCharacterUpdate);
SIZEOF(TPacketGCChat);
SIZEOF(TPacketGCTime);
SIZEOF(TPacketGCNPCPosition);
SIZEOF(TNPCPosition);
SIZEOF(TPacketGCChannel);
