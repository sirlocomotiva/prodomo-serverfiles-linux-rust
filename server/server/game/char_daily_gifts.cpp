/**
* Date : YMD.2020.08.23
* File : char_daily_gifts.cpp
* Author : Owsap
* 0x001
*
* (C) 2020 Owsap Productions
**/

#include "stdafx.h"
#include "utils.h"

#if defined(__DAILY_GIFT_SYSTEM__)
#include "char.h"
#include "questmanager.h"
#include "item.h"
#include "item_manager.h"
#include "desc.h"
#include "db.h"
#include "config.h"

bool b_gsNeedMission = true;
const char* c_szMissionState[2] = { "collect_quest_lv30", "__complete" };

void CHARACTER::LoadDailyGiftWeek()
{
	CheckDailyGiftRenewTime();

	m_bDailyGiftOpen = true;

	if (!m_bDailyGiftLoad)
	{
		m_vecDailyGift.empty();

		BYTE bWeek = GetDailyGiftWeek();
		const CDailyGiftItemGroup* pkGroup = ITEM_MANAGER::instance().GetDailyGiftItemGroup(bWeek);
		if (!pkGroup)
		{
			ChatPacket(CHAT_TYPE_INFO, LC_TEXT("There are no daily gifts for this week."));
			return;
		}

		TPacketGCDailyGift packetGC;
		packetGC.bHeader = HEADER_GC_DAILY_GIFT;
		packetGC.dwCash = GetCash(ERequestCharge_Cash);
		packetGC.bWeek = bWeek;
		for (BYTE bDay = 0; bDay < DAILY_GIFT_WEEK_DAYS; ++bDay)
		{
			DWORD dwCollectTime = GetDailyGiftStartTime() + 60 * 60 * 24 * (bDay + 1);
			packetGC.dwVnum[bDay] = pkGroup->GetVnum(bDay);
			packetGC.dwCount[bDay] = pkGroup->GetCount(bDay);
			packetGC.dwCollectTime[bDay] = dwCollectTime;
			packetGC.wCost[bDay] = pkGroup->GetCost(bDay);
			packetGC.bStatus[bDay] = GetDailyGiftStatus(bDay + 1, dwCollectTime);

			m_vecDailyGift.push_back(packetGC);
		}

		m_vecDailyGift.push_back(packetGC);
		if (!m_vecDailyGift.empty())
			m_bDailyGiftLoad = true;

		GetDesc()->Packet(&packetGC, sizeof(TPacketGCDailyGift));
	}

	if (!HasDailyGiftMission())
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot collect any reward until you complete the required quest. (Reach level 50)"));
		return;
	}
}

void CHARACTER::CloseDailyGift()
{
	m_bDailyGiftOpen = false;
	m_bDailyGiftLoad = false;
	m_vecDailyGift.empty();
}

bool CHARACTER::HasDailyGiftMission()
{
	
	if (GetLevel() >= 50)
		return true;

	return false;
}

void CHARACTER::CheckDailyGiftRenewTime()
{
	if (time(0) > m_dwDailyGiftRenewTime && m_dwDailyGiftRenewTime != 0)
	{
		if (RenewDailyGift())
		{
			DWORD dwCurrentTime = time(0);
			DWORD dwRenewTime = dwCurrentTime + 60 * 60 * 24 * DAILY_GIFT_WEEK_DAYS + 1;
			BYTE bWeek = 1;

			if (DailyGiftOrderWeek())
			{
				BYTE bWeekInc = GetDailyGiftWeek() + 1;
				if (bWeekInc > ITEM_MANAGER::instance().GetAllDailyGiftGroupSize())
					bWeek = 1;
				else
					bWeek = bWeekInc;
			}
			else
			{
				bWeek = number(1, ITEM_MANAGER::instance().GetAllDailyGiftGroupSize());
			}

			quest::CQuestManager::instance().RequestSetEventFlag("daily_gift_week", bWeek);
			quest::CQuestManager::instance().RequestSetEventFlag("daily_gift_start_time", dwCurrentTime);
			quest::CQuestManager::instance().RequestSetEventFlag("daily_gift_renew_time", dwRenewTime);
			m_dwDailyGiftRenewTime = dwRenewTime;
		}
		else
		{
			quest::CQuestManager::instance().RequestSetEventFlag("daily_gift_event", 0);
			quest::CQuestManager::instance().RequestSetEventFlag("daily_gift_week", 0);
			quest::CQuestManager::instance().RequestSetEventFlag("daily_gift_start_time", 0);
			quest::CQuestManager::instance().RequestSetEventFlag("daily_gift_renew_time", 0);
			quest::CQuestManager::instance().RequestSetEventFlag("daily_gift_renew", 0);
			quest::CQuestManager::instance().RequestSetEventFlag("daily_gift_order", 0);
			m_dwDailyGiftRenewTime = 0;
		}
	}
}

void CHARACTER::CollectDailyGift(BYTE bSlotIndex, bool bUseTicket)
{
	if (!m_bDailyGiftOpen && !m_bDailyGiftLoad)
		return;

	if (!CheckDailyGiftStatus())
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The event is no longer active."));
		return;
	}

	if (!HasDailyGiftMission())
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot collect any reward until you complete the required quest. (Reach level 50)"));
		return;
	}

	bool bCollect = false;
	BYTE bDay = bSlotIndex + 1;
	BYTE bWeek = GetDailyGiftWeek();

	const CDailyGiftItemGroup* pkGroup = ITEM_MANAGER::instance().GetDailyGiftItemGroup(bWeek);
	if (!pkGroup)
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("There are no daily gifts for this week."));
		return;
	}

	switch (GetDailyGiftStatus(bDay))
	{
	case DAILY_GIFT_COLLECT_LOCK:
	{
		DWORD dwCost = pkGroup->GetCost(bSlotIndex);

		if (GetCash(ERequestCharge_Cash) >= dwCost && !bUseTicket)
		{
			int iEmptyPos = GetEmptyInventory(1);
			if (iEmptyPos != -1)
			{
				SetCash(ERequestCharge_Cash, -dwCost);

#ifdef __WJ_PICKUP_ITEM_EFFECT__
				AutoGiveItem(pkGroup->GetVnum(bSlotIndex), pkGroup->GetCount(bSlotIndex), -1, false, false);
#else
				AutoGiveItem(pkGroup->GetVnum(bSlotIndex), pkGroup->GetCount(bSlotIndex), -1, false);
#endif
				bCollect = true;
			}
			else
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;1444]");
				return;
			}
		}
		else
		{
			if (bUseTicket)
			{
				int iEmptyPos = GetEmptyInventory(1);
				if (iEmptyPos != -1)
				{
					LPITEM pTicketItem = FindSpecifyItem(DAILY_GIFT_TICKET_ITEM);
					if (pTicketItem->GetCount() <= 0)
					{
						ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You don't have the ticket."));
						return;
					}
					RemoveSpecifyItem(DAILY_GIFT_TICKET_ITEM, 1);

#ifdef __WJ_PICKUP_ITEM_EFFECT__
					AutoGiveItem(pkGroup->GetVnum(bSlotIndex), pkGroup->GetCount(bSlotIndex), -1, false, false);
#else
					AutoGiveItem(pkGroup->GetVnum(bSlotIndex), pkGroup->GetCount(bSlotIndex), -1, false);
#endif
					bCollect = true;
				}
				else
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;1444]");
					return;
				}
			}
			else
			{
				ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Unfortunately you don't have enough cash."));
				return;
			}
		}
	}
	break;

	case DAILY_GIFT_COLLECT_WAIT:
	{
		int iEmptyPos = GetEmptyInventory(1);
		if (iEmptyPos != -1)
		{
#ifdef __WJ_PICKUP_ITEM_EFFECT__
			AutoGiveItem(pkGroup->GetVnum(bSlotIndex), pkGroup->GetCount(bSlotIndex), -1, false, false);
#else
			AutoGiveItem(pkGroup->GetVnum(bSlotIndex), pkGroup->GetCount(bSlotIndex), -1, false);
#endif
			bCollect = true;
		}
		else
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;1444]");
			return;
		}
	}
	break;

	case DAILY_GIFT_COLLECT:
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You have already recieved this reward."));
		return;
	}
	break;

	}

	if (bCollect)
	{
		char szQueryUpdate[QUERY_MAX_LEN];
		snprintf(szQueryUpdate, sizeof(szQueryUpdate), "UPDATE daily_gift SET day%d = %d WHERE player_id = %u;",
			bDay, DAILY_GIFT_COLLECT_SUCCESS, GetPlayerID());
		std::unique_ptr<SQLMsg> pUpdate(DBManager::instance().DirectQuery(szQueryUpdate));
	}

	m_bDailyGiftLoad = false;
	LoadDailyGiftWeek();
}

BYTE CHARACTER::GetDailyGiftStatus(BYTE bDay, DWORD dwCollectTime)
{
	if (!m_bDailyGiftOpen)
		return DAILY_GIFT_COLLECT_LOCK;

	if (!CheckDailyGiftStatus())
		return DAILY_GIFT_COLLECT_LOCK;

	if (!HasDailyGiftMission())
		return DAILY_GIFT_COLLECT_LOCK;

	char szQuery[QUERY_MAX_LEN], szQueryUpdate[QUERY_MAX_LEN], szQueryInsert[QUERY_MAX_LEN];
	snprintf(szQuery, sizeof(szQuery), "SELECT day%d FROM daily_gift WHERE player_id = %u AND week = %d AND UNIX_TIMESTAMP(datetime) > %u",
		bDay, GetPlayerID(), GetDailyGiftWeek(), GetDailyGiftStartTime());
	std::unique_ptr<SQLMsg> pMsg(DBManager::instance().DirectQuery(szQuery));

	BYTE bStatus = DAILY_GIFT_COLLECT_LOCK;
	bool bUpdate = false;

	if (pMsg->Get()->uiNumRows > 0)
	{
		MYSQL_ROW row = mysql_fetch_row(pMsg->Get()->pSQLResult);
		str_to_number(bStatus, row[0]);

		bUpdate = true;
	}
	else
	{
		snprintf(szQueryInsert, sizeof(szQueryInsert), "REPLACE INTO daily_gift VALUES(%u, %d, 1, 0, 0, 0, 0, 0, 0, FROM_UNIXTIME(%u));", GetPlayerID(), GetDailyGiftWeek(), GetDailyGiftRenewTime());
		std::unique_ptr<SQLMsg> pInsert(DBManager::instance().DirectQuery(szQueryInsert));

		if (bDay == 1)
			bStatus = DAILY_GIFT_COLLECT_WAIT;
	}

	if ((time(0) > dwCollectTime && dwCollectTime != 0) && bStatus != DAILY_GIFT_COLLECT_SUCCESS)
	{
		if (bUpdate)
		{
			snprintf(szQueryUpdate, sizeof(szQueryUpdate), "UPDATE daily_gift SET day%d = %d WHERE player_id = %d AND week = %d;",
				bDay, DAILY_GIFT_COLLECT_WAIT, GetPlayerID(), GetDailyGiftWeek());
			std::unique_ptr<SQLMsg> pUpdate(DBManager::instance().DirectQuery(szQueryUpdate));
		}

		bStatus = DAILY_GIFT_COLLECT_WAIT;
	}

	if (test_server)
	{
		time_t tCollectTime = dwCollectTime;
		ChatPacket(CHAT_TYPE_INFO, "[TEST] You can collect day %d at %s", bDay, ctime(&tCollectTime));
	}

	return bStatus;
}

DWORD CHARACTER::GetCash(BYTE bType)
{
	if (!GetDesc())
		return 0;

	DWORD dwAID = GetAID();
	DWORD dwAmount = 0;

	if (dwAID == 0)
		return 0;

	char szQuery[QUERY_MAX_LEN];
	switch (bType)
	{
	case ERequestCharge_Cash:
		snprintf(szQuery, sizeof(szQuery), "SELECT cash FROM account.account WHERE id = %u", dwAID);
		break;
	case ERequestCharge_Mileage:
		snprintf(szQuery, sizeof(szQuery), "SELECT mileage FROM account.account WHERE id = %u", dwAID);
	}
	SQLMsg* pMsg = DBManager::instance().DirectQuery(szQuery);
	if (pMsg != NULL)
	{
		if (pMsg->Get()->uiNumRows > 0)
		{
			MYSQL_ROW row = mysql_fetch_row(pMsg->Get()->pSQLResult);
			str_to_number(dwAmount, row[0]);
		}
		M2_DELETE(pMsg);
	}
	return dwAmount;
}

bool CHARACTER::SetCash(BYTE bType, DWORD dwAmount)
{
	if (!GetDesc())
		return false;

	DWORD dwAID = GetAID();
	if (dwAID == 0)
		return false;

	char szQuery[QUERY_MAX_LEN];
	switch (bType)
	{
	case ERequestCharge_Cash:
		snprintf(szQuery, sizeof(szQuery), "UPDATE account.account SET cash = cash + %d WHERE id = %u LIMIT 1", dwAmount, dwAID);
		break;
	case ERequestCharge_Mileage:
		snprintf(szQuery, sizeof(szQuery), "UPDATE account.account SET mileage = mileage + %d WHERE id = %u LIMIT 1", dwAmount, dwAID);
		break;
	}
	std::unique_ptr<SQLMsg> msg(DBManager::instance().DirectQuery(szQuery));
	if (msg->Get()->uiAffectedRows > 0)
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You have unlocked the item successfully."));
		return true;
	}

	return false;
}

BYTE CHARACTER::CheckDailyGiftStatus()
{
	BYTE bEventStatus = quest::CQuestManager::instance().GetEventFlag("daily_gift_event");
	if (bEventStatus > 0)
		return 1;
	return 0;
}

DWORD CHARACTER::GetDailyGiftStartTime()
{
	DWORD dwStartTime = quest::CQuestManager::instance().GetEventFlag("daily_gift_start_time");
	if (dwStartTime > 0)
		return dwStartTime;
	return 0;
}

DWORD CHARACTER::GetDailyGiftRenewTime()
{
	DWORD dwRenewTime = quest::CQuestManager::instance().GetEventFlag("daily_gift_renew_time");
	if (dwRenewTime > 0)
		return dwRenewTime;
	return 0;
}

BYTE CHARACTER::GetDailyGiftWeek()
{
	BYTE bWeek = quest::CQuestManager::instance().GetEventFlag("daily_gift_week");
	if (bWeek > 0)
		return bWeek;
	return 0;
}

BYTE CHARACTER::RenewDailyGift()
{
	BYTE bRenew = quest::CQuestManager::instance().GetEventFlag("daily_gift_renew");
	if (bRenew > 0)
		return 1;
	return 0;
}

BYTE CHARACTER::DailyGiftOrderWeek()
{
	BYTE bOrder = quest::CQuestManager::instance().GetEventFlag("daily_gift_order");
	if (bOrder > 0)
		return 1;
	return 0;
}
#endif
