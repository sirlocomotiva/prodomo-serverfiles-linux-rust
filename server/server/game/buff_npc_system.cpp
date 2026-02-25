#include "stdafx.h"

#ifdef ENABLE_ASLAN_BUFF_NPC_SYSTEM

#include "utils.h"
#include "vector.h"
#include "char.h"
#include "sectree_manager.h"
#include "char_manager.h"
#include "mob_manager.h"
#include "buff_npc_system.h"
#include "../common/VnumHelper.h"
#include "packet.h"
#include "item_manager.h"
#include "item.h"
#include "affect.h"
#include "skill.h"
#include "db.h"

extern int passes_per_sec;

const	DWORD m_dwSkillVnum[6] = { 94, 95, 96, 109, 110, 111};

const	DWORD m_dwLearnGrandmasterSkillCost[10] = {
	500000,
	1000000,
	2000000,
	3000000,
	5000000,
	7500000,
	12000000,
	15000000,
	20000000,
	25000000,
};

EVENTINFO(buffnpcsystem_event_info)
{
	CBuffNPCActor* pBuffNPCSystem;
};

EVENTFUNC(buffnpcsystem_update_event)
{
	buffnpcsystem_event_info* info = dynamic_cast<buffnpcsystem_event_info*>( event->info );
	if ( info == NULL )
	{
		sys_err( "check_speedhack_event> <Factor> Null pointer" );
		return 0;
	}

	CBuffNPCActor*	pBuffNPCSystem = info->pBuffNPCSystem;

	if (NULL == pBuffNPCSystem)
		return 0;

	pBuffNPCSystem->Update(0);
	return PASSES_PER_SEC(1) / 4;
}

///////////////////////////////////////////////////////////////////////////////////////
//  CbuffNPCActor
///////////////////////////////////////////////////////////////////////////////////////

CBuffNPCActor::CBuffNPCActor(LPCHARACTER owner, DWORD options)
{
	m_dwVID = 0;
	m_dwOptions = options;
	m_dwLastActionTime = 0;

	m_dwIntValue = 0;
		
	m_pkChar = 0;
	m_pkOwner = owner;

	m_originalMoveSpeed = 0;

	m_dwSex = -1;
	m_dwLevel = 1;
	m_dwLevelStep = 0;
	m_dwEXP = 0;
	m_dwNextEXP = 0;
	m_dwSkillPoints = 0;
	m_dwIntValue = 0;
	
	for (int i = 0; i < 6; i++) {
		m_dwSkill[i] = 0;
		m_dwSkillNextUse[i] = 0;
		m_dwSkillInUse[i] = 0;
	}
	
	m_dwUpdatePeriod = 400;
	m_dwLastUpdateTime = 0;
}

CBuffNPCActor::~CBuffNPCActor()
{
	this->Destroy();
}

void CBuffNPCActor::Destroy()
{
	SetIsActive(false);
	this->Unsummon();

	m_pkOwner = 0;
	m_dwSex = -1;
	m_dwLevel = 1;
	m_dwLevelStep = 0;
	m_dwEXP = 0;
	m_dwNextEXP = 0;
	m_dwSkillPoints = 0;
	m_dwIntValue = 0;
	
	for (int i = 0; i < 6; i++) {
		m_dwSkill[i] = 0;
		m_dwSkillNextUse[i] = 0;
		m_dwSkillInUse[i] = 0;
	}
	
	event_cancel(&m_pkBuffNPCSystemExpireEvent);
	m_pkBuffNPCSystemExpireEvent = NULL;
}

void CBuffNPCActor::LoadBuffNPC()
{
	std::unique_ptr<SQLMsg> pMsg2(DBManager::instance().DirectQuery("SELECT name, sex, level, exp, skillpoint, intvalue, skill0lv, skill1lv, skill2lv, skill3lv, skill4lv, skill5lv FROM player_buff_npc WHERE player_id = %lu", m_pkOwner->GetPlayerID()));
	if (pMsg2->Get()->uiNumRows > 0) {
		MYSQL_ROW row = mysql_fetch_row(pMsg2->Get()->pSQLResult);			
		this->SetName(row[0]);		
		this->m_dwSex					= atoi(row[1]);
		this->m_dwVnum 				= atoi(row[1])+10;
		this->m_dwLevel					= atoi(row[2]);
		this->m_dwEXP					= atoi(row[3]);
		this->m_dwSkillPoints			= atoi(row[4]);
		this->m_dwIntValue				= atoi(row[5]);
		this->m_dwSkill[0] 				= atoi(row[6]);
		this->m_dwSkill[1] 				= atoi(row[7]);
		this->m_dwSkill[2] 				= atoi(row[8]);
		this->m_dwSkill[3] 				= atoi(row[9]);
		this->m_dwSkill[4] 				= atoi(row[10]);
		this->m_dwSkill[5] 				= atoi(row[11]);
	}
	else {
		SetIsActive(false);
		return;
	}
	
	for (int i = 0; i < 6; i++) {
		char flag[128+1];
		memset(flag, 0, sizeof(flag));
		snprintf(flag, sizeof(flag), "buff_npc_learn.%u.use_deactivate", m_dwSkillVnum[i]);
		m_dwSkillInUse[i] = m_pkOwner->GetQuestFlag(flag);
	}

	SetIsActive(true);
	SetNextExp(m_pkOwner->BuffGetNextExpByLevel(m_dwLevel));
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCBasicInfo %s %d %d", m_name.c_str(), m_dwSex, m_dwIntValue);
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCEXPInfo %d %d %d", m_dwLevel, m_dwEXP, m_dwNextEXP);
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillInfo %d %d %d %d %d %d %d", m_dwSkill[0], m_dwSkill[1], m_dwSkill[2], m_dwSkill[3], m_dwSkill[4], m_dwSkill[5], m_dwSkillPoints);
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillUseStatus %d %d %d %d %d %d", m_dwSkillInUse[0], m_dwSkillInUse[1], m_dwSkillInUse[2], m_dwSkillInUse[3], m_dwSkillInUse[4], m_dwSkillInUse[5]);
}

DWORD CBuffNPCActor::Summon()
{
	std::unique_ptr<SQLMsg> pMsg(DBManager::instance().DirectQuery("SELECT * FROM player.player_buff_npc WHERE player_id=%ld LIMIT 1", m_pkOwner->GetPlayerID()));
	if (pMsg->Get()->uiNumRows == 0) {
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_YOU_DONT_HAVE_ACTIVE_SHAMAN"));
		return 0;
	}

	long x = m_pkOwner->GetX();
	long y = m_pkOwner->GetY();
	long z = m_pkOwner->GetZ();

	x += number(-300, 300);
	y += number(-300, 300);

	if (0 != m_pkChar) {
		m_pkChar->Show (m_pkOwner->GetMapIndex(), x, y);
		m_dwVID = m_pkChar->GetVID();

		return m_dwVID;
	}
	
	m_pkChar = CHARACTER_MANAGER::instance().SpawnMob( m_dwVnum, m_pkOwner->GetMapIndex(), x, y, z, false, (int)(m_pkOwner->GetRotation()+180), false);

	if (0 == m_pkChar) {
		sys_err("[CBuffNPCSystem::Summon] Failed to summon buff. (vnum: %d)", m_dwVnum);
		return 0;
	}

	m_pkChar->SetBuffNPC(true);
	m_pkChar->SetEmpire(m_pkOwner->GetEmpire());
	m_dwVID = m_pkChar->GetVID();

	SetName(m_name.c_str());		
	SetLevel(m_dwLevel);
	
	buffnpcsystem_event_info* info = AllocEventInfo<buffnpcsystem_event_info>();
	info->pBuffNPCSystem = this;
	m_pkBuffNPCSystemExpireEvent = event_create(buffnpcsystem_update_event, info, PASSES_PER_SEC(1) / 4);
		
	m_pkOwner->ComputePoints();
	m_pkOwner->SetBuffNPCInt(m_dwIntValue);
	m_pkChar->Show(m_pkOwner->GetMapIndex(), x, y, z);
	
	UpdateBuffEquipment();
	LPITEM iExpItem = m_pkOwner->GetBuffWear(3);
	if (iExpItem != NULL)
		iExpItem->StartUniqueExpireEvent();
	
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSummon");

	return m_dwVID;
}

void CBuffNPCActor::Unsummon()
{
	if (true == this->IsSummoned())
	{
		if (NULL != m_pkOwner)
			m_pkOwner->ComputePoints();

		if (NULL != m_pkChar) {
			SaveSQL();
			m_pkChar->SetBuffNPC(false);
			M2_DESTROY_CHARACTER(m_pkChar);
			m_pkChar = 0;
		}

		LPITEM iExpItem = m_pkOwner->GetBuffWear(3);
		if (iExpItem != NULL)
			iExpItem->StopUniqueExpireEvent();
	
		event_cancel(&m_pkBuffNPCSystemExpireEvent);
		m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCUnsummon");
	}
}

void CBuffNPCActor::ClearBuffNPC()
{
	m_dwSex = -1;
	m_dwLevel = 1;
	m_dwLevelStep = 0;
	m_dwEXP = 0;
	m_dwNextEXP = 0;
	m_dwSkillPoints = 0;
	m_dwIntValue = 0;
	
	for (int i = 0; i < 6; i++) {
		m_dwSkill[i] = 0;
		m_dwSkillNextUse[i] = 0;
		m_dwSkillInUse[i] = 0;
	}
	
	SetIsActive(false);
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCClear");
}

void CBuffNPCActor::SaveSQL() {
	DBManager::instance().DirectQuery
	(
		"UPDATE player_buff_npc SET player_name='%s', name='%s', sex=%d, level=%d, exp=%d, skillpoint=%d, intvalue=%d, skill0lv=%d, skill1lv=%d, skill2lv=%d, skill3lv=%d, skill4lv=%d, skill5lv=%d WHERE player_id = %d ",
		m_pkOwner->GetName(), m_name.c_str(), this->m_dwSex, this->GetLevel(), this->GetExp(), this->m_dwSkillPoints, this->m_dwIntValue,
		this->m_dwSkill[0], this->m_dwSkill[1], this->m_dwSkill[2], this->m_dwSkill[3], this->m_dwSkill[4], this->m_dwSkill[5], 
		m_pkOwner->GetPlayerID()
	);
}

void CBuffNPCActor::UpdateBuffEquipment()
{
	if (NULL == m_pkChar)
		return;
	
	LPITEM	iWeapon = m_pkOwner->GetBuffWear(0);
	LPITEM	iHead = m_pkOwner->GetBuffWear(1);
	LPITEM	iArmor = m_pkOwner->GetBuffWear(2);

	if (iWeapon != NULL)
		m_pkChar->SetBuffWearWeapon(iWeapon->GetVnum());
	else
		m_pkChar->SetBuffWearWeapon(0);

	if (iHead != NULL)
		m_pkChar->SetBuffWearHead(iHead->GetValue(3));
	else
		m_pkChar->SetBuffWearHead(0);
	
	if (iArmor != NULL)
		m_pkChar->SetBuffWearArmor(iArmor->GetVnum());
	else
		m_pkChar->SetBuffWearArmor(0);
	
	m_pkChar->UpdatePacket();
}

void CBuffNPCActor::SetName(const char* name)
{
	std::string buffName = "";

	if (0 != m_pkOwner && 
		0 == name && 
		0 != m_pkOwner->GetName())
	{
		buffName += "'s Buff";
	}
	else
		buffName += name;

	if (true == IsSummoned())
		m_pkChar->SetName(buffName);

	m_name = buffName;
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCBasicInfo %s %d %d", m_name.c_str(), m_dwSex, m_dwIntValue);
}

void CBuffNPCActor::SetCMDLevel(DWORD level)
{
	if(level > ASLAN_BUFF_NPC_MAX_LV)
		level = ASLAN_BUFF_NPC_MAX_LV;

	m_dwLevel = 1;
	m_dwIntValue = ASLAN_BUFF_NPC_START_INT;
	m_dwSkillPoints = level-1;
	
	for (int i = 0; i < 6; i++) {
		if(m_dwSkill[i] < 17)
			m_dwSkillPoints -= m_dwSkill[i];
		else
			m_dwSkillPoints -= 17;
	}
	
	for (int i = 1; i < level; i++) {
		m_dwLevel += 1;
		m_dwIntValue += 3;
	}
	if (m_dwIntValue > ASLAN_BUFF_NPC_MAX_INT)
		m_dwIntValue = ASLAN_BUFF_NPC_MAX_INT;

	m_dwLevelStep = 0;
	m_dwEXP = 0;

	if (true == IsSummoned()) {
		m_pkChar->SendBuffLevelUpEffect(m_pkChar->GetVID(), 1, GetLevel(), 1);
		SetLevel(m_dwLevel);
	}
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCBasicInfo %s %d %d", m_name.c_str(), m_dwSex, m_dwIntValue);
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCEXPInfo %d %d %d", GetLevel(), GetExp(), GetNextExp());
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillInfo %d %d %d %d %d %d %d", m_dwSkill[0], m_dwSkill[1], m_dwSkill[2], m_dwSkill[3], m_dwSkill[4], m_dwSkill[5], m_dwSkillPoints);
}

void CBuffNPCActor::SetLevel(DWORD level)
{
	m_pkChar->SetLevel(static_cast<char>(level));
	m_dwLevel = level;
	SetNextExp(m_pkChar->BuffGetNextExp());
	m_pkChar->UpdatePacket();
	m_pkOwner->UpdatePacket();
}

void CBuffNPCActor::SetLevelUp()
{
	if(GetLevel() == ASLAN_BUFF_NPC_MAX_LV) {
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_MAX_LEVEL"));
		return;
	}

	SetLevel(GetLevel() + 1);
	m_pkChar->SendBuffLevelUpEffect(m_pkChar->GetVID(), 1, GetLevel(), 1);
	m_dwLevelStep = 0;
	m_dwEXP = 0;
	m_dwSkillPoints = m_dwSkillPoints + 1;
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCEXPInfo %d %d %d", GetLevel(), GetExp(), GetNextExp());
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillInfo %d %d %d %d %d %d %d", m_dwSkill[0], m_dwSkill[1], m_dwSkill[2], m_dwSkill[3], m_dwSkill[4], m_dwSkill[5], m_dwSkillPoints);

	SaveSQL();
	return;
}

void CBuffNPCActor::SetNextExp(int nextExp)
{	
	m_dwNextEXP = nextExp;
}

void CBuffNPCActor::SetExp(DWORD exp)
{
	if (false == IsSummoned())
			return;

	if(GetLevel() >= 120) {
		m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCEXPInfo %d %d %d %d", m_dwLevel, 0, 0);
		return;
	}

	if (exp < 0)
		exp = MAX(m_dwEXP - exp, 0);

	if (GetExp() + exp >= GetNextExp())
		SetLevelUp();

	m_dwEXP += exp;
	m_pkChar->SetExp(m_dwEXP);
	
	if (GetLevelStep() < 3) {
		if (GetExp() >= GetNextExp() / 4 * 3 && m_dwLevelStep == 2) {
			m_dwLevelStep = 3;
			SetInt(m_dwIntValue + 1);
			m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCBasicInfo %s %d %d", m_name.c_str(), m_dwSex, m_dwIntValue);
			m_pkChar->SendBuffLevelUpEffect(m_pkChar->GetVID(), 25, GetLevel(), 1);
			SaveSQL();
		}
		else if (GetExp() >= GetNextExp() / 4 * 2 && m_dwLevelStep == 1) {
			m_dwLevelStep = 2;
			SetInt(m_dwIntValue + 1);
			m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCBasicInfo %s %d %d", m_name.c_str(), m_dwSex, m_dwIntValue);
			m_pkChar->SendBuffLevelUpEffect(m_pkChar->GetVID(), 25, GetLevel(), 1);
			SaveSQL();
		}
		else if (GetExp() >= GetNextExp() / 4 && m_dwLevelStep == 0) {
			m_dwLevelStep = 1;
			SetInt(m_dwIntValue + 1);
			m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCBasicInfo %s %d %d", m_name.c_str(), m_dwSex, m_dwIntValue);
			m_pkChar->SendBuffLevelUpEffect(m_pkChar->GetVID(), 25, GetLevel(), 1);
			SaveSQL();
		}
	}
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCEXPInfo %d %d %d", m_dwLevel, m_dwEXP, m_dwNextEXP);
}

void CBuffNPCActor::SetInt(DWORD value)
{
	if (m_dwIntValue < ASLAN_BUFF_NPC_MAX_INT) {
		m_dwIntValue = value;
		m_pkChar->SetBuffNPCInt(m_dwIntValue);
	}
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCBasicInfo %s %d %d", m_name.c_str(), m_dwSex, m_dwIntValue);
}

void CBuffNPCActor::SetSkillUseStatus(int skillSlot, int value)
{
	char flag[128+1];
	memset(flag, 0, sizeof(flag));
	snprintf(flag, sizeof(flag), "buff_npc_learn.%u.use_deactivate", m_dwSkillVnum[skillSlot]);
	if (m_dwSkillInUse[skillSlot] == 0) {
		m_dwSkillInUse[skillSlot] = 1;
		m_pkOwner->SetQuestFlag(flag, 1);
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_DEACTIVATE_SKILL"));
	}
	else {
		m_dwSkillInUse[skillSlot] = 0;
		m_pkOwner->SetQuestFlag(flag, 0);
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_ACTIVATE_SKILL"));
	}
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillUseStatus %d %d %d %d %d %d", m_dwSkillInUse[0], m_dwSkillInUse[1], m_dwSkillInUse[2], m_dwSkillInUse[3], m_dwSkillInUse[4], m_dwSkillInUse[5]);
}

bool CBuffNPCActor::UseSkill()
{
	if (NULL == m_pkOwner || m_pkOwner->IsDead() != false || m_pkOwner->IsStun() != false || get_dword_time() - this->GetLastSkillTime() < 4000)
		return false;

	if(m_dwSkillInUse[3] != 1 && m_pkOwner->GetHPPct() <= ASLAN_BUFF_NPC_MIN_HP_PERC_USE_SKILL_HEAL && m_pkOwner->GetHPPct() > 0 && this->m_dwSkill[3] > 0 && m_dwSkillNextUse[3] < get_dword_time())
	{
		CSkillProto * pkSk = CSkillManager::instance().Get(109);
		int iCooltime = (int) pkSk->kCooldownPoly.Eval();
		m_dwSkillNextUse[3] = iCooltime * 1000 + get_dword_time();
		
		m_pkOwner->ComputeSkill(109, m_pkOwner, this->m_dwSkill[3], true);
		this->SetLastSkillTime(get_dword_time());
		m_pkChar->SendBuffNPCUseSkillPacket(SKILL_HOSIN, this->m_dwSkill[3]);
		
		
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_USE_SKILL_CURE"), num_with_point(m_pkOwner->GetBuffHealValue()).c_str());
		m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillCooltime %d %d", 3, iCooltime);
		return true;
	}
	else if(m_dwSkillInUse[0] != 1 && this->m_dwSkill[0] > 0 && m_pkOwner->IsAffectFlag(AFF_HOSIN) == false && m_dwSkillNextUse[0] < get_dword_time())
	{
		CSkillProto * pkSk = CSkillManager::instance().Get(94);
		int iCooltime = (int) pkSk->kCooldownPoly.Eval();
		m_dwSkillNextUse[0] = iCooltime * 1000 + get_dword_time();
		
		m_pkOwner->ComputeSkill(94, m_pkOwner, this->m_dwSkill[0], true);
		this->SetLastSkillTime(get_dword_time());
		m_pkChar->SendBuffNPCUseSkillPacket(SKILL_HOSIN, this->m_dwSkill[0]);
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_USE_SKILL_BLESSING"));
		m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillCooltime %d %d", 0, iCooltime);
		return true;
	}
	else if(m_dwSkillInUse[1] != 1 && this->m_dwSkill[1] > 0 && m_pkOwner->IsAffectFlag(AFF_BOHO) == false && m_dwSkillNextUse[1] < get_dword_time())
	{
		CSkillProto * pkSk = CSkillManager::instance().Get(95);
		int iCooltime = (int) pkSk->kCooldownPoly.Eval();
		m_dwSkillNextUse[1] = iCooltime * 1000 + get_dword_time();
		
		m_pkOwner->ComputeSkill(95, m_pkOwner, this->m_dwSkill[1], true);		
		this->SetLastSkillTime(get_dword_time());
		m_pkChar->SendBuffNPCUseSkillPacket(SKILL_REFLECT, this->m_dwSkill[1]);
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_USE_SKILL_REFLECT"));
		m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillCooltime %d %d", 1, iCooltime);
		return true;
	}
	else if(m_dwSkillInUse[2] != 1 && this->m_dwSkill[2] > 0 && m_pkOwner->IsAffectFlag(AFF_GICHEON) == false && m_dwSkillNextUse[2] < get_dword_time())
	{
		CSkillProto * pkSk = CSkillManager::instance().Get(96);
		int iCooltime = (int) pkSk->kCooldownPoly.Eval();
		m_dwSkillNextUse[2] = iCooltime * 1000 + get_dword_time();
		
		m_pkOwner->ComputeSkill(96, m_pkOwner, this->m_dwSkill[2], true);	
		this->SetLastSkillTime(get_dword_time());	
		m_pkOwner->SendBuffNPCUseSkillPacket(SKILL_GICHEON, this->m_dwSkill[2]);
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_USE_SKILL_DRAGONS_STRANGE"));
		m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillCooltime %d %d", 2, iCooltime);
		return true;
	}
	else if(m_dwSkillInUse[4] != 1 && this->m_dwSkill[4] > 0 && m_pkOwner->IsAffectFlag(AFF_KWAESOK) == false && m_dwSkillNextUse[4] < get_dword_time())
	{
		CSkillProto * pkSk = CSkillManager::instance().Get(110);
		int iCooltime = (int) pkSk->kCooldownPoly.Eval();
		m_dwSkillNextUse[4] = iCooltime * 1000 + get_dword_time();
		
		m_pkOwner->ComputeSkill(110, m_pkOwner, this->m_dwSkill[4], true);		
		this->SetLastSkillTime(get_dword_time());
		m_pkChar->SendBuffNPCUseSkillPacket(SKILL_REFLECT, this->m_dwSkill[4]);
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_USE_SKILL_SWIFTNESS"));
		m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillCooltime %d %d", 4, iCooltime);
		return true;
	}
	else if(m_dwSkillInUse[5] != 1 && this->m_dwSkill[5] > 0 && m_pkOwner->IsAffectFlag(AFF_JEUNGRYEOK) == false && m_dwSkillNextUse[5] < get_dword_time())
	{
		CSkillProto * pkSk = CSkillManager::instance().Get(111);
		int iCooltime = (int) pkSk->kCooldownPoly.Eval();
		m_dwSkillNextUse[5] = iCooltime * 1000 + get_dword_time();
		
		m_pkOwner->ComputeSkill(111, m_pkOwner, this->m_dwSkill[5], true);	
		this->SetLastSkillTime(get_dword_time());	
		m_pkOwner->SendBuffNPCUseSkillPacket(SKILL_GICHEON, this->m_dwSkill[5]);
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_USE_SKILL_ATTACK_UP"));
		m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillCooltime %d %d", 5, iCooltime);
		return true;
	}
	return false;
}

void CBuffNPCActor::SetSkill(DWORD dwSkillSlot, DWORD dwSkillLevel)
{
	m_dwSkill[dwSkillSlot] = dwSkillLevel;
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillInfo %d %d %d %d %d %d %d", m_dwSkill[0], m_dwSkill[1], m_dwSkill[2], m_dwSkill[3], m_dwSkill[4], m_dwSkill[5], m_dwSkillPoints);
}

void CBuffNPCActor::LearnSkillByPoint(DWORD dwSkillType)
{
	if(m_dwSkillPoints > 0)
	{
		if(m_dwSkill[dwSkillType] >= 16)
		{
#ifdef ASLAN_BUFF_NPC_USE_SKILL_17_TO_M
			m_dwSkill[dwSkillType] = 20;
			m_dwSkillPoints = m_dwSkillPoints - 1;
#else
			if(m_dwSkill[dwSkillType] >= 19) {
				m_dwSkill[dwSkillType] = 20;
				m_dwSkillPoints = m_dwSkillPoints - 1;
			}
			else {
				int chance = number(1, 100);
				if(chance > ASLAN_BUFF_NPC_USE_SKILL_TECH_LEVEL_CHANCE) {
					m_dwSkill[dwSkillType] = this->m_dwSkill[dwSkillType] + 1;
					m_dwSkillPoints = m_dwSkillPoints - 1;
				}
				else {
					m_dwSkill[dwSkillType] = 20;
					m_dwSkillPoints = m_dwSkillPoints - 1;
				}
			}
#endif
		}
		else
		{
			m_dwSkill[dwSkillType] = this->m_dwSkill[dwSkillType] + 1;
			m_dwSkillPoints = m_dwSkillPoints - 1;
		}
		m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillInfo %d %d %d %d %d %d %d", m_dwSkill[0], m_dwSkill[1], m_dwSkill[2], m_dwSkill[3], m_dwSkill[4], m_dwSkill[5], m_dwSkillPoints);
	}
}

void CBuffNPCActor::LearnSkillByBook(DWORD dwSkillVnum)
{
	LPITEM item_book = m_pkOwner->GetBuffEquipmentItem(BUFF_SLOT_SKILLBOOK);
	
	if (item_book == NULL) {
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, "ERROR - No book avaible!");
		return;
	}
	
	if (item_book->GetVnum() == 50300 && item_book->GetSocket(0) == dwSkillVnum or (item_book->GetVnum()-50400) == dwSkillVnum)
	{
		int iSkillID = GetSkillID(dwSkillVnum);
		int iSkillLevel = m_dwSkill[iSkillID];
		int iSkillGrade = GetSkillGrade(iSkillLevel);

		if(iSkillID == -1 || iSkillGrade == -1)
			return;
			
		if(iSkillGrade != 1) {
			if (iSkillGrade > 1)
				m_pkOwner->ChatPacket(CHAT_TYPE_INFO, "[LS;959]");
			else
				m_pkOwner->ChatPacket(CHAT_TYPE_INFO, "[LS;959]");
			return;
		}

		char flag_next_read[128+1];
		memset(flag_next_read, 0, sizeof(flag_next_read));
		snprintf(flag_next_read, sizeof(flag_next_read), "buff_npc_learn.%u.next_read", dwSkillVnum);
		char flag_read_count[128+1];
		memset(flag_read_count, 0, sizeof(flag_read_count));
		snprintf(flag_read_count, sizeof(flag_read_count), "buff_npc_learn.%u.read_count", dwSkillVnum);
		
		int iNextReadLastTime = m_pkOwner->GetQuestFlag(flag_next_read);
		int iReadCount = m_pkOwner->GetQuestFlag(flag_read_count);

		LPITEM item_exo = m_pkOwner->GetBuffEquipmentItem(BUFF_SLOT_SKILLBOOK_COMMON_1);
		
#ifndef ASLAN_BUFF_NPC_USE_SKILL_TECH_LEVEL
		if (item_exo == NULL) {
			m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_NEED_EXORZISM_SCROLL"));
			return;
		}
#endif
		
		if(get_global_time() < iNextReadLastTime)
		{
			if (item_exo != NULL)
			{
				item_exo->SetCount(item_exo->GetCount() - 1);
				m_pkOwner->ChatPacket(CHAT_TYPE_INFO, "[LS;377]");
			}
			else
			{
				m_pkOwner->SkillLearnWaitMoreTimeMessage(iNextReadLastTime - get_global_time());
				return;
			}
		}

		item_book->SetCount(item_book->GetCount() - 1);
			
		int need_bookcount = iSkillLevel - 20;
		int percent = 65;

		LPITEM item_read_bonus = m_pkOwner->GetBuffEquipmentItem(BUFF_SLOT_SKILLBOOK_COMMON_2);
		if (item_read_bonus != NULL)
		{
			item_read_bonus->SetCount(item_read_bonus->GetCount() - 1);
			percent = 0;
		}

		if(number(1, 100) > percent)
		{
			
			DWORD nextTime = get_global_time() + number(28800, 43200);
			m_pkOwner->SetQuestFlag(flag_next_read, nextTime);

#ifdef ASLAN_BUFF_NPC_USE_SKILL_TECH_LEVEL
			if (iReadCount >= need_bookcount)
#else
			if (0 == 0)
#endif
			{
				m_dwSkill[iSkillID] = this->m_dwSkill[iSkillID] + 1;
				m_pkOwner->SetQuestFlag(flag_read_count, 0);

				m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("책으로 더 높은 경지의 수련을 성공적으로 끝내셨습니다."));
				m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillInfo %d %d %d %d %d %d %d", m_dwSkill[0], m_dwSkill[1], m_dwSkill[2], m_dwSkill[3], m_dwSkill[4], m_dwSkill[5], m_dwSkillPoints);
				return;
			}
			else
			{
				m_pkOwner->SetQuestFlag(flag_read_count, iReadCount + 1);

				switch (number(1, 3))
				{
					case 1:
						m_pkOwner->ChatPacket(CHAT_TYPE_TALKING, LC_TEXT("어느정도 이 기술에 대해 이해가 되었지만 조금 부족한듯 한데.."));
						break;
									
					case 2:
						m_pkOwner->ChatPacket(CHAT_TYPE_TALKING, LC_TEXT("드디어 끝이 보이는 건가...  이 기술은 이해하기가 너무 힘들어.."));
						break;

					case 3:
					default:
						m_pkOwner->ChatPacket(CHAT_TYPE_TALKING, LC_TEXT("열심히 하는 배움을 가지는 것만이 기술을 배울수 있는 유일한 길이다.."));
						break;
				}

				m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("%d 권을 더 읽어야 수련을 완료 할 수 있습니다."), need_bookcount - iReadCount);
				return;
			}
		}

		if(iSkillLevel != m_dwSkill[iSkillID]) {
			m_pkOwner->ChatPacket(CHAT_TYPE_TALKING, LC_TEXT("몸에서 뭔가 힘이 터져 나오는 기분이야!"));
			m_pkOwner->ChatPacket(CHAT_TYPE_TALKING, LC_TEXT("뜨거운 무엇이 계속 용솟음치고 있어! 이건, 이것은!"));
			m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("책으로 더 높은 경지의 수련을 성공적으로 끝내셨습니다."));
		}
		else {
			m_pkOwner->ChatPacket(CHAT_TYPE_TALKING, LC_TEXT("크윽, 기가 역류하고 있어! 이거 설마 주화입마인가!? 젠장!"));
			m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("수련이 실패로 끝났습니다. 다시 도전해주시기 바랍니다."));
		}

		return;
	}
	else
	{
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, "Its not correct book for this skill!");
		return;
	}
}

void CBuffNPCActor::LearnSkillBySoulstone(DWORD dwSkillVnum)
{
	LPITEM item_book = m_pkOwner->GetBuffEquipmentItem(BUFF_SLOT_SKILLBOOK);
	
	if (item_book == NULL) {
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, "ERROR - No Soulstone avaible!");
		return;
	}
	
	if (item_book->GetVnum() == 50513)
	{
		int iSkillID = GetSkillID(dwSkillVnum);
		int iSkillLevel = m_dwSkill[iSkillID];
		int iSkillGrade = GetSkillGrade(iSkillLevel);

		if (m_pkOwner->GetGold() < m_dwLearnGrandmasterSkillCost[m_dwSkill[iSkillID]-30]) {
			m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_NOT_ENOUGHT_MONEY"));
			return;
		}
		
		if(iSkillID == -1 || iSkillGrade == -1)
			return;
			
		if(iSkillGrade != 2)
		{
			if (iSkillGrade > 1)
				m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("퍼펙트 마스터된 스킬입니다. 더 이상 수련 할 수 없습니다."));
			else
				m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("이 스킬은 아직 그랜드 마스터 수련을 할 경지에 이르지 않았습니다."));
			return;
		}

		LPITEM item_exo = m_pkOwner->GetBuffEquipmentItem(BUFF_SLOT_SKILLBOOK_COMMON_1);
		
		if (item_exo == NULL) {
			m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_NEED_EXORZISM_SCROLL"));
			return;
		}

		item_book->SetCount(item_book->GetCount() - 1);
	
		int percent = 20; // 20% without Concentrated Reading

		LPITEM item_read_bonus = m_pkOwner->GetBuffEquipmentItem(BUFF_SLOT_SKILLBOOK_COMMON_2);
		if (item_read_bonus != NULL)
		{
			item_read_bonus->SetCount(item_read_bonus->GetCount() - 1);
			percent = 60;  // 60% with Concentrated Reading
		}
		
		if(number(1, 100) <= percent)
		{
			int iCost = (m_dwLearnGrandmasterSkillCost[m_dwSkill[iSkillID]-30]);
			m_pkOwner->PointChange(POINT_GOLD, -iCost, true);
			m_dwSkill[iSkillID] = this->m_dwSkill[iSkillID] + 1;

			m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillInfo %d %d %d %d %d %d %d", m_dwSkill[0], m_dwSkill[1], m_dwSkill[2], m_dwSkill[3], m_dwSkill[4], m_dwSkill[5], m_dwSkillPoints);
			m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCCreatePopup %d %d %d", 0, 1, dwSkillVnum);
		}
		else
		{
			int iCost = (m_dwLearnGrandmasterSkillCost[m_dwSkill[iSkillID]-30] / 2);
			m_pkOwner->PointChange(POINT_GOLD, -iCost, true);
			m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCCreatePopup %d %d %d", 0, 0, dwSkillVnum);
		}
	}
	else
	{
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, "Its not correct item for this skill!");
		return;
	}
}

void CBuffNPCActor::SetClearSkill(DWORD dwSkillVnum)
{
	LPITEM item_reset = m_pkOwner->GetBuffEquipmentItem(BUFF_SLOT_RESET_SKILL);
	
	if (item_reset == NULL) {
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, "ERROR - No reset skill item avaible!");
		return;
	}
	if (item_reset->GetVnum() == 71003)
	{
		int iSkillID = GetSkillID(dwSkillVnum);
		int iReturnSkillPoints = 0;
		
		item_reset->SetCount(item_reset->GetCount() - 1);
		
		if (m_dwSkill[iSkillID] >= 17)
			iReturnSkillPoints = 17;
		else
			iReturnSkillPoints = m_dwSkill[iSkillID];
		
		m_dwSkillPoints = m_dwSkillPoints + iReturnSkillPoints;
		m_dwSkill[iSkillID] = 0;
		
		m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCSkillInfo %d %d %d %d %d %d %d", m_dwSkill[0], m_dwSkill[1], m_dwSkill[2], m_dwSkill[3], m_dwSkill[4], m_dwSkill[5], m_dwSkillPoints);
		m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCCreatePopup %d %d %d", 5, dwSkillVnum, iReturnSkillPoints);
	}
	else
	{
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, "Its not correct item for this skill!");
		return;
	}
}

void CBuffNPCActor::SetChangeSex()
{
	LPITEM item_sex = m_pkOwner->GetBuffEquipmentItem(BUFF_SLOT_CHANGE_SEX);
	
	if (item_sex == NULL) {
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, "ERROR - No reset skill item avaible!");
		return;
	}
	
	if (!m_pkOwner->IsBuffEquipEmpty()){
		m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_REMOVE_EQUIPMENT_FROM_BUFF"));
		return;
	}
	
	item_sex->SetCount(item_sex->GetCount() - 1);
	
	if (m_dwSex == 0)
		m_dwSex = 1;
	else if (m_dwSex == 1)
		m_dwSex = 0;
	else
		m_dwSex = -1;
	
	m_dwVnum = m_dwSex+10;
	SaveSQL();
	
	m_pkOwner->ChatPacket(CHAT_TYPE_COMMAND, "BuffNPCBasicInfo %s %d %d", m_name.c_str(), m_dwSex, m_dwIntValue);
	if (IsSummoned())
		Unsummon();
	Summon();
}

int CBuffNPCActor::GetSkillID(int iSkillVnum)
{
	for (int i = 0; i < 6; i++)
	{
		if (iSkillVnum == m_dwSkillVnum[i])
			return i;
	}
	return -1;
}

int CBuffNPCActor::GetSkillGrade(int iSkillLevel)
{
	if (iSkillLevel >= 0 && iSkillLevel < 20)
		return 0;
	else if (iSkillLevel >= 20 && iSkillLevel < 30)
		return 1;
	else if (iSkillLevel >= 30 && iSkillLevel < 40) 
		return 2;
	else if (iSkillLevel == 40)
		return 3;
	else
		return -1;
}

bool CBuffNPCActor::Update(DWORD deltaTime)
{
	bool bResult = true;

	DWORD currentTime = get_dword_time();

#ifdef ASLAN_BUFF_NPC_USE_SUMMON_LEVEL_CAP
	if (this->GetLevel() > m_pkOwner->GetLevel()) {
		if ((this->GetLevel() - m_pkOwner->GetLevel()) > ASLAN_BUFF_NPC_MAX_LV_CAP) {
			if(ASLAN_BUFF_NPC_MAX_LV_CAP == 0)
				m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_UNSUMMON_BECAUSE_OVER_YOUR_LEVEL"));
			else
				m_pkOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("ASLAN_BUFF_NPC_UNSUMMON_BECAUSE_OVER_LEVELCAP"), ASLAN_BUFF_NPC_MAX_LV_CAP);
			this->Unsummon();
			m_pkOwner->SetQuestFlag("buff_npc.is_summon", 0);
			return true;
		}
	}
#endif

	if (this->UseSkill())
		return true;

	if (this->IsSummoned() && HasOption(EBuffNPCOption_Followable))
		bResult = bResult && this->_UpdateFollowAI();

	m_dwLastUpdateTime = currentTime;
	
	return bResult;
}

bool CBuffNPCActor::_UpdateFollowAI()
{
	if (0 == m_pkChar->m_pkMobData) {
		return false;
	}

	if (0 == m_originalMoveSpeed)
	{
		const CMob* mobData = CMobManager::Instance().Get(m_dwVnum);

		if (0 != mobData)
			m_originalMoveSpeed = mobData->m_table.sMovingSpeed;
	}
	float	START_FOLLOW_DISTANCE = 400.0f;
	float	START_RUN_DISTANCE = 1200.0f;	

	float	RESPAWN_DISTANCE = 4500.f;		
	int		MIN_APPROACH = 150;
	int		MAX_APPROACH = 300;

	bool bDoMoveAlone = true;
	bool bRun = false;	

	DWORD currentTime = get_dword_time();

	long ownerX = m_pkOwner->GetX();		long ownerY = m_pkOwner->GetY();
	long charX = m_pkChar->GetX();			long charY = m_pkChar->GetY();

	float fDist = DISTANCE_APPROX(charX - ownerX, charY - ownerY);

	if (fDist >= RESPAWN_DISTANCE)
	{
		float fOwnerRot = m_pkOwner->GetRotation() * 3.141592f / 180.f;
		float fx = -MAX_APPROACH * cos(fOwnerRot);
		float fy = -MAX_APPROACH * sin(fOwnerRot);
		if (m_pkChar->Show(m_pkOwner->GetMapIndex(), ownerX + fx, ownerY + fy))
		{
			m_pkChar->UpdatePacket();
			return true;
		}
	}
	
	if (fDist >= START_FOLLOW_DISTANCE)
	{
		if( fDist >= START_RUN_DISTANCE)
			bRun = true;

		m_pkChar->SetNowWalking(!bRun);
		
		if (get_dword_time() - this->GetLastSkillTime() < 4000)
			return false;

		Follow(MAX_APPROACH);

		m_pkChar->SetLastAttacked(currentTime);
		m_dwLastActionTime = currentTime;
	}
	else if (currentTime - m_dwLastActionTime > number(5000, 12000))
	{
		this->_UpdatAloneActionAI();
	}
	else
		m_pkChar->SendMovePacket(FUNC_WAIT, 0, 0, 0, 0);

	return true;
}

bool CBuffNPCActor::Follow(float fMinDistance)
{
	if( !m_pkOwner || !m_pkChar) 
		return false;

	float fOwnerX = m_pkOwner->GetX();
	float fOwnerY = m_pkOwner->GetY();

	float fBuffX = m_pkChar->GetX();
	float fBuffY = m_pkChar->GetY();

	float fDist = DISTANCE_SQRT(fOwnerX - fBuffX, fOwnerY - fBuffY);
	if (fDist <= fMinDistance)
		return false;

	m_pkChar->SetRotationToXY(fOwnerX, fOwnerY);

	float fx, fy;

	float fDistToGo = fDist - fMinDistance;
	GetDeltaByDegree(m_pkChar->GetRotation(), fDistToGo, &fx, &fy);
	
	if (!m_pkChar->Goto((int)(fBuffX+fx+0.5f), (int)(fBuffY+fy+0.5f)) )
		return false;

	m_pkChar->SendMovePacket(FUNC_WAIT, 0, 0, 0, 0, 0);

	return true;
}

bool CBuffNPCActor::_UpdatAloneActionAI()
{
	float fDist = number(500.f, 900.f);
	float r = (float)number (0, 359);
	float dest_x = GetOwner()->GetX() + fDist * cos(r);
	float dest_y = GetOwner()->GetY() + fDist * sin(r);
	
	m_pkChar->SetNowWalking(true);
	
	if (!m_pkChar->IsStateMove() && m_pkChar->Goto(dest_x, dest_y))
		m_pkChar->SendMovePacket(FUNC_WAIT, 0, 0, 0, 0);

	m_dwLastActionTime = get_dword_time();

	return true;
}
#endif