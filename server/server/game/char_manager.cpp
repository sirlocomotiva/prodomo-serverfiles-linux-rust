#include "stdafx.h"
#include "constants.h"
#include "utils.h"
#include "desc.h"
#include "char.h"
#include "char_manager.h"
#include "mob_manager.h"
#include "party.h"
#include "regen.h"
#include "p2p.h"
#include "dungeon.h"
#include "db.h"
#include "config.h"
#include "xmas_event.h"
#include "questmanager.h"
#include "questlua.h"
#include "locale_service.h"
#include "shutdown_manager.h"
#include "guild.h"
#include "desc_client.h"
#include "desc_manager.h"


#if defined(__SHIP_DEFENSE__)
	#include "ShipDefense.h"
#endif

#include "../common/prodomodefines.h"

#if (!defined(__GNUC__) || defined(__clang__)) && !defined(CXX11_ENABLED)
#	include <boost/bind.hpp>

#elif defined(CXX11_ENABLED)
#	include <functional>
	template <typename T>
	decltype(std::bind(&T::second, std::placeholders::_1)) select2nd()
	{
		return std::bind(&T::second, std::placeholders::_1);
	}
#endif

CHARACTER_MANAGER::CHARACTER_MANAGER() :
	m_iVIDCount(0),
	m_pkChrSelectedStone(NULL),
	m_bUsePendingDestroy(false)
{
	RegisterRaceNum(xmas::MOB_XMAS_FIRWORK_SELLER_VNUM);
	RegisterRaceNum(xmas::MOB_SANTA_VNUM);
	RegisterRaceNum(xmas::MOB_XMAS_TREE_VNUM);

	m_iMobItemRate = 100;
	m_iMobDamageRate = 100;
	m_iMobGoldAmountRate = 100;
	m_iMobGoldDropRate = 100;
	m_iMobExpRate = 100;

	m_iMobItemRatePremium = 100;
	m_iMobGoldAmountRatePremium = 100;
	m_iMobGoldDropRatePremium = 100;
	m_iMobExpRatePremium = 100;

	m_iUserDamageRate = 100;
	m_iUserDamageRatePremium = 100;
}

CHARACTER_MANAGER::~CHARACTER_MANAGER()
{
	Destroy();
}

void CHARACTER_MANAGER::Destroy()
{
	itertype(m_map_pkChrByVID) it = m_map_pkChrByVID.begin();
	while (it != m_map_pkChrByVID.end()) {
		LPCHARACTER ch = it->second;
		M2_DESTROY_CHARACTER(ch); // m_map_pkChrByVID is changed here
		it = m_map_pkChrByVID.begin();
	}
}



void CHARACTER_MANAGER::GracefulShutdown()
{
	NAME_MAP::iterator it = m_map_pkPCChr.begin();

	while (it != m_map_pkPCChr.end())
		(it++)->second->Disconnect("GracefulShutdown");
}

DWORD CHARACTER_MANAGER::AllocVID()
{
	++m_iVIDCount;
	return m_iVIDCount;
}

LPCHARACTER CHARACTER_MANAGER::CreateCharacter(const char * name, DWORD dwPID)
{
	DWORD dwVID = AllocVID();

#ifdef M2_USE_POOL
	LPCHARACTER ch = pool_.Construct();
#else
	LPCHARACTER ch = M2_NEW CHARACTER;
#endif
	ch->Create(name, dwVID, dwPID ? true : false);

	m_map_pkChrByVID.insert(std::make_pair(dwVID, ch));

	if (dwPID)
	{
		char szName[CHARACTER_NAME_MAX_LEN + 1];
		str_lower(name, szName, sizeof(szName));

		m_map_pkPCChr.insert(NAME_MAP::value_type(szName, ch));
		m_map_pkChrByPID.insert(std::make_pair(dwPID, ch));
	}

	return (ch);
}

#ifndef DEBUG_ALLOC
void CHARACTER_MANAGER::DestroyCharacter(LPCHARACTER ch)
#else
void CHARACTER_MANAGER::DestroyCharacter(LPCHARACTER ch, const char* file, size_t line)
#endif
{
	if (!ch)
		return;

	// <Factor> Check whether it has been already deleted or not.
	itertype(m_map_pkChrByVID) it = m_map_pkChrByVID.find(ch->GetVID());
	if (it == m_map_pkChrByVID.end()) {
		sys_err("[CHARACTER_MANAGER::DestroyCharacter] <Factor> %d not found", (long)(ch->GetVID()));
		return; // prevent duplicated destrunction
	}

	// ������ �Ҽӵ� ���ʹ� ���������� �����ϵ���.
	if (ch->IsNPC() && !ch->IsPet() && ch->GetRider() == NULL
#ifdef __ENABLE_SHAMAN_SYSTEM__
		&& !ch->IsAutoShaman())
#endif
	{
		if (ch->GetDungeon())
		{
			ch->GetDungeon()->DeadCharacter(ch);
		}
	}

#if defined(__SHIP_DEFENSE__)
	// Delete monsters from the ship defense.
	if (ch->IsPC() == false)
		CShipDefenseManager::Instance().OnKill(ch);
#endif


	if (m_bUsePendingDestroy)
	{
		m_set_pkChrPendingDestroy.insert(ch);
		return;
	}

	m_map_pkChrByVID.erase(it);

	if (true == ch->IsPC())
	{
		char szName[CHARACTER_NAME_MAX_LEN + 1];

		str_lower(ch->GetName(), szName, sizeof(szName));

		NAME_MAP::iterator it = m_map_pkPCChr.find(szName);

		if (m_map_pkPCChr.end() != it)
			m_map_pkPCChr.erase(it);
	}

	if (0 != ch->GetPlayerID())
	{
		itertype(m_map_pkChrByPID) it = m_map_pkChrByPID.find(ch->GetPlayerID());

		if (m_map_pkChrByPID.end() != it)
		{
			m_map_pkChrByPID.erase(it);
		}
	}

	UnregisterRaceNumMap(ch);

	RemoveFromStateList(ch);

#ifdef M2_USE_POOL
	pool_.Destroy(ch);
#else
#ifndef DEBUG_ALLOC
	M2_DELETE(ch);
#else
	M2_DELETE_EX(ch, file, line);
#endif
#endif
}

LPCHARACTER CHARACTER_MANAGER::Find(DWORD dwVID)
{
	itertype(m_map_pkChrByVID) it = m_map_pkChrByVID.find(dwVID);

	if (m_map_pkChrByVID.end() == it)
		return NULL;

	// <Factor> Added sanity check
	LPCHARACTER found = it->second;
	if (found != NULL && dwVID != (DWORD)found->GetVID()) {
		sys_err("[CHARACTER_MANAGER::Find] <Factor> %u != %u", dwVID, (DWORD)found->GetVID());
		return NULL;
	}
	return found;
}

LPCHARACTER CHARACTER_MANAGER::Find(const VID & vid)
{
	LPCHARACTER tch = Find((DWORD) vid);

	if (!tch || tch->GetVID() != vid)
		return NULL;

	return tch;
}

LPCHARACTER CHARACTER_MANAGER::FindByPID(DWORD dwPID)
{
	itertype(m_map_pkChrByPID) it = m_map_pkChrByPID.find(dwPID);

	if (m_map_pkChrByPID.end() == it)
		return NULL;

	// <Factor> Added sanity check
	LPCHARACTER found = it->second;
	if (found != NULL && dwPID != found->GetPlayerID()) {
		sys_err("[CHARACTER_MANAGER::FindByPID] <Factor> %u != %u", dwPID, found->GetPlayerID());
		return NULL;
	}
	return found;
}

LPCHARACTER CHARACTER_MANAGER::FindPC(const char * name)
{
	char szName[CHARACTER_NAME_MAX_LEN + 1];
	str_lower(name, szName, sizeof(szName));
	NAME_MAP::iterator it = m_map_pkPCChr.find(szName);

	if (it == m_map_pkPCChr.end())
		return NULL;

	// <Factor> Added sanity check
	LPCHARACTER found = it->second;
	if (found != NULL && strncasecmp(szName, found->GetName(), CHARACTER_NAME_MAX_LEN) != 0) {
		sys_err("[CHARACTER_MANAGER::FindPC] <Factor> %s != %s", name, found->GetName());
		return NULL;
	}
	return found;
}

LPCHARACTER CHARACTER_MANAGER::SpawnMobRandomPosition(DWORD dwVnum, long lMapIndex)
{
	// �ֱ� �������������� ������ �� �ְ���
	{
		if (dwVnum == 5001 && !quest::CQuestManager::instance().GetEventFlag("japan_regen"))
		{
			sys_log(1, "WAEGU[5001] regen disabled.");
			return NULL;
		}
	}

	// ���¸� �������� ������ ������ �� �ְ� ��
	{
		if (dwVnum == 5002 && !quest::CQuestManager::instance().GetEventFlag("newyear_mob"))
		{
			sys_log(1, "HAETAE (new-year-mob) [5002] regen disabled.");
			return NULL;
		}
	}

	// ������ �̺�Ʈ
	{
		if (dwVnum == 5004 && !quest::CQuestManager::instance().GetEventFlag("independence_day"))
		{
			sys_log(1, "INDEPENDECE DAY [5004] regen disabled.");
			return NULL;
		}
	}

	const CMob * pkMob = CMobManager::instance().Get(dwVnum);

	if (!pkMob)
	{
		return NULL;
	}

	if (!map_allow_find(lMapIndex))
	{
		sys_err("not allowed map %u", lMapIndex);
		return NULL;
	}

	LPSECTREE_MAP pkSectreeMap = SECTREE_MANAGER::instance().GetMap(lMapIndex);
	if (pkSectreeMap == NULL) {
		return NULL;
	}

	int i;
	long x, y;
	for (i=0; i<2000; i++)
	{
		x = number(1, (pkSectreeMap->m_setting.iWidth / 100)  - 1) * 100 + pkSectreeMap->m_setting.iBaseX;
		y = number(1, (pkSectreeMap->m_setting.iHeight / 100) - 1) * 100 + pkSectreeMap->m_setting.iBaseY;
		//LPSECTREE tree = SECTREE_MANAGER::instance().Get(lMapIndex, x, y);
		LPSECTREE tree = pkSectreeMap->Find(x, y);

		if (!tree)
			continue;

		DWORD dwAttr = tree->GetAttribute(x, y);

		if (IS_SET(dwAttr, ATTR_BLOCK | ATTR_OBJECT))
			continue;

		if (IS_SET(dwAttr, ATTR_BANPK))
			continue;

		break;
	}

	if (i == 2000)
	{
		sys_err("cannot find valid location");
		return NULL;
	}

	LPSECTREE sectree = SECTREE_MANAGER::instance().Get(lMapIndex, x, y);

	if (!sectree)
	{
		sys_log(0, "SpawnMobRandomPosition: cannot create monster at non-exist sectree %d x %d (map %d)", x, y, lMapIndex);
		return NULL;
	}

	LPCHARACTER ch = CHARACTER_MANAGER::instance().CreateCharacter(pkMob->m_table.szLocaleName);

	if (!ch)
	{
		sys_log(0, "SpawnMobRandomPosition: cannot create new character");
		return NULL;
	}

	ch->SetProto(pkMob);

	// if mob is npc with no empire assigned, assign to empire of map
	if (pkMob->m_table.bType == CHAR_TYPE_NPC)
		if (ch->GetEmpire() == 0)
			ch->SetEmpire(SECTREE_MANAGER::instance().GetEmpireFromMapIndex(lMapIndex));

	ch->SetRotation(number(0, 360));

	if (!ch->Show(lMapIndex, x, y, 0, false))
	{
		M2_DESTROY_CHARACTER(ch);
		sys_err(0, "SpawnMobRandomPosition: cannot show monster");
		return NULL;
	}

	char buf[512+1];
	long local_x = x - pkSectreeMap->m_setting.iBaseX;
	long local_y = y - pkSectreeMap->m_setting.iBaseY;
	snprintf(buf, sizeof(buf), "spawn %s[%d] random position at %ld %ld %ld %ld (time: %d)", ch->GetName(), dwVnum, x, y, local_x, local_y, get_global_time());

	if (test_server)
		SendNotice(buf);

	sys_log(0, buf);
	return (ch);
}

LPCHARACTER CHARACTER_MANAGER::SpawnMob(DWORD dwVnum, long lMapIndex, long x, long y, long z, bool bSpawnMotion, int iRot, bool bShow)
{
	const CMob * pkMob = CMobManager::instance().Get(dwVnum);
	if (!pkMob)
	{
		return NULL;
	}

	if (!(pkMob->m_table.bType == CHAR_TYPE_NPC || pkMob->m_table.bType == CHAR_TYPE_WARP || pkMob->m_table.bType == CHAR_TYPE_GOTO) || mining::IsVeinOfOre (dwVnum))
	{
		LPSECTREE tree = SECTREE_MANAGER::instance().Get(lMapIndex, x, y);

		if (!tree)
		{
			sys_log(0, "no sectree for spawn at %d %d mobvnum %d mapindex %d", x, y, dwVnum, lMapIndex);
			return NULL;
		}

		DWORD dwAttr = tree->GetAttribute(x, y);

		bool is_set = false;

		if ( mining::IsVeinOfOre (dwVnum) ) is_set = IS_SET(dwAttr, ATTR_BLOCK);
		else is_set = IS_SET(dwAttr, ATTR_BLOCK | ATTR_OBJECT);

		if ( is_set )
		{
			// SPAWN_BLOCK_LOG
			static bool s_isLog=quest::CQuestManager::instance().GetEventFlag("spawn_block_log");
			static DWORD s_nextTime=get_global_time()+10000;

			DWORD curTime=get_global_time();

			if (curTime>s_nextTime)
			{
				s_nextTime=curTime;
				s_isLog=quest::CQuestManager::instance().GetEventFlag("spawn_block_log");

			}

			if (s_isLog)
				sys_log(0, "SpawnMob: BLOCKED position for spawn %s %u at %d %d (attr %u)", pkMob->m_table.szName, dwVnum, x, y, dwAttr);
			// END_OF_SPAWN_BLOCK_LOG
			return NULL;
		}

		if (IS_SET(dwAttr, ATTR_BANPK))
		{
			sys_log(0, "SpawnMob: BAN_PK position for mob spawn %s %u at %d %d", pkMob->m_table.szName, dwVnum, x, y);
			return NULL;
		}
	}

	LPSECTREE sectree = SECTREE_MANAGER::instance().Get(lMapIndex, x, y);

	if (!sectree)
	{
		sys_log(0, "SpawnMob: cannot create monster at non-exist sectree %d x %d (map %d)", x, y, lMapIndex);
		return NULL;
	}

	LPCHARACTER ch = CHARACTER_MANAGER::instance().CreateCharacter(pkMob->m_table.szLocaleName);

	if (!ch)
	{
		sys_log(0, "SpawnMob: cannot create new character");
		return NULL;
	}

	if (iRot == -1)
		iRot = number(0, 360);

	ch->SetProto(pkMob);

	// if mob is npc with no empire assigned, assign to empire of map
	if (pkMob->m_table.bType == CHAR_TYPE_NPC)
		if (ch->GetEmpire() == 0)
			ch->SetEmpire(SECTREE_MANAGER::instance().GetEmpireFromMapIndex(lMapIndex));

	ch->SetRotation(iRot);

	if (bShow && !ch->Show(lMapIndex, x, y, z, bSpawnMotion))
	{
		M2_DESTROY_CHARACTER(ch);
		sys_log(0, "SpawnMob: cannot show monster");
		return NULL;
	}

	return (ch);
}

LPCHARACTER CHARACTER_MANAGER::SpawnMobRange(DWORD dwVnum, long lMapIndex, int sx, int sy, int ex, int ey, bool bIsException, bool bSpawnMotion, bool bAggressive )
{
	const CMob * pkMob = CMobManager::instance().Get(dwVnum);

	if (!pkMob)
		return NULL;

	if (pkMob->m_table.bType == CHAR_TYPE_STONE)	// ���� ������ SPAWN ����� �ִ�.
		bSpawnMotion = true;

	int i = 16;

	while (i--)
	{
		int x = number(sx, ex);
		int y = number(sy, ey);
		/*
		   if (bIsException)
		   if (is_regen_exception(x, y))
		   continue;
		 */
		LPCHARACTER ch = SpawnMob(dwVnum, lMapIndex, x, y, 0, bSpawnMotion);

		if (ch)
		{
			sys_log(1, "MOB_SPAWN: %s(%d) %dx%d", ch->GetName(), (DWORD) ch->GetVID(), ch->GetX(), ch->GetY());
			if ( bAggressive )
				ch->SetAggressive();
			return (ch);
		}
	}

	return NULL;
}

void CHARACTER_MANAGER::SelectStone(LPCHARACTER pkChr)
{
	m_pkChrSelectedStone = pkChr;
}

bool CHARACTER_MANAGER::SpawnMoveGroup(DWORD dwVnum, long lMapIndex, int sx, int sy, int ex, int ey, int tx, int ty, LPREGEN pkRegen, bool bAggressive_)
{
	if (!dwVnum)
		return false;
	
	CMobGroup * pkGroup = CMobManager::Instance().GetGroup(dwVnum);

	if (!pkGroup)
	{
		return false;
	}

	LPCHARACTER pkChrMaster = NULL;
	LPPARTY pkParty = NULL;

	const std::vector<DWORD> & c_rdwMembers = pkGroup->GetMemberVector();

	bool bSpawnedByStone = false;
	bool bAggressive = bAggressive_;

	if (m_pkChrSelectedStone)
	{
		bSpawnedByStone = true;
		if (m_pkChrSelectedStone->GetDungeon())
			bAggressive = true;
	}

	for (DWORD i = 0; i < c_rdwMembers.size(); ++i)
	{
		LPCHARACTER tch = SpawnMobRange(c_rdwMembers[i], lMapIndex, sx, sy, ex, ey, true, bSpawnedByStone);

		if (!tch)
		{
			if (i == 0)	// ������ ���Ͱ� ������ ��쿡�� �׳� ����
				return false;

			continue;
		}

		sx = tch->GetX() - number(300, 500);
		sy = tch->GetY() - number(300, 500);
		ex = tch->GetX() + number(300, 500);
		ey = tch->GetY() + number(300, 500);

		if (m_pkChrSelectedStone)
			tch->SetStone(m_pkChrSelectedStone);
		else if (pkParty)
		{
			pkParty->Join(tch->GetVID());
			pkParty->Link(tch);
		}
		else if (!pkChrMaster)
		{
			pkChrMaster = tch;
			pkChrMaster->SetRegen(pkRegen);

			pkParty = CPartyManager::instance().CreateParty(pkChrMaster);
		}
		if (bAggressive)
			tch->SetAggressive();

		if (tch->Goto(tx, ty))
			tch->SendMovePacket(FUNC_WAIT, 0, 0, 0, 0);
	}

	return true;
}

bool CHARACTER_MANAGER::SpawnGroupGroup(DWORD dwVnum, long lMapIndex, int sx, int sy, int ex, int ey, LPREGEN pkRegen, bool bAggressive_, LPDUNGEON pDungeon)
{
	const DWORD dwGroupID = CMobManager::Instance().GetGroupFromGroupGroup(dwVnum);

	if( dwGroupID != 0 )
	{
		return SpawnGroup(dwGroupID, lMapIndex, sx, sy, ex, ey, pkRegen, bAggressive_, pDungeon);
	}
	else
	{
		return false;
	}
}

LPCHARACTER CHARACTER_MANAGER::SpawnGroup(DWORD dwVnum, long lMapIndex, int sx, int sy, int ex, int ey, LPREGEN pkRegen, bool bAggressive_, LPDUNGEON pDungeon)
{
	if (!dwVnum)
		return NULL;
	
	CMobGroup * pkGroup = CMobManager::Instance().GetGroup(dwVnum);

	if (!pkGroup)
	{
		return NULL;
	}

	LPCHARACTER pkChrMaster = NULL;
	LPPARTY pkParty = NULL;

	const std::vector<DWORD> & c_rdwMembers = pkGroup->GetMemberVector();

	bool bSpawnedByStone = false;
	bool bAggressive = bAggressive_;

	if (m_pkChrSelectedStone)
	{
		bSpawnedByStone = true;

		if (m_pkChrSelectedStone->GetDungeon())
			bAggressive = true;
	}

	LPCHARACTER chLeader = NULL;

	for (DWORD i = 0; i < c_rdwMembers.size(); ++i)
	{
		LPCHARACTER tch = SpawnMobRange(c_rdwMembers[i], lMapIndex, sx, sy, ex, ey, true, bSpawnedByStone);

		if (!tch)
		{
			if (i == 0)	// ������ ���Ͱ� ������ ��쿡�� �׳� ����
				return NULL;

			continue;
		}

		if (i == 0)
			chLeader = tch;

		tch->SetDungeon(pDungeon);

		sx = tch->GetX() - number(300, 500);
		sy = tch->GetY() - number(300, 500);
		ex = tch->GetX() + number(300, 500);
		ey = tch->GetY() + number(300, 500);

		if (m_pkChrSelectedStone)
			tch->SetStone(m_pkChrSelectedStone);
		else if (pkParty)
		{
			pkParty->Join(tch->GetVID());
			pkParty->Link(tch);
		}
		else if (!pkChrMaster)
		{
			pkChrMaster = tch;
			pkChrMaster->SetRegen(pkRegen);

			pkParty = CPartyManager::instance().CreateParty(pkChrMaster);
		}

		if (bAggressive)
			tch->SetAggressive();
	}

	return chLeader;
}

struct FuncUpdateAndResetChatCounter
{
	void operator () (LPCHARACTER ch)
	{
		ch->ResetChatCounter();
		ch->CFSM::Update();
	}
};

void CHARACTER_MANAGER::Update(int iPulse)
{
	using namespace std;
#if defined(__GNUC__) && !defined(__clang__) && !defined(CXX11_ENABLED)
	using namespace __gnu_cxx;
#endif

	BeginPendingDestroy();

	// PC ĳ���� ������Ʈ
	{
		if (!m_map_pkPCChr.empty())
		{
			// �����̳� ����
			CHARACTER_VECTOR v;
			v.reserve(m_map_pkPCChr.size());
#if (defined(__GNUC__) && !defined(__clang__)) || defined(CXX11_ENABLED)
			transform(m_map_pkPCChr.begin(), m_map_pkPCChr.end(), back_inserter(v), select2nd<NAME_MAP::value_type>());
#else
			transform(m_map_pkPCChr.begin(), m_map_pkPCChr.end(), back_inserter(v), boost::bind(&NAME_MAP::value_type::second, _1));
#endif

			if (0 == (iPulse % PASSES_PER_SEC(5)))
			{
				FuncUpdateAndResetChatCounter f;
				for_each(v.begin(), v.end(), f);
			}
			else
			{
				for_each(v.begin(), v.end(), std::bind(&CHARACTER::UpdateCharacter, std::placeholders::_1, iPulse));
			}
		}

//		for_each_pc(bind2nd(mem_fun(&CHARACTER::UpdateCharacter), iPulse));
	}

	// ���� ������Ʈ
	{
		if (!m_set_pkChrState.empty())
		{
			CHARACTER_VECTOR v;
			v.reserve(m_set_pkChrState.size());
#if defined(__GNUC__) && !defined(__clang__) && !defined(CXX11_ENABLED)
			transform(m_set_pkChrState.begin(), m_set_pkChrState.end(), back_inserter(v), identity<CHARACTER_SET::value_type>());
#else
			v.insert(v.end(), m_set_pkChrState.begin(), m_set_pkChrState.end());
#endif
			for_each(v.begin(), v.end(), std::bind(&CHARACTER::UpdateStateMachine, std::placeholders::_1, iPulse));
		}
	}

	// ��Ÿ ���� ������Ʈ
	{
		CharacterVectorInteractor i;

		if (CHARACTER_MANAGER::instance().GetCharactersByRaceNum(xmas::MOB_SANTA_VNUM, i))
		{				   
			for_each(i.begin(), i.end(), std::bind(&CHARACTER::UpdateStateMachine, std::placeholders::_1, iPulse));
		}
	}

	// 1�ð��� �ѹ��� �� ��� ���� ���
	if (0 == (iPulse % PASSES_PER_SEC(3600)))
	{
		for (itertype(m_map_dwMobKillCount) it = m_map_dwMobKillCount.begin(); it != m_map_dwMobKillCount.end(); ++it)
			DBManager::instance().SendMoneyLog(MONEY_LOG_MONSTER_KILL, it->first, it->second);

		m_map_dwMobKillCount.clear();
	}

	// �׽�Ʈ ���������� 60�ʸ��� ĳ���� ������ ����
	if (test_server && 0 == (iPulse % PASSES_PER_SEC(60)))
		sys_log(0, "CHARACTER COUNT vid %zu pid %zu", m_map_pkChrByVID.size(), m_map_pkChrByPID.size());

	// ������ DestroyCharacter �ϱ�
	FlushPendingDestroy();

	// ShutdownManager Update
	CShutdownManager::Instance().Update();
}

void CHARACTER_MANAGER::ProcessDelayedSave()
{
	CHARACTER_SET::iterator it = m_set_pkChrForDelayedSave.begin();

	while (it != m_set_pkChrForDelayedSave.end())
	{
		LPCHARACTER pkChr = *it++;
		pkChr->SaveReal();
	}

	m_set_pkChrForDelayedSave.clear();
}

bool CHARACTER_MANAGER::AddToStateList(LPCHARACTER ch)
{
	assert(ch != NULL);

	CHARACTER_SET::iterator it = m_set_pkChrState.find(ch);

	if (it == m_set_pkChrState.end())
	{
		m_set_pkChrState.insert(ch);
		return true;
	}

	return false;
}

void CHARACTER_MANAGER::RemoveFromStateList(LPCHARACTER ch)
{
	CHARACTER_SET::iterator it = m_set_pkChrState.find(ch);

	if (it != m_set_pkChrState.end())
	{
		//sys_log(0, "RemoveFromStateList %p", ch);
		m_set_pkChrState.erase(it);
	}
}

void CHARACTER_MANAGER::DelayedSave(LPCHARACTER ch)
{
	m_set_pkChrForDelayedSave.insert(ch);
}

bool CHARACTER_MANAGER::FlushDelayedSave(LPCHARACTER ch)
{
	CHARACTER_SET::iterator it = m_set_pkChrForDelayedSave.find(ch);

	if (it == m_set_pkChrForDelayedSave.end())
		return false;

	m_set_pkChrForDelayedSave.erase(it);
	ch->SaveReal();
	return true;
}

void CHARACTER_MANAGER::RegisterForMonsterLog(LPCHARACTER ch)
{
	m_set_pkChrMonsterLog.insert(ch);
}

void CHARACTER_MANAGER::UnregisterForMonsterLog(LPCHARACTER ch)
{
	m_set_pkChrMonsterLog.erase(ch);
}

void CHARACTER_MANAGER::PacketMonsterLog(LPCHARACTER ch, const void* buf, int size)
{
	itertype(m_set_pkChrMonsterLog) it;

	for (it = m_set_pkChrMonsterLog.begin(); it!=m_set_pkChrMonsterLog.end();++it)
	{
		LPCHARACTER c = *it;

		if (ch && DISTANCE_APPROX(c->GetX()-ch->GetX(), c->GetY()-ch->GetY())>6000)
			continue;

		LPDESC d = c->GetDesc();

		if (d)
			d->Packet(buf, size);
	}
}

void CHARACTER_MANAGER::KillLog(DWORD dwVnum)
{
	const DWORD SEND_LIMIT = 10000;

	itertype(m_map_dwMobKillCount) it = m_map_dwMobKillCount.find(dwVnum);

	if (it == m_map_dwMobKillCount.end())
		m_map_dwMobKillCount.insert(std::make_pair(dwVnum, 1));
	else
	{
		++it->second;

		if (it->second > SEND_LIMIT)
		{
			DBManager::instance().SendMoneyLog(MONEY_LOG_MONSTER_KILL, it->first, it->second);
			m_map_dwMobKillCount.erase(it);
		}
	}
}

void CHARACTER_MANAGER::RegisterRaceNum(DWORD dwVnum)
{
	m_set_dwRegisteredRaceNum.insert(dwVnum);
}

void CHARACTER_MANAGER::RegisterRaceNumMap(LPCHARACTER ch)
{
	DWORD dwVnum = ch->GetRaceNum();

	if (m_set_dwRegisteredRaceNum.find(dwVnum) != m_set_dwRegisteredRaceNum.end()) // ��ϵ� ��ȣ �̸�
	{
		sys_log(0, "RegisterRaceNumMap %s %u", ch->GetName(), dwVnum);
		m_map_pkChrByRaceNum[dwVnum].insert(ch);
	}
}

void CHARACTER_MANAGER::UnregisterRaceNumMap(LPCHARACTER ch)
{
	DWORD dwVnum = ch->GetRaceNum();

	itertype(m_map_pkChrByRaceNum) it = m_map_pkChrByRaceNum.find(dwVnum);

	if (it != m_map_pkChrByRaceNum.end())
		it->second.erase(ch);
}

bool CHARACTER_MANAGER::GetCharactersByRaceNum(DWORD dwRaceNum, CharacterVectorInteractor & i)
{
	std::map<DWORD, CHARACTER_SET>::iterator it = m_map_pkChrByRaceNum.find(dwRaceNum);

	if (it == m_map_pkChrByRaceNum.end())
		return false;

	// �����̳� ����
	i = it->second;
	return true;
}

#define FIND_JOB_WARRIOR_0	(1 << 3)
#define FIND_JOB_WARRIOR_1	(1 << 4)
#define FIND_JOB_WARRIOR_2	(1 << 5)
#define FIND_JOB_WARRIOR	(FIND_JOB_WARRIOR_0 | FIND_JOB_WARRIOR_1 | FIND_JOB_WARRIOR_2)
#define FIND_JOB_ASSASSIN_0	(1 << 6)
#define FIND_JOB_ASSASSIN_1	(1 << 7)
#define FIND_JOB_ASSASSIN_2	(1 << 8)
#define FIND_JOB_ASSASSIN	(FIND_JOB_ASSASSIN_0 | FIND_JOB_ASSASSIN_1 | FIND_JOB_ASSASSIN_2)
#define FIND_JOB_SURA_0		(1 << 9)
#define FIND_JOB_SURA_1		(1 << 10)
#define FIND_JOB_SURA_2		(1 << 11)
#define FIND_JOB_SURA		(FIND_JOB_SURA_0 | FIND_JOB_SURA_1 | FIND_JOB_SURA_2)
#define FIND_JOB_SHAMAN_0	(1 << 12)
#define FIND_JOB_SHAMAN_1	(1 << 13)
#define FIND_JOB_SHAMAN_2	(1 << 14)
#define FIND_JOB_SHAMAN		(FIND_JOB_SHAMAN_0 | FIND_JOB_SHAMAN_1 | FIND_JOB_SHAMAN_2)

//
// (job+1)*3+(skill_group)
//
LPCHARACTER CHARACTER_MANAGER::FindSpecifyPC(unsigned int uiJobFlag, long lMapIndex, LPCHARACTER except, int iMinLevel, int iMaxLevel)
{
	LPCHARACTER chFind = NULL;
	itertype(m_map_pkChrByPID) it;
	int n = 0;

	for (it = m_map_pkChrByPID.begin(); it != m_map_pkChrByPID.end(); ++it)
	{
		LPCHARACTER ch = it->second;

		if (ch == except)
			continue;

		if (ch->GetLevel() < iMinLevel)
			continue;

		if (ch->GetLevel() > iMaxLevel)
			continue;

		if (ch->GetMapIndex() != lMapIndex)
			continue;

		if (uiJobFlag)
		{
			unsigned int uiChrJob = (1 << ((ch->GetJob() + 1) * 3 + ch->GetSkillGroup()));

			if (!IS_SET(uiJobFlag, uiChrJob))
				continue;
		}

		if (!chFind || number(1, ++n) == 1)
			chFind = ch;
	}

	return chFind;
}

int CHARACTER_MANAGER::GetMobItemRate(LPCHARACTER ch)
{
	//PREVENT_TOXICATION_FOR_CHINA
	if (g_bChinaIntoxicationCheck)
	{
		if ( ch->IsOverTime( OT_3HOUR ) )
		{
			if (ch && ch->GetPremiumRemainSeconds(PREMIUM_ITEM) > 0)
				return m_iMobItemRatePremium/2;
			return m_iMobItemRate/2;
		}
		else if ( ch->IsOverTime( OT_5HOUR ) )
		{
			return 0;
		}
	}
	//END_PREVENT_TOXICATION_FOR_CHINA
	if (ch && ch->GetPremiumRemainSeconds(PREMIUM_ITEM) > 0)
		return m_iMobItemRatePremium;
	return m_iMobItemRate;
}

int CHARACTER_MANAGER::GetMobDamageRate(LPCHARACTER ch)
{
	return m_iMobDamageRate;
}

int CHARACTER_MANAGER::GetMobGoldAmountRate(LPCHARACTER ch)
{
	if ( !ch )
		return m_iMobGoldAmountRate;

	//PREVENT_TOXICATION_FOR_CHINA
	if (g_bChinaIntoxicationCheck)
	{
		if ( ch->IsOverTime( OT_3HOUR ) )
		{
			if (ch && ch->GetPremiumRemainSeconds(PREMIUM_GOLD) > 0)
				return m_iMobGoldAmountRatePremium/2;
			return m_iMobGoldAmountRate/2;
		}
		else if ( ch->IsOverTime( OT_5HOUR ) )
		{
			return 0;
		}
	}
	//END_PREVENT_TOXICATION_FOR_CHINA
	if (ch && ch->GetPremiumRemainSeconds(PREMIUM_GOLD) > 0)
		return m_iMobGoldAmountRatePremium;
	return m_iMobGoldAmountRate;
}

int CHARACTER_MANAGER::GetMobGoldDropRate(LPCHARACTER ch)
{
	if ( !ch )
		return m_iMobGoldDropRate;

	//PREVENT_TOXICATION_FOR_CHINA
	if (g_bChinaIntoxicationCheck)
	{
		if ( ch->IsOverTime( OT_3HOUR ) )
		{
			if (ch && ch->GetPremiumRemainSeconds(PREMIUM_GOLD) > 0)
				return m_iMobGoldDropRatePremium/2;
			return m_iMobGoldDropRate/2;
		}
		else if ( ch->IsOverTime( OT_5HOUR ) )
		{
			return 0;
		}
	}
	//END_PREVENT_TOXICATION_FOR_CHINA
	if (ch && ch->GetPremiumRemainSeconds(PREMIUM_GOLD) > 0)
		return m_iMobGoldDropRatePremium;
	return m_iMobGoldDropRate;
}

int CHARACTER_MANAGER::GetMobExpRate(LPCHARACTER ch)
{
	if ( !ch )
		return m_iMobExpRate;

	//PREVENT_TOXICATION_FOR_CHINA
	if (g_bChinaIntoxicationCheck)
	{
		if ( ch->IsOverTime( OT_3HOUR ) )
		{
			if (ch && ch->GetPremiumRemainSeconds(PREMIUM_EXP) > 0)
				return m_iMobExpRatePremium/2;
			return m_iMobExpRate/2;
		}
		else if ( ch->IsOverTime( OT_5HOUR ) )
		{
			return 0;
		}
	}
	//END_PREVENT_TOXICATION_FOR_CHINA
	if (ch && ch->GetPremiumRemainSeconds(PREMIUM_EXP) > 0)
		return m_iMobExpRatePremium;
	return m_iMobExpRate;
}

int	CHARACTER_MANAGER::GetUserDamageRate(LPCHARACTER ch)
{
	if (!ch)
		return m_iUserDamageRate;

	if (ch && ch->GetPremiumRemainSeconds(PREMIUM_EXP) > 0)
		return m_iUserDamageRatePremium;

	return m_iUserDamageRate;
}

void CHARACTER_MANAGER::SendScriptToMap(long lMapIndex, const std::string & s)
{
	LPSECTREE_MAP pSecMap = SECTREE_MANAGER::instance().GetMap(lMapIndex);

	if (NULL == pSecMap)
		return;

	struct packet_script p;

	p.header = HEADER_GC_SCRIPT;
	p.skin = 1;
	p.src_size = s.size();

	quest::FSendPacket f;
	p.size = p.src_size + sizeof(struct packet_script);
	f.buf.write(&p, sizeof(struct packet_script));
	f.buf.write(&s[0], s.size());

	pSecMap->for_each(f);
}

bool CHARACTER_MANAGER::BeginPendingDestroy()
{
	// Begin �� �Ŀ� Begin�� �� �ϴ� ��쿡 Flush ���� �ʴ� ��� ������ ����
	// �̹� ���۵Ǿ������� false ���� ó��
	if (m_bUsePendingDestroy)
		return false;

	m_bUsePendingDestroy = true;
	return true;
}

void CHARACTER_MANAGER::FlushPendingDestroy()
{
	using namespace std;

	m_bUsePendingDestroy = false; // �÷��׸� ���� �����ؾ� ���� Destroy ó���� ��

	if (!m_set_pkChrPendingDestroy.empty())
	{
		sys_log(0, "FlushPendingDestroy size %d", m_set_pkChrPendingDestroy.size());

		CHARACTER_SET::iterator it = m_set_pkChrPendingDestroy.begin(),
			end = m_set_pkChrPendingDestroy.end();
		for ( ; it != end; ++it) {
			M2_DESTROY_CHARACTER(*it);
		}

		m_set_pkChrPendingDestroy.clear();
	}
}

CharacterVectorInteractor::CharacterVectorInteractor(const CHARACTER_SET & r)
{
	using namespace std;
#if defined(__GNUC__) && !defined(__clang__) && !defined(CXX11_ENABLED)
	using namespace __gnu_cxx;
#endif

	reserve(r.size());
#if defined(__GNUC__) && !defined(__clang__) && !defined(CXX11_ENABLED)
	transform(r.begin(), r.end(), back_inserter(*this), identity<CHARACTER_SET::value_type>());
#else
	insert(end(), r.begin(), r.end());
#endif

	if (CHARACTER_MANAGER::instance().BeginPendingDestroy())
		m_bMyBegin = true;
}

CharacterVectorInteractor::~CharacterVectorInteractor()
{
	if (m_bMyBegin)
		CHARACTER_MANAGER::instance().FlushPendingDestroy();
}


#ifdef ENABLE_MULTI_FARM_BLOCK
void CHARACTER_MANAGER::CheckMultiFarmAccounts(const char* szIP)
{
	auto it = m_mapmultiFarm.find(szIP);
	if (it != m_mapmultiFarm.end())
	{
		auto itVec = it->second.begin();
		while (itVec != it->second.end())
		{
			LPCHARACTER ch = FindByPID(itVec->playerID);
			CCI* chP2P = P2P_MANAGER::Instance().FindByPID(itVec->playerID);
			if (!ch && !chP2P)
				itVec = it->second.erase(itVec);
			else
				++itVec;
		}
		if (!it->second.size())
			m_mapmultiFarm.erase(szIP);
	}
}

void CHARACTER_MANAGER::RemoveMultiFarm(const char* szIP, const uint32_t playerID, const bool isP2P)
{
	if (!isP2P)
	{
		TPacketGGMultiFarm p;
		p.header = HEADER_GG_MULTI_FARM;
		p.subHeader = MULTI_FARM_REMOVE;
		p.playerID = playerID;
		strlcpy(p.playerIP, szIP, sizeof(p.playerIP));
		P2P_MANAGER::Instance().Send(&p, sizeof(TPacketGGMultiFarm));
	}

	auto it = m_mapmultiFarm.find(szIP);
	if (it != m_mapmultiFarm.end())
	{
		for (auto itVec = it->second.begin(); itVec != it->second.end(); ++itVec)
		{
			if (itVec->playerID == playerID)
			{
				it->second.erase(itVec);
				break;
			}
		}
		if (!it->second.size())
			m_mapmultiFarm.erase(szIP);
	}
}

void CHARACTER_MANAGER::SetMultiFarm(const char* szIP, const uint32_t playerID, const char* playerName, const bool bStatus, const uint8_t affectType, const int affectTime)
{
	const auto it = m_mapmultiFarm.find(szIP);
	if (it != m_mapmultiFarm.end())
	{
		for (auto itVec = it->second.begin(); itVec != it->second.end(); ++itVec)
		{
			if (itVec->playerID == playerID)
			{
				itVec->farmStatus = bStatus;
				itVec->affectType = affectType;
				itVec->affectTime = affectTime;
				return;
			}
		}
		it->second.emplace_back(TMultiFarm(playerID, playerName, bStatus, affectType, affectTime));
	}
	else
	{
		std::vector<TMultiFarm> m_vecFarmList;
		m_vecFarmList.emplace_back(TMultiFarm(playerID, playerName, bStatus, affectType, affectTime));
		m_mapmultiFarm.emplace(szIP, m_vecFarmList);
	}
}

int CHARACTER_MANAGER::GetMultiFarmCount(const char* playerIP, std::map<uint32_t, std::pair<std::string, bool>>& m_mapNames)
{
	int accCount = 0;
	bool affectTimeHas = false;
	uint8_t affectType = 0;
	const auto it = m_mapmultiFarm.find(playerIP);
	if (it != m_mapmultiFarm.end())
	{
		for (auto itVec = it->second.begin(); itVec != it->second.end(); ++itVec)
		{
			if (itVec->farmStatus)
				accCount++;
			if (itVec->affectTime > get_global_time())
				affectTimeHas = true;
			if (itVec->affectType > affectType)
				affectType = itVec->affectType;
			m_mapNames.emplace(itVec->playerID, std::make_pair(itVec->playerName, itVec->farmStatus));
		}
	}

	if (affectTimeHas && affectType > 0)
		accCount -= affectType;
	if (accCount < 0)
		accCount = 0;

	return accCount;
}

void CHARACTER_MANAGER::CheckMultiFarmAccount(const char* szIP, const uint32_t playerID, const char* playerName, const bool bStatus, uint8_t affectType, int affectDuration, bool isP2P)
{
	CheckMultiFarmAccounts(szIP);

	LPCHARACTER ch = FindByPID(playerID);
	if (ch && bStatus)
	{
		affectDuration = ch->FindAffect(AFFECT_MULTI_FARM_PREMIUM) ? get_global_time() + ch->FindAffect(AFFECT_MULTI_FARM_PREMIUM)->lDuration : 0;
		affectType = ch->FindAffect(AFFECT_MULTI_FARM_PREMIUM) ? ch->FindAffect(AFFECT_MULTI_FARM_PREMIUM)->lApplyValue : 0;
	}

	std::map<uint32_t, std::pair<std::string, bool>> m_mapNames;
	int farmPlayerCount = GetMultiFarmCount(szIP, m_mapNames);
	if (bStatus)
	{
		if (farmPlayerCount >= 2)
		{
			CheckMultiFarmAccount(szIP, playerID, playerName, false);
			return;
		}
	}

	if (!isP2P)
	{
		TPacketGGMultiFarm p;
		p.header = HEADER_GG_MULTI_FARM;
		p.subHeader = MULTI_FARM_SET;
		p.playerID = playerID;
		strlcpy(p.playerIP, szIP, sizeof(p.playerIP));
		strlcpy(p.playerName, playerName, sizeof(p.playerIP));
		p.farmStatus = bStatus;
		p.affectType = affectType;
		p.affectTime = affectDuration;
		P2P_MANAGER::Instance().Send(&p, sizeof(TPacketGGMultiFarm));
	}

	SetMultiFarm(szIP, playerID, playerName, bStatus, affectType, affectDuration);
	if (ch)
		ch->SetMultiStatus(bStatus);

	m_mapNames.clear();
	farmPlayerCount = GetMultiFarmCount(szIP, m_mapNames);

	for (auto it = m_mapNames.begin(); it != m_mapNames.end(); ++it)
	{
		LPCHARACTER newCh = FindByPID(it->first);
		if (newCh)
		{
			newCh->ChatPacket(CHAT_TYPE_COMMAND, "UpdateMultiFarmAffect %d %d", newCh->GetMultiStatus(), newCh == ch ? true : false);
			for (auto itEx = m_mapNames.begin(); itEx != m_mapNames.end(); ++itEx)
			{
				if (itEx->second.second)
					newCh->ChatPacket(CHAT_TYPE_COMMAND, "UpdateMultiFarmPlayer %s", itEx->second.first.c_str());
			}
		}
	}
}
#endif
#ifdef __DUNGEON_INFO__
bool sortByTime(const TDungeonRank& a, const TDungeonRank& b){return a.value < b.value;}
bool sortByVal(const TDungeonRank& a, const TDungeonRank& b){return a.value > b.value;}
void CHARACTER_MANAGER::SendDungeonRank(LPCHARACTER ch, DWORD mobIdx, BYTE rankIdx)
{
	const auto it = m_mapDungeonList.find(mobIdx);
	if (it == m_mapDungeonList.end())
		return;

	bool reLoad = false;

	auto itMob = m_mapDungeonRank.find(mobIdx);
	if (itMob == m_mapDungeonRank.end())
	{
		std::map<BYTE, std::pair<std::vector<TDungeonRank>, int>> m_data;
		m_mapDungeonRank.emplace(mobIdx, m_data);
		itMob = m_mapDungeonRank.find(mobIdx);
		reLoad = true;
	}

	auto itRank = itMob->second.find(rankIdx);
	if (itRank == itMob->second.end())
	{
		reLoad = true;
		std::pair<std::vector<TDungeonRank>, int> m_vec;
		itMob->second.emplace(rankIdx, m_vec);
		itRank = itMob->second.find(rankIdx);
	}

	if (!reLoad && itRank->second.second < time(0))
		reLoad = true;


	if (reLoad)
	{
		itRank->second.first.clear();
		char szQuery[1024];
		if(rankIdx == 0)
			snprintf(szQuery, sizeof(szQuery), "SELECT dwPID, lValue FROM player.quest WHERE szName = 'dungeon' and szState = '%u_completed' and lValue > 0 ORDER BY lValue DESC LIMIT 10", mobIdx);
		else if(rankIdx == 1)
			snprintf(szQuery, sizeof(szQuery), "SELECT dwPID, lValue FROM player.quest WHERE szName = 'dungeon' and szState = '%u_fastest' and lValue > 0 ORDER BY lValue ASC LIMIT 10", mobIdx);
		else if (rankIdx == 2)
			snprintf(szQuery, sizeof(szQuery), "SELECT dwPID, lValue FROM player.quest WHERE szName = 'dungeon' and szState = '%u_damage' and lValue > 0 ORDER BY lValue DESC LIMIT 10", mobIdx);

		std::unique_ptr<SQLMsg> pMsg(DBManager::Instance().DirectQuery(szQuery));
		if (pMsg->Get()->pSQLResult)
		{
			MYSQL_ROW mRow;
			while (NULL != (mRow = mysql_fetch_row(pMsg->Get()->pSQLResult)))
			{
				TDungeonRank dungeonRank;
				memset(&dungeonRank, 0, sizeof(dungeonRank));
				DWORD playerID;
				str_to_number(playerID, mRow[0]);
				str_to_number(dungeonRank.value, mRow[1]);
				snprintf(szQuery, sizeof(szQuery), "SELECT name, level FROM player.player WHERE id = %d", playerID);
				std::unique_ptr<SQLMsg> pMsg2(DBManager::instance().DirectQuery(szQuery));
				MYSQL_ROW  playerRow = mysql_fetch_row(pMsg2->Get()->pSQLResult);
				if (playerRow)
				{
					strlcpy(dungeonRank.name, playerRow[0], sizeof(dungeonRank.name));
					str_to_number(dungeonRank.level, playerRow[1]);
				}
				itRank->second.first.emplace_back(dungeonRank);
			}
			if (itRank->second.first.size())
			{
				if(rankIdx == 1)
					std::sort(itRank->second.first.begin(), itRank->second.first.end(), sortByTime);
				else
					std::sort(itRank->second.first.begin(), itRank->second.first.end(), sortByVal);
			}
		}
		itRank->second.second = time(0) + (60 * 5);
	}

	std::string cmd("");

	for (BYTE j = 0; j < itRank->second.first.size(); ++j)
	{
		const auto& rank = itRank->second.first[j];

		cmd += rank.name;
		cmd += "|";
		cmd += std::to_string(rank.value);
		cmd += "|";
		cmd += std::to_string(rank.level);
		cmd += "#";
	}
	if (cmd == "")
		cmd = "-";
	ch->ChatPacket(CHAT_TYPE_COMMAND, "dungeon_log_info %u %u %s", mobIdx, rankIdx, cmd.c_str());
}
#endif


#ifdef ENABLE_REWARD_SYSTEM
typedef struct SRewardGlobal
{
	BYTE	bType;
	DWORD	dwSubValue;
	std::vector<std::pair<DWORD, WORD>> mapRewardList;
	SRewardGlobal(const BYTE _bType, const DWORD _dwSubValue, const std::vector<std::pair<DWORD, WORD>> _mapRewardList) : bType(_bType), dwSubValue(_dwSubValue), mapRewardList(_mapRewardList){}
}TRewardGlobal;

const std::map<BYTE, TRewardGlobal> m_mapRewardGlobal = {
	{
		0, TRewardGlobal(
			REWARD_MISSION_DUNGEON,
			2, 
			{
				{31111, 1},
			}
		)
	},
	{
		1, TRewardGlobal(
			REWARD_MISSION_FIRST_ITEM,
			319,
			{
				{31111, 1},
			}
		)
	},
	{
		1, TRewardGlobal(
			REWARD_MISSION_AVERAGE_BONUS,
			55,
			{
				{31111, 1},
			}
		)
	},
	{
		2, TRewardGlobal(
			REWARD_MISSION_AVERAGE_BONUS,
			55,
			{
				{31111, 1},
			}
		)
	},
	{
		3, TRewardGlobal(
			REWARD_MISSION_CUSTOM_SASH,
			25,
			{
				{31111, 1},
			}
		)
	},
	{
		4, TRewardGlobal(
			REWARD_MISSION_FIRST_ITEM,
			310,
			{
				{31111, 1},
			}
		)
	},
	{
		5, TRewardGlobal(
			REWARD_MISSION_LEVEL_UP,
			120,
			{
				{31111, 1},
			}
		)
	},
	{
		6, TRewardGlobal(
			REWARD_MISSION_LEVEL_UP,
			115,
			{
				{31111, 1},
			}
		)
	},
	{
		7, TRewardGlobal(
			REWARD_MISSION_LEVEL_UP_PET,
			115,
			{
				{31111, 1},
			}
		)
	},
	{
		8, TRewardGlobal(
			REWARD_MISSION_DUNGEON,
			1,
			{
				{31111, 1},
			}
		)
	},
	{
		9, TRewardGlobal(
			REWARD_MISSION_DUNGEON,
			2,
			{
				{31111, 1},
			}
		)
	},
	{
		10, TRewardGlobal(
			REWARD_MISSION_SKILL_UPGRADE,
			40,
			{
				{31111, 1},
			}
		)
	},
	{
		11, TRewardGlobal(
			REWARD_MISSION_FIRST_ITEM,
			51002,
			{
				{31111, 1},
			}
		)
	},
	{
		12, TRewardGlobal(
			REWARD_MISSION_FIRST_ITEM,
			18099,
			{
				{31111, 1},
			}
		)
	},
	{
		13, TRewardGlobal(
			REWARD_MISSION_BATTLEPASS,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		14, TRewardGlobal(
			REWARD_MISSION_BIOLOG,
			94,
			{
				{31111, 1},
			}
		)
	},
	{
		15, TRewardGlobal(
			REWARD_MISSION_OFFLINESHOP_SLOT,
			40,
			{
				{31111, 1},
			}
		)
	},
	{
		16, TRewardGlobal(
			REWARD_MISSION_INVENTORY_SLOT,
			40,
			{
				{31111, 1},
			}
		)
	},
};
typedef struct SRewardSolo
{
	BYTE	bType;
	DWORD	dwSubValue;
	DWORD	dwMaxValue;
	BYTE	bLevelDifference;
	std::vector<std::pair<DWORD, WORD>> mapRewardList;
	SRewardSolo(const BYTE _bType, const DWORD _dwSubValue, const DWORD _dwMaxValue, const BYTE _bLevelDifference, const std::vector<std::pair<DWORD, WORD>> _mapRewardList) : bType(_bType), dwSubValue(_dwSubValue), dwMaxValue(_dwMaxValue), bLevelDifference(_bLevelDifference), mapRewardList(_mapRewardList) {}
}TRewardSolo;
const std::map<BYTE, TRewardSolo> m_mapRewardSolo = {
	{
		0, TRewardSolo(
			REWARD_MISSION_LEVEL_UP,
			120,
			1,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		1, TRewardSolo(
			REWARD_MISSION_KILL_BOSS,
			6500,
			50,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		2, TRewardSolo(
			REWARD_MISSION_KILL_BOSS,
			6501,
			200,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		3, TRewardSolo(
			REWARD_MISSION_USE_ITEM,
			67997,
			10000,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		4, TRewardSolo(
			REWARD_MISSION_USE_ITEM,
			67998,
			50,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		5, TRewardSolo(
			REWARD_MISSION_KILL_BOSS,
			2862,
			250,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		6, TRewardSolo(
			REWARD_MISSION_KILL_STONE,
			0,
			100000,
			15,
			{
				{31111, 1},
			}
		)
	},
	{
		7, TRewardSolo(
			REWARD_MISSION_DUNGEON,
			0,
			1000,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		8, TRewardSolo(
			REWARD_MISSION_KILL_BOSS,
			2493,
			100,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		9, TRewardSolo(
			REWARD_MISSION_KILL_BOSS,
			2598,
			100,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		10, TRewardSolo(
			REWARD_MISSION_KILL_BOSS,
			2092,
			100,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		11, TRewardSolo(
			REWARD_MISSION_KILL_BOSS,
			6091,
			400,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		12, TRewardSolo(
			REWARD_MISSION_KILL_BOSS,
			6191,
			400,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		13, TRewardSolo(
			REWARD_MISSION_KILL_BOSS,
			20435,
			100,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		14, TRewardSolo(
			REWARD_MISSION_COMPLETE_SKILL,
			0,
			1,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		15, TRewardSolo(
			REWARD_MISSION_KILL_BOSS,
			0,
			2500,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		16, TRewardSolo(
			REWARD_MISSION_PLAYTIME,
			0,
			43200,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		17, TRewardSolo(
			REWARD_MISSION_OFFLINESHOP_SLOT,
			40,
			1,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		18, TRewardSolo(
			REWARD_MISSION_INVENTORY_SLOT,
			40,
			1,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		19, TRewardSolo(
			REWARD_MISSION_BUY_ITEM,
			51501,
			1,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		20, TRewardSolo(
			REWARD_MISSION_USE_ITEM,
			71084,
			100000,
			0,
			{
				{31111, 1},
			}
		)
	},
	{
		20, TRewardSolo(
			REWARD_MISSION_BATTLEPASS,
			0,
			1,
			0,
			{
				{31111, 1},
			}
		)
	},
};

bool CHARACTER_MANAGER::GetGlobalReward(std::string& szPlayerName, const char* szFilename, ...)
{
	szPlayerName = "";
	char newFileName[254];
	va_list args;
	va_start(args, szFilename);
	vsnprintf(newFileName, sizeof(newFileName), szFilename, args);
	va_end(args);
	FILE* fp;
	if ((fp = fopen(newFileName, "r")) != NULL)
	{
		char	one_line[256];
		while (fgets(one_line, 256, fp))
			break;
		if (strlen(one_line))
			szPlayerName = one_line;
		fclose(fp);
	}
	return szPlayerName.length() ? true : false;
}
void CHARACTER_MANAGER::SetGlobalReward(const char* szPlayerName, const char* szFilename, ...)
{
	char newFileName[254];
	va_list args;
	va_start(args, szFilename);
	vsnprintf(newFileName, sizeof(newFileName), szFilename, args);
	va_end(args);
	FILE* fp;
	if ((fp = fopen(newFileName, "w+")) != NULL)
	{
		fprintf(fp, szPlayerName);
		fclose(fp);
	}
}
void CHARACTER_MANAGER::SendRewardInfo(const BYTE bType, const bool isGlobal, bool isP2P, LPCHARACTER ch)
{
	std::string szCommand("");
	bool clean = true;


	if (ch != NULL && bType == 255)
		ch->SetProtectTime("reward_opened", 1);

	if (isGlobal)
	{
		if (isP2P)
		{
			TPacketGGRewardInfo p;
			p.bHeader = HEADER_GG_REWARD_INFO;
			p.bType = bType;
			P2P_MANAGER::Instance().Send(&p, sizeof(TPacketGGRewardInfo));
		}

		if (bType == 255)
		{
			clean = true;

			for (itertype(m_mapRewardGlobal) it = m_mapRewardGlobal.begin(); it != m_mapRewardGlobal.end(); ++it)
			{
				std::string szPlayerName("");
				if (GetGlobalReward(szPlayerName, "/root/pd2_game/srv1/share/reward_global/%u", it->first))
				{
					szCommand += std::to_string(it->first);
					szCommand += "?";
					szCommand += szPlayerName.c_str();
					szCommand += "#";
				}
			}
		}
		else
		{
			if (ch && ch->GetProtectTime("reward_opened") == 0)
				return;

			clean = false;
			
			itertype(m_mapRewardGlobal) it = m_mapRewardGlobal.find(bType);
			if (it != m_mapRewardGlobal.end())
			{
				std::string szPlayerName("");
				if (GetGlobalReward(szPlayerName, "/root/pd2_game/srv1/share/reward_global/%u", it->first))
				{
					szCommand += std::to_string(it->first);
					szCommand += "?";
					szCommand += szPlayerName.c_str();
					szCommand += "#";
				}
			}
		}
	}
	else
	{
		if (!ch)
			return;

		if (bType == 255)
		{
			clean = true;

			for (itertype(m_mapRewardSolo) it = m_mapRewardSolo.begin(); it != m_mapRewardSolo.end(); ++it)
			{
				szCommand += std::to_string(it->first);
				szCommand += "?";
				szCommand += std::to_string(ch->GetRewardData(it->first));
				szCommand += "#";
			}
		}
		else
		{
			if (ch->GetProtectTime("reward_opened") == 0)
				return;

			clean = false;

			itertype(m_mapRewardSolo) it = m_mapRewardSolo.find(bType);
			if (it != m_mapRewardSolo.end())
			{
				szCommand += std::to_string(it->first);
				szCommand += "?";
				szCommand += std::to_string(ch->GetRewardData(it->first));
				szCommand += "#";
			}
		}
	}

	if (!szCommand.length())
		szCommand = "empty";

	if (ch)
	{
		ch->ChatPacket(CHAT_TYPE_COMMAND, "RewardData %d %d %s", isGlobal, clean, szCommand.c_str());
	}
	else
	{
		const DESC_MANAGER::DESC_SET& c_ref_set = DESC_MANAGER::instance().GetClientSet();
		if (c_ref_set.size())
		{
			for (auto it = c_ref_set.begin(); it != c_ref_set.end(); ++it)
			{
				auto desc = *it;
				if (desc)
				{
					LPCHARACTER tch = desc->GetCharacter();
					if (tch && tch->GetProtectTime("reward_opened") == 1)
						tch->ChatPacket(CHAT_TYPE_COMMAND, "RewardData %d %d %s", isGlobal, clean, szCommand.c_str());
				}
			}
		}
	}
}
bool level_check_difference(int iLevel, int yLevel, int iDifLev)
{
	return ((iLevel - iDifLev <= yLevel) && (iLevel + iDifLev >= yLevel));
}
void CHARACTER_MANAGER::DoReward(LPCHARACTER ch, const BYTE bType, const DWORD dwSubValue, DWORD value, const BYTE levelDifference)
{
	if (!ch)
		return;

	for (auto it = m_mapRewardGlobal.begin(); it != m_mapRewardGlobal.end(); ++it)
	{
		const TRewardGlobal& missionData = it->second;
		if (missionData.bType == bType)
		{
			if (missionData.dwSubValue != 0 && missionData.dwSubValue != dwSubValue)
				continue;
			std::string szPlayerName;
			if (!GetGlobalReward(szPlayerName, "/root/pd2_game/srv1/share/reward_global/%u", it->first))
			{
				SetGlobalReward(ch->GetName(), "/root/pd2_game/srv1/share/reward_global/%u", it->first);
				SendRewardInfo(it->first, true, true);

				//reward_part
				for (BYTE i = 0; i < missionData.mapRewardList.size(); ++i)
					ch->AutoGiveItem(missionData.mapRewardList[i].first, missionData.mapRewardList[i].second);

			}
		}
	}
	for (auto it = m_mapRewardSolo.begin(); it != m_mapRewardSolo.end(); ++it)
	{
		const TRewardSolo& missionData = it->second;
		if (missionData.bType == bType)
		{
			if (missionData.dwSubValue != 0 && missionData.dwSubValue != dwSubValue)
				continue;
			else if (missionData.bLevelDifference != 0 && !level_check_difference(ch->GetLevel(), levelDifference, missionData.bLevelDifference))
				continue;
			DWORD currentVal = ch->GetRewardData(it->first);
			if (currentVal >= missionData.dwMaxValue)
				continue;
			currentVal += value;
			if (currentVal > missionData.dwMaxValue)
				currentVal = missionData.dwMaxValue;
			ch->SetRewardData(it->first, currentVal);
			SendRewardInfo(it->first, false, false, ch);

			//reward_part
			if (currentVal == missionData.dwMaxValue)
			{
				for (BYTE i = 0; i < missionData.mapRewardList.size(); ++i)
					ch->AutoGiveItem(missionData.mapRewardList[i].first, missionData.mapRewardList[i].second);
			}
		}
	}
}
#endif
