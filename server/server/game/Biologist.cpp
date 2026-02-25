#include "stdafx.h"
#ifdef __ENABLE_BIOLOGIST_RENEWAL_SYSTEM__
#include "Biologist.h"
#include "char.h"
#include "packet.h"
#include "item.h"
#include "desc.h"
#include "config.h"
#include "questmanager.h"
#include "utils.h"

/* Mission title */
const char* arr_title[] = {"Dinþii de Orc", "Cãrþile Blestemate", "Suvenirul Demonului", "Globurile de Gheaþã", "Crengile Zelkova", "Tãbliþele lui Tugyi", "Ramurile Roºii", "Însemnele Liderilor", "Bijuterie Rea Vointã", "Bijuterie Înþelepciune"};

/* Mission level */
int arr_level_limit[] = {30, 40, 50, 60, 70, 80, 85, 90, 92, 94};

/* Mission items required */
int arr_item_req[] = {30006, 30047, 30015, 30050, 30165, 30166, 30167, 30168, 30251, 30252};

/* Mission items required count */
int arr_item_req_count[] = {10, 15, 15, 20, 25, 30, 40, 50, 10, 20};

/* Mision object to increase percentage chanse */
int arr_item_perc[] = {71035, 60, 100};

/* Mision object to forget the mission time */
int arr_item_forget_item = 72350;

/* Mission items reward */
int arr_item_reward[] = {50109, 50110, 50111, 50112, 50113, 50114, 50115, 50114, 71107, 71105};

/* Mission timeout */
int arr_timeout[] = {1800, 2700, 3600, 4500, 5400, 7200, 9000, 10800, 12600, 14400}; // to do time

/* Mission bonus reward */
int arr_bonus[10][8] = {
	{APPLY_MOV_SPEED, 10}, // Misiune Dinti Orc 30
	{APPLY_ATT_SPEED, 8}, // Misiune Carti 40
	{APPLY_DEF_GRADE_BONUS, 60}, // Misiune Suvenir 50
	{APPLY_ATT_GRADE_BONUS, 50}, // Misiune Globuri 60
	{APPLY_ATTBONUS_MONSTER, 5}, // Misiune Crengi 70
	{APPLY_ATT_SPEED, 8, APPLY_MALL_ATTBONUS, 10}, // Misiune Tablite 80
	{POINT_RESIST_WARRIOR, 10, POINT_RESIST_ASSASSIN, 10, POINT_RESIST_SURA, 10, POINT_RESIST_SHAMAN, 10}, // Misiune Ramuri Rosii
	{APPLY_ATTBONUS_MONSTER, 8}, // Însemnele Liderilor
	{APPLY_MAX_HP, 1000, APPLY_ATT_GRADE_BONUS, 50, APPLY_DEF_GRADE_BONUS, 120}, // Bijuterie Rea Vointa
	{APPLY_MAX_HP, 1100, APPLY_ATT_GRADE_BONUS, 60, APPLY_DEF_GRADE_BONUS, 140} // Bijuterie Intelepciune
};

int arr_affect[] = {AFFECT_BIOLOGIST_1, AFFECT_BIOLOGIST_2, AFFECT_BIOLOGIST_3, AFFECT_BIOLOGIST_4, AFFECT_BIOLOGIST_5, AFFECT_BIOLOGIST_6, AFFECT_BIOLOGIST_7, AFFECT_BIOLOGIST_8, AFFECT_BIOLOGIST_9, AFFECT_BIOLOGIST_10};

/* Mission is selective affect */
bool arr_is_selective_affect[] = {false, false, false, false, false, false, false, false, true, true};

/* Mission decrease time */
int arr_item_flower[2][2] = {
	{30169, 15*60},
	{30170, 45*60},
};

CBiologist::CBiologist() {}
CBiologist::~CBiologist() {}

void CBiologist::BiologistOpenPacket(LPCHARACTER pkChar)
{
	if (IsCompleted(pkChar))
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Misiunile Biologului Chaegirab au fost finalizate în totalitate!");
		return;
	}
	
	if (pkChar->GetBiologistState() > BiologistConfig::MAX_MISSION_STATE)
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Nu existã nicio informaþie pentru acest stadiu al Biologului Chaegirab. Contactaþi un membru din echipã noastrã!");
		return;
	}
	
	if (pkChar->GetLevel() < arr_level_limit[pkChar->GetBiologistState()])
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Atinge nivelul %d pentru a putea începe acest stadiu al Biologului Chaegirab!", arr_level_limit[pkChar->GetBiologistState()]);
		return;
	}

	TPacketGCBiologist kSendBiologistPacket;
	kSendBiologistPacket.byHeader = HEADER_GC_BIOLOGIST;
	kSendBiologistPacket.bySubHeader = BIOLOGIST_SUBHEADER_GC_OPEN;
	kSendBiologistPacket.bIsSelective = IsSelectiveAffect(pkChar) ? true : false;
	memcpy(kSendBiologistPacket.szTitle, arr_title[pkChar->GetBiologistState()], 64);
	kSendBiologistPacket.dwLevelLimit = arr_level_limit[pkChar->GetBiologistState()];
	kSendBiologistPacket.dwItemReq = arr_item_req[pkChar->GetBiologistState()];
	kSendBiologistPacket.dwItemReqCount = arr_item_req_count[pkChar->GetBiologistState()];
	kSendBiologistPacket.dwItemIncreaseRate = arr_item_perc[0];
	kSendBiologistPacket.dwItemForgetTime = arr_item_forget_item;
	kSendBiologistPacket.dwRewardBonusVnum[0] = arr_bonus[pkChar->GetBiologistState()][0];
	kSendBiologistPacket.dwRewardBonusVnum[1] = arr_bonus[pkChar->GetBiologistState()][2];
	kSendBiologistPacket.dwRewardBonusVnum[2] = arr_bonus[pkChar->GetBiologistState()][4];
	kSendBiologistPacket.dwRewardBonusValue[0] = arr_bonus[pkChar->GetBiologistState()][1];
	kSendBiologistPacket.dwRewardBonusValue[1] = arr_bonus[pkChar->GetBiologistState()][3];
	kSendBiologistPacket.dwRewardBonusValue[2] = arr_bonus[pkChar->GetBiologistState()][5];
	kSendBiologistPacket.iTime = GetTimeOut(pkChar);
	pkChar->GetDesc()->Packet(&kSendBiologistPacket, sizeof(TPacketGCBiologist));
}

void CBiologist::BiologistProvidesMaterialPacket(LPCHARACTER pkChar, BYTE byIsElixirUse, BYTE byIsBookTimeUse)
{
	if (IsCompleted(pkChar))
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Misiunile Biologului Chaegirab au fost finalizate în totalitate!");
		return;
	}
	
	if (pkChar->GetBiologistState() > BiologistConfig::MAX_MISSION_STATE)
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Nu existã nicio informaþie pentru acest stadiu al Biologului Chaegirab. Contactaþi un membru din echipã noastrã!");
		return;
	}

	if (pkChar->GetLevel() < arr_level_limit[pkChar->GetBiologistState()])
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Atinge nivelul %d pentru a putea începe acest stadiu al Biologului Chaegirab!", arr_level_limit[pkChar->GetBiologistState()]);
		return;
	}
	
	if (GetTimeOut(pkChar) > 0)
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Încã prelucrez ultimul obiect oferit! Te rog sã mai aºtepþi o vreme!");
		return;
	}
	
	if (IsSelectiveAffect(pkChar))
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Alege unul dintre bonusurile din lista de recompensã!");
		return;
	}
	
	if (pkChar->GetBiologistItemsTaken() >= arr_item_req_count[pkChar->GetBiologistState()])
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r OverFlow objects. Contactaþi un membru din echipa noastrã!");
		return;
	}
	
	DWORD dwMaterialNeeded = arr_item_req[pkChar->GetBiologistState()];
	if (pkChar->CountSpecifyItem(dwMaterialNeeded) <= 0)
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Nu ai niciun material de oferit Biologului Chaegirab!");
		return;
	}
	
	DWORD dwSuccessRate = arr_item_perc[1];
	DWORD dwRandom = number(1, 100);
	
	if (byIsElixirUse)
	{
		if (pkChar->CountSpecifyItem(arr_item_perc[0]) > 0)
		{
			pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Elixirul Exploatãrii a fost folositã pentru a vã oferi o ºansã perfectã de oferire a obiectului!");
			dwSuccessRate = arr_item_perc[2];
			pkChar->RemoveSpecifyItem(arr_item_perc[0], 1);
		}
		else
		{
			pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Elixirul Exploatãrii nu a putut fi gãsit în inventarul tãu!");
		}
	}
		
	
	if (dwRandom <= dwSuccessRate)
	{
		pkChar->ChatPacket(CHAT_TYPE_COMMAND, "BINARY_BiologistProvidesMaterialSuccesfully");
		
		pkChar->RemoveSpecifyItem(dwMaterialNeeded, 1);
		pkChar->PointChange(POINT_BIOLOGIST_ITEMS_TAKEN, 1, true);
	}
	else
	{
		pkChar->ChatPacket(CHAT_TYPE_COMMAND, "BINARY_BiologistProvidesMaterialFailed");
		pkChar->RemoveSpecifyItem(dwMaterialNeeded, 1);
	}
	
	SetTimeOut(pkChar, arr_timeout[pkChar->GetBiologistState()]);
	if (pkChar->GetBiologistItemsTaken() == arr_item_req_count[pkChar->GetBiologistState()])
	{
		if (IsSelectiveAffect(pkChar))
		{
			SetTimeOut(pkChar, 0);
		}
		else
		{
			pkChar->AutoGiveItem(arr_item_reward[pkChar->GetBiologistState()]);
			pkChar->AddAffect(arr_affect[pkChar->GetBiologistState()], aApplyInfo[arr_bonus[pkChar->GetBiologistState()][0]].bPointType, arr_bonus[pkChar->GetBiologistState()][1], 0, 60*60*60*365, 0, false);
			if (arr_bonus[pkChar->GetBiologistState()][3] > 0)
				pkChar->AddAffect(arr_affect[pkChar->GetBiologistState()], aApplyInfo[arr_bonus[pkChar->GetBiologistState()][2]].bPointType, arr_bonus[pkChar->GetBiologistState()][3], 0, 60*60*60*365, 0, false);
			if (arr_bonus[pkChar->GetBiologistState()][5] > 0)
				pkChar->AddAffect(arr_affect[pkChar->GetBiologistState()], aApplyInfo[arr_bonus[pkChar->GetBiologistState()][4]].bPointType, arr_bonus[pkChar->GetBiologistState()][5], 0, 60*60*60*365, 0, false);
			
			pkChar->PointChange(POINT_BIOLOGIST_STATE, 1, true);
			pkChar->PointChange(POINT_BIOLOGIST_ITEMS_TAKEN, -pkChar->GetBiologistItemsTaken(), true);
			SetTimeOut(pkChar, 0);
		}
	}
	
	if (byIsBookTimeUse)
	{
		if (pkChar->CountSpecifyItem(arr_item_forget_item) > 0)
		{
			pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Cartea Uitãrii a fost folositã pentru e elimina urmãtoarea limitã de timp!");
			SetTimeOut(pkChar, 0);
			pkChar->RemoveSpecifyItem(arr_item_forget_item, 1);
		}
		else
		{
			pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Cartea Uitãrii nu a putut fi gãsitã în inventarul tãu!");
		}
	}
	
	TPacketGCBiologist kSendBiologistPacket;
	kSendBiologistPacket.byHeader = HEADER_GC_BIOLOGIST;
	kSendBiologistPacket.bySubHeader = BIOLOGIST_SUBHEADER_GC_PROVIDES;
	kSendBiologistPacket.bIsSelective = IsSelectiveAffect(pkChar) ? true : false;
	memcpy(kSendBiologistPacket.szTitle, arr_title[pkChar->GetBiologistState()], 64);
	kSendBiologistPacket.dwLevelLimit = arr_level_limit[pkChar->GetBiologistState()];
	kSendBiologistPacket.dwItemReq = arr_item_req[pkChar->GetBiologistState()];
	kSendBiologistPacket.dwItemReqCount = arr_item_req_count[pkChar->GetBiologistState()];
	kSendBiologistPacket.dwItemIncreaseRate = arr_item_perc[0];
	kSendBiologistPacket.dwItemForgetTime = arr_item_forget_item;
	kSendBiologistPacket.dwRewardBonusVnum[0] = arr_bonus[pkChar->GetBiologistState()][0];
	kSendBiologistPacket.dwRewardBonusVnum[1] = arr_bonus[pkChar->GetBiologistState()][2];
	kSendBiologistPacket.dwRewardBonusVnum[2] = arr_bonus[pkChar->GetBiologistState()][4];
	kSendBiologistPacket.dwRewardBonusValue[0] = arr_bonus[pkChar->GetBiologistState()][1];
	kSendBiologistPacket.dwRewardBonusValue[1] = arr_bonus[pkChar->GetBiologistState()][3];
	kSendBiologistPacket.dwRewardBonusValue[2] = arr_bonus[pkChar->GetBiologistState()][5];
	kSendBiologistPacket.iTime = GetTimeOut(pkChar);
	pkChar->GetDesc()->Packet(&kSendBiologistPacket, sizeof(TPacketGCBiologist));
}

void CBiologist::BiologistChosenAffectPacket(LPCHARACTER pkChar, BYTE byChosenAffect)
{
	BYTE byIndex;
	if (byChosenAffect == 0)
		byIndex = 0;
	else if (byChosenAffect == 1)
		byIndex = 2;
	else if (byChosenAffect == 2)
		byIndex = 4;
	
	if (IsCompleted(pkChar))
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Misiunile Biologului Chaegirab au fost finalizate în totalitate!");
		return;
	}

	if (pkChar->GetBiologistState() > BiologistConfig::MAX_MISSION_STATE)
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Nu existã nicio informaþie pentru acest stadiu al Biologului Chaegirab. Contactaþi un membru din echipã noastrã!");
		return;
	}
	
	if (pkChar->GetLevel() < arr_level_limit[pkChar->GetBiologistState()])
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Atinge nivelul %d pentru a putea începe acest stadiu al Biologului Chaegirab!", arr_level_limit[pkChar->GetBiologistState()]);
		return;
	}
	
	if (pkChar->GetBiologistState() < BiologistConfig::MAX_MISSION_STATE)
		pkChar->PointChange(POINT_BIOLOGIST_ITEMS_TAKEN, -pkChar->GetBiologistItemsTaken(), true);
	
	pkChar->AutoGiveItem(arr_item_reward[pkChar->GetBiologistState()]);
	pkChar->AddAffect(arr_affect[pkChar->GetBiologistState()], aApplyInfo[arr_bonus[pkChar->GetBiologistState()][byIndex]].bPointType, arr_bonus[pkChar->GetBiologistState()][byIndex+1], 0, 60*60*60*365, 0, false);
	
	pkChar->PointChange(POINT_BIOLOGIST_STATE, 1, true);
	SetTimeOut(pkChar, 0);

	if ((pkChar->GetBiologistItemsTaken() == arr_item_req_count[pkChar->GetBiologistState()]) && (pkChar->GetBiologistState() == BiologistConfig::MAX_MISSION_STATE))
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Toate misiune au fost completate cu succes! Felicitãri!");
		pkChar->SetBiologistCompleted(1);
		SendBiologistClosePacket(pkChar);
		return;
	}
	
	TPacketGCBiologist kSendBiologistPacket;
	kSendBiologistPacket.byHeader = HEADER_GC_BIOLOGIST;
	kSendBiologistPacket.bySubHeader = BIOLOGIST_SUBHEADER_GC_PROVIDES;
	kSendBiologistPacket.bIsSelective = IsSelectiveAffect(pkChar) ? true : false;
	memcpy(kSendBiologistPacket.szTitle, arr_title[pkChar->GetBiologistState()], 64);
	kSendBiologistPacket.dwLevelLimit = arr_level_limit[pkChar->GetBiologistState()];
	kSendBiologistPacket.dwItemReq = arr_item_req[pkChar->GetBiologistState()];
	kSendBiologistPacket.dwItemReqCount = arr_item_req_count[pkChar->GetBiologistState()];
	kSendBiologistPacket.dwItemIncreaseRate = arr_item_perc[0];
	kSendBiologistPacket.dwItemForgetTime = arr_item_forget_item;
	kSendBiologistPacket.dwRewardBonusVnum[0] = arr_bonus[pkChar->GetBiologistState()][0];
	kSendBiologistPacket.dwRewardBonusVnum[1] = arr_bonus[pkChar->GetBiologistState()][2];
	kSendBiologistPacket.dwRewardBonusVnum[2] = arr_bonus[pkChar->GetBiologistState()][4];
	kSendBiologistPacket.dwRewardBonusValue[0] = arr_bonus[pkChar->GetBiologistState()][1];
	kSendBiologistPacket.dwRewardBonusValue[1] = arr_bonus[pkChar->GetBiologistState()][3];
	kSendBiologistPacket.dwRewardBonusValue[2] = arr_bonus[pkChar->GetBiologistState()][5];
	kSendBiologistPacket.iTime = GetTimeOut(pkChar);
	pkChar->GetDesc()->Packet(&kSendBiologistPacket, sizeof(TPacketGCBiologist));
}

void CBiologist::BiologistDecreaseTime(LPCHARACTER pkChar, BYTE byDecreaseTimeIndex)
{
	if (pkChar->CountSpecifyItem(arr_item_flower[byDecreaseTimeIndex][0]) <= 0)
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Ai nevoie de o floare pentru a putea reduce timpul de aºteptare!");
		return;
	}
	
	if (GetTimeOut(pkChar) <= 0)
	{
		pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Nu existã nicio limitã de timp. Urmãtorul obiect poate fi oferit!");
		return;
	}
	
	int iDecreaseTime = GetTimeOut(pkChar) - arr_item_flower[byDecreaseTimeIndex][1];
	
	SetTimeOut(pkChar, iDecreaseTime);
	
	pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r Timpul de aºteptare a fost redus cu %d de minute!", arr_item_flower[byDecreaseTimeIndex][1] / 60);
	pkChar->RemoveSpecifyItem(arr_item_flower[byDecreaseTimeIndex][0]);
	
	TPacketGCBiologist kSendBiologistPacket;
	kSendBiologistPacket.byHeader = HEADER_GC_BIOLOGIST;
	kSendBiologistPacket.bySubHeader = BIOLOGIST_SUBHEADER_GC_TIME;
	kSendBiologistPacket.bIsSelective = IsSelectiveAffect(pkChar) ? true : false;
	memcpy(kSendBiologistPacket.szTitle, arr_title[pkChar->GetBiologistState()], 64);
	kSendBiologistPacket.dwLevelLimit = arr_level_limit[pkChar->GetBiologistState()];
	kSendBiologistPacket.dwItemReq = arr_item_req[pkChar->GetBiologistState()];
	kSendBiologistPacket.dwItemReqCount = arr_item_req_count[pkChar->GetBiologistState()];
	kSendBiologistPacket.dwItemIncreaseRate = arr_item_perc[0];
	kSendBiologistPacket.dwItemForgetTime = arr_item_forget_item;
	kSendBiologistPacket.dwRewardBonusVnum[0] = arr_bonus[pkChar->GetBiologistState()][0];
	kSendBiologistPacket.dwRewardBonusVnum[1] = arr_bonus[pkChar->GetBiologistState()][2];
	kSendBiologistPacket.dwRewardBonusVnum[2] = arr_bonus[pkChar->GetBiologistState()][4];
	kSendBiologistPacket.dwRewardBonusValue[0] = arr_bonus[pkChar->GetBiologistState()][1];
	kSendBiologistPacket.dwRewardBonusValue[1] = arr_bonus[pkChar->GetBiologistState()][3];
	kSendBiologistPacket.dwRewardBonusValue[2] = arr_bonus[pkChar->GetBiologistState()][5];
	kSendBiologistPacket.iTime = GetTimeOut(pkChar);
	pkChar->GetDesc()->Packet(&kSendBiologistPacket, sizeof(TPacketGCBiologist));
}

void CBiologist::SendBiologistClosePacket(LPCHARACTER pkChar)
{	
	TPacketGCBiologist kSendBiologistPacket;
	kSendBiologistPacket.byHeader = HEADER_GC_BIOLOGIST;
	kSendBiologistPacket.bySubHeader = BIOLOGIST_SUBHEADER_GC_CLOSE;
	kSendBiologistPacket.bIsSelective = false;
	memcpy(kSendBiologistPacket.szTitle, "", 64);
	kSendBiologistPacket.dwItemReq = 0;
	kSendBiologistPacket.dwItemReqCount = 0;
	kSendBiologistPacket.dwItemIncreaseRate = 0;
	kSendBiologistPacket.dwItemForgetTime = 0;
	kSendBiologistPacket.dwRewardBonusVnum[0] = 0;
	kSendBiologistPacket.dwRewardBonusVnum[1] = 0;
	kSendBiologistPacket.dwRewardBonusVnum[2] = 0;
	kSendBiologistPacket.dwRewardBonusValue[0] = 0;
	kSendBiologistPacket.dwRewardBonusValue[1] = 0;
	kSendBiologistPacket.dwRewardBonusValue[2] = 0;
	kSendBiologistPacket.iTime = 0;
	pkChar->GetDesc()->Packet(&kSendBiologistPacket, sizeof(TPacketGCBiologist));
}

bool CBiologist::IsSelectiveAffect(LPCHARACTER pkChar)
{
	if (pkChar->GetBiologistItemsTaken() >= arr_item_req_count[pkChar->GetBiologistState()] && arr_is_selective_affect[pkChar->GetBiologistState()])
		return true;
	
	return false;
}

bool CBiologist::IsCompleted(LPCHARACTER pkChar)
{
	if (pkChar->GetBiologistCompleted() == 1)
		return true;
	
	return false;
}

void CBiologist::SetTimeOut(LPCHARACTER pkChar, int iTimeOut)
{
	quest::PC* pPC = quest::CQuestManager::instance().GetPC(pkChar->GetPlayerID());
	pPC->SetFlag("biologist.timeout", get_global_time() + iTimeOut);
}

int CBiologist::GetTimeOut(LPCHARACTER pkChar) const
{
	quest::PC* pPC = quest::CQuestManager::instance().GetPC(pkChar->GetPlayerID());
	int iTimeOut = pPC->GetFlag("biologist.timeout") - get_global_time();
	
	return iTimeOut;
}

void CBiologist::DropObjects(LPCHARACTER pkChar, LPCHARACTER pkVictim, DWORD pkVictimVnum)
{
	if (!pkChar || !pkVictim)
		return;
	
	if (pkChar->GetBiologistState() < 0 && pkChar->GetBiologistState() > BiologistConfig::MAX_MISSION_STATE)
		return;
		
	const DWORD dwArrMonsterList[][13] = {
		{601, 636, 656},
		{706, 756},
		{1001, 1002, 1003, 1004},
		{1107, 1105},
		{2302, 2303, 2304},
		{1401, 1601, 1602},
		{2313, 2314, 2315, 2311, 2312},
		{691, 2091, 2191, 794, 1901, 2206, 1304, 1093, 1191, 2492, 2495, 2493, 2598},
		{1137, 1135},
		{2402, 2403}
	};
	const int dwArrDropChance[] = {
		quest::CQuestManager::instance().GetEventFlag("biologist_lv_30_drop_chance"),
		quest::CQuestManager::instance().GetEventFlag("biologist_lv_40_drop_chance"), 
		quest::CQuestManager::instance().GetEventFlag("biologist_lv_50_drop_chance"), 
		quest::CQuestManager::instance().GetEventFlag("biologist_lv_60_drop_chance"), 
		quest::CQuestManager::instance().GetEventFlag("biologist_lv_70_drop_chance"), 
		quest::CQuestManager::instance().GetEventFlag("biologist_lv_80_drop_chance"), 
		quest::CQuestManager::instance().GetEventFlag("biologist_lv_85_drop_chance"), 
		quest::CQuestManager::instance().GetEventFlag("biologist_lv_90_drop_chance"), 
		quest::CQuestManager::instance().GetEventFlag("biologist_lv_92_drop_chance"), 
		quest::CQuestManager::instance().GetEventFlag("biologist_lv_94_drop_chance")
	};
	
	for (const auto& vnum : dwArrMonsterList[pkChar->GetBiologistState()])
	{
		if (vnum == 0)
			continue;

		if (pkVictimVnum == vnum)
		{
			if (pkChar->GetLevel() >= (arr_level_limit[pkChar->GetBiologistState()] + BiologistConfig::DROP_OBJECTS_LV_DIF))
				return;
			
			if (number(1, 100) <= dwArrDropChance[pkChar->GetBiologistState()])
			{
				pkChar->ChatPacket(CHAT_TYPE_INFO, "|cff808000[Biologist]|h|r 1x |cfff5b042%s|h|r a fost gãsit!", arr_title[pkChar->GetBiologistState()]);
				pkChar->AutoGiveItem(arr_item_req[pkChar->GetBiologistState()]);
			}
		}
	}
}
#endif


























