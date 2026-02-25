#if !defined(_WORLD_BOSS_EVENT_H_) && defined(__WORLD_BOSS_EVENT__)
#define _WORLD_BOSS_EVENT_H_

#define __WORLD_BOSS_SEASON_RANK_BY_POINTS__
//#define __WORLD_BOSS_SEASON_RANK_BY_DAMAGE__

#if defined(__WORLD_BOSS_SEASON_RANK_BY_POINTS__)
#undef __WORLD_BOSS_SEASON_RANK_BY_DAMAGE__
#endif
#if defined(__WORLD_BOSS_SEASON_RANK_BY_DAMAGE__)
#undef __WORLD_BOSS_SEASON_RANK_BY_POINTS__
#endif

#include "../common/tables.h"
#include "packet.h"

class CGroupNode;
class CGroupTextParseTreeLoader;

class CWorldBoss : public singleton<CWorldBoss>
{
public:
	enum EState : BYTE
	{
		WORLD_BOSS_STATE_NONE, // None
		WORLD_BOSS_STATE_WAIT_FOR_SPAWN, // Hasn't appeared yet
		WORLD_BOSS_STATE_BOSS_SPAWNED, // Appeared
		WORLD_BOSS_STATE_BOSS_KILLED, // Escaped
		WORLD_BOSS_STATE_BREAK_TIME, // Cooldown
		WORLD_BOSS_STATE_END_CYCLE // End Cycle
	};

	enum EConfig : DWORD
	{
		WORLD_BOSS_COOLDOWN_TIME = 7200,
		WORLD_BOSS_RUN_TIME = 14400,
		WORLD_BOSS_MIN_DAMAGE = 250000,
		WORLD_BOSS_MIN_LEVEL = 30,
		WORLD_BOSS_CHANNEL = 1,
	};

public:
	CWorldBoss();
	~CWorldBoss();

	void Initialize();
	void Destroy();

public:
	bool ReadWorldBossTableFile(const char* c_pszFileName);
	bool ReadWorldBossConfig();
	bool ReadWorldBossMap();
	bool ReadWorldBossVNum();
	bool ReadWorldBossRanking();

	struct Config
	{
		DWORD dwCooldownTime;
		DWORD dwRunTime;
		DWORD dwMinDamage;
		DWORD dwMinLevel;
		Config() :
			dwCooldownTime(WORLD_BOSS_COOLDOWN_TIME),
			dwRunTime(WORLD_BOSS_RUN_TIME),
			dwMinDamage(WORLD_BOSS_MIN_DAMAGE),
			dwMinLevel(WORLD_BOSS_MIN_LEVEL)
		{}
	};

	DWORD GetCooldownTime() const { return m_sConfig.dwCooldownTime; }
	DWORD GetRunTime() const { return m_sConfig.dwRunTime; }
	DWORD GetMinDamage() const { return m_sConfig.dwMinDamage; }

	using MapDataVector = std::vector<long>;
	long GetRandomMapIndex();

	using BossDataVector = std::vector<DWORD>;
	DWORD GetRandomBoss();

	using RankingDataMap = std::map<DWORD, DWORD>;
	DWORD GetRankingPointsByLootLevel(const std::size_t c_nSize) const;
	DWORD GetRankingPointsByPosition(const std::size_t c_nSize) const;

public:
	void Enable(const bool c_bEnable);

	bool Spawn();
	void Escape();
	void Kill(const LPCHARACTER c_lpBoss);

	long GetMapIndex() const { return m_lMapIndex; }

	DWORD GetBossVNum() const { return m_dwBossVNum; }
	DWORD GetBossVID() const { return m_dwBossVID; }

	bool IsSpawned() const { return m_bBossSpawn; }

	BYTE GetState() const { return m_bState; }

	void SetRunTime(const std::time_t c_dwDuration);
	void SetCooldown(const std::time_t c_dwDuration);

	void Process(const LPCHARACTER c_lpChar, const BYTE c_bSubHeader);
	bool Reward(const LPCHARACTER c_lpChar);

public:

	using RankingDataPair = std::pair<DWORD, TPacketGDTempWorldBossRanking>;
	using RankingDataVector = std::vector<RankingDataPair>;

	void GetRanking(const LPCHARACTER c_lpChar);
	static bool CompareRankingData(const RankingDataPair& lhs, const RankingDataPair& rhs)
	{
		return lhs.second.dwRecord > rhs.second.dwRecord;
	}

	void UpdateSeasonRanking();

public:
	void TempWorldBossRanking(const LPDESC c_lpDesc, const TPacketGDTempWorldBossRanking* pTable, const WORD c_wSize);
		
private:
	long m_lMapIndex;

	DWORD m_dwBossVNum;
	DWORD m_dwBossVID;

	bool m_bBossSpawn;
	bool m_bBossAttacked;

	BYTE m_bState;

	LPEVENT m_pCooldownEvent;
	LPEVENT m_pRunEvent;
	LPEVENT m_pSpawnEvent;

	DWORD m_dwCooldownLeftTime;
	DWORD m_dwRunTimeLeft;

	RankingDataVector m_vRankingData;

private:
	CGroupTextParseTreeLoader* m_pLoader;
	Config m_sConfig;

	MapDataVector m_vMapIndexGroup;
	BossDataVector m_vBossDataGroup;
	RankingDataMap m_mapRankingDataGroup;
};

#endif // _WORLD_BOSS_EVENT_H_
