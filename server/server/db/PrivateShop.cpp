#include "stdafx.h"
#include "PrivateShop.h"
#include "ClientManager.h"
#include "Cache.h"
#include "Main.h"
#include "../libgame/grid.h"

WORD GlobalToLocalPos(WORD wPos)
{
	return wPos - (wPos / PRIVATE_SHOP_PAGE_ITEM_MAX_NUM) * PRIVATE_SHOP_PAGE_ITEM_MAX_NUM;
}

CPrivateShop::CPrivateShop()
{
	m_dwShopPeerHandle = 0;
	m_dwOwnerPeerHandle = 0;
	m_dwOwnerHandle = 0;
	memset(&m_table, 0, sizeof(TPrivateShop));
	m_vec_privateShopItem.clear();

	for (int i = 0; i < PRIVATE_SHOP_PAGE_MAX_NUM; ++i)
		m_pGrid[i] = new CGrid(PRIVATE_SHOP_WIDTH, PRIVATE_SHOP_HEIGHT);
}

CPrivateShop::~CPrivateShop()
{
	for (int i = 0; i < PRIVATE_SHOP_PAGE_MAX_NUM; ++i)
		delete m_pGrid[i];
}

void CPrivateShop::UpdatePremiumTime(time_t tPremiumTime)
{
	SetPremiumTime(tPremiumTime);

	if (auto pShopCache = CClientManager::Instance().GetPrivateShopCache(GetOwner()))
	{
		TPrivateShop* pPrivateShopTable = pShopCache->Get();
		pPrivateShopTable->tPremiumTime = GetPremiumTime();
		pShopCache->Put(pPrivateShopTable);
	}
	else
	{
		char szQuery[128];
		snprintf(szQuery, sizeof(szQuery), "UPDATE private_shop%s SET premium_time=%u WHERE owner_id=%u", GetTablePostfix(), GetPremiumTime(), GetOwner());
		CDBManager::instance().AsyncQuery(szQuery);
	}
}

void CPrivateShop::SetUnlockedSlots(WORD wUnlockedSlots)
{
#ifdef ENABLE_PRIVATE_SHOP_LOCKED_SLOTS
	// Reset as we don't want a sum of previous unlocked slots
	WORD wStartPos = PRIVATE_SHOP_HOST_ITEM_MAX_NUM - PRIVATE_SHOP_LOCKED_SLOT_MAX_NUM;

	for (WORD i = wStartPos; i < PRIVATE_SHOP_HOST_ITEM_MAX_NUM; ++i)
	{
		CGrid* pGrid = GetGridByPosition(i);
		WORD wGridPos = GlobalToLocalPos(i);

		if (i < (wStartPos + wUnlockedSlots))
			pGrid->Get(wGridPos, 1, 1);
		else
			pGrid->Put(wGridPos, 1, 1);
	}
#endif

	m_table.wUnlockedSlots = wUnlockedSlots;
}

void CPrivateShop::IncreaseUnlockedSlots(WORD wUnlockedSlots)
{
#ifdef ENABLE_PRIVATE_SHOP_LOCKED_SLOTS
	WORD wStartPos = PRIVATE_SHOP_HOST_ITEM_MAX_NUM - PRIVATE_SHOP_LOCKED_SLOT_MAX_NUM + GetUnlockedSlots();

	for (WORD i = wStartPos; i < PRIVATE_SHOP_HOST_ITEM_MAX_NUM; ++i)
	{
		CGrid* pGrid = GetGridByPosition(i);
		WORD wGridPos = GlobalToLocalPos(i);

		if (i < (wStartPos + wUnlockedSlots))
			pGrid->Get(wGridPos, 1, 1);
		else
			pGrid->Put(wGridPos, 1, 1);
	}
#endif

	m_table.wUnlockedSlots += wUnlockedSlots;

	if (auto pShopCache = CClientManager::Instance().GetPrivateShopCache(GetOwner()))
	{
		TPrivateShop* pPrivateShopTable = pShopCache->Get();
		pPrivateShopTable->wUnlockedSlots = GetUnlockedSlots();
		pShopCache->Put(pPrivateShopTable);
	}
	else
	{
		char szQuery[128];
		snprintf(szQuery, sizeof(szQuery), "UPDATE private_shop%s SET unlocked_slots=%u WHERE owner_id=%u", GetTablePostfix(), GetUnlockedSlots(), GetOwner());
		CDBManager::instance().AsyncQuery(szQuery);
	}
}

void CPrivateShop::RemoveAllItem()
{
	for (int i = 0; i < PRIVATE_SHOP_PAGE_MAX_NUM; ++i)
		m_pGrid[i]->Clear();

	WORD wStartPos = PRIVATE_SHOP_HOST_ITEM_MAX_NUM - PRIVATE_SHOP_LOCKED_SLOT_MAX_NUM + GetUnlockedSlots();

	for (WORD i = wStartPos; i < PRIVATE_SHOP_HOST_ITEM_MAX_NUM; ++i)
	{
		CGrid* pGrid = GetGridByPosition(i);
		WORD wGridPos = GlobalToLocalPos(i);

		if (i < wStartPos)
			pGrid->Get(wGridPos, 1, 1);
		else
			pGrid->Put(wGridPos, 1, 1);
	}

	m_vec_privateShopItem.clear();
}

void CPrivateShop::ChangeTitle(const char* c_szTitle)
{
	memcpy(m_table.szTitle, c_szTitle, sizeof(m_table.szTitle));

	if (auto pShopCache = CClientManager::Instance().GetPrivateShopCache(GetOwner()))
	{
		TPrivateShop* pPrivateShopTable = pShopCache->Get();
		strlcpy(pPrivateShopTable->szTitle, c_szTitle, sizeof(pPrivateShopTable->szTitle));
		pShopCache->Put(pPrivateShopTable);
	}
	else
	{
		char szQuery[128];
		snprintf(szQuery, sizeof(szQuery), "UPDATE private_shop%s SET title='%s' WHERE owner_id=%u", GetTablePostfix(), c_szTitle, GetOwner());
		CDBManager::instance().AsyncQuery(szQuery);
	}
}

void CPrivateShop::ChangeState(BYTE bState)
{
	SetState(bState);

	if (auto pShopCache = CClientManager::Instance().GetPrivateShopCache(GetOwner()))
	{
		TPrivateShop* pPrivateShopTable = pShopCache->Get();
		pPrivateShopTable->bState = GetState();

		// Not really needed to flush state update?
		// pShopCache->Put(pPrivateShopTable);
	}
	else
	{
		char szQuery[128];
		snprintf(szQuery, sizeof(szQuery), "UPDATE private_shop%s SET state=%u WHERE owner_id=%u", GetTablePostfix(), GetState(), GetOwner());
		CDBManager::instance().AsyncQuery(szQuery);
	}
}

TPrivateShopItemInfo* CPrivateShop::SetItem(const TPlayerPrivateShopItem* pItem, TItemTable* pItemProto)
{
	if (pItem->wPos >= PRIVATE_SHOP_HOST_ITEM_MAX_NUM)
	{
		sys_err("Move checkin pos overflow %u private shop %u", pItem->wPos, GetOwner());
		return nullptr;
	}

	// Select grid based on item's position
	CGrid* pGrid = GetGridByPosition(pItem->wPos);

	WORD wGridPos = GlobalToLocalPos(pItem->wPos);
	if (!pGrid->IsEmpty(wGridPos, 1, pItemProto->bSize))
	{
		sys_log(0, "PRIVATE_SHOP: Cannot set item at pos %u private shop %u", pItem->wPos, GetOwner());
		return nullptr;
	}

	pGrid->Put(wGridPos, 1, pItemProto->bSize);

	TPrivateShopItemInfo rItemInfo {};
	memcpy(&rItemInfo.TItem, pItem, sizeof(TPlayerPrivateShopItem));
	rItemInfo.pItemProto = pItemProto;
	rItemInfo.bAvailable = true;

	m_vec_privateShopItem.emplace_back(rItemInfo);

	// bro?
	return &m_vec_privateShopItem.at(m_vec_privateShopItem.size() - 1);
}

void CPrivateShop::RemoveItem(WORD wPos)
{
	if (wPos >= PRIVATE_SHOP_HOST_ITEM_MAX_NUM)
	{
		sys_err("Remove item pos overflow %u private shop %u", wPos, GetOwner());
		return;
	}

	TPrivateShopItemInfo* pItem = GetItem(wPos);
	if (!pItem)
	{
		sys_err("Cannot find item at pos %u private shop %u", wPos, GetOwner());
		return;
	}

	// Remove it from the grid
	CGrid* pGrid = GetGridByPosition(wPos);
	WORD wGridPos = GlobalToLocalPos(wPos);
	pGrid->Get(wGridPos, 1, pItem->pItemProto->bSize);

	// Remove the item from SQL record
	if (!CClientManager::Instance().DeletePrivateShopItemCache(pItem->TItem.dwID))
	{
		char szQuery[64];
		snprintf(szQuery, sizeof(szQuery), "DELETE FROM private_shop_item%s WHERE id=%u", GetTablePostfix(), pItem->TItem.dwID);
		CDBManager::instance().AsyncQuery(szQuery);
	}

	// Remove item from vector
	for (auto it = m_vec_privateShopItem.begin(); it != m_vec_privateShopItem.end(); ++it)
	{
		if (it->TItem.wPos == wPos)
		{
			m_vec_privateShopItem.erase(it);
			return;
		}
	}
}

void CPrivateShop::RemoveItemByID(DWORD dwID)
{
	int iPos = -1;

	for (const auto& rItem : m_vec_privateShopItem)
	{
		if (rItem.TItem.dwID == dwID)
		{
			iPos = rItem.TItem.wPos;
			break;
		}
	}

	if (iPos < 0)
	{
		sys_err("Cannot find item with id %u in private shop %u", dwID, GetOwner());
		return;
	}

	RemoveItem(iPos);
}

TPrivateShopItemInfo* CPrivateShop::GetItem(WORD wPos)
{
	for (auto& item : m_vec_privateShopItem)
	{
		if (item.TItem.wPos == wPos)
			return &item;
	}

	return nullptr;
}

WORD CPrivateShop::GetItemCount()
{
	return m_vec_privateShopItem.size();
}

CGrid* CPrivateShop::GetGridByPosition(WORD wPos)
{
	BYTE bPage = wPos / PRIVATE_SHOP_PAGE_ITEM_MAX_NUM;
	if (bPage >= PRIVATE_SHOP_PAGE_MAX_NUM)
		return nullptr;

	return m_pGrid[bPage];
}

CGrid* CPrivateShop::GetGridByPage(BYTE bPage)
{
	if (bPage >= PRIVATE_SHOP_PAGE_MAX_NUM)
		return nullptr;

	return m_pGrid[bPage];
}

bool CPrivateShop::Buy(WORD wPos)
{
	TPrivateShopItemInfo* pItemInfo = GetItem(wPos);
	if (!pItemInfo)
		return false;

	UpdateBalance(pItemInfo->TItem.TPrice.llGold, pItemInfo->TItem.TPrice.dwCheque);
	return true;
}

void CPrivateShop::UpdateBalance(long long llGold, DWORD dwCheque)
{
	ChangeGold(llGold);
	ChangeCheque(dwCheque);

	if (auto pShopCache = CClientManager::Instance().GetPrivateShopCache(GetOwner()))
	{
		TPrivateShop* pPrivateShopTable = pShopCache->Get();
		pPrivateShopTable->llGold += llGold;
		pPrivateShopTable->dwCheque += dwCheque;

		pShopCache->Put(pPrivateShopTable);
	}
	else
	{
		char szQuery[128];
		snprintf(szQuery, sizeof(szQuery), "UPDATE private_shop%s SET gold=%lld, cheque=%u WHERE owner_id=%u", GetTablePostfix(), GetGold(), GetCheque(), GetOwner());
		CDBManager::instance().AsyncQuery(szQuery);
	}
}

void CPrivateShop::Withdraw()
{
	m_table.llGold = 0;
	m_table.dwCheque = 0;

	if (auto pShopCache = CClientManager::Instance().GetPrivateShopCache(GetOwner()))
	{
		TPrivateShop* pPrivateShopTable = pShopCache->Get();
		pPrivateShopTable->llGold = 0;
		pPrivateShopTable->dwCheque = 0;

		pShopCache->Put(pPrivateShopTable);
	}
	else
	{
		char szQuery[128];
		snprintf(szQuery, sizeof(szQuery), "UPDATE private_shop%s SET gold=0, cheque=0 WHERE owner_id=%u", GetTablePostfix(), GetOwner());
		CDBManager::instance().AsyncQuery(szQuery);
	}
}

bool CPrivateShop::ChangeItemPrice(WORD wPos, long long llGold, DWORD dwCheque)
{
	TPrivateShopItemInfo* pItemInfo = GetItem(wPos);
	if (!pItemInfo)
		return false;

	pItemInfo->TItem.TPrice.llGold = llGold;
	pItemInfo->TItem.TPrice.dwCheque = dwCheque;

	if (auto pPrivateShopItemCache = CClientManager::Instance().GetPrivateShopItemCache(pItemInfo->TItem.dwID))
	{
		TPlayerPrivateShopItem* pItemTable = pPrivateShopItemCache->Get();
		pItemTable->TPrice.llGold = llGold;
		pItemTable->TPrice.dwCheque = dwCheque;

		pPrivateShopItemCache->Put(pItemTable);
	}
	else
	{
		char szQuery[128];
		snprintf(szQuery, sizeof(szQuery), "UPDATE private_shop_item%s SET gold=%lld, cheque=%u WHERE id=%u", GetTablePostfix(), llGold, dwCheque, pItemInfo->TItem.dwID);
		CDBManager::instance().AsyncQuery(szQuery);
	}

	return true;
}

TPrivateShopItemInfo* CPrivateShop::MoveItem(WORD wPos, WORD wChangePos)
{
	TPrivateShopItemInfo* pItemInfo = GetItem(wPos);
	if (!pItemInfo)
	{
		sys_err("Cannot find item at pos %u private shop %u", wPos, GetOwner());
		return nullptr;
	}

	if (wChangePos >= PRIVATE_SHOP_HOST_ITEM_MAX_NUM)
	{
		sys_err("Move item pos overflow %u private shop %u", wChangePos, GetOwner());
		return nullptr;
	}

	// Create a fake scenario without the item being in the old position
	CGrid* pOldGrid = GetGridByPosition(wPos);
	WORD wOldGridPos = GlobalToLocalPos(wPos);

	pOldGrid->Get(wOldGridPos, 1, pItemInfo->pItemProto->bSize);

	// Try to move the item to a new position
	CGrid* pNewGrid = GetGridByPosition(wChangePos);
	WORD wGridPos = GlobalToLocalPos(wChangePos);

	if (!pNewGrid->IsEmpty(wGridPos, 1, pItemInfo->pItemProto->bSize))
	{
		// Restore previous grid's state
		pOldGrid->Put(wOldGridPos, 1, pItemInfo->pItemProto->bSize);

		sys_log(0, "PRIVATE_SHOP: Cannot set item at pos %u private shop %u", wChangePos, GetOwner());
		return nullptr;
	}

	// Update item's position and fill the grid
	pNewGrid->Put(wGridPos, 1, pItemInfo->pItemProto->bSize);
	pItemInfo->TItem.wPos = wChangePos;

	if (auto pPrivateShopItemCache = CClientManager::Instance().GetPrivateShopItemCache(pItemInfo->TItem.dwID))
	{
		TPlayerPrivateShopItem* pItemTable = pPrivateShopItemCache->Get();
		pItemTable->wPos = wChangePos;

		pPrivateShopItemCache->Put(pItemTable);
	}
	else
	{
		char szQuery[128]{};
		snprintf(szQuery, sizeof(szQuery), "UPDATE private_shop_item%s SET pos=%u WHERE id=%u", GetTablePostfix(), wChangePos, pItemInfo->TItem.dwID);
		CDBManager::instance().AsyncQuery(szQuery);
	}

	return pItemInfo;
}

void CPrivateShop::SetSale(TPrivateShopSale& rSale)
{
	// Create cache set if the owner is offline, it will be populated later on upon owner's login
	if (!CClientManager::Instance().GetPrivateShopSaleCacheSet(GetOwner()))
		CClientManager::Instance().CreatePrivateShopSaleCacheSet(GetOwner());

	CClientManager::Instance().PutPrivateShopSaleCache(&rSale);

	CClientManager::Instance().AddMarketItemPrice(rSale);
}
