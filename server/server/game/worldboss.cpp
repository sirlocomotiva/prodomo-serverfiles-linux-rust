#include "stdafx.h"

#if defined(__WORLD_BOSS_EVENT__)
#include "worldboss.h"
#include "group_text_parse_tree.h"
#include "char_manager.h"
#include "char.h"
#include "desc.h"
#include "desc_manager.h"
#include "desc_client.h"
#include "guild.h"
#include "buffer_manager.h"
#include "item_manager.h"
#include "questmanager.h"
#include "config.h"

#include <random>

static std::map<long, std::string> s_map_MapName
{
	{ 61, LC_TEXT("Mount Sohan") },
    { 62, LC_TEXT("Doyyumhwan") },
    { 63, LC_TEXT("Yongbi Desert") },
    { 64, LC_TEXT("Valley of Seungryong") },
};

const char* GetMapNameByIndex(const long c_lMapIndex)
{
	auto it = s_map_MapName.find(c_lMapIndex);
	if (it != s_map_MapName.end())
		return it->second.c_str();
	return strdup("");
}

EVENTINFO(worldboss_event_info)
{
	CWorldBoss* pWorldBoss;
	DWORD dwDuration;
	int iPassedSec;
	worldboss_event_info() : pWorldBoss(nullptr), dwDuration(0), iPassedSec(0) {}
};

/*
* NOTE: Cooldown until the boss can spawn again
*/
EVENTFUNC(worldboss_cooldown_event)
{
	worldboss_event_info* info = dynamic_cast<worldboss_event_info*>(event->info);
	if (info == nullptr)
	{
		sys_err("worldboss_cooldown_event> Null event info.");
		return 0;
	}

	CWorldBoss* pWorldBoss = info->pWorldBoss;
	if (pWorldBoss == nullptr)
	{
		sys_err("worldboss_cooldown_event> Null event class.");
		return 0;
	}

	pWorldBoss->SetRunTime(pWorldBoss->GetRunTime());

	return 0;
}

/*
* NOTE: Combat time for handling the boss.
* This event function will wait for the boss to randomly spawn and will also
* trigger another event function when finished, which will be the cooldown.
*/
EVENTFUNC(worldboss_combat_event)
{
	worldboss_event_info* info = dynamic_cast<worldboss_event_info*>(event->info);
	if (info == nullptr)
	{
		sys_err("worldboss_combat_event> Null event info.");
		return 0;
	}

	CWorldBoss* pWorldBoss = info->pWorldBoss;
	if (pWorldBoss == nullptr)
	{
		sys_err("worldboss_combat_event> WorldBoss event info is null.");
		return 0;
	}

	int iPassedSec = info->iPassedSec;

	// Check if the passed seconds have reached the duration.
	if (iPassedSec > info->dwDuration)
	{
		pWorldBoss->SetCooldown(pWorldBoss->GetCooldownTime());
		return 0;
	}

	// Check if the passed seconds are below half the duration time.
	if (iPassedSec >= (info->dwDuration / 2) && pWorldBoss->IsSpawned())
	{
		pWorldBoss->Escape();
	}

	++info->iPassedSec;

	return PASSES_PER_SEC(1);
}

/*
* NOTE: Spawn time for the boss.
* This event function will only spawn the boss.
*/
EVENTFUNC(worldboss_spawn_event)
{
	worldboss_event_info* info = dynamic_cast<worldboss_event_info*>(event->info);
	if (info == nullptr)
	{
		sys_err("worldboss_cooldown_event> Null event info.");
		return 0;
	}

	CWorldBoss* pWorldBoss = info->pWorldBoss;
	if (pWorldBoss == nullptr)
	{
		sys_err("worldboss_cooldown_event> Null event class.");
		return 0;
	}

	pWorldBoss->Spawn();
	return 0;
}

CWorldBoss::CWorldBoss()
{
	m_pLoader = nullptr;

	std::memset(&m_sConfig, 0, sizeof(m_sConfig));

	m_vMapIndexGroup.clear();
	m_vBossDataGroup.clear();
	m_mapRankingDataGroup.clear();

	Initialize();
}

CWorldBoss::~CWorldBoss()
{
	Destroy();
}

void CWorldBoss::Initialize()
{
	m_lMapIndex = 0;

	m_dwBossVNum = 0;
	m_dwBossVID = 0;

	m_bBossSpawn = false;
	m_bBossAttacked = false;

	m_bState = EState::WORLD_BOSS_STATE_NONE;

	if (m_pCooldownEvent)
		event_cancel(&m_pCooldownEvent);

	if (m_pRunEvent)
		event_cancel(&m_pRunEvent);

	if (m_pSpawnEvent)
		event_cancel(&m_pSpawnEvent);

	m_dwCooldownLeftTime = 0;
	m_dwRunTimeLeft = 0;

	m_vRankingData.clear();
}

void CWorldBoss::Destroy()
{
	if (m_pLoader)
	{
		delete m_pLoader;
		m_pLoader = nullptr;
	}

	m_vMapIndexGroup.clear();
	m_vBossDataGroup.clear();
	m_mapRankingDataGroup.clear();

	Initialize();
}

bool CWorldBoss::ReadWorldBossTableFile(const char* c_pszFileName)
{
	if (m_pLoader)
	{
		delete m_pLoader;
		m_pLoader = nullptr;
	}

	m_pLoader = new CGroupTextParseTreeLoader;
	CGroupTextParseTreeLoader& rkLoader = *m_pLoader;

	if (rkLoader.Load(c_pszFileName) == false)
		return false;

	if (!ReadWorldBossConfig())
		return false;

	if (!ReadWorldBossMap())
		return false;

	if (!ReadWorldBossVNum())
		return false;

	if (!ReadWorldBossRanking())
		return false;

	return true;
}

bool CWorldBoss::ReadWorldBossConfig()
{
	CGroupNode* pGroupNode = m_pLoader->GetGroup("config");
	if (pGroupNode == nullptr)
	{
		sys_err(0, "Group Config not found.");
		return false;
	}

	int iSize = pGroupNode->GetRowCount();
	if (iSize == 0)
	{
		sys_err(0, "Group Config is Empty.");
		return false;
	}

	std::memset(&m_sConfig, 0, sizeof(m_sConfig));

	for (int iLine = 0; iLine < iSize; iLine++)
	{
		const CGroupNode::CGroupNodeRow* c_pRow;
		pGroupNode->GetRow(iLine, &c_pRow);

		DWORD dwCooldownTime;
		if (!c_pRow->GetValue("cooldown", dwCooldownTime))
		{
			sys_err(0, "In Group Config, Column Cooldown not found.");
			return false;
		}
		m_sConfig.dwCooldownTime = dwCooldownTime;

		DWORD dwRunTime;
		if (!c_pRow->GetValue("runtime", dwRunTime))
		{
			sys_err(0, "In Group Config, Column RunTime not found.");
			return false;
		}
		m_sConfig.dwRunTime = dwRunTime;

		DWORD dwMinDamage;
		if (!c_pRow->GetValue("mindamage", dwMinDamage))
		{
			sys_err(0, "In Group Config, Column MinDamage not found.");
			return false;
		}
		m_sConfig.dwMinDamage = dwMinDamage;

		DWORD dwMinLevel;
		if (!c_pRow->GetValue("minlevel", dwMinLevel))
		{
			sys_err(0, "In Group Config, Column MinLevel not found.");
			return false;
		}
		m_sConfig.dwMinLevel = dwMinLevel;
	}

	return true;
}

bool CWorldBoss::ReadWorldBossMap()
{
	CGroupNode* pGroupNode = m_pLoader->GetGroup("map");
	if (pGroupNode == nullptr)
	{
		sys_err(0, "Group Map not found.");
		return false;
	}

	int iSize = pGroupNode->GetRowCount();
	if (iSize == 0)
	{
		sys_err(0, "Group Map is Empty.");
		return false;
	}

	m_vMapIndexGroup.clear();

	for (int iLine = 0; iLine < iSize; iLine++)
	{
		const CGroupNode::CGroupNodeRow* c_pRow;
		pGroupNode->GetRow(iLine, &c_pRow);

		long lMapIndex;
		if (!c_pRow->GetValue("index", lMapIndex))
		{
			sys_err(0, "In Group Map, Column Index not found.");
			return false;
		}

		m_vMapIndexGroup.emplace_back(lMapIndex);
	}

	return true;
}

bool CWorldBoss::ReadWorldBossVNum()
{
	CGroupNode* pGroupNode = m_pLoader->GetGroup("boss");
	if (pGroupNode == nullptr)
	{
		sys_err(0, "Group Boss not found.");
		return false;
	}

	int iSize = pGroupNode->GetRowCount();
	if (iSize == 0)
	{
		sys_err(0, "Group Boss is Empty.");
		return false;
	}

	m_vBossDataGroup.clear();

	for (int iLine = 0; iLine < iSize; iLine++)
	{
		const CGroupNode::CGroupNodeRow* c_pRow;
		pGroupNode->GetRow(iLine, &c_pRow);

		DWORD dwVNum;
		if (!c_pRow->GetValue("vnum", dwVNum))
		{
			sys_err(0, "In Group Boss, Column VNum not found.");
			return false;
		}

		m_vBossDataGroup.emplace_back(dwVNum);
	}

	return true;
}

bool CWorldBoss::ReadWorldBossRanking()
{
	CGroupNode* pGroupNode = m_pLoader->GetGroup("ranking");
	if (pGroupNode == nullptr)
	{
		sys_err(0, "Group Ranking not found.");
		return false;
	}

	int iSize = pGroupNode->GetRowCount();
	if (iSize == 0)
	{
		sys_err(0, "Group Ranking is Empty.");
		return false;
	}

	m_mapRankingDataGroup.clear();

	for (int iLine = 0; iLine < iSize; iLine++)
	{
		const CGroupNode::CGroupNodeRow* c_pRow;
		pGroupNode->GetRow(iLine, &c_pRow);

		DWORD dwPoints;
		if (!c_pRow->GetValue("points", dwPoints))
		{
			sys_err(0, "In Ranking Boss, Column Points not found.");
			return false;
		}

		m_mapRankingDataGroup.insert(std::make_pair(iLine, dwPoints));
	}

	return true;
}

long CWorldBoss::GetRandomMapIndex()
{
	if (m_vMapIndexGroup.empty())
	{
		throw std::runtime_error("No map index available");
	}

	static std::mt19937 rng(std::random_device{}());
	std::uniform_int_distribution<std::size_t> dist(0, m_vMapIndexGroup.size() - 1);

	return m_vMapIndexGroup[dist(rng)];
}

DWORD CWorldBoss::GetRandomBoss()
{
	if (m_vBossDataGroup.empty())
	{
		throw std::runtime_error("No random boss data available");
	}

	static std::mt19937 rng(std::random_device{}());
	std::uniform_int_distribution<std::size_t> dist(0, m_vBossDataGroup.size() - 1);

	return m_vBossDataGroup[dist(rng)];
}

void CWorldBoss::Enable(const bool c_bEnable)
{
	sys_err("CWorldBoss::Enable(): Enabling the event.");
	Initialize();

	if (c_bEnable)
	{
		sys_err("CWorldBoss::Enable(): Event is enabled.");
		SetCooldown(m_sConfig.dwCooldownTime);
		SendNotice(LC_TEXT("[World boss] Event started"));
	}
	else
	{
		SendNotice(LC_TEXT("[World boss] Event completed"));
	}

	std::function<void(const LPDESC)> BroadcastEvent = [&](const LPDESC c_lpDesc)
	{
		const LPCHARACTER c_lpChar = c_lpDesc->GetCharacter();
		if (c_lpChar != nullptr)
		{
			unsigned int iQuestIndex = quest::CQuestManager::instance().GetQuestIndexByName("daily_world_boss");
			if (iQuestIndex)
				quest::CQuestManager::instance().Letter(c_lpChar->GetPlayerID(), iQuestIndex, 0);

			c_lpDesc->ChatPacket(CHAT_TYPE_COMMAND, "worldboss %d", c_bEnable);
		}
	};

	const DESC_MANAGER::DESC_SET& rClientSet = DESC_MANAGER::instance().GetClientSet();
	std::for_each(rClientSet.begin(), rClientSet.end(), BroadcastEvent);
}

bool CWorldBoss::Spawn()
{
	sys_err("CWorldBoss::Spawn(): Spawning the boss.");
	// NOTE: Only spawn the boss on channel 1.
	if (g_bChannel != WORLD_BOSS_CHANNEL)
		return true;

	sys_err("CWorldBoss::Spawn(): Channel is %d", g_bChannel);

	if (m_bBossSpawn == true)
		return false;

	sys_err("CWorldBoss::Spawn(): Boss is spawned.");

	const long c_lMapIndex = GetRandomMapIndex();
	const DWORD c_dwBossVNum = GetRandomBoss();

	// Create the random selected boss.
	const LPCHARACTER c_pBoss = CHARACTER_MANAGER::instance().SpawnMobRandomPosition(c_dwBossVNum, c_lMapIndex);
	if (c_pBoss == nullptr)
	{
		sys_err("CWorldBoss::Spawn(): Failed to spawn character VNum %d", c_dwBossVNum);
		return false;
	}

	m_lMapIndex = c_lMapIndex;

	m_dwBossVNum = c_dwBossVNum;
	m_dwBossVID = c_pBoss->GetVID();

	m_bBossSpawn = true;
	m_bBossAttacked = false;

	m_bState = EState::WORLD_BOSS_STATE_BOSS_SPAWNED;

	// Broadcast spawn of the boss.
	char szNoticeBuf[1024];
	snprintf(szNoticeBuf, sizeof(szNoticeBuf),
		LC_TEXT("[World boss] In Channel %d, %s has appeared and is terrorising the area of %s. Let's go hunting!"),
		g_bChannel, c_pBoss->GetName(), GetMapNameByIndex(c_lMapIndex));
	BroadcastNotice(szNoticeBuf);

	if (test_server)
	{
		char szTestNoticeBuf[1024]{};
		snprintf(szTestNoticeBuf, sizeof(szTestNoticeBuf), "[World Boss Test] %s spawn at %d, %d",
			c_pBoss->GetName(), c_pBoss->GetX(), c_pBoss->GetY());

		BroadcastNotice(szTestNoticeBuf);
	}

	// Delete Ranking Results
	db_clientdesc->DBPacket(HEADER_GD_CLR_TEMP_WORLD_BOSS_RK, 0, nullptr, 0);
	m_vRankingData.clear();

	// Save the current state in an event flag.
	quest::CQuestManager::instance().RequestSetEventFlag("world_boss_state", m_bState);

	return true;
}

void CWorldBoss::Escape()
{
	// NOTE: Only spawn the boss on channel 1.
	if (g_bChannel != WORLD_BOSS_CHANNEL)
		return;

	// If the boss isn't spawned, he won't need to escape.
	if (m_bBossSpawn == false)
		return;

	const DWORD c_dwBossVID = GetBossVID();

	LPCHARACTER pBoss = CHARACTER_MANAGER::instance().Find(c_dwBossVID);
	if (pBoss != nullptr)
	{
		CHARACTER_MANAGER::instance().DestroyCharacter(pBoss);
		m_bBossSpawn = false;
	}

	m_bState = EState::WORLD_BOSS_STATE_BOSS_KILLED;

	BroadcastNotice(LC_TEXT("[World boss] The World boss has escaped! Good luck next time."));

	// Save the current state in an event flag.
	quest::CQuestManager::instance().RequestSetEventFlag("world_boss_state", m_bState);
}

void CWorldBoss::Kill(const LPCHARACTER c_lpBoss)
{
	if (c_lpBoss == nullptr)
		return;

	m_bState = EState::WORLD_BOSS_STATE_BOSS_KILLED;

	m_bBossSpawn = false;

	char szNoticeBuf[1024];
	snprintf(szNoticeBuf, sizeof(szNoticeBuf), LC_TEXT("[World boss] Well done: %s has been defeated and peace has been restored... for now."), c_lpBoss->GetName());
	BroadcastNotice(szNoticeBuf);

	// Update
	{
		std::function<void(const LPDESC)> UpdateRanking = [&](const LPDESC c_lpDesc)
		{
			const LPCHARACTER c_lpChar = c_lpDesc->GetCharacter();
			if (c_lpChar != nullptr)
			{
				c_lpChar->SetQuestFlag("world_boss_reward", 0);

				if (c_lpChar->GetMapIndex() == m_lMapIndex)
				{
					const DWORD c_dwTotalDamage = c_lpChar->GetAccumulateDamageByVID(m_dwBossVID);
					if (c_lpChar->GetLevel() >= m_sConfig.dwMinLevel && c_dwTotalDamage > m_sConfig.dwMinDamage)
					{
						TPacketGDTempWorldBossRanking Table = {};
						strlcpy(Table.szPlayerName, c_lpChar->GetName(), sizeof(Table.szPlayerName));
						strlcpy(Table.szGuildName, c_lpChar->GetGuild() ? c_lpChar->GetGuild()->GetName() : "", sizeof(Table.szGuildName));
						Table.bEmpire = c_lpChar->GetEmpire();
						Table.dwRecord = c_dwTotalDamage;
						m_vRankingData.emplace_back(std::make_pair(c_lpChar->GetPlayerID(), Table));
						db_clientdesc->DBPacket(HEADER_GD_ADD_TEMP_WORLD_BOSS_RK, c_lpChar->GetPlayerID(), &Table, sizeof(Table));
					}
				}
			}
		};

		const DESC_MANAGER::DESC_SET& c_rDescSet = DESC_MANAGER::instance().GetClientSet();
		std::for_each(c_rDescSet.begin(), c_rDescSet.end(), UpdateRanking);
	}

	UpdateSeasonRanking();

	// Save the current state in an event flag.
	quest::CQuestManager::instance().RequestSetEventFlag("world_boss_state", m_bState);
}

void CWorldBoss::SetRunTime(const std::time_t c_dwDuration)
{
	worldboss_event_info* info = AllocEventInfo<worldboss_event_info>();
	info->pWorldBoss = this;
	info->dwDuration = c_dwDuration;
	m_pRunEvent = event_create(worldboss_combat_event, info, PASSES_PER_SEC(1));

	sys_err("CWorldBoss::SetRunTime(): Setting the run time, channel is %d", g_bChannel);

	// NOTE: Only spawn the boss on channel 1.
	if (g_bChannel == WORLD_BOSS_CHANNEL)
	{
		static std::mt19937 rng(std::random_device{/*5*/}());
		std::uniform_int_distribution<std::size_t> dist(0, c_dwDuration / 3);
		std::size_t random_time = 5;

		worldboss_event_info* info = AllocEventInfo<worldboss_event_info>();
		info->pWorldBoss = this;
		m_pSpawnEvent = event_create(worldboss_spawn_event, info, PASSES_PER_SEC(random_time));

		if (test_server)
		{
			char szNoticeBuf[1024];
			snprintf(szNoticeBuf, sizeof(szNoticeBuf), "[Test World boss] The boss will appear in %d secconds.", random_time);
			SendNotice(szNoticeBuf);
		}
	}

	SendNotice(LC_TEXT("[World boss] Caution: the world boss will appear soon!"));

	m_dwCooldownLeftTime = 0;
	m_dwRunTimeLeft = time(nullptr) + c_dwDuration;

	m_bState = EState::WORLD_BOSS_STATE_WAIT_FOR_SPAWN;

	// Save the current state in an event flag.
	quest::CQuestManager::instance().RequestSetEventFlag("world_boss_state", m_bState);
}

void CWorldBoss::SetCooldown(const std::time_t c_dwDuration)
{
	worldboss_event_info* info = AllocEventInfo<worldboss_event_info>();
	info->pWorldBoss = this;
	info->dwDuration = c_dwDuration;
	m_pCooldownEvent = event_create(worldboss_cooldown_event, info, PASSES_PER_SEC(c_dwDuration));

	m_dwCooldownLeftTime = time(nullptr) + c_dwDuration;
	m_dwRunTimeLeft = 0;

	m_bState = EState::WORLD_BOSS_STATE_BREAK_TIME;

	// Save the current state in an event flag.
	quest::CQuestManager::instance().RequestSetEventFlag("world_boss_state", m_bState);
}

void CWorldBoss::Process(const LPCHARACTER c_lpChar, const BYTE c_bSubHeader)
{
	if (c_lpChar == nullptr)
		return;

	const LPDESC c_lpDesc = c_lpChar->GetDesc();
	if (c_lpDesc == nullptr)
		return;

	TEMP_BUFFER TempBuf;
	switch (c_bSubHeader)
	{
		case WORLD_BOSS_SUBHEADER_GC_INFO:
		{
			TPacketGCWorldBossProcess Table;

			if (g_bChannel == WORLD_BOSS_CHANNEL)
				Table.bState = m_bState;
			else
				Table.bState = quest::CQuestManager::instance().GetEventFlag("world_boss_state");

			Table.dwRunTimeLeft = m_dwRunTimeLeft;
			Table.dwCooldownTimeLeft = m_dwCooldownLeftTime;
			Table.dwTotalDamage = c_lpChar->GetAccumulateDamageByVID(m_dwBossVID);
			Table.dwMinDamage = m_sConfig.dwMinDamage;
			TempBuf.write(&Table, sizeof(Table));
		}
		break;

		case WORLD_BOSS_SUBHEADER_GC_DAMAGE:
		{
			if (m_bBossAttacked == false)
			{
				char szNoticeBuf[1024];
				snprintf(szNoticeBuf, sizeof(szNoticeBuf), LC_TEXT("[World boss] The world boss in Channel %d has been attacked. Join the hunt now!"), g_bChannel);
				BroadcastNotice(szNoticeBuf);
				m_bBossAttacked = true;
			}

			DWORD dwDamage = c_lpChar->GetAccumulateDamageByVID(m_dwBossVID);
			TempBuf.write(&dwDamage, sizeof(DWORD));
		}
		break;
	}

	TPacketGCWorldBoss Packet;
	Packet.bHeader = HEADER_GC_WORLD_BOSS;
	Packet.wSize = sizeof(Packet) + TempBuf.size();
	Packet.bSubHeader = c_bSubHeader;
	if (TempBuf.size())
	{
		c_lpDesc->BufferedPacket(&Packet, sizeof(Packet));
		c_lpDesc->Packet(TempBuf.read_peek(), TempBuf.size());
	}
	else
		c_lpDesc->Packet(&Packet, sizeof(Packet));
}

bool CWorldBoss::Reward(const LPCHARACTER c_lpChar)
{
	if (c_lpChar == nullptr)
		return false;

	if (m_bState == EState::WORLD_BOSS_STATE_BOSS_SPAWNED)
	{
		c_lpChar->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[World boss] You can't receive a reward now."));
		return false;
	}

	if (c_lpChar->GetWorldBossRequestPulse() > thecore_pulse() && !c_lpChar->IsGM())
		return false;

	c_lpChar->SetWorldBossRequestPulse(thecore_pulse() + PASSES_PER_SEC(3));

	if (m_bState != EState::WORLD_BOSS_STATE_BOSS_KILLED)
		return false;

	if (c_lpChar->GetQuestFlag("world_boss_reward") > 0)
		return false;

	if (c_lpChar->GetLevel() < m_sConfig.dwMinLevel)
		return false;

	if (c_lpChar->GetAccumulateDamageByVID(m_dwBossVID) < m_sConfig.dwMinDamage)
		return false;

	// Sort the vector by dwRecord0 in descending order.
	RankingDataVector vSortedRankingData(m_vRankingData.begin(), m_vRankingData.end());
	std::sort(vSortedRankingData.begin(), vSortedRankingData.end(), CompareRankingData);

	RankingDataVector::iterator it = std::find_if(vSortedRankingData.begin(), vSortedRankingData.end(),
		[&](const auto& pair)
		{
			return pair.first == c_lpChar->GetPlayerID();
		}
	);

	if (it == vSortedRankingData.end())
		return false;

	std::size_t nPosition = std::distance(vSortedRankingData.begin(), it) + 1;

	char szRewardBuf[1024];
	DWORD dwGroupNum = 0;
	long lAffectValue = 0;

	if (nPosition >= 1 && nPosition <= 10)
	{
		snprintf(szRewardBuf, sizeof(szRewardBuf), LC_TEXT("[World boss] Reward for Loot Level I in the rankings: %d"), nPosition);
		dwGroupNum = 11001;
		lAffectValue = 100;
	}
	else if (nPosition >= 11 && nPosition <= 25)
	{
		snprintf(szRewardBuf, sizeof(szRewardBuf), LC_TEXT("[World boss] Reward for Loot Level II in the rankings: %d"), nPosition);
		dwGroupNum = 11002;
		lAffectValue = 50;
	}
	else if (nPosition >= 26 && nPosition <= 100)
	{
		snprintf(szRewardBuf, sizeof(szRewardBuf), LC_TEXT("[World boss] Reward for Loot Level III in the rankings: %d"), nPosition);
		dwGroupNum = 11003;
		lAffectValue = 25;
	}
	else if (nPosition > 100)
	{
		snprintf(szRewardBuf, sizeof(szRewardBuf), LC_TEXT("[World boss] Reward for Loot Level IV in the rankings: %d"), nPosition);
		dwGroupNum = 11004;
	}

	if (dwGroupNum == 0)
		return false;

	const CSpecialItemGroup* c_pGroup = ITEM_MANAGER::instance().GetSpecialItemGroup(dwGroupNum);
	if (c_pGroup == nullptr)
	{
		sys_err("CWorldBoss::Reward(c_lpChar=%p): Cannot find special item group %d", c_lpChar, dwGroupNum);
		return false;
	}

	BYTE bNeedSizeCount = 0;
	std::vector<int/*5*/> vIndexs;
	int nSize = c_pGroup->GetMultiIndex(vIndexs);
	for (int iIndex = 0; iIndex < nSize; iIndex++)
	{
		DWORD dwVnum = c_pGroup->GetVnum(vIndexs[iIndex]);

		TItemTable* pProto = ITEM_MANAGER::instance().GetTable(dwVnum);
		if (pProto == nullptr)
			continue;

		bNeedSizeCount += pProto->bSize;
	}

	if (c_lpChar->CountEmptyInventory() > bNeedSizeCount)
	{
		c_lpChar->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[World boss] Well done! Here is your reward. Join in next time as well!"));
		c_lpChar->ChatPacket(CHAT_TYPE_INFO, szRewardBuf);
		std::vector<DWORD> dwVnums;
		std::vector<DWORD> dwCounts;
		std::vector<LPITEM> item_gets(0);
		int count = 0;

		if (!c_lpChar->GiveItemFromSpecialItemGroup(dwGroupNum, dwVnums, dwCounts, item_gets, count))
		{
			sys_log(0, "CWorldBoss::Reward(c_lpChar=%p): Failed to give item from special item group %d", c_lpChar, dwGroupNum);
			return false;
		}

		if (lAffectValue > 0)
			c_lpChar->AddAffect(AFFECT_WORLD_BOSS_REWARD, POINT_MALL_ITEMBONUS, lAffectValue, AFF_NONE, 7200, 0, true);

		c_lpChar->SetQuestFlag("world_boss_reward", 1);
		return true;
	}
	else
	{
		c_lpChar->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[World boss] You can't receive a reward now, because you do not have enough space in your inventory."));
		return true;
	}
}

void CWorldBoss::GetRanking(const LPCHARACTER c_lpChar)
{
	if (c_lpChar == nullptr)
		return;

	if (c_lpChar->GetWorldBossRequestPulse() > thecore_pulse() && !c_lpChar->IsGM())
		return;

	c_lpChar->SetWorldBossRequestPulse(thecore_pulse() + PASSES_PER_SEC(3));

	const LPDESC c_lpDesc = c_lpChar->GetDesc();
	if (c_lpDesc == nullptr)
		return;

	db_clientdesc->DBPacket(HEADER_GD_GET_TEMP_WORLD_BOSS_RK, c_lpDesc->GetHandle(), nullptr, 0);
}

DWORD CWorldBoss::GetRankingPointsByPosition(const std::size_t c_nSize) const
{
	RankingDataMap::const_iterator it = m_mapRankingDataGroup.find(c_nSize);
	return (it != m_mapRankingDataGroup.end() ? it->second : 0);
}

DWORD CWorldBoss::GetRankingPointsByLootLevel(const std::size_t c_nSize) const
{
	if (c_nSize >= 1 && c_nSize <= 10)
		return 10;
	else if (c_nSize > 10 && c_nSize <= 25)
		return 5;
	else if (c_nSize > 25 && c_nSize <= 100)
		return 2;
	else if (c_nSize > 100)
		return 1;

	return 0;
}

void CWorldBoss::UpdateSeasonRanking()
{
	if (m_vRankingData.empty())
		return;

	RankingDataVector vSortedRankingData(m_vRankingData.begin(), m_vRankingData.end());
	std::sort(vSortedRankingData.begin(), vSortedRankingData.end(), CompareRankingData);

	std::size_t nPos = 0;
	for (const RankingDataPair& it : vSortedRankingData)
	{
		const LPCHARACTER c_lpChar = CHARACTER_MANAGER::instance().FindByPID(it.first);
		if (c_lpChar == nullptr)
			continue;

		TPacketGDWorldBossRanking Table = {};
		Table.dwPID = c_lpChar->GetPlayerID();
		Table.dwGuildID = c_lpChar->GetGuild() ? c_lpChar->GetGuild()->GetID() : 0;
		Table.bEmpire = it.second.bEmpire;
		Table.dwRecord = GetRankingPointsByPosition(nPos);
		Table.dwStartTime = time(nullptr);

		db_clientdesc->DBPacket(HEADER_GD_WORLD_BOSS_RANKING, c_lpChar->GetPlayerID(), &Table, sizeof(Table));

		++nPos;
	}
}

void CWorldBoss::TempWorldBossRanking(const LPDESC c_lpDesc, const TPacketGDTempWorldBossRanking* pTable, const WORD c_wSize)
{
	if (c_lpDesc == nullptr)
		return;

	TEMP_BUFFER TempBuffer;
	for (WORD wSize = 0; wSize < c_wSize; ++wSize, ++pTable)
		TempBuffer.write(pTable, sizeof(TPacketGDTempWorldBossRanking));

	TPacketGCWorldBoss Packet;
	Packet.bHeader = HEADER_GC_WORLD_BOSS;
	Packet.wSize = sizeof(Packet) + TempBuffer.size();
	Packet.bSubHeader = WORLD_BOSS_SUBHEADER_GC_RANKING;

	if (TempBuffer.size())
	{
		c_lpDesc->BufferedPacket(&Packet, sizeof(Packet));
		c_lpDesc->Packet(TempBuffer.read_peek(), TempBuffer.size());
	}
	else
		c_lpDesc->Packet(&Packet, sizeof(Packet));
}
#endif // __WORLD_BOSS_EVENT__
