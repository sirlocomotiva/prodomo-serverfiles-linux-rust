#ifndef __INC_METIN_II_CHAR_H__
#define __INC_METIN_II_CHAR_H__

#include <boost/unordered_map.hpp>

#include "../common/stl.h"
#include "LettersConfig.h"
#include "entity.h"
#include "FSM.h"
#include "horse_rider.h"
#include "vid.h"
#include "constants.h"
#include "affect.h"
#include "affect_flag.h"
#ifndef ENABLE_CUBE_RENEWAL_WORLDARD
#include "cube.h"
#else
#include "cuberenewal.h"
#endif
#include "mining.h"
#include "../common/prodomodefines.h"
#include "packet.h"
#ifdef __PREMIUM_PRIVATE_SHOP__
#include "packet.h"
class CPrivateShop;
#endif

#define ENABLE_ANTI_CMD_FLOOD
#define ENABLE_OPEN_SHOP_WITH_ARMOR
enum eMountType {MOUNT_TYPE_NONE=0, MOUNT_TYPE_NORMAL=1, MOUNT_TYPE_COMBAT=2, MOUNT_TYPE_MILITARY=3};
eMountType GetMountLevelByVnum(DWORD dwMountVnum, bool IsNew);
const DWORD GetRandomSkillVnum(BYTE bJob = JOB_MAX_NUM);


class CBuffOnAttributes;
class CPetSystem;
#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
class CMountSystem;
#endif
#ifdef __ENABLE_SHAMAN_SYSTEM__
class CShamanSystem;
class CShamanActor;
#endif

#define INSTANT_FLAG_DEATH_PENALTY		(1 << 0)
#define INSTANT_FLAG_SHOP			(1 << 1)
#define INSTANT_FLAG_EXCHANGE			(1 << 2)
#define INSTANT_FLAG_STUN			(1 << 3)
#define INSTANT_FLAG_NO_REWARD			(1 << 4)

#define AI_FLAG_NPC				(1 << 0)
#define AI_FLAG_AGGRESSIVE			(1 << 1)
#define AI_FLAG_HELPER				(1 << 2)
#define AI_FLAG_STAYZONE			(1 << 3)
#define MAX_CARDS_IN_HAND	5
#define MAX_CARDS_IN_FIELD	3

#define SET_OVER_TIME(ch, time)	(ch)->SetOverTime(time)

extern int g_nPortalLimitTime;

enum
{
	MAIN_RACE_WARRIOR_M,
	MAIN_RACE_ASSASSIN_W,
	MAIN_RACE_SURA_M,
	MAIN_RACE_SHAMAN_W,
	MAIN_RACE_WARRIOR_W,
	MAIN_RACE_ASSASSIN_M,
	MAIN_RACE_SURA_W,
	MAIN_RACE_SHAMAN_M,

	MAIN_RACE_MAX_NUM,
};

enum
{
	POISON_LENGTH = 30,

	STAMINA_PER_STEP = 1,
	SAFEBOX_PAGE_SIZE = 9,
	AI_CHANGE_ATTACK_POISITION_TIME_NEAR = 10000,
	AI_CHANGE_ATTACK_POISITION_TIME_FAR = 1000,
	AI_CHANGE_ATTACK_POISITION_DISTANCE = 100,
	SUMMON_MONSTER_COUNT = 3,
};

enum
{
	FLY_NONE,
	FLY_EXP,
	FLY_HP_MEDIUM,
	FLY_HP_BIG,
	FLY_SP_SMALL,
	FLY_SP_MEDIUM,
	FLY_SP_BIG,
	FLY_FIREWORK1,
	FLY_FIREWORK2,
	FLY_FIREWORK3,
	FLY_FIREWORK4,
	FLY_FIREWORK5,
	FLY_FIREWORK6,
	FLY_FIREWORK_CHRISTMAS,
	FLY_CHAIN_LIGHTNING,
	FLY_HP_SMALL,
	FLY_SKILL_MUYEONG,
#if defined(__CONQUEROR_LEVEL__)
	FLY_CONQUEROR_EXP,
#endif
};

enum EDamageType
{
	DAMAGE_TYPE_NONE,
	DAMAGE_TYPE_NORMAL,
	DAMAGE_TYPE_NORMAL_RANGE,
	//��ų
	DAMAGE_TYPE_MELEE,
	DAMAGE_TYPE_RANGE,
	DAMAGE_TYPE_FIRE,
	DAMAGE_TYPE_ICE,
	DAMAGE_TYPE_ELEC,
	DAMAGE_TYPE_MAGIC,
	DAMAGE_TYPE_POISON,
	DAMAGE_TYPE_SPECIAL,

};

enum DamageFlag
{
	DAMAGE_NORMAL	= (1 << 0),
	DAMAGE_POISON	= (1 << 1),
	DAMAGE_DODGE	= (1 << 2),
	DAMAGE_BLOCK	= (1 << 3),
	DAMAGE_PENETRATE= (1 << 4),
	DAMAGE_CRITICAL = (1 << 5),

};

enum EPointTypes
{
	POINT_NONE,              
	POINT_LEVEL,             
	POINT_VOICE,             
	POINT_EXP,               
	POINT_NEXT_EXP,          
	POINT_HP,                
	POINT_MAX_HP,            
	POINT_SP,                
	POINT_MAX_SP,            
	POINT_STAMINA,           
	POINT_MAX_STAMINA,       
	POINT_GOLD,              
	POINT_ST,                
	POINT_HT,                
	POINT_DX,                
	POINT_IQ,                
	POINT_DEF_GRADE,		
	POINT_ATT_SPEED,         
	POINT_ATT_GRADE,		
	POINT_MOV_SPEED,         
	POINT_CLIENT_DEF_GRADE,	
	POINT_CASTING_SPEED,     
	POINT_MAGIC_ATT_GRADE,   
	POINT_MAGIC_DEF_GRADE,   
	POINT_EMPIRE_POINT,      
	POINT_LEVEL_STEP,        
	POINT_STAT,              
	POINT_SUB_SKILL,		
	POINT_SKILL,		
	POINT_WEAPON_MIN,		
	POINT_WEAPON_MAX,		
	POINT_PLAYTIME,             
	POINT_HP_REGEN,             
	POINT_SP_REGEN,             
	POINT_BOW_DISTANCE,         
	POINT_HP_RECOVERY,          
	POINT_SP_RECOVERY,          
	POINT_POISON_PCT,           
	POINT_STUN_PCT,             
	POINT_SLOW_PCT,             
	POINT_CRITICAL_PCT,         
	POINT_PENETRATE_PCT,        
	POINT_CURSE_PCT,            
	POINT_ATTBONUS_HUMAN,       
	POINT_ATTBONUS_ANIMAL,      
	POINT_ATTBONUS_ORC,         
	POINT_ATTBONUS_MILGYO,      
	POINT_ATTBONUS_UNDEAD,      
	POINT_ATTBONUS_DEVIL,       
	POINT_ATTBONUS_INSECT,      
	POINT_ATTBONUS_FIRE,        
	POINT_ATTBONUS_ICE,         
	POINT_ATTBONUS_DESERT,      
	POINT_ATTBONUS_MONSTER,     
	POINT_ATTBONUS_WARRIOR,     
	POINT_ATTBONUS_ASSASSIN,	
	POINT_ATTBONUS_SURA,		
	POINT_ATTBONUS_SHAMAN,		
	POINT_ATTBONUS_TREE,     	
	POINT_RESIST_WARRIOR,		
	POINT_RESIST_ASSASSIN,		
	POINT_RESIST_SURA,			
	POINT_RESIST_SHAMAN,		
	POINT_STEAL_HP,             
	POINT_STEAL_SP,             
	POINT_MANA_BURN_PCT,        
	POINT_DAMAGE_SP_RECOVER,    
	POINT_BLOCK,                
	POINT_DODGE,                
	POINT_RESIST_SWORD,         
	POINT_RESIST_TWOHAND,       
	POINT_RESIST_DAGGER,        
	POINT_RESIST_BELL,          
	POINT_RESIST_FAN,           
	POINT_RESIST_BOW,           
	POINT_RESIST_FIRE,          
	POINT_RESIST_ELEC,          
	POINT_RESIST_MAGIC,         
	POINT_RESIST_WIND,          
	POINT_REFLECT_MELEE,        
	POINT_REFLECT_CURSE,		
	POINT_POISON_REDUCE,		
	POINT_KILL_SP_RECOVER,		
	POINT_EXP_DOUBLE_BONUS,		
	POINT_GOLD_DOUBLE_BONUS,	
	POINT_ITEM_DROP_BONUS,		
	POINT_POTION_BONUS,			
	POINT_KILL_HP_RECOVERY,		
	POINT_IMMUNE_STUN,			
	POINT_IMMUNE_SLOW,			
	POINT_IMMUNE_FALL,			
	POINT_PARTY_ATTACKER_BONUS,	
	POINT_PARTY_TANKER_BONUS,	
	POINT_ATT_BONUS,			
	POINT_DEF_BONUS,			
	POINT_ATT_GRADE_BONUS,		
	POINT_DEF_GRADE_BONUS,
	POINT_MAGIC_ATT_GRADE_BONUS,
	POINT_MAGIC_DEF_GRADE_BONUS,
	POINT_RESIST_NORMAL_DAMAGE,
	POINT_HIT_HP_RECOVERY,
	POINT_HIT_SP_RECOVERY,
	POINT_MANASHIELD,
	POINT_PARTY_BUFFER_BONUS,
	POINT_PARTY_SKILL_MASTER_BONUS,
	POINT_HP_RECOVER_CONTINUE,
	POINT_SP_RECOVER_CONTINUE,
	POINT_STEAL_GOLD,
	POINT_POLYMORPH,
	POINT_MOUNT,
	POINT_PARTY_HASTE_BONUS,
	POINT_PARTY_DEFENDER_BONUS,
	POINT_STAT_RESET_COUNT,
	POINT_HORSE_SKILL,	
	POINT_MALL_ATTBONUS,
	POINT_MALL_DEFBONUS,
	POINT_MALL_EXPBONUS,
	POINT_MALL_ITEMBONUS,
	POINT_MALL_GOLDBONUS,
	POINT_MAX_HP_PCT,	
	POINT_MAX_SP_PCT,	
	POINT_SKILL_DAMAGE_BONUS,	
	POINT_NORMAL_HIT_DAMAGE_BONUS,
	POINT_SKILL_DEFEND_BONUS,	
	POINT_NORMAL_HIT_DEFEND_BONUS,
	POINT_RAMADAN_CANDY_BONUS_EXP,		
	POINT_ENERGY 										= 128,					
	POINT_ENERGY_END_TIME 								= 129,		
	POINT_COSTUME_ATTR_BONUS 							= 130,
	POINT_MAGIC_ATT_BONUS_PER 							= 131,
	POINT_MELEE_MAGIC_ATT_BONUS_PER 					= 132,
	POINT_RESIST_ICE 									= 133,
	POINT_RESIST_EARTH 									= 134,
	POINT_RESIST_DARK 									= 135,
	POINT_RESIST_CRITICAL 								= 136,
	POINT_RESIST_PENETRATE								= 137,
#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	POINT_INVEN 										= 145,
#endif
	POINT_ATTBONUS_METIN,
	POINT_ATTBONUS_BOSS,
	POINT_ENCHANT_ELECT,
	POINT_ENCHANT_FIRE,
	POINT_ENCHANT_ICE,
	POINT_ENCHANT_WIND,
	POINT_ENCHANT_EARTH,
	POINT_ENCHANT_DARK,
#ifdef __ENABLE_BIOLOGIST_RENEWAL_SYSTEM__
	POINT_BIOLOGIST_STATE 								= 166,
	POINT_BIOLOGIST_ITEMS_TAKEN 						= 167,
	POINT_BIOLOGIST_COMPLETED 							= 168,
#endif
#if defined(__CONQUEROR_LEVEL__)
	POINT_SUNGMA_STR,
	POINT_SUNGMA_HP,
	POINT_SUNGMA_MOVE,
	POINT_SUNGMA_IMMUNE,

	POINT_CONQUEROR_LEVEL,
	POINT_CONQUEROR_LEVEL_STEP,
	POINT_CONQUEROR_EXP,
	POINT_CONQUEROR_NEXT_EXP,
	POINT_CONQUEROR_POINT,
#endif
#ifdef BONUS_PCT
	POINT_ATTBONUS_ANIMAL_PCT,
	POINT_ATTBONUS_UNDEAD_PCT,
	POINT_ATTBONUS_DEVIL_PCT,
	POINT_ATTBONUS_ORC_PCT,
	POINT_ATTBONUS_MILGYO_PCT,
	POINT_ATTBONUS_DESERT_PCT,
	POINT_ATTBONUS_INSECT_PCT,
	POINT_ATTBONUS_TREE_PCT,
	POINT_ATTBONUS_BOSS_PCT,
	POINT_ATTBONUS_METIN_PCT,
	POINT_ATTBONUS_CZ_PCT,
	POINT_ATTBONUS_HUMAN_PCT,
	POINT_ATTBONUS_MONSTER_PCT,
	POINT_ENCHANT_ELECT_PCT,
	POINT_ENCHANT_FIRE_PCT,
	POINT_ENCHANT_ICE_PCT,
	POINT_ENCHANT_WIND_PCT,
	POINT_ENCHANT_EARTH_PCT,
	POINT_ENCHANT_DARK_PCT,
	POINT_RESIST_ELECT_PCT,
	POINT_RESIST_FIRE_PCT,
	POINT_RESIST_ICE_PCT,
	POINT_RESIST_WIND_PCT,
	POINT_RESIST_EARTH_PCT,
	POINT_RESIST_DARK_PCT,
	POINT_RESIST_HUMAN_PCT,
	POINT_RESIST_FALL,
	POINT_RESIST_COMBAT,
#endif
#ifdef ENABLE_GAYA_SYSTEM
	POINT_GAYA 											= 207,
#endif
#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	POINT_SECURED_STATE 								= 208,
	POINT_SECURED_PASSWORD 								= 209,
#endif
#ifdef __PREMIUM_PRIVATE_SHOP__
	POINT_PRIVATE_SHOP_UNLOCKED_SLOT					= 210,
#endif
};

enum EPKModes
{
	PK_MODE_PEACE,
	PK_MODE_REVENGE,
	PK_MODE_FREE,
	PK_MODE_PROTECT,
	PK_MODE_GUILD,
	PK_MODE_MAX_NUM
};

enum EPositions
{
	POS_DEAD,
	POS_SLEEPING,
	POS_RESTING,
	POS_SITTING,
	POS_FISHING,
	POS_FIGHTING,
	POS_MOUNTING,
	POS_STANDING
};

enum EBlockAction
{
	BLOCK_EXCHANGE		= (1 << 0),
	BLOCK_PARTY_INVITE		= (1 << 1),
	BLOCK_GUILD_INVITE		= (1 << 2),
	BLOCK_WHISPER		= (1 << 3),
	BLOCK_MESSENGER_INVITE	= (1 << 4),
	BLOCK_PARTY_REQUEST		= (1 << 5),
	BLOCK_VIEW_EQUIPMENT	= (1 << 6),
};
struct DynamicCharacterPtr {
	DynamicCharacterPtr() : is_pc(false), id(0) {}
	DynamicCharacterPtr(const DynamicCharacterPtr& o)
		: is_pc(o.is_pc), id(o.id) {}

	LPCHARACTER Get() const;

	void Reset() {
		is_pc = false;
		id = 0;
	}

	DynamicCharacterPtr& operator=(const DynamicCharacterPtr& rhs) {
		is_pc = rhs.is_pc;
		id = rhs.id;
		return *this;
	}

	DynamicCharacterPtr& operator=(LPCHARACTER character);

	operator LPCHARACTER() const {
		return Get();
	}

	bool is_pc;
	uint32_t id;
};
typedef struct character_point
{
	long			points[POINT_MAX_NUM];
	BYTE			job;
	BYTE			voice;
	BYTE			level;
	DWORD			exp;
#if defined(__CONQUEROR_LEVEL__)
	BYTE 			conqueror_level;
	DWORD 			conqueror_exp;
#endif
#ifdef ENABLE_REMOVE_LIMIT_GOLD
	unsigned long long			gold;
#else
	long			gold;
#endif
#ifdef ENABLE_GAYA_SYSTEM
	int 			gaya;
#endif
#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	int 			envanter;
#endif
	int				hp;
	int				sp;
	int				iRandomHP;
	int				iRandomSP;
	int				stamina;
	BYTE			skill_group;
#ifdef __ENABLE_BIOLOGIST_RENEWAL_SYSTEM__
	DWORD 			biologist_state;
	DWORD 			biologist_items_taken;
	DWORD 			biologist_completed;
#endif
#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	BYTE 			secured;
	int 			secured_password;
#endif

} CHARACTER_POINT;
typedef struct character_point_instant
{
	long			points[POINT_MAX_NUM];
	float			fRot;
	int				iMaxHP;
	int				iMaxSP;
	long			position;
	long			instant_flag;
	DWORD			dwAIFlag;
	DWORD			dwImmuneFlag;
	DWORD			dwLastShoutPulse;
	WORD			parts[PART_MAX_NUM];
	LPITEM			pItems[INVENTORY_AND_EQUIP_SLOT_MAX];
	WORD			bItemGrid[INVENTORY_AND_EQUIP_SLOT_MAX];
	LPITEM			pDSItems[DRAGON_SOUL_INVENTORY_MAX_NUM];
	WORD			wDSItemGrid[DRAGON_SOUL_INVENTORY_MAX_NUM];
	LPITEM			pCubeItems[CUBE_MAX_NUM];
	LPCHARACTER		pCubeNpc;
	LPITEM	pSashMaterials[SASH_WINDOW_MAX_MATERIALS];
	LPENTITY		m_pAuraRefineWindowOpener;
	LPITEM	pClMaterials[CL_WINDOW_MAX_MATERIALS];
	LPCHARACTER			battle_victim;
	BYTE			gm_level;
	BYTE			bBasePart;
	int				iMaxStamina;
	BYTE			bBlockMode;
	int				iDragonSoulActiveDeck;
	LPENTITY		m_pDragonSoulRefineWindowOpener;
#if defined(__ATTR_6TH_7TH__)
	LPITEM pAttr67AddItem;
#endif
#ifdef ENABLE_SWITCHBOT
	LPITEM			pSwitchbotItems[SWITCHBOT_SLOT_COUNT];
#endif
    bool            computed;
} CHARACTER_POINT_INSTANT;

#define TRIGGERPARAM		LPCHARACTER ch, LPCHARACTER causer

typedef struct trigger
{
	BYTE	type;
	int		(*func) (TRIGGERPARAM);
	long	value;
} TRIGGER;

class CTrigger
{
	public:
		CTrigger() : bType(0), pFunc(NULL)
		{
		}

		BYTE	bType;
		int	(*pFunc) (TRIGGERPARAM);
};

EVENTINFO(char_event_info)
{
	DynamicCharacterPtr ch;
};

typedef std::map<VID, size_t> target_map;
struct TSkillUseInfo
{
	int	    iHitCount;
	int	    iMaxHitCount;
	int	    iSplashCount;
	DWORD   dwNextSkillUsableTime;
	int	    iRange;
	bool    bUsed;
	DWORD   dwVID;
	bool    isGrandMaster;

	target_map TargetVIDMap;

	TSkillUseInfo()
		: iHitCount(0), iMaxHitCount(0), iSplashCount(0), dwNextSkillUsableTime(0), iRange(0), bUsed(false),
		dwVID(0), isGrandMaster(false)
   	{}

	bool    HitOnce(DWORD dwVnum = 0);

	bool    UseSkill(bool isGrandMaster, DWORD vid, DWORD dwCooltime, int splashcount = 1, int hitcount = -1, int range = -1);
	DWORD   GetMainTargetVID() const	{ return dwVID; }
	void    SetMainTargetVID(DWORD vid) { dwVID=vid; }
	void    ResetHitCount() { if (iSplashCount) { iHitCount = iMaxHitCount; iSplashCount--; } }
};

typedef struct packet_party_update TPacketGCPartyUpdate;
class CExchange;
class CSkillProto;
class CParty;
class CDungeon;
class CWarMap;
class CAffect;
class CGuild;
class CSafebox;
class CArena;
class CShop;
typedef class CShop * LPSHOP;

class CMob;
class CMobInstance;
typedef struct SMobSkillInfo TMobSkillInfo;

//SKILL_POWER_BY_LEVEL
extern int GetSkillPowerByLevelFromType(int job, int skillgroup, int skilllevel);
//END_SKILL_POWER_BY_LEVEL

namespace marriage
{
	class WeddingMap;
}
enum e_overtime
{
	OT_NONE,
	OT_3HOUR,
	OT_5HOUR,
};

#ifdef OFFLINE_SHOP
#include "../../libgame/include/grid.h"
typedef struct SPrivShop
{
	DWORD	shop_id;
	DWORD	shop_vid;
	char	szSign[SHOP_SIGN_MAX_LEN + 1];
	BYTE	item_count;
	BYTE	rest_count;
	BYTE	days;
	DWORD	date_close;
#ifdef ENABLE_REMOVE_LIMIT_GOLD
    unsigned long long gold;
#else
	int 	gold;
#endif
} TPrivShop;

typedef std::map<DWORD, TPrivShop> PSHOP_MAP;
#endif
#ifdef GIFT_SYSTEM
typedef struct SGiftItem
{
	DWORD	id;
	WORD	pos;
#ifdef ENABLE_REMOVE_LIMIT_GOLD
	unsigned long long 	count;
#else
	DWORD 	count;
#endif
	DWORD	vnum;
#ifdef __CHANGELOOK_SYSTEM__
	int 	look;
#endif
	long	alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute    aAttr[ITEM_ATTRIBUTE_MAX_NUM];
	char szFrom[101];
	char szReason[101];
	DWORD dwDateAdd;
} TGiftItem;
typedef std::map<int, std::vector<TGiftItem> > GIFT_MAP;
#endif

#define NEW_ICEDAMAGE_SYSTEM
class CHARACTER : public CEntity, public CFSM, public CHorseRider
{
	protected:
		virtual void	EncodeInsertPacket(LPENTITY entity);
		virtual void	EncodeRemovePacket(LPENTITY entity);

#ifdef RENEWAL_PICKUP_AFFECT
public:
	bool		CanPickupDirectly();
	void		AutoGiveItemNew(LPITEM item, bool printMsg = true);
#endif
public:
		LPCHARACTER			FindCharacterInView(const char * name, bool bFindPCOnly);
		void				UpdatePacket();
	protected:
		CStateTemplate<CHARACTER>	m_stateMove;
		CStateTemplate<CHARACTER>	m_stateBattle;
		CStateTemplate<CHARACTER>	m_stateIdle;

	public:
		virtual void		StateMove();
		virtual void		StateBattle();
		virtual void		StateIdle();
		virtual void		StateFlag();
		virtual void		StateFlagBase();
		void				StateHorse();
	protected:
		void				__StateIdle_Monster();
		void				__StateIdle_Stone();
		void				__StateIdle_NPC();
	public:
		DWORD GetAIFlag() const	{ return m_pointsInstant.dwAIFlag; }

		void				SetAggressive();
		bool				IsAggressive() const;

		void				SetCoward();
		bool				IsCoward() const;
		void				CowardEscape();

		void				SetNoAttackShinsu();
		bool				IsNoAttackShinsu() const;

		void				SetNoAttackChunjo();
		bool				IsNoAttackChunjo() const;

		void				SetNoAttackJinno();
		bool				IsNoAttackJinno() const;

		void				SetAttackMob();
		bool				IsAttackMob() const;

		virtual void			BeginStateEmpty();
		virtual void			EndStateEmpty() {}

		void				RestartAtSamePos();

	protected:
		DWORD				m_dwStateDuration;
		//////////////////////////////////////////////////////////////////////////////////

	public:
		CHARACTER();
		virtual ~CHARACTER();

		void			Create(const char * c_pszName, DWORD vid, bool isPC);
		void			Destroy();

		void			Disconnect(const char * c_pszReason);

	protected:
		void			Initialize();

#ifdef ENABLE_MULTI_FARM_BLOCK
public:
	bool GetMultiStatus() { return m_bmultiFarmStatus; }
	void SetMultiStatus(bool bValue) { m_bmultiFarmStatus = bValue; }

	void SetProtectTime(const std::string& flagname, int value);
	int GetProtectTime(const std::string& flagname) const;

protected:
	bool m_bmultiFarmStatus;
	std::map<std::string, int>  m_protection_Time;
#endif

#ifdef __SEND_TARGET_INFO__
	private:
		DWORD			dwLastTargetInfoPulse;

	public:
		DWORD			GetLastTargetInfoPulse() const	{ return dwLastTargetInfoPulse; }
		void			SetLastTargetInfoPulse(DWORD pulse) { dwLastTargetInfoPulse = pulse; }
#endif
#ifdef ENABLE_DRAGONSOUL_ALCHEMY_PLUS
		void			DragonSoulOpenChangeAttrWindow();
		void			DragonSoulChangeAttrResult(bool result);
		bool			DragonSoul_IsActiveFullDeck() const;
#endif
	public:
		DWORD			GetPlayerID() const	{ return m_dwPlayerID; }

		void			SetPlayerProto(const TPlayerTable * table);
		void			CreatePlayerProto(TPlayerTable & tab);	// ���� �� ���

		void			SetProto(const CMob * c_pkMob);
		WORD			GetRaceNum() const;

		void			Save();		// DelayedSave
		void			SaveReal();	// ���� ����
		void			FlushDelayedSaveItem();
		
#ifdef ENABLE_AFFECT_RENEWAL
		bool 			UseExtendedBlendAffect(LPITEM item, int affect_type, int apply_type, int apply_value, int apply_duration);
		bool 			SetBlendAffect(LPITEM item);
#endif

#ifdef ENABLE_SPECIAL_DROP_CHAT_RENEWAL
		void			SendPickupItemPacket(int item_vnum, int item_count);
#endif

		const char *	GetName() const;
		const VID &		GetVID() const		{ return m_vid;		}

		void			SetName(const std::string& name) { m_stName = name; }

		void			SetRace(BYTE race);
		bool			ChangeSex();

		DWORD			GetAID() const;
		int				GetChangeEmpireCount() const;
		void			SetChangeEmpireCount();
		int				ChangeEmpire(BYTE empire);

		BYTE			GetJob() const;
		BYTE			GetCharType() const;

		bool			IsPC() const		{ return GetDesc() ? true : false; }
		bool			IsNPC()	const		{ return m_bCharType != CHAR_TYPE_PC; }
		bool			IsMonster()	const	{ return m_bCharType == CHAR_TYPE_MONSTER; }
		bool			IsStone() const		{ return m_bCharType == CHAR_TYPE_STONE; }
		bool			IsDoor() const		{ return m_bCharType == CHAR_TYPE_DOOR; }
		bool			IsBuilding() const	{ return m_bCharType == CHAR_TYPE_BUILDING;  }
		bool			IsWarp() const		{ return m_bCharType == CHAR_TYPE_WARP; }
		bool			IsGoto() const		{ return m_bCharType == CHAR_TYPE_GOTO; }
		DWORD			GetLastShoutPulse() const	{ return m_pointsInstant.dwLastShoutPulse; }
		void			SetLastShoutPulse(DWORD pulse) { m_pointsInstant.dwLastShoutPulse = pulse; }
		int				GetLevel() const		{ return m_points.level;	}
		void			SetLevel(BYTE level);

#if defined(__CONQUEROR_LEVEL__)
		void			SetConqueror(bool bSet = true);
		int				GetConquerorLevel() const { return m_points.conqueror_level; }
		void			SetConquerorLevel(BYTE level) { m_points.conqueror_level = level; }
	
		DWORD			GetConquerorExp() const { return m_points.conqueror_exp; }
		void			SetConquerorExp(DWORD exp) { m_points.conqueror_exp = exp; }
		DWORD			GetConquerorNextExp() const;
#endif

		BYTE				GetGMLevel() const;
		BOOL 				IsGM() const;
		void				SetGMLevel();

		DWORD				GetExp() const		{ return m_points.exp;	}
		void				SetExp(DWORD exp)	{ m_points.exp = exp;	}
		DWORD				GetNextExp() const;
		LPCHARACTER			DistributeExp();	// ���� ���� ���� ����� �����Ѵ�.
		void				DistributeHP(LPCHARACTER pkKiller);
		void				DistributeSP(LPCHARACTER pkKiller, int iMethod=0);

		void				SetPosition(int pos);
		bool				IsPosition(int pos) const	{ return m_pointsInstant.position == pos ? true : false; }
		int					GetPosition() const		{ return m_pointsInstant.position; }

		void				SetPart(BYTE bPartPos, WORD wVal);
		WORD				GetPart(BYTE bPartPos) const;
		WORD				GetOriginalPart(BYTE bPartPos) const;

		void				SetHP(int hp)		{ m_points.hp = hp; }
		int					GetHP() const		{ return m_points.hp; }

		void				SetSP(int sp)		{ m_points.sp = sp; }
		int					GetSP() const		{ return m_points.sp; }

		void				SetStamina(int stamina)	{ m_points.stamina = stamina; }
		int					GetStamina() const		{ return m_points.stamina; }

		void				SetMaxHP(int iVal)	{ m_pointsInstant.iMaxHP = iVal; }
#if defined(__CONQUEROR_LEVEL__)
		int					GetMaxHP() const;
#else
		int					GetMaxHP() const { return m_pointsInstant.iMaxHP; }
#endif

		void				SetMaxSP(int iVal)	{ m_pointsInstant.iMaxSP = iVal; }
		int					GetMaxSP() const	{ return m_pointsInstant.iMaxSP; }

		void				SetMaxStamina(int iVal)	{ m_pointsInstant.iMaxStamina = iVal; }
		int					GetMaxStamina() const	{ return m_pointsInstant.iMaxStamina; }

		void				SetRandomHP(int v)	{ m_points.iRandomHP = v; }
		void				SetRandomSP(int v)	{ m_points.iRandomSP = v; }

		int					GetRandomHP() const	{ return m_points.iRandomHP; }
		int					GetRandomSP() const	{ return m_points.iRandomSP; }

		int					GetHPPct() const;

		void				SetRealPoint(BYTE idx, int val);
		int					GetRealPoint(BYTE idx) const;

		void				SetPoint(BYTE idx, int val);
		int		       		GetPoint(BYTE idx) const;
#ifdef BONUS_PCT
		float				GetPointPct(BYTE apply, int index) const;
		DWORD				GetRaceFlag() const;
#endif
		int					GetLimitPoint(BYTE idx) const;
		int					GetPolymorphPoint(BYTE idx) const;

		const TMobTable &	GetMobTable() const;
		BYTE				GetMobRank() const;
		BYTE				GetMobBattleType() const;
		BYTE				GetMobSize() const;
		DWORD				GetMobDamageMin() const;
		DWORD				GetMobDamageMax() const;
		WORD				GetMobAttackRange() const;
		DWORD				GetMobDropItemVnum() const;
		float				GetMobDamageMultiply() const;

		// NEWAI
		bool			IsBerserker() const;
		bool			IsBerserk() const;
		void			SetBerserk(bool mode);

		bool			IsStoneSkinner() const;

		bool			IsGodSpeeder() const;
		bool			IsGodSpeed() const;
		void			SetGodSpeed(bool mode);

		bool			IsDeathBlower() const;
		bool			IsDeathBlow() const;

		bool			IsReviver() const;
		bool			HasReviverInParty() const;
		bool			IsRevive() const;
		void			SetRevive(bool mode);
		// NEWAI END

		bool			IsRaceFlag(DWORD dwBit) const;
		bool			IsSummonMonster() const;
		DWORD			GetSummonVnum() const;

		DWORD			GetPolymorphItemVnum() const;
		DWORD			GetMonsterDrainSPPoint() const;

		void			MainCharacterPacket();	// ���� ����ĳ���Ͷ�� �����ش�.

		void			ComputePoints();
		void			ComputeBattlePoints();
		void			PointChange(BYTE type, int amount, bool bAmount = false, bool bBroadcast = false);
		void			PointsPacket();
		void			UpdatePointsPacket(BYTE type, long long val, long long amount = 0, bool bAmount = false, bool bBroadcast = false);
		void			ApplyPoint(BYTE bApplyType, int iVal);
		void			CheckMaximumPoints();	// HP, SP ���� ���� ���� �ִ밪 ���� ������ �˻��ϰ� ���ٸ� �����.

		bool			Show(long lMapIndex, long x, long y, long z = LONG_MAX, bool bShowSpawnMotion = false);

		void			Sitdown(int is_ground);
		void			Standup();

		void			SetRotation(float fRot);
		void			SetRotationToXY(long x, long y);
		float			GetRotation() const	{ return m_pointsInstant.fRot; }

		void			MotionPacketEncode(BYTE motion, LPCHARACTER victim, struct packet_motion * packet);
		void			Motion(BYTE motion, LPCHARACTER victim = NULL);

		void			ChatPacket(BYTE type, const char *format, ...);
		void			MonsterChat(BYTE bMonsterChatType);
		void			SendGreetMessage();

		void			ResetPoint(int iLv);

		void			SetBlockMode(BYTE bFlag);
		void			SetBlockModeForce(BYTE bFlag);
		bool			IsBlockMode(BYTE bFlag) const	{ return (m_pointsInstant.bBlockMode & bFlag)?true:false; }

		bool			IsPolymorphed() const		{ return m_dwPolymorphRace>0; }
		bool			IsPolyMaintainStat() const	{ return m_bPolyMaintainStat; } // ���� ������ �����ϴ� ��������.
		void			SetPolymorph(DWORD dwRaceNum, bool bMaintainStat = false);
		DWORD			GetPolymorphVnum() const	{ return m_dwPolymorphRace; }
		int				GetPolymorphPower() const;
		// FISING
		void			fishing();
		void			fishing_take();
		// END_OF_FISHING

		// MINING
		void			mining(LPCHARACTER chLoad);
		void			mining_cancel();
		void			mining_take();
		// END_OF_MINING

		void			ResetPlayTime(DWORD dwTimeRemain = 0);

		void			CreateFly(BYTE bType, LPCHARACTER pkVictim);

		void			ResetChatCounter();
		BYTE			IncreaseChatCounter();
		BYTE			GetChatCounter() const;

		void			ResetMountCounter();
		BYTE			IncreaseMountCounter();
		BYTE			GetMountCounter() const;

	protected:
		DWORD			m_dwPolymorphRace;
		bool			m_bPolyMaintainStat;
		DWORD			m_dwLoginPlayTime;
		DWORD			m_dwPlayerID;
		VID				m_vid;
		std::string		m_stName;
		BYTE			m_bCharType;


		CHARACTER_POINT		m_points;
		CHARACTER_POINT_INSTANT	m_pointsInstant;

		int				m_iMoveCount;
		DWORD			m_dwPlayStartTime;
		BYTE			m_bAddChrState;
		bool			m_bSkipSave;
		BYTE			m_bChatCounter;
		BYTE			m_bMountCounter;
	public:
		bool			IsStateMove() const			{ return IsState((CState&)m_stateMove); }
		bool			IsStateIdle() const			{ return IsState((CState&)m_stateIdle); }
		bool			IsWalking() const			{ return m_bNowWalking || GetStamina()<=0; }
		void			SetWalking(bool bWalkFlag)	{ m_bWalking=bWalkFlag; }
		void			SetNowWalking(bool bWalkFlag);
		void			ResetWalking()			{ SetNowWalking(m_bWalking); }

		bool			Goto(long x, long y);
		void			Stop();

		bool			CanMove() const;

		void			SyncPacket();
		bool			Sync(long x, long y);
		bool			Move(long x, long y);
		void			OnMove(bool bIsAttack = false);
		DWORD			GetMotionMode() const;
		float			GetMoveMotionSpeed() const;
		float			GetMoveSpeed() const;
		void			CalculateMoveDuration();
		void			SendMovePacket(BYTE bFunc, BYTE bArg, DWORD x, DWORD y, DWORD dwDuration, DWORD dwTime=0, int iRot=-1);
		DWORD			GetCurrentMoveDuration() const	{ return m_dwMoveDuration; }
		DWORD			GetWalkStartTime() const	{ return m_dwWalkStartTime; }
		DWORD			GetLastMoveTime() const		{ return m_dwLastMoveTime; }
		DWORD			GetLastAttackTime() const	{ return m_dwLastAttackTime; }

		void			SetLastAttacked(DWORD time);

		bool			SetSyncOwner(LPCHARACTER ch, bool bRemoveFromList = true);
		bool			IsSyncOwner(LPCHARACTER ch) const;

		bool			WarpSet(long x, long y, long lRealMapIndex = 0);
		void			SetWarpLocation(long lMapIndex, long x, long y);
		void			WarpEnd();
		const PIXEL_POSITION & GetWarpPosition() const { return m_posWarp; }
		bool			WarpToPID(DWORD dwPID);

		void			SaveExitLocation();
		void			ExitToSavedLocation();

		void			StartStaminaConsume();
		void			StopStaminaConsume();
		bool			IsStaminaConsume() const;
		bool			IsStaminaHalfConsume() const;

		void			ResetStopTime();
		DWORD			GetStopTime() const;
#ifdef __MULTI_LANGUAGE_SYSTEM__
	bool ChangeLanguage(BYTE bLanguage);
#endif
#ifdef ENABLE_MOVE_CHANNEL
		bool			MoveChannel(long lNewAddr, WORD wNewPort);
		bool			StartMoveChannel(long lNewAddr, WORD wNewPort);
#endif

	protected:
		void			ClearSync();

#ifdef ENABLE_FLY_FIX
		DWORD			m_fSyncTime;
#else
		float			m_fSyncTime;
#endif
		LPCHARACTER		m_pkChrSyncOwner;
		CHARACTER_LIST	m_kLst_pkChrSyncOwned;

		PIXEL_POSITION	m_posDest;
		PIXEL_POSITION	m_posStart;
		PIXEL_POSITION	m_posWarp;
		long			m_lWarpMapIndex;

		PIXEL_POSITION	m_posExit;
		long			m_lExitMapIndex;

		DWORD			m_dwMoveStartTime;
		DWORD			m_dwMoveDuration;

		DWORD			m_dwLastMoveTime;
		DWORD			m_dwLastAttackTime;
		DWORD			m_dwWalkStartTime;
		DWORD			m_dwStopTime;

		bool			m_bWalking;
		bool			m_bNowWalking;
		bool			m_bStaminaConsume;
	public:
		void			SyncQuickslot(BYTE bType, BYTE bOldPos, BYTE bNewPos);
		bool			GetQuickslot(BYTE pos, TQuickslot ** ppSlot);
		bool			SetQuickslot(BYTE pos, TQuickslot & rSlot);
		bool			DelQuickslot(BYTE pos);
		bool			SwapQuickslot(BYTE a, BYTE b);
		void			ChainQuickslotItem(LPITEM pItem, BYTE bType, BYTE bOldPos);

	protected:
		TQuickslot		m_quickslot[QUICKSLOT_MAX_NUM];
#ifdef ENABLE_FISH_EVENT
		TPlayerFishEventSlot*	m_fishSlots;
#endif
	public:
		void			StartAffectEvent();
		void			ClearAffect(bool bSave=false);
		void			ClearAffect_New(bool bSave = false);
		void			ComputeAffect(CAffect * pkAff, bool bAdd);
		bool			AddAffect(DWORD dwType, BYTE bApplyOn, long lApplyValue, DWORD dwFlag, long lDuration, long lSPCost, bool bOverride, bool IsCube = false);
		void			RefreshAffect();
		bool			RemoveAffect(DWORD dwType);
		bool			IsAffectFlag(DWORD dwAff) const;


		bool			UpdateAffect();	// called from EVENT
		int				ProcessAffect();

		void			LoadAffect(DWORD dwCount, TPacketAffectElement * pElements);
		void			SaveAffect();

		// Affect loading�� ���� �����ΰ�?
		bool			IsLoadedAffect() const	{ return m_bIsLoadedAffect; }

		bool			IsGoodAffect(BYTE bAffectType) const;

		void			RemoveGoodAffect();
		void			RemoveBadAffect();

		CAffect *		FindAffect(DWORD dwType, BYTE bApply=APPLY_NONE) const;
		const std::list<CAffect *> & GetAffectContainer() const	{ return m_list_pkAffect; }
		bool			RemoveAffect(CAffect * pkAff);

	protected:
		bool			m_bIsLoadedAffect;
		TAffectFlag		m_afAffectFlag;
		std::list<CAffect *>	m_list_pkAffect;

	public:
		void			SetParty(LPPARTY pkParty);
		LPPARTY			GetParty() const	{ return m_pkParty; }

		bool			RequestToParty(LPCHARACTER leader);
		void			DenyToParty(LPCHARACTER member);
		void			AcceptToParty(LPCHARACTER member);

		void			PartyInvite(LPCHARACTER pchInvitee);

		void			PartyInviteAccept(LPCHARACTER pchInvitee);

		void			PartyInviteDeny(DWORD dwPID);

		bool			BuildUpdatePartyPacket(TPacketGCPartyUpdate & out);
		int				GetLeadershipSkillLevel() const;

		bool			CanSummon(int iLeaderShip);

		void			SetPartyRequestEvent(LPEVENT pkEvent) { m_pkPartyRequestEvent = pkEvent; }

	protected:

		void			PartyJoin(LPCHARACTER pkLeader);

		enum PartyJoinErrCode {
			PERR_NONE		= 0,	///< ó������
			PERR_SERVER,			///< ���������� ��Ƽ���� ó�� �Ұ�
			PERR_DUNGEON,			///< ĳ���Ͱ� ������ ����
			PERR_OBSERVER,			///< ���������
			PERR_LVBOUNDARY,		///< ��� ĳ���Ϳ� �������̰� ��
			PERR_LOWLEVEL,			///< �����Ƽ�� �ְ��������� 30���� ����
			PERR_HILEVEL,			///< �����Ƽ�� ������������ 30���� ����
			PERR_ALREADYJOIN,		///< ��Ƽ���� ��� ĳ���Ͱ� �̹� ��Ƽ��
			PERR_PARTYISFULL,		///< ��Ƽ�ο� ���� �ʰ�
			PERR_SEPARATOR,			///< Error type separator.
			PERR_DIFFEMPIRE,		///< ��� ĳ���Ϳ� �ٸ� ������
			PERR_MAX				///< Error code �ְ�ġ. �� �տ� Error code �� �߰��Ѵ�.
		};


		static PartyJoinErrCode	IsPartyJoinableCondition(const LPCHARACTER pchLeader, const LPCHARACTER pchGuest);
		static PartyJoinErrCode	IsPartyJoinableMutableCondition(const LPCHARACTER pchLeader, const LPCHARACTER pchGuest);

		LPPARTY			m_pkParty;
		DWORD			m_dwLastDeadTime;
		LPEVENT			m_pkPartyRequestEvent;

		typedef std::map< DWORD, LPEVENT >	EventMap;
		EventMap		m_PartyInviteEventMap;
	public:
		void			SetDungeon(LPDUNGEON pkDungeon);
		LPDUNGEON		GetDungeon() const	{ return m_pkDungeon; }
		LPDUNGEON		GetDungeonForce() const;
	protected:
		LPDUNGEON	m_pkDungeon;
		int			m_iEventAttr;
	public:
		void			SetGuild(CGuild * pGuild);
		CGuild*			GetGuild() const	{ return m_pGuild; }

		void			SetWarMap(CWarMap* pWarMap);
		CWarMap*		GetWarMap() const	{ return m_pWarMap; }

	protected:
		CGuild *		m_pGuild;
		DWORD			m_dwUnderGuildWarInfoMessageTime;
		CWarMap *		m_pWarMap;
	public:
		bool			CanHandleItem(bool bSkipRefineCheck = false, bool bSkipObserver = false); // ������ ���� ������ �� �� �ִ°�?

		bool			IsItemLoaded() const	{ return m_bItemLoaded; }
		void			SetItemLoaded()	{ m_bItemLoaded = true; }

		void			ClearItem();
#ifdef __SORT_INVENTORY_ITEMS__
		void SortInventoryItems();
		void SetSortInventoryPulse(int pulse) { m_sortInventoryPulse = pulse; }
		int GetSortInventoryPulse() { return m_sortInventoryPulse; }
#endif
#if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
		void	SetItem(TItemPos Cell, LPITEM item, bool bHighlight = true);
#else
		void	SetItem(TItemPos Cell, LPITEM item);
#endif
		LPITEM			GetItem(TItemPos Cell) const;
		LPITEM			GetInventoryItem(WORD wCell) const;
		bool			IsEmptyItemGrid(TItemPos Cell, BYTE size, int iExceptionCell = -1) const;
		bool IsEmptyItemGridSpecial(const TItemPos &Cell, BYTE bSize, int iExceptionCell, std::vector<WORD>& vec) const;
		void			SetWear(BYTE bCell, LPITEM item);
		LPITEM			GetWear(BYTE bCell) const;
#ifdef ENABLE_CUSTOM_INVENTORY
		LPITEM			GetCustomInventoryItem(BYTE bCategory, WORD wCell) const;
		int				GetEmptyCustomInventory(BYTE bCategory, BYTE size) const;
		BYTE			GetInventoryPageByPos(int iCategory, WORD wPos) const;
		int 			GetInventoryTypeByPos(WORD wPos) const;
		int				GetEmptyInventory(LPITEM pItem, BYTE bSearchInventory = 0) const;
#endif
		void			UseSilkBotary(void); 		/// ��� ������ �������� ���
		void			UseSilkBotaryReal(const TPacketMyshopPricelistHeader* p);
		bool			UseItemEx(LPITEM item, TItemPos DestCell);
		bool			UseItem(TItemPos Cell, TItemPos DestCell = NPOS);
		bool			IsRefineThroughGuild() const;
		CGuild *		GetRefineGuild() const;
		int				ComputeRefineFee(int iCost, int iMultiply = 5) const;
		void			PayRefineFee(int iTotalMoney);
		void			SetRefineNPC(LPCHARACTER ch);
		bool			RefineItem(LPITEM pkItem, LPITEM pkTarget);
		bool			DestroyItem(TItemPos Cell);
		bool			DropItem(TItemPos Cell,  WORD bCount=0);
		bool			GiveRecallItem(LPITEM item);
		void			ProcessRecallItem(LPITEM item);
		void			EffectPacket(int enumEffectType);
		void			SpecificEffectPacket(const char filename[128]);
		bool			DoRefine(LPITEM item, bool bMoneyOnly = false);
		bool			DoRefineWithScroll(LPITEM item);
#ifdef ENABLE_CUSTOM_INVENTORY
		bool			RefineInformation(WORD bCell, BYTE bType, int iAdditionalCell = -1);
#else
		bool			RefineInformation(BYTE bCell, BYTE bType, int iAdditionalCell = -1);
#endif
		void			SetRefineMode(int iAdditionalCell = -1);
		void			ClearRefineMode();
		bool			GiveItem(LPCHARACTER victim, TItemPos Cell);
		bool			CanReceiveItem(LPCHARACTER from, LPITEM item) const;
		void			ReceiveItem(LPCHARACTER from, LPITEM item);
		bool			GiveItemFromSpecialItemGroup(DWORD dwGroupNum, std::vector <DWORD> &dwItemVnums,
						std::vector <DWORD> &dwItemCounts, std::vector <LPITEM> &item_gets, int &count);
		bool			MoveItem(TItemPos pos, TItemPos change_pos, WORD num);
		bool			PickupItem(DWORD vid);
		bool			EquipItem(LPITEM item, int iCandidateCell = -1);
		bool			UnequipItem(LPITEM item);
		bool			CanEquipNow(const LPITEM item, const TItemPos& srcCell = NPOS, const TItemPos& destCell = NPOS);
		bool			CanUnequipNow(const LPITEM item, const TItemPos& srcCell = NPOS, const TItemPos& destCell = NPOS);
#ifdef ENABLE_CUSTOM_INVENTORY
		bool			SwapItem(WORD bCell, WORD bDestCell);
#else	
		bool			SwapItem(BYTE bCell, BYTE bDestCell);
#endif
		LPITEM			AutoGiveItem(DWORD dwItemVnum, WORD bCount=1, int iRarePct = -1, bool bMsg = true);
		void			AutoGiveItem(LPITEM item, bool longOwnerShip = false);
		int				GetEmptyInventory(BYTE size) const;
		int				GetEmptyInventoryEx(LPITEM item);
		int				GetEmptyDragonSoulInventory(LPITEM pItem) const;
		void			CopyDragonSoulItemGrid(std::vector<WORD>& vDragonSoulItemGrid) const;
		int				GetEmptyDragonSoulInventoryWithExceptions(LPITEM pItem, std::vector<WORD>& vec /*= -1*/) const;
		int				CountEmptyInventory() const;

		int				CountSpecifyItem(DWORD vnum) const;
		void			RemoveSpecifyItem(DWORD vnum, DWORD count = 1);
		LPITEM			FindSpecifyItem(DWORD vnum) const;
		LPITEM			FindItemByID(DWORD id) const;

		int				CountSpecifyTypeItem(BYTE type) const;
		void			RemoveSpecifyTypeItem(BYTE type, DWORD count = 1);

		bool			IsEquipUniqueItem(DWORD dwItemVnum) const;

		// CHECK_UNIQUE_GROUP
		bool			IsEquipUniqueGroup(DWORD dwGroupVnum) const;
		// END_OF_CHECK_UNIQUE_GROUP

		void			SendEquipment(LPCHARACTER ch);
		// End of Item
	protected:
		void			SendMyShopPriceListCmd(DWORD dwItemVnum, long long dwItemPrice);

		bool			m_bNoOpenedShop;	///< �̹� ���� �� ���λ����� �� ���� �ִ����� ����(������ ���� ���ٸ� true)

		bool			m_bItemLoaded;
		int				m_iRefineAdditionalCell;
		bool			m_bUnderRefine;
		DWORD			m_dwRefineNPCVID;

#ifdef __SORT_INVENTORY_ITEMS__
		int m_sortInventoryPulse;
#endif

	public:
		////////////////////////////////////////////////////////////////////////////////////////
		// Money related
#ifdef ENABLE_REMOVE_LIMIT_GOLD
		unsigned long long 	GetGold() const		{ return m_points.gold;	}
		void 				SetGold(unsigned long long gold)	{ m_points.gold = gold;	}
		bool				DropGold(INT gold);
		unsigned long long 	GetAllowedGold() const;
		void				GiveGold(INT iAmount);
		void 				ChangeGold(long long amount);
#else
		INT				GetGold() const		{ return m_points.gold;	}
		void			SetGold(INT gold)	{ m_points.gold = gold;	}
		bool			DropGold(INT gold);
		INT				GetAllowedGold() const;
		void			GiveGold(INT iAmount);
#endif

#ifdef ENABLE_GAYA_SYSTEM
		INT				GetGaya() const		{ return m_points.gaya;	}
		void			SetGaya(INT gaya)	{ m_points.gaya = gaya;	}
#endif

		// End of Money
#ifdef ENABLE_EXTEND_INVEN_SYSTEM
		INT				Inven_Point() const			{ return m_points.envanter; }
		INT				Inventory_Size() const			{ return INVENTORY_OPEN_PAGE_SIZE + (INVENTORY_WIDTH*Inven_Point()); }
		void			Set_Inventory_Point(INT black)	{ m_points.envanter = black; }
		bool            Update_Inven();
#endif
		////////////////////////////////////////////////////////////////////////////////////////
		// Shop related
	public:
		void			SetShop(LPSHOP pkShop);
		LPSHOP			GetShop() const { return m_pkShop; }
		void			ShopPacket(BYTE bSubHeader);

		void			SetShopOwner(LPCHARACTER ch) { m_pkChrShopOwner = ch; }
		LPCHARACTER		GetShopOwner() const { return m_pkChrShopOwner;}

		void			OpenMyShop(const char * c_pszSign, TShopItemTable * pTable, BYTE bItemCount);
		LPSHOP			GetMyShop() const { return m_pkMyShop; }
		void			CloseMyShop();

	protected:

		LPSHOP			m_pkShop;
		LPSHOP			m_pkMyShop;
		std::string		m_stShopSign;
		LPCHARACTER		m_pkChrShopOwner;
		// End of shop

		////////////////////////////////////////////////////////////////////////////////////////
		// Exchange related
	public:
		bool			ExchangeStart(LPCHARACTER victim);
		void			SetExchange(CExchange * pkExchange);
		CExchange *		GetExchange() const	{ return m_pkExchange;	}
#ifdef ENABLE_FISH_EVENT
		void 			FishEventGeneralInfo();
		void			FishEventUseBox(TItemPos itemPos);
		bool 			FishEventIsValidPosition(BYTE shapePos, BYTE shapeType);
		void 			FishEventPlaceShape(BYTE shapePos, BYTE shapeType);
		void 			FishEventAddShape(BYTE shapePos);
		void 			FishEventCheckEnd();
#endif

	protected:
		CExchange *		m_pkExchange;
		// End of Exchange

		////////////////////////////////////////////////////////////////////////////////////////
		// Battle
	public:
		struct TBattleInfo
		{
			int iTotalDamage;
			int iAggro;

			TBattleInfo(int iTot, int iAggr)
				: iTotalDamage(iTot), iAggro(iAggr)
				{}
		};
		typedef std::map<VID, TBattleInfo>	TDamageMap;

		typedef struct SAttackLog
		{
			DWORD	dwVID;
			DWORD	dwTime;
		} AttackLog;

		bool				Damage(LPCHARACTER pAttacker, int dam, EDamageType type = DAMAGE_TYPE_NORMAL);
		bool				__Profile__Damage(LPCHARACTER pAttacker, int dam, EDamageType type = DAMAGE_TYPE_NORMAL);
		void				DeathPenalty(BYTE bExpLossPercent);
		void				ReviveInvisible(int iDur);

		bool				Attack(LPCHARACTER pkVictim, BYTE bType = 0);
		bool				IsAlive() const		{ return m_pointsInstant.position == POS_DEAD ? false : true; }
		bool				CanFight() const;

		bool				CanBeginFight() const;
		void				BeginFight(LPCHARACTER pkVictim);

		bool				CounterAttack(LPCHARACTER pkChr);

		bool				IsStun() const;
		void				Stun();
		bool				IsDead() const;
		void				Dead(LPCHARACTER pkKiller = NULL, bool bImmediateDead=false);

		void				Reward(bool bItemDrop);
		void				RewardGold(LPCHARACTER pkAttacker);

		bool				Shoot(BYTE bType);
		void				FlyTarget(DWORD dwTargetVID, long x, long y, BYTE bHeader);

		void				ForgetMyAttacker();
		void				AggregateMonster();
		void				AttractRanger();
		void				PullMonster();

		int					GetArrowAndBow(LPITEM * ppkBow, LPITEM * ppkArrow, int iArrowCount = 1);
		void				UseArrow(LPITEM pkArrow, DWORD dwArrowCount);

		void				AttackedByPoison(LPCHARACTER pkAttacker);
		void				RemovePoison();

		void				AttackedByFire(LPCHARACTER pkAttacker, int amount, int count);
		void				RemoveFire();
		void				UpdateAlignment(int iAmount);
		int					GetAlignment() const;


		//����ġ ���
		int					GetRealAlignment() const;
		void				ShowAlignment(bool bShow);

		void				SetKillerMode(bool bOn);
		bool				IsKillerMode() const;
		void				UpdateKillerMode();

		BYTE				GetPKMode() const;
		void				SetPKMode(BYTE bPKMode);

		void				ItemDropPenalty(LPCHARACTER pkKiller);

		void				UpdateAggrPoint(LPCHARACTER ch, EDamageType type, int dam);

		//
		// HACK
		//
	public:
		void SetComboSequence(BYTE seq);
		BYTE GetComboSequence() const;

		void SetLastComboTime(DWORD time);
		DWORD GetLastComboTime() const;

		int GetValidComboInterval() const;
		void SetValidComboInterval(int interval);

		BYTE GetComboIndex() const;

		void IncreaseComboHackCount(int k = 1);
		void ResetComboHackCount();
		void SkipComboAttackByTime(int interval);
		DWORD GetSkipComboAttackByTime() const;

	protected:
		BYTE m_bComboSequence;
		DWORD m_dwLastComboTime;
		int m_iValidComboInterval;
		BYTE m_bComboIndex;
		int m_iComboHackCount;
		DWORD m_dwSkipComboAttackByTime;

	protected:
		void				UpdateAggrPointEx(LPCHARACTER ch, EDamageType type, int dam, TBattleInfo & info);
		void				ChangeVictimByAggro(int iNewAggro, LPCHARACTER pNewVictim);

		DWORD				m_dwFlyTargetID;
		std::vector<DWORD>	m_vec_dwFlyTargets;
		TDamageMap			m_map_kDamage;	
		DWORD				m_dwKillerPID;

		int					m_iAlignment;	
		int					m_iRealAlignment;

		int					m_iKillerModePulse;
		BYTE				m_bPKMode;
		DWORD				m_dwLastVictimSetTime;
		int					m_iMaxAggro;

	public:
		void				SetStone(LPCHARACTER pkChrStone);
		void				ClearStone();
		void				DetermineDropMetinStone();
		DWORD				GetDropMetinStoneVnum() const { return m_dwDropMetinStone; }
		BYTE				GetDropMetinStonePct() const { return m_bDropMetinStonePct; }

	protected:
		LPCHARACTER			m_pkChrStone;		// ���� ������ ��
		CHARACTER_SET		m_set_pkChrSpawnedBy;	// ���� ������ ���
		DWORD				m_dwDropMetinStone;
		BYTE				m_bDropMetinStonePct;
		// End of Stone

	public:
		enum
		{
			SKILL_UP_BY_POINT,
			SKILL_UP_BY_BOOK,
			SKILL_UP_BY_TRAIN,

			// ADD_GRANDMASTER_SKILL
			SKILL_UP_BY_QUEST,
			// END_OF_ADD_GRANDMASTER_SKILL
		};
#ifdef __7AND8TH_SKILLS__
		bool				SkillCanUp(DWORD dwVnum);
#endif

		void				SkillLevelPacket();
		void				SkillLevelUp(DWORD dwVnum, BYTE bMethod = SKILL_UP_BY_POINT);
		bool				SkillLevelDown(DWORD dwVnum);
		// ADD_GRANDMASTER_SKILL
		bool				UseSkill(DWORD dwVnum, LPCHARACTER pkVictim, bool bUseGrandMaster = true);
		void				ResetSkill();
		void				SetSkillLevel(DWORD dwVnum, BYTE bLev);
		int					GetUsedSkillMasterType(DWORD dwVnum);

		bool				IsLearnableSkill(DWORD dwSkillVnum) const;
		// END_OF_ADD_GRANDMASTER_SKILL

		bool				CheckSkillHitCount(const BYTE SkillID, const VID dwTargetVID);
		bool				CanUseSkill(DWORD dwSkillVnum) const;
		bool				IsUsableSkillMotion(DWORD dwMotionIndex) const;
		int					GetSkillLevel(DWORD dwVnum) const;
		int					GetSkillMasterType(DWORD dwVnum) const;
		int					GetSkillPower(DWORD dwVnum, BYTE bLevel = 0) const;

		time_t				GetSkillNextReadTime(DWORD dwVnum) const;
		void				SetSkillNextReadTime(DWORD dwVnum, time_t time);
		void				SkillLearnWaitMoreTimeMessage(DWORD dwVnum);

		void				ComputePassiveSkill(DWORD dwVnum);
		int					ComputeSkill(DWORD dwVnum, LPCHARACTER pkVictim, BYTE bSkillLevel = 0);
		int					ComputeSkillParty(DWORD dwVnum, LPCHARACTER pkVictim, BYTE bSkillLevel = 0);
		int					ComputeSkillAtPosition(DWORD dwVnum, const PIXEL_POSITION& posTarget, BYTE bSkillLevel = 0);
		void				ComputeSkillPoints();

		void				SetSkillGroup(BYTE bSkillGroup);
		BYTE				GetSkillGroup() const		{ return m_points.skill_group; }

		int					ComputeCooltime(int time);

		void				GiveRandomSkillBook();

		void				DisableCooltime();
		bool				LearnSkillByBook(DWORD dwSkillVnum, BYTE bProb = 0);
		bool				LearnGrandMasterSkill(DWORD dwSkillVnum);

	private:
		bool				m_bDisableCooltime;
		DWORD				m_dwLastSkillTime;	///< ���������� skill �� �� �ð�(millisecond).
		// End of Skill

		// MOB_SKILL
	public:
		bool				HasMobSkill() const;
		size_t				CountMobSkill() const;
		const TMobSkillInfo * GetMobSkill(unsigned int idx) const;
		bool				CanUseMobSkill(unsigned int idx) const;
		bool				UseMobSkill(unsigned int idx);
		void				ResetMobSkillCooltime();
	protected:
		DWORD				m_adwMobSkillCooltime[MOB_SKILL_MAX_NUM];
		// END_OF_MOB_SKILL

		// for SKILL_MUYEONG
	public:
		void				StartMuyeongEvent();
		void				StopMuyeongEvent();

	private:
		LPEVENT				m_pkMuyeongEvent;

		// for SKILL_CHAIN lighting
	public:
		int					GetChainLightningIndex() const { return m_iChainLightingIndex; }
		void				IncChainLightningIndex() { ++m_iChainLightingIndex; }
		void				AddChainLightningExcept(LPCHARACTER ch) { m_setExceptChainLighting.insert(ch); }
		void				ResetChainLightningIndex() { m_iChainLightingIndex = 0; m_setExceptChainLighting.clear(); }
		int					GetChainLightningMaxCount() const;
		const CHARACTER_SET& GetChainLightingExcept() const { return m_setExceptChainLighting; }

	private:
		int					m_iChainLightingIndex;
		CHARACTER_SET m_setExceptChainLighting;

		// for SKILL_EUNHYUNG
	public:
		void				SetAffectedEunhyung();
		void				ClearAffectedEunhyung() { m_dwAffectedEunhyungLevel = 0; }
		bool				GetAffectedEunhyung() const { return m_dwAffectedEunhyungLevel; }

	private:
		DWORD				m_dwAffectedEunhyungLevel;

		//
		// Skill levels
		//
	protected:
		TPlayerSkill*					m_pSkillLevels;
		boost::unordered_map<BYTE, int>		m_SkillDamageBonus;
		std::map<int, TSkillUseInfo>	m_SkillUseInfo;

		////////////////////////////////////////////////////////////////////////////////////////
		// AI related
	public:
		void			AssignTriggers(const TMobTable * table);
		LPCHARACTER		GetVictim() const;	// ������ ��� ����
		void			SetVictim(LPCHARACTER pkVictim);
		LPCHARACTER		GetNearestVictim(LPCHARACTER pkChr);
		LPCHARACTER		GetProtege() const;	// ��ȣ�ؾ� �� ��� ����
		bool			Follow(LPCHARACTER pkChr, float fMinimumDistance = 150.0f);
		bool			Return();
		bool			IsGuardNPC() const;
		bool			IsChangeAttackPosition(LPCHARACTER target) const;
		void			ResetChangeAttackPositionTime() { m_dwLastChangeAttackPositionTime = get_dword_time() - AI_CHANGE_ATTACK_POISITION_TIME_NEAR;}
		void			SetChangeAttackPositionTime() { m_dwLastChangeAttackPositionTime = get_dword_time();}

		bool			OnIdle();

		void			OnAttack(LPCHARACTER pkChrAttacker);
		void			OnClick(LPCHARACTER pkChrCauser);

		VID				m_kVIDVictim;

	protected:
		DWORD			m_dwLastChangeAttackPositionTime;
		CTrigger		m_triggerOnClick;
		// End of AI

		////////////////////////////////////////////////////////////////////////////////////////
		// Target
	protected:
		LPCHARACTER				m_pkChrTarget;		// �� Ÿ��
		CHARACTER_SET	m_set_pkChrTargetedBy;	// ���� Ÿ������ ������ �ִ� �����

	public:
		void				SetTarget(LPCHARACTER pkChrTarget);
		void				BroadcastTargetPacket();
		void				ClearTarget();
		void				CheckTarget();
		LPCHARACTER			GetTarget() const { return m_pkChrTarget; }

		////////////////////////////////////////////////////////////////////////////////////////
		// Safebox
	public:
		int					GetSafeboxSize() const;
		void				QuerySafeboxSize();
		void				SetSafeboxSize(int size);

		CSafebox *			GetSafebox() const;
		void				LoadSafebox(int iSize, DWORD dwGold, int iItemCount, TPlayerItem * pItems);
		void				ChangeSafeboxSize(BYTE bSize);
		void				CloseSafebox();

		void				ReqSafeboxLoad(const char* pszPassword);

		void				CancelSafeboxLoad( void ) { m_bOpeningSafebox = false; }

		void				SetMallLoadTime(int t) { m_iMallLoadTime = t; }
		int					GetMallLoadTime() const { return m_iMallLoadTime; }

		CSafebox *			GetMall() const;
		void				LoadMall(int iItemCount, TPlayerItem * pItems);
		void				CloseMall();

		void				SetSafeboxOpenPosition();
		float				GetDistanceFromSafeboxOpen() const;

	protected:
		CSafebox *			m_pkSafebox;
		int					m_iSafeboxSize;
		int					m_iSafeboxLoadTime;
		bool				m_bOpeningSafebox;	///< â���� ���� ��û ���̰ų� �����ִ°� ����, true �� ��� �����û�̰ų� ��������.

		CSafebox *			m_pkMall;
		int					m_iMallLoadTime;

		PIXEL_POSITION		m_posSafeboxOpen;

	public:
		void				MountVnum(DWORD vnum);
		DWORD				GetMountVnum() const { return m_dwMountVnum; }
		DWORD				GetLastMountTime() const { return m_dwMountTime; }

		bool				CanUseHorseSkill();

		// Horse
		virtual	void		SetHorseLevel(int iLevel);

		virtual	bool		StartRiding();
		virtual	bool		StopRiding();

		virtual	DWORD		GetMyHorseVnum() const;

		virtual	void		HorseDie();
		virtual bool		ReviveHorse();

		virtual void		SendHorseInfo();
		virtual	void		ClearHorseInfo();

		void				HorseSummon(bool bSummon, bool bFromFar = false, DWORD dwVnum = 0, const char* name = 0);

		LPCHARACTER			GetHorse() const			{ return m_chHorse; }	 // ���� ��ȯ���� ��
		LPCHARACTER			GetRider() const; // rider on horse
		void				SetRider(LPCHARACTER ch);

#ifdef ENABLE_PET_COSTUME_SYSTEM
	public:
		CPetSystem*			GetPetSystem()				{ return m_petSystem; }
		void 				PetSummon(LPITEM petItem);
		void 				PetUnsummon(LPITEM petItem);
		void 				CheckPet();

	protected:
		CPetSystem*			m_petSystem;

	public:
#endif

		bool				IsRiding() const;

#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
	public:
		CMountSystem*		GetMountSystem() { return m_mountSystem; }
		
		void 				MountSummon(LPITEM mountItem);
		void 				MountUnsummon(LPITEM mountItem);
		void 				CheckMount();
		bool 				IsRidingMount();
	protected:
		CMountSystem*		m_mountSystem;
#endif


#ifdef __ENABLE_SHAMAN_SYSTEM__
	public:
		CShamanSystem* GetShamanSystem() { return m_shamanSystem; }
		
		void SetAutoShaman(CShamanActor* pShaman) { m_shamanActor = pShaman; }
		bool IsAutoShaman() { return m_shamanActor; }
		
		void SendAutoShamanSkill(DWORD dwSkillVnum, BYTE byLevel);
		void SendAutoShamanInformations();
		
	protected:
		CShamanSystem* m_shamanSystem;
		CShamanActor* m_shamanActor;
#endif

	protected:
		LPCHARACTER			m_chHorse;
		LPCHARACTER			m_chRider;

		DWORD				m_dwMountVnum;
		DWORD				m_dwMountTime;

		BYTE				m_bSendHorseLevel;
		BYTE				m_bSendHorseHealthGrade;
		BYTE				m_bSendHorseStaminaGrade;

	public:
		void				DetailLog() { m_bDetailLog = !m_bDetailLog; }
		void				ToggleMonsterLog();
		void				MonsterLog(const char* format, ...);
	private:
		bool				m_bDetailLog;
		bool				m_bMonsterLog;


	public:
		void 				SetEmpire(BYTE bEmpire);
		BYTE				GetEmpire() const { return m_bEmpire; }

	protected:
		BYTE				m_bEmpire;

	public:
		void				SetRegen(LPREGEN pkRegen);

	protected:
		PIXEL_POSITION			m_posRegen;
		float				m_fRegenAngle;
		LPREGEN				m_pkRegen;
		size_t				regen_id_; // to help dungeon regen identification

	public:
		bool				CannotMoveByAffect() const;	// Ư�� ȿ���� ���� ������ �� ���� �����ΰ�?
		bool				IsImmune(DWORD dwImmuneFlag);
		void				SetImmuneFlag(DWORD dw) { m_pointsInstant.dwImmuneFlag = dw; }

	protected:
		void				ApplyMobAttribute(const TMobTable* table);
		// End of Resists & Proofs

		////////////////////////////////////////////////////////////////////////////////////////
		// QUEST
		//
	public:
		void				SetQuestNPCID(DWORD vid);
		DWORD				GetQuestNPCID() const { return m_dwQuestNPCVID; }
		LPCHARACTER			GetQuestNPC() const;

		void				SetQuestItemPtr(LPITEM item);
		void				ClearQuestItemPtr();
		LPITEM				GetQuestItemPtr() const;

		void				SetQuestBy(DWORD dwQuestVnum)	{ m_dwQuestByVnum = dwQuestVnum; }
		DWORD				GetQuestBy() const			{ return m_dwQuestByVnum; }

		int					GetQuestFlag(const std::string& flag) const;
		void				SetQuestFlag(const std::string& flag, int value);

		void				ConfirmWithMsg(const char* szMsg, int iTimeout, DWORD dwRequestPID);

	private:
		DWORD				m_dwQuestNPCVID;
		DWORD				m_dwQuestByVnum;
		LPITEM				m_pQuestItem;

		// Events
	public:
		bool				StartStateMachine(int iPulse = 1);
		void				StopStateMachine();
		void				UpdateStateMachine(DWORD dwPulse);
		void				SetNextStatePulse(int iPulseNext);

		// ĳ���� �ν��Ͻ� ������Ʈ �Լ�. ������ �̻��� ��ӱ����� CFSM::Update �Լ��� ȣ���ϰų� UpdateStateMachine �Լ��� ����ߴµ�, ������ ������Ʈ �Լ� �߰���.
		void				UpdateCharacter(DWORD dwPulse);

	protected:
		DWORD				m_dwNextStatePulse;

		// Marriage
	public:
		LPCHARACTER			GetMarryPartner() const;
		void				SetMarryPartner(LPCHARACTER ch);
		int					GetMarriageBonus(DWORD dwItemVnum, bool bSum = true);

		void				SetWeddingMap(marriage::WeddingMap* pMap);
		marriage::WeddingMap* GetWeddingMap() const { return m_pWeddingMap; }

	private:
		marriage::WeddingMap* m_pWeddingMap;
		LPCHARACTER			m_pkChrMarried;

		// Warp Character
	public:
		void				StartWarpNPCEvent();

	public:
		void				StartSaveEvent();
		void				StartRecoveryEvent();
		void				StartCheckSpeedHackEvent();
		void				StartDestroyWhenIdleEvent();

		LPEVENT				m_pkDeadEvent;
		LPEVENT				m_pkStunEvent;
		LPEVENT				m_pkSaveEvent;
		LPEVENT				m_pkRecoveryEvent;
		LPEVENT				m_pkTimedEvent;
		LPEVENT				m_pkFishingEvent;
		LPEVENT				m_pkAffectEvent;
		LPEVENT				m_pkPoisonEvent;

		LPEVENT				m_pkFireEvent;
		LPEVENT				m_pkWarpNPCEvent;
		//DELAYED_WARP
		//END_DELAYED_WARP

		// MINING
		LPEVENT				m_pkMiningEvent;
		// END_OF_MINING
		LPEVENT				m_pkWarpEvent;
		LPEVENT				m_pkCheckSpeedHackEvent;
		LPEVENT				m_pkDestroyWhenIdleEvent;
		LPEVENT				m_pkPetSystemUpdateEvent;
#ifdef __ENABLE_PREMIUM_PLAYERS__
		LPEVENT 			m_pkPremiumPlayersUpdateEvent;
#endif

		bool IsWarping() const { return m_pkWarpEvent ? true : false; }

		bool				m_bHasPoisoned;
		const CMob *		m_pkMobData;
		CMobInstance *		m_pkMobInst;

		std::map<int, LPEVENT> m_mapMobSkillEvent;

		friend struct FuncSplashDamage;
		friend struct FuncSplashAffect;
		friend class CFuncShoot;

	public:
		int				GetPremiumRemainSeconds(BYTE bType) const;

	private:
		int				m_aiPremiumTimes[PREMIUM_MAX_NUM];


		static const char		msc_szLastChangeItemAttrFlag[];	

	private :
		bool m_isinPCBang;

	public :
		bool SetPCBang(bool flag) { m_isinPCBang = flag; return m_isinPCBang; }
		bool IsPCBang() const { return m_isinPCBang; }
		// END_PC_BANG_ITEM_ADD

		// NEW_HAIR_STYLE_ADD
	public :
		bool ItemProcess_Hair(LPITEM item, int iDestCell);
		// END_NEW_HAIR_STYLE_ADD

	public :
		void ClearSkill();
		void ClearSubSkill();

		// RESET_ONE_SKILL
		bool ResetOneSkill(DWORD dwVnum);
		// END_RESET_ONE_SKILL

#ifdef ENABLE_RUNE_SYSTEM
public:
	void SendDamagePacket(LPCHARACTER pAttacker, int Damage, BYTE DamageFlag);
#else
private:
	void SendDamagePacket(LPCHARACTER pAttacker, int Damage, BYTE DamageFlag);
#endif

	// ARENA
	private :
		CArena *m_pArena;
		bool m_ArenaObserver;
		int m_nPotionLimit;

	public :
		void 	SetArena(CArena* pArena) { m_pArena = pArena; }
		void	SetArenaObserverMode(bool flag) { m_ArenaObserver = flag; }

		CArena* GetArena() const { return m_pArena; }
		bool	GetArenaObserverMode() const { return m_ArenaObserver; }

		void	SetPotionLimit(int count) { m_nPotionLimit = count; }
		int		GetPotionLimit() const { return m_nPotionLimit; }
	// END_ARENA

		//PREVENT_TRADE_WINDOW
	public:
		bool	IsOpenSafebox() const { return m_isOpenSafebox ? true : false; }
		void 	SetOpenSafebox(bool b) { m_isOpenSafebox = b; }

		int		GetSafeboxLoadTime() const { return m_iSafeboxLoadTime; }
		void	SetSafeboxLoadTime() { m_iSafeboxLoadTime = thecore_pulse(); }
		//END_PREVENT_TRADE_WINDOW
	private:
		bool	m_isOpenSafebox;

	public:
		int		GetSkillPowerByLevel(int level, bool bMob = false) const;

		//PREVENT_REFINE_HACK
		int		GetRefineTime() const { return m_iRefineTime; }
		void	SetRefineTime() { m_iRefineTime = thecore_pulse(); }
		int		m_iRefineTime;
		//END_PREVENT_REFINE_HACK

		//RESTRICT_USE_SEED_OR_MOONBOTTLE
		int 	GetUseSeedOrMoonBottleTime() const { return m_iSeedTime; }
		void  	SetUseSeedOrMoonBottleTime() { m_iSeedTime = thecore_pulse(); }
		int 	m_iSeedTime;
		//END_RESTRICT_USE_SEED_OR_MOONBOTTLE

		//PREVENT_PORTAL_AFTER_EXCHANGE
		int		GetExchangeTime() const { return m_iExchangeTime; }
		void	SetExchangeTime() { m_iExchangeTime = thecore_pulse(); }
		int		m_iExchangeTime;
		//END_PREVENT_PORTAL_AFTER_EXCHANGE

		int 	m_iMyShopTime;
		int		GetMyShopTime() const	{ return m_iMyShopTime; }
		void	SetMyShopTime() { m_iMyShopTime = thecore_pulse(); }

		// Hack ������ ���� üũ.
		bool	IsHack(bool bSendMsg = true, bool bCheckShopOwner = true, int limittime = g_nPortalLimitTime);

		// MONARCH
		BOOL	IsMonarch() const;
		// END_MONARCH
		void Say(const std::string & s);

		enum MONARCH_COOLTIME
		{
			MC_HEAL = 10,
			MC_WARP	= 60,
			MC_TRANSFER = 60,
			MC_TAX = (60 * 60 * 24 * 7),
			MC_SUMMON = (60 * 60),
		};

		enum MONARCH_INDEX
		{
			MI_HEAL = 0,
			MI_WARP,
			MI_TRANSFER,
			MI_TAX,
			MI_SUMMON,
			MI_MAX
		};

		DWORD m_dwMonarchCooltime[MI_MAX];
		DWORD m_dwMonarchCooltimelimit[MI_MAX];

		void  InitMC();
		DWORD GetMC(enum MONARCH_INDEX e) const;
		void SetMC(enum MONARCH_INDEX e);
		bool IsMCOK(enum MONARCH_INDEX e) const;
		DWORD GetMCL(enum MONARCH_INDEX e) const;
		DWORD GetMCLTime(enum MONARCH_INDEX e) const;

	public:
		bool ItemProcess_Polymorph(LPITEM item);

		// by mhh
		LPITEM*	GetCubeItem() { return m_pointsInstant.pCubeItems; }
		bool IsCubeOpen () const	{ return (m_pointsInstant.pCubeNpc?true:false); }
		void SetCubeNpc(LPCHARACTER npc)	{ m_pointsInstant.pCubeNpc = npc; }
		bool CanDoCube() const;

	public:
		bool IsSiegeNPC() const;

	private:
		e_overtime m_eOverTime;

	public:
		bool IsOverTime(e_overtime e) const { return (e == m_eOverTime); }
		void SetOverTime(e_overtime e) { m_eOverTime = e; }

	private:
		int		m_deposit_pulse;

	public:
		void	UpdateDepositPulse();
		bool	CanDeposit() const;

	private:
		void	__OpenPrivateShop();

	public:
		struct AttackedLog
		{
			DWORD 	dwPID;
			DWORD	dwAttackedTime;

			AttackedLog() : dwPID(0), dwAttackedTime(0)
			{
			}
		};

		AttackLog	m_kAttackLog;
		AttackedLog m_AttackedLog;
		int			m_speed_hack_count;

	private :
		std::string m_strNewName;

	public :
		const std::string GetNewName() const { return this->m_strNewName; }
		void SetNewName(const std::string name) { this->m_strNewName = name; }

	public :
		void GoHome();

	private :
		std::set<DWORD>	m_known_guild;

	public :
		void SendGuildName(CGuild* pGuild);
		void SendGuildName(DWORD dwGuildID);

	private :
		DWORD m_dwLogOffInterval;

	public :
		DWORD GetLogOffInterval() const { return m_dwLogOffInterval; }

#ifdef ENABLE_FISH_EVENT
	private:
		DWORD m_dwFishUseCount;
		BYTE m_bFishAttachedShape;
	public:
		DWORD GetFishEventUseCount() const { return m_dwFishUseCount; }
		void FishEventIncreaseUseCount() { m_dwFishUseCount++; }
		
		BYTE GetFishAttachedShape() const { return m_bFishAttachedShape; }
		void SetFishAttachedShape(BYTE bShape) { m_bFishAttachedShape = bShape; }
#endif

	public:
		bool UnEquipSpecialRideUniqueItem ();

		bool CanWarp () const;

	private:
		DWORD m_dwLastGoldDropTime;
#ifdef ENABLE_NEWSTUFF
		DWORD m_dwLastItemDropTime;
		DWORD m_dwLastBoxUseTime;
		DWORD m_dwLastBuySellTime;
	public:
		DWORD GetLastBuySellTime() const { return m_dwLastBuySellTime; }
		void SetLastBuySellTime(DWORD dwLastBuySellTime) { m_dwLastBuySellTime = dwLastBuySellTime; }
#endif
	public:
		void AutoRecoveryItemProcess (const EAffectTypes);

	public:
		void BuffOnAttr_AddBuffsFromItem(LPITEM pItem);
		void BuffOnAttr_RemoveBuffsFromItem(LPITEM pItem);

	private:
		void BuffOnAttr_ValueChange(BYTE bType, BYTE bOldValue, BYTE bNewValue);
		void BuffOnAttr_ClearAll();

		typedef std::map <BYTE, CBuffOnAttributes*> TMapBuffOnAttrs;
		TMapBuffOnAttrs m_map_buff_on_attrs;
		// ���� : ��Ȱ�� �׽�Ʈ�� ���Ͽ�.
	public:
		void SetArmada() { cannot_dead = true; }
		void ResetArmada() { cannot_dead = false; }
	private:
		bool cannot_dead;

#ifdef __PET_SYSTEM__
	private:
		bool m_bIsPet;
	public:
		void SetPet() { m_bIsPet = true; }
		bool IsPet() { return m_bIsPet; }
#endif

#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
	private:
		bool m_bIsMount;
	public:
		void SetMount() { m_bIsMount = true; }
		bool IsMount() { return m_bIsMount; }
#endif

	public:
		int			LStatusEveniment;
		void		SendDropLettersItem();

#ifdef NEW_ICEDAMAGE_SYSTEM
	private:
		DWORD m_dwNDRFlag;
		std::set<DWORD> m_setNDAFlag;
	public:
		const DWORD GetNoDamageRaceFlag();
		void SetNoDamageRaceFlag(DWORD dwRaceFlag);
		void UnsetNoDamageRaceFlag(DWORD dwRaceFlag);
		void ResetNoDamageRaceFlag();
		const std::set<DWORD> & GetNoDamageAffectFlag();
		void SetNoDamageAffectFlag(DWORD dwAffectFlag);
		void UnsetNoDamageAffectFlag(DWORD dwAffectFlag);
		void ResetNoDamageAffectFlag();
#endif

	//���� ������ ����.
	private:
		float m_fAttMul;
		float m_fDamMul;
	public:
		float GetAttMul() { return this->m_fAttMul; }
		void SetAttMul(float newAttMul) {this->m_fAttMul = newAttMul; }
		float GetDamMul() { return this->m_fDamMul; }
		void SetDamMul(float newDamMul) {this->m_fDamMul = newDamMul; }

	private:
		bool IsValidItemPosition(TItemPos Pos) const;

	public:
		void	DragonSoul_Initialize();

		bool	DragonSoul_IsQualified() const;
		void	DragonSoul_GiveQualification();

		int		DragonSoul_GetActiveDeck() const;
		bool	DragonSoul_IsDeckActivated() const;
		bool	DragonSoul_ActivateDeck(int deck_idx);

		void	DragonSoul_DeactivateAll();
		void	DragonSoul_CleanUp();

	public:
		bool		DragonSoul_RefineWindow_Open(LPENTITY pEntity);
		bool		DragonSoul_RefineWindow_Close();
		LPENTITY	DragonSoul_RefineWindow_GetOpener() { return  m_pointsInstant.m_pDragonSoulRefineWindowOpener; }
		bool		DragonSoul_RefineWindow_CanRefine();

#if defined(OFFLINE_MESSAGE_REWORKED)
	protected:
		DWORD				dwLastOfflinePMTime;
	public:
		DWORD				GetLastOfflinePMTime() const { return dwLastOfflinePMTime; }
		void				SetLastOfflinePMTime() { dwLastOfflinePMTime = get_dword_time(); }
		void				SendOfflineMessage(const char* To, const char* Message);
		void				ReadOfflineMessages();
#endif
	private:
		unsigned int itemAward_vnum;
		char		 itemAward_cmd[20];

	public:
		unsigned int GetItemAward_vnum() { return itemAward_vnum; }
		char*		 GetItemAward_cmd() { return itemAward_cmd;	  }
		void		 SetItemAward_vnum(unsigned int vnum) { itemAward_vnum = vnum; }
		void		 SetItemAward_cmd(char* cmd) { strcpy(itemAward_cmd,cmd); }
#ifdef ENABLE_ANTI_CMD_FLOOD
	private:
		int m_dwCmdAntiFloodPulse;
		DWORD m_dwCmdAntiFloodCount;
	public:
		int GetCmdAntiFloodPulse(){return m_dwCmdAntiFloodPulse;}
		DWORD GetCmdAntiFloodCount(){return m_dwCmdAntiFloodCount;}
		DWORD IncreaseCmdAntiFloodCount(){return ++m_dwCmdAntiFloodCount;}
		void SetCmdAntiFloodPulse(int dwPulse){m_dwCmdAntiFloodPulse=dwPulse;}
		void SetCmdAntiFloodCount(DWORD dwCount){m_dwCmdAntiFloodCount=dwCount;}
#endif
	private:
		timeval		m_tvLastSyncTime;
		int			m_iSyncHackCount;
	public:
		void			SetLastSyncTime(const timeval &tv) { memcpy(&m_tvLastSyncTime, &tv, sizeof(timeval)); }
		const timeval&	GetLastSyncTime() { return m_tvLastSyncTime; }
		void			SetSyncHackCount(int iCount) { m_iSyncHackCount = iCount;}
		int				GetSyncHackCount() { return m_iSyncHackCount; }



#ifdef ENABLE_GAYA_SYSTEM
	public:
		struct Gaya_Shop_Values
		{
			int		value_1;
			int		value_2;
			int 	value_3;
			int 	value_4;
			int 	value_5;
			int 	value_6;
			bool operator == (const Gaya_Shop_Values& b)
			{
				return (this->value_1 == b.value_1) && (this->value_2 == b.value_2) && 
					   (this->value_3 == b.value_3) && (this->value_4 == b.value_4) &&
					   (this->value_5 == b.value_5) && (this->value_6 == b.value_6);
			}
		};

		struct Gaya_Load_Values
		{

			DWORD	items;
			DWORD	gaya;
			DWORD	count;
			DWORD	glimmerstone;
			DWORD	gaya_expansion;
			DWORD	gaya_refresh;
			DWORD	glimmerstone_count;
			DWORD 	gaya_expansion_count;
			DWORD 	gaya_refresh_count;
			DWORD	grade_stone;
			DWORD	give_gaya;
			DWORD	prob_gaya;
			DWORD	cost_gaya_yang;
		};

		bool CheckItemsFull();
		void UpdateItemsGayaMarker0();
		void UpdateItemsGayaMarker(); 
		void InfoGayaMarker();
		void ClearGayaMarket();
		bool CheckSlotGayaMarket(int slot);
		void UpdateSlotGayaMarket(int slot);
		void BuyItemsGayaMarket(int slot);
		void RefreshItemsGayaMarket();
		void CraftGayaItems(int slot);
		void MarketGayaItems(int slot);
		void RefreshGayaItems();
		void lOAD_GAYA();
		int	GetGayaState(const std::string& state) const;
		void SetGayaState(const std::string& state, int szValue);
		void StartCheckTimeMarket();
		void StartCheckTimeMarketLogin();

	private:
		std::vector<Gaya_Shop_Values> info_items;
		std::vector<Gaya_Shop_Values> info_slots;	
		std::vector<Gaya_Load_Values> load_gaya_items;
		Gaya_Load_Values	load_gaya_values;
		LPEVENT	GayaUpdateTime;
#endif


#ifdef PRODOMO_HIDE_COSTUME
	public:
		void SetBodyCostumeHidden(bool hidden);
		bool IsBodyCostumeHidden() const { return m_bHideBodyCostume; };

		void SetHairCostumeHidden(bool hidden);
		bool IsHairCostumeHidden() const { return m_bHideHairCostume; };


		void SetSashCostumeHidden(bool hidden);
		bool IsSashCostumeHidden() const { return m_bHideSashCostume; };

		void SetAuraCostumeHidden(bool hidden);
		bool IsAuraCostumeHidden() const { return m_bHideAuraCostume; };

		void SetWeaponCostumeHidden(bool hidden);
		bool IsWeaponCostumeHidden() const { return m_bHideWeaponCostume; };

	private:
		bool m_bHideBodyCostume;
		bool m_bHideHairCostume;
		bool m_bHideSashCostume;
		bool m_bHideAuraCostume;
		bool m_bHideWeaponCostume;

#endif


		
#ifdef ENABLE_TOP_PLAYERS_EFFECT
		void			SetTopPlayerEffect();
#endif	

#ifdef OFFLINE_SHOP
	public:
		void			OpenMyShop(const char * c_pszSign, TShopItemTable * pTable, BYTE bItemCount, DWORD days);
		void			SendShops(bool isGm = false);
		void			OpenShop(DWORD id, const char *name, bool onboot = false);
		void			SetPrivShop(DWORD shop_id) { bprivShop = shop_id; }
		BOOL			IsPrivShop(void)  const { return bprivShop>0; }
		DWORD			GetPrivShop()  const { return bprivShop; }
		void			SetPrivShopOwner(DWORD id) { bprivShopOwner = id; }
		DWORD			GetPrivShopOwner()  const { return bprivShopOwner; }
		void			DeleteMyShop();
		DWORD			GetShopTime()  const { return dw_ShopTime; }
		void			SetShopTime(DWORD time) { dw_ShopTime = time; }
		void			SetShopSign(const char * name);
		void			LoadPrivShops();
		TPrivShop		GetPrivShopTable(DWORD id);
		void			RemovePrivShopTable(DWORD id);
		void			UpdatePrivShopTable(DWORD id, TPrivShop shop);
		void			UpdateShopItems();
		void			SendShopCost();
	private:
		PSHOP_MAP		m_mapshops;
		DWORD			bprivShop;
		DWORD			bprivShopOwner;
		DWORD			dw_ShopTime;
	public:
		void			StartRefreshShopEvent();
	protected:
		LPEVENT			m_pkRefreshShopEvent;
	public:
		void			StartShopEditModeEvent();
		void			SetShopEditMode(bool val);
		bool			GetShopEditMode() { return m_bShopEditMode; }
		void			SetShopEditModeTick();
		DWORD			GetShopEditModeTick() { return m_dwShopEditModeTick; }
	protected:
		LPEVENT			m_pkEditShopEvent;
		bool			m_bShopEditMode;
		DWORD			m_dwShopEditModeTick;
#endif

#ifdef GIFT_SYSTEM
	protected:
		void			AddGiftGrid(int page);
		int				AddGiftGridItem(int page, int size);
		GIFT_MAP		m_mapGiftGrid;
		LPEVENT			m_pkGiftRefresh;
		DWORD			m_dwLastGiftPage;
	public:
		void			StartRefreshGift();
		void			LoadGiftPage(int page);
		void			RefreshGift();
		int				GetGiftPages() { return m_mapGiftGrid.size(); }
		int				GetLastGiftPage() { return m_dwLastGiftPage; }
#endif

#ifdef ENABLE_SHOP_SEARCH
	protected:
		std::string		m_strShopOwnerName;
	public:
		void			SetShopOwnerName(std::string name){ m_strShopOwnerName=name; }
		const char *	GetShopOwnerName() const { return m_strShopOwnerName.empty()? GetName() : m_strShopOwnerName.c_str(); }
#endif

#ifdef __SASH_SYSTEM__
	protected:
		bool	m_bSashCombination, m_bSashAbsorption;
	
	public:
		bool	isSashOpened(bool bCombination) {return bCombination ? m_bSashCombination : m_bSashAbsorption;}
		void	OpenSash(bool bCombination);
		void	CloseSash();
		void	ClearSashMaterials();
		bool	CleanSashAttr(LPITEM pkItem, LPITEM pkTarget);
		LPITEM*	GetSashMaterials() {return m_pointsInstant.pSashMaterials;}
		bool	SashIsSameGrade(long lGrade);
		DWORD	GetSashCombinePrice(long lGrade);
		void	GetSashCombineResult(DWORD & dwItemVnum, DWORD & dwMinAbs, DWORD & dwMaxAbs);
		BYTE	CheckEmptyMaterialSlot();
		void	AddSashMaterial(TItemPos tPos, BYTE bPos);
		void	RemoveSashMaterial(BYTE bPos);
		BYTE	CanRefineSashMaterials();
		void	RefineSashMaterials();
#endif

#ifdef __CHANGELOOK_SYSTEM__
	protected:
		bool	m_bChangeLook;
	
	public:
		bool	isChangeLookOpened() {return m_bChangeLook;}
		void	ChangeLookWindow(bool bOpen = false, bool bRequest = false);
		void	ClearClWindowMaterials();
		LPITEM*	GetClWindowMaterials() {return m_pointsInstant.pClMaterials;}
		BYTE	CheckClEmptyMaterialSlot();
		void	AddClMaterial(TItemPos tPos, BYTE bPos);
		void	RemoveClMaterial(BYTE bPos);
		void	RefineClMaterials();
		bool	CleanTransmutation(LPITEM pkItem, LPITEM pkTarget);
		void 	ClearChangeLookWindow();
#endif

	// ITEM_STACK_ATTR_FLOOD
	protected:
		int m_dwUseItemStackAttrFlood;
	
	public:
		int GetUseItemStackAttrFlood() const { return m_dwUseItemStackAttrFlood; }
		void SetUseItemStackAttrFlood(int iPulseCore) { m_dwUseItemStackAttrFlood = iPulseCore; }
	// ITEM_STACK_ATTR_FLOOD

	public:
		struct S_CARD
		{
			DWORD	type;
			DWORD	value;
		};

		struct CARDS_INFO
		{
			S_CARD cards_in_hand[MAX_CARDS_IN_HAND];
			S_CARD cards_in_field[MAX_CARDS_IN_FIELD];
			DWORD	cards_left;
			DWORD	field_points;
			DWORD	points;
		};
		
		void			Cards_open(DWORD safemode);
		void			Cards_clean_list();
		DWORD			GetEmptySpaceInHand();
		void			Cards_pullout();
		void			RandomizeCards();
		bool			CardWasRandomized(DWORD type, DWORD value);
		void			SendUpdatedInformations();
		void			SendReward();
		void			CardsDestroy(DWORD reject_index);
		void			CardsAccept(DWORD accept_index);
		void			CardsRestore(DWORD restore_index);
		DWORD			GetEmptySpaceInField();
		DWORD			GetAllCardsCount();
		bool			TypesAreSame();
		bool			ValuesAreSame();
		bool			CardsMatch();
		DWORD			GetLowestCard();
		bool			CheckReward();
		void			CheckCards();
		void			RestoreField();
		void			ResetField();
		void			CardsEnd();
		void			GetGlobalRank(char * buffer, size_t buflen);
		void			GetRundRank(char * buffer, size_t buflen);
	protected:
		CARDS_INFO	character_cards;
		S_CARD	randomized_cards[24];
#ifdef __AURA_SYSTEM__
	private:
		BYTE		m_bAuraRefineWindowType;
		bool		m_bAuraRefineWindowOpen;
		TItemPos	m_pAuraRefineWindowItemSlot[AURA_SLOT_MAX];
		TAuraRefineInfo m_bAuraRefineInfo[AURA_REFINE_INFO_SLOT_MAX];

	protected:
		BYTE		__GetAuraAbsorptionRate(BYTE bLevel, BYTE bBoostIndex) const;
		TAuraRefineInfo __GetAuraRefineInfo(TItemPos Cell);
		TAuraRefineInfo __CalcAuraRefineInfo(TItemPos Cell, TItemPos MaterialCell);
		TAuraRefineInfo __GetAuraEvolvedRefineInfo(TItemPos Cell);

	public:
		void		OpenAuraRefineWindow(LPENTITY pOpener, EAuraWindowType type);
		bool		IsAuraRefineWindowOpen() const { return  m_bAuraRefineWindowOpen; }
		BYTE		GetAuraRefineWindowType() const { return  m_bAuraRefineWindowType; }
		LPENTITY	GetAuraRefineWindowOpener() { return  m_pointsInstant.m_pAuraRefineWindowOpener; }

		bool		IsAuraRefineWindowCanRefine();

		void		AuraRefineWindowCheckIn(BYTE bAuraRefineWindowType, TItemPos AuraCell, TItemPos ItemCell);
		void		AuraRefineWindowCheckOut(BYTE bAuraRefineWindowType, TItemPos AuraCell);
		void		AuraRefineWindowAccept(BYTE bAuraRefineWindowType);
		void		AuraRefineWindowClose();
#endif

#ifdef ENABLE_REFINE_ELEMENT
	public:
		BYTE			GetRefineElementType();
		
		void 			ClearRefineElement();
		bool 			DoRefineElement(BYTE bArg);
		void 			SendRefineElementPacket(WORD wSrcCell, WORD wDstCell, BYTE bType);
		bool 			RefineElementInformation(WORD wSrcCell, WORD wDstCell, BYTE bType);
	private:
		short			m_sRefineElementSrcCell;
		short			m_sRefineElementDstCell;
		char 			m_cRefineElementType;
#endif
#if defined(__ATTR_6TH_7TH__)
public:
	bool IsOpenAttr67Add() const { return m_bIsOpenAttr67Add ? true : false; }
	void SetOpenAttr67Add(bool bOpen) { m_bIsOpenAttr67Add = bOpen; }

	LPITEM GetAttr67AddItem(BYTE byCell = 0) const;
	bool Attr67Add(const TAttr67AddData kAttr67AddData);

private:
	bool m_bIsOpenAttr67Add;
#endif
public:
	bool UseItemTalismanAddAttribute(CItem& item, CItem& targetItem);
	bool UseItemTalismanChangeAttribute(CItem& item, CItem& targetItem);
	
#ifdef ENABLE_GLOVE_SYSTEM
	bool UseItemGloveAddAttribute(CItem& item, CItem& targetItem);
	bool UseItemGloveChangeAttribute(CItem& item, CItem& targetItem);
#endif
#if defined(__DAILY_GIFT_SYSTEM__)
	typedef std::vector<TPacketGCDailyGift> TDailyGift;
public:
	void LoadDailyGiftWeek();
	void CloseDailyGift();

	bool HasDailyGiftMission();
	BYTE GetDailyGiftStatus(BYTE bDay, DWORD dwCollectTime = 0);
	DWORD GetDailyGiftStartTime();
	DWORD GetDailyGiftRenewTime();
	BYTE GetDailyGiftWeek();
	BYTE CheckDailyGiftStatus();
	BYTE RenewDailyGift();
	BYTE DailyGiftOrderWeek();
	void CheckDailyGiftRenewTime();

	void CollectDailyGift(BYTE bSlotIndex, bool bUseTicket);

	bool SetCash(BYTE bType = ERequestCharge_Cash, DWORD dwAmount = 0);
	DWORD GetCash(BYTE bType = ERequestCharge_Cash);

private:
	TDailyGift m_vecDailyGift;
	DWORD m_dwDailyGiftRenewTime;
	bool m_bDailyGiftLoad;
	bool m_bDailyGiftOpen;
#endif



#ifdef __ENABLE_BIOLOGIST_RENEWAL_SYSTEM__
public:
	DWORD GetBiologistState() const { return m_points.biologist_state; }
	DWORD GetBiologistItemsTaken() const { return m_points.biologist_items_taken; }
	DWORD GetBiologistCompleted() const { return m_points.biologist_completed; }
	void SetBiologistState(DWORD dwValue) { m_points.biologist_state = dwValue; }
	void SetBiologistItemsTaken(DWORD dwValue) { m_points.biologist_items_taken = dwValue; }
	void SetBiologistCompleted(DWORD dwValue) { m_points.biologist_completed = dwValue; }
#endif

#ifdef __ENABLE_ADVANCE_SKILL_SELECT__
public:
	void AdvanceSkillSelect();
#endif


#ifdef __ENABLE_PREMIUM_PLAYERS__
public:
	void PremiumPlayersOpenPacket();
	void PremiumPlayersListPacket();
	void PremiumPlayersActivatePacket();
	
	void StartPremiumPlayersUpdateEvent();
	void StopPremiumPlayersUpdateEvent();
	
	bool IsPremiumPlayer() { return (GetPremiumPlayer() == 1 ? true : false); }
	
	BYTE GetPremiumPlayer() const { return m_byPremium; }
	void SetPremiumPlayer(BYTE byValue);
	
	int GetPremiumPlayerTimer() const { return m_iPremiumTime; }
	void SetPremiumPlayerTimer(int iTime);
	
	void CheckPremiumPlayersAffects();
	
protected:
	BYTE m_byPremium;
	long int m_iPremiumTime;
#endif

#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
public:
	BYTE GetSecuredState() const { return m_points.secured; }
	int GetSecuredPassword() const { return m_points.secured_password; }
	void SetSecuredState(BYTE byValue) { m_points.secured = byValue; }
	void SetSecuredPassword(int iValue) { m_points.secured_password = iValue; }
	
	bool IsSecured() { return GetSecuredState() > 0 ? true : false; }
	
	void ActivateProtectedSystem(const char* szPassword, bool bActivated);
	void ChangeProtectedSystemPassword(const char* szPasswordNow, const char* szPasswordNew);
#endif
#if defined(__CONQUEROR_LEVEL__)
	public:
		bool IsNewWorldMap(long lMapIndex);
		void SetSungMaWill();
		uint8_t GetSungMaWill(uint8_t type) const;
#endif


#ifdef __NEW_SET_BONUS__
public:
	bool			IsNewSetNeedRefresh(DWORD itemIdx);
#endif

#ifdef __DUNGEON_INFO__
public:
	void			SendDungeonCooldown(DWORD bossIdx);
	int				GetQuestFlagSpecial(const char* szQuestFlag, ...);
#endif


#ifdef ENABLE_BATTLE_PASS
public:
	void GetBattlePass(const TPlayerBattlePass* battle_pass);

	void BattlePassAction(uint8_t mission, uint8_t type, uint32_t points);

	void CheckBattlePassMissions(uint8_t timeInfo);
	const auto& GetPlayerBattlePass() { return battlePass; };

	void UpdateBattlePass(uint8_t subHeader = 0);
	void SetPremiumBattlePass();

	void SetBattlePassOpen(bool open) { battlePassOpen = open; }
	bool IsBattlePassOpen() { return battlePassOpen; }

	void ClearBattlePass();

private:
	std::map<uint8_t, std::vector<TPlayerBattlePass>> battlePass; //First if it's daily or weekly, second the mission data
	bool battlePassOpen = false;
#endif

#ifdef __PREMIUM_PRIVATE_SHOP__
	private:
		LPPRIVATE_SHOP		m_pPrivateShop;
		LPPRIVATE_SHOP		m_pMyPrivateShop;
		DWORD				m_dwPrivateShopOwner;
		TPrivateShop		m_privateShopTable;
		bool				m_bIsEditingPrivateShop;
		BYTE				m_bShopSearchMode;

		time_t				m_tLastPrivateShopModify;
		time_t				m_tLastPrivateShopWithdraw;
		time_t				m_tLastPrivateShopClose;
		time_t				m_tLastPrivateShopBuy;
		time_t				m_tLastPrivateShopSearch;
		time_t				m_tLastPrivateShopStateChange;
		time_t				m_tLastPrivateShopBuild;

		std::vector<TPlayerPrivateShopItem>		m_vec_privateShopItem;
		std::vector<TPrivateShopSale>			m_vec_privateShopSale;

	public:
		bool				BuildPrivateShop(const char* c_szTitle, DWORD dwPolyVnum, BYTE bTitleType, BYTE bPageCount, WORD wItemCount, TPrivateShopItem* pShopItemTable);
		void				ClosePrivateShop();

		void				SetViewingPrivateShop(LPPRIVATE_SHOP pShop) { m_pPrivateShop = pShop; }
		LPPRIVATE_SHOP		GetViewingPrivateShop() const { return m_pPrivateShop; }

		void				SetMyPrivateShop(LPPRIVATE_SHOP pShop) { m_pMyPrivateShop = pShop; }
		LPPRIVATE_SHOP		GetMyPrivateShop() const { return m_pMyPrivateShop; }

		void				SetPrivateShopOwner(DWORD dwPID) { m_dwPrivateShopOwner = dwPID; }
		DWORD				GetPrivateShopOwner() { return m_dwPrivateShopOwner; }

		void				SetPrivateShopTable(const TPrivateShop& rPrivateShopTable);
		TPrivateShop*		GetPrivateShopTable() { return &m_privateShopTable; }
		bool				IsPrivateShopOwner() { return m_privateShopTable.dwOwner != 0; }
		bool				CanModifyPrivateShop() { return m_privateShopTable.bState == STATE_MODIFY; }

		void				SetEditingPrivateShop(bool bEditingPrivateShop) { m_bIsEditingPrivateShop = bEditingPrivateShop; }
		bool				IsEditingPrivateShop() const { return m_bIsEditingPrivateShop; }
		void				OpenPrivateShopPanel();
		void				ClosePrivateShopPanel(bool bSendClient = false);

		void				OpenShopSearch(BYTE bMode);
		void				CloseShopSearch();
		bool				IsShopSearch() const { return m_bShopSearchMode != MODE_NONE; }
		BYTE				GetShopSearchMode() { return m_bShopSearchMode; }

		long long						GetPrivateShopTotalGold();
		DWORD							GetPrivateShopTotalCheque();

		void							SetPrivateShopItem(const TPlayerPrivateShopItem& c_rPrivateShopItem);
		bool							RemovePrivateShopItem(WORD wPos);
		const TPlayerPrivateShopItem*	GetPrivateShopItem(WORD wPos);
		WORD							GetPrivateShopItemCount() { return m_vec_privateShopItem.size(); }

		void							SetPrivateShopSale(const TPrivateShopSale& c_rPrivateShopSale);
		WORD							GetPrivateShopSaleCount() { return m_vec_privateShopSale.size(); }

		void							ChangePrivateShopItemPrice(WORD wPos, long long llGold, DWORD dwCheque);
		void							ChangePrivateShopItemPos(WORD wPos, WORD wChangePos);
		void							ChangePrivateShopTitle(const char* c_szTitle);
		void							SaleUpdate(const TPrivateShopSale* c_pShopSale);
		void							ItemExpireUpdate(WORD wPos);
		void							SetPrivateShopState(BYTE bState, bool bIsMainPlayerPrivateShop);
		void							WithdrawPrivateShop(long long llGold, DWORD dwCheque);
		void							WarpToPrivateShop(long lAddr, WORD wPort);

		bool							SetPremiumPrivateShopBonus(time_t tDuration);

		int								GetLastPrivateShopModifyTime() const { return m_tLastPrivateShopModify; }
		void							SetLastPrivateShopModifyTime() { m_tLastPrivateShopModify = thecore_pulse(); }

		int								GetLastPrivateShopWithdrawTime() const { return m_tLastPrivateShopWithdraw; }
		void							SetLastPrivateShopWithdrawTime() { m_tLastPrivateShopWithdraw = thecore_pulse(); }

		int								GetLastPrivateShopCloseTime() const { return m_tLastPrivateShopClose; }
		void							SetLastPrivateShopCloseTime() { m_tLastPrivateShopClose = thecore_pulse(); }

		int								GetLastPrivateShopBuildTime() const { return m_tLastPrivateShopBuild; }
		void							SetLastPrivateShopBuildTime() { m_tLastPrivateShopBuild = thecore_pulse(); }

		int								GetLastPrivateShopBuyTime() const { return m_tLastPrivateShopBuy; }
		void							SetLastPrivateShopBuyTime() { m_tLastPrivateShopBuy = thecore_pulse(); }

		int								GetLastPrivateShopSearchTime() const { return m_tLastPrivateShopSearch; }
		void							SetLastPrivateShopSearchTime() { m_tLastPrivateShopSearch = thecore_pulse(); }

		int								GetLastPrivateShopStateChangeTime() const { return m_tLastPrivateShopStateChange; }
		void							SetLastPrivateShopStateChangeTime() { m_tLastPrivateShopStateChange = thecore_pulse(); }
#endif


#if defined(__WORLD_BOSS_EVENT__)
public:
	void SetAccumulateDamageByVID(const DWORD dwVID, const DWORD dwDamage);
	DWORD GetAccumulateDamageByVID(const DWORD dwVID) const;
		void CompleteMission(uint16_t mission);
		using AccumulateDamageMap = std::unordered_map<DWORD, DWORD>;
private:
	AccumulateDamageMap m_dwAccumulateDamageMap;

public:
	DWORD GetWorldBossRequestPulse() const { return m_dwWorldBossRequestPulse; }
	void SetWorldBossRequestPulse(DWORD dwPulse) { m_dwWorldBossRequestPulse = dwPulse; }
private:
	DWORD m_dwWorldBossRequestPulse;
#endif

#ifdef ENABLE_REWARD_SYSTEM
public:
	DWORD			GetRewardData(const BYTE bType);
	void			SetRewardData(const BYTE bType, const DWORD value);

	void			LoadRewardData();
	void			SaveRewardData();
protected:
	std::map<BYTE, DWORD> m_mapRewardData;
	bool				m_bRewardLoaded;
#endif


#ifdef ENABLE_CUSTOM_INVENTORY
	protected:
		int		m_sortcustomInventoryPulse[CUSTOM_INVENTORY_CATEGORY_NUM];
		bool	m_bSortBlockActions;
		
	public:
		void			SetNextSortCustomInventoryPulse(int sortCustomInv, int pulse) { m_sortCustomInventoryPulse[sortCustomInv] = pulse; }
		int	 			GetSortCustomInventoryPulse(int sortCustomInv) { return m_sortCustomInventoryPulse[sortCustomInv]; }
	protected:
		int		m_sortCustomInventoryPulse[CUSTOM_INVENTORY_CATEGORY_NUM];		
#endif	

};


ESex GET_SEX(LPCHARACTER ch);

#endif

