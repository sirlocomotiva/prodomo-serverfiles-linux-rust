#include "stdafx.h"
#include "utils.h"
#include "vector.h"
#include "char.h"
#include "sectree_manager.h"
#include "char_manager.h"
#include "mob_manager.h"
#include "ShamanSystem.h"
#include "../common/VnumHelper.h"
#include "packet.h"
#include "item_manager.h"
#include "item.h"
#include "desc.h"
#include "desc_manager.h"

extern int passes_per_sec;
EVENTINFO(shamansystem_event_info)
{
	CShamanSystem* pkShamanSystem;
};

namespace
{
	const uint32_t SHAMAN_MOB_VNUM = 20114;
	const BYTE DEFAULT_SHAMAN_SKILLS_LEVEL = 20;
	const uint32_t DEFAULT_SKILLS_TIME = 60*60*1;
	struct SBuffSkill
	{
		uint32_t dwVnum;
		uint32_t dwAffectFlag;
		uint32_t dwPoints;
		float fCalc;
		bool bPremium;
	};

	const std::map<BYTE, SBuffSkill> m_BuffSkills =
	{
		{0, { 94, AFF_HOSIN, POINT_RESIST_NORMAL_DAMAGE, 1.3f, false } }, // Blogoslawienstwo
		{1, { 95, AFF_BOHO, POINT_MAX_HP, 100, false } }, // Odbicie
		{2, { 96, AFF_GICHEON, POINT_CRITICAL_PCT, 1.3f, false } }, // Pomoc Smoka
		{3, { 111, AFF_JEUNGRYEOK, POINT_ATT_GRADE, 8.75f, true } }, // Zwiekszenie Ataku
		{4, { 110, AFF_KWAESOK, POINT_NORMAL_HIT_DAMAGE_BONUS, 0.3f, true } }, // Zwinnosc
	};
	
	const std::string sBuffPremium("shaman_system.is_premium");
}

EVENTFUNC(shamansystem_update_event)
{
	auto info = static_cast<shamansystem_event_info*>(event->info);
	if (!info) {
		sys_err("shamansystem_update_event> <Factor> Null pointer");
		return 0;
	}

	auto pkSystem = info->pkShamanSystem;
	if (!pkSystem)
		return 0;

	if (!pkSystem->Update()) 
	{
		pkSystem->m_pkShamanUpdateEvent = nullptr;
		return 0;
	}

	return 3;
}

///////////////////////////////////////////////////////////////////////////////////////
//  CShamanActor
///////////////////////////////////////////////////////////////////////////////////////

CShamanActor::CShamanActor(CHARACTER& owner)
	: m_owner(owner)
	, m_dwVnum(SHAMAN_MOB_VNUM)
	, m_dwLastActionTime(0)
	, m_pkChar(nullptr)
	, m_dwLastSkillTime(0)
{
	// ctor
}

CShamanActor::~CShamanActor()
{
	Unsummon();
}

uint32_t CShamanActor::Summon(std::string shamanName, bool bSpawnFar)
{
	int32_t x = m_owner.GetX();
	int32_t y = m_owner.GetY();
	int32_t z = m_owner.GetZ();

	if (!m_owner.GetDungeon() && m_owner.GetMapIndex() < 10000) 
	{
		if (bSpawnFar) 
		{
			x += (number(0, 1) * 2 - 1) * number(2000, 2500);
			y += (number(0, 1) * 2 - 1) * number(2000, 2500);
		} 
		else 
		{
			x += number(-100, 100);
			y += number(-100, 100);
		}
	}

	if (m_pkChar)
	{
		m_pkChar->Show(m_owner.GetMapIndex(), x, y);
		return m_pkChar->GetVID();
	}

	m_pkChar = CHARACTER_MANAGER::instance().SpawnMob(m_dwVnum,
		m_owner.GetMapIndex(),
		x, y, z,
		false,
		(int)(m_owner.GetRotation() + 180),
		false);

	if (!m_pkChar)
	{
		sys_err("[CShamanSystem::Summon] Failed to summon the Shaman. (vnum: %u)", m_dwVnum);
		return 0;
	}

	m_pkChar->SetAutoShaman(this);

	//m_owner.DetailLog();
	//m_pkChar->DetailLog();

	m_pkChar->SetEmpire(m_owner.GetEmpire());
	m_pkChar->SetName(shamanName);
	m_pkChar->Show(m_owner.GetMapIndex(), x, y, z);
	UpdateEquipment();
	UpdateInformations();
	
	return m_pkChar->GetVID();
}

void CShamanActor::UpdateEquipment()
{
	if (!m_pkChar)
		return;
	
	m_pkChar->SetPart(PART_WEAPON, 5109);
	m_pkChar->SetPart(PART_MAIN, 11899);
	m_pkChar->UpdatePacket();
}

void CShamanActor::Unsummon()
{
	if (!m_pkChar)
		return;
	
	m_pkChar->SetAutoShaman(nullptr);
	M2_DESTROY_CHARACTER(m_pkChar);
	m_pkChar = nullptr;
}

bool CShamanActor::Update()
{
	if (IsSummoned()) 
	{
		if (!m_owner.IsDead())
			this->UseSkills();
		
		return UpdateFollowAI();
	}

	return true;
}

bool CShamanActor::IsTrainingSkill(uint32_t dwVnum) const
{
	for (const auto& it : m_BuffSkills)
	{
		if (it.second.dwVnum == dwVnum)
		{
			if (it.second.bPremium && HasPremium() == false)
				return false;
			
			return true;
		}
	}
	
	return false;
}

bool CShamanActor::TrainByQuest(BYTE byVnum)
{
	bool bFound = false;
	BYTE byIndex = 0;
	
	for (const auto& it : m_BuffSkills)
	{
		if (it.second.dwVnum == byVnum)
		{
			bFound = true;
			byIndex = it.first;
		}
	}
	
	if (!bFound)
		return false;
	
	BYTE byLevel = GetBuffLevel(byIndex);
	if (byLevel >= 40 || byLevel < 30)
	{
		sys_err("WTF? Someone is trying to learn skill by quest without skill on Grand Master Level.");
		return false;
	}
	
	int iRandChance = 30; // Wyjebane w nich mam niech kurwa kupują kamole.
	if (m_owner.FindAffect(AFFECT_SKILL_BOOK_BONUS))
	{
		iRandChance = 45;
		m_owner.RemoveAffect(AFFECT_SKILL_BOOK_BONUS);
	}
	
	if (number(1, 100) <= iRandChance)
	{
		SetBuffLevel(byIndex, byLevel + 1);
		UpdateInformations();
		return true;
	}

	return false;
}

void CShamanActor::UpgradeSkill(uint32_t dwVnum, LPITEM pkItem)
{
	if (!pkItem)
		return;
	
	bool bFound = false;
	BYTE byIndex = 0;
	
	for (const auto& it : m_BuffSkills)
	{
		if (it.second.dwVnum == dwVnum)
		{
			bFound = true;
			byIndex = it.first;
		}
	}
	
	if (!bFound)
	{
		sys_err("CShamanActor::UpgradeSkill strange skill vnum used: %u", dwVnum);
		return;
	}
	
	bool can_continue = true;
	if (get_global_time() < m_owner.GetSkillNextReadTime(dwVnum))
	{
		can_continue = false;
		if (m_owner.FindAffect(AFFECT_SKILL_NO_BOOK_DELAY))
		{
			can_continue = true;
			m_owner.RemoveAffect(AFFECT_SKILL_NO_BOOK_DELAY);
		}
	}
	
	if (GetBuffLevel(byIndex) >= 30)
	{
		m_owner.ChatPacket(CHAT_TYPE_INFO, "You cannot train buffer skill with the book, you need to use special stone.");
		return;
	}
	
	if (can_continue)
	{
		int percent = 65;

		if (m_owner.FindAffect(AFFECT_SKILL_BOOK_BONUS))
		{
			percent = 0;
			m_owner.RemoveAffect(AFFECT_SKILL_BOOK_BONUS);
		}
		
		const std::string strBooksFlag = "shaman_system.read_count" + std::to_string(byIndex);
		
		int iNeedBooks = GetBuffLevel(byIndex) - 20;
		int iReadCount = m_owner.GetQuestFlag(strBooksFlag);
		int iReadDelay = number(SKILLBOOK_DELAY_MIN, SKILLBOOK_DELAY_MAX);
		
		if (number(1, 100) > percent)
		{
			if (iReadCount >= iNeedBooks)
			{
				SetBuffLevel(byIndex, GetBuffLevel(byIndex) + 1);
				UpdateInformations();
				
				m_owner.SetQuestFlag(strBooksFlag, 0);
				m_owner.SetSkillNextReadTime(dwVnum, get_global_time() + iReadDelay);
				pkItem->SetCount(pkItem->GetCount() - 1);
				m_owner.ChatPacket(CHAT_TYPE_INFO, "You finished your training with the Book successfully read.");
				
				UseSkill(byIndex);
			}
			else
			{
				m_owner.SetQuestFlag(strBooksFlag, iReadCount+1);
				m_owner.ChatPacket(CHAT_TYPE_INFO, "You need: %d more books to level up this skill.", iNeedBooks - iReadCount);
			}
			return;
		}
		else
		{
			m_owner.SetSkillNextReadTime(dwVnum, get_global_time() + iReadDelay);
			pkItem->SetCount(pkItem->GetCount() - 1);
			m_owner.ChatPacket(CHAT_TYPE_INFO, "That did not work. Darn!");
			m_owner.ChatPacket(CHAT_TYPE_INFO, "Training failed. Please try again later.");
			return;
		}
	}
	
	m_owner.SkillLearnWaitMoreTimeMessage(m_owner.GetSkillNextReadTime(dwVnum) - get_global_time());
}

void CShamanActor::UpdateInformations()
{
	if (!m_owner.GetDesc())
		return;
	
	if (!m_owner.GetDesc()->IsPhase(PHASE_GAME))
		return;
	
	std::string strToSend = "RECV_ShamanSkills ";
	for (const auto & it : m_BuffSkills)
	{
		strToSend += std::to_string(it.second.dwVnum);
		strToSend += "|";
		strToSend += std::to_string(GetBuffLevel(it.first));
		strToSend += ",";
	}
	
	strToSend.pop_back();
	m_owner.ChatPacket(CHAT_TYPE_COMMAND, strToSend.c_str());
	m_owner.ChatPacket(CHAT_TYPE_COMMAND, "RECV_ShamanPremium %d", HasPremium());
}

BYTE CShamanActor::GetBuffLevel(BYTE dwVnum) const
{
	const std::string strBuffLevel = "shaman_system.skill_level" + std::to_string(dwVnum);
	int iValue = MINMAX(DEFAULT_SHAMAN_SKILLS_LEVEL, m_owner.GetQuestFlag(strBuffLevel), SKILL_MAX_LEVEL);
	m_owner.ChatPacket(CHAT_TYPE_INFO, "TEST: CShamanActor::GetBuffLevel %d Level %d", dwVnum, iValue);
	return iValue;
}

void CShamanActor::SetBuffLevel(BYTE dwVnum, BYTE byLevel)
{
	const std::string strBuffLevel = "shaman_system.skill_level" + std::to_string(dwVnum);
	m_owner.SetQuestFlag(strBuffLevel, byLevel);
	m_owner.ChatPacket(CHAT_TYPE_INFO, "TEST: CShamanActor::SetBuffLevel %d Level %d", dwVnum, byLevel);
}

bool CShamanActor::SetPremium(LPITEM pkItem)
{
	if (!pkItem)
		return false;
	
	if (HasPremium())
	{
		m_owner.ChatPacket(CHAT_TYPE_INFO, "You buffer is already premium");
		return false;
	}
	
	m_owner.SetQuestFlag(sBuffPremium, 1);
	pkItem->SetCount(pkItem->GetCount()-1);
	
	UpdateInformations();
	return true;
}

bool CShamanActor::HasPremium() const
{
	return m_owner.GetQuestFlag(sBuffPremium) != 0;
}

void CShamanActor::UseSkill(BYTE dwVnum)
{
	auto it = m_BuffSkills.find(dwVnum);
	if (it == m_BuffSkills.end())
		return;
	
	m_dwLastSkillTime = 0;
	m_owner.RemoveAffect(it->second.dwVnum); // Remove Old Skill
}

void CShamanActor::UseSkills()
{
	if (m_dwLastSkillTime >= get_global_time() || m_owner.IsLoadedAffect() == false)
		return;
	
	for (auto it : m_BuffSkills)
	{
		auto pSkill = it.second;
		auto pIndex = it.first;
		
		if (pSkill.bPremium && HasPremium() == false)
			continue;
		
		if (m_owner.IsAffectFlag(pSkill.dwAffectFlag) == false)
		{
			m_owner.AddAffect(pSkill.dwVnum, pSkill.dwPoints, static_cast<int>(GetBuffLevel(pIndex)*pSkill.fCalc) , pSkill.dwAffectFlag, DEFAULT_SKILLS_TIME, 0, false);
			if (pSkill.dwAffectFlag == AFF_KWAESOK)
				m_owner.AddAffect(pSkill.dwVnum, POINT_CASTING_SPEED, static_cast<int>(GetBuffLevel(pIndex)) , pSkill.dwAffectFlag, DEFAULT_SKILLS_TIME, 0, false);
			
			m_pkChar->SendAutoShamanSkill(pSkill.dwVnum, GetBuffLevel(pIndex));
			sys_log(0, "CShamanActor::UseSkills() -> Using skill by vnum: %d because skill affect was not found.", pSkill.dwVnum);
			m_dwLastSkillTime = get_global_time() + 3;
			return;
		}
	}
}

bool CShamanActor::UpdatAloneActionAI(float fMinDist, float fMaxDist)
{
	float fDist = fnumber(fMinDist, fMaxDist);
	float r = static_cast<float>(fnumber(0.0f, 359.0f));
	float dest_x = m_owner.GetX() + fDist * std::cos(r);
	float dest_y = m_owner.GetY() + fDist * std::sin(r);

	//m_pkChar->SetRotation(GetRandomInt(0, 359));       

	//GetDeltaByDegree(m_pkChar->GetRotation(), fDist, &fx, &fy);

	// Check for loose property; Do not go if the final position and the intermediate position can not be reached.
	//if (!(SECTREE_MANAGER::instance().IsMovablePosition(m_pkChar->GetMapIndex(), m_pkChar->GetX() + (int) fx, m_pkChar->GetY() + (int) fy)
	//			&& SECTREE_MANAGER::instance().IsMovablePosition(m_pkChar->GetMapIndex(), m_pkChar->GetX() + (int) fx/2, m_pkChar->GetY() + (int) fy/2)))
	//	return true;

	m_pkChar->SetNowWalking(true);

	if (!m_pkChar->IsStateMove())
		m_pkChar->Goto(dest_x, dest_y);

	m_dwLastActionTime = get_dword_time();
	return true;
}

bool CShamanActor::UpdateFollowAI()
{
	int START_FOLLOW_DISTANCE = 400; 
	int START_RUN_DISTANCE = 750; 
	int RESPAWN_DISTANCE = 4500; 
	int APPROACH = 250; 

	bool bRun = false;

	uint32_t currentTime = get_dword_time();

	int32_t ownerX = m_owner.GetX();
	int32_t ownerY = m_owner.GetY();

	int32_t charX = m_pkChar->GetX();
	int32_t charY = m_pkChar->GetY();

	const auto dist = DISTANCE_APPROX(charX - ownerX, charY - ownerY);

	if (dist >= RESPAWN_DISTANCE)
	{
		float fx, fy;
		GetDeltaByDegree(m_owner.GetRotation(), -APPROACH, &fx, &fy);

		if (m_pkChar->Show(m_owner.GetMapIndex(), ownerX + fx, ownerY + fy))
			return true;
	}

	if (dist >= START_FOLLOW_DISTANCE && !m_pkChar->IsStateMove())
	{
		if (dist >= START_RUN_DISTANCE)
			bRun = true;

		m_pkChar->SetNowWalking(!bRun);

		Follow(APPROACH);

		m_pkChar->SetLastAttacked(currentTime);
		m_dwLastActionTime = currentTime;
	}

	return true;
}

bool CShamanActor::Follow(int32_t minDistance) const
{
	if(!m_pkChar) 
		return false;

	int32_t ownerX = m_owner.GetX();
	int32_t ownerY = m_owner.GetY();

	int32_t shamanX = m_pkChar->GetX();
	int32_t shamanY = m_pkChar->GetY();

	auto dist = DISTANCE_APPROX(ownerX - shamanX, ownerY - shamanY);
	if (dist <= minDistance)
		return false;

	m_pkChar->SetRotationToXY(ownerX, ownerY);

	float fx, fy;

	float fDistToGo = dist - minDistance;
	GetDeltaByDegree(m_pkChar->GetRotation(), fDistToGo, &fx, &fy);
	
	if (!m_pkChar->Goto((int)(shamanX+fx+0.5f), (int)(shamanY+fy+0.5f)) )
		return false;

	m_pkChar->SendMovePacket(FUNC_WAIT, 0, 0, 0, 0, 0);
	
	return true;
}

///////////////////////////////////////////////////////////////////////////////////////
//  CShamanSystem
///////////////////////////////////////////////////////////////////////////////////////

CShamanSystem::CShamanSystem(CHARACTER& owner)
	: m_owner(owner)
	, m_dwUpdatePeriod(400)
	, m_dwLastUpdateTime(0)
{
	// ctor
}

CShamanSystem::~CShamanSystem()
{
	event_cancel(&m_pkShamanUpdateEvent);
}

bool CShamanSystem::Update()
{
	uint32_t now = get_dword_time();


	if (m_dwUpdatePeriod + m_dwLastUpdateTime > now)
		return true;

	for (auto it = m_ShamanItemsMap.begin(), end = m_ShamanItemsMap.end(); it != end; ) 
	{
		const auto& shamanActor = it->second;
		if (!shamanActor->IsSummoned() || !shamanActor->Update())
			it = m_ShamanItemsMap.erase(it);
		else
			++it;
	}

	m_dwLastUpdateTime = now;
	return !m_ShamanItemsMap.empty();
}

CShamanActor* CShamanSystem::SummonItem(uint32_t itemId, bool bSpawnFar)
{
	const auto it = m_ShamanItemsMap.find(itemId);
	if (it != m_ShamanItemsMap.end()) 
	{
		sys_err("Item %u already has a Shaman", itemId);
		return nullptr;
	}

	auto shamanActor = new CShamanActor(m_owner);

	std::string name = "Buff";
	shamanActor->Summon(std::move(name), bSpawnFar);

	if (!m_pkShamanUpdateEvent) 
	{
		auto info = AllocEventInfo<shamansystem_event_info>();
		info->pkShamanSystem = this;
		m_pkShamanUpdateEvent = event_create(shamansystem_update_event, info, 3);
	}

	m_ShamanItemsMap.emplace(itemId, std::unique_ptr<CShamanActor>(shamanActor));
	return shamanActor;
}

bool CShamanSystem::UpgradePremium(LPITEM pkItem)
{
	if (!pkItem)
		return false;
	
	for (auto it = m_ShamanItemsMap.begin(), end = m_ShamanItemsMap.end(); it != end; ) 
	{
		const auto& shamanActor = it->second;
		if (shamanActor->IsSummoned())
		{
			return shamanActor->SetPremium(pkItem);
		}
	}
	
	return false;
}

bool CShamanSystem::TrainSkill(uint32_t dwSkillVnum, LPITEM pkItem)
{
	if (!pkItem)
		return false;
	
	for (auto it = m_ShamanItemsMap.begin(), end = m_ShamanItemsMap.end(); it != end; ) 
	{
		const auto& shamanActor = it->second;
		if (shamanActor->IsSummoned())
		{
			if (!shamanActor->IsTrainingSkill(dwSkillVnum))
				return false;
			
			shamanActor->UpgradeSkill(dwSkillVnum, pkItem);
			return true;
		}
	}
	
	return false;
}

void CShamanSystem::SendInformations()
{
	for (auto it = m_ShamanItemsMap.begin(), end = m_ShamanItemsMap.end(); it != end; ) 
	{
		const auto& shamanActor = it->second;
		if (shamanActor->IsSummoned())
		{
			shamanActor->UpdateInformations();
			return;
		}
	}
}

bool CShamanSystem::TrainByQuest(BYTE bySkillVnum)
{
	for (auto it = m_ShamanItemsMap.begin(), end = m_ShamanItemsMap.end(); it != end; ) 
	{
		const auto& shamanActor = it->second;
		if (shamanActor->IsSummoned())
		{
			return shamanActor->TrainByQuest(bySkillVnum);
		}
	}
	
	return false;
}

BYTE CShamanSystem::GetSkillLevel(BYTE bySkillVnum)
{
	for (auto it = m_ShamanItemsMap.begin(), end = m_ShamanItemsMap.end(); it != end; ) 
	{
		const auto& shamanActor = it->second;
		if (shamanActor->IsSummoned())
		{
			for (const auto& it : m_BuffSkills)
			{
				if (it.second.dwVnum == bySkillVnum)
					return shamanActor->GetBuffLevel(it.first);
			}
		}
		
	}
	
	return 0;
}
void CShamanSystem::UnsummonItem(uint32_t itemId)
{
	const auto it = m_ShamanItemsMap.find(itemId);
	if (it == m_ShamanItemsMap.end()) 
	{
		sys_err("Item %u not found", itemId);
		return;
	}

	m_ShamanItemsMap.erase(it);

	if (m_ShamanItemsMap.empty() && m_pkShamanUpdateEvent)
		event_cancel(&m_pkShamanUpdateEvent);
}

