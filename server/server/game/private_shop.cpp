#include "stdafx.h"
#include "private_shop.h"
#include "private_shop_manager.h"
#include "private_shop_util.h"
#include "char.h"
#include "char_manager.h"
#include "desc.h"
#include "item.h"
#include "item_manager.h"
#include "log.h"
#include "p2p.h"
#include "sectree_manager.h"
#include "DragonSoul.h"
#include "config.h"
#include "buffer_manager.h"
#include "desc_client.h"
#include "../libgame/grid.h"

CPrivateShop::CPrivateShop()
{
	CEntity::Initialize(ENTITY_PRIVATE_SHOP);

	m_set_shopViewer.clear();
	m_map_shopItem.clear();

	m_dwID = 0;
	m_dwVID = 0;
	m_dwVnum = 0;
	m_strOwnerName = "NoName";
	m_strTitle = "NoName";
	m_bTitleType = 0;
	m_bState = 0;
	m_bPageCount = 1;
	m_bIsClosing = false;
}

CPrivateShop::~CPrivateShop()
{
	CleanShopViewers();

	// Remove items that are inside the shop
	for (const auto& kv : m_map_shopItem)
	{
		LPITEM pItem = kv.second;

		// Item has been deleted when shutting down the game, therefore no further actions needed
		if (!pItem)
			continue;

		CPrivateShopManager::Instance().RemoveSearchItem(pItem);

		// Double check
		if (pItem->GetPrivateShop() && pItem->GetCell() < PRIVATE_SHOP_HOST_ITEM_MAX_NUM)
			M2_DESTROY_ITEM(pItem);
	}

	CEntity::Destroy();

	if (GetSectree())
		GetSectree()->RemoveEntity(this);
}

bool CPrivateShop::Show(long lX, long lY, long lZ, long lMapIndex)
{
	LPSECTREE pSectree = SECTREE_MANAGER::instance().Get(lMapIndex, lX, lY);
	if (!pSectree)
	{
		sys_err("%s PRIVATE_SHOP: Cannot find sectree %dx%d mapindex %d", GetOwnerName().c_str(), lX, lY, lMapIndex);
		return false;
	}

	SetXYZ(lX, lY, lZ);
	SetMapIndex(lMapIndex);

	bool bChangeTree = false;

	if (!GetSectree() || GetSectree() != pSectree)
		bChangeTree = true;

	if (bChangeTree)
	{
		if (GetSectree())
			GetSectree()->RemoveEntity(this);

		ViewCleanup();
	}

	if (bChangeTree)
	{
		EncodeInsertPacket(this);
		pSectree->InsertEntity(this);

		UpdateSectree();
	}
	else
	{
		ViewReencode();
	}

	return true;
}

bool CPrivateShop::Initialize(const std::vector<TPlayerPrivateShopItem>& c_vec_shopItem, bool bRespawn /* = false */)
{
	// Grid for each shop's page
	std::unique_ptr<CGrid> aGrid[PRIVATE_SHOP_PAGE_MAX_NUM];
	for (int i = 0; i < PRIVATE_SHOP_PAGE_MAX_NUM; ++i)
		aGrid[i] = std::make_unique<CGrid>(PRIVATE_SHOP_WIDTH, PRIVATE_SHOP_HEIGHT);

	for (const auto& rShopItem : c_vec_shopItem)
	{
		LPITEM pItem = nullptr;
		WORD wInventoryPos = 0;
		// On re spawn items haven't been created yet, while on
		// manual shop creation items are just transfered from the owner
		if (bRespawn)
		{
			pItem = ITEM_MANAGER::instance().CreateItem(rShopItem.dwVnum, rShopItem.dwCount, rShopItem.dwID);

			if (!pItem)
			{
				sys_err("%s PRIVATE_SHOP: Cannot create item %u vnum %u", GetOwnerName().c_str(), rShopItem.dwID, rShopItem.dwVnum);
				return false;
			}
		}
		else
		{
			pItem = ITEM_MANAGER::Instance().Find(rShopItem.dwID);

			if (!pItem)
			{
				sys_err("%s PRIVATE_SHOP: Cannot find item %u vnum %u", GetOwnerName().c_str(), rShopItem.dwID, rShopItem.dwVnum);
				return false;
			}

			wInventoryPos = pItem->GetCell();
		}

		CopyItemData(rShopItem, pItem, pItem->GetOwner());
		if (bRespawn)
		{
			pItem->SetSkipSave(true);
			pItem->OnAfterCreatedItem();
			pItem->SetCell(nullptr, rShopItem.wPos);
			pItem->BindPrivateShop(this);
		}
		else
		{
			// Keep the old position and owner as we need to remove the item later on
			pItem->SetCell(pItem->GetOwner(), wInventoryPos);
		}

		// Select grid based on item's position
		CGrid* pGrid = (rShopItem.wPos < PRIVATE_SHOP_PAGE_ITEM_MAX_NUM) ? aGrid[0].get() : aGrid[1].get();

		// Adapt item's position to the grid's position
		WORD wGridPos = rShopItem.wPos < PRIVATE_SHOP_PAGE_ITEM_MAX_NUM ? rShopItem.wPos : rShopItem.wPos - PRIVATE_SHOP_PAGE_ITEM_MAX_NUM;

		if (!pGrid->IsEmpty(wGridPos, 1, pItem->GetSize()))
		{
			sys_err("%s PRIVATE_SHOP: Cannot find empty space for item on position %u", GetOwnerName().c_str(), rShopItem.wPos);
			return false;
		}
		pGrid->Put(wGridPos, 1, pItem->GetSize());

		// Add item to the shop
		m_map_shopItem.emplace(rShopItem.wPos, pItem);
		CPrivateShopManager::Instance().AddSearchItem(pItem);
	}

	return true;
}

bool CPrivateShop::TransferItems(LPCHARACTER pShopOwner)
{
	if (m_map_shopItem.empty())
		return true;

	std::vector<LPITEM> vec_inventoryItem;
	std::vector<LPITEM> vec_dragonSoulItem;
	std::map<BYTE, std::vector<LPITEM>> map_specialInventory;

	for (const auto& kv : m_map_shopItem)
	{
		LPITEM pItem = kv.second;

		if (pItem->IsDragonSoul())
			vec_dragonSoulItem.push_back(pItem);
#ifdef ENABLE_PRIVATE_SHOP_SPECIAL_INV
		else if (pItem->IsSkillBook())
			map_specialInventory[SKILL_BOOK_INVENTORY].push_back(pItem);
		else if (pItem->IsUpgradeItem())
			map_specialInventory[UPGRADE_ITEMS_INVENTORY].push_back(pItem);
		else if (pItem->IsStone())
			map_specialInventory[STONE_INVENTORY].push_back(pItem);
		else if (pItem->IsGiftBox())
			map_specialInventory[GIFT_BOX_INVENTORY].push_back(pItem);
#endif
		else
			vec_inventoryItem.push_back(pItem);
	}

	// No items are present in the shop, return
	if (vec_inventoryItem.empty() &&
		vec_dragonSoulItem.empty()
#ifdef ENABLE_PRIVATE_SHOP_SPECIAL_INV
		&& map_specialInventory.empty()
#endif
		)
	{
		sys_err("%s PRIVATE_SHOP: Cannot find suitable inventory for items", GetOwnerName().c_str());
		return false;
	}

	// Sort items by size, prioritize bigger ones
	std::sort(vec_inventoryItem.begin(), vec_inventoryItem.end(),
		[](const LPITEM lhs, const LPITEM rhs)
		{
			return lhs->GetSize() > rhs->GetSize();
		}
	);


	std::unique_ptr<CGrid> aGrid[INVENTORY_PAGE_MAX_NUM];
	for (BYTE i = 0; i < INVENTORY_PAGE_MAX_NUM; ++i)
		aGrid[i] = std::make_unique<CGrid>(INVENTORY_PAGE_WIDTH, INVENTORY_PAGE_HEIGHT);

	// Fill the grids with current inventory items
	for (WORD i = 0; i < INVENTORY_MAX_NUM; ++i)
	{
#ifdef ENABLE_PRIVATE_SHOP_EXTEND_INV
		const WORD wUnlockedSlots = pShopOwner->GetInvSize(INVENTORY_UNLOCK_NORMAL);
#endif
		const BYTE bItemInventoryPage = i / INVENTORY_PAGE_ITEM_MAX_NUM;
		CGrid* pGrid = aGrid[bItemInventoryPage].get();
		int iPos = i - (bItemInventoryPage * INVENTORY_PAGE_ITEM_MAX_NUM);

#ifdef ENABLE_PRIVATE_SHOP_EXTEND_INV
		if (i > wUnlockedSlots)
		{
			pGrid->Put(iPos, 1, 1);
			continue;
		}
#endif

		LPITEM pItem = pShopOwner->GetInventoryItem(i);
		if (pItem)
		{
			pGrid->Put(iPos, 1, pItem->GetSize());
		}
	}

	// Fill the grids with remaining items in the shop
	for (const auto& pItem : vec_inventoryItem)
	{
		bool bPositionFound = false;

		// Find a suitable position at each grid
		for (BYTE j = 0; j < INVENTORY_PAGE_MAX_NUM; j++)
		{
			CGrid* pGrid = aGrid[j].get();

			const int iPos = pGrid->FindBlank(1, pItem->GetSize());
			if (iPos >= 0)
			{
				pGrid->Put(iPos, 1, pItem->GetSize());
				bPositionFound = true;
				break;
			}
		}

		// Cancel if an item cannot be placed in the player's inventory
		if (!bPositionFound)
		{
			pShopOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You don't have enough space in your inventory."));
			return false;
		}
	}

#ifdef ENABLE_PRIVATE_SHOP_SPECIAL_INV
	for (const auto& kv : map_specialInventory)
	{
		BYTE bInventoryType = kv.first;
		auto& vec_items = kv.second;

		switch (bInventoryType)
		{
		case SKILL_BOOK_INVENTORY:
		{
			WORD wItemCount = 0;
#ifdef ENABLE_PRIVATE_SHOP_EXTEND_SPECIAL_INV
			for (WORD i = SKILL_BOOK_INVENTORY_SLOT_START; i < SKILL_BOOK_INVENTORY_SLOT_START + pShopOwner->GetInvSize(INVENTORY_UNLOCK_BOOK); ++i)
#else
			for (WORD i = SKILL_BOOK_INVENTORY_SLOT_START; i < SKILL_BOOK_INVENTORY_SLOT_END; ++i)
#endif
			{
				if (pShopOwner->GetInventoryItem(i))
					++wItemCount;
			}

#ifdef ENABLE_PRIVATE_SHOP_EXTEND_SPECIAL_INV
			if (wItemCount + vec_items.size() > pShopOwner->GetInvSize(INVENTORY_UNLOCK_BOOK))
#else
			if (wItemCount + vec_items.size() > SKILL_BOOK_INVENTORY_MAX_NUM)
#endif
			{
				pShopOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You don't have enough space in your inventory."));
				return false;
			}

		} break;

		case UPGRADE_ITEMS_INVENTORY:
		{
			WORD wItemCount = 0;
#ifdef ENABLE_PRIVATE_SHOP_EXTEND_SPECIAL_INV
			for (WORD i = UPGRADE_ITEMS_INVENTORY_SLOT_START; i < UPGRADE_ITEMS_INVENTORY_SLOT_START + pShopOwner->GetInvSize(INVENTORY_UNLOCK_UPGRADE); ++i)
#else
			for (WORD i = SKILL_BOOK_INVENTORY_SLOT_START; i < UPGRADE_ITEMS_INVENTORY_SLOT_END; ++i)
#endif
			{
				if (pShopOwner->GetInventoryItem(i))
					++wItemCount;
			}

#ifdef ENABLE_PRIVATE_SHOP_EXTEND_SPECIAL_INV
			if (wItemCount + vec_items.size() > pShopOwner->GetInvSize(INVENTORY_UNLOCK_UPGRADE))
#else
			if (wItemCount + vec_items.size() > UPGRADE_ITEMS_INVENTORY_MAX_NUM)
#endif
			{
				pShopOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You don't have enough space in your inventory."));
				return false;
			}
		} break;

		case STONE_INVENTORY:
		{
			WORD wItemCount = 0;
#ifdef ENABLE_PRIVATE_SHOP_EXTEND_SPECIAL_INV
			for (WORD i = STONE_INVENTORY_SLOT_START; i < STONE_INVENTORY_SLOT_START + pShopOwner->GetInvSize(INVENTORY_UNLOCK_STONE); ++i)
#else
			for (WORD i = SKILL_BOOK_INVENTORY_SLOT_START; i < STONE_INVENTORY_SLOT_END; ++i)
#endif
			{
				if (pShopOwner->GetInventoryItem(i))
					++wItemCount;
			}

#ifdef ENABLE_PRIVATE_SHOP_EXTEND_SPECIAL_INV
			if (wItemCount + vec_items.size() > pShopOwner->GetInvSize(INVENTORY_UNLOCK_STONE))
#else
			if (wItemCount + vec_items.size() > STONE_INVENTORY_MAX_NUM)
#endif
			{
				pShopOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You don't have enough space in your inventory."));
				return false;
			}
} break;

		case GIFT_BOX_INVENTORY:
		{
			WORD wItemCount = 0;
#ifdef ENABLE_PRIVATE_SHOP_EXTEND_SPECIAL_INV
			for (WORD i = GIFT_BOX_INVENTORY_SLOT_START; i < GIFT_BOX_INVENTORY_SLOT_START + pShopOwner->GetInvSize(INVENTORY_UNLOCK_CHEST); ++i)
#else
			for (WORD i = SKILL_BOOK_INVENTORY_SLOT_START; i < GIFT_BOX_INVENTORY_SLOT_END; ++i)
#endif
			{
				if (pShopOwner->GetInventoryItem(i))
					++wItemCount;
			}

#ifdef ENABLE_PRIVATE_SHOP_EXTEND_SPECIAL_INV
			if (wItemCount + vec_items.size() > pShopOwner->GetInvSize(INVENTORY_UNLOCK_CHEST))
#else
			if (wItemCount + vec_items.size() > GIFT_BOX_INVENTORY_MAX_NUM)
#endif
			{
				pShopOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You don't have enough space in your inventory."));
				return false;
			}
		} break;

		}
	}
#endif

	if (!vec_dragonSoulItem.empty())
	{
		std::vector<WORD> s_vDSGrid(DRAGON_SOUL_INVENTORY_MAX_NUM);
		pShopOwner->CopyDragonSoulItemGrid(s_vDSGrid);

		for (const auto pItem : vec_dragonSoulItem)
		{
			bool bPositionFound = false;
			WORD wBasePos = DSManager::instance().GetBasePosition(pItem);

			if (wBasePos >= DRAGON_SOUL_INVENTORY_MAX_NUM)
			{
				pShopOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You don't have enough space in your inventory."));
				return false;
			}

			/* Copied from exchange.cpp */
			for (int i = 0; i < DRAGON_SOUL_BOX_SIZE; i++)
			{
				WORD wPos = wBasePos + i;
				if (0 == s_vDSGrid[wPos])
				{
					bool bEmpty = true;
					for (int j = 1; j < pItem->GetSize(); j++)
					{
						if (s_vDSGrid[wPos + j * DRAGON_SOUL_BOX_COLUMN_NUM])
						{
							bEmpty = false;

						}
					}

					if (bEmpty)
					{
						for (int j = 0; j < pItem->GetSize(); j++)
						{
							s_vDSGrid[wPos + j * DRAGON_SOUL_BOX_COLUMN_NUM] = wPos + 1;
						}

						bPositionFound = true;
						break;
					}
				}

				if (bPositionFound)
					break;
			}

			if (!bPositionFound)
			{
				pShopOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You don't have enough space in your inventory."));
				return false;
			}
		}
	}

	while(!m_map_shopItem.empty())
	{
		auto it = m_map_shopItem.begin();
		const WORD wPos = it->first;
		LPITEM pItem = it->second;

		int iPos = GetEmptyInventory(pShopOwner, pItem);
		BYTE bWindow = INVENTORY;

		if (pItem->IsDragonSoul())
			bWindow = DRAGON_SOUL_INVENTORY;

		if (iPos < 0)
		{
			pShopOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You don't have enough space in your inventory."));

			sys_err("%s PRIVATE_SHOP: Position mismatch for item %u vnum %u size %u target_cell %u", GetOwnerName().c_str(),
				pItem->GetID(), pItem->GetVnum(), pItem->GetSize(), pItem->GetCell());
			return false;
		}

		pItem->SetSkipSave(false);
		pItem->SetCell(nullptr, 0);	// Reset the item ownership

		if (pItem->AddToCharacter(pShopOwner, TItemPos(bWindow, iPos)))
		{
			pItem->BindPrivateShop(nullptr);
			pItem->SetGoldPrice(0);
			pItem->SetChequePrice(0);

			pShopOwner->RemovePrivateShopItem(wPos);
			RemoveItem(wPos);

			TPlayerItem t{};
			CopyItemData(pItem, t);
			CPrivateShopManager::Instance().SendItemTransfer(&t);
			char szHint[128 + 1]{};
			snprintf(szHint, sizeof(szHint), "%s x%u", pItem->GetName(), pItem->GetCount());
			LogManager::Instance().ItemLog(pItem->GetOwner(), pItem, "PRIVATE SHOP CLOSE CHECKOUT", szHint);
			continue;
		}
		else
		{
			sys_err("%s PRIVATE_SHOP: Failed item transfer for item %u vnum %u size %u target_cell %u", GetOwnerName().c_str(),
				pItem->GetID(), pItem->GetVnum(), pItem->GetSize(), pItem->GetCell());

			pItem->SetSkipSave(true);
			return false;
		}
	}

	return true;
}

void CPrivateShop::CleanItems()
{
	for (const auto& kv : m_map_shopItem)
	{
		LPITEM pItem = kv.second;
		CPrivateShopManager::Instance().RemoveSearchItem(pItem);
		M2_DESTROY_ITEM(pItem);
	}

	m_map_shopItem.clear();
}

LPITEM CPrivateShop::GetItem(WORD wPos)
{
	auto it = m_map_shopItem.find(wPos);

	if (it == m_map_shopItem.end())
	{
		sys_err("%s PRIVATE_SHOP: Cannot find item on pos %u", GetOwnerName().c_str(), wPos);
		return nullptr;
	}

	return it->second;
}

void CPrivateShop::RemoveItem(WORD wPos)
{
	auto it = m_map_shopItem.find(wPos);

	if (it == m_map_shopItem.end())
	{
		sys_err("%s PRIVATE_SHOP: Cannot find item on pos %u", GetOwnerName().c_str(), wPos);
	}
	else
	{
		LPITEM pItem = it->second;
		CPrivateShopManager::Instance().RemoveSearchItem(pItem);

		// Remove the item if it wasn't transfered to the character (p2p transaction)
		if (pItem->GetPrivateShop())
			M2_DESTROY_ITEM(pItem);

		m_map_shopItem.erase(it);
	}

	// Refresh window for current viewers
	TPacketGCPrivateShop mainPacket{};
	mainPacket.bHeader = HEADER_GC_PRIVATE_SHOP;
	mainPacket.wSize = sizeof(TPacketGCPrivateShop) + sizeof(WORD);
	mainPacket.bSubHeader = SUBHEADER_GC_PRIVATE_SHOP_REMOVE_ITEM;

	TEMP_BUFFER buf;
	buf.write(&mainPacket, sizeof(mainPacket));
	buf.write(&wPos, sizeof(WORD));

	BroadcastPacket(buf.read_peek(), buf.size());

	CPrivateShopManager::Instance().SendShopSearchUpdate(GetID(), STATE_REMOVED, wPos);
}

bool CPrivateShop::ItemCheckin(const TPlayerPrivateShopItem& c_rShopItem)
{
	LPITEM pItem = ITEM_MANAGER::instance().CreateItem(c_rShopItem.dwVnum, c_rShopItem.dwCount, c_rShopItem.dwID);

	if (!pItem)
	{
		sys_err("%s PRIVATE_SHOP: Cannot create item %u vnum %u", GetOwnerName().c_str(), c_rShopItem.dwID, c_rShopItem.dwVnum);
		return false;
	}

	pItem->SetSkipSave(true);
	pItem->BindPrivateShop(this);
	CopyItemData(c_rShopItem, pItem);
	pItem->OnAfterCreatedItem();

	m_map_shopItem.emplace(c_rShopItem.wPos, pItem);
	CPrivateShopManager::Instance().AddSearchItem(pItem);

	sys_log(0, "%s PRIVATE_SHOP: ITEM CHECKIN cell %u vnum %u count %u", GetOwnerName().c_str(), c_rShopItem.wPos, c_rShopItem.dwVnum, c_rShopItem.dwCount);

	// Refresh window for current viewers
	TPacketGCPrivateShop mainPacket{};
	mainPacket.bHeader = HEADER_GC_PRIVATE_SHOP;
	mainPacket.wSize = sizeof(TPacketGCPrivateShop) + sizeof(TPrivateShopItemData);
	mainPacket.bSubHeader = SUBHEADER_GC_PRIVATE_SHOP_ADD_ITEM;

	TPrivateShopItemData shopItem{};
	CopyItemData(c_rShopItem, shopItem);

	TEMP_BUFFER buf;
	buf.write(&mainPacket, sizeof(mainPacket));
	buf.write(&shopItem, sizeof(TPrivateShopItemData));

	BroadcastPacket(buf.read_peek(), buf.size());

	return true;
}

void CPrivateShop::ChangeItemPrice(WORD wPos, long long llGold, DWORD dwCheque)
{
	LPITEM pItem = GetItem(wPos);
	if (!pItem)
	{
		sys_err("%s PRIVATE_SHOP: Cannot find an item at pos %u", GetOwnerName().c_str(), wPos);
		return;
	}

	pItem->SetGoldPrice(llGold);
	pItem->SetChequePrice(dwCheque);

	sys_log(0, "%s PRIVATE_SHOP: PRICE CHANGE cell %u gold %lld cheque %u", GetOwnerName().c_str(), wPos, llGold, dwCheque);

	// Refresh window for current viewers
	TPacketGCPrivateShop mainPacket{};
	mainPacket.bHeader = HEADER_GC_PRIVATE_SHOP;
	mainPacket.wSize = sizeof(TPacketGCPrivateShop) + sizeof(TPacketGCPrivateShopItemPriceChange);
	mainPacket.bSubHeader = SUBHEADER_GC_PRIVATE_SHOP_ITEM_PRICE_CHANGE;

	TPacketGCPrivateShopItemPriceChange subPacket{};
	subPacket.wPos = wPos;
	subPacket.TPrice.llGold = llGold;
	subPacket.TPrice.dwCheque = dwCheque;

	TEMP_BUFFER buf;
	buf.write(&mainPacket, sizeof(TPacketGCPrivateShop));
	buf.write(&subPacket, sizeof(TPacketGCPrivateShopItemPriceChange));

	BroadcastPacket(buf.read_peek(), buf.size());

	// disable momentan
	//char szHint[128 + 1]{};
	//snprintf(szHint, sizeof(szHint), "%s x%u Gold %lld Cheque %u", pItem->GetName(), pItem->GetCount(), llGold, dwCheque);
	//LogManager::Instance().ItemLog(GetID(), GetX(), GetY(), pItem->GetID(), "PRIVATE SHOP PRICE CHANGE", szHint, "", pItem->GetOriginalVnum(), 0);
}

void CPrivateShop::MoveItem(WORD wPos, WORD wChangePos)
{
	LPITEM pItem = GetItem(wPos);
	if (!pItem)
	{
		sys_err("%s PRIVATE_SHOP: Cannot find an item at pos %u", GetOwnerName().c_str(), wPos);
		return;
	}

	if (m_map_shopItem.find(wChangePos) != m_map_shopItem.end())
	{
		sys_err("%s PRIVATE_SHOP: Cell %u is already occupied", GetOwnerName().c_str(), wChangePos);
		return;
	}

	pItem->SetCell(nullptr, wChangePos);

	m_map_shopItem.erase(wPos);
	m_map_shopItem.emplace(wChangePos, pItem);

	sys_log(0, "%s PRIVATE_SHOP: ITEM MOVE src_cell %u -> target_cell %u", GetOwnerName().c_str(), wPos, wChangePos);

	// Refresh window for current viewers
	TPacketGCPrivateShop mainPacket{};
	mainPacket.bHeader = HEADER_GC_PRIVATE_SHOP;
	mainPacket.wSize = sizeof(TPacketGCPrivateShop) + sizeof(TPacketGCPrivateShopItemMove);
	mainPacket.bSubHeader = SUBHEADER_GC_PRIVATE_SHOP_ITEM_MOVE;

	TPacketGCPrivateShopItemMove subPacket{};
	subPacket.wPos = wPos;
	subPacket.wChangePos = wChangePos;

	TEMP_BUFFER buf;
	buf.write(&mainPacket, sizeof(TPacketGCPrivateShop));
	buf.write(&subPacket, sizeof(TPacketGCPrivateShopItemMove));

	BroadcastPacket(buf.read_peek(), buf.size());
}

void CPrivateShop::ChangeTitle(const char* c_szTitle)
{
	sys_log(0, "%s PRIVATE_SHOP: TITLE CHANGE %s -> %s", GetOwnerName().c_str(), GetTitle().c_str(), c_szTitle);

	m_strTitle = c_szTitle;

	TPacketGCPrivateShop mainPacket{};
	mainPacket.bHeader = HEADER_GC_PRIVATE_SHOP;
	mainPacket.wSize = sizeof(TPacketGCPrivateShop) + sizeof(TPacketGCPrivateShopTitle);
	mainPacket.bSubHeader = SUBHEADER_GC_PRIVATE_SHOP_TITLE;

	TPacketGCPrivateShopTitle subPacket{};
	subPacket.dwVID = GetVID();
	strlcpy(subPacket.szTitle, c_szTitle, sizeof(subPacket.szTitle));
	subPacket.bTitleType = GetTitleType();

	TEMP_BUFFER buf;
	buf.write(&mainPacket, sizeof(TPacketGCPrivateShop));
	buf.write(&subPacket, sizeof(TPacketGCPrivateShopTitle));

	PacketAround(buf.read_peek(), buf.size());
}

void CPrivateShop::SetState(BYTE bState)
{
	CPrivateShopManager::Instance().SendShopSearchUpdate(GetID(), bState);

	if (bState == STATE_CLOSED)
	{
		CPrivateShopManager::Instance().DeletePrivateShop(GetID());
		return;
	}

	sys_log(0, "%s PRIVATE_SHOP: STATE CHANGE %u -> %u", GetOwnerName().c_str(), GetState(), bState);

	m_bState = bState;
	if (m_set_shopViewer.size())
	{
		for (const auto& pShopViewer : m_set_shopViewer)
		{
			if (pShopViewer->GetViewingPrivateShop() != this)
				continue;

			pShopViewer->SetPrivateShopState(bState, false);
		}
	}
}

void CPrivateShop::SetUnlockedSlots(WORD wUnlockedSlots)
{
	m_wUnlockedSlots = wUnlockedSlots;

	sys_log(0, "%s PRIVATE_SHOP: UNLOCKED SLOT %u", GetOwnerName().c_str(), m_wUnlockedSlots);

	TPacketGCPrivateShop mainPacket{};
	mainPacket.bHeader = HEADER_GC_PRIVATE_SHOP;
	mainPacket.wSize = sizeof(TPacketGCPrivateShop) + sizeof(WORD);
	mainPacket.bSubHeader = SUBHEADER_GC_PRIVATE_SHOP_UNLOCKED_SLOTS_CHANGE;

	TEMP_BUFFER buf;
	buf.write(&mainPacket, sizeof(TPacketGCPrivateShop));
	buf.write(&m_wUnlockedSlots, sizeof(WORD));

	BroadcastPacket(buf.read_peek(), buf.size());
}

bool CPrivateShop::AddShopViewer(LPCHARACTER pShopViewer)
{
	if (!pShopViewer)
		return false;

	if (!CheckTradeWindows(pShopViewer))
		return false;

	if (pShopViewer->GetViewingPrivateShop())
		return false;

	if (!pShopViewer->GetDesc())
		return false;

	pShopViewer->SetViewingPrivateShop(this);
	m_set_shopViewer.emplace(pShopViewer);

	sys_log(0, "%s PRIVATE_SHOP: VIEWER ADDED name %s pid %u", GetOwnerName().c_str(), pShopViewer->GetName(), pShopViewer->GetPlayerID());

	TPacketGCPrivateShop mainPacket{};
	mainPacket.bHeader = HEADER_GC_PRIVATE_SHOP;
	mainPacket.wSize = sizeof(TPacketGCPrivateShop) + sizeof(TPacketGCPrivateShopOpen);
	mainPacket.bSubHeader = SUBHEADER_GC_PRIVATE_SHOP_START;

	TPacketGCPrivateShopOpen subPacket{};
	memset(&subPacket, 0, sizeof(TPacketGCPrivateShopOpen));

	subPacket.bState = GetState();
	subPacket.bPageCount = GetPageCount();
	subPacket.wUnlockedSlots = GetUnlockedSlots();
	subPacket.dwVID = GetVID();
	strlcpy(subPacket.szTitle, GetTitle().c_str(), sizeof(subPacket.szTitle));

	for (const auto& kv : m_map_shopItem)
	{
		LPITEM pItem = kv.second;
		WORD i = pItem->GetCell();

		if (i >= PRIVATE_SHOP_HOST_ITEM_MAX_NUM)
		{
			sys_err("%s PRIVATE_SHOP: Invalid item position %u on item %u", GetOwnerName().c_str(), i, pItem->GetID());
			return false;
		}

		CopyItemData(pItem, subPacket.aItems[i]);
	}

	pShopViewer->GetDesc()->BufferedPacket(&mainPacket, sizeof(TPacketGCPrivateShop));
	pShopViewer->GetDesc()->Packet(&subPacket, sizeof(TPacketGCPrivateShopOpen));

	return true;
}

bool CPrivateShop::RemoveShopViewer(LPCHARACTER pShopViewer)
{
	if (pShopViewer->GetViewingPrivateShop() != this)
		return false;

	for (auto it = m_set_shopViewer.begin(); it != m_set_shopViewer.end(); ++it)
	{
		LPCHARACTER ch = *it;
		if (ch && ch == pShopViewer)
		{
			ch->SetViewingPrivateShop(nullptr);

			TPacketGCPrivateShop mainPacket{};
			mainPacket.bHeader = HEADER_GC_PRIVATE_SHOP;
			mainPacket.wSize = sizeof(TPacketGCPrivateShop);
			mainPacket.bSubHeader = SUBHEADER_GC_PRIVATE_SHOP_END;

			ch->GetDesc()->Packet(&mainPacket, sizeof(TPacketGCPrivateShop));

			m_set_shopViewer.erase(it);
			return true;
		}
	}

	return false;
}

void CPrivateShop::CleanShopViewers()
{
	if (m_set_shopViewer.empty())
		return;

	for (const auto& pShopViewer : m_set_shopViewer)
	{
		pShopViewer->SetViewingPrivateShop(nullptr);

		TPacketGCPrivateShop mainPacket{};
		mainPacket.bHeader = HEADER_GC_PRIVATE_SHOP;
		mainPacket.wSize = sizeof(TPacketGCPrivateShop);
		mainPacket.bSubHeader = SUBHEADER_GC_PRIVATE_SHOP_END;

		pShopViewer->GetDesc()->Packet(&mainPacket, sizeof(TPacketGCPrivateShop));
	}

	sys_log(0, "%s PRIVATE_SHOP: REMOVED VIEWERS", GetOwnerName().c_str());

	m_set_shopViewer.clear();
}

void CPrivateShop::BroadcastPacket(const void* c_pData, int iSize)
{
	for (const auto& pShopViewer : m_set_shopViewer)
	{
		if (pShopViewer->GetViewingPrivateShop() != this)
			continue;

		if (pShopViewer->GetDesc())
			pShopViewer->GetDesc()->Packet(c_pData, iSize);
	}
}

void CPrivateShop::EncodeInsertPacket(LPENTITY pEntity)
{
	if (LPDESC d = pEntity->GetDesc())
	{
		const PIXEL_POSITION& c_Pos = GetXYZ();

		TPacketGCPrivateShop mainPacket{};
		mainPacket.bHeader = HEADER_GC_PRIVATE_SHOP;
		mainPacket.wSize = sizeof(TPacketGCPrivateShop) + sizeof(TPacketGCPrivateShopAddEntity);
		mainPacket.bSubHeader = SUBHEADER_GC_PRIVATE_SHOP_ADD_ENTITY;

		TPacketGCPrivateShopAddEntity subPacket{};
		subPacket.lX = c_Pos.x;
		subPacket.lY = c_Pos.y;
		subPacket.lZ = c_Pos.z;
		subPacket.dwVID = GetVID();
		subPacket.dwVnum = GetVnum();
		strlcpy(subPacket.szName, GetOwnerName().c_str(), sizeof(subPacket.szName));
		subPacket.bTitleType = GetTitleType();
		strlcpy(subPacket.szTitle, GetTitle().c_str(), sizeof(subPacket.szTitle));

		TEMP_BUFFER buf;
		buf.write(&mainPacket, sizeof(TPacketGCPrivateShop));
		buf.write(&subPacket, sizeof(TPacketGCPrivateShopAddEntity));

		d->Packet(buf.read_peek(), buf.size());
	}
}

void CPrivateShop::EncodeRemovePacket(LPENTITY pEntity)
{
	if (LPDESC d = pEntity->GetDesc())
	{
		TPacketGCPrivateShop mainPacket{};
		mainPacket.bHeader = HEADER_GC_PRIVATE_SHOP;
		mainPacket.wSize = sizeof(TPacketGCPrivateShop) + sizeof(TPacketGCPrivateShopDelEntity);
		mainPacket.bSubHeader = SUBHEADER_GC_PRIVATE_SHOP_DEL_ENTITY;

		TPacketGCPrivateShopDelEntity subPacket{};
		subPacket.dwVID = GetVID();

		TEMP_BUFFER buf;
		buf.write(&mainPacket, sizeof(TPacketGCPrivateShop));
		buf.write(&subPacket, sizeof(TPacketGCPrivateShopDelEntity));

		d->Packet(buf.read_peek(), buf.size());
	}
}
bool CPrivateShop::HasItemByID(const DWORD itemID)
{
	for (const auto& [_, item] : m_map_shopItem)
		if (item->GetID() == itemID)
			return true;
		return false;
}
