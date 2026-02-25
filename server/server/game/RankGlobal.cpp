//RankGlobal
#include "../common/prodomodefines.h"
#include "stdafx.h"
#ifdef ENABLE_GLOBAL_RANK
#include <sstream>
#include "constants.h"
#include "utils.h"
#include "log.h"
#include "desc.h"
#include "desc_manager.h"
#include "item_manager.h"
#include "p2p.h"
#include "char.h"
#include "ip_ban.h"
#include "war_map.h"
#include "locale_service.h"
#include "config.h"
#include "dev_log.h"
#include "db.h"
#include "desc_client.h"
#include "skill_power.h"
#include "questmanager.h"
#include "RankGlobal.h"

std::vector <stRankGlobal_player> rank_vec;
std::vector <stRankGlobal_player_sort> rank_vec_sorted;
std::vector <stRankGlobal_item> rank_item_vec;
static bool can_load = true;

struct SortVec
{
	bool operator() (const stRankGlobal_player_sort & c1, const stRankGlobal_player_sort & c2) const
	{
		return (c1.count > c2.count);
	}
};

void RankGlobal_send_players(LPCHARACTER ch)
{
	if (!ch || !can_load)
	{
		return;
	}

	ch->ChatPacket(CHAT_TYPE_COMMAND, "GlobalRankData clear_rank");
	ch->ChatPacket(CHAT_TYPE_COMMAND, "GlobalRankMyChar clear");
	ch->ChatPacket(CHAT_TYPE_COMMAND, "GlobalRankData ref_season|%d", (int)quest::CQuestManager::instance().GetEventFlag("RankGlobalSeason"));

	stRankGlobal_player tmp;
	for (int type = 0; type < sizeof(tmp.count) / sizeof(tmp.count[0]); type++)
	{
		int pos = 1;
		for (std::vector<stRankGlobal_player_sort>::const_iterator i = rank_vec_sorted.begin(); i != rank_vec_sorted.end(); ++i)
		{
			if (i->type == type && i->count)
			{
				if (pos <= 10)
				{
					ch->ChatPacket(CHAT_TYPE_COMMAND, "GlobalRankData %d|%s|%d|%d|%d", type, i->name, i->count, i->lv, i->empire);
				}

				if (i->pid == ch->GetPlayerID())
				{
					ch->ChatPacket(CHAT_TYPE_COMMAND, "GlobalRankMyChar %d|%s|%d|%d|%d|%d", type, i->name, pos, i->count, i->lv, i->empire);
				}

				pos += 1;
			}
		}
	}

	ch->ChatPacket(CHAT_TYPE_COMMAND, "GlobalRankRefresh");
}

void RankGlobal_send_items(LPCHARACTER ch)
{
	if (!ch || rank_item_vec.empty())
	{
		return;
	}

	ch->ChatPacket(CHAT_TYPE_COMMAND, "GlobalRankDataItem clear_items");

	for (std::vector<stRankGlobal_item>::const_iterator i = rank_item_vec.begin(); i != rank_item_vec.end(); ++i)
	{
		ch->ChatPacket(CHAT_TYPE_COMMAND, "GlobalRankDataItem %d|%d|%d|%d", i->type, i->pos, i->item_id, i->item_count);
	}
}

void RankGlobal_end_season(LPCHARACTER ch)
{
	if (!ch)
	{
		return;
	}

	quest::CQuestManager::instance().SetEventFlag("RankGlobalSeason", (int)quest::CQuestManager::instance().GetEventFlag("RankGlobalSeason") + 1);

	stRankGlobal_state tmp;
	tmp.option = 1;
	db_clientdesc->DBPacket(HEADER_DG_RANKGLOBAL_LOAD_STATE, 0, &tmp, sizeof(stRankGlobal_state));

	ch->ChatPacket(CHAT_TYPE_INFO, "RankGlobal - Season is over.");
}

void RankGlobal_add_point(LPCHARACTER ch, BYTE type, int count)
{
	stRankGlobal_player add;
	if (type > (sizeof(add.count) / sizeof(add.count[0]) - 1))
	{
		sys_err("error; RankGlobal_add_point bad type.");
		return;
	}

	if (!ch)
	{
		return;
	}

	stRankGlobal_player_sort tmp;
	tmp.pid = ch->GetPlayerID();
	strlcpy(tmp.name, ch->GetName(), sizeof(tmp.name));
	tmp.type = type;
	tmp.count = count;
	tmp.lv = ch->GetLevel();
	tmp.empire = ch->GetEmpire();
	db_clientdesc->DBPacket(HEADER_DG_RANKGLOBAL_ADD_POINT, 0, &tmp, sizeof(stRankGlobal_player_sort));
}


void RankGlobal_DB_load_player(stRankGlobal_player * data)
{
	if (data->pid)
	{
		rank_vec.push_back(*data);
	}
}

void RankGlobal_DB_load_item(stRankGlobal_item * data)
{
	if (data->item_id && data->item_count)
	{
		rank_item_vec.push_back(*data);
	}
}

void RankGlobal_sort()
{
	rank_vec_sorted.clear();

	stRankGlobal_player add;
	for (int type = 0; type < sizeof(add.count) / sizeof(add.count[0]); type++)
	{
		std::vector<stRankGlobal_player_sort> sort_by_type;

		for (size_t it = 0; it < rank_vec.size(); it++)
		{
			stRankGlobal_player_sort tmp;
			if (rank_vec[it].pid && rank_vec[it].count[type])
			{
				tmp.pid = rank_vec[it].pid;
				strlcpy(tmp.name, rank_vec[it].name, sizeof(tmp.name));
				tmp.count = rank_vec[it].count[type];
				tmp.type = type;
				tmp.lv = rank_vec[it].lv;
				tmp.empire = rank_vec[it].empire;
				sort_by_type.push_back(tmp);
			}
		}

		std::sort(sort_by_type.begin(), sort_by_type.end(), SortVec());
		std::copy(sort_by_type.begin(), sort_by_type.end(), back_inserter(rank_vec_sorted));
		sort_by_type.clear();
	}
}

void RankGlobal_DB_load_state(stRankGlobal_state * data)
{
	if (data->option)
	{
		switch (data->option)
		{
		case 1:
			rank_vec.clear();
			can_load = false;
			break;

		case 2:
			RankGlobal_sort();
			can_load = true;
			break;

		case 3:
			rank_item_vec.clear();
			break;

		case 4:
			can_load = true;
			rank_vec_sorted.clear();
			break;

		default:
			break;
		}
	}
}

#endif