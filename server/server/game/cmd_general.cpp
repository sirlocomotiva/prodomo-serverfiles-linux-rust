#include "stdafx.h"
#ifdef __FreeBSD__
#include <md5.h>
#else
#include "../../libthecore/include/xmd5.h"
#endif

#include "utils.h"
#include "config.h"
#include "desc_client.h"
#include "desc_manager.h"
#include "char.h"
#include "char_manager.h"
#include "motion.h"
#include "packet.h"
#include "affect.h"
#include "pvp.h"
#include "start_position.h"
#include "party.h"
#include "guild_manager.h"
#include "p2p.h"
#include "dungeon.h"
#include "messenger_manager.h"
#include "war_map.h"
#include "questmanager.h"
#include "item_manager.h"
#include "monarch.h"
#include "mob_manager.h"
#include "dev_log.h"
#include "item.h"
#include "arena.h"
#include "buffer_manager.h"
#include "unique_item.h"
#include "threeway_war.h"
#include "log.h"
#include "../common/VnumHelper.h"
#include "LettersConfig.h"
#include "../common/prodomodefines.h"
#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
#include "MountSystem.h"
#endif
#ifdef ENABLE_DRAGONSOUL_ALCHEMY_PLUS
#include "DragonSoul.h"
#endif

#ifdef ENABLE_PET_COSTUME_SYSTEM
#include "PetSystem.h"
#endif

#ifdef ENABLE_BATTLE_PASS
#include "BattlePassManager.h"
#endif

#ifdef ENABLE_GLOBAL_RANK
	#include "RankGlobal.h"
#endif

ACMD(do_user_horse_ride)
{
	if (ch->IsObserverMode())
		return;

	if (ch->IsDead() || ch->IsStun())
		return;

	if (ch->IsHorseRiding() == false)
	{
		if (ch->GetMountVnum())
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1054]");
			return;
		}

		if (ch->GetHorse() == NULL)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;501]");
			return;
		}

		ch->StartRiding();
	}
	else
	{
		ch->StopRiding();
	}
}

ACMD(do_user_horse_back)
{
	if (ch->GetHorse() != NULL)
	{
		ch->HorseSummon(false);
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;502]");
	}
	else if (ch->IsHorseRiding() == true)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;504]");
	}
	else
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;501]");
	}
}

ACMD(do_user_horse_feed)
{
	if (ch->GetMyShop())
		return;

	if (ch->GetHorse() == NULL)
	{
		if (ch->IsHorseRiding() == false)
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;501]");
		else
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;505]");
		return;
	}

	DWORD dwFood = ch->GetHorseGrade() + 50054 - 1;

	if (ch->CountSpecifyItem(dwFood) > 0)
	{
		ch->RemoveSpecifyItem(dwFood, 1);
		ch->FeedHorse();
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;506;%s;%s]",
				ITEM_MANAGER::instance().GetTable(dwFood)->szLocaleName,
				"");
	}
	else
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;507;%s]", ITEM_MANAGER::instance().GetTable(dwFood)->szLocaleName);
	}
}

#define MAX_REASON_LEN		128

EVENTINFO(TimedEventInfo)
{
	DynamicCharacterPtr ch;
	int		subcmd;
	int         	left_second;
	char		szReason[MAX_REASON_LEN];

	TimedEventInfo()
	: ch()
	, subcmd( 0 )
	, left_second( 0 )
	{
		::memset( szReason, 0, MAX_REASON_LEN );
	}
};

struct SendDisconnectFunc
{
	void operator () (LPDESC d)
	{
		if (d->GetCharacter())
		{
#ifdef FLUSH_AT_SHUTDOWN
			d->GetCharacter()->SaveReal();
			DWORD pid = d->GetCharacter()->GetPlayerID();
			db_clientdesc->DBPacketHeader(HEADER_GD_FLUSH_CACHE, 0, sizeof(DWORD));
			db_clientdesc->Packet(&pid, sizeof(DWORD));
#endif			
			if (d->GetCharacter()->GetGMLevel() == GM_PLAYER)
				d->GetCharacter()->ChatPacket(CHAT_TYPE_COMMAND, "quit Shutdown(SendDisconnectFunc)");
		}
	}
};

struct DisconnectFunc
{
	void operator () (LPDESC d)
	{
		if (d->GetType() == DESC_TYPE_CONNECTOR)
			return;

		if (d->IsPhase(PHASE_P2P))
			return;

		if (d->GetCharacter())
			d->GetCharacter()->Disconnect("Shutdown(DisconnectFunc)");

		d->SetPhase(PHASE_CLOSE);
	}
};

EVENTINFO(shutdown_event_data)
{
	int seconds;

	shutdown_event_data()
	: seconds( 0 )
	{
	}
};

EVENTFUNC(shutdown_event)
{
	shutdown_event_data* info = dynamic_cast<shutdown_event_data*>( event->info );

	if ( info == NULL )
	{
		sys_err( "shutdown_event> <Factor> Null pointer" );
		return 0;
	}

	int * pSec = & (info->seconds);

	if (*pSec < 0)
	{
		sys_log(0, "shutdown_event sec %d", *pSec);

		if (--*pSec == -10)
		{
			const DESC_MANAGER::DESC_SET & c_set_desc = DESC_MANAGER::instance().GetClientSet();
			std::for_each(c_set_desc.begin(), c_set_desc.end(), DisconnectFunc());
			return passes_per_sec;
		}
		else if (*pSec < -10)
			return 0;

		return passes_per_sec;
	}
	else if (*pSec == 0)
	{
		const DESC_MANAGER::DESC_SET & c_set_desc = DESC_MANAGER::instance().GetClientSet();
		std::for_each(c_set_desc.begin(), c_set_desc.end(), SendDisconnectFunc());
		g_bNoMoreClient = true;
		--*pSec;
		return passes_per_sec;
	}
	else
	{
#ifdef __MULTI_LANGUAGE_SYSTEM__
		SendNotice(LC_TEXT("[LS;508;%d]"), *pSec);
#endif

		--*pSec;
		return passes_per_sec;
	}
}

void Shutdown(int iSec)
{
	if (g_bNoMoreClient)
	{
		thecore_shutdown();
		return;
	}

	CWarMapManager::instance().OnShutdown();

#ifdef __MULTI_LANGUAGE_SYSTEM__
	SendNotice(LC_TEXT("[LS;509;%d]"), iSec);
#endif


	shutdown_event_data* info = AllocEventInfo<shutdown_event_data>();
	info->seconds = iSec;

	event_create(shutdown_event, info, 1);
}

ACMD(do_shutdown)
{
	if (NULL == ch)
	{
		sys_err("Accept shutdown command from %s.", ch->GetName());
	}
	TPacketGGShutdown p;
	p.bHeader = HEADER_GG_SHUTDOWN;
	P2P_MANAGER::instance().Send(&p, sizeof(TPacketGGShutdown));

	Shutdown(10);
}


#ifdef RENEWAL_PICKUP_AFFECT
ACMD(do_pickup_affect)
{
	
	CAffect* affect = ch->FindAffect(AFFECT_PICKUP_ENABLE);
	if (!affect)
	{
		affect = ch->FindAffect(AFFECT_PICKUP_DEACTIVE);
		if (!affect)
			return;
	}
	const long duration = affect->lDuration - time(0);
	const bool isActive = affect->dwType == AFFECT_PICKUP_ENABLE;
	ch->RemoveAffect(affect);
	if (duration <= 0)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "@@(cmd_general)tradus:Auto Pick-up-ul a fost dezactivat.");
		return;
	}
	ch->AddAffect(isActive ? AFFECT_PICKUP_DEACTIVE : AFFECT_PICKUP_ENABLE, POINT_NONE, 0, AFF_NONE, duration, 0, 0, false);
}
#endif


#ifdef ENABLE_MULTI_FARM_BLOCK
ACMD(do_multi_farm)
{
	if (!ch->GetDesc())
		return;

	if (ch->GetProtectTime("multi-farm") > get_global_time())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "@@(cmd_general)tradus:You need be slow! You can try after %d second.", ch->GetProtectTime("multi-farm") - get_global_time());
		return;
	}

	ch->SetProtectTime("multi-farm", get_global_time() + 10);
	CHARACTER_MANAGER::Instance().CheckMultiFarmAccount(ch->GetDesc()->GetHostName(), ch->GetPlayerID(), ch->GetName(), !ch->GetMultiStatus());
}
#endif

EVENTFUNC(timed_event)
{
	TimedEventInfo * info = dynamic_cast<TimedEventInfo *>( event->info );

	if ( info == NULL )
	{
		sys_err( "timed_event> <Factor> Null pointer" );
		return 0;
	}

	LPCHARACTER	ch = info->ch;
	if (ch == NULL) { // <Factor>
		return 0;
	}
	LPDESC d = ch->GetDesc();

	if (info->left_second <= 0)
	{
		ch->m_pkTimedEvent = NULL;
		
		switch (info->subcmd)
		{
			case SCMD_LOGOUT:
			case SCMD_QUIT:
			case SCMD_PHASE_SELECT:
				{
					TPacketNeedLoginLogInfo acc_info;
					acc_info.dwPlayerID = ch->GetDesc()->GetAccountTable().id;

#ifdef __MULTI_LANGUAGE_SYSTEM__
					acc_info.bLanguage = ch->GetDesc()->GetAccountTable().bLanguage;
#endif


					db_clientdesc->DBPacket( HEADER_GD_VALID_LOGOUT, 0, &acc_info, sizeof(acc_info) );

					LogManager::instance().DetailLoginLog( false, ch );
				}
				break;
		}

		switch (info->subcmd)
		{
			case SCMD_LOGOUT:
				if (d)
					d->SetPhase(PHASE_CLOSE);
				break;

			case SCMD_QUIT:
				ch->ChatPacket(CHAT_TYPE_COMMAND, "quit");
				break;

			case SCMD_PHASE_SELECT:
				{
					ch->Disconnect("timed_event - SCMD_PHASE_SELECT");

					if (d)
					{
						d->SetPhase(PHASE_SELECT);
					}
				}
				break;
		}

		return 0;
	}
	else
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;510;%d]", info->left_second);
		--info->left_second;
	}

	return PASSES_PER_SEC(1);
}

ACMD(do_cmd)
{
	if (ch->m_pkTimedEvent)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;511]");
		event_cancel(&ch->m_pkTimedEvent);
		return;
	}

	switch (subcmd)
	{
		case SCMD_LOGOUT:
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;512]");
			break;

		case SCMD_QUIT:
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;513]");
			break;

		case SCMD_PHASE_SELECT:
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;515]");
			break;
	}

	int nExitLimitTime = 10;

	if (ch->IsHack(false, true, nExitLimitTime) &&
		false == CThreeWayWar::instance().IsSungZiMapIndex(ch->GetMapIndex()) &&
	   	(!ch->GetWarMap() || ch->GetWarMap()->GetType() == GUILD_WAR_TYPE_FLAG))
	{
		return;
	}

	switch (subcmd)
	{
		case SCMD_LOGOUT:
		case SCMD_QUIT:
		case SCMD_PHASE_SELECT:
			{
				TimedEventInfo* info = AllocEventInfo<TimedEventInfo>();

				{
						info->left_second = 3;
				}

				info->ch		= ch;
				info->subcmd		= subcmd;
				strlcpy(info->szReason, argument, sizeof(info->szReason));

				ch->m_pkTimedEvent	= event_create(timed_event, info, 1);
			}
			break;
	}
}

ACMD(do_mount)
{
}

ACMD(do_fishing)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
		return;

	ch->SetRotation(atof(arg1));
	ch->fishing();
}

ACMD(do_console)
{
	ch->ChatPacket(CHAT_TYPE_COMMAND, "ConsoleEnable");
}

ACMD(do_restart_now)
{
	if (false == ch->IsDead())
	{
		ch->ChatPacket(CHAT_TYPE_COMMAND, "CloseRestartWindow");
		ch->StartRecoveryEvent();
		return;
	}

	if (ch->GetGold() < 300000)
	{
		ch->ChatPacket(CHAT_TYPE_INFO,"@@(cmd_general)tradus:You have to pay 300000 Gold");
		return;
	}

	if (NULL == ch->m_pkDeadEvent)
		return;

	int iTimeToDead = (event_time(ch->m_pkDeadEvent) / passes_per_sec);

	if (!ch->GetWarMap() || ch->GetWarMap()->GetType() == GUILD_WAR_TYPE_FLAG)
	{
		if (!test_server)
		{
			if (ch->IsHack())
				{
					if ((!ch->GetWarMap() || ch->GetWarMap()->GetType() == GUILD_WAR_TYPE_FLAG) ||
						false == CThreeWayWar::instance().IsSungZiMapIndex(ch->GetMapIndex()))
					{
						ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You paid 300000 Gold"), iTimeToDead - (180 - g_nPortalLimitTime));
						return;
					}
				}
			}

			ch->ChatPacket(CHAT_TYPE_COMMAND, "CloseRestartWindow");
			ch->GetDesc()->SetPhase(PHASE_GAME);
			ch->SetPosition(POS_STANDING);
			ch->StartRecoveryEvent();
			ch->RestartAtSamePos();
			ch->PointChange(POINT_HP, ch->GetMaxHP() - ch->GetHP());
			ch->DeathPenalty(0);
			ch->ReviveInvisible(5);

#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
			ch->CheckMount();
#endif
#ifdef ENABLE_REMOVE_LIMIT_GOLD
			ch->ChangeGold(-300000);
#else
            ch->PointChange(POINT_GOLD, -300000, false);;
#endif
	}
}

ACMD(do_restart)
{
	if (false == ch->IsDead())
	{
		ch->ChatPacket(CHAT_TYPE_COMMAND, "CloseRestartWindow");
		ch->StartRecoveryEvent();
		return;
	}

	if (NULL == ch->m_pkDeadEvent)
		return;

	int iTimeToDead = (event_time(ch->m_pkDeadEvent) / passes_per_sec);

	if (subcmd != SCMD_RESTART_TOWN && (!ch->GetWarMap() || ch->GetWarMap()->GetType() == GUILD_WAR_TYPE_FLAG))
	{
		if (!test_server)
		{
			if (ch->IsHack())
			{
				if (false == CThreeWayWar::instance().IsSungZiMapIndex(ch->GetMapIndex()))
				{
					ch->ChatPacket(CHAT_TYPE_INFO, "[LS;516;%d]", iTimeToDead - (180 - g_nPortalLimitTime));
					return;
				}
			}
#define eFRS_HERESEC	170
			if (iTimeToDead > eFRS_HERESEC)
			{
				ch->ChatPacket(CHAT_TYPE_INFO, "[LS;516;%d]", iTimeToDead - eFRS_HERESEC);
				return;
			}
		}
	}
	if (subcmd == SCMD_RESTART_TOWN)
	{
		if (ch->IsHack())
		{
			if ((!ch->GetWarMap() || ch->GetWarMap()->GetType() == GUILD_WAR_TYPE_FLAG) ||
			   	false == CThreeWayWar::instance().IsSungZiMapIndex(ch->GetMapIndex()))
			{
				ch->ChatPacket(CHAT_TYPE_INFO, "[LS;516;%d]", iTimeToDead - (180 - g_nPortalLimitTime));
				return;
			}
		}

#define eFRS_TOWNSEC	173
		if (iTimeToDead > eFRS_TOWNSEC)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;516;%d]", iTimeToDead - eFRS_TOWNSEC);
			return;
		}
	}
	ch->ChatPacket(CHAT_TYPE_COMMAND, "CloseRestartWindow");
	ch->GetDesc()->SetPhase(PHASE_GAME);
	ch->SetPosition(POS_STANDING);
	ch->StartRecoveryEvent();
	if (1 == quest::CQuestManager::instance().GetEventFlag("threeway_war"))
	{
		if (subcmd == SCMD_RESTART_TOWN || subcmd == SCMD_RESTART_HERE)
		{
			if (true == CThreeWayWar::instance().IsThreeWayWarMapIndex(ch->GetMapIndex()) &&
					false == CThreeWayWar::instance().IsSungZiMapIndex(ch->GetMapIndex()))
			{
				ch->WarpSet(EMPIRE_START_X(ch->GetEmpire()), EMPIRE_START_Y(ch->GetEmpire()));
				ch->ReviveInvisible(5);
#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
				ch->CheckMount();
#endif
#ifdef ENABLE_PET_COSTUME_SYSTEM
				ch->CheckPet();
#endif
				ch->PointChange(POINT_HP, ch->GetMaxHP() - ch->GetHP());
				ch->PointChange(POINT_SP, ch->GetMaxSP() - ch->GetSP());

				return;
			}
			if (true == CThreeWayWar::instance().IsSungZiMapIndex(ch->GetMapIndex()))
			{
				if (CThreeWayWar::instance().GetReviveTokenForPlayer(ch->GetPlayerID()) <= 0)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(cmd_general)tradus:#ERR_COD:1103030623"));
					ch->WarpSet(EMPIRE_START_X(ch->GetEmpire()), EMPIRE_START_Y(ch->GetEmpire()));
				}
				else
				{
					ch->Show(ch->GetMapIndex(), GetSungziStartX(ch->GetEmpire()), GetSungziStartY(ch->GetEmpire()));
				}

				ch->PointChange(POINT_HP, ch->GetMaxHP() - ch->GetHP());
				ch->PointChange(POINT_SP, ch->GetMaxSP() - ch->GetSP());
				ch->ReviveInvisible(5);
#ifdef ENABLE_PET_COSTUME_SYSTEM
				ch->CheckPet();
#endif
#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
				ch->CheckMount();
#endif

				return;
			}
		}
	}

	if (ch->GetDungeon())
		ch->GetDungeon()->UseRevive(ch);

	if (ch->GetWarMap() && !ch->IsObserverMode())
	{
		CWarMap * pMap = ch->GetWarMap();
		DWORD dwGuildOpponent = pMap ? pMap->GetGuildOpponent(ch) : 0;

		if (dwGuildOpponent)
		{
			switch (subcmd)
			{
				case SCMD_RESTART_TOWN:
					sys_log(0, "do_restart: restart town");
					PIXEL_POSITION pos;

					if (CWarMapManager::instance().GetStartPosition(ch->GetMapIndex(), ch->GetGuild()->GetID() < dwGuildOpponent ? 0 : 1, pos))
						ch->Show(ch->GetMapIndex(), pos.x, pos.y);
					else
						ch->ExitToSavedLocation();

					ch->PointChange(POINT_HP, ch->GetMaxHP() - ch->GetHP());
					ch->PointChange(POINT_SP, ch->GetMaxSP() - ch->GetSP());
					ch->ReviveInvisible(5);
#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
					ch->CheckMount();
#endif
#ifdef ENABLE_PET_COSTUME_SYSTEM
					ch->CheckPet();
#endif
					break;

				case SCMD_RESTART_HERE:
					sys_log(0, "do_restart: restart here");
					ch->RestartAtSamePos();
					ch->PointChange(POINT_HP, ch->GetMaxHP() - ch->GetHP());
					ch->PointChange(POINT_SP, ch->GetMaxSP() - ch->GetSP());
					ch->ReviveInvisible(5);

#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
					ch->CheckMount();
#endif
#ifdef ENABLE_PET_COSTUME_SYSTEM
					ch->CheckPet();
#endif
					break;
			}

			return;
		}
	}
	switch (subcmd)
	{
		case SCMD_RESTART_TOWN:
			sys_log(0, "do_restart: restart town");
			PIXEL_POSITION pos;

			if (SECTREE_MANAGER::instance().GetRecallPositionByEmpire(ch->GetMapIndex(), ch->GetEmpire(), pos))
				ch->WarpSet(pos.x, pos.y);
			else
				ch->WarpSet(EMPIRE_START_X(ch->GetEmpire()), EMPIRE_START_Y(ch->GetEmpire()));
			ch->PointChange(POINT_HP, 50 - ch->GetHP());
			ch->DeathPenalty(1);
			break;

		case SCMD_RESTART_HERE:
			sys_log(0, "do_restart: restart here");
			ch->RestartAtSamePos();
			ch->PointChange(POINT_HP, 50 - ch->GetHP());
			ch->DeathPenalty(0);
#ifdef ENABLE_PET_COSTUME_SYSTEM
			ch->CheckPet();
#endif
			ch->ReviveInvisible(5);
			break;
	}
}

#define MAX_STAT g_iStatusPointSetMaxValue

ACMD(do_stat_reset)
{
	ch->PointChange(POINT_STAT_RESET_COUNT, 12 - ch->GetPoint(POINT_STAT_RESET_COUNT));
}

ACMD(do_stat_minus)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
		return;

	if (ch->IsPolymorphed())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;521]");
		return;
	}
	
#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	if (ch->IsSecured())
	{
		ch->ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT("@@(cmd_general)tradus:Acest_cont_este_securizat!"));
		return;
	}
#endif

	if (ch->GetPoint(POINT_STAT_RESET_COUNT) <= 0)
		return;

	if (!strcmp(arg1, "st"))
	{
		if (ch->GetRealPoint(POINT_ST) <= JobInitialPoints[ch->GetJob()].st)
			return;

		ch->SetRealPoint(POINT_ST, ch->GetRealPoint(POINT_ST) - 1);
		ch->SetPoint(POINT_ST, ch->GetPoint(POINT_ST) - 1);
		ch->ComputePoints();
		ch->PointChange(POINT_ST, 0);
	}
	else if (!strcmp(arg1, "dx"))
	{
		if (ch->GetRealPoint(POINT_DX) <= JobInitialPoints[ch->GetJob()].dx)
			return;

		ch->SetRealPoint(POINT_DX, ch->GetRealPoint(POINT_DX) - 1);
		ch->SetPoint(POINT_DX, ch->GetPoint(POINT_DX) - 1);
		ch->ComputePoints();
		ch->PointChange(POINT_DX, 0);
	}
	else if (!strcmp(arg1, "ht"))
	{
		if (ch->GetRealPoint(POINT_HT) <= JobInitialPoints[ch->GetJob()].ht)
			return;

		ch->SetRealPoint(POINT_HT, ch->GetRealPoint(POINT_HT) - 1);
		ch->SetPoint(POINT_HT, ch->GetPoint(POINT_HT) - 1);
		ch->ComputePoints();
		ch->PointChange(POINT_HT, 0);
		ch->PointChange(POINT_MAX_HP, 0);
	}
	else if (!strcmp(arg1, "iq"))
	{
		if (ch->GetRealPoint(POINT_IQ) <= JobInitialPoints[ch->GetJob()].iq)
			return;

		ch->SetRealPoint(POINT_IQ, ch->GetRealPoint(POINT_IQ) - 1);
		ch->SetPoint(POINT_IQ, ch->GetPoint(POINT_IQ) - 1);
		ch->ComputePoints();
		ch->PointChange(POINT_IQ, 0);
		ch->PointChange(POINT_MAX_SP, 0);
	}
	else
		return;

	ch->PointChange(POINT_STAT, +1);
	ch->PointChange(POINT_STAT_RESET_COUNT, -1);
	ch->ComputePoints();
}

ACMD(do_stat)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
		return;

	if (ch->IsPolymorphed())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;521]");
		return;
	}
	
#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	if (ch->IsSecured())
	{
		ch->ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT("@@(cmd_general)tradus:Acest_cont_este_securizat!"));
		return;
	}
#endif

	if (ch->GetPoint(POINT_STAT) <= 0)
		return;

	BYTE idx = 0;

	if (!strcmp(arg1, "st"))
		idx = POINT_ST;
	else if (!strcmp(arg1, "dx"))
		idx = POINT_DX;
	else if (!strcmp(arg1, "ht"))
		idx = POINT_HT;
	else if (!strcmp(arg1, "iq"))
		idx = POINT_IQ;
	else
		return;
	if (ch->GetRealPoint(idx) >= MAX_STAT)
		return;

	ch->SetRealPoint(idx, ch->GetRealPoint(idx) + 1);
	ch->SetPoint(idx, ch->GetPoint(idx) + 1);
	ch->ComputePoints();
	ch->PointChange(idx, 0);

	if (idx == POINT_IQ)
	{
		ch->PointChange(POINT_MAX_HP, 0);
	}
	else if (idx == POINT_HT)
	{
		ch->PointChange(POINT_MAX_SP, 0);
	}

	ch->PointChange(POINT_STAT, -1);
	ch->ComputePoints();
}

ACMD(do_pvp)
{
	if (ch->GetArena() != NULL || CArenaManager::instance().IsArenaMap(ch->GetMapIndex()) == true)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;403]");
		return;
	}

	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	DWORD vid = 0;
	str_to_number(vid, arg1);
	LPCHARACTER pkVictim = CHARACTER_MANAGER::instance().Find(vid);

	if (!pkVictim)
		return;

	if (pkVictim->IsNPC())
		return;

	if (pkVictim->GetArena() != NULL)
	{
		pkVictim->ChatPacket(CHAT_TYPE_INFO, "[LS;522]");
		return;
	}

	CPVPManager::instance().Insert(ch, pkVictim);
}

ACMD(do_guildskillup)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
		return;

	if (!ch->GetGuild())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;523]");
		return;
	}

	CGuild* g = ch->GetGuild();
	TGuildMember* gm = g->GetMember(ch->GetPlayerID());
	if (gm->grade == GUILD_LEADER_GRADE)
	{
		DWORD vnum = 0;
		str_to_number(vnum, arg1);
		g->SkillLevelUp(vnum);
	}
	else
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;524]");
	}
}

ACMD(do_skillup)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
	{
		return;
	}
	
#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	if (ch->IsSecured())
	{
		ch->ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT("@@(cmd_general)tradus:Acest_cont_este_securizat!"));
		return;
	}
#endif
	

	DWORD vnum = 0;
	str_to_number(vnum, arg1);

	if (true == ch->CanUseSkill(vnum))
	{
		ch->SkillLevelUp(vnum);
	}
	else
	{
		switch(vnum)
		{
			case SKILL_HORSE_WILDATTACK:
			case SKILL_HORSE_CHARGE:
			case SKILL_HORSE_ESCAPE:
			case SKILL_HORSE_WILDATTACK_RANGE:

			case SKILL_7_A_ANTI_TANHWAN:
			case SKILL_7_B_ANTI_AMSEOP:
			case SKILL_7_C_ANTI_SWAERYUNG:
			case SKILL_7_D_ANTI_YONGBI:

			case SKILL_8_A_ANTI_GIGONGCHAM:
			case SKILL_8_B_ANTI_YEONSA:
			case SKILL_8_C_ANTI_MAHWAN:
			case SKILL_8_D_ANTI_BYEURAK:

			case SKILL_ADD_HP:
			case SKILL_RESIST_PENETRATE:
#ifdef __7AND8TH_SKILLS__
			case SKILL_ANTI_PALBANG:
			case SKILL_ANTI_AMSEOP:
			case SKILL_ANTI_SWAERYUNG:
			case SKILL_ANTI_YONGBI:
			case SKILL_ANTI_GIGONGCHAM:
			case SKILL_ANTI_HWAJO:
			case SKILL_ANTI_MARYUNG:
			case SKILL_ANTI_BYEURAK:
			case SKILL_HELP_PALBANG:
			case SKILL_HELP_AMSEOP:
			case SKILL_HELP_SWAERYUNG:
			case SKILL_HELP_YONGBI:
			case SKILL_HELP_GIGONGCHAM:
			case SKILL_HELP_HWAJO:
			case SKILL_HELP_MARYUNG:
			case SKILL_HELP_BYEURAK:
#endif
				ch->SkillLevelUp(vnum);
				break;
		}
	}
}
ACMD(do_safebox_close)
{
	ch->CloseSafebox();
}
ACMD(do_safebox_password)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));
	ch->ReqSafeboxLoad(arg1);
}

ACMD(do_safebox_change_password)
{
	char arg1[256];
	char arg2[256];

	two_arguments(argument, arg1, sizeof(arg1), arg2, sizeof(arg2));

	if (!*arg1 || strlen(arg1)>6)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;526]");
		return;
	}

	if (!*arg2 || strlen(arg2)>6)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;526]");
		return;
	}

	TSafeboxChangePasswordPacket p;

	p.dwID = ch->GetDesc()->GetAccountTable().id;
	strlcpy(p.szOldPassword, arg1, sizeof(p.szOldPassword));
	strlcpy(p.szNewPassword, arg2, sizeof(p.szNewPassword));

	db_clientdesc->DBPacket(HEADER_GD_SAFEBOX_CHANGE_PASSWORD, ch->GetDesc()->GetHandle(), &p, sizeof(p));
}

ACMD(do_mall_password)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1 || strlen(arg1) > 6)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;526]");
		return;
	}

	int iPulse = thecore_pulse();

	if (ch->GetMall())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;527]");
		return;
	}

	if (iPulse - ch->GetMallLoadTime() < passes_per_sec * 10)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;528]");
		return;
	}

	ch->SetMallLoadTime(iPulse);

	TSafeboxLoadPacket p;
	p.dwID = ch->GetDesc()->GetAccountTable().id;
	strlcpy(p.szLogin, ch->GetDesc()->GetAccountTable().login, sizeof(p.szLogin));
	strlcpy(p.szPassword, arg1, sizeof(p.szPassword));

	db_clientdesc->DBPacket(HEADER_GD_MALL_LOAD, ch->GetDesc()->GetHandle(), &p, sizeof(p));
}

ACMD(do_mall_close)
{
	if (ch->GetMall())
	{
		ch->SetMallLoadTime(thecore_pulse());
		ch->CloseMall();
		ch->Save();
	}
}

ACMD(do_ungroup)
{
	if (!ch->GetParty())
		return;

	if (!CPartyManager::instance().IsEnablePCParty())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;530]");
		return;
	}

	if (ch->GetDungeon())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;531]");
		return;
	}

	LPPARTY pParty = ch->GetParty();

	if (pParty->GetMemberCount() == 2)
	{
		CPartyManager::instance().DeleteParty(pParty);
	}
	else
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;532]");
		pParty->Quit(ch->GetPlayerID());
	}
}

#ifdef GIFT_SYSTEM
#include "db.h"
#include <string>
#include <boost/algorithm/string.hpp>

ACMD(do_gift_show)
{
	ch->ChatPacket(CHAT_TYPE_COMMAND, "gift_show");
}

bool GetGift(LPCHARACTER ch, DWORD id, bool all = false)
{
	char szSockets[1024] = { '\0' };
	char *tempSockets = szSockets;
	for (int i = 0; i < ITEM_SOCKET_MAX_NUM; i++)
	{
		tempSockets += sprintf(tempSockets, "socket%d", i);

		if (i<ITEM_SOCKET_MAX_NUM - 1)
			tempSockets += sprintf(tempSockets, ",");
	}

	char szAttrs[1024] = { '\0' };
	char *tempAttrs = szAttrs;
	for (int i = 0; i < ITEM_ATTRIBUTE_MAX_NUM; i++)
	{
		if (i < 7)
			tempAttrs += sprintf(tempAttrs, "attrtype%d,attrvalue%d", i, i);
		else
			tempAttrs += sprintf(tempAttrs, "applytype%d,applyvalue%d", i - 7, i - 7);
		if (i<ITEM_ATTRIBUTE_MAX_NUM - 1)
			tempAttrs += sprintf(tempAttrs, ",");
	}
	char query[8192]={0};
	int szLength=0;
	szLength+=snprintf(query+szLength, sizeof(query)-szLength,
#ifdef __CHANGELOOK_SYSTEM__
		"SELECT id,vnum,count,look,"
#else
		"SELECT id,vnum,count,"
#endif
		"%s,%s "
		"FROM player_gift WHERE status='WAIT' AND ",
		szSockets, szAttrs
	);

	if (!all)
		szLength+=snprintf(query+szLength, sizeof(query)-szLength, "id='%d' and owner_id=%d", id, ch->GetPlayerID());
	else
		szLength+=snprintf(query+szLength, sizeof(query)-szLength, "owner_id=%d", ch->GetPlayerID());

	memset(&szSockets, 0, sizeof(szSockets));
	memset(&szAttrs, 0, sizeof(szAttrs));
	std::unique_ptr<SQLMsg> pkMsg(DBManager::instance().DirectQuery(query));
	SQLResult * pRes = pkMsg->Get();
	if (pRes->uiNumRows > 0)
	{
		DWORD vnum, socket[ITEM_SOCKET_MAX_NUM], attr[ITEM_ATTRIBUTE_MAX_NUM][2];
		MYSQL_ROW row;
		bool force = false;
		while ((row = mysql_fetch_row(pRes->pSQLResult)) != NULL)
		{
			int col = 0;
			long long count = 0;
#ifdef __CHANGELOOK_SYSTEM__
			int look = 0;
#endif
			str_to_number(id, row[col++]);
			str_to_number(vnum, row[col++]);
			str_to_number(count, row[col++]);
#ifdef __CHANGELOOK_SYSTEM__
			str_to_number(look, row[col++]);
#endif
			
			if (vnum == 1)
			{
				long long nTotalMoney = ch->GetGold() + count;

				if (GOLD_MAX_MAX <= nTotalMoney)
				{
					sys_err("[OVERFLOW_GOLD] Overflow (GOLD_MAX_MAX) id %u name %s", ch->GetPlayerID(), ch->GetName());
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You reached the yang limit."));
					return true;
				}
#ifdef ENABLE_REMOVE_LIMIT_GOLD
					ch->ChangeGold(count);
#else
					ch->PointChange(POINT_GOLD, gold, false);
#endif

			}
			else {
				if (force)
					continue;
				for (int s = 0; s < ITEM_SOCKET_MAX_NUM; s++)
					str_to_number(socket[s], row[col++]);

				for (int a = 0; a < ITEM_ATTRIBUTE_MAX_NUM; a++)
				{
					str_to_number(attr[a][0], row[col++]);
					str_to_number(attr[a][1], row[col++]);
				}
				LPITEM item = ITEM_MANAGER::instance().CreateItem(vnum, count, 0, true);
				if (item)
				{
#ifdef __CHANGELOOK_SYSTEM__
					item->SetTransmutation(look);
#endif
					for (int s = 0; s < ITEM_SOCKET_MAX_NUM; s++)
						item->SetSocket(s, socket[s], false);
					item->ClearAttribute();
					for (int a = 0; a < ITEM_ATTRIBUTE_MAX_NUM; a++)
						item->SetForceAttribute(a, attr[a][0], attr[a][1]);

					int iEmptyPos;
					if (item->IsDragonSoul()) {
						iEmptyPos = ch->GetEmptyDragonSoulInventory(item); 
					}
					else {
						iEmptyPos = ch->GetEmptyInventory(item->GetSize());
					}
					if (item->IsDragonSoul())
					{
						if (iEmptyPos != -1)
						{
							item->AddToCharacter(ch, TItemPos(DRAGON_SOUL_INVENTORY, iEmptyPos));
						}
						else
						{
							M2_DESTROY_ITEM(item);
							if (!ch->DragonSoul_IsQualified())
							{
								ch->ChatPacket(CHAT_TYPE_INFO, "<Gift> %s #3", LC_TEXT("UNKNOW_ERROR"));
								force = true;
								continue;
							}
							else {
								ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("INVENTORY_FULL_ERROR"));
								force = true;
								continue;
							}
						}
					}
					else
					{
						if (iEmptyPos != -1)
						{										
									item->AddToCharacter(ch, TItemPos(INVENTORY, iEmptyPos));

								
						}
						else
						{
							M2_DESTROY_ITEM(item);
							ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("INVENTORY_FULL_ERROR"));
							force = true;
							continue;
						}
					}
				}
				else
				{
					ch->ChatPacket(CHAT_TYPE_INFO, "<Gift> %s #4", LC_TEXT("UNKNOW_ERROR"));
					force = true;
					continue;
				}

			}
			std::unique_ptr<SQLMsg> (DBManager::instance().DirectQuery("UPDATE player_gift SET status='OK',date_get=NOW() where id=%d;", id));
		}
		if (all)
			ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("GIFT_ADD_ALL_SUCCESS"));
		else
			ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("GIFT_ADD_SUCCESS"));
		memset(&socket, 0, sizeof(socket));
		memset(&attr, 0, sizeof(attr));
		return true;
	}
	return false;
}

ACMD(do_gift_get)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));
	bool full = !isdigit(*arg1);
	DWORD id;
	str_to_number(id, arg1);
	if (GetGift(ch, id, full))
	{
		ch->RefreshGift();
		ch->LoadGiftPage(ch->GetLastGiftPage());
	}
}

ACMD(do_gift_refresh)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));
	if (!*arg1)
	{
		if (ch->GetGiftPages() > 0)
			ch->ChatPacket(CHAT_TYPE_COMMAND, "gift_info %d", ch->GetGiftPages());
	}
	else {
		int page;
		str_to_number(page, arg1);
		ch->LoadGiftPage(page);

	}
}
#endif

#ifdef OFFLINE_SHOP
void DeleteShop(DWORD id)
{
	CharacterVectorInteractor i;
	if (CHARACTER_MANAGER::instance().GetCharactersByRaceNum(30000, i))
	{
		CharacterVectorInteractor::iterator it = i.begin();

		while (it != i.end()) {
			LPCHARACTER pc = *it++;
			if (pc)
				if (pc->GetRaceNum() == 30000 && pc->GetPrivShop() == id) {
					pc->DeleteMyShop();
					return;
				}


		}
	}
	TPacketShopClose packet;
	packet.shop_id = id;
	packet.pid = 0;
	packet.error = false;
	packet.reload = false;
	db_clientdesc->DBPacket(HEADER_GD_SHOP_CLOSE, 0, &packet, sizeof(packet));
}

ACMD(do_shop_open)
{
	ch->ChatPacket(CHAT_TYPE_COMMAND, "OpenPrivateShop");
}

ACMD(do_close_shop)
{
	if (ch->IsObserverMode() || ch->GetExchange())
		return;
	DWORD id;
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));
	if (ch->GetMyShop())
	{
		ch->CloseMyShop();
	}
	else {
		str_to_number(id, arg1);
		char pid[4096];
		sprintf(pid, "and player_id=%d", ch->GetPlayerID());
		if (id > 0)
		{
			std::unique_ptr<SQLMsg> pkMsg(DBManager::instance().DirectQuery("SELECT player_id,channel from player_shop WHERE id = %d %s", id, (ch->GetGMLevel() >= SHOP_GM_PRIVILEGES ? "" : pid)));
			SQLResult * pRes = pkMsg->Get();
			if (pRes->uiNumRows > 0)
				DeleteShop(id);
		}
	}
}
#include "banword.h"
ACMD(do_set_name_shop)
{
	if (ch->IsObserverMode() || ch->GetExchange())
		return;
	DWORD id;
	char arg1[256];
	char arg2[256];
	two_arguments(argument, arg1, sizeof(arg1), arg2, sizeof(arg2));
	if (!*arg1 || !*arg2)
		return;
	str_to_number(id, arg1);
	char pid[4096];
	sprintf(pid, "and player_id=%d", ch->GetPlayerID());
	std::string m_stShopSign(arg2);
	boost::replace_all(m_stShopSign, "%", "%%");
	if (m_stShopSign.length()>SHOP_SIGN_MAX_LEN)
		m_stShopSign.resize(SHOP_SIGN_MAX_LEN);
	if (m_stShopSign.length()<1) {
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The store cannot be opened with a store name containing slang or sweetfish."));
		return;
	}
#ifdef STRING_PROTECTION
	if (CBanwordManager::instance().CheckString(m_stShopSign.c_str(), m_stShopSign.length()) != "")
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The store cannot be opened with a store name containing slang or sweetfish."));
		return;
	}
#else

	if (CBanwordManager::instance().CheckString(m_stShopSign.c_str(), m_stShopSign.length()))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The store cannot be opened with a store name containing slang or sweetfish."));
		return;
	}
#endif
	if (id>0)
	{
		std::unique_ptr<SQLMsg> pkMsg(DBManager::instance().DirectQuery("SELECT id,player_id,channel from player_shop WHERE id = %d %s", id, (ch->GetGMLevel() >= SHOP_GM_PRIVILEGES ? "" : pid)));
		SQLResult * pRes = pkMsg->Get();
		if (pRes->uiNumRows > 0)
		{
			char szName[SHOP_SIGN_MAX_LEN * 2+1]={0};
			DBManager::instance().EscapeString(szName, sizeof(szName), m_stShopSign.c_str(), m_stShopSign.length());
			std::unique_ptr<SQLMsg> (DBManager::instance().DirectQuery("UPDATE player_shop SET name='%s' WHERE id=%d", szName, id));
			memset(&szName, 0, sizeof(szName));
			ch->LoadPrivShops();
			CharacterVectorInteractor i;
			if (CHARACTER_MANAGER::instance().GetCharactersByRaceNum(30000, i))
			{
				CharacterVectorInteractor::iterator it = i.begin();

				while (it != i.end()) {
					LPCHARACTER pc = *it++;
					if (pc)
						if (pc->GetMyShop() && pc->GetPrivShop() == id) {
							pc->SetShopSign(m_stShopSign.c_str());
							return;
						}


				}
			}
			TPacketShopName packet;
			packet.shop_id = id;
			strlcpy(packet.szSign, m_stShopSign.c_str(), sizeof(packet.szSign) - 1);
			db_clientdesc->DBPacket(HEADER_GD_SHOP_NAME, 0, &packet, sizeof(packet));


		}
	}
}

ACMD(do_shop_refresh)
{
	ch->SendShops();
}

ACMD(do_shop_yang)
{
	if (ch->IsObserverMode() || ch->GetExchange())
		return;
	DWORD id;
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));
	if (!*arg1)
		return;
	str_to_number(id, arg1);
	if (*arg1)
	{
		std::unique_ptr<SQLMsg> pkMsg(DBManager::instance().DirectQuery("SELECT gold from player_shop WHERE id = %d", id));
		SQLResult * pRes = pkMsg->Get();
		if (pRes->uiNumRows>0)
		{
			MYSQL_ROW row;
			while ((row = mysql_fetch_row(pRes->pSQLResult)) != NULL)
			{
				long long gold;
				str_to_number(gold, row[0]);
				if (gold >0)
				{
					long long nTotalMoney = ch->GetGold() + gold;

					if (GOLD_MAX_MAX <= nTotalMoney)
					{
						sys_err("[OVERFLOW_GOLD] Overflow (GOLD_MAX_MAX) id %u name %s", ch->GetPlayerID(), ch->GetName());
						ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You reached the yang limit."));
						return;
					}
#ifdef ENABLE_REMOVE_LIMIT_GOLD
					ch->ChangeGold(gold);
#else
					ch->PointChange(POINT_GOLD, gold, false);
#endif

					TPrivShop s = ch->GetPrivShopTable(id);
					s.gold = s.gold - gold;
					s.rest_count = s.item_count - 1;
					ch->UpdatePrivShopTable(s.shop_id, s);
					ch->SendShops();
#ifdef ENABLE_REMOVE_LIMIT_GOLD
                    std::unique_ptr<SQLMsg> (DBManager::instance().DirectQuery("UPDATE player_shop SET gold=gold - %lld WHERE id = %d", gold, id));
#else
					std::unique_ptr<SQLMsg> (DBManager::instance().DirectQuery("UPDATE player_shop SET gold=gold - %d WHERE id = %d", gold, id));
#endif
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("SHOP_YANG_ADD"));
					char szLog[1024]={0};
#ifdef ENABLE_REMOVE_LIMIT_GOLD
                    sprintf(szLog,"Get %lld", gold);
#else
					sprintf(szLog,"Get %d", gold);
#endif
					LogManager::instance().CharLog(ch, s.shop_id, "OFFLINE_SHOP_YANG", szLog);
					memset(&szLog,0, sizeof(szLog));
				}
			}
		}
	}
}
#include <boost/algorithm/string.hpp>
LPCHARACTER _GetOfflineShop(DWORD shop_id)
{
	CharacterVectorInteractor i;
	if (CHARACTER_MANAGER::instance().GetCharactersByRaceNum(30000, i))
	{
		CharacterVectorInteractor::iterator it = i.begin();
		while (it != i.end()) {
			LPCHARACTER pc = *it++;
			if (pc)
				if (pc->GetRaceNum() == 30000 && pc->GetPrivShop() == shop_id)
					return pc;
		}
	}
	return NULL;
}

void _UpdateOfflineShop(DWORD shop_id, bool tick, bool shop_locked, bool refresh = false)
{
	LPCHARACTER pc = _GetOfflineShop(shop_id);
	if (pc && pc->GetPrivShop() == shop_id && pc->GetMyShop()) {
		pc->SetShopEditModeTick();
		if(refresh)
			pc->UpdateShopItems();
		else if(shop_locked || !tick && !shop_locked)
			pc->SetShopEditMode(shop_locked);
		return;
	}
	TPacketShopUpdateItem packet;
	packet.shop_id = shop_id;
	packet.tick = tick;
	packet.refresh = refresh;
	packet.shop_locked = shop_locked;
	db_clientdesc->DBPacket(HEADER_GD_SHOP_UPDATE_ITEM, 0, &packet, sizeof(packet));
}

ACMD(do_shop_update_item)
{

	if (ch->IsObserverMode() || ch->GetExchange())
		return;
	extern ACMD(do_shop_refresh_items);
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));
	if (!*arg1)
		return;
	std::vector<std::string> args;
	boost::split(args, arg1, boost::is_any_of("|"));
	if (args.size()<2) {
		return;
	}
	DWORD shop_id;
	str_to_number(shop_id, args[1].c_str());
	LPCHARACTER pc_shop = _GetOfflineShop(shop_id);
	bool edit_mode;
	if(pc_shop)
	{
		if(pc_shop->GetPrivShopOwner() != ch->GetPlayerID())
			return;
		edit_mode = pc_shop->GetShopEditMode();
	}else{
		std::unique_ptr<SQLMsg> pkMsg(DBManager::instance().DirectQuery("SELECT edit_mode from player_shop WHERE id = %d and player_id=%d", shop_id, ch->GetPlayerID()));
		SQLResult * pRes = pkMsg->Get();
		if (!pRes->uiNumRows)
			return;
		MYSQL_ROW row;
		if((row = mysql_fetch_row(pRes->pSQLResult)) != NULL)
			str_to_number(edit_mode,row[0]);
	}
	{
		/*
		/update_shop_item edit|%d
		/update_shop_item price|%d|%d|%d)
		/update_shop_item remove|%d|%d
		/update_shop_item add|%d|%d|%d|%d|%s

		*/
		if (args.size() == 2)
		{
			_UpdateOfflineShop(shop_id, args[0] == "edit_tick", args[0] == "edit_start");
			return;
		}
		if(!edit_mode)
		{
			do_shop_refresh_items(ch,args[1].c_str(),0,0);
			ch->ChatPacket(CHAT_TYPE_INFO,LC_TEXT("SHOP_NO_EDIT_MODE"));
			return;
		}
		if (args.size() == 4)
		{
			DWORD item_id;
			str_to_number(item_id, args[2].c_str());
#ifdef ENABLE_REMOVE_LIMIT_GOLD
            long long price;
#else
			int price;
#endif
			str_to_number(price, args[3].c_str());
			if (price <= 0 || item_id <= 0)
				return;
			{
				std::unique_ptr<SQLMsg> pkMsg(DBManager::instance().DirectQuery("SELECT id from player_shop_items WHERE id = %d and shop_id=%d", item_id, shop_id));
				SQLResult * pRes = pkMsg->Get();
				if (pRes->uiNumRows > 0)
#ifdef ENABLE_REMOVE_LIMIT_GOLD
					std::unique_ptr<SQLMsg> (DBManager::instance().DirectQuery("UPDATE player_shop_items SET price=%lld WHERE id = %d and shop_id=%d", price, item_id, shop_id));

#else
					std::unique_ptr<SQLMsg> (DBManager::instance().DirectQuery("UPDATE player_shop_items SET price=%d WHERE id = %d and shop_id=%d", price, item_id, shop_id));
#endif



			}
#ifdef ENABLE_SHOP_SEARCH
			extern void UpdateSearchShopItem(DWORD);
			TPrivShop pshop = ch->GetPrivShopTable(shop_id);
			if (pshop.shop_vid>0)
				UpdateSearchShopItem(pshop.shop_vid);
#endif
		}
		else if (args.size() == 3)
		{
			DWORD item_id;
			str_to_number(item_id, args[2].c_str());
			if (item_id <= 0)
				return;
			{

				std::string shop_name("#PLAYER_NAME# Shop");
				boost::replace_all(shop_name, "#PLAYER_NAME#", ch->GetName());
				boost::replace_all(shop_name, "#ID#", "");


				char szSockets[1024] = { '\0' };
				char *tempSockets = szSockets;
				for (int i = 0; i < ITEM_SOCKET_MAX_NUM; i++)
				{
					tempSockets += sprintf(tempSockets, "socket%d", i);

					if (i<ITEM_SOCKET_MAX_NUM - 1)
						tempSockets += sprintf(tempSockets, ",");
				}

				char szAttrs[1024] = { '\0' };
				char *tempAttrs = szAttrs;
				for (int i = 0; i < ITEM_ATTRIBUTE_MAX_NUM; i++)
				{
					if (i < 7)
						tempAttrs += sprintf(tempAttrs, "attrtype%d,attrvalue%d", i, i);
					else
						tempAttrs += sprintf(tempAttrs, "applytype%d,applyvalue%d", i - 7, i - 7);
					if (i<ITEM_ATTRIBUTE_MAX_NUM - 1)
						tempAttrs += sprintf(tempAttrs, ",");
				}
				std::unique_ptr<SQLMsg> pkMsg(DBManager::instance().DirectQuery(
#ifdef __CHANGELOOK_SYSTEM__
					"SELECT vnum,count,look,"
#else
					"SELECT vnum,count,"
#endif
#ifdef ENABLE_SHOP_SEARCH
					"display_pos,"
#endif
					"(UNIX_TIMESTAMP(now()) - UNIX_TIMESTAMP(date_add) < 10) as date_add,"
					"%s,%s "
					"FROM player_shop_items where shop_id='%d' and id=%d", szSockets, szAttrs, shop_id, item_id));
				SQLResult * pRes = pkMsg->Get();
				if (pRes && pRes->uiNumRows>0)
				{
					MYSQL_ROW row;
					while ((row = mysql_fetch_row(pRes->pSQLResult)) != NULL)
					{

						int col = 0;
						char query[8192];
						sprintf(query, "INSERT INTO player_gift SET owner_id=%d,reason='%s',`from`=replace(\"%s\",' ','_'),status='WAIT',date_add=NOW()", ch->GetPlayerID(), LC_TEXT("SHOP_ITEM_REASON"), shop_name.c_str());
						sprintf(query, "%s, vnum='%s'", query, row[col++]);
						sprintf(query, "%s, count='%s'", query, row[col++]);
#ifdef __CHANGELOOK_SYSTEM__
						sprintf(query, "%s, look='%s'", query, row[col++]);
#endif
#ifdef ENABLE_SHOP_SEARCH
						BYTE bPos;
						str_to_number(bPos, row[col++]);
#endif
						if(std::stoi(row[col++]) == 1)
						{
							ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT( "SHOP_ITEM_DELETE_TOO_FAST"));
							do_shop_refresh_items(ch,args[1].c_str(),0,0);
							memset(&query,0,sizeof(query));
							return;
						}
						for (int s = 0; s < ITEM_SOCKET_MAX_NUM; s++)
							sprintf(query, "%s, socket%d='%s'", query, s, row[col++]);
						for (int ia = 0; ia < ITEM_ATTRIBUTE_MAX_NUM; ia++)
						{
							if (ia < 7)
							{
								sprintf(query, "%s, attrtype%d='%s'", query, ia, row[col++]);
								sprintf(query, "%s, attrvalue%d='%s'", query, ia, row[col++]);
							}
							else
							{
								sprintf(query, "%s, applytype%d='%s'", query, ia - 7, row[col++]);
								sprintf(query, "%s, applyvalue%d='%s'", query, ia - 7, row[col++]);
							}
						}
						std::unique_ptr<SQLMsg> pkMsg(DBManager::instance().DirectQuery(query));
						SQLResult * pRes = pkMsg->Get();
						DWORD gift_id = pRes->uiInsertID;
						if (gift_id > 0)
						{
							GetGift(ch, gift_id);
							std::unique_ptr<SQLMsg> (DBManager::instance().DirectQuery("delete from player_shop_items where id='%d'", item_id));
#ifdef ENABLE_SHOP_SEARCH
							extern void ClearSearchShopItem(BYTE, DWORD);
							TPrivShop pshop = ch->GetPrivShopTable(shop_id);
							if (pshop.shop_vid>0)
								ClearSearchShopItem(bPos, pshop.shop_vid);
#endif
						}
						memset(&query, 0, sizeof(query));
					}
				}

				memset(&szSockets, 0, sizeof(szSockets));
				memset(&szAttrs, 0, sizeof(szAttrs));
			}
		}

		else if (args.size() == 6)
		{
			BYTE display_pos;
			str_to_number(display_pos, args[2].c_str());

			if (display_pos < 0)
				return;
			WORD pos;
			str_to_number(pos, args[3].c_str());
			if (pos < 0)
				return;

			BYTE window_type;
			str_to_number(window_type, args[4].c_str());
			if (window_type < 0)
				return;

			long long price;

			str_to_number(price, args[5].c_str());
			
			if (price <= 0)
				return;
			
			LPITEM item = ch->GetItem(TItemPos(window_type, pos));
			if (item)
			{
				const TItemTable * item_table = item->GetProto();

				if (item_table && (IS_SET(item_table->dwAntiFlags, ITEM_ANTIFLAG_GIVE | ITEM_ANTIFLAG_MYSHOP)))
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("This item is not for sale."));
					do_shop_refresh_items(ch,args[1].c_str(),0,0);
					return;
				}

				if (item->IsEquipped())
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Equipped items can not be sale in the personal store."));
					do_shop_refresh_items(ch,args[1].c_str(),0,0);
					return;
				}

				if (item->isLocked())
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Items in used can not be sale in the personal store."));
					do_shop_refresh_items(ch,args[1].c_str(),0,0);
					return;
				}
				
				if (item->GetOwner() != ch)
				{
					return;
				}
				
				char query[1024];
				sprintf(query, "INSERT INTO player_shop_items SET");
				sprintf(query, "%s player_id='%d'", query, ch->GetPlayerID());
				sprintf(query, "%s, shop_id='%d'", query, shop_id);
				sprintf(query, "%s, vnum='%d'", query, item->GetVnum());
				sprintf(query, "%s, count='%d'", query, item->GetCount());
#ifdef __CHANGELOOK_SYSTEM__
				sprintf(query, "%s, look='%d'", query, item->GetTransmutation());
#endif
				sprintf(query, "%s, price='%lld'", query, price);
				sprintf(query, "%s, display_pos='%u'", query, display_pos);
				
				for (int s = 0; s < ITEM_SOCKET_MAX_NUM; s++)
				{
					sprintf(query, "%s, socket%d='%ld'", query, s, item->GetSocket(s));

				}
				for (int ia = 0; ia < ITEM_ATTRIBUTE_MAX_NUM; ia++)
				{
					const TPlayerItemAttribute& attr = item->GetAttribute(ia);
					if (ia < 7)
					{
						sprintf(query, "%s, attrtype%d='%u'", query, ia, attr.bType);
						sprintf(query, "%s, attrvalue%d='%d'", query, ia, attr.sValue);
					}
					else
					{
						sprintf(query, "%s, applytype%d='%u'", query, ia - 7, attr.bType);
						sprintf(query, "%s, applyvalue%d='%d'", query, ia - 7, attr.sValue);
					}
				}

				std::unique_ptr<SQLMsg> (DBManager::instance().DirectQuery(query));
				ITEM_MANAGER::Instance().RemoveItem(item, "Priv shop");
				memset(&query, 0, sizeof(query));

			}
			else
				return;
			do_shop_refresh_items(ch,args[1].c_str(),0,0);
		}
		else
			return;
		_UpdateOfflineShop(shop_id, false, true, true);
		do_shop_refresh_items(ch,args[1].c_str(),0,0);
	}
}

ACMD(do_shop_refresh_items)
{
	if (ch->IsObserverMode() || ch->GetExchange())
		return;

	DWORD id;
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));
	if (!*arg1)
		return;
	str_to_number(id, arg1);
	char szSockets[1024] = { '\0' };
	char *tempSockets = szSockets;
	for (int i = 0; i < ITEM_SOCKET_MAX_NUM; i++)
	{
		tempSockets += sprintf(tempSockets, "socket%d", i);

		if (i<ITEM_SOCKET_MAX_NUM - 1)
			tempSockets += sprintf(tempSockets, ",");
	}
	char szAttrs[1024] = { '\0' };
	char *tempAttrs = szAttrs;
	for (int i = 0; i < ITEM_ATTRIBUTE_MAX_NUM; i++)
	{
		if (i < 7)
			tempAttrs += sprintf(tempAttrs, "attrtype%d,attrvalue%d", i, i);
		else
			tempAttrs += sprintf(tempAttrs, "applytype%d,applyvalue%d", i - 7, i - 7);
		if (i<ITEM_ATTRIBUTE_MAX_NUM - 1)
			tempAttrs += sprintf(tempAttrs, ",");
	}
	std::unique_ptr<SQLMsg> pkMsg(DBManager::instance().DirectQuery(
		"SELECT "
#ifdef __CHANGELOOK_SYSTEM__
		"id,vnum,count,display_pos,price,look,"
#else
		"id,vnum,count,display_pos,price,"
#endif
		"%s,%s "
		"FROM player_shop_items WHERE shop_id='%d'", szSockets, szAttrs, id));
	SQLResult * pRes = pkMsg->Get();
	BYTE bItemCount = pRes->uiNumRows;
	ch->ChatPacket(CHAT_TYPE_COMMAND, "shop_item_clear");
	if (bItemCount>0)
	{
		bItemCount = 0;
		MYSQL_ROW row;
		int c = 0;
		while ((row = mysql_fetch_row(pRes->pSQLResult)) != NULL)
		{
			int max_col = 5;
#ifdef __CHANGELOOK_SYSTEM__
			max_col++;
#endif
			int col = max_col;
			char attrs[1024] = { '\0' };
			char sockets[1024] = { '\0' };
			for (int i = 0; i<ITEM_SOCKET_MAX_NUM; i++)
				sprintf(sockets, "%s%s%s", sockets, row[col++], (i<ITEM_SOCKET_MAX_NUM - 1 ? "|" : ""));

			for (int i = 0; i<ITEM_ATTRIBUTE_MAX_NUM; i++)
				sprintf(attrs, "%s%s,%s%s", attrs, row[col++], row[col++], (i<ITEM_ATTRIBUTE_MAX_NUM - 1 ? "|" : ""));

			char szCMD[1024] = {0};
			int szLength = 0;
			szLength+=snprintf(szCMD+szLength, sizeof(szCMD)-szLength, "shop_item ");
			for(int i = 0; i < max_col; i++)
				szLength+=snprintf(szCMD+szLength, sizeof(szCMD)-szLength, "%s#", row[i]);

			szLength+=snprintf(szCMD+szLength, sizeof(szCMD)-szLength, "%s#", sockets);
			szLength+=snprintf(szCMD+szLength, sizeof(szCMD)-szLength, "%s#", attrs);

			ch->ChatPacket(CHAT_TYPE_COMMAND, szCMD);
			memset(&attrs, 0, sizeof(attrs));
			memset(&sockets, 0, sizeof(sockets));
			memset(&szCMD, 0, sizeof(szCMD));
		}
	}
	memset(&szSockets, 0, sizeof(szSockets));
	memset(&szAttrs, 0, sizeof(szAttrs));

}

ACMD(do_shop_update)
{
	if (ch->IsObserverMode() || ch->GetExchange())
		return;

	DWORD id;
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));
	if (!*arg1)
		return;
	str_to_number(id, arg1);
	if (*arg1)
	{
		std::unique_ptr<SQLMsg> pkMsg(DBManager::instance().DirectQuery("SELECT player_id from player_shop WHERE id = %d", id));
		SQLResult * pRes = pkMsg->Get();
		if (pRes->uiNumRows>0)
		{
			CharacterVectorInteractor i;
			if (CHARACTER_MANAGER::instance().GetCharactersByRaceNum(30000, i))
			{
				CharacterVectorInteractor::iterator it = i.begin();

				while (it != i.end()) {
					LPCHARACTER pc = *it++;
					if (pc)
						if (pc->GetRaceNum() == 30000 && pc->GetPrivShop() == id) {
							pc->UpdateShopItems();
							return;
						}


				}
			}
			TPacketShopUpdateItem packet;
			packet.shop_id = id;
			db_clientdesc->DBPacket(HEADER_GD_SHOP_UPDATE_ITEM, 0, &packet, sizeof(packet));
		}
	}
}

ACMD(do_shop_delete)
{
	if (ch->IsObserverMode() || ch->GetExchange())
		return;
	char arg1[256];
	char arg2[256];
	MYSQL_ROW row;
	DWORD id;
	two_arguments(argument, arg1, sizeof(arg1), arg2, sizeof(arg2));
	if (!*arg1 || !*arg2)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "Usage:");
		ch->ChatPacket(CHAT_TYPE_INFO, "/delete_shop <type> <arg> ");
		ch->ChatPacket(CHAT_TYPE_INFO, "Types:");
		ch->ChatPacket(CHAT_TYPE_INFO, "		shopid - Delete shop using ID");
		ch->ChatPacket(CHAT_TYPE_INFO, "		player - Delete all player shops by player name");
		ch->ChatPacket(CHAT_TYPE_INFO, "Example:");
		ch->ChatPacket(CHAT_TYPE_INFO, "		/delete_shop player [DEV]Logic");
		ch->ChatPacket(CHAT_TYPE_INFO, "		/delete_shop shopid 1");
		return;
	}
	if (!strcmp(arg1, "player"))
	{
		std::unique_ptr<SQLMsg> pkMsg(DBManager::instance().DirectQuery("SELECT id from player_shop WHERE player_id=(select id from player where name='%s')", arg2));
		SQLResult * pRes = pkMsg->Get();
		if (pRes->uiNumRows>0)
		{
			while ((row = mysql_fetch_row(pRes->pSQLResult)) != NULL)
			{
				str_to_number(id, row[0]);
				DeleteShop(id);
			}
		}
	}
	if (!strcmp(arg1, "shopid"))
	{
		std::unique_ptr<SQLMsg> pkMsg(DBManager::instance().DirectQuery("SELECT id from player_shop WHERE id='%s'", arg2));
		SQLResult * pRes = pkMsg->Get();
		if (pRes->uiNumRows>0)
		{
			while ((row = mysql_fetch_row(pRes->pSQLResult)) != NULL)
			{
				str_to_number(id, arg2);
				DeleteShop(id);
			}
		}
	}
}
#else
ACMD(do_close_shop)
{
	if (ch->IsObserverMode())
		return;
	if (ch->GetMyShop())
	{
		ch->CloseMyShop();
		return;
	}
}
#endif

ACMD(do_set_walk_mode)
{
	ch->SetNowWalking(true);
	ch->SetWalking(true);
}

ACMD(do_set_run_mode)
{
	ch->SetNowWalking(false);
	ch->SetWalking(false);
}

ACMD(do_war)
{
	//�� ��� ������ ������
	CGuild * g = ch->GetGuild();

	if (!g)
		return;

	//���������� üũ�ѹ�!
	if (g->UnderAnyWar())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;533]");
		return;
	}

	//�Ķ���͸� �ι�� ������
	char arg1[256], arg2[256];
	DWORD type = GUILD_WAR_TYPE_FIELD; //fixme102 base int modded uint
	two_arguments(argument, arg1, sizeof(arg1), arg2, sizeof(arg2));

	if (!*arg1)
		return;

	if (*arg2)
	{
		str_to_number(type, arg2);

		if (type >= GUILD_WAR_TYPE_MAX_NUM)
			type = GUILD_WAR_TYPE_FIELD;
		
#ifdef __FIX_COMANDA_RAZBOI__
		if (arg2[0]=='-'||strtol(arg2,0,10)<0)
		{
			return;
		}
#endif
	}

	//����� ������ ���̵� ���µ�
	DWORD gm_pid = g->GetMasterPID();

	//���������� üũ(������ ����常�� ����)
	if (gm_pid != ch->GetPlayerID())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("#ERR_CODE: 1116030623 Send this to a GM."));
		return;
	}

	//��� ��带 ������
	CGuild * opp_g = CGuildManager::instance().FindGuildByName(arg1);

	if (!opp_g)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("#ERR_CODE: 1117030623 Send this to a GM."));
		return;
	}

	//�������� ���� üũ
	switch (g->GetGuildWarState(opp_g->GetID()))
	{
		case GUILD_WAR_NONE:
			{
				if (opp_g->UnderAnyWar())
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("#ERR_CODE: 1118030623 Send this to a GM."));
					return;
				}

				int iWarPrice = KOR_aGuildWarInfo[type].iWarPrice;

				if (g->GetGuildMoney() < iWarPrice)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("#ERR_CODE: 1119030623 Send this to a GM."));
					return;
				}

				if (opp_g->GetGuildMoney() < iWarPrice)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("#ERR_CODE: 1120030623 Send this to a GM."));
					return;
				}
			}
			break;

		case GUILD_WAR_SEND_DECLARE:
			{
				ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("�̹�#ERR_CODE: 1121030623 Send this to a GM."));
				return;
			}
			break;

		case GUILD_WAR_RECV_DECLARE:
			{
				if (opp_g->UnderAnyWar())
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("#ERR_CODE: 1122030623 Send this to a GM."));
					g->RequestRefuseWar(opp_g->GetID());
					return;
				}
			}
			break;

		case GUILD_WAR_RESERVE:
			{
				ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("#ERR_CODE: 1123030623 Send this to a GM."));
				return;
			}
			break;

		case GUILD_WAR_END:
			return;

		default:
			ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("#ERR_CODE: 1124030623 Send this to a GM."));
			g->RequestRefuseWar(opp_g->GetID());
			return;
	}

	if (!g->CanStartWar(type))
	{
		// ������� �� �� �ִ� ������ ���������ʴ´�.
		if (g->GetLadderPoint() == 0)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1308]");
			sys_log(0, "GuildWar.StartError.NEED_LADDER_POINT");
		}
		else if (g->GetMemberCount() < GUILD_WAR_MIN_MEMBER_COUNT)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;543;%d]", GUILD_WAR_MIN_MEMBER_COUNT);
			sys_log(0, "GuildWar.StartError.NEED_MINIMUM_MEMBER[%d]", GUILD_WAR_MIN_MEMBER_COUNT);
		}
		else
		{
			sys_log(0, "GuildWar.StartError.UNKNOWN_ERROR");
		}
		return;
	}

	// �ʵ��� üũ�� �ϰ� ������ üũ�� ������ �³��Ҷ� �Ѵ�.
	if (!opp_g->CanStartWar(GUILD_WAR_TYPE_FIELD))
	{
		if (opp_g->GetLadderPoint() == 0)
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;544]");
		else if (opp_g->GetMemberCount() < GUILD_WAR_MIN_MEMBER_COUNT)
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;545]");
		return;
	}

	do
	{
		if (g->GetMasterCharacter() != NULL)
			break;

		CCI *pCCI = P2P_MANAGER::instance().FindByPID(g->GetMasterPID());

		if (pCCI != NULL)
			break;

		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1048]");
		g->RequestRefuseWar(opp_g->GetID());
		return;

	} while (false);

	do
	{
		if (opp_g->GetMasterCharacter() != NULL)
			break;

		CCI *pCCI = P2P_MANAGER::instance().FindByPID(opp_g->GetMasterPID());

		if (pCCI != NULL)
			break;

		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1048]");
		g->RequestRefuseWar(opp_g->GetID());
		return;

	} while (false);

	g->RequestDeclareWar(opp_g->GetID(), type);
}

ACMD(do_nowar)
{
	CGuild* g = ch->GetGuild();
	if (!g)
		return;

	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
		return;

	DWORD gm_pid = g->GetMasterPID();

	if (gm_pid != ch->GetPlayerID())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;534]");
		return;
	}

	CGuild* opp_g = CGuildManager::instance().FindGuildByName(arg1);

	if (!opp_g)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;535]");
		return;
	}

	g->RequestRefuseWar(opp_g->GetID());
}

ACMD(do_detaillog)
{
	ch->DetailLog();
}

ACMD(do_monsterlog)
{
	ch->ToggleMonsterLog();
}

ACMD(do_pkmode)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
		return;

	BYTE mode = 0;
	str_to_number(mode, arg1);

	if (mode == PK_MODE_PROTECT)
		return;

	if (ch->GetLevel() < PK_PROTECT_LEVEL && mode != 0)
		return;

	ch->SetPKMode(mode);
}

ACMD(do_messenger_auth)
{
	if (ch->GetArena())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;403]");
		return;
	}

	char arg1[256], arg2[256];
	two_arguments(argument, arg1, sizeof(arg1), arg2, sizeof(arg2));

	if (!*arg1 || !*arg2)
		return;

	char answer = LOWER(*arg1);
	// @fixme130 AuthToAdd void -> bool
	bool bIsDenied = answer != 'y';
	bool bIsAdded = MessengerManager::instance().AuthToAdd(ch->GetName(), arg2, bIsDenied); // DENY
	if (bIsAdded && bIsDenied)
	{
		LPCHARACTER tch = CHARACTER_MANAGER::instance().FindPC(arg2);

		if (tch)
			tch->ChatPacket(CHAT_TYPE_INFO, "[LS;548;%s]", ch->GetName());
	}

}

ACMD(do_setblockmode)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (*arg1)
	{
		BYTE flag = 0;
		str_to_number(flag, arg1);
		ch->SetBlockMode(flag);
	}
}

ACMD(do_unmount)
{
#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
	if(ch->GetWear(WEAR_COSTUME_MOUNT))
	{
		CMountSystem* mountSystem = ch->GetMountSystem();
		LPITEM mount = ch->GetWear(WEAR_COSTUME_MOUNT);
		DWORD mobVnum = 0;
		
		if (!mountSystem && !mount)
			return;
		
#ifdef __CHANGELOOK_SYSTEM__	
		if(mount->GetTransmutation())
		{
			const TItemTable* itemTable = ITEM_MANAGER::instance().GetTable(mount->GetTransmutation());
			
			if (itemTable)
				mobVnum = itemTable->alValues[1];
			else
				mobVnum = mount->GetValue(1);
		}
		else
			mobVnum = mount->GetValue(1);
#else
		if(mount->GetValue(1) != 0)
			mobVnum = mount->GetValue(1);
#endif

		if (ch->GetMountVnum())
		{
			if(mountSystem->CountSummoned() == 0)
			{
				mountSystem->Unmount(mobVnum);
			}
		}
		return;
	}
#endif

	if (true == ch->UnEquipSpecialRideUniqueItem())
	{
		ch->RemoveAffect(AFFECT_MOUNT);
		ch->RemoveAffect(AFFECT_MOUNT_BONUS);

		if (ch->IsHorseRiding())
		{
			ch->StopRiding();
		}
	}
	else
	{
		ch->ChatPacket( CHAT_TYPE_INFO, LC_TEXT("[#Unk]You cannot dismount because your inventory is full."));
	}

}

ACMD(do_observer_exit)
{
	if (ch->IsObserverMode())
	{
		if (ch->GetWarMap())
			ch->SetWarMap(NULL);

		if (ch->GetArena() != NULL || ch->GetArenaObserverMode() == true)
		{
			ch->SetArenaObserverMode(false);

			if (ch->GetArena() != NULL)
				ch->GetArena()->RemoveObserver(ch->GetPlayerID());

			ch->SetArena(NULL);
			ch->WarpSet(ARENA_RETURN_POINT_X(ch->GetEmpire()), ARENA_RETURN_POINT_Y(ch->GetEmpire()));
		}
		else
		{
			ch->ExitToSavedLocation();
		}
		ch->SetObserverMode(false);
	}
}

ACMD(do_view_equip)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));
	
	if (*arg1)
	{
		DWORD vid = 0;
		str_to_number(vid, arg1);
		LPCHARACTER tch = CHARACTER_MANAGER::instance().Find(vid);

		if (!tch)
			return;

		if (!tch->IsPC())
			return;
	 
		if (tch->IsBlockMode(BLOCK_VIEW_EQUIPMENT))
		{
			ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[i] Player %s blocked the equipment view system"), tch->GetName());
			return;
		}
      
		if ((tch->IsGM()) || (!test_server && tch->IsGM()))
		{
			ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[i] Player %s has this function blocked"), tch->GetName());
			return;
		}
		
		tch->SendEquipment(ch);
	}
}

ACMD(do_party_request)
{
	if (ch->GetArena())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;403]");
		return;
	}

	if (ch->GetParty())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;549]");
		return;
	}

	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
		return;

	DWORD vid = 0;
	str_to_number(vid, arg1);
	LPCHARACTER tch = CHARACTER_MANAGER::instance().Find(vid);

	if (tch)
		if (!ch->RequestToParty(tch))
			ch->ChatPacket(CHAT_TYPE_COMMAND, "PartyRequestDenied");
}

ACMD(do_party_request_accept)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
		return;

	DWORD vid = 0;
	str_to_number(vid, arg1);
	LPCHARACTER tch = CHARACTER_MANAGER::instance().Find(vid);

	if (tch)
		ch->AcceptToParty(tch);
}
// cdf26213a150dc3ecb610f18f6b38b46
ACMD(do_party_request_deny)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
		return;

	DWORD vid = 0;
	str_to_number(vid, arg1);
	LPCHARACTER tch = CHARACTER_MANAGER::instance().Find(vid);

	if (tch)
		ch->DenyToParty(tch);
}
// cdf26213a150dc3ecb610f18f6b38b46
ACMD(do_monarch_warpto)
{
	if (!CMonarch::instance().IsMonarch(ch->GetPlayerID(), ch->GetEmpire()))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1007]");
		return;
	}

	//���� ��Ÿ�� �˻�
	if (!ch->IsMCOK(CHARACTER::MI_WARP))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;551;%d]", ch->GetMCLTime(CHARACTER::MI_WARP));
		return;
	}

	//���� �� ��ȯ ���
	const int WarpPrice = 10000;

	//���� ���� �˻�
	if (!CMonarch::instance().IsMoneyOk(WarpPrice, ch->GetEmpire()))
	{
		int NationMoney = CMonarch::instance().GetMoney(ch->GetEmpire());
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;552;%u;%u]", NationMoney, WarpPrice);
		return;
	}

	int x = 0, y = 0;
	char arg1[256];

	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;553]");
		return;
	}

	LPCHARACTER tch = CHARACTER_MANAGER::instance().FindPC(arg1);

	if (!tch)
	{
		CCI * pkCCI = P2P_MANAGER::instance().Find(arg1);

		if (pkCCI)
		{
			if (pkCCI->bEmpire != ch->GetEmpire())
			{
				ch->ChatPacket (CHAT_TYPE_INFO, "[LS;554]");
				return;
			}

			if (pkCCI->bChannel != g_bChannel)
			{
				ch->ChatPacket(CHAT_TYPE_INFO, "[LS;555;%d;%d]", pkCCI->bChannel, g_bChannel);
				return;
			}
			if (!IsMonarchWarpZone(pkCCI->lMapIndex))
			{
				ch->ChatPacket (CHAT_TYPE_INFO, LC_TEXT("[#Unk]Cannot be summoned to that area."));
				return;
			}

			PIXEL_POSITION pos;

			if (!SECTREE_MANAGER::instance().GetCenterPositionOfMap(pkCCI->lMapIndex, pos))
				ch->ChatPacket(CHAT_TYPE_INFO, "Cannot find map (index %d)", pkCCI->lMapIndex);
			else
			{
				//ch->ChatPacket(CHAT_TYPE_INFO, "You warp to (%d, %d)", pos.x, pos.y);
				ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1010;%s]", arg1);
				ch->WarpSet(pos.x, pos.y);

				//���� �� �谨
				CMonarch::instance().SendtoDBDecMoney(WarpPrice, ch->GetEmpire(), ch);

				//��Ÿ�� �ʱ�ȭ
				ch->SetMC(CHARACTER::MI_WARP);
			}
		}
		else if (NULL == CHARACTER_MANAGER::instance().FindPC(arg1))
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "There is no one by that name");
		}

		return;
	}
	else
	{
		if (tch->GetEmpire() != ch->GetEmpire())
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;554]");
			return;
		}
		if (!IsMonarchWarpZone(tch->GetMapIndex()))
		{
			ch->ChatPacket (CHAT_TYPE_INFO, LC_TEXT("[#Unk]Can't go to that area"));
			return;
		}
		x = tch->GetX();
		y = tch->GetY();
	}

	ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1010;%s]", arg1);
	ch->WarpSet(x, y);
	ch->Stop();

	//���� �� �谨
	CMonarch::instance().SendtoDBDecMoney(WarpPrice, ch->GetEmpire(), ch);

	//��Ÿ�� �ʱ�ȭ
	ch->SetMC(CHARACTER::MI_WARP);
}
// cdf26213a150dc3ecb610f18f6b38b46
ACMD(do_monarch_transfer)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1006]");
		return;
	}

	if (!CMonarch::instance().IsMonarch(ch->GetPlayerID(), ch->GetEmpire()))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1007]");
		return;
	}

	//���� ��Ÿ�� �˻�
	if (!ch->IsMCOK(CHARACTER::MI_TRANSFER))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;551;%d]", ch->GetMCLTime(CHARACTER::MI_TRANSFER));
		return;
	}

	//���� ���� ���
	const int WarpPrice = 10000;

	//���� ���� �˻�
	if (!CMonarch::instance().IsMoneyOk(WarpPrice, ch->GetEmpire()))
	{
		int NationMoney = CMonarch::instance().GetMoney(ch->GetEmpire());
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;552;%u;%u]", NationMoney, WarpPrice);
		return;
	}
// cdf26213a150dc3ecb610f18f6b38b46
	LPCHARACTER tch = CHARACTER_MANAGER::instance().FindPC(arg1);
// cdf26213a150dc3ecb610f18f6b38b46
	if (!tch)
	{
		CCI * pkCCI = P2P_MANAGER::instance().Find(arg1);

		if (pkCCI)
		{
			if (pkCCI->bEmpire != ch->GetEmpire())
			{
				ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1008]");
				return;
			}
			if (pkCCI->bChannel != g_bChannel)
			{
				ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1009;%s;%d;%d]", arg1, pkCCI->bChannel, g_bChannel);
				return;
			}
			if (!IsMonarchWarpZone(pkCCI->lMapIndex))
			{
				ch->ChatPacket (CHAT_TYPE_INFO, LC_TEXT("[#Unk]Can't go to that area"));
				return;
			}
			if (!IsMonarchWarpZone(ch->GetMapIndex()))
			{
				ch->ChatPacket (CHAT_TYPE_INFO, LC_TEXT("[#Unk]Cannot be summoned to that area."));
				return;
			}

			TPacketGGTransfer pgg;

			pgg.bHeader = HEADER_GG_TRANSFER;
			strlcpy(pgg.szName, arg1, sizeof(pgg.szName));
			pgg.lX = ch->GetX();
			pgg.lY = ch->GetY();

			P2P_MANAGER::instance().Send(&pgg, sizeof(TPacketGGTransfer));
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1010;%s]", arg1);

			//���� �� �谨
			CMonarch::instance().SendtoDBDecMoney(WarpPrice, ch->GetEmpire(), ch);
			//��Ÿ�� �ʱ�ȭ
			ch->SetMC(CHARACTER::MI_TRANSFER);
		}
		else
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1011]");
		}

		return;
	}


	if (ch == tch)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1012]");
		return;
	}

	if (tch->GetEmpire() != ch->GetEmpire())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1008]");
		return;
	}
	if (!IsMonarchWarpZone(tch->GetMapIndex()))
	{
		ch->ChatPacket (CHAT_TYPE_INFO, LC_TEXT("[#Unk]Can't go to that area"));
		return;
	}
	if (!IsMonarchWarpZone(ch->GetMapIndex()))
	{
		ch->ChatPacket (CHAT_TYPE_INFO, LC_TEXT("[#Unk]Cannot be summoned to that area."));
		return;
	}

	//tch->Show(ch->GetMapIndex(), ch->GetX(), ch->GetY(), ch->GetZ());
	tch->WarpSet(ch->GetX(), ch->GetY(), ch->GetMapIndex());

	//���� �� �谨
	CMonarch::instance().SendtoDBDecMoney(WarpPrice, ch->GetEmpire(), ch);
	//��Ÿ�� �ʱ�ȭ
	ch->SetMC(CHARACTER::MI_TRANSFER);
}

ACMD(do_monarch_info)
{
	if (CMonarch::instance().IsMonarch(ch->GetPlayerID(), ch->GetEmpire()))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;560]");
		TMonarchInfo * p = CMonarch::instance().GetMonarch();
		for (int n = 1; n < 4; ++n)
		{
			if (n == ch->GetEmpire())
				ch->ChatPacket(CHAT_TYPE_INFO, "[LS;561;%s;%s;%lld]", EMPIRE_NAME(n), p->name[n], p->money[n]);
			else
				ch->ChatPacket(CHAT_TYPE_INFO, "[LS;562;%s;%s]", EMPIRE_NAME(n), p->name[n]);

		}
	}
	else
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;563]");
		TMonarchInfo * p = CMonarch::instance().GetMonarch();
		for (int n = 1; n < 4; ++n)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;562;%s;%s]", EMPIRE_NAME(n), p->name[n]);

		}
	}

}

ACMD(do_elect)
{
	db_clientdesc->DBPacketHeader(HEADER_GD_COME_TO_VOTE, ch->GetDesc()->GetHandle(), 0);
}

// LUA_ADD_GOTO_INFO
struct GotoInfo
{
	std::string 	st_name;

	BYTE 	empire;
	int 	mapIndex;
	DWORD 	x, y;

	GotoInfo()
	{
		st_name 	= "";
		empire 		= 0;
		mapIndex 	= 0;

		x = 0;
		y = 0;
	}

	GotoInfo(const GotoInfo& c_src)
	{
		__copy__(c_src);
	}

	void operator = (const GotoInfo& c_src)
	{
		__copy__(c_src);
	}

	void __copy__(const GotoInfo& c_src)
	{
		st_name 	= c_src.st_name;
		empire 		= c_src.empire;
		mapIndex 	= c_src.mapIndex;

		x = c_src.x;
		y = c_src.y;
	}
};

ACMD(do_monarch_tax)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "Usage: monarch_tax <1-50>");
		return;
	}

	// ���� �˻�
	if (!ch->IsMonarch())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;564]");
		return;
	}

	// ���ݼ���
	int tax = 0;
	str_to_number(tax,  arg1);

	if (tax < 1 || tax > 50)
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;565]");

	quest::CQuestManager::instance().SetEventFlag("trade_tax", tax);

	// ���ֿ��� �޼��� �ϳ�
	ch->ChatPacket(CHAT_TYPE_INFO, "[LS;375;%d]");

	// ����
	char szMsg[1024];

	snprintf(szMsg, sizeof(szMsg), "In the name of the monarch, the tax has been changed to %d%%", tax);
	BroadcastNotice(szMsg);

	snprintf(szMsg, sizeof(szMsg), "In the future, %d%% of the transaction amount will go into the national treasury.", tax);
	BroadcastNotice(szMsg);

	// ��Ÿ�� �ʱ�ȭ
	ch->SetMC(CHARACTER::MI_TAX);
}

static const DWORD cs_dwMonarchMobVnums[] =
{
	191, //	��߽�
	192, //	����
	193, //	����
	194, //	ȣ��
	391, //	����
	392, //	����
	393, //	����
	394, //	����
	491, //	��ȯ
	492, //	����
	493, //	����
	494, //	����
	591, //	����ܴ���
	691, //	���� ����
	791, //	�б�����
	1304, // ��������
	1901, // ����ȣ
	2091, // ���հŹ�
	2191, // �Ŵ�縷�ź�
	2206, // ȭ����i
	0,
};

ACMD(do_monarch_mob)
{
	char arg1[256];
	LPCHARACTER	tch;

	one_argument(argument, arg1, sizeof(arg1));

	if (!ch->IsMonarch())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;564]");
		return;
	}

	if (!*arg1)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "Usage: mmob <mob name>");
		return;
	}

#ifdef ENABLE_MONARCH_MOB_CMD_MAP_CHECK // @warme006
	BYTE pcEmpire = ch->GetEmpire();
	BYTE mapEmpire = SECTREE_MANAGER::instance().GetEmpireFromMapIndex(ch->GetMapIndex());
	if (mapEmpire != pcEmpire && mapEmpire != 0)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[#Unk]This feature is only available in your own territory."));
		return;
	}
#endif

	// ���� �� ��ȯ ���
	const int SummonPrice = 5000000;

	// ���� ��Ÿ�� �˻�
	if (!ch->IsMCOK(CHARACTER::MI_SUMMON))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;551;%d]", ch->GetMCLTime(CHARACTER::MI_SUMMON));
		return;
	}

	// ���� ���� �˻�
	if (!CMonarch::instance().IsMoneyOk(SummonPrice, ch->GetEmpire()))
	{
		int NationMoney = CMonarch::instance().GetMoney(ch->GetEmpire());
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;552;%u;%u]", NationMoney, SummonPrice);
		return;
	}

	const CMob * pkMob;
	DWORD vnum = 0;

	if (isdigit(*arg1))
	{
		str_to_number(vnum, arg1);

		if ((pkMob = CMobManager::instance().Get(vnum)) == NULL)
			vnum = 0;
	}
	else
	{
		pkMob = CMobManager::Instance().Get(arg1, true);

		if (pkMob)
			vnum = pkMob->m_table.dwVnum;
	}

	DWORD count;

	// ��ȯ ���� �� �˻�
	for (count = 0; cs_dwMonarchMobVnums[count] != 0; ++count)
		if (cs_dwMonarchMobVnums[count] == vnum)
			break;

	if (0 == cs_dwMonarchMobVnums[count])
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;567]");
		return;
	}

	tch = CHARACTER_MANAGER::instance().SpawnMobRange(vnum,
			ch->GetMapIndex(),
			ch->GetX() - number(200, 750),
			ch->GetY() - number(200, 750),
			ch->GetX() + number(200, 750),
			ch->GetY() + number(200, 750),
			true,
			pkMob->m_table.bType == CHAR_TYPE_STONE,
			true);

	if (tch)
	{
		// ���� �� �谨
		CMonarch::instance().SendtoDBDecMoney(SummonPrice, ch->GetEmpire(), ch);

		// ��Ÿ�� �ʱ�ȭ
		ch->SetMC(CHARACTER::MI_SUMMON);
	}
}

static const char* FN_point_string(int apply_number)
{
	switch (apply_number)
	{
		case POINT_MAX_HP:	return LC_TEXT("POINT_MAX_HP +%d");
		case POINT_MAX_SP:	return LC_TEXT("POINT_MAX_SP +%d");
		case POINT_HT:		return LC_TEXT("POINT_HT +%d");
		case POINT_IQ:		return LC_TEXT("POINT_IQ +%d");
		case POINT_ST:		return LC_TEXT("POINT_ST +%d");
		case POINT_DX:		return LC_TEXT("POINT_DX +%d");
		case POINT_ATT_SPEED:	return LC_TEXT("POINT_ATT_SPEED +%d");
		case POINT_MOV_SPEED:	return LC_TEXT("POINT_MOV_SPEED %d");
		case POINT_CASTING_SPEED:	return LC_TEXT("POINT_CASTING_SPEED -%d");
		case POINT_HP_REGEN:	return LC_TEXT("POINT_HP_REGEN +%d");
		case POINT_SP_REGEN:	return LC_TEXT("POINT_SP_REGEN +%d");
		case POINT_POISON_PCT:	return LC_TEXT("POINT_POISON_PCT %d");
		case POINT_STUN_PCT:	return LC_TEXT("POINT_STUN_PCT +%d");
		case POINT_SLOW_PCT:	return LC_TEXT("POINT_SLOW_PCT +%d");
		case POINT_CRITICAL_PCT:	return LC_TEXT("POINT_CRITICAL_PCT %d%%");
		case POINT_RESIST_CRITICAL:	return LC_TEXT("POINT_RESIST_CRITICAL %d%%");
		case POINT_PENETRATE_PCT:	return LC_TEXT("POINT_PENETRATE_PCT %d%%");
		case POINT_RESIST_PENETRATE: return LC_TEXT("POINT_RESIST_PENETRATE %d%%");
		case POINT_ATTBONUS_HUMAN:	return LC_TEXT("POINT_ATTBONUS_HUMAN +%d%%");
		case POINT_ATTBONUS_ANIMAL:	return LC_TEXT("POINT_ATTBONUS_ANIMAL +%d%%");
		case POINT_ATTBONUS_ORC:	return LC_TEXT("POINT_ATTBONUS_ORC +%d%%");
		case POINT_ATTBONUS_MILGYO:	return LC_TEXT("POINT_ATTBONUS_MILGYO +%d%%");
		case POINT_ATTBONUS_UNDEAD:	return LC_TEXT("POINT_ATTBONUS_UNDEAD +%d%%");
		case POINT_ATTBONUS_DEVIL:	return LC_TEXT("POINT_ATTBONUS_DEVIL +%d%%");
		case POINT_STEAL_HP:		return LC_TEXT("POINT_STEAL_HP %d%%");
		case POINT_STEAL_SP:		return LC_TEXT("POINT_STEAL_SP %d%%");
		case POINT_MANA_BURN_PCT:	return LC_TEXT("POINT_MANA_BURN_PCT %d%%");
		case POINT_DAMAGE_SP_RECOVER:	return LC_TEXT("POINT_DAMAGE_SP_RECOVER %d%%");
		case POINT_BLOCK:			return LC_TEXT("POINT_BLOCK %d%%");
		case POINT_DODGE:			return LC_TEXT("POINT_DODGE %d%%");
		case POINT_RESIST_SWORD:	return LC_TEXT("POINT_RESIST_SWORD %d%%");
		case POINT_RESIST_TWOHAND:	return LC_TEXT("POINT_RESIST_TWOHAND %d%%");
		case POINT_RESIST_DAGGER:	return LC_TEXT("POINT_RESIST_DAGGER %d%%");
		case POINT_RESIST_BELL:		return LC_TEXT("POINT_RESIST_BELL %d%%");
		case POINT_RESIST_FAN:		return LC_TEXT("POINT_RESIST_FAN %d%%");
		case POINT_RESIST_BOW:		return LC_TEXT("POINT_RESIST_BOW %d%%");

		case POINT_RESIST_FIRE:		return LC_TEXT("POINT_RESIST_FIRE %d%%");
		case POINT_RESIST_ELEC:		return LC_TEXT("POINT_RESIST_ELEC %d%%");
		case POINT_RESIST_MAGIC:	return LC_TEXT("POINT_RESIST_MAGIC %d%%");
#ifdef ENABLE_MAGIC_REDUCTION_SYSTEM
		case POINT_RESIST_MAGIC_REDUCTION:	return LC_TEXT("POINT_RESIST_MAGIC_REDUCTION %d%%");
#endif
		case POINT_RESIST_WIND:		return LC_TEXT("POINT_RESIST_WIND %d%%");
		case POINT_RESIST_ICE:		return LC_TEXT("POINT_RESIST_ICE %d%%");
		case POINT_RESIST_EARTH:	return LC_TEXT("POINT_RESIST_EARTH %d%%");
		case POINT_RESIST_DARK:		return LC_TEXT("POINT_RESIST_DARK %d%%");
		case POINT_REFLECT_MELEE:	return LC_TEXT("POINT_REFLECT_MELEE %d%%");
		case POINT_REFLECT_CURSE:	return LC_TEXT("POINT_REFLECT_CURSE %d%%");
		case POINT_POISON_REDUCE:	return LC_TEXT("POINT_POISON_REDUCE %d%%");

		case POINT_KILL_SP_RECOVER:	return LC_TEXT("POINT_KILL_SP_RECOVER %d%%");
		case POINT_EXP_DOUBLE_BONUS:	return LC_TEXT("POINT_EXP_DOUBLE_BONUS %d%%");
		case POINT_GOLD_DOUBLE_BONUS:	return LC_TEXT("POINT_GOLD_DOUBLE_BONUS %d%%");
		case POINT_ITEM_DROP_BONUS:	return LC_TEXT("POINT_ITEM_DROP_BONUS %d%%");
		case POINT_POTION_BONUS:	return LC_TEXT("POINT_POTION_BONUS %d%%");
		case POINT_KILL_HP_RECOVERY:	return LC_TEXT("POINT_KILL_HP_RECOVERY %d%%");
		case POINT_ATT_GRADE_BONUS:	return LC_TEXT("POINT_ATT_GRADE_BONUS +%d");
		case POINT_DEF_GRADE_BONUS:	return LC_TEXT("POINT_DEF_GRADE_BONUS +%d");
		case POINT_MAGIC_ATT_GRADE:	return LC_TEXT("POINT_MAGIC_ATT_GRADE +%d");
		case POINT_MAGIC_DEF_GRADE:	return LC_TEXT("POINT_MAGIC_DEF_GRADE +%d");
		case POINT_MAX_STAMINA:	return LC_TEXT("POINT_MAX_STAMINA +%d");
		case POINT_ATTBONUS_WARRIOR:	return LC_TEXT("POINT_ATTBONUS_WARRIOR +%d%%");
		case POINT_ATTBONUS_ASSASSIN:	return LC_TEXT("POINT_ATTBONUS_ASSASSIN +%d%%");
		case POINT_ATTBONUS_SURA:		return LC_TEXT("POINT_ATTBONUS_SURA +%d%%");
		case POINT_ATTBONUS_SHAMAN:		return LC_TEXT("POINT_ATTBONUS_SHAMAN +%d%%");
		case POINT_ATTBONUS_MONSTER:	return LC_TEXT("POINT_ATTBONUS_MONSTER +%d%%");
		case POINT_MALL_ATTBONUS:		return LC_TEXT("POINT_MALL_ATTBONUS +%d%%");
		case POINT_MALL_DEFBONUS:		return LC_TEXT("���� +%d%%");
		case POINT_MALL_EXPBONUS:		return LC_TEXT("POINT_MALL_DEFBONUS %d%%");
		case POINT_MALL_ITEMBONUS:		return LC_TEXT("POINT_MALL_ITEMBONUS %.1f");
		case POINT_MALL_GOLDBONUS:		return LC_TEXT("POINT_MALL_GOLDBONUS %.1f");
		case POINT_MAX_HP_PCT:			return LC_TEXT("POINT_MAX_HP_PCT +%d%%");
		case POINT_MAX_SP_PCT:			return LC_TEXT("POINT_MAX_SP_PCT +%d%%");
		case POINT_SKILL_DAMAGE_BONUS:	return LC_TEXT("POINT_SKILL_DAMAGE_BONUS %d%%");
		case POINT_NORMAL_HIT_DAMAGE_BONUS:	return LC_TEXT("POINT_NORMAL_HIT_DAMAGE_BONUS %d%%");
		case POINT_SKILL_DEFEND_BONUS:		return LC_TEXT("POINT_SKILL_DEFEND_BONUS %d%%");
		case POINT_NORMAL_HIT_DEFEND_BONUS:	return LC_TEXT("POINT_NORMAL_HIT_DEFEND_BONUS %d%%");
		case POINT_RESIST_WARRIOR:	return LC_TEXT("POINT_RESIST_WARRIOR %d%%");
		case POINT_RESIST_ASSASSIN:	return LC_TEXT("POINT_RESIST_ASSASSIN %d%%");
		case POINT_RESIST_SURA:		return LC_TEXT("POINT_RESIST_SURA %d%%");
		case POINT_RESIST_SHAMAN:	return LC_TEXT("POINT_RESIST_SHAMAN %d%%");
		case POINT_ATTBONUS_METIN:	return LC_TEXT("POINT_ATTBONUS_METIN +%d%%");
		case POINT_ATTBONUS_BOSS:	return LC_TEXT("POINT_ATTBONUS_BOSS +%d%%");
		default:					return NULL;
	}
}

static bool FN_hair_affect_string(LPCHARACTER ch, char *buf, size_t bufsiz)
{
	if (NULL == ch || NULL == buf)
		return false;

	CAffect* aff = NULL;
	time_t expire = 0;
	struct tm ltm;
	int	year, mon, day;
	int	offset = 0;

	aff = ch->FindAffect(AFFECT_HAIR);

	if (NULL == aff)
		return false;

	expire = ch->GetQuestFlag("hair.limit_time");

	if (expire < get_global_time())
		return false;
	offset = snprintf(buf, bufsiz, FN_point_string(aff->bApplyOn), aff->lApplyValue);

	if (offset < 0 || offset >= (int) bufsiz)
		offset = bufsiz - 1;

	localtime_r(&expire, &ltm);

	year	= ltm.tm_year + 1900;
	mon		= ltm.tm_mon + 1;
	day		= ltm.tm_mday;

	snprintf(buf + offset, bufsiz - offset, "[LS;651;%d;%d;%d]", year, mon, day);

	return true;
}

ACMD(do_costume)
{
#ifdef __SASH_SYSTEM__
	char buf[768];
#else
	char buf[512];
#endif
	const size_t bufferSize = sizeof(buf);

	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	CItem* pBody = ch->GetWear(WEAR_COSTUME_BODY);
	CItem* pHair = ch->GetWear(WEAR_COSTUME_HAIR);
#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
	CItem* pMount = ch->GetWear(WEAR_COSTUME_MOUNT);
#endif
#ifdef __SASH_SYSTEM__
	CItem * pSash = ch->GetWear(WEAR_COSTUME_SASH);
#endif
#ifdef ENABLE_WEAPON_COSTUME_SYSTEM
	CItem* pWeapon = ch->GetWear(WEAR_COSTUME_WEAPON);
#endif

	ch->ChatPacket(CHAT_TYPE_INFO, "COSTUME status:");

	if (pHair)
	{
		const char* itemName = pHair->GetName();
		ch->ChatPacket(CHAT_TYPE_INFO, "  HAIR : %s", itemName);

		for (int i = 0; i < pHair->GetAttributeCount(); ++i)
		{
			const TPlayerItemAttribute& attr = pHair->GetAttribute(i);
			if (0 < attr.bType)
			{
				snprintf(buf, bufferSize, FN_point_string(attr.bType), attr.sValue);
				ch->ChatPacket(CHAT_TYPE_INFO, "     %s", buf);
			}
		}

		if (pHair->IsEquipped() && arg1[0] == 'h')
			ch->UnequipItem(pHair);
	}

	if (pBody)
	{
		const char* itemName = pBody->GetName();
		ch->ChatPacket(CHAT_TYPE_INFO, "  BODY : %s", itemName);

		if (pBody->IsEquipped() && arg1[0] == 'b')
			ch->UnequipItem(pBody);
	}

#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
	if (pMount)
	{
		const char* itemName = pMount->GetName();
		ch->ChatPacket(CHAT_TYPE_INFO, "  MOUNT : %s", itemName);

		if (pMount->IsEquipped() && arg1[0] == 'm')
			ch->UnequipItem(pMount);
	}
#endif

	
#ifdef __SASH_SYSTEM__
	if (pSash)
	{
		const char * itemName = pSash->GetName();
		ch->ChatPacket(CHAT_TYPE_INFO, "  SASH: %s", itemName);
		for (int i = 0; i < pSash->GetAttributeCount(); ++i)
		{
			const TPlayerItemAttribute& attr = pSash->GetAttribute(i);
			if (attr.bType > 0)
			{
				const char * pAttrName = FN_point_string(attr.bType);
				if (pAttrName == NULL)
					continue;
				
				snprintf(buf, sizeof(buf), FN_point_string(attr.bType), attr.sValue);
				ch->ChatPacket(CHAT_TYPE_INFO, "     %s", buf);
			}
		}

		if (pSash->IsEquipped() && arg1[0] == 's')
			ch->UnequipItem(pSash);
	}
#endif

#ifdef ENABLE_WEAPON_COSTUME_SYSTEM
	if (pWeapon)
	{
		const char* itemName = pWeapon->GetName();
		ch->ChatPacket(CHAT_TYPE_INFO, "  WEAPON : %s", itemName);

		if (pWeapon->IsEquipped() && arg1[0] == 'w')
			ch->UnequipItem(pWeapon);
	}
#endif
#ifdef __AURA_SYSTEM__
	CItem* pAura = ch->GetWear(WEAR_COSTUME_AURA);
	if (pAura)
	{
		const char* itemName = pAura->GetName();
		ch->ChatPacket(CHAT_TYPE_INFO, "  AURA : %s", itemName);
		if (pAura->IsEquipped() && arg1[0] == 'a')
			ch->UnequipItem(pAura);
	}
#endif
}

ACMD(do_hair)
{
	char buf[256];

	if (false == FN_hair_affect_string(ch, buf, sizeof(buf)))
		return;

	ch->ChatPacket(CHAT_TYPE_INFO, buf);
}

ACMD(do_inventory)
{
	int	index = 0;
	int	count		= 1;

	char arg1[256];
	char arg2[256];

	LPITEM	item;

	two_arguments(argument, arg1, sizeof(arg1), arg2, sizeof(arg2));

	if (!*arg1)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "Usage: inventory <start_index> <count>");
		return;
	}

	if (!*arg2)
	{
		index = 0;
		str_to_number(count, arg1);
	}
	else
	{
		str_to_number(index, arg1); index = MIN(index, INVENTORY_MAX_NUM);
		str_to_number(count, arg2); count = MIN(count, INVENTORY_MAX_NUM);
	}

	for (int i = 0; i < count; ++i)
	{
		if (index >= INVENTORY_MAX_NUM)
			break;

		item = ch->GetInventoryItem(index);

		ch->ChatPacket(CHAT_TYPE_INFO, "inventory [%d] = %s",
						index, item ? item->GetName() : "<NONE>");
		++index;
	}
}
ACMD(do_gift)
{
	ch->ChatPacket(CHAT_TYPE_COMMAND, "gift");
}

#ifdef ENABLE_CUBE_RENEWAL_WORLDARD
ACMD(do_cube)
{

	const char *line;
	char arg1[256], arg2[256], arg3[256];
	line = two_arguments(argument, arg1, sizeof(arg1), arg2, sizeof(arg2));
	one_argument(line, arg3, sizeof(arg3));

	if (0 == arg1[0])
	{
		return;
	}

	switch (LOWER(arg1[0]))
	{
		case 'o':	// open
			Cube_open(ch);
			break;

		default:
			return;
	}
}
#else
ACMD(do_cube)
{
	if (!ch->CanDoCube())
		return;

	dev_log(LOG_DEB0, "CUBE COMMAND <%s>: %s", ch->GetName(), argument);
	int cube_index = 0, inven_index = 0;
	const char *line;

	int inven_type = 0;
	
	char arg1[256], arg2[256], arg3[256];
	line = two_arguments(argument, arg1, sizeof(arg1), arg2, sizeof(arg2));
	one_argument(line, arg3, sizeof(arg3));

	if (0 == arg1[0])
	{
		// print usage
		ch->ChatPacket(CHAT_TYPE_INFO, "Usage: cube open");
		ch->ChatPacket(CHAT_TYPE_INFO, "       cube close");
		ch->ChatPacket(CHAT_TYPE_INFO, "       cube add <inveltory_index>");
		ch->ChatPacket(CHAT_TYPE_INFO, "       cube delete <cube_index>");
		ch->ChatPacket(CHAT_TYPE_INFO, "       cube list");
		ch->ChatPacket(CHAT_TYPE_INFO, "       cube cancel");
		ch->ChatPacket(CHAT_TYPE_INFO, "       cube make [all]");
		return;
	}

	const std::string& strArg1 = std::string(arg1);
	if (strArg1 == "r_info")
	{
		if (0 == arg2[0])
			Cube_request_result_list(ch);
		else
		{
			if (isdigit(*arg2))
			{
				int listIndex = 0, requestCount = 1;
				str_to_number(listIndex, arg2);

				if (0 != arg3[0] && isdigit(*arg3))
					str_to_number(requestCount, arg3);

				Cube_request_material_info(ch, listIndex, requestCount);
			}
		}

		return;
	}

	switch (LOWER(arg1[0]))
	{
		case 'o':	// open
			Cube_open(ch);
			break;

		case 'c':	// close
			Cube_close(ch);
			break;

		case 'l':	// list
			Cube_show_list(ch);
			break;

		case 'a':	// add cue_index inven_index
			{
				if (0 == arg2[0] || !isdigit(*arg2) || 0 == arg3[0] || !isdigit(*arg3))
					return;

				str_to_number(cube_index, arg2);
				str_to_number(inven_index, arg3);
				Cube_add_item (ch, cube_index, inven_index);
			}
			break;

		case 'd':	// delete
			{
				if (0 == arg2[0] || !isdigit(*arg2))
					return;

				str_to_number(cube_index, arg2);
				Cube_delete_item (ch, cube_index);
			}
			break;

		case 'm':	// make
			if (0 != arg2[0])
			{
				while (true == Cube_make(ch))
					dev_log (LOG_DEB0, "cube make success");
			}
			else
				Cube_make(ch);
			break;

		default:
			return;
	}
}
#endif

ACMD(do_in_game_mall)
{
	if (LC_IsEurope() == true)
	{
		char country_code[3];

		switch (LC_GetLocalType())
		{
			case LC_GERMANY:	country_code[0] = 'd'; country_code[1] = 'e'; country_code[2] = '\0'; break;
			case LC_FRANCE:		country_code[0] = 'f'; country_code[1] = 'r'; country_code[2] = '\0'; break;
			case LC_ITALY:		country_code[0] = 'i'; country_code[1] = 't'; country_code[2] = '\0'; break;
			case LC_SPAIN:		country_code[0] = 'e'; country_code[1] = 's'; country_code[2] = '\0'; break;
			case LC_UK:			country_code[0] = 'e'; country_code[1] = 'n'; country_code[2] = '\0'; break;
			case LC_TURKEY:		country_code[0] = 't'; country_code[1] = 'r'; country_code[2] = '\0'; break;
			case LC_POLAND:		country_code[0] = 'p'; country_code[1] = 'l'; country_code[2] = '\0'; break;
			case LC_PORTUGAL:	country_code[0] = 'p'; country_code[1] = 't'; country_code[2] = '\0'; break;
			case LC_GREEK:		country_code[0] = 'g'; country_code[1] = 'r'; country_code[2] = '\0'; break;
			case LC_RUSSIA:		country_code[0] = 'r'; country_code[1] = 'u'; country_code[2] = '\0'; break;
			case LC_DENMARK:	country_code[0] = 'd'; country_code[1] = 'k'; country_code[2] = '\0'; break;
			case LC_BULGARIA:	country_code[0] = 'b'; country_code[1] = 'g'; country_code[2] = '\0'; break;
			case LC_CROATIA:	country_code[0] = 'h'; country_code[1] = 'r'; country_code[2] = '\0'; break;
			case LC_MEXICO:		country_code[0] = 'm'; country_code[1] = 'x'; country_code[2] = '\0'; break;
			case LC_ARABIA:		country_code[0] = 'a'; country_code[1] = 'e'; country_code[2] = '\0'; break;
			case LC_CZECH:		country_code[0] = 'c'; country_code[1] = 'z'; country_code[2] = '\0'; break;
			case LC_ROMANIA:	country_code[0] = 'r'; country_code[1] = 'o'; country_code[2] = '\0'; break;
			case LC_HUNGARY:	country_code[0] = 'h'; country_code[1] = 'u'; country_code[2] = '\0'; break;
			case LC_NETHERLANDS: country_code[0] = 'n'; country_code[1] = 'l'; country_code[2] = '\0'; break;
			case LC_USA:		country_code[0] = 'u'; country_code[1] = 's'; country_code[2] = '\0'; break;
			case LC_CANADA:	country_code[0] = 'c'; country_code[1] = 'a'; country_code[2] = '\0'; break;
#ifdef __MULTI_LANGUAGE_SYSTEM__
			case LC_EUROPE: country_code[0] = 'e'; country_code[1] = 'u'; country_code[2] = '\0'; break;
#endif
			default:
				if (test_server == true)
				{
#ifdef __MULTI_LANGUAGE_SYSTEM__
				country_code[0] = 'p'; country_code[1] = 't'; country_code[2] = '\0';
#else
				country_code[0] = 'd'; country_code[1] = 'e'; country_code[2] = '\0';
#endif
				}
				break;
		}

		char buf[512+1];
		char sas[33];
		MD5_CTX ctx;
		const char sas_key[] = "GF9001";

		snprintf(buf, sizeof(buf), "%u%u%s", ch->GetPlayerID(), ch->GetAID(), sas_key);

		MD5Init(&ctx);
		MD5Update(&ctx, (const unsigned char *) buf, strlen(buf));
#ifdef __FreeBSD__
		MD5End(&ctx, sas);
#else
		static const char hex[] = "0123456789abcdef";
		unsigned char digest[16];
		MD5Final(digest, &ctx);
		int i;
		for (i = 0; i < 16; ++i) {
			sas[i+i] = hex[digest[i] >> 4];
			sas[i+i+1] = hex[digest[i] & 0x0f];
		}
		sas[i+i] = '\0';
#endif

		snprintf(buf, sizeof(buf), "mall https://%s/ishop?pid=%u&c=%s&sid=%d&sas=%s",
				g_strWebMallURL.c_str(), ch->GetPlayerID(), country_code, g_server_id, sas);

		ch->ChatPacket(CHAT_TYPE_COMMAND, buf);
	}
}
ACMD(do_dice)
{
	char arg1[256], arg2[256];
	int start = 1, end = 100;

	two_arguments(argument, arg1, sizeof(arg1), arg2, sizeof(arg2));

	if (*arg1 && *arg2)
	{
		start = atoi(arg1);
		end = atoi(arg2);
	}
	else if (*arg1 && !*arg2)
	{
		start = 1;
		end = atoi(arg1);
	}

	end = MAX(start, end);
	start = MIN(start, end);

	int n = number(start, end);

#ifdef ENABLE_DICE_SYSTEM
	if (ch->GetParty())
		ch->GetParty()->ChatPacketToAllMember(CHAT_TYPE_DICE_INFO, "[LS;1046;%s;%d;%d;%d]", ch->GetName(), n, start, end);
	else
		ch->ChatPacket(CHAT_TYPE_DICE_INFO, "[LS;1047;%d;%d;%d]", n, start, end);
#else
	if (ch->GetParty())
		ch->GetParty()->ChatPacketToAllMember(CHAT_TYPE_INFO, "[LS;1046;%s;%d;%d;%d]", ch->GetName(), n, start, end);
	else
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1047;%d;%d;%d]", n, start, end);
#endif
}

#ifdef ENABLE_NEWSTUFF
ACMD(do_click_safebox)
{
	if ((ch->GetGMLevel() <= GM_PLAYER) && (ch->GetDungeon() || ch->GetWarMap()))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot open the safebox in dungeon or at war"));
		return;
	}

	ch->SetSafeboxOpenPosition();
	ch->ChatPacket(CHAT_TYPE_COMMAND, "ShowMeSafeboxPassword");
}
ACMD(do_force_logout)
{
	LPDESC pDesc=DESC_MANAGER::instance().FindByCharacterName(ch->GetName());
	if (!pDesc)
		return;
	pDesc->DelayedDisconnect(0);
}
#endif

ACMD(do_click_mall)
{
	ch->ChatPacket(CHAT_TYPE_COMMAND, "ShowMeMallPassword");
}
ACMD(do_ride)
{
    dev_log(LOG_DEB0, "[DO_RIDE] start");
    if (ch->IsDead() || ch->IsStun())
	return;

	if (ch->GetMapIndex() == 113)
		return;
#ifdef ENABLE_MOUNT_COSTUME_SYSTEM

	if (ch->IsPolymorphed() == true){
		ch->ChatPacket(CHAT_TYPE_INFO, "Nu poti folosi un mount atat timp cat esti transformat.");
		return;
	}
	if(ch->GetWear(WEAR_COSTUME_MOUNT))
	{
		CMountSystem* mountSystem = ch->GetMountSystem();
		LPITEM mount = ch->GetWear(WEAR_COSTUME_MOUNT);
		DWORD mobVnum = 0;
		
		if (!mountSystem && !mount)
			return;
		
#ifdef __CHANGELOOK_SYSTEM__	
		if(mount->GetTransmutation())
		{
			const TItemTable* itemTable = ITEM_MANAGER::instance().GetTable(mount->GetTransmutation());
			
			if (itemTable)
				mobVnum = itemTable->alValues[1];
			else
				mobVnum = mount->GetValue(1);
		}
		else
			mobVnum = mount->GetValue(1);
#else
		if(mount->GetValue(1) != 0)
			mobVnum = mount->GetValue(1);
#endif

		if (ch->GetMountVnum())
		{
			if(mountSystem->CountSummoned() == 0)
			{
				mountSystem->Unmount(mobVnum);
			}
		}
		else
		{
			if(mountSystem->CountSummoned() == 1)
			{
				mountSystem->Mount(mobVnum, mount);
			}
		}
		
		return;
	}
#endif

    // ������
    {
	if (ch->IsHorseRiding())
	{
	    dev_log(LOG_DEB0, "[DO_RIDE] stop riding");
	    ch->StopRiding();
	    return;
	}

	if (ch->GetMountVnum())
	{
	    dev_log(LOG_DEB0, "[DO_RIDE] unmount");
	    do_unmount(ch, NULL, 0, 0);
	    return;
	}
    }

    // Ÿ��
    {
	if (ch->GetHorse() != NULL)
	{
	    dev_log(LOG_DEB0, "[DO_RIDE] start riding");
	    ch->StartRiding();
	    return;
	}

	for (BYTE i=0; i<INVENTORY_MAX_NUM; ++i)
	{
	    LPITEM item = ch->GetInventoryItem(i);
	    if (NULL == item)
		continue;

	    // ����ũ Ż�� ������
		if (item->IsRideItem())
		{
			if (
				NULL==ch->GetWear(WEAR_UNIQUE1)
				|| NULL==ch->GetWear(WEAR_UNIQUE2)
#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
				|| NULL==ch->GetWear(WEAR_COSTUME_MOUNT)
#endif
			)
			{
				dev_log(LOG_DEB0, "[DO_RIDE] USE UNIQUE ITEM");
				//ch->EquipItem(item);
				ch->UseItem(TItemPos (INVENTORY, i));
				return;
			}
		}
	    switch (item->GetVnum())
	    {
		case 71114:
		case 71116:
		case 71118:
		case 71120:
		    dev_log(LOG_DEB0, "[DO_RIDE] USE QUEST ITEM");
		    ch->UseItem(TItemPos (INVENTORY, i));
		    return;
	    }
		if( (item->GetVnum() > 52000) && (item->GetVnum() < 52091) )	{
			dev_log(LOG_DEB0, "[DO_RIDE] USE QUEST ITEM");
			ch->UseItem(TItemPos (INVENTORY, i));
		    return;
		}
	}
    }
    ch->ChatPacket(CHAT_TYPE_INFO, "[LS;501]");
}
#ifdef ENABLE_MOVE_CHANNEL
ACMD(do_move_channel)
{
	if (!ch)
		return;

	if (ch->m_pkTimedEvent)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;511]");
		event_cancel(&ch->m_pkTimedEvent);
		return;
	}

	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Using: channel <channel number>"));
		return;
	}

	short channel;
	str_to_number(channel, arg1);

	if (channel < 1 || channel > 4)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Please enter a valid number"));
		return;
	}

	if (channel == g_bChannel)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You are already on channel %d"), g_bChannel);
		return;
	}

	if (g_bChannel == 99)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You can't change channel in a global map"));
		return;
	}

	if (ch->GetDungeon() || ch->GetMapIndex() >= 10000)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You can't change channel in a dungeon"));
		return;
	}

	TPacketChangeChannel p;
	p.iChannel = channel;
	p.lMapIndex = ch->GetMapIndex();

	db_clientdesc->DBPacket(HEADER_GD_FIND_CHANNEL, ch->GetDesc()->GetHandle(), &p, sizeof(p));
}
#endif
ACMD(do_stat_val)
{
	char	arg1[256], arg2[256];
	two_arguments(argument, arg1, sizeof(arg1), arg2, sizeof(arg2));
	int val = 0;
	str_to_number(val, arg2);
	
	if (!*arg1 || val <= 0)
		return;

	if (ch->IsPolymorphed())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(cmd_general)tradus:You cannot change your state as long as you are transformed"));
		return;
	}

	if (ch->GetPoint(POINT_STAT) <= 0)
		return;

	BYTE idx = 0;
	
	if (!strcmp(arg1, "st"))
		idx = POINT_ST;
	else if (!strcmp(arg1, "dx"))
		idx = POINT_DX;
	else if (!strcmp(arg1, "ht"))
		idx = POINT_HT;
	else if (!strcmp(arg1, "iq"))
		idx = POINT_IQ;
	else
		return;

	if (ch->GetRealPoint(idx) >= MAX_STAT)
		return;
	
	if (val > ch->GetPoint(POINT_STAT))
		val = ch->GetPoint(POINT_STAT);
	
	if (ch->GetRealPoint(idx) + val > MAX_STAT)
		val = MAX_STAT - ch->GetRealPoint(idx);

	ch->SetRealPoint(idx, ch->GetRealPoint(idx) + val);
	ch->SetPoint(idx, ch->GetPoint(idx) + val);
	ch->ComputePoints();
	ch->PointChange(idx, 0);

	if (idx == POINT_IQ)
		ch->PointChange(POINT_MAX_HP, 0);
	else if (idx == POINT_HT)
		ch->PointChange(POINT_MAX_SP, 0);

	ch->PointChange(POINT_STAT, -val);
	ch->ComputePoints();
}

ACMD(letters_open)
{
	ch->SendDropLettersItem();
	ch->ChatPacket(CHAT_TYPE_COMMAND, "OpenLetterWindow");
}


ACMD(give_reward)
{
	if (ch->IsOpenSafebox() || ch->GetShop() || ch->IsCubeOpen() || ch->IsDead() || ch->GetExchange() || ch->GetMyShop())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "@@(cmd_general)tradus:You cannot do that");
		return;
	}

	if (0 == quest::CQuestManager::instance().GetEventFlag("letters_event"))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "@@(cmd_general)tradus:The event is not available");
		return;
	}

	if (ch->CountSpecifyItem(30216) <= 0 ||
	ch->CountSpecifyItem(30213) <= 0 ||
	ch->CountSpecifyItem(30219) <= 0 ||
	ch->CountSpecifyItem(30214) <= 0 ||
	ch->CountSpecifyItem(30217) <= 0 ||
	ch->CountSpecifyItem(30210) <= 0

	)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "@@(cmd_general)tradus:You do not have all the necessary letters");
	}
	
	DWORD dwBoxVnum = BOX_VNUM_L;
	std::vector <DWORD> dwVnums;
	std::vector <DWORD> dwCounts;
	std::vector <LPITEM> item_gets;
	int count = 0;
	
	if (ch->CountSpecifyItem(30216) >= 1 &&
	ch->CountSpecifyItem(30213) >= 1 &&
	ch->CountSpecifyItem(30219) >= 1 &&
	ch->CountSpecifyItem(30214) >= 1 &&
	ch->CountSpecifyItem(30217) >= 1 &&
	ch->CountSpecifyItem(30210) >= 1

	)
	{
		ch->RemoveSpecifyItem(30216, 1);
		ch->RemoveSpecifyItem(30213, 1);
		ch->RemoveSpecifyItem(30219, 1);
		ch->RemoveSpecifyItem(30214, 1);
		ch->RemoveSpecifyItem(30217, 1);
		ch->RemoveSpecifyItem(30210, 1);
		ch->GiveItemFromSpecialItemGroup(dwBoxVnum, dwVnums, dwCounts, item_gets, count);
	}
}

ACMD(do_cards)
{
	const char *line;

	char arg1[256], arg2[256];

	line = two_arguments(argument, arg1, sizeof(arg1), arg2, sizeof(arg2));
	switch (LOWER(arg1[0]))
	{
		case 'o':	// open
			if (isdigit(*arg2))
			{
				DWORD safemode;
				str_to_number(safemode, arg2);
				ch->Cards_open(safemode);
			}
			break;
		case 'p':	// open
			ch->Cards_pullout();
			break;
		case 'e':	// open
			ch->CardsEnd();
			break;
		case 'd':	// open
			if (isdigit(*arg2))
			{
				DWORD destroy_index;
				str_to_number(destroy_index, arg2);
				ch->CardsDestroy(destroy_index);
			}
			break;
		case 'a':	// open
			if (isdigit(*arg2))
			{
				DWORD accpet_index;
				str_to_number(accpet_index, arg2);
				ch->CardsAccept(accpet_index);
			}
			break;
		case 'r':	// open
			if (isdigit(*arg2))
			{
				DWORD restore_index;
				str_to_number(restore_index, arg2);
				ch->CardsRestore(restore_index);
			}
			break;
		default:
			return;
	}
}
#ifdef ENABLE_DRAGONSOUL_ALCHEMY_PLUS
ACMD(do_ds_change_attr)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
		return;

	if (ch->GetExchange() || ch->GetMyShop() || ch->GetShopOwner() || ch->IsOpenSafebox() || ch->IsCubeOpen()) 
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(cmd_general)tradus:dragonsoul_plus_cannot_open"));
		return;
	}

	DWORD dwPos = 0;
	str_to_number(dwPos, arg1);
	BYTE iNeedFlame = 0;
	int iNeedYang = 0;

	if (dwPos < 0 || dwPos >= DRAGON_SOUL_INVENTORY_MAX_NUM)
		return;

	LPITEM item = ch->GetItem(TItemPos(DRAGON_SOUL_INVENTORY, dwPos));

	if (!item || item->IsExchanging() || item->IsEquipped())
		return;

	if (item->IsDragonSoul()) 
	{
		iNeedFlame = CHANGE_ATTR_NEED_FLAME[(item->GetVnum() / 100) % 10];
		iNeedYang = CHANGE_ATTR_NEED_YANG[(item->GetVnum() / 100) % 10];

		if (DSManager::instance().IsActiveDragonSoul(item)) 
		{
			ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(cmd_general)tradus:dragonsoul_plus_cannot_change_equiped"));
			return;
		}

		if (ch->GetGold() < iNeedYang)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(cmd_general)tradus:dragonsoul_plus_not_enough_yangs%i"), iNeedYang);
			return;
		}

		if (ch->CountSpecifyItem(CHANGE_ATTR_FLAME_VNUM) >= iNeedFlame) 
		{
			for (BYTE i=0;i < 6;i++) 
			{
				item->SetForceAttribute(i, 0, 0); 
			}
			
			bool ChangeAttr = DSManager::instance().ChangeAttributes(item);
			if (ChangeAttr)
				ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(cmd_general)tradus:dragonsoul_plus_change_success"));
			else
				ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(cmd_general)tradus:dragonsoul_plus_change_fail"));

			ch->RemoveSpecifyItem(CHANGE_ATTR_FLAME_VNUM, iNeedFlame);
			ch->ChangeGold(-5000000);
			ch->DragonSoulChangeAttrResult(ChangeAttr);
		}
		else
		{
			ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(cmd_general)tradus:dragonsoul_plus_not_enough_items"));
			return;
		}
	}
}
#endif
#ifdef PRODOMO_HIDE_COSTUME
ACMD (do_hide_costume)
{
	char arg1[256], arg2[256];
	two_arguments(argument, arg1, sizeof(arg1), arg2, sizeof(arg2));

	if (!*arg1)
		return;

	bool hidden = true;
	BYTE bPartPos = 0;
	BYTE bHidden = 0;

	str_to_number(bPartPos, arg1);

	if (*arg2)
	{
		str_to_number(bHidden, arg2);

		if (bHidden == 0)
			hidden = false;
	}

	bool bAttacking = (get_dword_time() - ch->GetLastAttackTime()) < 1500;
	bool bMoving = (get_dword_time() - ch->GetLastMoveTime()) < 1500;

	if (ch->IsStateMove() || bAttacking || bMoving)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1971]");
		return;
	}
	
	if (bPartPos == 1)
		ch->SetBodyCostumeHidden(hidden);
	else if (bPartPos == 2)
		ch->SetHairCostumeHidden(hidden);
	else if (bPartPos == 3)
		ch->SetSashCostumeHidden(hidden);
	else if (bPartPos == 4)
		ch->SetAuraCostumeHidden(hidden);
	else if (bPartPos == 5)
		ch->SetWeaponCostumeHidden(hidden);
	else
		return;

	ch->UpdatePacket();
}
#endif
#ifdef __ENABLE_ADVANCE_SKILL_SELECT__
ACMD(do_advance_skill_select)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));
	
	if (!*arg1)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "@@(cmd_general)tradus:Wrong Syntax");
		return;
	}
	if (ch->GetSkillGroup() != 0 || ch->GetLevel() < 5)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(cmd_general)tradus:YOU_CAN_NOT_DO_THAT"));
		return;
	}
	
	int iValue = 0;
	str_to_number(iValue, arg1);
	ch->SetSkillGroup(iValue);
	ch->ClearSkill();
}

#endif

ACMD(do_click_special_inv)
{
	ch->ChatPacket(CHAT_TYPE_COMMAND, "ShowSpecialInventory");
}

#if defined(__CONQUEROR_LEVEL__)
ACMD(do_conqueror_point)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
		return;

	if (ch->IsPolymorphed())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "@@(cmd_general)tradus:Nu poti sa ti dai punct cat timp esti transformat!");
		return;
	}

	if (ch->GetPoint(POINT_CONQUEROR_POINT) <= 0)
		return;

	BYTE idx = 0;
	if (!strcmp(arg1, "sstr"))
		idx = POINT_SUNGMA_STR;
	else if (!strcmp(arg1, "shp"))
		idx = POINT_SUNGMA_HP;
	else if (!strcmp(arg1, "smove"))
		idx = POINT_SUNGMA_MOVE;
	else if (!strcmp(arg1, "simmune"))
		idx = POINT_SUNGMA_IMMUNE;
	else
		return;

	if (ch->GetRealPoint(idx) >= 90)
		return;

	ch->SetRealPoint(idx, ch->GetRealPoint(idx) + 1);
	ch->SetPoint(idx, ch->GetPoint(idx) + 1);
	ch->ComputePoints();
	ch->PointChange(idx, 0);

	ch->PointChange(POINT_CONQUEROR_POINT, -1);
	ch->ComputePoints();

	ch->PointsPacket(); // Refresh points.
}
#endif

#ifdef ENABLE_GAYA_SYSTEM
ACMD(do_gaya_system)
{
	char arg1[255];
	char arg2[255];
	two_arguments (argument, arg1, sizeof(arg1), arg2, sizeof(arg2));

	if (0 == arg1[0])
		return;

	const std::string& strArg1 = std::string(arg1);

	if (strArg1 == "craft"){

		if (0 == arg2[0])
			return;

		int slot = atoi(arg2);
		ch->CraftGayaItems(slot);

	}
	else if(strArg1 == "market"){

		if (0 == arg2[0])
			return;

		int slot = atoi(arg2);
		ch->MarketGayaItems(slot);

	}
	else if(strArg1 == "refresh")
	{
		ch->RefreshGayaItems();
	}
}
#endif


#ifdef __DUNGEON_INFO__
ACMD(do_dungeon_info)
{
	std::vector<std::string> vecArgs;
	split_argument(argument, vecArgs);
	if (vecArgs.size() < 2) { return; }
	else if (vecArgs[1] == "rank")
	{
		if (vecArgs.size() < 4) { return; }
		DWORD mobIdx, rankIdx;
		if (!str_to_number(mobIdx, vecArgs[2].c_str()) || !str_to_number(rankIdx, vecArgs[3].c_str()))
			return;
		CHARACTER_MANAGER::Instance().SendDungeonRank(ch, mobIdx, rankIdx);
	}
	else if (vecArgs[1] == "test_cooldown" && ch->IsGM())
	{
		ch->SetQuestFlag("jotun.cooldown", time(0) + 30);
		ch->SetQuestFlag("hydra.cooldown", time(0) + 60);
		ch->SetQuestFlag("meley.cooldown", time(0) + 90);
		ch->SendDungeonCooldown(0);
	}
	else if (vecArgs[1] == "update" && ch->IsGM())
		ch->SendDungeonCooldown(0);
}
#endif
#ifdef ENABLE_GLOBAL_RANK
ACMD(do_open_global_rank)
{
	RankGlobal_send_players(ch);
}

ACMD(do_global_rank_end_season)
{
	if (!ch->IsGM())
	{
		return;
	}

	RankGlobal_end_season(ch);
}
#endif
#ifdef __CASKET_PREVIEW_ENABLE__
ACMD(do_fetch_casket_items)
{
	char arg1[256];
	one_argument(argument, arg1, sizeof(arg1));

	if (!*arg1)
	{
		return;
	}

	DWORD dwVnum = atoi(arg1);
	// Support for special_item_group
	{
		const auto ptrGroup = ITEM_MANAGER::instance().GetSpecialItemGroup(dwVnum);
		if (ptrGroup)
		{
			for (const auto & rItem : ptrGroup->m_vecItems)
			{
				ch->ChatPacket(CHAT_TYPE_COMMAND, "AddCasketItem %u %u %u", dwVnum, rItem.vnum, rItem.count);
			}

			ch->ChatPacket(CHAT_TYPE_COMMAND, "SortCasketWindow");
			return;
		}
	}

	if (m_casket_preview.find(dwVnum) == m_casket_preview.end())
	{
		return;
	}

	for (const auto & v_casket_content : m_casket_preview[dwVnum])
	{
		ch->ChatPacket(CHAT_TYPE_COMMAND, "AddCasketItem %u %u %u", dwVnum, v_casket_content.first, v_casket_content.second);
	}

	ch->ChatPacket(CHAT_TYPE_COMMAND, "SortCasketWindow");
}
#endif

#ifdef ENABLE_BATTLE_PASS
const char* battlepass[] = {
	"open",
	"premium",

	"\n",
};

ACMD(do_battle_pass)
{
	char arg1[256], arg2[256];

	int i, len;

	two_arguments(argument, arg1, sizeof(arg1), arg2, sizeof(arg2));

	if (!*arg1 || !*arg2)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "Usage: battle_pass <field> <value>");
		return;
	}

	len = strlen(arg1);

	for (i = 0; *(battlepass[i]) != '\n'; i++)
		if (!strncmp(arg1, battlepass[i], len))
			break;

	switch (i)
	{
	case 0:
		{
			int value = 0;
			str_to_number(value, arg2);
			if (ch->IsBattlePassOpen())
				return;

			ch->SetBattlePassOpen(true);
			ch->UpdateBattlePass();
		}
		break;

	case 1:
		{
			ch->SetPremiumBattlePass();
		}
	}
}
const char* battlepassadmin[] = {
	"completemission",
	"getmissions",
	"resetMissions",

	"\n",
};

ACMD(do_battle_pass_admin)
{
	char arg1[256], arg2[256];

	int i, len;

	two_arguments(argument, arg1, sizeof(arg1), arg2, sizeof(arg2));

	if (!*arg1 || !*arg2)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "Usage: battle_pass_admin <field> <value>");
		return;
	}

	len = strlen(arg1);

	for (i = 0; *(battlepassadmin[i]) != '\n'; i++)
		if (!strncmp(arg1, battlepassadmin[i], len))
			break;

	switch (i)
	{
	case 0:
		{
			int mission = 0;
			str_to_number(mission, arg2);

			for (auto& items : ch->GetPlayerBattlePass())
			{
				for (auto& missionInfo : items.second)
				{
					auto mission_map = CBattlePass::Instance().GetMissionMap();
					auto mission_data = mission_map[mission];
					if (missionInfo.missionID == mission)
					{
						const auto& maxProgress = mission_data.maxProgress;
						const auto& actualProgress = missionInfo.progress;
						const auto& pointsToComplete = maxProgress - actualProgress;

						ch->BattlePassAction(missionInfo.missionID, missionInfo.type, pointsToComplete);
					}
				}
			}
		}
		break;

	case 1:
		{
			for (const auto& items : ch->GetPlayerBattlePass())
			{
				for (const auto& missionInfo : items.second)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, "MissionID: %d, Type: %d, Progress: %d", missionInfo.missionID, missionInfo.type, missionInfo.progress);
				}
			}
		}
		break;
	case 2:
		{
			ch->ClearBattlePass();
		}
	}
}

#endif

#ifdef ENABLE_REWARD_SYSTEM
ACMD(do_reward_system)
{
	std::vector<std::string> vecArgs;
	split_argument(argument, vecArgs);
	if (vecArgs.size() < 2) { return; }
	else if (vecArgs[1] == "update")
	{
		CHARACTER_MANAGER::Instance().SendRewardInfo(255, true, false, ch);
		CHARACTER_MANAGER::Instance().SendRewardInfo(255, false, false, ch);
	}
}
#endif

