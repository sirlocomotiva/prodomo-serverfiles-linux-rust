#include "stdafx.h"
#include "private_shop_manager.h"
#include "private_shop.h"
#include "private_shop_util.h"
#include "char.h"
#include "char_manager.h"
#include "desc_client.h"
#include "mob_manager.h"
#include "config.h"
#include "db.h"
#include "item_manager.h"
#include "item.h"
#include "utils.h"
#include "entity.h"
#include "sectree_manager.h"
#include "p2p.h"
#include "buffer_manager.h"
#include "desc_manager.h"
#include "p2p.h"
#include "DragonSoul.h"
#include "log.h"

void CPrivateShopManager::Destroy()
{
	m_map_privateShop.clear();
}

LPPRIVATE_SHOP CPrivateShopManager::CreatePrivateShop(DWORD dwPID)
{
	if (GetPrivateShop(dwPID))
		return nullptr;

	std::unique_ptr<CPrivateShop> upPrivateShop = std::make_unique<CPrivateShop>();
	LPPRIVATE_SHOP pPrivateShop = upPrivateShop.get();

	DWORD dwVID = AllocVID();
	upPrivateShop->SetVID(dwVID);

	m_map_privateShop.emplace(dwPID, std::move(upPrivateShop));
	m_map_privateShopVID.emplace(dwVID, pPrivateShop);

	return pPrivateShop;
}

LPPRIVATE_SHOP CPrivateShopManager::GetPrivateShop(DWORD dwPID)
{
	auto it = m_map_privateShop.find(dwPID);
	if (it == m_map_privateShop.end())
		return nullptr;

	return it->second.get();
}

LPPRIVATE_SHOP CPrivateShopManager::GetPrivateShopByOwnerName(const char* c_szOwnerName)
{
	for (auto it = m_map_privateShop.begin(); it != m_map_privateShop.end(); ++it)
	{
		LPPRIVATE_SHOP pPrivateShop = it->second.get();

		if (pPrivateShop->GetOwnerName().compare(c_szOwnerName) == 0)
			return pPrivateShop;
	}

	return nullptr;
}

LPPRIVATE_SHOP CPrivateShopManager::GetPrivateShopByVID(DWORD dwVID)
{
	auto it = m_map_privateShopVID.find(dwVID);
	if (it == m_map_privateShopVID.end())
		return nullptr;

	return it->second;
}

bool CPrivateShopManager::DeletePrivateShop(DWORD dwPID)
{
	auto it = m_map_privateShop.find(dwPID);
	if (it == m_map_privateShop.end())
		return false;

	DWORD dwVID = it->second->GetVID();

	m_map_privateShopVID.erase(dwVID);
	m_map_privateShop.erase(it);
	return true;
}

bool CPrivateShopManager::BuildPrivateShop(LPCHARACTER pShopOwner, const char* c_szTitle, DWORD dwPolyVnum, BYTE bTitleType, BYTE bPageCount, const std::vector<TPrivateShopItem>& c_vec_shopItem)
{
	const CMob* pMobTable = CMobManager::Instance().Get(dwPolyVnum);
	if (!pMobTable)
	{
		sys_err("Cannot find mob table for vnum %u", dwPolyVnum);
		return false;
	}

	LPPRIVATE_SHOP pPrivateShop = CreatePrivateShop(pShopOwner->GetPlayerID());
	if (!pPrivateShop)
	{
		sys_err("Cannot create a private shop instance for player %s %u", pShopOwner->GetName(), pShopOwner->GetPlayerID());
		return false;
	}

	// Create private shop item instances
	std::vector<TPlayerPrivateShopItem> vec_privateShopItem;
	for (const auto& c_rShopItem : c_vec_shopItem)
	{
		LPITEM pItem = pShopOwner->GetItem(c_rShopItem.TPos);

		TPlayerPrivateShopItem t{};

		CopyItemData(pItem, t);

		t.TPrice.llGold = c_rShopItem.TPrice.llGold;
		t.TPrice.dwCheque = c_rShopItem.TPrice.dwCheque;
		t.wPos = c_rShopItem.wDisplayPos;
		t.tCheckin = time(0);

		pItem->Lock(true);

		vec_privateShopItem.push_back(t);
	}

	std::string strTitle(c_szTitle);

	pPrivateShop->SetID(pShopOwner->GetPlayerID());
	pPrivateShop->SetVnum(dwPolyVnum);
	pPrivateShop->SetOwnerName(pShopOwner->GetName());
	pPrivateShop->SetTitle(c_szTitle);
	pPrivateShop->SetTitleType(bTitleType);
	pPrivateShop->SetState(STATE_OPEN);
	pPrivateShop->SetPageCount(bPageCount);
	pPrivateShop->SetUnlockedSlots(pShopOwner->GetPoint(POINT_PRIVATE_SHOP_UNLOCKED_SLOT));

	// Transfer items to the private shop
	if (!pPrivateShop->Initialize(vec_privateShopItem))
	{
		sys_err("Cannot initialize items to private shop");
		DeletePrivateShop(pShopOwner->GetPlayerID());

		return false;
	}

	// Flush delayed item data as we don't want it flushed later on after (if) its saved in private shop
	const auto& itemContainer = pPrivateShop->GetItemContainer();
	for (const auto& kv : itemContainer)
		ITEM_MANAGER::Instance().FlushDelayedSave(kv.second);

	// Send data to db and wait for result
	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_CREATE;
	WORD wCount = vec_privateShopItem.size();

	// Fill the shop's table with values
	TPrivateShop t{};

	t.dwOwner = pShopOwner->GetPlayerID();
	strlcpy(t.szTitle, c_szTitle, sizeof(t.szTitle));
	strlcpy(t.szOwnerName, pShopOwner->GetName(), sizeof(t.szOwnerName));
	t.dwVnum = dwPolyVnum;
	t.bTitleType = bTitleType;
	t.lMapIndex = pShopOwner->GetMapIndex();
	t.lX = pShopOwner->GetX();
	t.lY = pShopOwner->GetY();
	t.bChannel = g_bChannel;
	t.wPort = mother_port;
	t.bState = STATE_OPEN;
	t.llGold = 0;
	t.dwCheque = 0;
	t.bPageCount = bPageCount;
	t.tPremiumTime = pShopOwner->GetPremiumRemainSeconds(PREMIUM_PRIVATE_SHOP) + time(0);
	t.wUnlockedSlots = pShopOwner->GetPoint(POINT_PRIVATE_SHOP_UNLOCKED_SLOT);

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, pShopOwner->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(TPrivateShop) + sizeof(WORD) + sizeof(TPlayerPrivateShopItem) * wCount);
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&t, sizeof(TPrivateShop));
	db_clientdesc->Packet(&wCount, sizeof(WORD));
	db_clientdesc->Packet(&vec_privateShopItem[0], sizeof(TPlayerPrivateShopItem) * wCount);

	return true;
}

void CPrivateShopManager::BuildPrivateShopResult(DWORD dwPID, TPrivateShop* pPrivateShopTable, bool bSuccess)
{
	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(dwPID);
	if (!pPrivateShop)
	{
		sys_err("Cannot find private shop with id %u", dwPID);
		return;
	}

	LPCHARACTER pOwner = CHARACTER_MANAGER::Instance().FindByPID(dwPID);
	if (!pOwner)
	{
		sys_err("Cannot find private shop owner with id %u", dwPID);
		return;
	}

	pOwner->ClosePrivateShopPanel(true);

	// If creation failed, return items back to the player
	// otherwise transfer them to the private shop
	const auto& map_privateShopItem = pPrivateShop->GetItemContainer();
	for (const auto& kv : map_privateShopItem)
	{
		WORD wPos = kv.first;
		LPITEM pItem = kv.second;

		if (bSuccess)
		{
			pItem->Lock(false);
			// Turn off saving of the item
			pItem->SetSkipSave(true);

			// If previous owner was a player, remove it from his inventory
			pItem->RemoveFromCharacter();
			pOwner->SyncQuickslot(QUICKSLOT_TYPE_ITEM, pItem->GetCell(), 255);

			// Item still holds original inventory position in case there was a failure
			pItem->SetCell(nullptr, wPos);
			pItem->BindPrivateShop(pPrivateShop);

			TPlayerPrivateShopItem t{};
			CopyItemData(pItem, t);

			pOwner->SetPrivateShopItem(t);

			char szHint[128 + 1]{};
			snprintf(szHint, sizeof(szHint), "%s x%u pos %u gold %lld cheque %u",
				pItem->GetName(), pItem->GetCount(), t.wPos, t.TPrice.llGold, t.TPrice.dwCheque);
			LogManager::Instance().ItemLog(pOwner, pItem, "PRIVATE SHOP BUILD CHECKIN", szHint);
		}
		else
		{
			pItem->BindPrivateShop(nullptr);
			pItem->SetCheckinTime(0);
			pItem->SetGoldPrice(0);
			pItem->SetChequePrice(0);
			pItem->Lock(false);
		}
	}

	if (!bSuccess)
	{
		DeletePrivateShop(dwPID);
		pOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot build a personal shop at this moment. "));

		sys_err("Private shop building process stopped for player %u", dwPID);
		return;
	}

	pPrivateShop->Show(pPrivateShopTable->lX, pPrivateShopTable->lY, 0, pPrivateShopTable->lMapIndex);
	pOwner->SetPrivateShopTable(*pPrivateShopTable);


	LogManager::Instance().CharLog(pOwner, 0, "PRIVATE SHOP BUILT", "");

	sys_log(0, "%s PRIVATE_SHOP: SUCCESS Shop entity created", pPrivateShopTable->szOwnerName);
}

void CPrivateShopManager::ClosePrivateShop(LPCHARACTER pShopOwner)
{
	DWORD dwPID = pShopOwner->GetPlayerID();

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(dwPID);
	if (!pPrivateShop)
	{
		pShopOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You are too far away from your personal shop."));
		return;
	}

	// Cancel if the player is not near his shop character
	long lShopMapIndex = pPrivateShop->GetMapIndex();
	if (pShopOwner->GetMapIndex() != lShopMapIndex)
	{
		pShopOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot close a personal shop from a different location."));
		return;
	}

	if (DISTANCE_APPROX(pPrivateShop->GetX() - pShopOwner->GetX(), pPrivateShop->GetY() - pShopOwner->GetY()) > 3000)
	{
		pShopOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You are too far away from your personal shop."));
		return;
	}

	pPrivateShop->SetClosing(true);
	if (pPrivateShop->TransferItems(pShopOwner))
	{
		// Close the window at customer's client
		pPrivateShop->CleanShopViewers();

		BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_DELETE;

		db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(DWORD));
		db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
		db_clientdesc->Packet(&dwPID, sizeof(DWORD));

		DeletePrivateShop(dwPID);

		pShopOwner->ClosePrivateShop();

		sys_log(0, "%s PRIVATE_SHOP: SUCCESS Shop entity closed", pShopOwner->GetName());
		LogManager::Instance().CharLog(pShopOwner, 0, "PRIVATE SHOP CLOSED", "");
	}
	else
	{
		sys_err("Failed to transfer items back to owner %u", pShopOwner->GetPlayerID());
		pPrivateShop->SetClosing(false);
	}
}

void CPrivateShopManager::SpawnPrivateShop(TPrivateShop* pPrivateShopTable, const std::vector<TPlayerPrivateShopItem>& c_vec_shopItem)
{
	const CMob* pMobTable = CMobManager::Instance().Get(pPrivateShopTable->dwVnum);
	if (!pMobTable)
	{
		sys_err("Cannot find mob table for vnum %u", pPrivateShopTable->dwVnum);
		return;
	}

	if (GetPrivateShop(pPrivateShopTable->dwOwner))
		DeletePrivateShop(pPrivateShopTable->dwOwner);

	LPPRIVATE_SHOP pPrivateShop = CreatePrivateShop(pPrivateShopTable->dwOwner);
	if (!pPrivateShop)
	{
		sys_err("Cannot create a private shop instance for player %s %u", pPrivateShopTable->szOwnerName, pPrivateShopTable->dwOwner);
		return;
	}

	LPSECTREE pSectree = SECTREE_MANAGER::instance().Get(pPrivateShopTable->lMapIndex, pPrivateShopTable->lX, pPrivateShopTable->lY);

	if (!pSectree)
	{
		sys_err("Cannot find sectree %dx%d mapindex %d for private shop %s %d", pPrivateShopTable->lY, pPrivateShopTable->lY, pPrivateShopTable->lMapIndex,
			pPrivateShopTable->szOwnerName, pPrivateShopTable->dwOwner);
		return;
	}

	if (pPrivateShopTable->bState < STATE_OPEN)
	{
		sys_err("Cannot spawn a private shop %u with state %u", pPrivateShopTable->dwOwner, pPrivateShopTable->bState);
		return;
	}

	// Bind data to the private shop
	pPrivateShop->SetID(pPrivateShopTable->dwOwner);
	pPrivateShop->SetVnum(pPrivateShopTable->dwVnum);
	pPrivateShop->SetOwnerName(pPrivateShopTable->szOwnerName);
	pPrivateShop->SetTitle(pPrivateShopTable->szTitle);
	pPrivateShop->SetTitleType(pPrivateShopTable->bTitleType);
	pPrivateShop->SetState(pPrivateShopTable->bState);
	pPrivateShop->SetPageCount(pPrivateShopTable->bPageCount);
	pPrivateShop->SetUnlockedSlots(pPrivateShopTable->wUnlockedSlots);

	// Create items and transfer them to the private shop
	if (!pPrivateShop->Initialize(c_vec_shopItem, true))
	{
		sys_err("Cannot initialize items to private shop");
		DespawnPrivateShop(pPrivateShopTable->dwOwner);

		sys_log(0, "%s PRIVATE_SHOP: FAILURE Shop entity not spawned", pPrivateShopTable->szOwnerName);
		return;
	}

	pPrivateShop->Show(pPrivateShopTable->lX, pPrivateShopTable->lY, 0, pPrivateShopTable->lMapIndex);

	sys_log(0, "%s PRIVATE_SHOP: SUCCESS Shop entity spawned", pPrivateShopTable->szOwnerName);

	//LogManager::Instance().CharLog(pPrivateShopTable->dwOwner, pPrivateShopTable->lX, pPrivateShopTable->lY, 0, "PRIVATE SHOP SPAWN", "", "", );
}

void CPrivateShopManager::DespawnPrivateShop(DWORD dwPID)
{
	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_DESPAWN;
	DWORD dwOwner = dwPID;

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(DWORD));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&dwOwner, sizeof(DWORD));

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(dwPID);
	if (!pPrivateShop)
	{
		sys_err("Cannot find private shop with id %u", dwPID);
		return;
	}

	sys_log(0, "%s PRIVATE_SHOP: Shop entity despawned", pPrivateShop->GetOwnerName().c_str());
	DeletePrivateShop(dwPID);
}

bool CPrivateShopManager::StopShopping(LPCHARACTER pShopViewer)
{
	LPPRIVATE_SHOP pPrivateShop = pShopViewer->GetViewingPrivateShop();
	if (!pPrivateShop)
		return false;

	pPrivateShop->RemoveShopViewer(pShopViewer);
	pShopViewer->SetMyShopTime();

	return true;
}

void CPrivateShopManager::ItemCheckin(LPCHARACTER pOwner, const TPlayerPrivateShopItem* c_pShopItem)
{
	if (db_clientdesc->GetSocket() == INVALID_SOCKET)
		return;

	LPITEM pItem = ITEM_MANAGER::Instance().Find(c_pShopItem->dwID);
	if (!pItem)
	{
		sys_err("Cannot checkin item %u vnum %u for private shop %u (item not found)", c_pShopItem->dwID, c_pShopItem->dwVnum, pOwner->GetPlayerID());
		return;
	}

	if (!pItem->GetOwner())
	{
		sys_err("Cannot checkin item %u vnum %u for private shop %u (owner not found)", c_pShopItem->dwID, c_pShopItem->dwVnum, pOwner->GetPlayerID());
		return;
	}

	if (pItem->GetOwner() != pOwner)
	{
		sys_err("Cannot checkin item %u vnum %u for private shop %u (owner not equal to private shop owner)", c_pShopItem->dwID, c_pShopItem->dwVnum, pOwner->GetPlayerID());
		return;
	}

	pOwner->SetPrivateShopItem(*c_pShopItem);

	// Send check-in update back to db
	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_ITEM_CHECKIN_UPDATE;
	DWORD dwPID = pOwner->GetPlayerID();

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, pOwner->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(TPlayerPrivateShopItem));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(c_pShopItem, sizeof(TPlayerPrivateShopItem));

	char szHint[128 + 1]{};
	snprintf(szHint, sizeof(szHint), "%s x%u pos %u gold %lld cheque %u",
		pItem->GetName(), pItem->GetCount(), c_pShopItem->wPos, c_pShopItem->TPrice.llGold, c_pShopItem->TPrice.dwCheque);
	LogManager::Instance().ItemLog(pItem->GetOwner(), pItem, "PRIVATE SHOP CHECKIN", szHint);

	// Do not save the removal of item, will be done later on
	pItem->SetSkipSave(true);
	ITEM_MANAGER::Instance().RemoveItem(pItem);
}

void CPrivateShopManager::ItemCheckout(LPCHARACTER pOwner, WORD wSrcPos, TItemPos TDstPos)
{
	if (db_clientdesc->GetSocket() == INVALID_SOCKET)
		return;

	const TPlayerPrivateShopItem* c_pShopItem = pOwner->GetPrivateShopItem(wSrcPos);

	if (!c_pShopItem)
	{
		sys_err("cannot find private shop item at pos %u player %u", wSrcPos, pOwner->GetPlayerID());
		return;
	}

	LPITEM pItem = ITEM_MANAGER::Instance().Find(c_pShopItem->dwID);

	if (!pItem)
	{
		pItem = ITEM_MANAGER::Instance().CreateItem(c_pShopItem->dwVnum, c_pShopItem->dwCount, c_pShopItem->dwID);
		if (!pItem)
		{
			sys_err("cannot create item by vnum %u (id %u)", c_pShopItem->dwVnum, c_pShopItem->dwID);
			return;
		}

		pItem->OnAfterCreatedItem();
		CopyItemData(*c_pShopItem, pItem);
	}

	// Try to move the item to the owner
	if (!pItem->AddToCharacter(pOwner, TItemPos(TDstPos.window_type, TDstPos.cell)))
	{
		sys_err("Cannot checkout item %u vnum %u from private shop %u", c_pShopItem->dwID, c_pShopItem->dwVnum, pOwner->GetPlayerID());

		// If item had no private shop bound it was re-created and can be removed
		if (!pItem->GetPrivateShop())
			M2_DESTROY_ITEM(pItem);

		return;
	}

	// Item was successfully transfered to owner, reset private shop values
	pItem->BindPrivateShop(nullptr);
	pItem->SetGoldPrice(0);
	pItem->SetChequePrice(0);

	// Enable saving of the item
	pItem->SetSkipSave(false);

	// Remove the private shop item
	pOwner->RemovePrivateShopItem(wSrcPos);

	// Send check-out update back to db
	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_ITEM_CHECKOUT_UPDATE;
	DWORD dwPID = pOwner->GetPlayerID();

	TPlayerItem TItem {};
	CopyItemData(pItem, TItem);

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, pOwner->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(WORD) + sizeof(TPlayerItem));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&wSrcPos, sizeof(WORD));
	db_clientdesc->Packet(&TItem, sizeof(TPlayerItem));

	char szHint[128 + 1]{};
	snprintf(szHint, sizeof(szHint), "%s x%u", pItem->GetName(), pItem->GetCount());
	LogManager::Instance().ItemLog(pItem->GetOwner(), pItem, "PRIVATE SHOP CHECKOUT", szHint);
}

bool CPrivateShopManager::ItemTransaction(LPCHARACTER pCustomer, TPlayerPrivateShopItem* c_pShopItem)
{
	if (db_clientdesc->GetSocket() == INVALID_SOCKET)
		return false;

	for (int i = 0; i < SELECTED_ITEM_MAX_NUM; ++i, ++c_pShopItem)
	{
		if (!c_pShopItem->dwOwner)
			break;

		LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(c_pShopItem->dwOwner);

		// Check if the item already exists, if not create it
		LPITEM pItem = ITEM_MANAGER::Instance().Find(c_pShopItem->dwID);

		if (pPrivateShop && !pItem)
			sys_err("No item data found for item %u vnum %u on private shop %u", c_pShopItem->dwID, c_pShopItem->dwVnum, c_pShopItem->dwOwner);

		bool bNewItem = false;
		if (!pItem)
		{
			pItem = ITEM_MANAGER::Instance().CreateItem(c_pShopItem->dwVnum, c_pShopItem->dwCount, c_pShopItem->dwID);
			if (!pItem)
			{
				sys_err("Cannot create item %u vnum %u", c_pShopItem->dwID, c_pShopItem->dwVnum);

				SendItemTransactionFailedResult(c_pShopItem->dwOwner, c_pShopItem->wPos);
				continue;
			}

			CopyItemData(*c_pShopItem, pItem);
			bNewItem = true;
		}

		// Check if there is enough space for the item
		int iPos = GetEmptyInventory(pCustomer, pItem);

		if (iPos < 0)
		{
			sys_err("Cannot find empty cell for item %u vnum %u", c_pShopItem->dwID, c_pShopItem->dwVnum);

			pCustomer->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You don't have enough space in your inventory."));
			SendItemTransactionFailedResult(c_pShopItem->dwOwner, c_pShopItem->wPos);

			if (bNewItem)
				M2_DESTROY_ITEM(pItem);

			continue;
		}

		// Was already checked on db?
		if (pCustomer->GetGold() < c_pShopItem->TPrice.llGold)
		{
			sys_log(0, "PRIVATE_SHOP: Insufficient gold at customer %s to buy item %u vnum %u", pCustomer->GetName(), c_pShopItem->dwID, c_pShopItem->dwVnum);

			pCustomer->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You don't have enough Yang."));
			SendItemTransactionFailedResult(c_pShopItem->dwOwner, c_pShopItem->wPos);

			if (bNewItem)
				M2_DESTROY_ITEM(pItem);

			continue;
		}

#ifdef ENABLE_PRIVATE_SHOP_CHEQUE
		if (pCustomer->GetCheque() < c_pShopItem->TPrice.dwCheque)
		{
			sys_log(0, "PRIVATE_SHOP: Insufficient cheque at customer %s to buy item %u vnum %u", pCustomer->GetName(), c_pShopItem->dwID, c_pShopItem->dwVnum);

			pCustomer->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You don't have enough Won."));
			SendItemTransactionFailedResult(c_pShopItem->dwOwner, c_pShopItem->wPos);
			continue;
		}

#endif
		// Bind data to the item
		pItem->SetSkipSave(true);
		pItem->BindPrivateShop(nullptr); // Break the private shop connection if the item was taken directly from shop (not p2p)

		// Remove private shop data
		pItem->SetGoldPrice(0);
		pItem->SetChequePrice(0);
		pItem->SetCheckinTime(0);

		// Reset item's ownership and cell
		pItem->SetCell(nullptr, 0);

		// Enable saving of the item
		pItem->SetSkipSave(false);

		BYTE bWindow = pItem->GetType() == ITEM_DS ? DRAGON_SOUL_INVENTORY : INVENTORY;
		if (!pItem->AddToCharacter(pCustomer, TItemPos(bWindow, iPos)))
		{
			// Bind private shop to the item if it exists on this core
			if (pPrivateShop)
			{
				CopyItemData(*c_pShopItem, pItem);
				pItem->BindPrivateShop(pPrivateShop);
				pItem->SetSkipSave(true);
			}
			else
			{
				M2_DESTROY_ITEM(pItem);
			}

			sys_log(0, "PRIVATE_SHOP: Failed to add item %u vnum %u to customer %s", c_pShopItem->dwID, c_pShopItem->dwVnum, pCustomer->GetName());
			SendItemTransactionFailedResult(c_pShopItem->dwOwner, c_pShopItem->wPos);
			continue;
		}

		// Remove item from the shop before resetting owner/position (if the shop is in another core, request from db will be sent)
		if (pPrivateShop)
			pPrivateShop->RemoveItem(c_pShopItem->wPos);

		pCustomer->ChangeGold(-c_pShopItem->TPrice.llGold);

#ifdef ENABLE_PRIVATE_SHOP_CHEQUE
		int iChequePrice = c_pShopItem->TPrice.dwCheque;
		pCustomer->PointChange(POINT_CHEQUE, -iChequePrice);
#endif

		ITEM_MANAGER::Instance().SaveSingleItem(pItem);

		if (pItem->GetCount() > 1)
			pCustomer->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You have bought x%d %s."), pItem->GetCount(), pItem->GetName());
		else
			pCustomer->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You have bought %s."), pItem->GetName());
		sys_log(0, "PRIVATE_SHOP: SUCCESS Item transaction for customer %s item %u %u from shop %u", pCustomer->GetName(), c_pShopItem->dwID, c_pShopItem->dwVnum, c_pShopItem->dwOwner);

		// Send result back to db
		SendItemTransaction(c_pShopItem, pCustomer);

		char szHint[128 + 1]{};
		snprintf(szHint, sizeof(szHint), "%s x%u (Private Shop %u)", pItem->GetName(), pItem->GetCount(), c_pShopItem->dwOwner);
		LogManager::Instance().ItemLog(pItem->GetOwner(), pItem, "PRIVATE SHOP ITEM TRANSACTION", szHint);
	}

	return true;
}

void CPrivateShopManager::SendItemTransaction(const TPlayerPrivateShopItem* c_pShopItem /* = nullptr */, LPCHARACTER pCustomer /* = nullptr */)
{
	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_BUY;

	TPacketGDPrivateShopBuy subPacket{};

	subPacket.tTime = time(nullptr);
	std::memcpy(&subPacket.TItem, c_pShopItem, sizeof(subPacket.TItem));
	strlcpy(subPacket.szCustomerName, pCustomer->GetName(), sizeof(subPacket.szCustomerName));
	subPacket.dwCustomer = pCustomer->GetPlayerID();

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(TPacketGDPrivateShopBuy));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&subPacket, sizeof(TPacketGDPrivateShopBuy));
}

void CPrivateShopManager::SendItemTransactionFailedResult(DWORD dwShopID, WORD wPos)
{
	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_FAILED_BUY;

	TPacketGDPrivateShopFailedBuy subPacket{};

	subPacket.dwShopID = dwShopID;
	subPacket.wPos = wPos;

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(TPacketGDPrivateShopFailedBuy));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&subPacket, sizeof(TPacketGDPrivateShopFailedBuy));
}

void CPrivateShopManager::SendItemTransfer(const TPlayerItem* c_pItem)
{
	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_ITEM_TRANSFER;

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(TPlayerItem));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(c_pItem, sizeof(TPlayerItem));
}

void CPrivateShopManager::SendItemExpire(LPITEM pItem)
{
	LPPRIVATE_SHOP pPrivateShop = pItem->GetPrivateShop();

	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_ITEM_EXPIRE;

	TPacketGDPrivateShopItemExpire subPacket{};
	subPacket.dwShopID = pPrivateShop->GetID();
	subPacket.wPos = pItem->GetCell();

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(TPacketGDPrivateShopItemExpire));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&subPacket, sizeof(TPacketGDPrivateShopItemExpire));

	pPrivateShop->RemoveItem(pItem->GetCell());
}

void CPrivateShopManager::SendShopSearchUpdate(DWORD dwShopID, BYTE bState, int iSpecificItemPos /*= -1*/)
{
	TPacketGCPrivateShop mainPacket{};
	mainPacket.bHeader = HEADER_GC_PRIVATE_SHOP;
	mainPacket.wSize = sizeof(TPacketGCPrivateShop) + sizeof(TPacketGCPrivateShopSearchUpdate);
	mainPacket.bSubHeader = SUBHEADER_GC_PRIVATE_SHOP_SEARCH_UPDATE;

	TPacketGCPrivateShopSearchUpdate subPacket {};
	subPacket.dwShopID = dwShopID;
	subPacket.iSpecificItemPos = iSpecificItemPos;
	subPacket.bState = bState;

	TEMP_BUFFER buf;
	buf.write(&mainPacket, sizeof(mainPacket));
	buf.write(&subPacket, sizeof(subPacket));

	const DESC_MANAGER::DESC_SET& c_ref_set = DESC_MANAGER::instance().GetClientSet();
	std::for_each(c_ref_set.begin(), c_ref_set.end(),
		[&buf](LPDESC pDesc)
		{
			if (pDesc && pDesc->GetCharacter())
			{
				if (pDesc->GetCharacter()->IsShopSearch())
					pDesc->Packet(buf.read_peek(), buf.size());
			}
		}
	);

	TPacketGGPrivateShopItemSearchUpdate packet{};
	packet.bHeader = HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH_UPDATE;
	packet.dwShopID = dwShopID;
	packet.iSpecificItemPos = iSpecificItemPos;
	packet.bState = bState;

	P2P_MANAGER::Instance().Send(&packet, sizeof(packet));
}

void CPrivateShopManager::AddSearchItem(LPITEM pItem)
{
	BYTE bItemType = pItem->GetType();
	BYTE bItemSubType = pItem->GetSubType();

	TItemList& itemList = m_map_searchItem[bItemType][bItemSubType];
	itemList.emplace(pItem);
}

void CPrivateShopManager::RemoveSearchItem(LPITEM pItem)
{
	if (!pItem)
		return;

	BYTE bItemType = pItem->GetType();
	BYTE bItemSubType = pItem->GetSubType();

	auto typeIt = m_map_searchItem.find(bItemType);
	if (typeIt == m_map_searchItem.end())
		return;

	TSubTypeItemMap& map_itemSubType = typeIt->second;
	auto subTypeIt = map_itemSubType.find(bItemSubType);
	if (subTypeIt == map_itemSubType.end())
		return;

	TItemList& itemList = subTypeIt->second;

	for (auto it = itemList.begin(); it != itemList.end(); ++it)
	{
		LPITEM pListItem = *it;
		if (pListItem->GetID() == pItem->GetID())
		{
			itemList.erase(it);
			return;
		}
	}
}

bool FilterItem(LPITEM pItem, TPrivateShopSearchFilter& rFilter, bool bUseFilter)
{
	if (!pItem->GetPrivateShop())
	{
		sys_err("Could not find private shop for item %u", pItem->GetID());
		return false;
	}

	if (rFilter.iItemType >= 0)
	{
		if (pItem->GetType() != rFilter.iItemType)
			return false;
	}

	if (rFilter.iItemSubType >= 0)
	{
		if (pItem->GetSubType() != rFilter.iItemSubType)
			return false;
	}

	if (bUseFilter)
	{
		if (rFilter.dwVnum)
		{
			if (pItem->GetVnum() != rFilter.dwVnum)
				return false;
		}

		if (rFilter.iItemType == ITEM_WEAPON || rFilter.iItemType == ITEM_ARMOR)
		{
			if (rFilter.bMinRefine > 0)
			{
				if ((pItem->GetVnum() % 10) < rFilter.bMinRefine) return false;
			}

			if (rFilter.bMaxRefine > 0)
			{
				if ((pItem->GetVnum() % 10) > rFilter.bMaxRefine) return false;
			}
		}

		if (pItem->IsDragonSoul())
		{
			BYTE bType, bGrade, bStep, bStrenght;
			DSManager::Instance().GetDragonSoulInfo(pItem->GetVnum(), bType, bGrade, bStep, bStrenght);

			if (rFilter.bAlchemyClarity > bGrade) return false;

			if (rFilter.bAlchemyLevel > bStrenght) return false;
		}

		if (rFilter.iJob >= 0)
		{
			switch (rFilter.iJob)
			{
			case JOB_WARRIOR:
				if (pItem->GetAntiFlag() & ITEM_ANTIFLAG_WARRIOR)
					return false;
				break;

			case JOB_ASSASSIN:
				if (pItem->GetAntiFlag() & ITEM_ANTIFLAG_ASSASSIN)
					return false;
				break;

			case JOB_SHAMAN:
				if (pItem->GetAntiFlag() & ITEM_ANTIFLAG_SHAMAN)
					return false;
				break;

			case JOB_SURA:
				if (pItem->GetAntiFlag() & ITEM_ANTIFLAG_SURA)
					return false;
				break;
			}
		}

		if (rFilter.iGender >= 0)
		{
			switch (rFilter.iGender)
			{
			case SEX_MALE:
				if (pItem->GetAntiFlag() & ITEM_ANTIFLAG_MALE)
					return false;
				break;

			case SEX_FEMALE:
				if (pItem->GetAntiFlag() & ITEM_ANTIFLAG_FEMALE)
					return false;
				break;
			}
		}


		if (rFilter.wMinStack > 0)
		{
			if (pItem->GetCount() < rFilter.wMinStack) return false;
		}

		if (rFilter.wMaxStack > 0)
		{
			if (pItem->GetCount() > rFilter.wMaxStack) return false;
		}

		if (rFilter.dwMinLevel > 0)
		{
			if (pItem->GetLevelLimit() < rFilter.dwMinLevel) return false;
		}

		if (rFilter.dwMaxLevel > 0)
		{
			if (pItem->GetLevelLimit() > rFilter.dwMaxLevel) return false;
		}

		bool bAttrFound = false;
		if (rFilter.iItemType == ITEM_WEAPON || rFilter.iItemType == ITEM_ARMOR)
		{
			for (BYTE i = 0; i < ITEM_ATTRIBUTE_MAX_NUM; ++i)
			{
				if (!rFilter.aAttr[i].bType)
				{
					bAttrFound = true;
					continue;
				}

				for (BYTE j = 0; j < ITEM_ATTRIBUTE_MAX_NUM; ++j)
				{
					if (pItem->GetAttribute(j).bType == rFilter.aAttr[i].bType &&
						pItem->GetAttribute(j).sValue >= rFilter.aAttr[i].sValue)
					{
						bAttrFound = true;
						break;
					}
					else
						bAttrFound = false;
				}

				if (!bAttrFound)
					break;
			}
		}
		else
			bAttrFound = true;

		if (!bAttrFound)
			return false;
	}

	return true;
}

void CPrivateShopManager::SearchItem(LPDESC pCustomerDesc, TPrivateShopSearchFilter& rFilter, bool bUseFilter, TEMP_BUFFER* pBuf /* = nullptr */)
{
	TEMP_BUFFER buf {};
	DWORD dwCount = 0;
	const static size_t SAFE_RECV_BUFSIZE = 8192;

	/*
		1. Item vnum search
		2. Seller name search
		3. Item type/subtype search
	*/
	if (rFilter.dwVnum)
	{
		TItemTable* pItemTable = ITEM_MANAGER::Instance().GetTable(rFilter.dwVnum);
		if (!pItemTable)
			return;

		rFilter.iItemType = pItemTable->bType;
		rFilter.iItemSubType = pItemTable->bSubType;

		auto typeIt = m_map_searchItem.find(rFilter.iItemType);
		if (typeIt == m_map_searchItem.end())
			return;

		TSubTypeItemMap& map_itemSubType = typeIt->second;
		auto subTypeIt = map_itemSubType.find(rFilter.iItemSubType);
		if (subTypeIt == map_itemSubType.end())
			return;

		TItemList& itemList = subTypeIt->second;

		for (const auto& pItem : itemList)
		{
			if (buf.size() + sizeof(TPrivateShopSearchData) > SAFE_RECV_BUFSIZE)
				break;

			if (FilterItem(pItem, rFilter, bUseFilter))
			{
				TPrivateShopSearchData item{};
				CopyItemData(pItem, item);

				if (pCustomerDesc)
					buf.write(&item, sizeof(TPrivateShopSearchData));
				else if (pBuf)
					pBuf->write(&item, sizeof(TPrivateShopSearchData));

				++dwCount;
			}
		}
	}
	else if (strlen(rFilter.szOwnerName))
	{
		LPPRIVATE_SHOP pPrivateShop = GetPrivateShopByOwnerName(rFilter.szOwnerName);
		if (!pPrivateShop)
			return;

		const auto& itemMap = pPrivateShop->GetItemContainer();
		for (auto it = itemMap.begin(); it != itemMap.end(); it++)
		{
			if (buf.size() + sizeof(TPrivateShopSearchData) > SAFE_RECV_BUFSIZE)
				break;

			LPITEM pItem = it->second;
			if (FilterItem(pItem, rFilter, bUseFilter))
			{
				TPrivateShopSearchData item{};
				CopyItemData(pItem, item);

				if (pCustomerDesc)
					buf.write(&item, sizeof(TPrivateShopSearchData));
				else if (pBuf)
					pBuf->write(&item, sizeof(TPrivateShopSearchData));

				++dwCount;
			}
		}
	}
	else if (rFilter.iItemType >= 0)
	{
		auto typeIt = m_map_searchItem.find(rFilter.iItemType);
		if (typeIt == m_map_searchItem.end())
			return;

		TSubTypeItemMap& map_itemSubType = typeIt->second;

		// Search by type-subtype
		if (rFilter.iItemSubType >= 0)
		{
			auto subTypeIt = map_itemSubType.find(rFilter.iItemSubType);
			if (subTypeIt == map_itemSubType.end())
				return;

			TItemList& itemList = subTypeIt->second;
			for (const auto& pItem : itemList)
			{
				if (buf.size() + sizeof(TPrivateShopSearchData) > SAFE_RECV_BUFSIZE)
					break;

				if (FilterItem(pItem, rFilter, bUseFilter))
				{
					TPrivateShopSearchData item{};
					CopyItemData(pItem, item);

					if (pCustomerDesc)
						buf.write(&item, sizeof(TPrivateShopSearchData));
					else if (pBuf)
						pBuf->write(&item, sizeof(TPrivateShopSearchData));

					++dwCount;
				}
			}
		}
		// Search by type
		else
		{
			for (const auto& kv : map_itemSubType)
			{
				for (const auto& pItem : kv.second)
				{
					if (buf.size() + sizeof(TPrivateShopSearchData) > SAFE_RECV_BUFSIZE)
						break;

					if (FilterItem(pItem, rFilter, bUseFilter))
					{
						TPrivateShopSearchData item{};
						CopyItemData(pItem, item);

						if (pCustomerDesc)
							buf.write(&item, sizeof(TPrivateShopSearchData));
						else if (pBuf)
							pBuf->write(&item, sizeof(TPrivateShopSearchData));

						++dwCount;
					}
				}
			}
		}
	}

	if (pCustomerDesc)
	{
		if (!dwCount)
			return;

		TPacketGCPrivateShop mainPacket{};
		mainPacket.bHeader = HEADER_GC_PRIVATE_SHOP;
		mainPacket.wSize = sizeof(TPacketGCPrivateShop) + dwCount * sizeof(TPrivateShopSearchData);
		mainPacket.bSubHeader = SUBHEADER_GC_PRIVATE_SHOP_SEARCH_RESULT;

		pCustomerDesc->BufferedPacket(&mainPacket, sizeof(TPacketGCPrivateShop));
		pCustomerDesc->LargePacket(buf.read_peek(), buf.size());
	}
}

void CPrivateShopManager::AddMarketItemPrice(DWORD dwVnum, TItemPrice& rSalePrice)
{
	auto it = m_map_marketItemPrice.find(dwVnum);
	if (it != m_map_marketItemPrice.end())
		m_map_marketItemPrice.erase(it);

	m_map_marketItemPrice.emplace(dwVnum, rSalePrice);
}

TItemPrice* CPrivateShopManager::GetMarketItemPrice(DWORD dwVnum)
{
	auto it = m_map_marketItemPrice.find(dwVnum);

	if (it == m_map_marketItemPrice.end())
		return nullptr;

	return &it->second;
}

void CPrivateShopManager::SendMarketItemPriceData(LPCHARACTER pChar)
{
	static std::unordered_set<DWORD> set_requiredItem;
	set_requiredItem.clear();

	// Get list of items present on the character
	for (int i = 0; i < INVENTORY_AND_EQUIP_SLOT_MAX; ++i)
	{
		LPITEM pItem = pChar->GetInventoryItem(i);
		if (pItem)
			set_requiredItem.emplace(pItem->GetVnum());
	}

	for (int i = 0; i < DRAGON_SOUL_INVENTORY_MAX_NUM; ++i)
	{
		LPITEM pDSItem = pChar->GetItem(TItemPos(DRAGON_SOUL_INVENTORY, i));
		if (pDSItem)
			set_requiredItem.emplace(pDSItem->GetVnum());
	}

	if (pChar->IsPrivateShopOwner())
	{
		for (int i = 0; i < PRIVATE_SHOP_HOST_ITEM_MAX_NUM; ++i)
		{
			auto pPrivateShopItem = pChar->GetPrivateShopItem(i);
			if (pPrivateShopItem)
				set_requiredItem.emplace(pPrivateShopItem->dwVnum);
		}
	}

	if (set_requiredItem.empty())
		return;

	TEMP_BUFFER buf;

	TPacketGCPrivateShop mainPacket{};
	mainPacket.bHeader = HEADER_GC_PRIVATE_SHOP;
	mainPacket.wSize = sizeof(TPacketGCPrivateShop) + sizeof(TMarketItemPrice) * set_requiredItem.size();
	mainPacket.bSubHeader = SUBHEADER_GC_PRIVATE_SHOP_MARKET_ITEM_PRICE_DATA_RESULT;

	TMarketItemPrice TMarketPrice{};

	for (const auto& dwVnum : set_requiredItem)
	{
		// If the price was not found send empty one so the client will not request it later on
		memset(&TMarketPrice, 0, sizeof(TMarketItemPrice));
		TMarketPrice.dwVnum = dwVnum;

		const auto pMarketItemPrice = GetMarketItemPrice(dwVnum);

		if (pMarketItemPrice)
		{
			TMarketPrice.TPrice.llGold = pMarketItemPrice->llGold;
			TMarketPrice.TPrice.dwCheque = pMarketItemPrice->dwCheque;
		}

		buf.write(&TMarketPrice, sizeof(TMarketItemPrice));
	}

	if (pChar->GetDesc())
	{
		pChar->GetDesc()->BufferedPacket(&mainPacket, sizeof(TPacketGCPrivateShop));
		pChar->GetDesc()->Packet(buf.read_peek(), buf.size());
	}
}

void CPrivateShopManager::SendMarketItemPrice(LPCHARACTER pChar, DWORD dwVnum)
{
	TPacketGCPrivateShop mainPacket{};
	mainPacket.bHeader = HEADER_GC_PRIVATE_SHOP;
	mainPacket.wSize = sizeof(TPacketGCPrivateShop) + sizeof(TMarketItemPrice);
	mainPacket.bSubHeader = SUBHEADER_GC_PRIVATE_SHOP_MARKET_ITEM_PRICE_RESULT;

	TMarketItemPrice TPrice{};
	TPrice.dwVnum = dwVnum;

	// If the price was not found send empty one so the client will not request it later on
	const auto pMarketItemPrice = GetMarketItemPrice(dwVnum);
	if (pMarketItemPrice)
	{
		TPrice.TPrice.llGold = pMarketItemPrice->llGold;
		TPrice.TPrice.dwCheque = pMarketItemPrice->dwCheque;
	}

	if (pChar->GetDesc())
	{
		pChar->GetDesc()->BufferedPacket(&mainPacket, sizeof(TPacketGCPrivateShop));
		pChar->GetDesc()->Packet(&TPrice, sizeof(TMarketItemPrice));
	}
}
