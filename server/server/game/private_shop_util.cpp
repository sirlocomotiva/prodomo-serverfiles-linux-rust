#include "stdafx.h"
#include "private_shop_util.h"
#include "packet.h"
#include "item.h"
#include "char.h"
#include "private_shop.h"
#include "private_shop_manager.h"

// Messages
/*
	"You have increased available space on your personal shop.";
	"Your personal shop is currently unavailable.";
	"You cannot place any more items in your personal shop.";
	"You cannot move the item to that position.";
	"You cannot manage personal shop's content while it is not in a modifying state.";
	"There is no money to collect.";
	"Refresh your search result to sync price of the item.";
	"This item is currently unavailable.";
	"You do not have an open personal shop.";
	"You must withdraw your earnings before you can close your personal shop.";
	"You cannot use a personal shop now.";
	"You cannot open a personal shop without a Bundle.";
	"You need a Kashmir Bundle to decorate your personal shop.";
	"You have already opened your personal shop.";
	"You cannot open a personal shop in a dungeon.";
	"Please choose another title name for your personal shop.";
	"The entered name is too short.";
	"You cannot sell Item-Shop items in a personal shop.";
	"You cannot sell equipped items in a personal shop.";
	"You cannot sell locked items in a personal shop.";
	"Your personal shop has been closed.";
	"Teleport to the private shop has been canceled.";
	"%u seconds until teleport.";
	"Your logout has been canceled.";
	"You can not teleport to your private shop right now.";
	"You are teleporting to your private shop. Please wait.";
	"You have sold x%lld %s for %s Yangs.";
	"%s has been removed from the shop as the time has expired.";
	"You cannot collect the money, because you would then have more than 2 billion Yang.";
	"You have withdrawn %lld Yang from your personal shop earnings.";
	"You cannot trade while another window is open.";
	"You have just recently reached maximum premium time for your personal shop. ";
	"You have increased Premium Personal Shop duration.";
	"You have reached maximum premium time for your personal shop. ";
	"You cannot use items while editing your personal shop.";
	"Please wait a few moments before building your personal shop again.";
	"You cannot open a personal shop while another window is open.";
	"You cannot close a personal shop while another window is open.";
	"Your logout has been canceled.";
	"You cannot view a personal shop while having other trading windows open.";
	"Please wait a moment before buying from a personal shop again.";
	"You cannot buy an item from your own personal shop.";
	"Please wait a few moments before withdrawing your personal shop again.";
	"You cannot modify a personal shop while having other trading windows open.";
	"The items you put up for sale must not exceed the permitted total value.";
	"You cannot unlock a slot on your personal shop without Personal Shop Key.";
	"You cannot unlock any more slots.";
	"Please wait a moment before searching other personal shops again.";
	"You cannot buy an item from your own personal shop.";
	"Please wait a moment before buying from a personal shop again.";
*/

bool CanBuildPrivateShop(LPCHARACTER ch)
{
	switch (ch->GetMapIndex())
	{
	case 1:
	case 21:
	case 41:
		return true;

	default:
		return false;
	}
}
bool CheckTradeWindows(LPCHARACTER ch)
{
	// Default windows
	if (
		ch->GetExchange() ||
		ch->IsOpenSafebox() ||
		ch->GetMyShop() ||
		ch->GetShopOwner() ||
		ch->IsCubeOpen() ||
		ch->IsRefineThroughGuild()
		)
		return false;

	return true;
}

int GetEmptyInventory(LPCHARACTER pChar, LPITEM pItem /* = nullptr */)
{
	int iPos = -1;

	if (pItem->IsDragonSoul())
	{
		iPos = pChar->GetEmptyDragonSoulInventory(pItem);
	}
#ifdef ENABLE_PRIVATE_SHOP_SPECIAL_INV
	else if (pItem->IsSkillBook())
	{
		iPos = pChar->GetEmptySkillBookInventory(pItem->GetSize());
	}
	else if (pItem->IsUpgradeItem())
	{
		iPos = pChar->GetEmptyUpgradeItemsInventory(pItem->GetSize());
	}
	else if (pItem->IsStone())
	{
		iPos = pChar->GetEmptyStoneInventory(pItem->GetSize());
	}
	else if (pItem->IsGiftBox())
	{
		iPos = pChar->GetEmptyGiftBoxInventory(pItem->GetSize());
	}
#endif
	else
	{
		iPos = pChar->GetEmptyInventory(pItem->GetSize());
	}

	return iPos;
}

/*		LPITEM -> TPlayerItem		*/
void CopyItemData(LPITEM pItem, TPlayerItem& rTargetTable)
{
	rTargetTable.id = pItem->GetID();

	if (pItem->GetOwner())
	{
		rTargetTable.owner = pItem->GetOwner()->GetPlayerID();
	}
	else if (pItem->GetPrivateShop())
	{
		rTargetTable.owner = pItem->GetPrivateShop()->GetID();
	}
	else
	{
		rTargetTable.owner = 0;
		sys_err("Could not find owner of the item %u", pItem->GetID());
	}

	rTargetTable.vnum = pItem->GetVnum();
	rTargetTable.count = pItem->GetCount();
	rTargetTable.pos = pItem->GetCell();
	rTargetTable.window = pItem->GetWindow();
#ifdef ENABLE_PRIVATE_SHOP_CHANGE_LOOK
    rTargetTable.dwTransmutationVnum = pItem->GetTransmutationVnum();
#endif
#ifdef ENABLE_PRIVATE_SHOP_REFINE_ELEMENT
    rTargetTable.dwRefineElement = pItem->GetRefineElement();
#endif
	memcpy(rTargetTable.alSockets, pItem->GetSockets(), sizeof(rTargetTable.alSockets));
#ifdef ENABLE_PRIVATE_SHOP_APPLY_RANDOM
    memcpy(rTargetTable.aApplyRandom, pItem->GetRandomApplies(), sizeof(rTargetTable.aApplyRandom));
#endif
	memcpy(rTargetTable.aAttr, pItem->GetAttributes(), sizeof(rTargetTable.aAttr));
}

/*		LPITEM -> TPlayerPrivateShopItem	*/
void CopyItemData(LPITEM pItem, TPlayerPrivateShopItem& rTargetTable)
{
	rTargetTable.dwID = pItem->GetID();

	if (pItem->GetOwner())
	{
		rTargetTable.dwOwner = pItem->GetOwner()->GetPlayerID();
	}
	else if (pItem->GetPrivateShop())
	{
		rTargetTable.dwOwner = pItem->GetPrivateShop()->GetID();
	}
	else
	{
		rTargetTable.dwOwner = 0;
		sys_err("Could not find owner of the item %u", pItem->GetID());
	}

	rTargetTable.dwVnum = pItem->GetVnum();
	rTargetTable.dwCount = pItem->GetCount();
	rTargetTable.wPos = pItem->GetCell();
	rTargetTable.TPrice.llGold = pItem->GetGoldPrice();
	rTargetTable.TPrice.dwCheque = pItem->GetChequePrice();
	rTargetTable.tCheckin = pItem->GetCheckinTime();
#ifdef ENABLE_PRIVATE_SHOP_CHANGE_LOOK
    rTargetTable.dwTransmutationVnum = pItem->GetTransmutationVnum();
#endif
#ifdef ENABLE_PRIVATE_SHOP_REFINE_ELEMENT
    rTargetTable.dwRefineElement = pItem->GetRefineElement();
#endif
	memcpy(rTargetTable.alSockets, pItem->GetSockets(), sizeof(rTargetTable.alSockets));
#ifdef ENABLE_PRIVATE_SHOP_APPLY_RANDOM
    memcpy(rTargetTable.aApplyRandom, pItem->GetRandomApplies(), sizeof(rTargetTable.aApplyRandom));
#endif
	memcpy(rTargetTable.aAttr, pItem->GetAttributes(), sizeof(rTargetTable.aAttr));
}

/*		LPITEM -> TPrivateShopItemData	*/
void CopyItemData(LPITEM pItem, TPrivateShopItemData& rTargetTable)
{
	rTargetTable.dwVnum = pItem->GetVnum();
	rTargetTable.dwCount = pItem->GetCount();
	rTargetTable.wPos = pItem->GetCell();
	rTargetTable.TPrice.llGold = pItem->GetGoldPrice();
	rTargetTable.TPrice.dwCheque = pItem->GetChequePrice();
	rTargetTable.tCheckin = pItem->GetCheckinTime();
#ifdef ENABLE_PRIVATE_SHOP_CHANGE_LOOK
    rTargetTable.dwTransmutationVnum = pItem->GetTransmutationVnum();
#endif
#ifdef ENABLE_PRIVATE_SHOP_REFINE_ELEMENT
    rTargetTable.dwRefineElement = pItem->GetRefineElement();
#endif
	memcpy(rTargetTable.alSockets, pItem->GetSockets(), sizeof(rTargetTable.alSockets));
#ifdef ENABLE_PRIVATE_SHOP_APPLY_RANDOM
    memcpy(rTargetTable.aApplyRandom, pItem->GetRandomApplies(), sizeof(rTargetTable.aApplyRandom));
#endif
	memcpy(rTargetTable.aAttr, pItem->GetAttributes(), sizeof(rTargetTable.aAttr));
}

/*		LPITEM -> TPrivateShopSearchData	*/
void CopyItemData(LPITEM pItem, TPrivateShopSearchData& rTargetTable)
{
	rTargetTable.dwShopID = pItem->GetPrivateShop()->GetID();
	strlcpy(rTargetTable.szOwnerName, pItem->GetPrivateShop()->GetOwnerName().c_str(), sizeof(rTargetTable.szOwnerName));
	rTargetTable.dwVnum = pItem->GetVnum();
	rTargetTable.dwCount = pItem->GetCount();
	rTargetTable.wPos = pItem->GetCell();
	rTargetTable.TPrice.llGold = pItem->GetGoldPrice();
	rTargetTable.TPrice.dwCheque = pItem->GetChequePrice();
#ifdef ENABLE_PRIVATE_SHOP_CHANGE_LOOK
    rTargetTable.dwTransmutationVnum = pItem->GetTransmutationVnum();
#endif
#ifdef ENABLE_PRIVATE_SHOP_REFINE_ELEMENT
    rTargetTable.dwRefineElement = pItem->GetRefineElement();
#endif
	memcpy(rTargetTable.alSockets, pItem->GetSockets(), sizeof(rTargetTable.alSockets));
#ifdef ENABLE_PRIVATE_SHOP_APPLY_RANDOM
    memcpy(rTargetTable.aApplyRandom, pItem->GetRandomApplies(), sizeof(rTargetTable.aApplyRandom));
#endif
	memcpy(rTargetTable.aAttr, pItem->GetAttributes(), sizeof(rTargetTable.aAttr));
    rTargetTable.tCheckin = pItem->GetCheckinTime();
	rTargetTable.bState = pItem->GetPrivateShop()->GetState();
}

/*		TPlayerPrivateShopItem -> LPITEM	*/
void CopyItemData(const TPlayerPrivateShopItem& rSourceTable, LPITEM pItem, LPCHARACTER pOwner /* = nullptr */)
{
	pItem->SetGoldPrice(rSourceTable.TPrice.llGold);
	pItem->SetChequePrice(rSourceTable.TPrice.dwCheque);
#ifdef ENABLE_PRIVATE_SHOP_CHANGE_LOOK
    pItem->SetTransmutationVnum(rSourceTable.dwTransmutationVnum);
#endif
#ifdef ENABLE_PRIVATE_SHOP_REFINE_ELEMENT
    pItem->SetRefineElement(rSourceTable.dwRefineElement);
#endif
	pItem->SetSockets(rSourceTable.alSockets);
#ifdef ENABLE_PRIVATE_SHOP_APPLY_RANDOM
    pItem->SetRandomApplies(rSourceTable.aApplyRandom);
#endif
	pItem->SetAttributes(rSourceTable.aAttr);
	pItem->SetCell(pOwner, rSourceTable.wPos);
	pItem->SetCheckinTime(rSourceTable.tCheckin);
}

/*		TPlayerPrivateShopItem -> TPrivateShopItemData		*/
void CopyItemData(const TPlayerPrivateShopItem& rSourceTable, TPrivateShopItemData& rTargetTable)
{
	rTargetTable.dwVnum = rSourceTable.dwVnum;
	rTargetTable.dwCount = rSourceTable.dwCount;
	rTargetTable.wPos = rSourceTable.wPos;
	rTargetTable.TPrice.llGold = rSourceTable.TPrice.llGold;
	rTargetTable.TPrice.dwCheque = rSourceTable.TPrice.dwCheque;
	rTargetTable.tCheckin = rSourceTable.tCheckin;
#ifdef ENABLE_PRIVATE_SHOP_CHANGE_LOOK
    rTargetTable.dwTransmutationVnum = rSourceTable.dwTransmutationVnum;
#endif
#ifdef ENABLE_PRIVATE_SHOP_REFINE_ELEMENT
    rTargetTable.dwRefineElement = rSourceTable.dwRefineElement;
#endif
	memcpy(rTargetTable.alSockets, rSourceTable.alSockets, sizeof(rTargetTable.alSockets));
#ifdef ENABLE_PRIVATE_SHOP_APPLY_RANDOM
    thecore_memcpy(rTargetTable.aApplyRandom, rSourceTable.aApplyRandom, sizeof(rTargetTable.aApplyRandom));
#endif
	memcpy(rTargetTable.aAttr, rSourceTable.aAttr, sizeof(rTargetTable.aAttr));

}

/*		TPrivateShopSale -> TPrivateShopSaleData		*/
void CopyItemData(const TPrivateShopSale& rSourceTable, TPrivateShopSaleData& rTargetTable)
{
	rTargetTable.TItem.dwVnum = rSourceTable.TItem.dwVnum;
	rTargetTable.TItem.dwCount = rSourceTable.TItem.dwCount;
	rTargetTable.TItem.wPos = rSourceTable.TItem.wPos;
	rTargetTable.TItem.TPrice.llGold = rSourceTable.TItem.TPrice.llGold;
	rTargetTable.TItem.TPrice.dwCheque = rSourceTable.TItem.TPrice.dwCheque;
	rTargetTable.TItem.tCheckin = rSourceTable.TItem.tCheckin;
#ifdef ENABLE_PRIVATE_SHOP_CHANGE_LOOK
    rTargetTable.TItem.dwTransmutationVnum = rSourceTable.TItem.dwTransmutationVnum;
#endif
#ifdef ENABLE_PRIVATE_SHOP_REFINE_ELEMENT
    rTargetTable.TItem.dwRefineElement = rSourceTable.TItem.dwRefineElement;
#endif
	memcpy(rTargetTable.TItem.alSockets, rSourceTable.TItem.alSockets, sizeof(rTargetTable.TItem.alSockets));
	memcpy(rTargetTable.TItem.aAttr, rSourceTable.TItem.aAttr, sizeof(rTargetTable.TItem.aAttr));
#ifdef ENABLE_PRIVATE_SHOP_APPLY_RANDOM
    thecore_memcpy(rTargetTable.TItem.aApplyRandom, rSourceTable.TItem.aApplyRandom, sizeof(rTargetTable.TItem.aApplyRandom));
#endif
}

/*		TPrivateShopSale -> TPrivateShopItemData		*/
void CopyItemData(const TPrivateShopSale& rSourceTable, TPrivateShopItemData& rTargetTable)
{
	rTargetTable.dwVnum = rSourceTable.TItem.dwVnum;
	rTargetTable.dwCount = rSourceTable.TItem.dwCount;
	rTargetTable.wPos = rSourceTable.TItem.wPos;
	rTargetTable.TPrice.llGold = rSourceTable.TItem.TPrice.llGold;
	rTargetTable.TPrice.dwCheque = rSourceTable.TItem.TPrice.dwCheque;
	rTargetTable.tCheckin = rSourceTable.TItem.tCheckin;
#ifdef ENABLE_PRIVATE_SHOP_CHANGE_LOOK
    rTargetTable.dwTransmutationVnum = rSourceTable.TItem.dwTransmutationVnum;
#endif
#ifdef ENABLE_PRIVATE_SHOP_REFINE_ELEMENT
    rTargetTable.dwRefineElement = rSourceTable.TItem.dwRefineElement;
#endif
	memcpy(rTargetTable.alSockets, rSourceTable.TItem.alSockets, sizeof(rTargetTable.alSockets));
	memcpy(rTargetTable.aAttr, rSourceTable.TItem.aAttr, sizeof(rTargetTable.aAttr));
#ifdef ENABLE_PRIVATE_SHOP_APPLY_RANDOM
    thecore_memcpy(rTargetTable.aApplyRandom, rSourceTable.TItem.aApplyRandom, sizeof(rTargetTable.aApplyRandom));
#endif
}
