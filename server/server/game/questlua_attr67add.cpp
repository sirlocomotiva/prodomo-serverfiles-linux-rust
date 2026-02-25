#include "stdafx.h"

#if defined(__ATTR_6TH_7TH__)
/*
* Title: 6th and 7th Attribute
* Description: Allows your character to add an extra bonus (6th and 7th) to an item.
* Author: Owsap
* Last Date: 2021.08.11 (YMD)
*
* Skype: owsap.
* Discord: Owsap#0905
*
* Web: https://owsap-productions.com/
* GitHub: https://github.com/Owsap
*/

#include "questlua.h"
#include "questmanager.h"
#include "char.h"
#include "item.h"
#include "item_manager.h"

namespace quest
{
	// Syntax: attr67add.holding()
	// It will indicate if the ATTR67_ADD (window) contains an item.
	// (holding to be collected)
	int attr67add_holding(lua_State* L)
	{
		LPCHARACTER pkCh = CQuestManager::instance().GetCurrentCharacterPtr();
		if (pkCh)
		{
			LPITEM pkItem = pkCh->GetAttr67AddItem();
			if (pkItem)
			{
				lua_pushboolean(L, true);
				return 1;
			}
		}
		lua_pushboolean(L, false);
		return 1;
	}

	// Syntax: attr67add.collect()
	// Collect the item from the ATTR67_ADD (window).
	int attr67add_collect(lua_State* L)
	{
		LPCHARACTER pkCh = CQuestManager::instance().GetCurrentCharacterPtr();
		if (pkCh)
		{
			LPITEM pkItem = pkCh->GetAttr67AddItem();
			if (pkItem)
			{
				int iEmptyPos = pkCh->GetEmptyInventory(pkItem->GetSize());
				if (iEmptyPos != -1)
				{
					pkItem->RemoveFromCharacter();
					pkCh->SetItem(TItemPos(INVENTORY, iEmptyPos), pkItem);

					// After collecting, reset all quest flags.
					pkCh->SetQuestFlag("attr67add_item.success", 0);
					pkCh->SetQuestFlag("attr67add_item.wait_time", 0);
					pkCh->SetQuestFlag("attr67add_item.add", 0);
				}
				else
					pkCh->ChatPacket(CHAT_TYPE_INFO, "Not enough inventory space.");
				return 1;
			}
		}
		return 1;
	}

	// Syntax: attr67add.success()
	// Check if the add has succeeded.
	int attr67add_success(lua_State* L)
	{
		LPCHARACTER pkCh = CQuestManager::instance().GetCurrentCharacterPtr();
		if (pkCh)
		{
			LPITEM pkItem = pkCh->GetAttr67AddItem();
			if (pkItem)
			{
				if (pkCh->GetQuestFlag("attr67add_item.success") > 0)
				{
					// Control of adding the rare attribute to avoid adding it again if the
					// character does not have space in the inventory.
					if (pkCh->GetQuestFlag("attr67add_item.add") <= 0)
					{
						pkItem->AddRareAttribute();
						pkCh->SetQuestFlag("attr67add_item.add", 1);
					}
					lua_pushboolean(L, true);
					return 1;
				}
			}
		}
		lua_pushboolean(L, false);
		return 1;
	}

	// Syntax: attr67add.item_vnum()
	// Simply it returns the vnum of the item inside of the ATTR67_ADD (window).
	int attr67add_item_vnum(lua_State* L)
	{
		LPCHARACTER pkCh = CQuestManager::instance().GetCurrentCharacterPtr();
		if (pkCh)
		{
			LPITEM pkItem = pkCh->GetAttr67AddItem();
			if (pkItem)
			{
				lua_pushnumber(L, pkItem->GetVnum());
				return 1;
			}
		}
		lua_pushnumber(L, 0);
		return 1;
	}

	// Syntax: attr67add.window_type()
	// Simply it returns the index of the ATTR67_ADD (window).
	int attr67add_window_type(lua_State* L)
	{
		lua_pushnumber(L, ATTR67_ADD);
		return 1;
	}

	// Syntax: attr67add.window_cell()
	// Since we are just using the ATTR67_ADD (window) for one item at
	// a time, we will simply return 0 as it being the first position
	// of the window.
	int attr67add_window_cell(lua_State* L)
	{
		lua_pushnumber(L, 0);
		return 1;
	}

	void RegisterAttr67AddFunctionTable()
	{
		luaL_reg attr67ad_functions[] =
		{
			{ "holding", attr67add_holding },
			{ "success", attr67add_success },
			{ "collect", attr67add_collect },
			{ "item_vnum", attr67add_item_vnum },
			{ "window_type", attr67add_window_type },
			{ "window_cell", attr67add_window_cell },
			{ NULL, NULL }
		};

		CQuestManager::instance().AddLuaFunctionTable("attr67add", attr67ad_functions);
	}
}
#endif
