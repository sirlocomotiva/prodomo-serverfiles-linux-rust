#include "stdafx.h"
#include "../common/tables.h"
#include "item.h"
#include "item_manager.h"
#include "char.h"
#include "ItemUtils.h"
#include "../common/VnumHelper.h"
#include "skill.h"
#include "motion.h"
#include "utils.h"

#ifdef __ENABLE_SHAMAN_SYSTEM__
	#include "ShamanSystem.h"
#endif


bool CanModifyItem(const CItem* item)
{
	if (item->IsExchanging())
	{
		return false;
	}

	if (item->isLocked())
	{
		return false;
	}

	return true;
}

bool CanStack(const CItem* from, const CItem* to)
{
	if (from == to)
	{
		return false;
	}

	if (from->GetVnum() != to->GetVnum())
	{
		return false;
	}

	if (!to->IsStackable())
	{
		return false;
	}

	for (auto i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
		if (from->GetSocket(i) != to->GetSocket(i))
		{
			return false;
		}

	return true;
}

bool ActivateToggleItem(CHARACTER* ch, CItem* item, bool bIsLoad)
{

	if (item->GetCount() > 1)
	{
		int pos = ch->GetEmptyInventory(item->GetSize());
		if (-1 == pos)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "Not enough space in your inventory", ch);
			return false;
		}

		item->SetCount(item->GetCount() - 1);

		const auto item2 = ITEM_MANAGER::instance().CreateItem(item->GetVnum());
		item2->AddToCharacter(ch, TItemPos(INVENTORY, pos));

		item = item2;
	}

	item->SetSocket(ITEM_SOCKET_TOGGLE_ACTIVE, true);

	item->Lock(true);

	if (item->FindLimit(LIMIT_TIMER_BASED_ON_WEAR))
	{
		item->StartTimerBasedOnWearExpireEvent();
	}

	switch (item->GetSubType())
	{

#ifdef __ENABLE_SHAMAN_SYSTEM__
		case TOGGLE_SHAMAN:
		{
			if (ch->GetShamanSystem())
				ch->GetShamanSystem()->SummonItem(item->GetID(), false);
			
			item->ModifyPoints(true);
			break;
		}
#endif
	}

	ch->CheckMaximumPoints();
	ch->UpdatePacket();

	return true;
}

void DeactivateToggleItem(CHARACTER* ch, CItem* item)
{
	item->SetSocket(ITEM_SOCKET_TOGGLE_ACTIVE, false);

	switch (item->GetSubType())
	{
#ifdef __ENABLE_SHAMAN_SYSTEM__
		case TOGGLE_SHAMAN:
		{
			if (ch->GetShamanSystem())
				ch->GetShamanSystem()->UnsummonItem(item->GetID());
			
			item->ModifyPoints(false);
			break;
		}
#endif
	}

	if (item->FindLimit(LIMIT_TIMER_BASED_ON_WEAR))
	{
		item->StopTimerBasedOnWearExpireEvent();
	}

	item->Lock(false);

	ch->CheckMaximumPoints();
	ch->UpdatePacket();
}

void OnCreateToggleItem(CItem* item)
{
	switch (item->GetSubType())
	{
		// todo
	}
}

void OnLoadToggleItem(CHARACTER* ch, CItem* item)
{
	// Don't do anything if we're not active.
	if (!item->GetSocket(ITEM_SOCKET_TOGGLE_ACTIVE))
	{
		return;
	}

	// If our toggle item has an unique group, check if we already activated
	// another item of the same group.
	const auto group = item->GetValue(ITEM_VALUE_TOGGLE_GROUP);
	if (-1 != group && FindToggleItem(ch, true, item->GetSubType(), group, item))
	{
		item->SetSocket(ITEM_SOCKET_TOGGLE_ACTIVE, false);
		return;
	}
		
	ActivateToggleItem(ch, item, true);
}

void OnRemoveToggleItem(CHARACTER* ch, CItem* item)
{
	// Don't do anything if we're not active.
	if (!item->GetSocket(ITEM_SOCKET_TOGGLE_ACTIVE))
	{
		return;
	}

	DeactivateToggleItem(ch, item);
}

bool OnUseToggleItem(CHARACTER* ch, CItem* item)
{
	if (item->GetSocket(ITEM_SOCKET_TOGGLE_ACTIVE))
	{
		DeactivateToggleItem(ch, item);
		return true;
	}

	// If our toggle item has an unique group, check if we already activated
	// another item of the same group.
	const auto group = item->GetValue(ITEM_VALUE_TOGGLE_GROUP);
	if (-1 != group && FindToggleItem(ch, true, item->GetSubType(), group))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "You cannot activate two items of this kind.", ch);
		return false;
	}

	return ActivateToggleItem(ch, item);
}

void OnCreateItem(CItem* item)
{
	switch (item->GetType())
	{
	case ITEM_TOGGLE:
		OnCreateToggleItem(item);
		break;
	}
}

void OnLoadItem(CHARACTER* ch, CItem* item)
{
	switch (item->GetType())
	{
	case ITEM_TOGGLE:
		OnLoadToggleItem(ch, item);
		break;
	}
}

void OnRemoveItem(CHARACTER* ch, CItem* item)
{
	switch (item->GetType())
	{
	case ITEM_TOGGLE:
		OnRemoveToggleItem(ch, item);
		break;
	}
}

bool OnUseItem(CHARACTER* ch, CItem* item)
{
	switch (item->GetType())
	{
	case ITEM_TOGGLE:
		return OnUseToggleItem(ch, item);
	}

	return true;
}

CItem* FindToggleItem(CHARACTER* ch, bool active,
					  int32_t subType, int32_t group, CItem* except)
{
	if (ch == NULL)
	{
		return nullptr;
	}

	for (int i = 0; i < INVENTORY_MAX_NUM; ++i)
	{
		const auto item = ch->GetInventoryItem(i);
		if (!item || item == except || item->GetType() != ITEM_TOGGLE)
		{
			continue;
		}

		if (subType != -1 && item->GetSubType() != subType)
		{
			continue;
		}

		if (active != static_cast<bool>(item->GetSocket(ITEM_SOCKET_TOGGLE_ACTIVE)))
		{
			continue;
		}

		if (group != -1 &&
				group != item->GetValue(ITEM_VALUE_TOGGLE_GROUP))
		{
			continue;
		}

		return item;
	}

	return nullptr;
}
