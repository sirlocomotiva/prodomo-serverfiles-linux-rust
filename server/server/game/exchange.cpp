#include "stdafx.h"
#include "../libgame/grid.h"
#include "utils.h"
#include "desc.h"
#include "desc_client.h"
#include "char.h"
#include "item.h"
#include "item_manager.h"
#include "packet.h"
#include "log.h"
#include "db.h"
#include "locale_service.h"
#include "../common/length.h"
#include "exchange.h"
#include "DragonSoul.h"
#include "questmanager.h" // @fixme150

#ifdef ENABLE_REMOVE_LIMIT_GOLD
void exchange_packet(LPCHARACTER ch, BYTE sub_header, bool is_me, unsigned long long arg1, TItemPos arg2, DWORD arg3, void * pvData = NULL);
#else
void exchange_packet(LPCHARACTER ch, BYTE sub_header, bool is_me, DWORD arg1, TItemPos arg2, DWORD arg3, void * pvData = NULL);
#endif


#ifdef ENABLE_REMOVE_LIMIT_GOLD
void exchange_packet(LPCHARACTER ch, BYTE sub_header, bool is_me, unsigned long long arg1, TItemPos arg2, DWORD arg3, void * pvData)
#else
void exchange_packet(LPCHARACTER ch, BYTE sub_header, bool is_me, DWORD arg1, TItemPos arg2, DWORD arg3, void * pvData)
#endif
{
	if (!ch->GetDesc())
		return;

	struct packet_exchange pack_exchg;

	pack_exchg.header 		= HEADER_GC_EXCHANGE;
	pack_exchg.sub_header 	= sub_header;
	pack_exchg.is_me		= is_me;
	pack_exchg.arg1		= arg1;
	pack_exchg.arg2		= arg2;
	pack_exchg.arg3		= arg3;

	if (sub_header == EXCHANGE_SUBHEADER_GC_ITEM_ADD && pvData)
	{
#ifdef WJ_ENABLE_TRADABLE_ICON
		pack_exchg.arg4 = TItemPos(((LPITEM) pvData)->GetWindow(), ((LPITEM) pvData)->GetCell());
#endif
		thecore_memcpy(&pack_exchg.alSockets, ((LPITEM) pvData)->GetSockets(), sizeof(pack_exchg.alSockets));
		thecore_memcpy(&pack_exchg.aAttr, ((LPITEM) pvData)->GetAttributes(), sizeof(pack_exchg.aAttr));
#ifdef ENABLE_REFINE_ELEMENT
		pack_exchg.dwRefineElement = ((LPITEM) pvData)->GetRefineElement();
#endif

#ifdef __CHANGELOOK_SYSTEM__
		pack_exchg.dwTransmutation = ((LPITEM) pvData)->GetTransmutation();
#endif
	}
	else
	{
#ifdef WJ_ENABLE_TRADABLE_ICON
		pack_exchg.arg4 = TItemPos(RESERVED_WINDOW, 0);
#endif
		memset(&pack_exchg.alSockets, 0, sizeof(pack_exchg.alSockets));
		memset(&pack_exchg.aAttr, 0, sizeof(pack_exchg.aAttr));
#ifdef ENABLE_REFINE_ELEMENT
		pack_exchg.dwRefineElement = 0;
#endif
#ifdef __CHANGELOOK_SYSTEM__
		pack_exchg.dwTransmutation = 0;
#endif
	}

	ch->GetDesc()->Packet(&pack_exchg, sizeof(pack_exchg));
}

// ��ȯ�� ����
bool CHARACTER::ExchangeStart(LPCHARACTER victim)
{
	if (this == victim)	// �ڱ� �ڽŰ��� ��ȯ�� ���Ѵ�.
		return false;

	if (IsObserverMode())
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot trade while spectating."));
		return false;
	}

	if (victim->IsNPC())
		return false;

	//PREVENT_TRADE_WINDOW
	if ( IsOpenSafebox() || GetShopOwner() || GetMyShop() || IsCubeOpen() || IsAuraRefineWindowOpen())
	{
		ChatPacket( CHAT_TYPE_INFO, LC_TEXT("You cannot trade if another trade window is open." ) );
		return false;
	}

	if ( victim->IsOpenSafebox() || victim->GetShopOwner() || victim->GetMyShop() || victim->IsCubeOpen() || IsAuraRefineWindowOpen())
	{
		ChatPacket( CHAT_TYPE_INFO, LC_TEXT("You cannot trade because the other player is in the middle of another transaction." ) );
		return false;
	}
	
	
#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	if (IsSecured())
	{
		ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT("|cffd93c3c[Err]:|h|r_Acest_cont_este_securizat!"));
		return false;
	}

	if (victim->IsSecured())
	{
		ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT("|cffd93c3c[Err]:|h|r_Acest_jucator_are_contul_securizat!"));
		return false;
	}
#endif
	
	//END_PREVENT_TRADE_WINDOW
	int iDist = DISTANCE_APPROX(GetX() - victim->GetX(), GetY() - victim->GetY());

	// �Ÿ� üũ
	if (iDist >= EXCHANGE_MAX_DISTANCE)
		return false;

	if (GetExchange())
		return false;

	if (victim->GetExchange())
	{
		exchange_packet(this, EXCHANGE_SUBHEADER_GC_ALREADY, 0, 0, NPOS, 0);
		return false;
	}

	if (victim->IsBlockMode(BLOCK_EXCHANGE))
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The other player is rejecting the exchange."));
		return false;
	}

	SetExchange(M2_NEW CExchange(this));
	victim->SetExchange(M2_NEW CExchange(victim));

	victim->GetExchange()->SetCompany(GetExchange());
	GetExchange()->SetCompany(victim->GetExchange());

	//
	SetExchangeTime();
	victim->SetExchangeTime();

	exchange_packet(victim, EXCHANGE_SUBHEADER_GC_START, 0, GetVID(), NPOS, 0);
	exchange_packet(this, EXCHANGE_SUBHEADER_GC_START, 0, victim->GetVID(), NPOS, 0);

	return true;
}

CExchange::CExchange(LPCHARACTER pOwner)
{
	m_pCompany = NULL;

	m_bAccept = false;

	for (int i = 0; i < EXCHANGE_ITEM_MAX_NUM; ++i)
	{
		m_apItems[i] = NULL;
		m_aItemPos[i] = NPOS;
		m_abItemDisplayPos[i] = 0;
	}

	m_lGold = 0;

	m_pOwner = pOwner;
	pOwner->SetExchange(this);

#ifdef __NEW_EXCHANGE_WINDOW__
	m_pGrid = M2_NEW CGrid(6, 4);
#else
	m_pGrid = M2_NEW CGrid(4, 3);
#endif
}

CExchange::~CExchange()
{
	M2_DELETE(m_pGrid);
}

bool CExchange::AddItem(TItemPos item_pos, BYTE display_pos)
{
	assert(m_pOwner != NULL && GetCompany());

	if (!item_pos.IsValidItemPosition())
		return false;

	if (item_pos.IsEquipPosition())
		return false;

	LPITEM item;

	if (!(item = m_pOwner->GetItem(item_pos)))
		return false;

	if (IS_SET(item->GetAntiFlag(), ITEM_ANTIFLAG_GIVE))
	{
		m_pOwner->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Item cannot be handed over."));
		return false;
	}

	if (true == item->isLocked())
	{
		return false;
	}
	if (item->IsExchanging())
	{
		sys_log(0, "EXCHANGE under exchanging");
		return false;
	}

	if (!m_pGrid->IsEmpty(display_pos, 1, item->GetSize()))
	{
		sys_log(0, "EXCHANGE not empty item_pos %d %d %d", display_pos, 1, item->GetSize());
		return false;
	}

	Accept(false);
	GetCompany()->Accept(false);

	for (int i = 0; i < EXCHANGE_ITEM_MAX_NUM; ++i)
	{
		if (m_apItems[i])
			continue;

		m_apItems[i]		= item;
		m_aItemPos[i]		= item_pos;
		m_abItemDisplayPos[i]	= display_pos;
		m_pGrid->Put(display_pos, 1, item->GetSize());

		item->SetExchanging(true);

		exchange_packet(m_pOwner,
				EXCHANGE_SUBHEADER_GC_ITEM_ADD,
				true,
				item->GetVnum(),
				TItemPos(RESERVED_WINDOW, display_pos),
				item->GetCount(),
				item);

		exchange_packet(GetCompany()->GetOwner(),
				EXCHANGE_SUBHEADER_GC_ITEM_ADD,
				false,
				item->GetVnum(),
				TItemPos(RESERVED_WINDOW, display_pos),
				item->GetCount(),
				item);

		sys_log(0, "EXCHANGE AddItem success %s pos(%d, %d) %d", item->GetName(), item_pos.window_type, item_pos.cell, display_pos);

		return true;
	}

	return false;
}

bool CExchange::RemoveItem(BYTE pos)
{
	if (pos >= EXCHANGE_ITEM_MAX_NUM)
		return false;

	if (!m_apItems[pos])
		return false;

	TItemPos PosOfInventory = m_aItemPos[pos];
	m_apItems[pos]->SetExchanging(false);

	m_pGrid->Get(m_abItemDisplayPos[pos], 1, m_apItems[pos]->GetSize());

	exchange_packet(GetOwner(),	EXCHANGE_SUBHEADER_GC_ITEM_DEL, true, pos, NPOS, 0);
	exchange_packet(GetCompany()->GetOwner(), EXCHANGE_SUBHEADER_GC_ITEM_DEL, false, pos, PosOfInventory, 0);

	Accept(false);
	GetCompany()->Accept(false);

	m_apItems[pos]	    = NULL;
	m_aItemPos[pos]	    = NPOS;
	m_abItemDisplayPos[pos] = 0;
	return true;
}

#ifdef ENABLE_REMOVE_LIMIT_GOLD
bool CExchange::AddGold(unsigned long long gold)
#else
bool CExchange::AddGold(long gold)
#endif
{
	if (gold <= 0)
		return false;

	if (GetOwner()->GetGold() < gold)
	{
		exchange_packet(GetOwner(), EXCHANGE_SUBHEADER_GC_LESS_GOLD, 0, 0, NPOS, 0);
		return false;
	}

	if (m_lGold > 0)
		return false;

	Accept(false);
	GetCompany()->Accept(false);

	m_lGold = gold;

	exchange_packet(GetOwner(), EXCHANGE_SUBHEADER_GC_GOLD_ADD, true, m_lGold, NPOS, 0);
	exchange_packet(GetCompany()->GetOwner(), EXCHANGE_SUBHEADER_GC_GOLD_ADD, false, m_lGold, NPOS, 0);
	return true;
}
bool CExchange::Check(int * piItemCount)
{
	if (GetOwner()->GetGold() < m_lGold)
		return false;

	int item_count = 0;

	for (int i = 0; i < EXCHANGE_ITEM_MAX_NUM; ++i)
	{
		if (!m_apItems[i])
			continue;

		if (!m_aItemPos[i].IsValidItemPosition())
			return false;

		if (m_apItems[i] != GetOwner()->GetItem(m_aItemPos[i]))
			return false;

		++item_count;
	}

	*piItemCount = item_count;
	return true;
}

bool CExchange::CheckSpace()
{
	std::array<std::unique_ptr<CGrid>, static_cast<const int>(INVENTORY_PAGE_COUNT)> s_grid;
	static std::vector <WORD> s_vDSGrid(DRAGON_SOUL_INVENTORY_MAX_NUM);
	


#ifdef ENABLE_CUSTOM_INVENTORY
	CGrid* pCustomGrids[CUSTOM_INVENTORY_CATEGORY_NUM][CUSTOM_INVENTORY_PAGE_COUNT];
	for (int i = 0; i < CUSTOM_INVENTORY_CATEGORY_NUM; i++)
	{
		for (int j = 0; j < CUSTOM_INVENTORY_PAGE_COUNT; j++)
		{
			pCustomGrids[i][j] = new CGrid(5, CUSTOM_INVENTORY_PAGE_SIZE / 5);
			pCustomGrids[i][j]->Clear();
		}
	}
#endif
	LPCHARACTER	victim = GetCompany()->GetOwner();
	LPCHARACTER    me = GetOwner();
	LPITEM item;
	
#ifdef ENABLE_CUSTOM_INVENTORY
	for (int m = 0; m < CUSTOM_INVENTORY_CATEGORY_NUM; m++)
	{
		for (int n = 0; n < CUSTOM_INVENTORY_PAGE_COUNT; n++)
		{
			int nAdd = n * CUSTOM_INVENTORY_PAGE_SIZE;
			for (int k = 0; k < CUSTOM_INVENTORY_PAGE_SIZE; k++)
			{
				if (!(item = victim->GetCustomInventoryItem(m, nAdd + k)))
					continue;
				
				pCustomGrids[m][n]->Put(k, 1, item->GetSize());
			}
		}
	}
#endif
	
	bool bDSInitialized = false;
	int new_size, invenpoint = GetCompany()->GetOwner()->Inven_Point();
	for (size_t i = 0; i < s_grid.size(); i++) {
		if (i < INVENTORY_OPEN_PAGE_COUNT) {
			s_grid[i] = std::make_unique<CGrid>(INVENTORY_WIDTH,INVENTORY_HEIGHT);
			continue;
		}
		if (invenpoint > 0 && invenpoint <= INVENTORY_HEIGHT)
			new_size = invenpoint;
		else if (invenpoint >= INVENTORY_HEIGHT)
			new_size = INVENTORY_HEIGHT;
		else
			new_size = 0;
		s_grid[i] = std::make_unique<CGrid>(INVENTORY_WIDTH,new_size);
		invenpoint -= INVENTORY_HEIGHT;
	}
	for (size_t j = 0; j < s_grid.size(); j++) {
		s_grid[j]->Clear();
		for (auto i = INVENTORY_PAGE_SIZE * j; i < INVENTORY_PAGE_SIZE * (j+1); ++i) 
		{
			if (!(item = GetCompany()->GetOwner()->GetInventoryItem(i))) 
				continue;
			s_grid[j]->Put(i - INVENTORY_PAGE_SIZE * j, 1, item->GetSize());
		}
	}
	for (auto i = 0; i < EXCHANGE_ITEM_MAX_NUM; ++i) 
	{
		if (!(item = m_apItems[i])) 
			continue;
		
#ifdef ENABLE_CUSTOM_INVENTORY
		BYTE itemSize = item->GetSize();
#endif
		
		if (item->IsDragonSoul()) 
		{
			if (!GetCompany()->GetOwner()->DragonSoul_IsQualified()) 
				return false;
			if (!bDSInitialized) 
			{
				bDSInitialized = true;
				GetCompany()->GetOwner()->CopyDragonSoulItemGrid(s_vDSGrid);
			}
			bool bExistEmptySpace = false;
			WORD wBasePos = DSManager::instance().GetBasePosition(item);
			if (wBasePos >= DRAGON_SOUL_INVENTORY_MAX_NUM) 
				return false;
			for (int i = 0; i < DRAGON_SOUL_BOX_SIZE; i++) {
				WORD wPos = wBasePos + i;
				if (0 == s_vDSGrid[wBasePos]) 
				{
					bool bEmpty = true;
					for (int j = 1; j < item->GetSize(); j++)	
					{
						if (s_vDSGrid[wPos + j * DRAGON_SOUL_BOX_COLUMN_NUM]) 
						{
							bEmpty = false;
							break;
						}
					} 
					if (bEmpty)	
					{
						for (int j = 0; j < item->GetSize(); j++)
							s_vDSGrid[wPos + j * DRAGON_SOUL_BOX_COLUMN_NUM] =  wPos + 1;
						bExistEmptySpace = true;
						break;
					}
				}
				if (bExistEmptySpace) 
					break;
            }
			if (!bExistEmptySpace) 
				return false;
		}
		else 
		{
			
#ifdef ENABLE_CUSTOM_INVENTORY
			// If item is special category, try find empty place in his category
			bool bFound = false;
			for (int m = 0; m < CUSTOM_INVENTORY_CATEGORY_NUM; m++)
			{
				if(bFound)
					break;
				
				if(item->IsCustomCategory(m))
				{
					for (int n = 0; n < CUSTOM_INVENTORY_PAGE_COUNT; n++)
					{
						int iCustomPos = pCustomGrids[m][n]->FindBlank(1, itemSize);
						
						if (iCustomPos >= 0) 
						{
							pCustomGrids[m][n]->Put(iCustomPos, 1, itemSize);
							bFound = true;
							break;
						}
					}
				}
			}
			
			if(bFound)
				continue;
#endif
			
			bool existspace = false;
			for (size_t i = 0; i < s_grid.size(); i++) 
			{
				const auto& iPos = s_grid[i]->FindBlank(1, item->GetSize());
				if (iPos >= 0) {
					s_grid[i]->Put(iPos, 1, item->GetSize());
					existspace = true;
					break;
				}
			}
			if (!existspace)
				return false;
		}
	}
	
	
#ifdef ENABLE_CUSTOM_INVENTORY
	for (int m = 0; m < CUSTOM_INVENTORY_CATEGORY_NUM; m++)
	{
		for (int n = 0; n < CUSTOM_INVENTORY_PAGE_COUNT; n++)
		{
			M2_DELETE(pCustomGrids[m][n]);
		}
	}
#endif
	
	return true;
}
bool CExchange::Done()
{
	int		empty_pos, i;
	LPITEM	item;

	LPCHARACTER	victim = GetCompany()->GetOwner();

	for (i = 0; i < EXCHANGE_ITEM_MAX_NUM; ++i)
	{
		if (!(item = m_apItems[i]))
			continue;

		if (item->IsDragonSoul())
			empty_pos = victim->GetEmptyDragonSoulInventory(item);
		else
#ifdef ENABLE_CUSTOM_INVENTORY
			empty_pos = victim->GetEmptyInventory(item);
#else
			empty_pos = victim->GetEmptyInventory(item->GetSize());

#endif

		if (empty_pos < 0)
		{
			sys_err("Exchange::Done : Cannot find blank position in inventory %s <-> %s item %s",
					m_pOwner->GetName(), victim->GetName(), item->GetName());
			continue;
		}

		assert(empty_pos >= 0);

		m_pOwner->SyncQuickslot(QUICKSLOT_TYPE_ITEM, item->GetCell(), 255);

		item->RemoveFromCharacter();
		if (item->IsDragonSoul())
			item->AddToCharacter(victim, TItemPos(DRAGON_SOUL_INVENTORY, empty_pos));
		else
			item->AddToCharacter(victim, TItemPos(INVENTORY, empty_pos));
		ITEM_MANAGER::instance().FlushDelayedSave(item);

		item->SetExchanging(false);
		{
			char exchange_buf[51];

			snprintf(exchange_buf, sizeof(exchange_buf), "%s %u %u", item->GetName(), GetOwner()->GetPlayerID(), item->GetCount());
			LogManager::instance().ItemLog(victim, item, "EXCHANGE_TAKE", exchange_buf);

			snprintf(exchange_buf, sizeof(exchange_buf), "%s %u %u", item->GetName(), victim->GetPlayerID(), item->GetCount());
			LogManager::instance().ItemLog(GetOwner(), item, "EXCHANGE_GIVE", exchange_buf);

			if (item->GetVnum() >= 80003 && item->GetVnum() <= 80007)
			{
				LogManager::instance().GoldBarLog(victim->GetPlayerID(), item->GetID(), EXCHANGE_TAKE, "");
				LogManager::instance().GoldBarLog(GetOwner()->GetPlayerID(), item->GetID(), EXCHANGE_GIVE, "");
			}
		}

		m_apItems[i] = NULL;
	}

	if (m_lGold)
	{
#ifdef ENABLE_REMOVE_LIMIT_GOLD
		GetOwner()->ChangeGold(-m_lGold);
		victim->ChangeGold(m_lGold);
#else
		GetOwner()->PointChange(POINT_GOLD, -m_lGold, true);
		victim->PointChange(POINT_GOLD, m_lGold, true);
#endif

		if (m_lGold > 1000)
		{
			char exchange_buf[51];
			snprintf(exchange_buf, sizeof(exchange_buf), "%u %s", GetOwner()->GetPlayerID(), GetOwner()->GetName());
			LogManager::instance().CharLog(victim, m_lGold, "EXCHANGE_GOLD_TAKE", exchange_buf);

			snprintf(exchange_buf, sizeof(exchange_buf), "%u %s", victim->GetPlayerID(), victim->GetName());
			LogManager::instance().CharLog(GetOwner(), m_lGold, "EXCHANGE_GOLD_GIVE", exchange_buf);
		}
	}

	m_pGrid->Clear();
	return true;
}

// ��ȯ�� ����
bool CExchange::Accept(bool bAccept)
{
	if (m_bAccept == bAccept)
		return true;

	m_bAccept = bAccept;

	// �� �� ���� �����Ƿ� ��ȯ ����
	if (m_bAccept && GetCompany()->m_bAccept)
	{
		int	iItemCount;

		LPCHARACTER victim = GetCompany()->GetOwner();

		//PREVENT_PORTAL_AFTER_EXCHANGE
		GetOwner()->SetExchangeTime();
		victim->SetExchangeTime();
		//END_PREVENT_PORTAL_AFTER_EXCHANGE

		// @fixme150 BEGIN
		if (quest::CQuestManager::instance().GetPCForce(GetOwner()->GetPlayerID())->IsRunning() == true)
		{
			GetOwner()->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot trade if you're using quests"));
			victim->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot trade if the other part using quests"));
			goto EXCHANGE_END;
		}
		else if (quest::CQuestManager::instance().GetPCForce(victim->GetPlayerID())->IsRunning() == true)
		{
			victim->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot trade if you're using quests"));
			GetOwner()->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot trade if the other part using quests"));
			goto EXCHANGE_END;
		}
		// @fixme150 END

		// exchange_check ������ ��ȯ�� �����۵��� ���ڸ��� �ֳ� Ȯ���ϰ�,
		// ��ũ�� ����� �ֳ� Ȯ���Ѵ�, �ι�° ���ڷ� ��ȯ�� ������ ����
		// �� �����Ѵ�.
		if (!Check(&iItemCount))
		{
			GetOwner()->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You are out of money or an item is out of place."));
			victim->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The opponent is out of money or the item is out of place."));
			goto EXCHANGE_END;
		}

		// ���� ���� ������ ������ ������ ����ǰ�� ���� �ڸ��� �ֳ� Ȯ���Ѵ�.
		if (!CheckSpace())
		{
			GetOwner()->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("There are no empty spaces in the opponent's inventory."));
			victim->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("There are no empty spaces in your inventory."));
			goto EXCHANGE_END;
		}

		// ���浵 ����������..
		if (!GetCompany()->Check(&iItemCount))
		{
			victim->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You are out of money or an item is out of place."));
			GetOwner()->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The opponent is out of money or the item is out of place."));
			goto EXCHANGE_END;
		}

		if (!GetCompany()->CheckSpace())
		{
			victim->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("There are no empty spaces in the opponent's inventory."));
			GetOwner()->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("There are no empty spaces in your inventory."));
			goto EXCHANGE_END;
		}

		if (db_clientdesc->GetSocket() == INVALID_SOCKET)
		{
			sys_err("Cannot use exchange feature while DB cache connection is dead");
			victim->ChatPacket(CHAT_TYPE_INFO, "Unknown error");
			GetOwner()->ChatPacket(CHAT_TYPE_INFO, "Unknown error");
			goto EXCHANGE_END;
		}

		if (Done())
		{
			if (m_lGold)
				GetOwner()->Save();

			if (GetCompany()->Done())
			{
				if (GetCompany()->m_lGold) // ���� ���� ���� ����
					victim->Save();

				// INTERNATIONAL_VERSION
				GetOwner()->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The exchange with %s has been completed."), victim->GetName());
				victim->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The exchange with %s has been completed."), GetOwner()->GetName());
				// END_OF_INTERNATIONAL_VERSION
			}
		}

EXCHANGE_END:
		Cancel();
		return false;
	}
	else
	{
		// �ƴϸ� accept�� ���� ��Ŷ�� ������.
		exchange_packet(GetOwner(), EXCHANGE_SUBHEADER_GC_ACCEPT, true, m_bAccept, NPOS, 0);
		exchange_packet(GetCompany()->GetOwner(), EXCHANGE_SUBHEADER_GC_ACCEPT, false, m_bAccept, NPOS, 0);
		return true;
	}
}

// ��ȯ ���
void CExchange::Cancel()
{
	exchange_packet(GetOwner(), EXCHANGE_SUBHEADER_GC_END, 0, 0, NPOS, 0);
	GetOwner()->SetExchange(NULL);

	for (int i = 0; i < EXCHANGE_ITEM_MAX_NUM; ++i)
	{
		if (m_apItems[i])
			m_apItems[i]->SetExchanging(false);
	}

	if (GetCompany())
	{
		GetCompany()->SetCompany(NULL);
		GetCompany()->Cancel();
	}

	M2_DELETE(this);
}

