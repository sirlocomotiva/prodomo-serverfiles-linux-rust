#include "stdafx.h"
#include "questlua.h"
#include "questmanager.h"
#include "desc_client.h"
#include "char.h"
#include "item_manager.h"
#include "item.h"
#include "cmd.h"
#include "packet.h"
#ifdef ENABLE_DICE_SYSTEM
#include "party.h"
#endif
#if defined(__WORLD_BOSS_EVENT__)
#	include "worldboss.h"
#endif
#include "utils.h"

#undef sys_err
#ifndef __WIN32__
#define sys_err(fmt, args...) quest::CQuestManager::instance().QuestError(__FUNCTION__, __LINE__, fmt, ##args)
#else
#define sys_err(fmt, ...) quest::CQuestManager::instance().QuestError(__FUNCTION__, __LINE__, fmt, __VA_ARGS__)
#endif

extern ACMD(do_in_game_mall);

namespace quest
{
	ALUA(game_set_event_flag)
	{
		CQuestManager & q = CQuestManager::instance();

		if (lua_isstring(L,1) && lua_isnumber(L, 2))
			q.RequestSetEventFlag(lua_tostring(L,1), (int)lua_tonumber(L,2));

		return 0;
	}

	ALUA(game_get_event_flag)
	{
		CQuestManager& q = CQuestManager::instance();

		if (lua_isstring(L,1))
			lua_pushnumber(L, q.GetEventFlag(lua_tostring(L,1)));
		else
			lua_pushnumber(L, 0);

		return 1;
	}

	ALUA(game_request_make_guild)
	{
		CQuestManager& q = CQuestManager::instance();
		LPDESC d = q.GetCurrentCharacterPtr()->GetDesc();
		if (d)
		{
			BYTE header = HEADER_GC_REQUEST_MAKE_GUILD;
			d->Packet(&header, 1);
		}
		return 0;
	}

	ALUA(game_get_safebox_level)
	{
		CQuestManager& q = CQuestManager::instance();
		lua_pushnumber(L, q.GetCurrentCharacterPtr()->GetSafeboxSize()/SAFEBOX_PAGE_SIZE);
		return 1;
	}

	ALUA(game_set_safebox_level)
	{
		CQuestManager& q = CQuestManager::instance();

		//q.GetCurrentCharacterPtr()->ChangeSafeboxSize(3*(int)lua_tonumber(L,-1));
		TSafeboxChangeSizePacket p;
		p.dwID = q.GetCurrentCharacterPtr()->GetDesc()->GetAccountTable().id;
		p.bSize = (int)lua_tonumber(L,-1);
		db_clientdesc->DBPacket(HEADER_GD_SAFEBOX_CHANGE_SIZE,  q.GetCurrentCharacterPtr()->GetDesc()->GetHandle(), &p, sizeof(p));

		q.GetCurrentCharacterPtr()->SetSafeboxSize(SAFEBOX_PAGE_SIZE * (int)lua_tonumber(L,-1));
		return 0;
	}

	ALUA(game_open_safebox)
	{
		CQuestManager& q = CQuestManager::instance();
		LPCHARACTER ch = q.GetCurrentCharacterPtr();
		ch->SetSafeboxOpenPosition();
		ch->ChatPacket(CHAT_TYPE_COMMAND, "ShowMeSafeboxPassword");
		return 0;
	}
	
	
#ifdef ENABLE_GAYA_SYSTEM
	int game_open_gaya_c(lua_State*)
	{
		CQuestManager& q = CQuestManager::instance();
		LPCHARACTER ch = q.GetCurrentCharacterPtr();
		ch->ChatPacket(CHAT_TYPE_COMMAND, "OpenGuiGaya");
		return 0;
	}

	int game_open_gaya_m(lua_State*)
	{
		CQuestManager& q = CQuestManager::instance();
		LPCHARACTER ch = q.GetCurrentCharacterPtr();
		if (ch->CheckItemsFull() == false)
		{
			ch->SetGayaState("system_gaya.gaya_time_world_4", init_gayaTime() + (60*60*5));
			ch->UpdateItemsGayaMarker();
			ch->InfoGayaMarker();
			ch->StartCheckTimeMarket();
		}
		else
		{
			//ch->CheckTimeW();
			ch->InfoGayaMarker();
			ch->StartCheckTimeMarket();

		//	ch->StartCheckTimeMarket();

		}
		ch->ChatPacket(CHAT_TYPE_COMMAND, "OpenGuiGayaMarket");
		return 0;
	}
#endif
	

	ALUA(game_open_mall)
	{
		CQuestManager& q = CQuestManager::instance();
		LPCHARACTER ch = q.GetCurrentCharacterPtr();
		ch->SetSafeboxOpenPosition();
		ch->ChatPacket(CHAT_TYPE_COMMAND, "ShowMeMallPassword");
		return 0;
	}

	ALUA(game_drop_item)
	{
		//
		// Syntax: game.drop_item(50050, 1)
		//
		LPCHARACTER ch = CQuestManager::instance().GetCurrentCharacterPtr();

		DWORD item_vnum = (DWORD) lua_tonumber(L, 1);
		int count = (int) lua_tonumber(L, 2);
		long x = ch->GetX();
		long y = ch->GetY();

		LPITEM item = ITEM_MANAGER::instance().CreateItem(item_vnum, count);

		if (!item)
		{
			sys_err("cannot create item vnum %d count %d", item_vnum, count);
			return 0;
		}

		PIXEL_POSITION pos;
		pos.x = x + number(-200, 200);
		pos.y = y + number(-200, 200);

		item->AddToGround(ch->GetMapIndex(), pos);
		item->StartDestroyEvent();

		return 0;
	}

	ALUA(game_drop_item_with_ownership)
	{
		LPCHARACTER ch = CQuestManager::instance().GetCurrentCharacterPtr();

		LPITEM item = NULL;
		switch (lua_gettop(L))
		{
		case 1:
			item = ITEM_MANAGER::instance().CreateItem((DWORD) lua_tonumber(L, 1));
			break;
		case 2:
		case 3:
			item = ITEM_MANAGER::instance().CreateItem((DWORD) lua_tonumber(L, 1), (int) lua_tonumber(L, 2));
			break;
		default:
			return 0;
		}

		if ( item == NULL )
		{
			return 0;
		}

		if (lua_isnumber(L, 3))
		{
			int sec = (int) lua_tonumber(L, 3);
			if (sec <= 0)
			{
				item->SetOwnership( ch );
			}
			else
			{
				item->SetOwnership( ch, sec );
			}
		}
		else
			item->SetOwnership( ch );

		PIXEL_POSITION pos;
		pos.x = ch->GetX() + number(-200, 200);
		pos.y = ch->GetY() + number(-200, 200);

		item->AddToGround(ch->GetMapIndex(), pos);
		item->StartDestroyEvent();

		return 0;
	}

#ifdef ENABLE_DICE_SYSTEM
	ALUA(game_drop_item_with_ownership_and_dice)
	{
		LPITEM item = NULL;
		switch (lua_gettop(L))
		{
		case 1:
			item = ITEM_MANAGER::instance().CreateItem((DWORD) lua_tonumber(L, 1));
			break;
		case 2:
		case 3:
			item = ITEM_MANAGER::instance().CreateItem((DWORD) lua_tonumber(L, 1), (int) lua_tonumber(L, 2));
			break;
		default:
			return 0;
		}

		if ( item == NULL )
		{
			return 0;
		}

		LPCHARACTER ch = CQuestManager::instance().GetCurrentCharacterPtr();
		if (ch->GetParty())
		{
			FPartyDropDiceRoll f(item, ch);
			f.Process(NULL);
		}

		if (lua_isnumber(L, 3))
		{
			int sec = (int) lua_tonumber(L, 3);
			if (sec <= 0)
			{
				item->SetOwnership( ch );
			}
			else
			{
				item->SetOwnership( ch, sec );
			}
		}
		else
			item->SetOwnership( ch );

		PIXEL_POSITION pos;
		pos.x = ch->GetX() + number(-200, 200);
		pos.y = ch->GetY() + number(-200, 200);

		item->AddToGround(ch->GetMapIndex(), pos);
		item->StartDestroyEvent();

		return 0;
	}
#endif

#if defined(__WORLD_BOSS_EVENT__)
	int game_get_worldboss_min_damage(lua_State* L)
	{
		lua_pushnumber(L, CWorldBoss::instance().GetMinDamage());
		return 1;
	}

	int game_give_worldboss_affect(lua_State* L)
	{
		if (!lua_isnumber(L, 1) || !lua_isnumber(L, 2) || !lua_isnumber(L, 3))
		{
			sys_err("invalid argument");
			return 0;
		}

		BYTE bApplyOn = static_cast<BYTE>(lua_tonumber(L, 1));
		if (bApplyOn >= MAX_APPLY_NUM || bApplyOn < 1)
		{
			sys_err("apply is out of range : %d", bApplyOn);
			return 0;
		}

		const LPCHARACTER c_lpChar = CQuestManager::instance().GetCurrentCharacterPtr();
		if (c_lpChar == nullptr)
			return 0;

		if (c_lpChar->FindAffect(AFFECT_WORLD_BOSS_DAILY_REWARD, bApplyOn))
			return 0;

		long lValue = static_cast<long>(lua_tonumber(L, 2));
		long lDuration = static_cast<long>(lua_tonumber(L, 3));

		c_lpChar->AddAffect(AFFECT_WORLD_BOSS_DAILY_REWARD, aApplyInfo[bApplyOn].bPointType, lValue, AFF_NONE, lDuration, 0, true);

		return 0;
	}
#endif


#ifdef __AURA_SYSTEM__
	//ALUA(game_open_aura_absorb_window)
	int game_open_aura_absorb_window(lua_State* L)
	{
		const LPCHARACTER ch = CQuestManager::instance().GetCurrentCharacterPtr();
		if (NULL == ch)
		{
			sys_err("NULL POINT ERROR");
			return 0;
		}

		ch->OpenAuraRefineWindow(CQuestManager::instance().GetCurrentNPCCharacterPtr(), AURA_WINDOW_TYPE_ABSORB);
		return 0;
	}

	//ALUA(game_open_aura_growth_window)
	int game_open_aura_growth_window(lua_State* L)
	{
		const LPCHARACTER ch = CQuestManager::instance().GetCurrentCharacterPtr();
		if (NULL == ch)
		{
			sys_err("NULL POINT ERROR");
			return 0;
		}

		ch->OpenAuraRefineWindow(CQuestManager::instance().GetCurrentNPCCharacterPtr(), AURA_WINDOW_TYPE_GROWTH);
		return 0;
	}

	//ALUA(game_open_aura_evolve_window)
	int game_open_aura_evolve_window(lua_State* L)
	{
		const LPCHARACTER ch = CQuestManager::instance().GetCurrentCharacterPtr();
		if (NULL == ch)
		{
			sys_err("NULL POINT ERROR");
			return 0;
		}

		ch->OpenAuraRefineWindow(CQuestManager::instance().GetCurrentNPCCharacterPtr(), AURA_WINDOW_TYPE_EVOLVE);
		return 0;
	}
#endif

	ALUA(game_web_mall)
	{
		LPCHARACTER ch = CQuestManager::instance().GetCurrentCharacterPtr();

		if ( ch != NULL )
		{
			do_in_game_mall(ch, const_cast<char*>(""), 0, 0);
		}
		return 0;
	}
	void RegisterGameFunctionTable()
	{
		luaL_reg game_functions[] =
		{
			{ "get_safebox_level",			game_get_safebox_level			},
			{ "request_make_guild",			game_request_make_guild			},
			{ "set_safebox_level",			game_set_safebox_level			},
			{ "open_safebox",				game_open_safebox				},
			{ "open_mall",					game_open_mall					},
			{ "get_event_flag",				game_get_event_flag				},
			{ "set_event_flag",				game_set_event_flag				},
			{ "drop_item",					game_drop_item					},
			{ "drop_item_with_ownership",	game_drop_item_with_ownership	},
#ifdef ENABLE_GAYA_SYSTEM
			{ "open_gaya",					game_open_gaya_c				},
			{ "open_gaya_market",			game_open_gaya_m 				},
#endif
#ifdef ENABLE_DICE_SYSTEM
			{ "drop_item_with_ownership_and_dice",	game_drop_item_with_ownership_and_dice	},
#endif
#ifdef __AURA_SYSTEM__
			{ "open_aura_absorb_window",	game_open_aura_absorb_window	},
			{ "open_aura_growth_window",	game_open_aura_growth_window	},
			{ "open_aura_evolve_window",	game_open_aura_evolve_window	},
#endif
#if defined(__WORLD_BOSS_EVENT__)
			{ "get_worldboss_min_damage",	game_get_worldboss_min_damage 	},
			{ "give_worldboss_affect", 		game_give_worldboss_affect 		},
#endif

			{ "open_web_mall",				game_web_mall					},
			{ NULL,					NULL									}

		};

		CQuestManager::instance().AddLuaFunctionTable("game", game_functions);
	}
}

