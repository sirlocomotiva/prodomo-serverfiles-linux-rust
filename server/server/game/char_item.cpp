#include "stdafx.h"
#include <stack>
#include "utils.h"
#include "config.h"
#include "char.h"
#include "char_manager.h"
#include "item_manager.h"
#include "desc.h"
#include "desc_client.h"
#include "desc_manager.h"
#include "packet.h"
#include "affect.h"
#include "item_stack_attribute.h"
#include "skill.h"
#include "start_position.h"
#include "mob_manager.h"
#include "db.h"
#include "log.h"
#include "vector.h"
#include "buffer_manager.h"
#include "questmanager.h"
#include "fishing.h"
#include "party.h"
#include "dungeon.h"
#include "refine.h"
#include "unique_item.h"
#include "war_map.h"
#include "xmas_event.h"
#include "marriage.h"
#include "monarch.h"
#include "polymorph.h"
#include "blend_item.h"
#include "castle.h"
#include "BattleArena.h"
#include "arena.h"
#include "dev_log.h"
#include "threeway_war.h"
#include "safebox.h"
#include "shop.h"
#include "refactorized_switchbot.h"
#include "pvp.h"
#include "shop_search.h"
#include "MeleyLair.h"
#include "../common/VnumHelper.h"
#include "DragonSoul.h"
#include "buff_on_attributes.h"
#include "belt_inventory_helper.h"
#include "../common/prodomodefines.h"
#include "ItemUtils.h"
#include "private_shop_manager.h"
#include "private_shop.h"
#ifdef __ENABLE_SHAMAN_SYSTEM__
	#include "ShamanSystem.h"
#endif

#ifdef ENABLE_BATTLE_PASS
#include "BattlePassManager.h"
#endif
const int ITEM_BROKEN_METIN_VNUM = 28960;
#define ENABLE_EFFECT_EXTRAPOT
#define ENABLE_BOOKS_STACKFIX
const char CHARACTER::msc_szLastChangeItemAttrFlag[] = "Item.LastChangeItemAttr";
const BYTE g_aBuffOnAttrPoints[] = { POINT_ENERGY, POINT_COSTUME_ATTR_BONUS };

struct FFindStone
{
	std::map<DWORD, LPCHARACTER> m_mapStone;

	void operator()(LPENTITY pEnt)
	{
		if (pEnt->IsType(ENTITY_CHARACTER) == true)
		{
			LPCHARACTER pChar = (LPCHARACTER)pEnt;

			if (pChar->IsStone() == true)
			{
				m_mapStone[(DWORD)pChar->GetVID()] = pChar;
			}
		}
	}
};
bool IS_SUMMON_ITEM(int vnum)
{
	switch (vnum)
	{
		case 22000:
		case 22010:
		case 22011:
		case 22020:
		case ITEM_MARRIAGE_RING:
			return true;
	}

	return false;
}

static bool IS_MONKEY_DUNGEON(int map_index)
{
	switch (map_index)
	{
		case 5:
		case 25:
		case 45:
		case 108:
		case 109:
			return true;;
	}

	return false;
}

bool IS_SUMMONABLE_ZONE(int map_index)
{
	// ��Ű����
	if (IS_MONKEY_DUNGEON(map_index))
		return false;
	// ��
	if (IS_CASTLE_MAP(map_index))
		return false;

	switch (map_index)
	{
		case 66 : // ���Ÿ��
		case 71 : // �Ź� ���� 2��
		case 72 : // õ�� ����
		case 73 : // õ�� ���� 2��
		case 193 : // �Ź� ���� 2-1��
#if 0
		case 184 : // õ�� ����(�ż�)
		case 185 : // õ�� ���� 2��(�ż�)
		case 186 : // õ�� ����(õ��)
		case 187 : // õ�� ���� 2��(õ��)
		case 188 : // õ�� ����(����)
		case 189 : // õ�� ���� 2��(����)
#endif
//		case 206 : // �Ʊ͵���
		case 216 : // �Ʊ͵���
		case 217 : // �Ź� ���� 3��
		case 208 : // õ�� ���� (���)

		case 113 : // OX Event ��
			return false;
	}

	if (CBattleArena::IsBattleArenaMap(map_index)) return false;

	// ��� private ������ ���� �Ұ���
	if (map_index > 10000) return false;

	return true;
}

bool IS_BOTARYABLE_ZONE(int nMapIndex)
{
	if (!g_bEnableBootaryCheck) return true;

	switch (nMapIndex)
	{
		case 1 :
		case 3 :
		case 21 :
		case 23 :
		case 41 :
		case 43 :
			return true;
	}

	return false;
}

// item socket �� ������Ÿ�԰� ������ üũ -- by mhh
static bool FN_check_item_socket(LPITEM item)
{
	for (int i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
	{
		if (item->GetSocket(i) != item->GetProto()->alSockets[i])
			return false;
	}

	return true;
}

// item socket ���� -- by mhh
static void FN_copy_item_socket(LPITEM dest, LPITEM src)
{
	for (int i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
	{
		dest->SetSocket(i, src->GetSocket(i));
	}
}
static bool FN_check_item_sex(LPCHARACTER ch, LPITEM item)
{
	// ���� ����
	if (IS_SET(item->GetAntiFlag(), ITEM_ANTIFLAG_MALE))
	{
		if (SEX_MALE==GET_SEX(ch))
			return false;
	}
	// ���ڱ���
	if (IS_SET(item->GetAntiFlag(), ITEM_ANTIFLAG_FEMALE))
	{
		if (SEX_FEMALE==GET_SEX(ch))
			return false;
	}

	return true;
}


/////////////////////////////////////////////////////////////////////////////
// ITEM HANDLING
/////////////////////////////////////////////////////////////////////////////
bool CHARACTER::CanHandleItem(bool bSkipCheckRefine, bool bSkipObserver)
{
	if (!bSkipObserver)
		if (m_bIsObserver)
			return false;

	if (GetMyShop())
		return false;

	if (!bSkipCheckRefine)
		if (m_bUnderRefine)
			return false;

	if (IsCubeOpen() || NULL != DragonSoul_RefineWindow_GetOpener())
		return false;

	if (IsWarping())
		return false;
	
#ifdef __SASH_SYSTEM__
	if ((m_bSashCombination) || (m_bSashAbsorption))
		return false;
#endif
#ifdef __AURA_SYSTEM__
	if (IsAuraRefineWindowOpen() || NULL != GetAuraRefineWindowOpener())
		return false;
#endif
#ifdef __CHANGELOOK_SYSTEM__
	if (m_bChangeLook)
		return false;
#endif


	return true;
}

LPITEM CHARACTER::GetInventoryItem(WORD wCell) const
{
	return GetItem(TItemPos(INVENTORY, wCell));
}

LPITEM CHARACTER::GetItem(TItemPos Cell) const
{
	if (!IsValidItemPosition(Cell))
		return NULL;
	WORD wCell = Cell.cell;
	BYTE window_type = Cell.window_type;
	switch (window_type)
	{
	case INVENTORY:
	case EQUIPMENT:
		if (wCell >= INVENTORY_AND_EQUIP_SLOT_MAX)
		{
			sys_err("CHARACTER::GetInventoryItem: invalid item cell %d", wCell);
			return NULL;
		}
		return m_pointsInstant.pItems[wCell];
	case DRAGON_SOUL_INVENTORY:
		if (wCell >= DRAGON_SOUL_INVENTORY_MAX_NUM)
		{
			sys_err("CHARACTER::GetInventoryItem: invalid DS item cell %d", wCell);
			return NULL;
		}
		return m_pointsInstant.pDSItems[wCell];

#if defined(__ATTR_6TH_7TH__)
	case ATTR67_ADD:
	{
		if (wCell >= ATTR67_ADD_SLOT_MAX)
		{
			sys_err("CHARACTER::GetItem: invalid ATTR67_ADD item cell %d", wCell);
			return NULL;
		}

		return m_pointsInstant.pAttr67AddItem;
	}
#endif

#ifdef ENABLE_SWITCHBOT
	case SWITCHBOT:
		if (wCell >= SWITCHBOT_SLOT_COUNT)
		{
			sys_err("CHARACTER::GetInventoryItem: invalid switchbot item cell %d", wCell);
			return NULL;
		}
		return m_pointsInstant.pSwitchbotItems[wCell];
#endif

	default:
		return NULL;
	}
	return NULL;
}


#ifdef ENABLE_CUSTOM_INVENTORY
LPITEM CHARACTER::GetCustomInventoryItem(BYTE bCategory, WORD wCell) const
{
	if(bCategory >= CUSTOM_INVENTORY_CATEGORY_NUM)
		return NULL;
	
	WORD wCustomStartIndex = CUSTOM_INVENTORY_SLOT_START + (bCategory * CUSTOM_INVENTORY_MAX_NUM);
	WORD wRealCell = wCustomStartIndex + wCell;
	return GetItem(TItemPos(INVENTORY, wRealCell));
}

int CHARACTER::GetEmptyCustomInventory(BYTE bCategory, BYTE size) const
{
	if(bCategory >= CUSTOM_INVENTORY_CATEGORY_NUM)
		return -1;
	
	WORD wCategoryStartIndex = CUSTOM_INVENTORY_SLOT_START + (bCategory * CUSTOM_INVENTORY_MAX_NUM);
	WORD wCategoryEndIndex = CUSTOM_INVENTORY_SLOT_START + ((bCategory + 1) * CUSTOM_INVENTORY_MAX_NUM);
	
	for (int i = wCategoryStartIndex; i < wCategoryEndIndex; ++i)
		if (IsEmptyItemGrid(TItemPos(INVENTORY, i), size))
			return i;
		
	return -1;
}

BYTE CHARACTER::GetInventoryPageByPos(int iCategory, WORD wPos) const
{
	if(wPos < INVENTORY_MAX_NUM && iCategory == -1)
		return wPos / (INVENTORY_PAGE_SIZE);

	for(BYTE pageIndex = 0; pageIndex < CUSTOM_INVENTORY_PAGE_COUNT; pageIndex++)
	{
		WORD wPosStart = CUSTOM_INVENTORY_SLOT_START + (iCategory * CUSTOM_INVENTORY_MAX_NUM);
		if(wPos >= (wPosStart + pageIndex * CUSTOM_INVENTORY_PAGE_SIZE) && wPos < (wPosStart + (pageIndex + 1) * CUSTOM_INVENTORY_PAGE_SIZE))
			return pageIndex;
	}
	
	return -1;
}

int CHARACTER::GetInventoryTypeByPos(WORD wPos) const
{
	if(wPos < INVENTORY_MAX_NUM)
		return 0;

	for(BYTE catIndex = 0; catIndex < CUSTOM_INVENTORY_CATEGORY_NUM; catIndex++)
	{
		if(wPos >= (CUSTOM_INVENTORY_SLOT_START + catIndex * CUSTOM_INVENTORY_MAX_NUM) && wPos < (CUSTOM_INVENTORY_SLOT_START + (catIndex + 1) * CUSTOM_INVENTORY_MAX_NUM))
			return catIndex + 1;
	}
	
	return -1;
}
#endif


#if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
void CHARACTER::SetItem(TItemPos Cell, LPITEM pItem, bool bHighlight)
#else
void CHARACTER::SetItem(TItemPos Cell, LPITEM pItem)
#endif
{
	WORD wCell = Cell.cell;
	BYTE window_type = Cell.window_type;
	if ((unsigned long)((CItem*)pItem) == 0xff || (unsigned long)((CItem*)pItem) == 0xffffffff)
	{
		sys_err("!!! FATAL ERROR !!! item == 0xff (char: %s cell: %u)", GetName(), wCell);
		core_dump();
		return;
	}

	if (pItem && pItem->GetOwner())
	{
		assert(!"GetOwner exist");
		return;
	}
	// �⺻ �κ��丮
	switch(window_type)
	{
	case INVENTORY:
	case EQUIPMENT:
		{
			if (wCell >= INVENTORY_AND_EQUIP_SLOT_MAX)
			{
				sys_err("CHARACTER::SetItem: invalid item cell %d", wCell);
				return;
			}


#ifdef ENABLE_CUSTOM_INVENTORY
			WORD wCategoryStartIndex = 0;
			WORD wCategoryEndIndex = INVENTORY_MAX_NUM;
			
			if(Cell.IsCustomInventoryPosition() && Cell.GetCustomInventoryCategory() != -1)
			{
				wCategoryStartIndex = CUSTOM_INVENTORY_SLOT_START + (Cell.GetCustomInventoryCategory() * CUSTOM_INVENTORY_MAX_NUM);
				wCategoryEndIndex = CUSTOM_INVENTORY_SLOT_START + ((Cell.GetCustomInventoryCategory() + 1) * CUSTOM_INVENTORY_MAX_NUM);
			}
#endif


			LPITEM pOld = m_pointsInstant.pItems[wCell];

			if (pOld)
			{
#ifdef ENABLE_CUSTOM_INVENTORY
				if (wCell >= wCategoryStartIndex && wCell < wCategoryEndIndex)
#else
				if (wCell < INVENTORY_MAX_NUM)
#endif
				{
					for (int i = 0; i < pOld->GetSize(); ++i)
					{
						int p = wCell + (i * 5);

#ifdef ENABLE_CUSTOM_INVENTORY
						if (p >= wCategoryEndIndex)
							continue;
#else
						if (p >= INVENTORY_MAX_NUM)
							continue;
#endif

						if (m_pointsInstant.pItems[p] && m_pointsInstant.pItems[p] != pOld)
							continue;

						m_pointsInstant.bItemGrid[p] = 0;
					}
				}
				else
					m_pointsInstant.bItemGrid[wCell] = 0;
			}

			if (pItem)
			{
#ifdef ENABLE_CUSTOM_INVENTORY
				if (wCell >= wCategoryStartIndex && wCell < wCategoryEndIndex)
#else
				if (wCell < INVENTORY_MAX_NUM)
#endif
				{
					for (int i = 0; i < pItem->GetSize(); ++i)
					{
						int p = wCell + (i * 5);

#ifdef ENABLE_CUSTOM_INVENTORY
						if (p >= wCategoryEndIndex)
							continue;
#else
						if (p >= INVENTORY_MAX_NUM)
							continue;
#endif
						m_pointsInstant.bItemGrid[p] = wCell + 1;
					}
				}
				else
					m_pointsInstant.bItemGrid[wCell] = wCell + 1;
			}

			m_pointsInstant.pItems[wCell] = pItem;
		}
		break;
	case DRAGON_SOUL_INVENTORY:
		{
			LPITEM pOld = m_pointsInstant.pDSItems[wCell];

			if (pOld)
			{
				if (wCell < DRAGON_SOUL_INVENTORY_MAX_NUM)
				{
					for (int i = 0; i < pOld->GetSize(); ++i)
					{
						int p = wCell + (i * DRAGON_SOUL_BOX_COLUMN_NUM);

						if (p >= DRAGON_SOUL_INVENTORY_MAX_NUM)
							continue;

						if (m_pointsInstant.pDSItems[p] && m_pointsInstant.pDSItems[p] != pOld)
							continue;

						m_pointsInstant.wDSItemGrid[p] = 0;
					}
				}
				else
					m_pointsInstant.wDSItemGrid[wCell] = 0;
			}

			if (pItem)
			{
				if (wCell >= DRAGON_SOUL_INVENTORY_MAX_NUM)
				{
					sys_err("CHARACTER::SetItem: invalid DS item cell %d", wCell);
					return;
				}

				if (wCell < DRAGON_SOUL_INVENTORY_MAX_NUM)
				{
					for (int i = 0; i < pItem->GetSize(); ++i)
					{
						int p = wCell + (i * DRAGON_SOUL_BOX_COLUMN_NUM);

						if (p >= DRAGON_SOUL_INVENTORY_MAX_NUM)
							continue;
						m_pointsInstant.wDSItemGrid[p] = wCell + 1;
					}
				}
				else
					m_pointsInstant.wDSItemGrid[wCell] = wCell + 1;
			}

			m_pointsInstant.pDSItems[wCell] = pItem;
		}
		break;
#if defined(__ATTR_6TH_7TH__)
	case ATTR67_ADD:
	{
		if (wCell >= ATTR67_ADD_SLOT_MAX)
		{
			sys_err("CHARACTER::SetItem: invalid ATTR67_ADD item cell %d", wCell);
			return;
		}
		m_pointsInstant.pAttr67AddItem = pItem;
	}
	break;
#endif
#ifdef ENABLE_SWITCHBOT
	case SWITCHBOT:
	{
		LPITEM pOld = m_pointsInstant.pSwitchbotItems[wCell];
		if (pItem && pOld)
		{
			return;
		}

		if (wCell >= SWITCHBOT_SLOT_COUNT)
		{
			sys_err("CHARACTER::SetItem: invalid switchbot item cell %d", wCell);
			return;
		}

		if (pItem)
		{
			CSwitchbotManager::Instance().RegisterItem(GetPlayerID(), pItem->GetID(), wCell);
		}
		else
		{
			CSwitchbotManager::Instance().UnregisterItem(GetPlayerID(), wCell);
		}

		m_pointsInstant.pSwitchbotItems[wCell] = pItem;
	}
	break;
#endif
	default:
		sys_err ("Invalid Inventory type %d", window_type);
		return;
	}

	if (GetDesc())
	{
		if (pItem)
		{
			TPacketGCItemSet pack;
			pack.header = HEADER_GC_ITEM_SET;
			pack.Cell = Cell;

			pack.count = pItem->GetCount();
#ifdef ENABLE_REFINE_ELEMENT
			pack.dwRefineElement = pItem->GetRefineElement();
#endif
#ifdef __CHANGELOOK_SYSTEM__
			pack.transmutation = pItem->GetTransmutation();
#endif
			pack.vnum = pItem->GetVnum();
			pack.flags = pItem->GetFlag();
			pack.anti_flags	= pItem->GetAntiFlag();
#if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
			pack.highlight = bHighlight;
#else
			pack.highlight = (Cell.window_type == DRAGON_SOUL_INVENTORY);
#endif

			thecore_memcpy(pack.alSockets, pItem->GetSockets(), sizeof(pack.alSockets));
			thecore_memcpy(pack.aAttr, pItem->GetAttributes(), sizeof(pack.aAttr));
			GetDesc()->Packet(&pack, sizeof(TPacketGCItemSet));
		}
		else
		{
			TPacketGCItemDelDeprecated pack;
			pack.header = HEADER_GC_ITEM_DEL;
			pack.Cell = Cell;
			pack.count = 0;
#ifdef ENABLE_REFINE_ELEMENT
			pack.dwRefineElement = 0;
#endif
#ifdef __CHANGELOOK_SYSTEM__
			pack.transmutation = 0;
#endif
			pack.vnum = 0;
			memset(pack.alSockets, 0, sizeof(pack.alSockets));
			memset(pack.aAttr, 0, sizeof(pack.aAttr));
			GetDesc()->Packet(&pack, sizeof(TPacketGCItemDelDeprecated));
		}
	}

	if (pItem)
	{
		pItem->SetCell(this, wCell);
		switch (window_type)
		{
		case INVENTORY:
		case EQUIPMENT:
#ifdef ENABLE_CUSTOM_INVENTORY
			if ((wCell < INVENTORY_MAX_NUM) || (BELT_INVENTORY_SLOT_START <= wCell && BELT_INVENTORY_SLOT_END > wCell) || Cell.IsCustomInventoryPosition())
#else
			if ((wCell < INVENTORY_MAX_NUM) || (BELT_INVENTORY_SLOT_START <= wCell && BELT_INVENTORY_SLOT_END > wCell))
#endif
				pItem->SetWindow(INVENTORY);
			else
				pItem->SetWindow(EQUIPMENT);
			break;
		case DRAGON_SOUL_INVENTORY:
			pItem->SetWindow(DRAGON_SOUL_INVENTORY);
			break;
#if defined(__ATTR_6TH_7TH__)
		case ATTR67_ADD:
			pItem->SetWindow(ATTR67_ADD);
			break;
#endif
#ifdef ENABLE_SWITCHBOT
		case SWITCHBOT:
			pItem->SetWindow(SWITCHBOT);
			break;
#endif	
		}
	}
}

LPITEM CHARACTER::GetWear(BYTE bCell) const
{
	if (bCell >= WEAR_MAX_NUM + DRAGON_SOUL_DECK_MAX_NUM * DS_SLOT_MAX)
	{
		sys_err("CHARACTER::GetWear: invalid wear cell %d", bCell);
		return NULL;
	}

	return m_pointsInstant.pItems[INVENTORY_MAX_NUM + bCell];
}

void CHARACTER::SetWear(BYTE bCell, LPITEM item)
{
	if (bCell >= WEAR_MAX_NUM + DRAGON_SOUL_DECK_MAX_NUM * DS_SLOT_MAX)
	{
		sys_err("CHARACTER::SetItem: invalid item cell %d", bCell);
		return;
	}

#if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
	SetItem(TItemPos(INVENTORY, INVENTORY_MAX_NUM + bCell), item, false);
#else
	SetItem(TItemPos (INVENTORY, INVENTORY_MAX_NUM + bCell), item);
#endif

	if (!item && bCell == WEAR_WEAPON)
	{
		if (IsAffectFlag(AFF_GWIGUM))
			RemoveAffect(SKILL_GWIGEOM);

		if (IsAffectFlag(AFF_GEOMGYEONG))
			RemoveAffect(SKILL_GEOMKYUNG);
	}
}

void CHARACTER::ClearItem()
{
	int		i;
	LPITEM	item;

	for (i = 0; i < INVENTORY_AND_EQUIP_SLOT_MAX; ++i)
	{
		if ((item = GetInventoryItem(i)))
		{
			item->SetSkipSave(true);
			ITEM_MANAGER::instance().FlushDelayedSave(item);

			item->RemoveFromCharacter();
			M2_DESTROY_ITEM(item);

			SyncQuickslot(QUICKSLOT_TYPE_ITEM, i, 255);
		}
	}
	for (i = 0; i < DRAGON_SOUL_INVENTORY_MAX_NUM; ++i)
	{
		if ((item = GetItem(TItemPos(DRAGON_SOUL_INVENTORY, i))))
		{
			item->SetSkipSave(true);
			ITEM_MANAGER::instance().FlushDelayedSave(item);

			item->RemoveFromCharacter();
			M2_DESTROY_ITEM(item);
		}
	}
	if ((item = GetAttr67AddItem()))
	{
		item->SetSkipSave(true);
		ITEM_MANAGER::instance().FlushDelayedSave(item);

		item->RemoveFromCharacter();
		M2_DESTROY_ITEM(item);
	}
	
	
#ifdef ENABLE_SWITCHBOT
	for (i = 0; i < SWITCHBOT_SLOT_COUNT; ++i)
	{
		if ((item = GetItem(TItemPos(SWITCHBOT, i))))
		{
			item->SetSkipSave(true);
			ITEM_MANAGER::instance().FlushDelayedSave(item);

			item->RemoveFromCharacter();
			M2_DESTROY_ITEM(item);
		}
	}
#endif
	
}

bool CHARACTER::IsEmptyItemGrid(TItemPos Cell, BYTE bSize, int iExceptionCell) const
{
#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	switch (Cell.window_type)
	{
	case INVENTORY:
		{
#ifdef ENABLE_CUSTOM_INVENTORY
			WORD bCell = Cell.cell;
			

			WORD wCategoryStartIndex = 0;
			WORD wCategoryEndIndex = INVENTORY_MAX_NUM;
			BYTE bInventoryPageSize = INVENTORY_PAGE_SIZE;
			
			if(Cell.IsCustomInventoryPosition() && Cell.GetCustomInventoryCategory() != -1)
			{
			
				wCategoryStartIndex = CUSTOM_INVENTORY_SLOT_START + (Cell.GetCustomInventoryCategory() * CUSTOM_INVENTORY_MAX_NUM);
				wCategoryEndIndex = CUSTOM_INVENTORY_SLOT_START + ((Cell.GetCustomInventoryCategory() + 1) * CUSTOM_INVENTORY_MAX_NUM);
				bInventoryPageSize = CUSTOM_INVENTORY_PAGE_SIZE;
			}
#else
			WORD bCell = Cell.cell;
#endif
			++iExceptionCell;

			if (Cell.IsBeltInventoryPosition())
			{
				LPITEM beltItem = GetWear(WEAR_BELT);
				if (NULL == beltItem)
					return false;
				if (false == CBeltInventoryHelper::IsAvailableCell(bCell - BELT_INVENTORY_SLOT_START, beltItem->GetValue(0)))
					return false;
				if (m_pointsInstant.bItemGrid[bCell]) {
					if (m_pointsInstant.bItemGrid[bCell] == iExceptionCell)
						return true;
					return false;
				}
				if (bSize == 1)
					return true;
			}
#ifdef ENABLE_CUSTOM_INVENTORY
			else if (bCell >= wCategoryEndIndex)
				return false;
#else
			else if (bCell >= Inventory_Size())
				return false;
#endif

			if (m_pointsInstant.bItemGrid[bCell]) {
				if (m_pointsInstant.bItemGrid[bCell] == iExceptionCell) {
					if (bSize == 1)
						return true;

					int j = 1;
#ifdef ENABLE_CUSTOM_INVENTORY
					BYTE bPage = GetInventoryPageByPos(Cell.GetCustomInventoryCategory(), bCell);
#else
					BYTE bPage = bCell / (INVENTORY_MAX_NUM / INVENTORY_PAGE_COUNT);

#endif
					do {
#ifdef ENABLE_CUSTOM_INVENTORY
						WORD p = bCell + (5 * j);

						if (p >= wCategoryEndIndex)
							return false;

						if (GetInventoryPageByPos(Cell.GetCustomInventoryCategory(), p) != bPage)
							return false;
#else
						BYTE p = bCell + (5 * j);
					
						if (p >= INVENTORY_MAX_NUM)
							return false;

						if (p / (INVENTORY_MAX_NUM / INVENTORY_PAGE_COUNT) != bPage)
							return false;
#endif

						if (m_pointsInstant.bItemGrid[p])
							if (m_pointsInstant.bItemGrid[p] != iExceptionCell)
								return false;
					}
					while (++j < bSize);
					return true;
				} else
					return false;
			}
			if (1 == bSize)
				return true;
			else {
				int j = 1;
#ifdef ENABLE_CUSTOM_INVENTORY
				BYTE bPage = GetInventoryPageByPos(Cell.GetCustomInventoryCategory(), bCell);
#else
				BYTE bPage = bCell / (INVENTORY_PAGE_SIZE);
#endif

				do {
#ifdef ENABLE_CUSTOM_INVENTORY
					WORD p = bCell + (5 * j);
						
					if (p >= wCategoryEndIndex)
						return false;

					if (GetInventoryPageByPos(Cell.GetCustomInventoryCategory(), p) != bPage)
						return false;
#else
					BYTE p = bCell + (5 * j);

					if (p >= Inventory_Size())
						return false;
					if (p / (INVENTORY_PAGE_SIZE) != bPage)
						return false;
#endif
					if (m_pointsInstant.bItemGrid[p])
						if (m_pointsInstant.bItemGrid[p] != iExceptionCell)
							return false;
				} while (++j < bSize);
				return true;
			}
		} break;
#else
	switch (Cell.window_type)
	{
	case INVENTORY:
		{
			UINT bCell = Cell.cell;
			++iExceptionCell;

			if (Cell.IsBeltInventoryPosition())
			{
				LPITEM beltItem = GetWear(WEAR_BELT);

				if (NULL == beltItem)
					return false;

				if (false == CBeltInventoryHelper::IsAvailableCell(bCell - BELT_INVENTORY_SLOT_START, beltItem->GetValue(0)))
					return false;

				if (m_pointsInstant.bItemGrid[bCell])
				{
					if (m_pointsInstant.bItemGrid[bCell] == iExceptionCell)
						return true;

					return false;
				}

				if (bSize == 1)
					return true;

			}
		
			else if (bCell >= INVENTORY_MAX_NUM)
				return false;

			if (m_pointsInstant.bItemGrid[bCell])
			{
				if (m_pointsInstant.bItemGrid[bCell] == iExceptionCell)
				{
					if (bSize == 1)
						return true;

					int j = 1;
					BYTE bPage = bCell / (INVENTORY_MAX_NUM / INVENTORY_PAGE_COUNT);

					do
					{
						BYTE p = bCell + (5 * j);

						if (p >= INVENTORY_MAX_NUM)
							return false;

						if (p / (INVENTORY_MAX_NUM / INVENTORY_PAGE_COUNT) != bPage)
							return false;

						if (m_pointsInstant.bItemGrid[p])
							if (m_pointsInstant.bItemGrid[p] != iExceptionCell)
								return false;
					}
					while (++j < bSize);

					return true;
				}
				else
					return false;
			}

			if (1 == bSize)
				return true;
			else
			{
				int j = 1;
				BYTE bPage = bCell / (INVENTORY_MAX_NUM / INVENTORY_PAGE_COUNT);

				do
				{
					BYTE p = bCell + (5 * j);

					if (p >= INVENTORY_MAX_NUM)
						return false;

					if (p / (INVENTORY_MAX_NUM / INVENTORY_PAGE_COUNT) != bPage)
						return false;

					if (m_pointsInstant.bItemGrid[p])
						if (m_pointsInstant.bItemGrid[p] != iExceptionCell)
							return false;
				}
				while (++j < bSize);

				return true;
			}
		}
		break;
#endif
	case DRAGON_SOUL_INVENTORY:
		{
			WORD wCell = Cell.cell;
			if (wCell >= DRAGON_SOUL_INVENTORY_MAX_NUM)
				return false;

			// bItemCell�� 0�� false���� ��Ÿ���� ���� + 1 �ؼ� ó���Ѵ�.
			// ���� iExceptionCell�� 1�� ���� ���Ѵ�.
			iExceptionCell++;

			if (m_pointsInstant.wDSItemGrid[wCell])
			{
				if (m_pointsInstant.wDSItemGrid[wCell] == iExceptionCell)
				{
					if (bSize == 1)
						return true;

					int j = 1;

					do
					{
						int p = wCell + (DRAGON_SOUL_BOX_COLUMN_NUM * j);

						if (p >= DRAGON_SOUL_INVENTORY_MAX_NUM)
							return false;

						if (m_pointsInstant.wDSItemGrid[p])
							if (m_pointsInstant.wDSItemGrid[p] != iExceptionCell)
								return false;
					}
					while (++j < bSize);

					return true;
				}
				else
					return false;
			}
			if (1 == bSize)
				return true;
			else
			{
				int j = 1;

				do
				{
					int p = wCell + (DRAGON_SOUL_BOX_COLUMN_NUM * j);

					if (p >= DRAGON_SOUL_INVENTORY_MAX_NUM)
						return false;

					if (m_pointsInstant.bItemGrid[p])
						if (m_pointsInstant.wDSItemGrid[p] != iExceptionCell)
							return false;
				}
				while (++j < bSize);

				return true;
			}
		}
		
		
#ifdef ENABLE_SWITCHBOT
	case SWITCHBOT:
		{
		WORD wCell = Cell.cell;
		if (wCell >= SWITCHBOT_SLOT_COUNT)
		{
			return false;
		}

		if (m_pointsInstant.pSwitchbotItems[wCell])
		{
			return false;
		}

		return true;
		}
#endif
		
	}
	return false;
}

bool CHARACTER::IsEmptyItemGridSpecial(const TItemPos &Cell, BYTE bSize, int iExceptionCell, std::vector<WORD>& vec) const
{

    if (std::find(vec.begin(), vec.end(), Cell.cell) != vec.end()) {
        return false;
    }

    switch (Cell.window_type)
    {
    case INVENTORY:
    {
        WORD bCell = (WORD)Cell.cell;
        ++iExceptionCell;

        if (Cell.IsBeltInventoryPosition())
        {
            LPITEM beltItem = GetWear(WEAR_BELT);

            if (NULL == beltItem)
                return false;

            if (false == CBeltInventoryHelper::IsAvailableCell(bCell - BELT_INVENTORY_SLOT_START, beltItem->GetValue(0)))
                return false;

            if (m_pointsInstant.bItemGrid[bCell])
            {
                if (m_pointsInstant.bItemGrid[bCell] == iExceptionCell)
                    return true;

                return false;
            }

            if (bSize == 1)
                return true;

        }
        else if (bCell >= INVENTORY_MAX_NUM)
            return false;

        if (m_pointsInstant.bItemGrid[bCell])
        {
            if (m_pointsInstant.bItemGrid[bCell] == iExceptionCell)
            {
                if (bSize == 1)
                    return true;

                int j = 1;
                WORD bPage = bCell / (45);

                do
                {
                    WORD p = bCell + (5 * j);

                    if (p >= INVENTORY_MAX_NUM)
                        return false;

                    if (p / (45) != bPage)
                        return false;

                    if (m_pointsInstant.bItemGrid[p])
                        if (m_pointsInstant.bItemGrid[p] != iExceptionCell)
                            return false;
                } while (++j < bSize);

                return true;
            }
            else
                return false;
        }

        // A�ϡ�a�Ƣ� 1AI��e CNAA�� A��AoCI��A ��IAI��C��I ������E ����AI
        if (1 == bSize)
            return true;
        else
        {
            int j = 1;
            WORD bPage = bCell / (45);

            do
            {
                WORD p = bCell + (5 * j);

                if (p >= INVENTORY_MAX_NUM)
                    return false;

                if (p / (45) != bPage)
                    return false;

                if (m_pointsInstant.bItemGrid[p])
                    if (m_pointsInstant.bItemGrid[p] != iExceptionCell)
                        return false;
            } while (++j < bSize);

            return true;
        }
    }
    break;
    case DRAGON_SOUL_INVENTORY:
    {
        WORD wCell = Cell.cell;
        if (wCell >= DRAGON_SOUL_INVENTORY_MAX_NUM)
            return false;

        // bItemCellA�� 0AI falseAOA�� ����A��������a A��C�� + 1 C���� A������CN��U.
        // ��u��o�� iExceptionCell���� 1A�� ��oC�� ��n����CN��U.
        iExceptionCell++;

        if (m_pointsInstant.wDSItemGrid[wCell])
        {
            if (m_pointsInstant.wDSItemGrid[wCell] == iExceptionCell)
            {
                if (bSize == 1)
                    return true;

                int j = 1;

                do
                {
                    WORD p = wCell + (DRAGON_SOUL_BOX_COLUMN_NUM * j);

                    if (p >= DRAGON_SOUL_INVENTORY_MAX_NUM)
                        return false;

                    if (m_pointsInstant.wDSItemGrid[p])
                        if (m_pointsInstant.wDSItemGrid[p] != iExceptionCell)
                            return false;
                } while (++j < bSize);

                return true;
            }
            else
                return false;
        }

        // A�ϡ�a�Ƣ� 1AI��e CNAA�� A��AoCI��A ��IAI��C��I ������E ����AI
        if (1 == bSize)
            return true;
        else
        {
            int j = 1;

            do
            {
                WORD p = wCell + (DRAGON_SOUL_BOX_COLUMN_NUM * j);

                if (p >= DRAGON_SOUL_INVENTORY_MAX_NUM)
                    return false;

                if (m_pointsInstant.bItemGrid[p])
                    if (m_pointsInstant.wDSItemGrid[p] != iExceptionCell)
                        return false;
            } while (++j < bSize);

            return true;
        }
    }
    break;
    }
    return false;
}



#ifdef ENABLE_CUSTOM_INVENTORY
int CHARACTER::GetEmptyInventory(LPITEM pItem, BYTE bSearchInventory) const
{
	if (NULL == pItem)
		return -1;
	
	BYTE bSize = pItem->GetSize();
	
	if(bSearchInventory != 1)
	{
		int iEmptyPos = -1;
		bool bFoundCategory = false;
		
		for(int catIndex = 0; catIndex < CUSTOM_INVENTORY_MAX_NUM; catIndex++)
		{
			if(pItem->IsCustomCategory(catIndex) && !bFoundCategory)
			{
				iEmptyPos = GetEmptyCustomInventory(catIndex, pItem->GetSize());
				if(iEmptyPos != -1)
					bFoundCategory = true;
			}
		}
		
		if(bFoundCategory)
			return iEmptyPos;
	}
	
	if(bSearchInventory != 2)
	{
		for (int i = 0; i < INVENTORY_MAX_NUM; ++i)
			if (IsEmptyItemGrid(TItemPos (INVENTORY, i), bSize))
				return i;
	}
		
	return -1;
}
#endif

int CHARACTER::GetEmptyInventoryEx(LPITEM item)
{
	if (!item)
		return -1;

	int cell = -1;
	if (item->IsDragonSoul())
		cell = GetEmptyDragonSoulInventory(item);
 	else
#ifdef ENABLE_CUSTOM_INVENTORY
		cell = GetEmptyInventory(item);
#else		
		cell = GetEmptyInventory(item->GetSize());

#endif

	return cell;
}

int CHARACTER::GetEmptyInventory(BYTE size) const
{
#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	for ( int i = 0; i < Inventory_Size(); ++i)
#else
	for ( int i = 0; i < INVENTORY_MAX_NUM; ++i)	
#endif
		if (IsEmptyItemGrid(TItemPos(INVENTORY, i), size))
			return i;
	return -1;
}

int CHARACTER::GetEmptyDragonSoulInventory(LPITEM pItem) const
{
	if (NULL == pItem || !pItem->IsDragonSoul())
		return -1;
	if (!DragonSoul_IsQualified())
	{
		return -1;
	}
	BYTE bSize = pItem->GetSize();
	WORD wBaseCell = DSManager::instance().GetBasePosition(pItem);

	if (WORD_MAX == wBaseCell)
		return -1;

	for (int i = 0; i < DRAGON_SOUL_BOX_SIZE; ++i)
		if (IsEmptyItemGrid(TItemPos(DRAGON_SOUL_INVENTORY, i + wBaseCell), bSize))
			return i + wBaseCell;

	return -1;
}

int CHARACTER::GetEmptyDragonSoulInventoryWithExceptions(LPITEM pItem, std::vector<WORD>& vec /*= -1*/) const
{
    if (NULL == pItem || !pItem->IsDragonSoul())
        return -1;
    if (!DragonSoul_IsQualified())
    {
        return -1;
    }
    BYTE bSize = pItem->GetSize();
    WORD wBaseCell = DSManager::instance().GetBasePosition(pItem);

    if (WORD_MAX == wBaseCell)
        return -1;

    for (int i = 0; i < DRAGON_SOUL_BOX_SIZE; ++i)
        if (IsEmptyItemGridSpecial(TItemPos(DRAGON_SOUL_INVENTORY, i + wBaseCell), bSize, -1, vec))
            return i + wBaseCell;

    return -1;
}


void CHARACTER::CopyDragonSoulItemGrid(std::vector<WORD>& vDragonSoulItemGrid) const
{
	vDragonSoulItemGrid.resize(DRAGON_SOUL_INVENTORY_MAX_NUM);

	std::copy(m_pointsInstant.wDSItemGrid, m_pointsInstant.wDSItemGrid + DRAGON_SOUL_INVENTORY_MAX_NUM, vDragonSoulItemGrid.begin());
}
/*
int CHARACTER::CountEmptyInventory() const
{
	int	count = 0;

	for (int i = 0; i < INVENTORY_MAX_NUM; ++i)
		if (GetInventoryItem(i))
			count += GetInventoryItem(i)->GetSize();

	return (INVENTORY_MAX_NUM - count);
}*/

int CHARACTER::CountEmptyInventory() const
{
	int	count = 0;
#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	for (int i = 0; i < Inventory_Size(); ++i)
#else
	for (int i = 0; i < INVENTORY_MAX_NUM; ++i)	
#endif
		if (GetInventoryItem(i))
			count += GetInventoryItem(i)->GetSize();

#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	return (Inventory_Size() - count);
#else
	return (INVENTORY_MAX_NUM - count);
#endif
}

void TransformRefineItem(LPITEM pkOldItem, LPITEM pkNewItem)
{
	// ACCESSORY_REFINE
	if (pkOldItem->IsAccessoryForSocket())
	{
		for (int i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
		{
			pkNewItem->SetSocket(i, pkOldItem->GetSocket(i));
		}
		//pkNewItem->StartAccessorySocketExpireEvent();
	}
	// END_OF_ACCESSORY_REFINE
	else
	{
		// ���⼭ �������� �ڵ������� û�� ��
		for (int i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
		{
			if (!pkOldItem->GetSocket(i))
				break;
			else
				pkNewItem->SetSocket(i, 1);
		}

		// ���� ����
		int slot = 0;

		for (int i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
		{
			long socket = pkOldItem->GetSocket(i);

			if (socket > 2 && socket != ITEM_BROKEN_METIN_VNUM)
				pkNewItem->SetSocket(slot++, socket);
		}

	}

	// ���� ������ ����
	pkOldItem->CopyAttributeTo(pkNewItem);
}

void NotifyRefineSuccess(LPCHARACTER ch, LPITEM item, const char* way)
{
	if (NULL != ch && item != NULL)
	{	
		ch->ChatPacket(CHAT_TYPE_COMMAND, "RefineSuceeded");

		LogManager::instance().RefineLog(ch->GetPlayerID(), item->GetName(), item->GetID(), item->GetRefineLevel(), 1, way);
	}
}

void NotifyRefineFail(LPCHARACTER ch, LPITEM item, const char* way, int success = 0)
{
	if (NULL != ch && NULL != item)
	{
		ch->ChatPacket(CHAT_TYPE_COMMAND, "RefineFailed");

		LogManager::instance().RefineLog(ch->GetPlayerID(), item->GetName(), item->GetID(), item->GetRefineLevel(), success, way);
	}
}

void CHARACTER::SetRefineNPC(LPCHARACTER ch)
{
	if ( ch != NULL )
	{
		m_dwRefineNPCVID = ch->GetVID();
	}
	else
	{
		m_dwRefineNPCVID = 0;
	}
}

bool CHARACTER::DoRefine(LPITEM item, bool bMoneyOnly)
{
	
#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	if (IsSecured())
	{
		ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT("|cffd93c3c[Err]:|h|r_Acest_cont_este_securizat!"));
		return false;
	}
#endif
	
	if (!CanHandleItem(true))
	{
		ClearRefineMode();
		return false;
	}

	//���� �ð����� : upgrade_refine_scroll.quest ���� ������ 5���̳��� �Ϲ� ������
	//�����Ҽ� ����
	if (quest::CQuestManager::instance().GetEventFlag("update_refine_time") != 0)
	{
		if (get_global_time() < quest::CQuestManager::instance().GetEventFlag("update_refine_time") + (60 * 5))
		{
			sys_log(0, "can't refine %d %s", GetPlayerID(), GetName());
			return false;
		}
	}

	const TRefineTable * prt = CRefineManager::instance().GetRefineRecipe(item->GetRefineSet());

	if (!prt)
		return false;

	DWORD result_vnum = item->GetRefinedVnum();

	// REFINE_COST
	int cost = ComputeRefineFee(prt->cost);

	int RefineChance = GetQuestFlag("main_quest_lv7.refine_chance");

	if (RefineChance > 0)
	{
		if (!item->CheckItemUseLevel(20) || item->GetType() != ITEM_WEAPON)
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;1003]");
			return false;
		}

		cost = 0;
		SetQuestFlag("main_quest_lv7.refine_chance", RefineChance - 1);
	}
	// END_OF_REFINE_COST

	if (result_vnum == 0)
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;991]");
		return false;
	}

	if (item->GetType() == ITEM_USE && item->GetSubType() == USE_TUNING)
		return false;

	TItemTable * pProto = ITEM_MANAGER::instance().GetTable(item->GetRefinedVnum());

	if (!pProto)
	{
		sys_err("DoRefine NOT GET ITEM PROTO %d", item->GetRefinedVnum());
		ChatPacket(CHAT_TYPE_INFO, "[LS;1002]");
		return false;
	}

	// REFINE_COST
#ifdef ENABLE_REMOVE_LIMIT_GOLD
	if (GetGold() < static_cast<unsigned long long>(cost))
#else
	if (GetGold() < cost)
#endif
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;67]");
		return false;
	}

	if (!bMoneyOnly && !RefineChance)
	{
		for (int i = 0; i < prt->material_count; ++i)
		{
			if (CountSpecifyItem(prt->materials[i].vnum) < prt->materials[i].count)
			{
				if (test_server)
				{
					ChatPacket(CHAT_TYPE_INFO, "Find %d, count %d, require %d", prt->materials[i].vnum, CountSpecifyItem(prt->materials[i].vnum), prt->materials[i].count);
				}
				ChatPacket(CHAT_TYPE_INFO, "[LS;1035]");
				return false;
			}
		}

		for (int i = 0; i < prt->material_count; ++i)
			RemoveSpecifyItem(prt->materials[i].vnum, prt->materials[i].count);
	}

	int prob = number(1, 100);

	if (IsRefineThroughGuild() || bMoneyOnly)
		prob -= 10;

	// END_OF_REFINE_COST
	if (prob <= prt->prob)
	{
		// ����! ��� �������� �������, ���� �Ӽ��� �ٸ� ������ ȹ��
		LPITEM pkNewItem = ITEM_MANAGER::instance().CreateItem(result_vnum, 1, 0, false);

		if (pkNewItem)
		{

			ITEM_MANAGER::CopyAllAttrTo(item, pkNewItem);
			LogManager::instance().ItemLog(this, pkNewItem, "REFINE SUCCESS", pkNewItem->GetName());

#ifdef ENABLE_CUSTOM_INVENTORY
			WORD bCell = item->GetCell();
#else
			BYTE bCell = item->GetCell();

#endif

			// DETAIL_REFINE_LOG
			NotifyRefineSuccess(this, item, IsRefineThroughGuild() ? "GUILD" : "POWER");
			DBManager::instance().SendMoneyLog(MONEY_LOG_REFINE, item->GetVnum(), -cost);
			ITEM_MANAGER::instance().RemoveItem(item, "REMOVE (REFINE SUCCESS)");
			// END_OF_DETAIL_REFINE_LOG

			pkNewItem->AddToCharacter(this, TItemPos(INVENTORY, bCell));
			ITEM_MANAGER::instance().FlushDelayedSave(pkNewItem);

			sys_log(0, "Refine Success %d", cost);
			pkNewItem->AttrLog();

			sys_log(0, "PayPee %d", cost);
			PayRefineFee(cost);
			sys_log(0, "PayPee End %d", cost);
		}
		else
		{
			// DETAIL_REFINE_LOG
			// ������ ������ ���� -> ���� ���з� ����
			sys_err("cannot create item %u", result_vnum);
			NotifyRefineFail(this, item, IsRefineThroughGuild() ? "GUILD" : "POWER");
			// END_OF_DETAIL_REFINE_LOG
		}
	}
	else
	{
		// ����! ��� �������� �����.
		DBManager::instance().SendMoneyLog(MONEY_LOG_REFINE, item->GetVnum(), -cost);
		NotifyRefineFail(this, item, IsRefineThroughGuild() ? "GUILD" : "POWER");
		item->AttrLog();
		ITEM_MANAGER::instance().RemoveItem(item, "REMOVE (REFINE FAIL)");
		PayRefineFee(cost);
	}

	return true;
}

enum enum_RefineScrolls
{
	CHUKBOK_SCROLL = 0,
	HYUNIRON_CHN   = 1, // �߱������� ���
	YONGSIN_SCROLL = 2,
	MUSIN_SCROLL   = 3,
	YAGONG_SCROLL  = 4,
	MEMO_SCROLL	   = 5,
	BDRAGON_SCROLL	= 6,
};

bool CHARACTER::DoRefineWithScroll(LPITEM item)
{
	
	
#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	if (IsSecured())
	{
		ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT("|cffd93c3c[Err]:|h|r_Acest_cont_este_securizat!"));
		return false;
	}
#endif
	
	if (!CanHandleItem(true))
	{
		ClearRefineMode();
		return false;
	}

	ClearRefineMode();

	//���� �ð����� : upgrade_refine_scroll.quest ���� ������ 5���̳��� �Ϲ� ������
	//�����Ҽ� ����
	if (quest::CQuestManager::instance().GetEventFlag("update_refine_time") != 0)
	{
		if (get_global_time() < quest::CQuestManager::instance().GetEventFlag("update_refine_time") + (60 * 5))
		{
			sys_log(0, "can't refine %d %s", GetPlayerID(), GetName());
			return false;
		}
	}

	const TRefineTable * prt = CRefineManager::instance().GetRefineRecipe(item->GetRefineSet());

	if (!prt)
		return false;

	LPITEM pkItemScroll;

	// ������ üũ
	if (m_iRefineAdditionalCell < 0)
		return false;

	pkItemScroll = GetInventoryItem(m_iRefineAdditionalCell);

	if (!pkItemScroll)
		return false;

	if (!(pkItemScroll->GetType() == ITEM_USE && pkItemScroll->GetSubType() == USE_TUNING))
		return false;

	if (pkItemScroll->GetVnum() == item->GetVnum())
		return false;

	DWORD result_vnum = item->GetRefinedVnum();
	DWORD result_fail_vnum = item->GetRefineFromVnum();

	if (result_vnum == 0)
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;991]");
		return false;
	}

	// MUSIN_SCROLL
	if (pkItemScroll->GetValue(0) == MUSIN_SCROLL)
	{
		if (item->GetRefineLevel() >= 4)
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;1056]");
			return false;
		}
	}
	// END_OF_MUSIC_SCROLL

	else if (pkItemScroll->GetValue(0) == MEMO_SCROLL)
	{
		if (item->GetRefineLevel() != pkItemScroll->GetValue(1))
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;162]");
			return false;
		}
	}
	else if (pkItemScroll->GetValue(0) == BDRAGON_SCROLL)
	{
		if (item->GetType() != ITEM_METIN || item->GetRefineLevel() != 4)
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;1056]");
			return false;
		}
	}

	TItemTable * pProto = ITEM_MANAGER::instance().GetTable(item->GetRefinedVnum());

	if (!pProto)
	{
		sys_err("DoRefineWithScroll NOT GET ITEM PROTO %d", item->GetRefinedVnum());
		ChatPacket(CHAT_TYPE_INFO, "[LS;1002]");
		return false;
	}

	if (GetGold() < prt->cost)
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;67]");
		return false;
	}

	for (int i = 0; i < prt->material_count; ++i)
	{
		if (CountSpecifyItem(prt->materials[i].vnum) < prt->materials[i].count)
		{
			if (test_server)
			{
				ChatPacket(CHAT_TYPE_INFO, "Find %d, count %d, require %d", prt->materials[i].vnum, CountSpecifyItem(prt->materials[i].vnum), prt->materials[i].count);
			}
			ChatPacket(CHAT_TYPE_INFO, "[LS;1035]");
			return false;
		}
	}

	for (int i = 0; i < prt->material_count; ++i)
		RemoveSpecifyItem(prt->materials[i].vnum, prt->materials[i].count);

	int prob = number(1, 100);
	int success_prob = prt->prob;
	bool bDestroyWhenFail = false;

	const char* szRefineType = "SCROLL";

	if (pkItemScroll->GetValue(0) == HYUNIRON_CHN ||
		pkItemScroll->GetValue(0) == YONGSIN_SCROLL ||
		pkItemScroll->GetValue(0) == YAGONG_SCROLL) // ��ö, ����� �ູ��, �߰��� ������  ó��
	{
		const char hyuniron_prob[9] = { 100, 75, 65, 55, 45, 40, 35, 25, 20 };
		const char yagong_prob[9] = { 100, 100, 90, 80, 70, 60, 50, 30, 20 };

		if (pkItemScroll->GetValue(0) == YONGSIN_SCROLL)
		{
			success_prob = hyuniron_prob[MINMAX(0, item->GetRefineLevel(), 8)];
		}
		else if (pkItemScroll->GetValue(0) == YAGONG_SCROLL)
		{
			success_prob = yagong_prob[MINMAX(0, item->GetRefineLevel(), 8)];
		}
		else if (pkItemScroll->GetValue(0) == HYUNIRON_CHN) {} // @fixme121
		else
		{
			sys_err("REFINE : Unknown refine scroll item, Value0: %d", pkItemScroll->GetValue(0));
		}

		if (test_server)
		{
			ChatPacket(CHAT_TYPE_INFO, "[Only Test] Success_Prob %d, RefineLevel %d ", success_prob, item->GetRefineLevel());
		}
		if (pkItemScroll->GetValue(0) == HYUNIRON_CHN) // ��ö�� �������� �μ����� �Ѵ�.
			bDestroyWhenFail = true;

		// DETAIL_REFINE_LOG
		if (pkItemScroll->GetValue(0) == HYUNIRON_CHN)
		{
			szRefineType = "HYUNIRON";
		}
		else if (pkItemScroll->GetValue(0) == YONGSIN_SCROLL)
		{
			szRefineType = "GOD_SCROLL";
		}
		else if (pkItemScroll->GetValue(0) == YAGONG_SCROLL)
		{
			szRefineType = "YAGONG_SCROLL";
		}
		// END_OF_DETAIL_REFINE_LOG
	}

	// DETAIL_REFINE_LOG
	if (pkItemScroll->GetValue(0) == MUSIN_SCROLL) // ������ �ູ���� 100% ���� (+4������)
	{
		success_prob = 100;

		szRefineType = "MUSIN_SCROLL";
	}
	// END_OF_DETAIL_REFINE_LOG
	else if (pkItemScroll->GetValue(0) == MEMO_SCROLL)
	{
		success_prob = 100;
		szRefineType = "MEMO_SCROLL";
	}
	else if (pkItemScroll->GetValue(0) == BDRAGON_SCROLL)
	{
		success_prob = 80;
		szRefineType = "BDRAGON_SCROLL";
	}
	pkItemScroll->SetCount(pkItemScroll->GetCount() - 1);

	if (prob <= success_prob)
	{
		// ����! ��� �������� �������, ���� �Ӽ��� �ٸ� ������ ȹ��
		LPITEM pkNewItem = ITEM_MANAGER::instance().CreateItem(result_vnum, 1, 0, false);

		if (pkNewItem)
		{

			ITEM_MANAGER::CopyAllAttrTo(item, pkNewItem);
			LogManager::instance().ItemLog(this, pkNewItem, "REFINE SUCCESS", pkNewItem->GetName());

#ifdef ENABLE_CUSTOM_INVENTORY
			WORD bCell = item->GetCell();
#else
			BYTE bCell = item->GetCell();

#endif

			NotifyRefineSuccess(this, item, szRefineType);
			DBManager::instance().SendMoneyLog(MONEY_LOG_REFINE, item->GetVnum(), -prt->cost);
			ITEM_MANAGER::instance().RemoveItem(item, "REMOVE (REFINE SUCCESS)");

			pkNewItem->AddToCharacter(this, TItemPos(INVENTORY, bCell));
			ITEM_MANAGER::instance().FlushDelayedSave(pkNewItem);
			pkNewItem->AttrLog();
			PayRefineFee(prt->cost);
		}
		else
		{
			// ������ ������ ���� -> ���� ���з� ����
			sys_err("cannot create item %u", result_vnum);
			NotifyRefineFail(this, item, szRefineType);
		}
	}
	else if (!bDestroyWhenFail && result_fail_vnum)
	{
		// ����! ��� �������� �������, ���� �Ӽ��� ���� ����� ������ ȹ��
		LPITEM pkNewItem = ITEM_MANAGER::instance().CreateItem(result_fail_vnum, 1, 0, false);

		if (pkNewItem)
		{
			ITEM_MANAGER::CopyAllAttrTo(item, pkNewItem);
			LogManager::instance().ItemLog(this, pkNewItem, "REFINE FAIL", pkNewItem->GetName());

#ifdef ENABLE_CUSTOM_INVENTORY
			WORD bCell = item->GetCell();
#else
			BYTE bCell = item->GetCell();

#endif

			DBManager::instance().SendMoneyLog(MONEY_LOG_REFINE, item->GetVnum(), -prt->cost);
			NotifyRefineFail(this, item, szRefineType, -1);
			ITEM_MANAGER::instance().RemoveItem(item, "REMOVE (REFINE FAIL)");

			pkNewItem->AddToCharacter(this, TItemPos(INVENTORY, bCell));
			ITEM_MANAGER::instance().FlushDelayedSave(pkNewItem);

			pkNewItem->AttrLog();
			PayRefineFee(prt->cost);
		}
		else
		{
			// ������ ������ ���� -> ���� ���з� ����
			sys_err("cannot create item %u", result_fail_vnum);
			NotifyRefineFail(this, item, szRefineType);
		}
	}
	else
	{
		NotifyRefineFail(this, item, szRefineType); // ������ ������ ������� ����

		PayRefineFee(prt->cost);
	}

	return true;
}

#ifdef ENABLE_CUSTOM_INVENTORY
bool CHARACTER::RefineInformation(WORD bCell, BYTE bType, int iAdditionalCell)
{
	if (bCell >= INVENTORY_MAX_NUM && (bCell < CUSTOM_INVENTORY_SLOT_START || bCell >= CUSTOM_INVENTORY_SLOT_END))
		return false;
#else
bool CHARACTER::RefineInformation(BYTE bCell, BYTE bType, int iAdditionalCell)
{
	if (bCell > INVENTORY_MAX_NUM)
		return false;
#endif

	LPITEM item = GetInventoryItem(bCell);

	if (!item)
		return false;

	// REFINE_COST
	if (bType == REFINE_TYPE_MONEY_ONLY && !GetQuestFlag("deviltower_zone.can_refine"))
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;1067]");
		return false;
	}
	// END_OF_REFINE_COST

	TPacketGCRefineInformation p;

	p.header = HEADER_GC_REFINE_INFORMATION;
	p.pos = bCell;
	p.src_vnum = item->GetVnum();
	p.result_vnum = item->GetRefinedVnum();
	p.type = bType;

	if (p.result_vnum == 0)
	{
		sys_err("RefineInformation p.result_vnum == 0");
		ChatPacket(CHAT_TYPE_INFO, "[LS;1002]");
		return false;
	}

	if (item->GetType() == ITEM_USE && item->GetSubType() == USE_TUNING)
	{
		if (bType == 0)
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;1077]");
			return false;
		}
		else
		{
			LPITEM itemScroll = GetInventoryItem(iAdditionalCell);
			if (!itemScroll || item->GetVnum() == itemScroll->GetVnum())
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;1106]");
				ChatPacket(CHAT_TYPE_INFO, "[LS;1096]");
				return false;
			}
		}
	}

	CRefineManager & rm = CRefineManager::instance();

	const TRefineTable* prt = rm.GetRefineRecipe(item->GetRefineSet());

	if (!prt)
	{
		sys_err("RefineInformation NOT GET REFINE SET %d", item->GetRefineSet());
		ChatPacket(CHAT_TYPE_INFO, "[LS;1002]");
		return false;
	}

	// REFINE_COST

	//MAIN_QUEST_LV7
	if (GetQuestFlag("main_quest_lv7.refine_chance") > 0)
	{
		// �Ϻ��� ����
		if (!item->CheckItemUseLevel(20) || item->GetType() != ITEM_WEAPON)
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;1003]");
			return false;
		}
		p.cost = 0;
	}
	else
		p.cost = ComputeRefineFee(prt->cost);

	//END_MAIN_QUEST_LV7
	p.prob = prt->prob;
	if (bType == REFINE_TYPE_MONEY_ONLY)
	{
		p.material_count = 0;
		memset(p.materials, 0, sizeof(p.materials));
	}
	else
	{
		p.material_count = prt->material_count;
		thecore_memcpy(&p.materials, prt->materials, sizeof(prt->materials));
	}
	// END_OF_REFINE_COST

	GetDesc()->Packet(&p, sizeof(TPacketGCRefineInformation));

	SetRefineMode(iAdditionalCell);
	return true;
}

bool CHARACTER::RefineItem(LPITEM pkItem, LPITEM pkTarget)
{
	if (!CanHandleItem())
		return false;

	if (pkItem->GetSubType() == USE_TUNING)
	{
		// XXX ����, ���� �������� ��������ϴ�...
		// XXX ���ɰ������� �ູ�� ���� �Ǿ���!
		// MUSIN_SCROLL
		if (pkItem->GetValue(0) == MUSIN_SCROLL)
			RefineInformation(pkTarget->GetCell(), REFINE_TYPE_MUSIN, pkItem->GetCell());
		// END_OF_MUSIN_SCROLL
		else if (pkItem->GetValue(0) == HYUNIRON_CHN)
			RefineInformation(pkTarget->GetCell(), REFINE_TYPE_HYUNIRON, pkItem->GetCell());
		else if (pkItem->GetValue(0) == BDRAGON_SCROLL)
		{
			if (pkTarget->GetRefineSet() != 702) return false;
			RefineInformation(pkTarget->GetCell(), REFINE_TYPE_BDRAGON, pkItem->GetCell());
		}
		else
		{
			if (pkTarget->GetRefineSet() == 501) return false;
			RefineInformation(pkTarget->GetCell(), REFINE_TYPE_SCROLL, pkItem->GetCell());
		}
	}
	else if (pkItem->GetSubType() == USE_DETACHMENT && IS_SET(pkTarget->GetFlag(), ITEM_FLAG_REFINEABLE))
	{
		LogManager::instance().ItemLog(this, pkTarget, "USE_DETACHMENT", pkTarget->GetName());

		bool bHasMetinStone = false;

#ifdef ENABLE_EXTENDED_SOCKETS
		for (int i = 0; i < ITEM_STONES_MAX_NUM; i++)
#else
		for (int i = 0; i < ITEM_SOCKET_MAX_NUM; i++)
#endif
		{
			long socket = pkTarget->GetSocket(i);
			if (socket > 2 && socket != ITEM_BROKEN_METIN_VNUM)
			{
				bHasMetinStone = true;
				break;
			}
		}

		if (bHasMetinStone)
		{
#ifdef ENABLE_EXTENDED_SOCKETS
			for (int i = 0; i < ITEM_STONES_MAX_NUM; i++)
#else
			for (int i = 0; i < ITEM_SOCKET_MAX_NUM; i++)
#endif
			{
				long socket = pkTarget->GetSocket(i);
				if (socket > 2 && socket != ITEM_BROKEN_METIN_VNUM)
				{
					AutoGiveItem(socket);
					//TItemTable* pTable = ITEM_MANAGER::instance().GetTable(pkTarget->GetSocket(i));
					//pkTarget->SetSocket(i, pTable->alValues[2]);
					// �������� ��ü���ش�
					pkTarget->SetSocket(i, ITEM_BROKEN_METIN_VNUM);
				}
			}
			pkItem->SetCount(pkItem->GetCount() - 1);
			return true;
		}
		else
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;1108]");
			return false;
		}
	}

	return false;
}

EVENTFUNC(kill_campfire_event)
{
	char_event_info* info = dynamic_cast<char_event_info*>( event->info );

	if ( info == NULL )
	{
		sys_err( "kill_campfire_event> <Factor> Null pointer" );
		return 0;
	}

	LPCHARACTER	ch = info->ch;

	if (ch == NULL) { // <Factor>
		return 0;
	}
	ch->m_pkMiningEvent = NULL;
	M2_DESTROY_CHARACTER(ch);
	return 0;
}

bool CHARACTER::GiveRecallItem(LPITEM item)
{
	int idx = GetMapIndex();
	int iEmpireByMapIndex = -1;

	if (idx < 20)
		iEmpireByMapIndex = 1;
	else if (idx < 40)
		iEmpireByMapIndex = 2;
	else if (idx < 60)
		iEmpireByMapIndex = 3;
	else if (idx < 10000)
		iEmpireByMapIndex = 0;

	switch (idx)
	{
		case 66:
		case 216:
			iEmpireByMapIndex = -1;
			break;
	}

	if (iEmpireByMapIndex && GetEmpire() != iEmpireByMapIndex)
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;1119]");
		return false;
	}

	int pos;

	if (item->GetCount() == 1)	// �������� �ϳ���� �׳� ����.
	{
		item->SetSocket(0, GetX());
		item->SetSocket(1, GetY());
	}
	else if ((pos = GetEmptyInventory(item->GetSize())) != -1) // �׷��� �ʴٸ� �ٸ� �κ��丮 ������ ã�´�.
	{
		LPITEM item2 = ITEM_MANAGER::instance().CreateItem(item->GetVnum(), 1);

		if (NULL != item2)
		{
			item2->SetSocket(0, GetX());
			item2->SetSocket(1, GetY());
			item2->AddToCharacter(this, TItemPos(INVENTORY, pos));

			item->SetCount(item->GetCount() - 1);
		}
	}
	else
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;1130]");
		return false;
	}

	return true;
}

void CHARACTER::ProcessRecallItem(LPITEM item)
{
	int idx;

	if ((idx = SECTREE_MANAGER::instance().GetMapIndex(item->GetSocket(0), item->GetSocket(1))) == 0)
		return;

	int iEmpireByMapIndex = -1;

	if (idx < 20)
		iEmpireByMapIndex = 1;
	else if (idx < 40)
		iEmpireByMapIndex = 2;
	else if (idx < 60)
		iEmpireByMapIndex = 3;
	else if (idx < 10000)
		iEmpireByMapIndex = 0;

	switch (idx)
	{
		case 66:
		case 216:
			iEmpireByMapIndex = -1;
			break;
		// �Ƿ決�� �϶�
		case 301:
		case 302:
		case 303:
		case 304:
			if( GetLevel() < 90 )
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;1013]");
				return;
			}
			else
				break;
	}

	if (iEmpireByMapIndex && GetEmpire() != iEmpireByMapIndex)
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;1141]");
		item->SetSocket(0, 0);
		item->SetSocket(1, 0);
	}
	else
	{
		sys_log(1, "Recall: %s %d %d -> %d %d", GetName(), GetX(), GetY(), item->GetSocket(0), item->GetSocket(1));
		WarpSet(item->GetSocket(0), item->GetSocket(1));
		item->SetCount(item->GetCount() - 1);
	}
}

void CHARACTER::__OpenPrivateShop()
{
#ifdef ENABLE_OPEN_SHOP_WITH_ARMOR
	ChatPacket(CHAT_TYPE_COMMAND, "OpenPrivateShop");
#else
	unsigned bodyPart = GetPart(PART_MAIN);
	switch (bodyPart)
	{
        case 0:
        case 1:
        case 2:
            if (GetGMLevel() > GM_PLAYER)
            {
                ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[i]Un GameMaster nu poate deschide un shop-offline."));
            }
            else
            {
                ChatPacket(CHAT_TYPE_COMMAND, "OpenPrivateShop");
            }
            break;
        default:
            ChatPacket(CHAT_TYPE_INFO, LC_TEXT("UNKNOWN_STRING_OPENPRIVATESHOP"));
            break;
	}
#endif
}

// MYSHOP_PRICE_LIST
void CHARACTER::SendMyShopPriceListCmd(DWORD dwItemVnum, long long dwItemPrice)
{
	char szLine[256];
	snprintf(szLine, sizeof(szLine), "MyShopPriceList %u %lld", dwItemVnum, dwItemPrice);
	ChatPacket(CHAT_TYPE_COMMAND, szLine);
	sys_log(0, szLine);
}

//
// DB ĳ�÷� ���� ���� ����Ʈ�� User ���� �����ϰ� ������ ����� Ŀ�ǵ带 ������.
//
void CHARACTER::UseSilkBotaryReal(const TPacketMyshopPricelistHeader* p)
{
	const TItemPriceInfo* pInfo = (const TItemPriceInfo*)(p + 1);

	if (!p->byCount)
		// ���� ����Ʈ�� ����. dummy �����͸� ���� Ŀ�ǵ带 �����ش�.
		SendMyShopPriceListCmd(1, 0);
	else {
		for (int idx = 0; idx < p->byCount; idx++)
			SendMyShopPriceListCmd(pInfo[ idx ].dwVnum, pInfo[ idx ].dwPrice);
	}

	__OpenPrivateShop();
}

#ifdef ENABLE_AFFECT_RENEWAL
bool CHARACTER::SetBlendAffect(LPITEM item)
{
	switch (item->GetVnum())
	{
		// DEWS
	case 50821: // Roua Rosie
	case 20210:
		AddAffect(AFFECT_BLEND_POTION_1, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;

	case 50822:
	case 20209:
		AddAffect(AFFECT_BLEND_POTION_2, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;

	case 50823:
	case 20214:
		AddAffect(AFFECT_BLEND_POTION_3, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;

	case 50824:
	case 20213:
		AddAffect(AFFECT_BLEND_POTION_4, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;

	case 50825:
	case 20211:
		AddAffect(AFFECT_BLEND_POTION_3, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;

	case 50826:
	case 20212:
		AddAffect(AFFECT_BLEND_POTION_6, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;
	
	case 20215:
		AddAffect(AFFECT_BLEND_POTION_7, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;
		// END_OF_DEWS

		// ENERGY_CRISTAL
	case 51002:
	case 20216:
		AddAffect(AFFECT_ENERGY, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;
		// END_OF_ENERGY_CRISTAL

		// DRAGON_GOD_MEDALS
	case 20205:
		AddAffect(AFFECT_DRAGON_GOD_1, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;
	case 20206:
		AddAffect(AFFECT_DRAGON_GOD_2, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;
	case 20207:
		AddAffect(AFFECT_DRAGON_GOD_3, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;
	case 20208:
		AddAffect(AFFECT_DRAGON_GOD_4, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;
		// END_OF_DRAGON_GOD_MEDALS

		// CRITICAL_AND_PENETRATION
	case 20203:
		AddAffect(AFFECT_CRITICAL, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;
	case 20204:
		AddAffect(AFFECT_PENETRATE, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;
		// END_OF_CRITICAL_AND_PENETRATION

		// ATTACK_AND_MOVE_SPEED
	case 20202:
		AddAffect(AFFECT_ATTACK_SPEED, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;
	case 20201:
		AddAffect(AFFECT_MOVE_SPEED, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;
	case 20217:
		AddAffect(AFFECT_WIND_SHOES, APPLY_NONE, 0, AFF_NONE, item->GetSocket(2), 0, false, false);
		break;
		// END_OF_ATTACK_AND_MOVE_SPEED

	default:
		return false;
	}

	return true;
}

bool CHARACTER::UseExtendedBlendAffect(LPITEM item, int affect_type, int apply_type, int apply_value, int apply_duration)
{
	apply_duration = apply_duration <= 0 ? INFINITE_AFFECT_DURATION : apply_duration;
	bool bStatus = item->GetSocket(3);

	if (FindAffect(affect_type, apply_type))
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
		return false;
	}

	if (FindAffect(AFFECT_EXP_BONUS_EURO_FREE, apply_type) || FindAffect(AFFECT_MALL, apply_type))
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
		return false;
	}

	switch (item->GetVnum())
	{
		// DEWS
	case 50821: // Roua Rosie
	case 20210: // Roua Rosie(P)
	{
		if (FindAffect(AFFECT_BLEND_POTION_1))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_BLEND_POTION_1);
			return false;
		}
		AddAffect(AFFECT_BLEND_POTION_1, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	case 50822: // Roua Portocalie
	case 20209: // Roua Portocalie(P)
	{
		if (FindAffect(AFFECT_BLEND_POTION_2))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_BLEND_POTION_2);
			return false;
		}
		AddAffect(AFFECT_BLEND_POTION_2, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	case 50823: // Roua Galbena
	case 20214: // Roua Galbena(P)
	{
		if (FindAffect(AFFECT_BLEND_POTION_3))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_BLEND_POTION_3);
			return false;
		}
		AddAffect(AFFECT_BLEND_POTION_3, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	case 50824: // Roua Verde
	case 20213: // Roua Verde(P)
	{
		if (FindAffect(AFFECT_BLEND_POTION_4))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_BLEND_POTION_4);
			return false;
		}
		if (FindAffect(AFFECT_EXP_BONUS_EURO_FREE, POINT_RESIST_MAGIC))
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
			return false;
		}

		AddAffect(AFFECT_BLEND_POTION_4, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	case 50825: // Roua Albastra
	case 20211: // Roua Albastra(P)
	{
		if (FindAffect(AFFECT_BLEND_POTION_5))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_BLEND_POTION_5);
			return false;
		}
		AddAffect(AFFECT_BLEND_POTION_5, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	case 50826: // Roua Alba
	case 20212: // Roua Alba(P)
	{
		if (FindAffect(AFFECT_BLEND_POTION_6))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_BLEND_POTION_6);
			return false;
		}
		AddAffect(AFFECT_BLEND_POTION_6, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	
	case 20215: // Roua Neagra
	{
		if (FindAffect(AFFECT_BLEND_POTION_7))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_BLEND_POTION_7);
			return false;
		}

		AddAffect(AFFECT_BLEND_POTION_7, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	
	// END_OF_DEWS

	// ENERGY_CRISTAL
	case 51002: // Cristal Energie
	case 20216: // Cristal Energie(P)
	{
		if (FindAffect(AFFECT_ENERGY))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_ENERGY);
			return false;
		}
		AddAffect(AFFECT_ENERGY, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	// END_OF_ENERGY_CRISTAL

	// DRAGON_GOD_MEDALS
	case 20205: // Viata Zeului Dragon(P)
	{
		if (FindAffect(AFFECT_DRAGON_GOD_1))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_DRAGON_GOD_1);
			return false;
		}

		AddAffect(AFFECT_DRAGON_GOD_1, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	
	case 20206: // Atacul Zeului Dragon(P)
	{
		if (FindAffect(AFFECT_DRAGON_GOD_2))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_DRAGON_GOD_2);
			return false;
		}
		AddAffect(AFFECT_DRAGON_GOD_2, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	case 20207: // Inteligenta zeului dragon (P)
	{
		if (FindAffect(AFFECT_DRAGON_GOD_3))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_DRAGON_GOD_3);
			return false;
		}
		AddAffect(AFFECT_DRAGON_GOD_3, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	case 20208: // Apararea zeului dragon (P)
	{
		if (FindAffect(AFFECT_DRAGON_GOD_4))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_DRAGON_GOD_4);
			return false;
		}
		AddAffect(AFFECT_DRAGON_GOD_4, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	// END_OF_DRAGON_GOD_MEDALS

	// CRITICAL_AND_PENETRATION
	case 20203: // Lovitura Critica (P)
	{
		if (FindAffect(AFFECT_CRITICAL))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_CRITICAL);
			return false;
		}
		AddAffect(AFFECT_CRITICAL, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	case 20204: // Lovitura Patrunzatoare(P)
	{
		if (FindAffect(AFFECT_PENETRATE))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_PENETRATE);
			return false;
		}
		AddAffect(AFFECT_PENETRATE, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	// END_OF_CRITICAL_AND_PENETRATION

	// ATTACK_AND_MOVE_SPEED
	case 20202: // Potiune Verde(P)
	{
		if (FindAffect(AFFECT_ATTACK_SPEED))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_ATTACK_SPEED);
			return false;
		}

		if (FindAffect(AFFECT_ATT_SPEED))
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
			return false;
		}

		AddAffect(AFFECT_ATTACK_SPEED, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	case 20201: // Potiune Mov(P)
	{
		if (FindAffect(AFFECT_MOVE_SPEED))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_MOVE_SPEED);
			return false;
		}

		if (FindAffect(AFFECT_MOV_SPEED))
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
			return false;
		}

		AddAffect(AFFECT_MOVE_SPEED, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	
	case 20217: // Wind Shoes (P)
	{
		if (FindAffect(AFFECT_WIND_SHOES))
		{
			if (!bStatus)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				return false;
			}
			RemoveAffect(AFFECT_WIND_SHOES);
			return false;
		}

		if (FindAffect(AFFECT_MOV_SPEED))
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
			return false;
		}

		AddAffect(AFFECT_WIND_SHOES, apply_type, apply_value, 0, apply_duration, 0, true);
	}
	break;
	// END_OF_ATTACK_AND_MOVE_SPEED
	}

	return true;
}
#endif

void CHARACTER::UseSilkBotary(void)
{
	if (m_bNoOpenedShop) {
		DWORD dwPlayerID = GetPlayerID();
		db_clientdesc->DBPacket(HEADER_GD_MYSHOP_PRICELIST_REQ, GetDesc()->GetHandle(), &dwPlayerID, sizeof(DWORD));
		m_bNoOpenedShop = false;
	} else {
		__OpenPrivateShop();
	}
}
// END_OF_MYSHOP_PRICE_LIST

int CalculateConsume(LPCHARACTER ch)
{
	static const int WARP_NEED_LIFE_PERCENT	= 30;
	static const int WARP_MIN_LIFE_PERCENT	= 10;
	// CONSUME_LIFE_WHEN_USE_WARP_ITEM
	int consumeLife = 0;
	{
		// CheckNeedLifeForWarp
		const int curLife		= ch->GetHP();
		const int needPercent	= WARP_NEED_LIFE_PERCENT;
		const int needLife = ch->GetMaxHP() * needPercent / 100;
		if (curLife < needLife)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1152]");
			return -1;
		}

		consumeLife = needLife;


		// CheckMinLifeForWarp: ���� ���ؼ� ������ �ȵǹǷ� ������ �ּҷ��� �����ش�
		const int minPercent	= WARP_MIN_LIFE_PERCENT;
		const int minLife	= ch->GetMaxHP() * minPercent / 100;
		if (curLife - needLife < minLife)
			consumeLife = curLife - minLife;

		if (consumeLife < 0)
			consumeLife = 0;
	}
	// END_OF_CONSUME_LIFE_WHEN_USE_WARP_ITEM
	return consumeLife;
}

int CalculateConsumeSP(LPCHARACTER lpChar)
{
	static const int NEED_WARP_SP_PERCENT = 30;

	const int curSP = lpChar->GetSP();
	const int needSP = lpChar->GetMaxSP() * NEED_WARP_SP_PERCENT / 100;

	if (curSP < needSP)
	{
		lpChar->ChatPacket(CHAT_TYPE_INFO, "[LS;1162]");
		return -1;
	}

	return needSP;
}

// #define ENABLE_FIREWORK_STUN
#define ENABLE_ADDSTONE_FAILURE
bool CHARACTER::UseItemEx(LPITEM item, TItemPos DestCell)
{
	int iLimitRealtimeStartFirstUseFlagIndex = -1;
	//int iLimitTimerBasedOnWearFlagIndex = -1;

	WORD wDestCell = DestCell.cell;
	BYTE bDestInven = DestCell.window_type;
	for (int i = 0; i < ITEM_LIMIT_MAX_NUM; ++i)
	{
		long limitValue = item->GetProto()->aLimits[i].lValue;

		switch (item->GetProto()->aLimits[i].bType)
		{
			case LIMIT_LEVEL:
				if (GetLevel() < limitValue)
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;1013]");
					return false;
				}
				break;

#if defined(__CONQUEROR_LEVEL__)
			case LIMIT_CHAMPION:
				if (GetConquerorLevel() < limitValue)
				{
					ChatPacket(CHAT_TYPE_INFO, "Nivelul tau campion este prea mic pentru a putea purta acest item!");
					return false;
				}
				break;
#endif

			case LIMIT_REAL_TIME_START_FIRST_USE:
				iLimitRealtimeStartFirstUseFlagIndex = i;
				break;

			case LIMIT_TIMER_BASED_ON_WEAR:
				//iLimitTimerBasedOnWearFlagIndex = i;
				break;
		}
	}

	if (test_server)
	{
		sys_log(0, "USE_ITEM %s, Inven %d, Cell %d, ItemType %d, SubType %d", item->GetName(), bDestInven, wDestCell, item->GetType(), item->GetSubType());
	}

	if ( CArenaManager::instance().IsLimitedItem( GetMapIndex(), item->GetVnum() ) == true )
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
		return false;
	}
#ifdef ENABLE_NEWSTUFF
	else if (g_NoPotionsOnPVP && CPVPManager::instance().IsFighting(GetPlayerID()) && IsLimitedPotionOnPVP(item->GetVnum()))
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
		return false;
	}
#endif

	// @fixme402 (IsLoadedAffect to block affect hacking)
	if (!IsLoadedAffect())
	{
		ChatPacket(CHAT_TYPE_INFO, "Affects are not loaded yet!");
		return false;
	}

	// @fixme141 BEGIN
	if (TItemPos(item->GetWindow(), item->GetCell()).IsBeltInventoryPosition())
	{
		LPITEM beltItem = GetWear(WEAR_BELT);

		if (NULL == beltItem)
		{
			ChatPacket(CHAT_TYPE_INFO, "<Belt> You can't use this item if you have no equipped belt");
			return false;
		}

		if (false == CBeltInventoryHelper::IsAvailableCell(item->GetCell() - BELT_INVENTORY_SLOT_START, beltItem->GetValue(0)))
		{
			ChatPacket(CHAT_TYPE_INFO, "<Belt> You can't use this item if you don't upgrade your belt");
			return false;
		}
	}
	// @fixme141 END

	// ������ ���� ��� ���ĺ��ʹ� ������� �ʾƵ� �ð��� �����Ǵ� ��� ó��.
	if (-1 != iLimitRealtimeStartFirstUseFlagIndex)
	{
		// �� ���̶� ����� ���������� ���δ� Socket1�� ���� �Ǵ��Ѵ�. (Socket1�� ���Ƚ�� ���)
		if (0 == item->GetSocket(1))
		{
			// ��밡�ɽð��� Default ������ Limit Value ���� ����ϵ�, Socket0�� ���� ������ �� ���� ����ϵ��� �Ѵ�. (������ ��)
			long duration = (0 != item->GetSocket(0)) ? item->GetSocket(0) : item->GetProto()->aLimits[iLimitRealtimeStartFirstUseFlagIndex].lValue;

			if (0 == duration)
				duration = 60 * 60 * 24 * 7;

			item->SetSocket(0, time(0) + duration);
			item->StartRealTimeExpireEvent();
		}

		if (false == item->IsEquipped())
			item->SetSocket(1, item->GetSocket(1) + 1);
	}

#ifdef ENABLE_BATTLE_PASS
	if (item->GetType() != ITEM_BLEND)
		CBattlePass::Instance().RegisterItemMission(MISSION_TYPE_USE_ITEM, 1, this, item);

	if (item->GetVnum() == 40004)
	{
		SetPremiumBattlePass();
		return true;
	}
#endif

	switch (item->GetType())
	{
		case ITEM_HAIR:
			return ItemProcess_Hair(item, wDestCell);

		case ITEM_POLYMORPH:
			return ItemProcess_Polymorph(item);

		case ITEM_QUEST:
			if (GetArena() != NULL || IsObserverMode() == true)
			{
				if (item->GetVnum() == 50051 || item->GetVnum() == 50052 || item->GetVnum() == 50053)
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
					return false;
				}
			}

#ifdef RENEWAL_PICKUP_AFFECT
		if (item->GetVnum() == 70002)
		{
			CAffect* affect = FindAffect(AFFECT_PICKUP_ENABLE);
			if (!affect)
			{
				affect = FindAffect(AFFECT_PICKUP_DEACTIVE);
				if (affect)
				{
					ChatPacket(CHAT_TYPE_INFO, "@@(char_item.cpp)tradus: Pick-up Already Active.");
					return false;
				}
			}
			else
			{
				ChatPacket(CHAT_TYPE_INFO, "@@(char_item.cpp)tradus:Deja ai auto-pick-up-ul.");
				return false;
			}
			AddAffect(AFFECT_PICKUP_ENABLE, POINT_NONE, 0, AFF_NONE, 60 * 60 * 7, 0, 0, false);
			ChatPacket(CHAT_TYPE_INFO, "@@(char_item.cpp)tradus:Auto Pick-up-ul a fost activat cu succes!");
			return true;
		}
#endif



			if (!IS_SET(item->GetFlag(), ITEM_FLAG_QUEST_USE | ITEM_FLAG_QUEST_USE_MULTIPLE))
			{
				if (item->GetSIGVnum() == 0)
				{
					quest::CQuestManager::instance().UseItem(GetPlayerID(), item, false);
				}
				else
				{
					quest::CQuestManager::instance().SIGUse(GetPlayerID(), item->GetSIGVnum(), item, false);
				}
			}
			break;

		case ITEM_CAMPFIRE:
			{
#ifdef __FIX_CAMPFIRE__
				if (GetMapIndex() == 113)
				{
					ChatPacket(CHAT_TYPE_INFO, "Forbidden");
					return false;
				}
		
				int son_ates = GetQuestFlag("kamp.ates");
				if (get_global_time() - son_ates < __FIX_CAMPFIRE__SEC)
				{
					ChatPacket(CHAT_TYPE_INFO, "Once at 60 seconds you can use the fire");
					return false;
				}
		
		
				SetQuestFlag("kamp.ates", get_global_time());
#endif
				float fx, fy;
				GetDeltaByDegree(GetRotation(), 100.0f, &fx, &fy);

				LPSECTREE tree = SECTREE_MANAGER::instance().Get(GetMapIndex(), (long)(GetX()+fx), (long)(GetY()+fy));

				if (!tree)
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;1217]");
					return false;
				}

				if (tree->IsAttr((long)(GetX()+fx), (long)(GetY()+fy), ATTR_WATER))
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;1228]");
					return false;
				}

				LPCHARACTER campfire = CHARACTER_MANAGER::instance().SpawnMob(fishing::CAMPFIRE_MOB, GetMapIndex(), (long)(GetX()+fx), (long)(GetY()+fy), 0, false, number(0, 359));

				char_event_info* info = AllocEventInfo<char_event_info>();

				info->ch = campfire;

				campfire->m_pkMiningEvent = event_create(kill_campfire_event, info, PASSES_PER_SEC(40));

				item->SetCount(item->GetCount() - 1);
			}
			break;

		case ITEM_UNIQUE:
			{
				switch (item->GetSubType())
				{
					case USE_ABILITY_UP:
						{
							switch (item->GetValue(0))
							{
								case APPLY_MOV_SPEED:
									AddAffect(AFFECT_UNIQUE_ABILITY, POINT_MOV_SPEED, item->GetValue(2), AFF_MOV_SPEED_POTION, item->GetValue(1), 0, true, true);
									break;

								case APPLY_ATT_SPEED:
									AddAffect(AFFECT_UNIQUE_ABILITY, POINT_ATT_SPEED, item->GetValue(2), AFF_ATT_SPEED_POTION, item->GetValue(1), 0, true, true);
									break;

								case APPLY_STR:
									AddAffect(AFFECT_UNIQUE_ABILITY, POINT_ST, item->GetValue(2), 0, item->GetValue(1), 0, true, true);
									break;

								case APPLY_DEX:
									AddAffect(AFFECT_UNIQUE_ABILITY, POINT_DX, item->GetValue(2), 0, item->GetValue(1), 0, true, true);
									break;

								case APPLY_CON:
									AddAffect(AFFECT_UNIQUE_ABILITY, POINT_HT, item->GetValue(2), 0, item->GetValue(1), 0, true, true);
									break;

								case APPLY_INT:
									AddAffect(AFFECT_UNIQUE_ABILITY, POINT_IQ, item->GetValue(2), 0, item->GetValue(1), 0, true, true);
									break;

								case APPLY_CAST_SPEED:
									AddAffect(AFFECT_UNIQUE_ABILITY, POINT_CASTING_SPEED, item->GetValue(2), 0, item->GetValue(1), 0, true, true);
									break;

								case APPLY_RESIST_MAGIC:
									AddAffect(AFFECT_UNIQUE_ABILITY, POINT_RESIST_MAGIC, item->GetValue(2), 0, item->GetValue(1), 0, true, true);
									break;

								case APPLY_ATT_GRADE_BONUS:
									AddAffect(AFFECT_UNIQUE_ABILITY, POINT_ATT_GRADE_BONUS,
											item->GetValue(2), 0, item->GetValue(1), 0, true, true);
									break;

								case APPLY_DEF_GRADE_BONUS:
									AddAffect(AFFECT_UNIQUE_ABILITY, POINT_DEF_GRADE_BONUS,
											item->GetValue(2), 0, item->GetValue(1), 0, true, true);
									break;
							}
						}

						if (GetDungeon())
							GetDungeon()->UsePotion(this);

						if (GetWarMap())
							GetWarMap()->UsePotion(this, item);

						item->SetCount(item->GetCount() - 1);
						break;

					default:
						{
							if (item->GetSubType() == USE_SPECIAL)
							{
								sys_log(0, "ITEM_UNIQUE: USE_SPECIAL %u", item->GetVnum());

								switch (item->GetVnum())
								{
									case 71049: // ��ܺ�����
										if (g_bEnableBootaryCheck)
										{
											if (IS_BOTARYABLE_ZONE(GetMapIndex()) == true)
											{
												UseSilkBotary();
											}
											else
											{
												ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[#Unk]You cannot open a private shop on this map."));
											}
										}
										else
										{
											UseSilkBotary();
										}
										break;
								}
							}
							else
							{
								if (!item->IsEquipped())
									EquipItem(item);
								else
									UnequipItem(item);
							}
						}
						break;
				}
			}
			break;

		case ITEM_COSTUME:
		case ITEM_WEAPON:
		case ITEM_ARMOR:
		case ITEM_ROD:
		case ITEM_RING:		// �ű� ���� ������
		case ITEM_BELT:		// �ű� ��Ʈ ������
		case ITEM_TALISMAN:
			// MINING
		case ITEM_PICK:
			// END_OF_MINING
			if (!item->IsEquipped())
				EquipItem(item);
			else
				UnequipItem(item);
			break;
			// �������� ���� ��ȥ���� ����� �� ����.
			// �������� Ŭ����, ��ȥ���� ���Ͽ� item use ��Ŷ�� ���� �� ����.
			// ��ȥ�� ������ item move ��Ŷ���� �Ѵ�.
			// ������ ��ȥ���� �����Ѵ�.
		case ITEM_DS:
			{
				if (!item->IsEquipped())
					return false;
				return DSManager::instance().PullOut(this, NPOS, item);
			break;
			}
		case ITEM_SPECIAL_DS:
			if (!item->IsEquipped())
				EquipItem(item);
			else
				UnequipItem(item);
			break;

		case ITEM_FISH:
			{
				if (CArenaManager::instance().IsArenaMap(GetMapIndex()) == true)
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
					return false;
				}
#ifdef ENABLE_NEWSTUFF
				else if (g_NoPotionsOnPVP && CPVPManager::instance().IsFighting(GetPlayerID()) && !IsAllowedPotionOnPVP(item->GetVnum()))
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
					return false;
				}
#endif

				if (item->GetSubType() == FISH_ALIVE)
					fishing::UseFish(this, item);
			}
			break;

		case ITEM_TREASURE_BOX:
			{
				return false;
			}
			break;

		case ITEM_TREASURE_KEY:
			{
				LPITEM item2;

				if (!GetItem(DestCell) || !(item2 = GetItem(DestCell)))
					return false;

				if (item2->IsExchanging() || item2->IsEquipped()) // @fixme114
					return false;

				if (item2->GetType() != ITEM_TREASURE_BOX)
				{
					ChatPacket(CHAT_TYPE_TALKING, "[LS;1248]");
					return false;
				}

				if (item->GetValue(0) == item2->GetValue(0))
				{
					DWORD dwBoxVnum = item2->GetVnum();
					std::vector <DWORD> dwVnums;
					std::vector <DWORD> dwCounts;
					std::vector <LPITEM> item_gets(0);
					int count = 0;

					if (GiveItemFromSpecialItemGroup(dwBoxVnum, dwVnums, dwCounts, item_gets, count))
					{
						ITEM_MANAGER::instance().RemoveItem(item);
						ITEM_MANAGER::instance().RemoveItem(item2);

						for (int i = 0; i < count; i++){
							switch (dwVnums[i])
							{
								case CSpecialItemGroup::GOLD:
									ChatPacket(CHAT_TYPE_INFO, "[LS;1269;%d]", dwCounts[i]);
									break;
								case CSpecialItemGroup::EXP:
									ChatPacket(CHAT_TYPE_INFO, "[LS;1279]");
									ChatPacket(CHAT_TYPE_INFO, "[LS;1290;%d]", dwCounts[i]);
									break;
								case CSpecialItemGroup::MOB:
									ChatPacket(CHAT_TYPE_INFO, "[LS;1299]");
									break;
								case CSpecialItemGroup::SLOW:
									ChatPacket(CHAT_TYPE_INFO, "[LS;1310]");
									break;
								case CSpecialItemGroup::DRAIN_HP:
									ChatPacket(CHAT_TYPE_INFO, "[LS;3]");
									break;
								case CSpecialItemGroup::POISON:
									ChatPacket(CHAT_TYPE_INFO, "[LS;13]");
									break;

								case CSpecialItemGroup::MOB_GROUP:
									ChatPacket(CHAT_TYPE_INFO, "[LS;1299]");
									break;
								default:
									if (item_gets[i])
									{
										if (dwCounts[i] > 1)
											ChatPacket(CHAT_TYPE_INFO, "[LS;204;%s;%d]", item_gets[i]->GetName(), dwCounts[i]);
										else
											ChatPacket(CHAT_TYPE_INFO, "[LS;35;%s]", item_gets[i]->GetName());

									}
							}
						}
					}
					else
					{
						ChatPacket(CHAT_TYPE_TALKING, "[LS;46]");
						return false;
					}
				}
				else
				{
					ChatPacket(CHAT_TYPE_TALKING, "[LS;46]");
					return false;
				}
			}
			break;

		case ITEM_GIFTBOX:
			{
#ifdef ENABLE_NEWSTUFF
				if (0 != g_BoxUseTimeLimitValue)
				{
					if (get_dword_time() < m_dwLastBoxUseTime+g_BoxUseTimeLimitValue)
					{
						ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[#Unk]You cannot drop Yang yet"));
						return false;
					}
				}

				m_dwLastBoxUseTime = get_dword_time();
#endif

#ifdef ENABLE_BATTLE_PASS
				CBattlePass::Instance().RegisterItemMission(MISSION_TYPE_CHEST_OPEN, 1, this, item);
#endif
				DWORD dwBoxVnum = item->GetVnum();
				std::vector <DWORD> dwVnums;
				std::vector <DWORD> dwCounts;
				std::vector <LPITEM> item_gets(0);
				int count = 0;

				if( dwBoxVnum > 51500 && dwBoxVnum < 52000 )	// ��ȥ������
				{
					if( !(this->DragonSoul_IsQualified()) )
					{
						ChatPacket(CHAT_TYPE_INFO,"[LS;1094]");
						return false;
					}
				}

				if (GiveItemFromSpecialItemGroup(dwBoxVnum, dwVnums, dwCounts, item_gets, count))
				{
					item->SetCount(item->GetCount()-1);

					for (int i = 0; i < count; i++){
						switch (dwVnums[i])
						{
						case CSpecialItemGroup::GOLD:
							ChatPacket(CHAT_TYPE_INFO, "[LS;1269;%d]", dwCounts[i]);
							break;
						case CSpecialItemGroup::EXP:
							ChatPacket(CHAT_TYPE_INFO, "[LS;1279]");
							ChatPacket(CHAT_TYPE_INFO, "[LS;1290;%d]", dwCounts[i]);
							break;
						case CSpecialItemGroup::MOB:
							ChatPacket(CHAT_TYPE_INFO, "[LS;1299]");
							break;
						case CSpecialItemGroup::SLOW:
							ChatPacket(CHAT_TYPE_INFO, "[LS;1310]");
							break;
						case CSpecialItemGroup::DRAIN_HP:
							ChatPacket(CHAT_TYPE_INFO, "[LS;3]");
							break;
						case CSpecialItemGroup::POISON:
							ChatPacket(CHAT_TYPE_INFO, "[LS;13]");
							break;

						case CSpecialItemGroup::MOB_GROUP:
							ChatPacket(CHAT_TYPE_INFO, "[LS;1299]");
							break;
						default:
							if (item_gets[i])
							{
								if (dwCounts[i] > 1)
									ChatPacket(CHAT_TYPE_INFO, "[LS;204;%s;%d]", item_gets[i]->GetName(), dwCounts[i]);
								else
									ChatPacket(CHAT_TYPE_INFO, "[LS;35;%s]", item_gets[i]->GetName());
							}
						}
					}
				}
				else
				{
					ChatPacket(CHAT_TYPE_TALKING, "[LS;56]");
					return false;
				}
			}
			break;

		case ITEM_SKILLFORGET:
			{
				if (!item->GetSocket(0))
				{
					ITEM_MANAGER::instance().RemoveItem(item);
					return false;
				}

				DWORD dwVnum = item->GetSocket(0);

				if (SkillLevelDown(dwVnum))
				{
					ITEM_MANAGER::instance().RemoveItem(item);
					ChatPacket(CHAT_TYPE_INFO, "[LS;78]");
				}
				else
					ChatPacket(CHAT_TYPE_INFO, "[LS;88]");
			}
			break;

		case ITEM_SKILLBOOK:
			{
				if (IsPolymorphed())
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;1041]");
					return false;
				}

				DWORD dwVnum = 0;

				if (item->GetVnum() == 50300)
				{
					dwVnum = item->GetSocket(0);
				}
				else
				{
					// ���ο� ���ü��� value 0 �� ��ų ��ȣ�� �����Ƿ� �װ��� ���.
					dwVnum = item->GetValue(0);
				}

				if (0 == dwVnum)
				{
					ITEM_MANAGER::instance().RemoveItem(item);

					return false;
				}

#ifdef __ENABLE_SHAMAN_SYSTEM__
				if (GetShamanSystem() && GetShamanSystem()->TrainSkill(dwVnum, item))
					return true;
#endif

				if (true == LearnSkillByBook(dwVnum))
				{
#ifdef ENABLE_BOOKS_STACKFIX
					item->SetCount(item->GetCount() - 1);
#else
					ITEM_MANAGER::instance().RemoveItem(item);
#endif

					int iReadDelay = number(SKILLBOOK_DELAY_MIN, SKILLBOOK_DELAY_MAX);


					SetSkillNextReadTime(dwVnum, get_global_time() + iReadDelay);
				}
			}
			break;

		case ITEM_USE: // LINGOURI_SURSA
			{
				
                switch (item->GetVnum())
                {
                    case 80003:
                    case 80004:
                    case 80005:
                    case 80006:
                    case 80007:
                    case 80008:
                        {
                            static const int sGold[6] =
                            {
                                500000,      ///< 80003
                                1000000,     ///< 80004
                                500000,     ///< 80005
                                1000000,    ///< 80006
                                2000000,     ///< 80007
                                10000000     ///< 80008
                            };

                            if (IsOpenSafebox() || GetExchange() || GetMyShop() || IsCubeOpen())
                            {
                                ChatPacket(CHAT_TYPE_INFO, "Nu poti folosi lingoul.");
                                return false;
                            }

                            const int amount = sGold[item->GetVnum() - 80003];
                            if ((GOLD_MAX_MAX - amount) <= GetGold())
                            {
                                ChatPacket(CHAT_TYPE_INFO, "Nu poti detine mai mult de x Yang.");
                                return false;
                            }

                            item->SetCount(item->GetCount() - 1);
                            ChangeGold(amount);
                        }
                        break;
                    default:
                        break;
                }

				if (item->GetVnum() > 50800 && item->GetVnum() <= 50820)
				{
					if (test_server)
						sys_log (0, "ADD addtional effect : vnum(%d) subtype(%d)", item->GetOriginalVnum(), item->GetSubType());

					int affect_type = AFFECT_EXP_BONUS_EURO_FREE;
					int apply_type = aApplyInfo[item->GetValue(0)].bPointType;

					const auto apply_value = item->GetValue(2);

					int apply_duration = item->GetValue(1);

					switch (item->GetSubType())
					{
						case USE_ABILITY_UP:
							if (FindAffect(affect_type, apply_type))
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
								return false;
							}

							{
								switch (item->GetValue(0))
								{
									case APPLY_MOV_SPEED:
										AddAffect(affect_type, apply_type, apply_value, AFF_MOV_SPEED_POTION, apply_duration, 0, true, true);
										break;

									case APPLY_ATT_SPEED:
										AddAffect(affect_type, apply_type, apply_value, AFF_ATT_SPEED_POTION, apply_duration, 0, true, true);
										break;

									case APPLY_STR:
									case APPLY_DEX:
									case APPLY_CON:
									case APPLY_INT:
									case APPLY_CAST_SPEED:
									case APPLY_RESIST_MAGIC:
									case APPLY_ATT_GRADE_BONUS:
									case APPLY_DEF_GRADE_BONUS:
										AddAffect(affect_type, apply_type, apply_value, 0, apply_duration, 0, true, true);
										break;
								}
							}

							if (GetDungeon())
								GetDungeon()->UsePotion(this);

							if (GetWarMap())
								GetWarMap()->UsePotion(this, item);

							item->SetCount(item->GetCount() - 1);
							break;

					case USE_AFFECT :
						{
							if (FindAffect(AFFECT_EXP_BONUS_EURO_FREE, aApplyInfo[item->GetValue(1)].bPointType))
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
							}
							else
							{
								AddAffect(AFFECT_EXP_BONUS_EURO_FREE, aApplyInfo[item->GetValue(1)].bPointType, item->GetValue(2), 0, item->GetValue(3), 0, false, true);
								item->SetCount(item->GetCount() - 1);
							}
						}
						break;

					case USE_POTION_NODELAY:
						{
							if (CArenaManager::instance().IsArenaMap(GetMapIndex()) == true)
							{
								if (quest::CQuestManager::instance().GetEventFlag("arena_potion_limit") > 0)
								{
									ChatPacket(CHAT_TYPE_INFO, "[LS;403]");
									return false;
								}

								switch (item->GetVnum())
								{
									case 70020 :
									case 71018 :
									case 71019 :
									case 71020 :
										if (quest::CQuestManager::instance().GetEventFlag("arena_potion_limit_count") < 10000)
										{
											if (m_nPotionLimit <= 0)
											{
												ChatPacket(CHAT_TYPE_INFO, "[LS;122]");
												return false;
											}
										}
										break;

									default :
										ChatPacket(CHAT_TYPE_INFO, "[LS;403]");
										return false;
										break;
								}
							}
#ifdef ENABLE_NEWSTUFF
							else if (g_NoPotionsOnPVP && CPVPManager::instance().IsFighting(GetPlayerID()) && !IsAllowedPotionOnPVP(item->GetVnum()))
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
								return false;
							}
#endif

							bool used = false;

							if (item->GetValue(0) != 0) // HP ���밪 ȸ��
							{
								if (GetHP() < GetMaxHP())
								{
									PointChange(POINT_HP, item->GetValue(0) * (100 + GetPoint(POINT_POTION_BONUS)) / 100);
									EffectPacket(SE_HPUP_RED);
									used = TRUE;
								}
							}

							if (item->GetValue(1) != 0)	// SP ���밪 ȸ��
							{
								if (GetSP() < GetMaxSP())
								{
									PointChange(POINT_SP, item->GetValue(1) * (100 + GetPoint(POINT_POTION_BONUS)) / 100);
									EffectPacket(SE_SPUP_BLUE);
									used = TRUE;
								}
							}

							if (item->GetValue(3) != 0) // HP % ȸ��
							{
								if (GetHP() < GetMaxHP())
								{
									PointChange(POINT_HP, item->GetValue(3) * GetMaxHP() / 100);
									EffectPacket(SE_HPUP_RED);
									used = TRUE;
								}
							}

							if (item->GetValue(4) != 0) // SP % ȸ��
							{
								if (GetSP() < GetMaxSP())
								{
									PointChange(POINT_SP, item->GetValue(4) * GetMaxSP() / 100);
									EffectPacket(SE_SPUP_BLUE);
									used = TRUE;
								}
							}

							if (used)
							{
								if (item->GetVnum() == 50085 || item->GetVnum() == 50086)
								{
									if (test_server)
										ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[#Unk] Used moon cake or seeds."));
									SetUseSeedOrMoonBottleTime();
								}
								if (GetDungeon())
									GetDungeon()->UsePotion(this);

								if (GetWarMap())
									GetWarMap()->UsePotion(this, item);

								m_nPotionLimit--;

								//RESTRICT_USE_SEED_OR_MOONBOTTLE
								item->SetCount(item->GetCount() - 1);
								//END_RESTRICT_USE_SEED_OR_MOONBOTTLE
							}
						}
						break;
					}

					return true;
				}


				if (item->GetVnum() >= 27863 && item->GetVnum() <= 27883)
				{
					if (CArenaManager::instance().IsArenaMap(GetMapIndex()) == true)
					{
						ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
						return false;
					}
#ifdef ENABLE_NEWSTUFF
					else if (g_NoPotionsOnPVP && CPVPManager::instance().IsFighting(GetPlayerID()) && !IsAllowedPotionOnPVP(item->GetVnum()))
					{
						ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
						return false;
					}
#endif
				}

				if (test_server)
				{
					 sys_log (0, "USE_ITEM %s Type %d SubType %d vnum %d", item->GetName(), item->GetType(), item->GetSubType(), item->GetOriginalVnum());
				}

				switch (item->GetSubType())
				{
					case USE_TIME_CHARGE_PER:
						{
							LPITEM pDestItem = GetItem(DestCell);
							if (NULL == pDestItem)
							{
								return false;
							}
							// �켱 ��ȥ���� ���ؼ��� �ϵ��� �Ѵ�.
							if (pDestItem->IsDragonSoul())
							{
								int ret;
								char buf[128];
								if (item->GetVnum() == DRAGON_HEART_VNUM)
								{
									ret = pDestItem->GiveMoreTime_Per((float)item->GetSocket(ITEM_SOCKET_CHARGING_AMOUNT_IDX));
								}
								else
								{
									ret = pDestItem->GiveMoreTime_Per((float)item->GetValue(ITEM_VALUE_CHARGING_AMOUNT_IDX));
								}
								if (ret > 0)
								{
									if (item->GetVnum() == DRAGON_HEART_VNUM)
									{
										sprintf(buf, "Inc %ds by item{VN:%d SOC%d:%ld}", ret, item->GetVnum(), ITEM_SOCKET_CHARGING_AMOUNT_IDX, item->GetSocket(ITEM_SOCKET_CHARGING_AMOUNT_IDX));
									}
									else
									{
										sprintf(buf, "Inc %ds by item{VN:%d VAL%d:%ld}", ret, item->GetVnum(), ITEM_VALUE_CHARGING_AMOUNT_IDX, item->GetValue(ITEM_VALUE_CHARGING_AMOUNT_IDX));
									}

									ChatPacket(CHAT_TYPE_INFO, "[LS;1093;%d]", ret);
									item->SetCount(item->GetCount() - 1);
									LogManager::instance().ItemLog(this, item, "DS_CHARGING_SUCCESS", buf);
									return true;
								}
								else
								{
									if (item->GetVnum() == DRAGON_HEART_VNUM)
									{
										sprintf(buf, "No change by item{VN:%d SOC%d:%ld}", item->GetVnum(), ITEM_SOCKET_CHARGING_AMOUNT_IDX, item->GetSocket(ITEM_SOCKET_CHARGING_AMOUNT_IDX));
									}
									else
									{
										sprintf(buf, "No change by item{VN:%d VAL%d:%ld}", item->GetVnum(), ITEM_VALUE_CHARGING_AMOUNT_IDX, item->GetValue(ITEM_VALUE_CHARGING_AMOUNT_IDX));
									}

									ChatPacket(CHAT_TYPE_INFO, "[LS;1066]");
									LogManager::instance().ItemLog(this, item, "DS_CHARGING_FAILED", buf);
									return false;
								}
							}
							else
								return false;
						}
						break;
					case USE_TIME_CHARGE_FIX:
						{
							LPITEM pDestItem = GetItem(DestCell);
							if (NULL == pDestItem)
							{
								return false;
							}
							// �켱 ��ȥ���� ���ؼ��� �ϵ��� �Ѵ�.
							if (pDestItem->IsDragonSoul())
							{
								int ret = pDestItem->GiveMoreTime_Fix(item->GetValue(ITEM_VALUE_CHARGING_AMOUNT_IDX));
								char buf[128];
								if (ret)
								{
									ChatPacket(CHAT_TYPE_INFO, "[LS;1093;%d]", ret);
									sprintf(buf, "Increase %ds by item{VN:%d VAL%d:%ld}", ret, item->GetVnum(), ITEM_VALUE_CHARGING_AMOUNT_IDX, item->GetValue(ITEM_VALUE_CHARGING_AMOUNT_IDX));
									LogManager::instance().ItemLog(this, item, "DS_CHARGING_SUCCESS", buf);
									item->SetCount(item->GetCount() - 1);
									return true;
								}
								else
								{
									ChatPacket(CHAT_TYPE_INFO, "[LS;1066]");
									sprintf(buf, "No change by item{VN:%d VAL%d:%ld}", item->GetVnum(), ITEM_VALUE_CHARGING_AMOUNT_IDX, item->GetValue(ITEM_VALUE_CHARGING_AMOUNT_IDX));
									LogManager::instance().ItemLog(this, item, "DS_CHARGING_FAILED", buf);
									return false;
								}
							}
							else
								return false;
						}
						break;
						
#ifdef ENABLE_REFINE_ELEMENT
					case USE_ELEMENT_UPGRADE:
						{
							LPITEM pDestItem;
							if (!IsValidItemPosition(DestCell) || !(pDestItem = GetItem(DestCell)))
								return false;
							
							if (pDestItem->IsExchanging() || pDestItem->IsEquipped())
								return false;
							
							if(item->GetValue(0) <= REFINE_ELEMENT_CATEGORY_NONE || item->GetValue(0) >= REFINE_ELEMENT_CATEGORY_MAX)
							{
								ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Cannot be used with this item."));
								return false;
							}
							
							if(pDestItem->GetType() != ITEM_WEAPON)
							{
								ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Cannot be used with this item."));
								return false;
							}
							
							if(pDestItem->GetRefineLevel() < ELEMENT_MIN_REFINE_LEVEL)
							{
								ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The elemental enchantment is only available for +7 weapons or higher."));
								return false;
							}
							
							if(pDestItem->GetRefineElementPlus() == REFINE_ELEMENT_MAX)
							{
								ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The maximum elemental enchantment level has already been reached."));
								return false;
							}
							
							if(pDestItem->GetRefineElementType() > 0 && pDestItem->GetRefineElementType() != item->GetValue(0))
							{
								ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The item already has another element."));
								return false;
							}
							
							if(GetGold() < REFINE_ELEMENT_UPGRADE_YANG)
							{
								ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Not enough Yang."));
								return false;
							}
							
							if (!CanHandleItem())
								return false;
							
							RefineElementInformation(item->GetCell(), pDestItem->GetCell(), REFINE_ELEMENT_TYPE_UPGRADE);
							return true;
						}
						break;
						
					case USE_ELEMENT_DOWNGRADE:
						{
							LPITEM pDestItem;
							if (!IsValidItemPosition(DestCell) || !(pDestItem = GetItem(DestCell)))
								return false;
							
							if (pDestItem->IsExchanging() || pDestItem->IsEquipped())
								return false;

							if(pDestItem->GetType() != ITEM_WEAPON)
							{
								ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Cannot be used with this item."));
								return false;
							}
							
							if(!pDestItem->GetRefineElementPlus())
								return false;
							
							if(GetGold() < REFINE_ELEMENT_DOWNGRADE_YANG)
							{
								ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Not enough Yang."));
								return false;
							}
							
							if (!CanHandleItem())
								return false;
							
							RefineElementInformation(item->GetCell(), pDestItem->GetCell(), REFINE_ELEMENT_TYPE_DOWNGRADE);
							return true;
						}
						break;
						
					case USE_ELEMENT_CHANGE:
						{
							LPITEM pDestItem;
							if (!IsValidItemPosition(DestCell) || !(pDestItem = GetItem(DestCell)))
								return false;
							
							if (pDestItem->IsExchanging() || pDestItem->IsEquipped())
								return false;

							if(pDestItem->GetType() != ITEM_WEAPON)
							{
								ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Cannot be used with this item."));
								return false;
							}
							
							if(!pDestItem->GetRefineElementPlus())
								return false;
							
							if(GetGold() < REFINE_ELEMENT_CHANGE_YANG)
							{
								ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Not enough Yang."));
								return false;
							}
							
							if (!CanHandleItem())
								return false;
							
							RefineElementInformation(item->GetCell(), pDestItem->GetCell(), REFINE_ELEMENT_TYPE_CHANGE);
							return true;
						}
						break;
#endif
	
					case USE_SPECIAL:

						switch (item->GetVnum())
						{
							//ũ�������� ����
							case ITEM_NOG_POCKET:
								{
									/*
									���ִɷ�ġ : item_proto value �ǹ�
										�̵��ӵ�  value 1
										���ݷ�	  value 2
										����ġ    value 3
										���ӽð�  value 0 (���� ��)

									*/
									if (FindAffect(AFFECT_NOG_ABILITY))
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
										return false;
									}
									long time = item->GetValue(0);
									long moveSpeedPer	= item->GetValue(1);
									long attPer	= item->GetValue(2);
									long expPer			= item->GetValue(3);
									AddAffect(AFFECT_NOG_ABILITY, POINT_MOV_SPEED, moveSpeedPer, AFF_MOV_SPEED_POTION, time, 0, true, true);
									AddAffect(AFFECT_NOG_ABILITY, POINT_MALL_ATTBONUS, attPer, AFF_NONE, time, 0, true, true);
									AddAffect(AFFECT_NOG_ABILITY, POINT_MALL_EXPBONUS, expPer, AFF_NONE, time, 0, true, true);
									item->SetCount(item->GetCount() - 1);
								}
								break;

							//�󸶴ܿ� ����
							case ITEM_RAMADAN_CANDY:
								{
									/*
									�����ɷ�ġ : item_proto value �ǹ�
										�̵��ӵ�  value 1
										���ݷ�	  value 2
										����ġ    value 3
										���ӽð�  value 0 (���� ��)

									*/
									// @fixme147 BEGIN
									if (FindAffect(AFFECT_RAMADAN_ABILITY))
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
										return false;
									}
									// @fixme147 END
									long time = item->GetValue(0);
									long moveSpeedPer	= item->GetValue(1);
									long attPer	= item->GetValue(2);
									long expPer			= item->GetValue(3);
									AddAffect(AFFECT_RAMADAN_ABILITY, POINT_MOV_SPEED, moveSpeedPer, AFF_MOV_SPEED_POTION, time, 0, true, true);
									AddAffect(AFFECT_RAMADAN_ABILITY, POINT_MALL_ATTBONUS, attPer, AFF_NONE, time, 0, true, true);
									AddAffect(AFFECT_RAMADAN_ABILITY, POINT_MALL_EXPBONUS, expPer, AFF_NONE, time, 0, true, true);
									item->SetCount(item->GetCount() - 1);
								}
								break;
							case ITEM_MARRIAGE_RING:
								{
									marriage::TMarriage* pMarriage = marriage::CManager::instance().Get(GetPlayerID());
									if (pMarriage)
									{
										if (pMarriage->ch1 != NULL)
										{
											if (CArenaManager::instance().IsArenaMap(pMarriage->ch1->GetMapIndex()) == true)
											{
												ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
												break;
											}
										}

										if (pMarriage->ch2 != NULL)
										{
											if (CArenaManager::instance().IsArenaMap(pMarriage->ch2->GetMapIndex()) == true)
											{
												ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
												break;
											}
										}

										int consumeSP = CalculateConsumeSP(this);

										if (consumeSP < 0)
											return false;

										PointChange(POINT_SP, -consumeSP, false);

										WarpToPID(pMarriage->GetOther(GetPlayerID()));
									}
									else
										ChatPacket(CHAT_TYPE_INFO, "[LS;143]");
								}
								break;

								//���� ����� ����
							case UNIQUE_ITEM_CAPE_OF_COURAGE:
								//�󸶴� ����� ����� ����
							case 70057:
							case REWARD_BOX_UNIQUE_ITEM_CAPE_OF_COURAGE:
								AggregateMonster();
								if (MANTIE_PERMANENT == false)
								{
									item->SetCount(item->GetCount()-1);
									break;
								}
								break;


							case UNIQUE_ITEM_WHITE_FLAG:
								ForgetMyAttacker();
								item->SetCount(item->GetCount()-1);
								break;

							case UNIQUE_ITEM_TREASURE_BOX:
								break;


#ifdef ENABLE_MULTI_FARM_BLOCK
						case 55610:
						case 55611:
						case 55612:
						case 55613:
						case 55614:
						case 55615:
						{
							if (FindAffect(AFFECT_MULTI_FARM_PREMIUM))
							{
								ChatPacket(CHAT_TYPE_INFO, "Ai deja acest affect activ!");
								return false;
							}
							else
							{
								AddAffect(AFFECT_MULTI_FARM_PREMIUM, POINT_NONE, item->GetValue(1), AFF_NONE, item->GetValue(0), 0, false, false);
								item->SetCount(item->GetCount() - 1);
								CHARACTER_MANAGER::Instance().CheckMultiFarmAccount(GetDesc()->GetHostName(), GetPlayerID(), GetName(), GetMultiStatus());
								ChatPacket(CHAT_TYPE_INFO, "Affect succesfully added on your character!");
								ChatPacket(CHAT_TYPE_INFO, "If you want use this affect this character need active drop status!");
							}
						}
						break;
#endif


							case 30093:
							case 30094:
							case 30095:
							case 30096:
								// ���ָӴ�
								{
									const int MAX_BAG_INFO = 26;
									static struct LuckyBagInfo
									{
										DWORD count;
										int prob;
										DWORD vnum;
									} b1[MAX_BAG_INFO] =
									{
										{ 1000,	302,	1 },
										{ 10,	150,	27002 },
										{ 10,	75,	27003 },
										{ 10,	100,	27005 },
										{ 10,	50,	27006 },
										{ 10,	80,	27001 },
										{ 10,	50,	27002 },
										{ 10,	80,	27004 },
										{ 10,	50,	27005 },
										{ 1,	10,	50300 },
										{ 1,	6,	92 },
										{ 1,	2,	132 },
										{ 1,	6,	1052 },
										{ 1,	2,	1092 },
										{ 1,	6,	2082 },
										{ 1,	2,	2122 },
										{ 1,	6,	3082 },
										{ 1,	2,	3122 },
										{ 1,	6,	5052 },
										{ 1,	2,	5082 },
										{ 1,	6,	7082 },
										{ 1,	2,	7122 },
										{ 1,	1,	11282 },
										{ 1,	1,	11482 },
										{ 1,	1,	11682 },
										{ 1,	1,	11882 },
									};

									LuckyBagInfo * bi = NULL;
									bi = b1;

									int pct = number(1, 1000);

									int i;
									for (i=0;i<MAX_BAG_INFO;i++)
									{
										if (pct <= bi[i].prob)
											break;
										pct -= bi[i].prob;
									}
									if (i>=MAX_BAG_INFO)
										return false;

									if (bi[i].vnum == 50300)
									{
										// ��ų���ü��� Ư���ϰ� �ش�.
										GiveRandomSkillBook();
									}
									else if (bi[i].vnum == 1)
									{
#ifdef ENABLE_REMOVE_LIMIT_GOLD
										ChangeGold(1000);
#else
										PointChange(POINT_GOLD, 1000, true);
#endif
									}
									else
									{
										AutoGiveItem(bi[i].vnum, bi[i].count);
									}
									ITEM_MANAGER::instance().RemoveItem(item);
								}
								break;



							case 50004: // �̺�Ʈ�� ������
								{
									if (item->GetSocket(0))
									{
										item->SetSocket(0, item->GetSocket(0) + 1);
									}
									else
									{
										// ó�� ����
										int iMapIndex = GetMapIndex();

										PIXEL_POSITION pos;

										if (SECTREE_MANAGER::instance().GetRandomLocation(iMapIndex, pos, 700))
										{
											item->SetSocket(0, 1);
											item->SetSocket(1, pos.x);
											item->SetSocket(2, pos.y);
										}
										else
										{
											ChatPacket(CHAT_TYPE_INFO, "[LS;154]");
											return false;
										}
									}

									int dist = 0;
									float distance = (DISTANCE_SQRT(GetX()-item->GetSocket(1), GetY()-item->GetSocket(2)));

									if (distance < 1000.0f)
									{
										// �߰�!
										ChatPacket(CHAT_TYPE_INFO, "[LS;165]");

										// ���Ƚ���� ���� �ִ� �������� �ٸ��� �Ѵ�.
										struct TEventStoneInfo
										{
											DWORD dwVnum;
											int count;
											int prob;
										};
										const int EVENT_STONE_MAX_INFO = 15;
										TEventStoneInfo info_10[EVENT_STONE_MAX_INFO] =
										{
											{ 27001, 10,  8 },
											{ 27004, 10,  6 },
											{ 27002, 10, 12 },
											{ 27005, 10, 12 },
											{ 27100,  1,  9 },
											{ 27103,  1,  9 },
											{ 27101,  1, 10 },
											{ 27104,  1, 10 },
											{ 27999,  1, 12 },

											{ 25040,  1,  4 },

											{ 27410,  1,  0 },
											{ 27600,  1,  0 },
											{ 25100,  1,  0 },

											{ 50001,  1,  0 },
											{ 50003,  1,  1 },
										};
										TEventStoneInfo info_7[EVENT_STONE_MAX_INFO] =
										{
											{ 27001, 10,  1 },
											{ 27004, 10,  1 },
											{ 27004, 10,  9 },
											{ 27005, 10,  9 },
											{ 27100,  1,  5 },
											{ 27103,  1,  5 },
											{ 27101,  1, 10 },
											{ 27104,  1, 10 },
											{ 27999,  1, 14 },

											{ 25040,  1,  5 },

											{ 27410,  1,  5 },
											{ 27600,  1,  5 },
											{ 25100,  1,  5 },

											{ 50001,  1,  0 },
											{ 50003,  1,  5 },

										};
										TEventStoneInfo info_4[EVENT_STONE_MAX_INFO] =
										{
											{ 27001, 10,  0 },
											{ 27004, 10,  0 },
											{ 27002, 10,  0 },
											{ 27005, 10,  0 },
											{ 27100,  1,  0 },
											{ 27103,  1,  0 },
											{ 27101,  1,  0 },
											{ 27104,  1,  0 },
											{ 27999,  1, 25 },

											{ 25040,  1,  0 },

											{ 27410,  1,  0 },
											{ 27600,  1,  0 },
											{ 25100,  1, 15 },

											{ 50001,  1, 10 },
											{ 50003,  1, 50 },

										};

										{
											TEventStoneInfo* info;
											if (item->GetSocket(0) <= 4)
												info = info_4;
											else if (item->GetSocket(0) <= 7)
												info = info_7;
											else
												info = info_10;

											int prob = number(1, 100);

											for (int i = 0; i < EVENT_STONE_MAX_INFO; ++i)
											{
												if (!info[i].prob)
													continue;

												if (prob <= info[i].prob)
												{
													if (info[i].dwVnum == 50001)
													{
														DWORD * pdw = M2_NEW DWORD[2];

														pdw[0] = info[i].dwVnum;
														pdw[1] = info[i].count;

														// ��÷���� ������ �����Ѵ�
														DBManager::instance().ReturnQuery(QID_LOTTO, GetPlayerID(), pdw,
																"INSERT INTO lotto_list VALUES(0, 'server%s', %u, NOW())",
																get_table_postfix(), GetPlayerID());
													}
													else
														AutoGiveItem(info[i].dwVnum, info[i].count);

													break;
												}
												prob -= info[i].prob;
											}
										}

										char chatbuf[CHAT_MAX_LEN + 1];
										int len = snprintf(chatbuf, sizeof(chatbuf), "StoneDetect %u 0 0", (DWORD)GetVID());

										if (len < 0 || len >= (int) sizeof(chatbuf))
											len = sizeof(chatbuf) - 1;

										++len;  // \0 ���ڱ��� ������

										TPacketGCChat pack_chat;
										pack_chat.header	= HEADER_GC_CHAT;
										pack_chat.size		= sizeof(TPacketGCChat) + len;
										pack_chat.type		= CHAT_TYPE_COMMAND;
										pack_chat.id		= 0;
										pack_chat.bEmpire	= GetDesc()->GetEmpire();
										//pack_chat.id	= vid;

										TEMP_BUFFER buf;
										buf.write(&pack_chat, sizeof(TPacketGCChat));
										buf.write(chatbuf, len);

										PacketAround(buf.read_peek(), buf.size());

										ITEM_MANAGER::instance().RemoveItem(item, "REMOVE (DETECT_EVENT_STONE) 1");
										return true;
									}
									else if (distance < 20000)
										dist = 1;
									else if (distance < 70000)
										dist = 2;
									else
										dist = 3;

									// ���� ��������� �������.
									const int STONE_DETECT_MAX_TRY = 10;
									if (item->GetSocket(0) >= STONE_DETECT_MAX_TRY)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;176]");
										ITEM_MANAGER::instance().RemoveItem(item, "REMOVE (DETECT_EVENT_STONE) 0");
										AutoGiveItem(27002);
										return true;
									}

									if (dist)
									{
										char chatbuf[CHAT_MAX_LEN + 1];
										int len = snprintf(chatbuf, sizeof(chatbuf),
												"StoneDetect %u %d %d",
											   	(DWORD)GetVID(), dist, (int)GetDegreeFromPositionXY(GetX(), item->GetSocket(2), item->GetSocket(1), GetY()));

										if (len < 0 || len >= (int) sizeof(chatbuf))
											len = sizeof(chatbuf) - 1;

										++len;  // \0 ���ڱ��� ������

										TPacketGCChat pack_chat;
										pack_chat.header	= HEADER_GC_CHAT;
										pack_chat.size		= sizeof(TPacketGCChat) + len;
										pack_chat.type		= CHAT_TYPE_COMMAND;
										pack_chat.id		= 0;
										pack_chat.bEmpire	= GetDesc()->GetEmpire();
										//pack_chat.id		= vid;

										TEMP_BUFFER buf;
										buf.write(&pack_chat, sizeof(TPacketGCChat));
										buf.write(chatbuf, len);

										PacketAround(buf.read_peek(), buf.size());
									}

								}
								break;

							case 27989: // ����������
							case 76006: // ������ ����������
								{
									LPSECTREE_MAP pMap = SECTREE_MANAGER::instance().GetMap(GetMapIndex());

									if (pMap != NULL)
									{
										item->SetSocket(0, item->GetSocket(0) + 1);

										FFindStone f;

										// <Factor> SECTREE::for_each -> SECTREE::for_each_entity
										pMap->for_each(f);

										if (f.m_mapStone.size() > 0)
										{
											std::map<DWORD, LPCHARACTER>::iterator stone = f.m_mapStone.begin();

											DWORD max = UINT_MAX;
											LPCHARACTER pTarget = stone->second;

											while (stone != f.m_mapStone.end())
											{
												DWORD dist = (DWORD)DISTANCE_SQRT(GetX()-stone->second->GetX(), GetY()-stone->second->GetY());

												if (dist != 0 && max > dist)
												{
													max = dist;
													pTarget = stone->second;
												}
												stone++;
											}

											if (pTarget != NULL)
											{
												int val = 3;

												if (max < 10000) val = 2;
												else if (max < 70000) val = 1;

												ChatPacket(CHAT_TYPE_COMMAND, "StoneDetect %u %d %d", (DWORD)GetVID(), val,
														(int)GetDegreeFromPositionXY(GetX(), pTarget->GetY(), pTarget->GetX(), GetY()));
											}
											else
											{
												ChatPacket(CHAT_TYPE_INFO, "[LS;1461]");
											}
										}
										else
										{
											ChatPacket(CHAT_TYPE_INFO, "[LS;1461]");
										}

										if (item->GetSocket(0) >= 6)
										{
											ChatPacket(CHAT_TYPE_COMMAND, "StoneDetect %u 0 0", (DWORD)GetVID());
											ITEM_MANAGER::instance().RemoveItem(item);
										}
									}
									break;
								}
								break;

							case 27996: // ����
								item->SetCount(item->GetCount() - 1);
								AttackedByPoison(NULL); // @warme008
								break;

							case 27987: // ����
								// 50  ������ 47990
								// 30  ��
								// 10  ������ 47992
								// 7   û���� 47993
								// 3   ������ 47994
								{
#ifdef ENABLE_BATTLE_PASS
									CBattlePass::Instance().RegisterItemMission(MISSION_TYPE_CHEST_OPEN, 1, this, item);
#endif
									
									item->SetCount(item->GetCount() - 1);

									int r = number(1, 100);

									if (r <= 50)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;221]");
										AutoGiveItem(27990);
									}
									else
									{
										const int prob_table_gb2312[] =
										{
											95, 97, 99
										};

										const int * prob_table = prob_table_gb2312;

										if (r <= prob_table[0])
										{
											ChatPacket(CHAT_TYPE_INFO, "[LS;232]");
										}
										else if (r <= prob_table[1])
										{
											ChatPacket(CHAT_TYPE_INFO, "[LS;243]");
											AutoGiveItem(27992);
										}
										else if (r <= prob_table[2])
										{
											ChatPacket(CHAT_TYPE_INFO, "[LS;253]");
											AutoGiveItem(27993);
										}
										else
										{
											ChatPacket(CHAT_TYPE_INFO, "[LS;264]");
											AutoGiveItem(27994);
										}
									}
								}
								break;

							case 71013: // ����������
								CreateFly(number(FLY_FIREWORK1, FLY_FIREWORK6), this);
								item->SetCount(item->GetCount() - 1);
								break;

							case 50100: // ����
							case 50101:
							case 50102:
							case 50103:
							case 50104:
							case 50105:
							case 50106:
								CreateFly(item->GetVnum() - 50100 + FLY_FIREWORK1, this);
								item->SetCount(item->GetCount() - 1);
								break;

			case 50200:
			{
#ifdef __PREMIUM_PRIVATE_SHOP__
				if (IsPrivateShopOwner())
				{
					ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Close your current personal shop before opening a new one."));
					return false;
				}

				OpenPrivateShopPanel();
#else
				__OpenPrivateShop();
#endif
			}
			break;
			
#ifdef __PREMIUM_PRIVATE_SHOP__
			case 71221:
			{
				if (IsPrivateShopOwner())
				{
					ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Close your current personal shop before opening a new one."));
					return false;
				}

				OpenPrivateShopPanel();
				ChatPacket(CHAT_TYPE_COMMAND, "SetPrivateShopPremiumBuild");
			} break;

			case 60004:
			{
				OpenShopSearch(MODE_LOOKING);
			} break;

			case 60005:
			{
				OpenShopSearch(MODE_TRADING);
			} break;
#endif
							case fishing::FISH_MIND_PILL_VNUM:
								AddAffect(AFFECT_FISH_MIND_PILL, POINT_NONE, 0, AFF_FISH_MIND, 20*60, 0, true);
								item->SetCount(item->GetCount() - 1);
								break;

							case 50301: // ��ַ� ���ü�
							case 50302:
							case 50303:
								{
									if (IsPolymorphed() == true)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;521]");
										return false;
									}

									int lv = GetSkillLevel(SKILL_LEADERSHIP);

									if (lv < item->GetValue(0))
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;274]");
										return false;
									}

									if (lv >= item->GetValue(1))
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;284]");
										return false;
									}

									if (LearnSkillByBook(SKILL_LEADERSHIP))
									{
#ifdef ENABLE_BOOKS_STACKFIX
										item->SetCount(item->GetCount() - 1);
#else
										ITEM_MANAGER::instance().RemoveItem(item);
#endif

										int iReadDelay = number(SKILLBOOK_DELAY_MIN, SKILLBOOK_DELAY_MAX);

										SetSkillNextReadTime(SKILL_LEADERSHIP, get_global_time() + iReadDelay);
									}
								}
								break;

							case 50304: // ����� ���ü�
							case 50305:
							case 50306:
								{
									if (IsPolymorphed())
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;1041]");
										return false;

									}
									if (GetSkillLevel(SKILL_COMBO) == 0 && GetLevel() < 30)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;295]");
										return false;
									}

									if (GetSkillLevel(SKILL_COMBO) == 1 && GetLevel() < 50)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;305]");
										return false;
									}

									if (GetSkillLevel(SKILL_COMBO) >= 2)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;316]");
										return false;
									}

									int iPct = item->GetValue(0);
 
									if (LearnSkillByBook(SKILL_COMBO, iPct))
									{
#ifdef ENABLE_BOOKS_STACKFIX
										item->SetCount(item->GetCount() - 1);
#else
										ITEM_MANAGER::instance().RemoveItem(item);
#endif

										int iReadDelay = number(SKILLBOOK_DELAY_MIN, SKILLBOOK_DELAY_MAX);

										SetSkillNextReadTime(SKILL_COMBO, get_global_time() + iReadDelay);
									}
								}
								break;
							case 50311: // ��� ���ü�
							case 50312:
							case 50313:
								{
									if (IsPolymorphed())
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;1041]");
										return false;

									}
									DWORD dwSkillVnum = item->GetValue(0);
									int iPct = MINMAX(0, item->GetValue(1), 100);
									if (GetSkillLevel(dwSkillVnum)>=20 || dwSkillVnum-SKILL_LANGUAGE1+1 == GetEmpire())
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;328]");
										return false;
									}

									if (LearnSkillByBook(dwSkillVnum, iPct))
									{
#ifdef ENABLE_BOOKS_STACKFIX
										item->SetCount(item->GetCount() - 1);
#else
										ITEM_MANAGER::instance().RemoveItem(item);
#endif

										int iReadDelay = number(SKILLBOOK_DELAY_MIN, SKILLBOOK_DELAY_MAX);
										SetSkillNextReadTime(dwSkillVnum, get_global_time() + iReadDelay);
									}
								}
								break;

							case 50061 : // �Ϻ� �� ��ȯ ��ų ���ü�
								{
									if (IsPolymorphed())
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;1041]");
										return false;

									}
									DWORD dwSkillVnum = item->GetValue(0);
									int iPct = MINMAX(0, item->GetValue(1), 100);

									if (GetSkillLevel(dwSkillVnum) >= 10)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;948]");
										return false;
									}

									if (LearnSkillByBook(dwSkillVnum, iPct))
									{
#ifdef ENABLE_BOOKS_STACKFIX
										item->SetCount(item->GetCount() - 1);
#else
										ITEM_MANAGER::instance().RemoveItem(item);
#endif

										int iReadDelay = number(SKILLBOOK_DELAY_MIN, SKILLBOOK_DELAY_MAX);

										SetSkillNextReadTime(dwSkillVnum, get_global_time() + iReadDelay);
									}
								}
								break;

							case 50314: case 50315: case 50316: // ���� ���ü�
							case 50323: case 50324: // ���� ���ü�
							case 50325: case 50326: // ö�� ���ü�
#ifdef ENABLE_NEW_PASSIVE_SKILL
							case 50335: case 50336: //SKILL_MONSTER_BONUS
							case 50337: case 50338: //SKILL_BOSS_BONUS
							case 50339: case 50340: //SKILL_BOSS_BONUS
#endif

								{
									if (IsPolymorphed() == true)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;521]");
										return false;
									}

									int iSkillLevelLowLimit = item->GetValue(0);
									int iSkillLevelHighLimit = item->GetValue(1);
									int iPct = MINMAX(0, item->GetValue(2), 100);
									int iLevelLimit = item->GetValue(3);
									DWORD dwSkillVnum = 0;

									switch (item->GetVnum())
									{
										case 50314: case 50315: case 50316:
											dwSkillVnum = SKILL_POLYMORPH;
											break;

										case 50323: case 50324:
											dwSkillVnum = SKILL_ADD_HP;
											break;

										case 50325: case 50326:
											dwSkillVnum = SKILL_RESIST_PENETRATE;
											break;
											
#ifdef ENABLE_NEW_PASSIVE_SKILL
										case 50335: case 50336:
											dwSkillVnum = SKILL_MONSTER_BONUS;
											break;
										case 50337: case 50338:
											dwSkillVnum = SKILL_STONE_BONUS;
											break;
										case 50339: case 50340:
											dwSkillVnum = SKILL_BOSS_BONUS;
											break;
#endif

										default:
											return false;
									}

									if (0 == dwSkillVnum)
										return false;

									if (GetLevel() < iLevelLimit)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;350]");
										return false;
									}

									if (GetSkillLevel(dwSkillVnum) >= 40)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;948]");
										return false;
									}

									if (GetSkillLevel(dwSkillVnum) < iSkillLevelLowLimit)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;274]");
										return false;
									}

									if (GetSkillLevel(dwSkillVnum) >= iSkillLevelHighLimit)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;371]");
										return false;
									}

									if (LearnSkillByBook(dwSkillVnum, iPct))
									{
#ifdef ENABLE_BOOKS_STACKFIX
										item->SetCount(item->GetCount() - 1);
#else
										ITEM_MANAGER::instance().RemoveItem(item);
#endif

										int iReadDelay = number(SKILLBOOK_DELAY_MIN, SKILLBOOK_DELAY_MAX);

										SetSkillNextReadTime(dwSkillVnum, get_global_time() + iReadDelay);
									}
								}
								break;

							case 50902:
							case 50903:
							case 50904:
								{
									if (IsPolymorphed())
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;1041]");
										return false;

									}
									DWORD dwSkillVnum = SKILL_CREATE;
									int iPct = MINMAX(0, item->GetValue(1), 100);

									if (GetSkillLevel(dwSkillVnum)>=40)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;948]");
										return false;
									}

									if (LearnSkillByBook(dwSkillVnum, iPct))
									{
#ifdef ENABLE_BOOKS_STACKFIX
										item->SetCount(item->GetCount() - 1);
#else
										ITEM_MANAGER::instance().RemoveItem(item);
#endif

										int iReadDelay = number(SKILLBOOK_DELAY_MIN, SKILLBOOK_DELAY_MAX);

										SetSkillNextReadTime(dwSkillVnum, get_global_time() + iReadDelay);

										if (test_server)
										{
											ChatPacket(CHAT_TYPE_INFO, "[TEST_SERVER] Success to learn skill ");
										}
									}
									else
									{
										if (test_server)
										{
											ChatPacket(CHAT_TYPE_INFO, "[TEST_SERVER] Failed to learn skill ");
										}
									}
								}
								break;

								// MINING
							case ITEM_MINING_SKILL_TRAIN_BOOK:
								{
									if (IsPolymorphed())
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;1041]");
										return false;

									}
									DWORD dwSkillVnum = SKILL_MINING;
									int iPct = MINMAX(0, item->GetValue(1), 100);

									if (GetSkillLevel(dwSkillVnum)>=40)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;948]");
										return false;
									}

									if (LearnSkillByBook(dwSkillVnum, iPct))
									{
#ifdef ENABLE_BOOKS_STACKFIX
										item->SetCount(item->GetCount() - 1);
#else
										ITEM_MANAGER::instance().RemoveItem(item);
#endif

										int iReadDelay = number(SKILLBOOK_DELAY_MIN, SKILLBOOK_DELAY_MAX);

										SetSkillNextReadTime(dwSkillVnum, get_global_time() + iReadDelay);
									}
								}
								break;
								// END_OF_MINING

							case ITEM_HORSE_SKILL_TRAIN_BOOK:
								{
									if (IsPolymorphed())
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;1041]");
										return false;

									}
									DWORD dwSkillVnum = SKILL_HORSE;
									int iPct = MINMAX(0, item->GetValue(1), 100);

									if (GetLevel() < 50)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;376]");
										return false;
									}

									if (!test_server && get_global_time() < GetSkillNextReadTime(dwSkillVnum))
									{
										if (FindAffect(AFFECT_SKILL_NO_BOOK_DELAY))
										{
											// �־ȼ��� ����߿��� �ð� ���� ����
											RemoveAffect(AFFECT_SKILL_NO_BOOK_DELAY);
											ChatPacket(CHAT_TYPE_INFO, "[LS;377]");
										}
										else
										{
											SkillLearnWaitMoreTimeMessage(GetSkillNextReadTime(dwSkillVnum) - get_global_time());
											return false;
										}
									}

									if (GetPoint(POINT_HORSE_SKILL) >= 20 ||
											GetSkillLevel(SKILL_HORSE_WILDATTACK) + GetSkillLevel(SKILL_HORSE_CHARGE) + GetSkillLevel(SKILL_HORSE_ESCAPE) >= 60 ||
											GetSkillLevel(SKILL_HORSE_WILDATTACK_RANGE) + GetSkillLevel(SKILL_HORSE_CHARGE) + GetSkillLevel(SKILL_HORSE_ESCAPE) >= 60)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;378]");
										return false;
									}

									if (number(1, 100) <= iPct)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;379]");
										ChatPacket(CHAT_TYPE_INFO, "[LS;380]");
										PointChange(POINT_HORSE_SKILL, 1);

										int iReadDelay = number(SKILLBOOK_DELAY_MIN, SKILLBOOK_DELAY_MAX);

										if (!test_server)
											SetSkillNextReadTime(dwSkillVnum, get_global_time() + iReadDelay);
									}
									else
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;382]");
									}
#ifdef ENABLE_BOOKS_STACKFIX
									item->SetCount(item->GetCount() - 1);
#else
									ITEM_MANAGER::instance().RemoveItem(item);
#endif
								}
								break;

							case 70102: // ����
							case 70103: // ����
								{
									if (GetAlignment() >= 0)
										return false;

									int delta = MIN(-GetAlignment(), item->GetValue(0));

									sys_log(0, "%s ALIGNMENT ITEM %d", GetName(), delta);

									UpdateAlignment(delta);
									item->SetCount(item->GetCount() - 1);

									if (delta / 10 > 0)
									{
										ChatPacket(CHAT_TYPE_TALKING, "[LS;383]");
										ChatPacket(CHAT_TYPE_INFO, "[LS;384;%d]", delta/10);
									}
								}
								break;

							case 71107: // õ��������
							case 39032:
								{
									int val = item->GetValue(0);
									int interval = item->GetValue(1);
									quest::PC* pPC = quest::CQuestManager::instance().GetPC(GetPlayerID());
									int last_use_time = pPC->GetFlag("mythical_peach.last_use_time");
									
									if (!pPC)
                                        return false;

									if (get_global_time() - last_use_time < interval * 60 * 60)
									{
										if (test_server == false)
										{
											ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[#Unk]You cannot use this item yet."));
											return false;
										}
										else
										{
											ChatPacket(CHAT_TYPE_INFO, "[LS;1034]");
										}
									}

									if (GetAlignment() == 200000)
									{
										ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[#Unk]You already have your rank at the maximum."));
										return false;
									}

									if (200000 - GetAlignment() < val * 10)
									{
										val = (200000 - GetAlignment()) / 10;
									}

									int old_alignment = GetAlignment() / 10;

									UpdateAlignment(val*10);

									item->SetCount(item->GetCount()-1);
									pPC->SetFlag("mythical_peach.last_use_time", get_global_time());

									ChatPacket(CHAT_TYPE_TALKING, "[LS;383]");
									ChatPacket(CHAT_TYPE_INFO, "[LS;384;%d]", val);

									char buf[256 + 1];
									snprintf(buf, sizeof(buf), "%d %d", old_alignment, GetAlignment() / 10);
									LogManager::instance().CharLog(this, val, "MYTHICAL_PEACH", buf);
								}
								break;

							case 71109: // Ż����
							case 72719:
								{
									LPITEM item2;

									if (!IsValidItemPosition(DestCell) || !(item2 = GetItem(DestCell)))
										return false;

									if (item2->IsExchanging() || item2->IsEquipped()) // @fixme114
										return false;

									if (item2->GetSocketCount() == 0)
										return false;
									#ifdef PRODOMO_PIATRA_FIX
									 if (item2->IsEquipped())
										return false;
									#endif

									switch( item2->GetType() )
									{
										case ITEM_WEAPON:
											break;
										case ITEM_ARMOR:
											switch (item2->GetSubType())
											{
											case ARMOR_EAR:
											case ARMOR_WRIST:
											case ARMOR_NECK:
												ChatPacket(CHAT_TYPE_INFO, "[LS;1032]");
												return false;
											}
											break;

										default:
											return false;
									}

									std::stack<long> socket;

									for (int i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
										socket.push(item2->GetSocket(i));

									int idx = ITEM_SOCKET_MAX_NUM - 1;

									while (socket.size() > 0)
									{
										if (socket.top() > 2 && socket.top() != ITEM_BROKEN_METIN_VNUM)
											break;

										idx--;
										socket.pop();
									}

									if (socket.size() == 0)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;1032]");
										return false;
									}

									LPITEM pItemReward = AutoGiveItem(socket.top());

									if (pItemReward != NULL)
									{
										item2->SetSocket(idx, 1);

										char buf[256+1];
										snprintf(buf, sizeof(buf), "%s(%u) %s(%u)",
												item2->GetName(), item2->GetID(), pItemReward->GetName(), pItemReward->GetID());
										LogManager::instance().ItemLog(this, item, "USE_DETACHMENT_ONE", buf);

										item->SetCount(item->GetCount() - 1);
									}
								}
								break;

							case 70201:
							case 70202:
							case 70203:
							case 70204:
							case 70205:
							case 70206:
								{
									int flag = GetQuestFlag("fix_bug.colorante");
									
									if (get_global_time() - flag < 3)
									{
										ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You have to wait 3 seconds to use this item again."));
										return false;
									}

									if (GetPart(PART_HAIR) >= 1001)
										ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot dye or bleach your current Hairstyle."));
									else
									{
										quest::CQuestManager& q = quest::CQuestManager::instance();
										quest::PC* pPC = q.GetPC(GetPlayerID());

										if (pPC)
										{
											SetPart(PART_HAIR, item->GetVnum() - 70201);
											// item->SetCount(item->GetCount() - 1);
											UpdatePacket();
											SetQuestFlag("fix_bug.colorante", get_global_time());
										}
									}
								}
								break;

							case ITEM_NEW_YEAR_GREETING_VNUM:
								{
									DWORD dwBoxVnum = ITEM_NEW_YEAR_GREETING_VNUM;
									std::vector <DWORD> dwVnums;
									std::vector <DWORD> dwCounts;
									std::vector <LPITEM> item_gets;
									int count = 0;

									if (GiveItemFromSpecialItemGroup(dwBoxVnum, dwVnums, dwCounts, item_gets, count))
									{
										for (int i = 0; i < count; i++)
										{
											if (dwVnums[i] == CSpecialItemGroup::GOLD)
												ChatPacket(CHAT_TYPE_INFO, "[LS;1269;%d]", dwCounts[i]);
										}

										item->SetCount(item->GetCount() - 1);

									}
								}
								break;

							case ITEM_VALENTINE_ROSE:
							case ITEM_VALENTINE_CHOCOLATE:
								{
									DWORD dwBoxVnum = item->GetVnum();
									std::vector <DWORD> dwVnums;
									std::vector <DWORD> dwCounts;
									std::vector <LPITEM> item_gets(0);
									int count = 0;


									if (((item->GetVnum() == ITEM_VALENTINE_ROSE) && (SEX_MALE==GET_SEX(this))) ||
										((item->GetVnum() == ITEM_VALENTINE_CHOCOLATE) && (SEX_FEMALE==GET_SEX(this))))
									{
										// ������ �����ʾ� �� �� ����.
										ChatPacket(CHAT_TYPE_INFO, "[LS;387]");
										return false;
									}


									if (GiveItemFromSpecialItemGroup(dwBoxVnum, dwVnums, dwCounts, item_gets, count))
										item->SetCount(item->GetCount()-1);
								}
								break;

							case ITEM_WHITEDAY_CANDY:
							case ITEM_WHITEDAY_ROSE:
								{
									DWORD dwBoxVnum = item->GetVnum();
									std::vector <DWORD> dwVnums;
									std::vector <DWORD> dwCounts;
									std::vector <LPITEM> item_gets(0);
									int count = 0;


									if (((item->GetVnum() == ITEM_WHITEDAY_CANDY) && (SEX_MALE==GET_SEX(this))) ||
										((item->GetVnum() == ITEM_WHITEDAY_ROSE) && (SEX_FEMALE==GET_SEX(this))))
									{
										// ������ �����ʾ� �� �� ����.
										ChatPacket(CHAT_TYPE_INFO, "[LS;387]");
										return false;
									}


									if (GiveItemFromSpecialItemGroup(dwBoxVnum, dwVnums, dwCounts, item_gets, count))
										item->SetCount(item->GetCount()-1);
								}
								break;

							case 50011: // ��������
								{
									DWORD dwBoxVnum = 50011;
									std::vector <DWORD> dwVnums;
									std::vector <DWORD> dwCounts;
									std::vector <LPITEM> item_gets(0);
									int count = 0;

									if (GiveItemFromSpecialItemGroup(dwBoxVnum, dwVnums, dwCounts, item_gets, count))
									{
										for (int i = 0; i < count; i++)
										{
											char buf[50 + 1];
											snprintf(buf, sizeof(buf), "%u %u", dwVnums[i], dwCounts[i]);
											LogManager::instance().ItemLog(this, item, "MOONLIGHT_GET", buf);

											//ITEM_MANAGER::instance().RemoveItem(item);
											item->SetCount(item->GetCount() - 1);

											switch (dwVnums[i])
											{
											case CSpecialItemGroup::GOLD:
												ChatPacket(CHAT_TYPE_INFO, "[LS;1269;%d]", dwCounts[i]);
												break;

											case CSpecialItemGroup::EXP:
												ChatPacket(CHAT_TYPE_INFO, "[LS;1279]");
												ChatPacket(CHAT_TYPE_INFO, "[LS;1290;%d]", dwCounts[i]);
												break;

											case CSpecialItemGroup::MOB:
												ChatPacket(CHAT_TYPE_INFO, "[LS;1299]");
												break;

											case CSpecialItemGroup::SLOW:
												ChatPacket(CHAT_TYPE_INFO, "[LS;1310]");
												break;

											case CSpecialItemGroup::DRAIN_HP:
												ChatPacket(CHAT_TYPE_INFO, "[LS;3]");
												break;

											case CSpecialItemGroup::POISON:
												ChatPacket(CHAT_TYPE_INFO, "[LS;13]");
												break;

											case CSpecialItemGroup::MOB_GROUP:
												ChatPacket(CHAT_TYPE_INFO, "[LS;1299]");
												break;

											default:
												if (item_gets[i])
												{
													if (dwCounts[i] > 1)
														ChatPacket(CHAT_TYPE_INFO, "[LS;204;%s;%d]", item_gets[i]->GetName(), dwCounts[i]);
													else
														ChatPacket(CHAT_TYPE_INFO, "[LS;35;%s]", item_gets[i]->GetName());
												}
												break;
											}
										}
									}
									else
									{
										ChatPacket(CHAT_TYPE_TALKING, "[LS;56]");
										return false;
									}
								}
								break;

							case ITEM_GIVE_STAT_RESET_COUNT_VNUM:
								{
									PointChange(POINT_STAT_RESET_COUNT, 1);
									item->SetCount(item->GetCount()-1);
								}
								break;

							case 50107:
								{
									if (CArenaManager::instance().IsArenaMap(GetMapIndex()) == true)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
										return false;
									}
#ifdef ENABLE_NEWSTUFF
									else if (g_NoPotionsOnPVP && CPVPManager::instance().IsFighting(GetPlayerID()) && !IsAllowedPotionOnPVP(item->GetVnum()))
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
										return false;
									}
#endif

									EffectPacket(SE_CHINA_FIREWORK);
#ifdef ENABLE_FIREWORK_STUN
									// ���� ������ �÷��ش�
									AddAffect(AFFECT_CHINA_FIREWORK, POINT_STUN_PCT, 30, AFF_CHINA_FIREWORK, 5*60, 0, true);
#endif
									item->SetCount(item->GetCount()-1);
								}
								break;

							case 50108:
								{
									if (CArenaManager::instance().IsArenaMap(GetMapIndex()) == true)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
										return false;
									}
#ifdef ENABLE_NEWSTUFF
									else if (g_NoPotionsOnPVP && CPVPManager::instance().IsFighting(GetPlayerID()) && !IsAllowedPotionOnPVP(item->GetVnum()))
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
										return false;
									}
#endif

									EffectPacket(SE_SPIN_TOP);
#ifdef ENABLE_FIREWORK_STUN
									// ���� ������ �÷��ش�
									AddAffect(AFFECT_CHINA_FIREWORK, POINT_STUN_PCT, 30, AFF_CHINA_FIREWORK, 5*60, 0, true);
#endif
									item->SetCount(item->GetCount()-1);
								}
								break;

							case ITEM_WONSO_BEAN_VNUM:
								PointChange(POINT_HP, GetMaxHP() - GetHP());
								item->SetCount(item->GetCount()-1);
								break;

							case ITEM_WONSO_SUGAR_VNUM:
								PointChange(POINT_SP, GetMaxSP() - GetSP());
								item->SetCount(item->GetCount()-1);
								break;

							case ITEM_WONSO_FRUIT_VNUM:
								PointChange(POINT_STAMINA, GetMaxStamina()-GetStamina());
								item->SetCount(item->GetCount()-1);
								break;

							case ITEM_ELK_VNUM: // ���ٷ���
								{
									int iGold = item->GetSocket(0);
									ITEM_MANAGER::instance().RemoveItem(item);
									ChatPacket(CHAT_TYPE_INFO, "[LS;1269;%d]", iGold);
#ifdef ENABLE_REMOVE_LIMIT_GOLD
									ChangeGold(iGold);
#else
									PointChange(POINT_GOLD, iGold);
#endif
								}
								break;


								//������ ��ǥ
							case 70021:
								{
									int HealPrice = quest::CQuestManager::instance().GetEventFlag("MonarchHealGold");
									if (HealPrice == 0)
										HealPrice = 2000000;

									if (CMonarch::instance().HealMyEmpire(this, HealPrice))
									{
										char szNotice[256];
										snprintf(szNotice, sizeof(szNotice), LC_TEXT("[559]When the Blessing of the Emperor is used %s the HP and SP are restored again."), EMPIRE_NAME(GetEmpire()));
										SendNoticeMap(szNotice, GetMapIndex(), false);

										ChatPacket(CHAT_TYPE_INFO, "[LS;570]");
									}
								}
								break;

							case 27995:
								{
								}
								break;

							case 71092 : // ���� ��ü�� �ӽ�
								{
									if (m_pkChrTarget != NULL)
									{
										if (m_pkChrTarget->IsPolymorphed())
										{
											m_pkChrTarget->SetPolymorph(0);
											m_pkChrTarget->RemoveAffect(AFFECT_POLYMORPH);
										}
									}
									else
									{
										if (IsPolymorphed())
										{
											SetPolymorph(0);
											RemoveAffect(AFFECT_POLYMORPH);
										}
									}
								}
								break;

							case 71051 : // ���簡
								{
									// ����, �̰���, ��Ʈ�� ���簡 ������
									LPITEM item2;

									if (!IsValidItemPosition(DestCell) || !(item2 = GetInventoryItem(wDestCell)))
										return false;

									if (ITEM_COSTUME == item2->GetType()
#ifdef ENABLE_GLOVE_SYSTEM	
										|| (ITEM_ARMOR == item2->GetType() && ARMOR_GLOVE == item2->GetSubType())
#endif	
										) // @fixme124
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;396]");
										return false;
									}

									if (item2->IsExchanging() || item2->IsEquipped()) // @fixme114
										return false;

									if (item2->GetAttributeSetIndex() == -1)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;396]");
										return false;
									}

									if (item2->AddRareAttribute() == true)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;400]");

										int iAddedIdx = item2->GetRareAttrCount() + 4;
										char buf[21];
										snprintf(buf, sizeof(buf), "%u", item2->GetID());

										LogManager::instance().ItemLog(
												GetPlayerID(),
												item2->GetAttributeType(iAddedIdx),
												item2->GetAttributeValue(iAddedIdx),
												item->GetID(),
												"ADD_RARE_ATTR",
												buf,
												GetDesc()->GetHostName(),
												item->GetOriginalVnum());

										item->SetCount(item->GetCount() - 1);
									}
									else
									{
										ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[#Unk]You can no longer add enhancements to this item."));
									}
								}
								break;

							case 71052 : // �����
								{
									// ����, �̰���, ��Ʈ�� ���簡 ������
									LPITEM item2;

									if (!IsValidItemPosition(DestCell) || !(item2 = GetItem(DestCell)))
										return false;

									if (ITEM_COSTUME == item2->GetType()
#ifdef ENABLE_GLOVE_SYSTEM	
										|| (ITEM_ARMOR == item2->GetType() && ARMOR_GLOVE == item2->GetSubType())
#endif	
										) // @fixme124
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;396]");
										return false;
									}

									if (item2->IsExchanging() || item2->IsEquipped()) // @fixme114
										return false;

									if (item2->GetAttributeSetIndex() == -1)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;396]");
										return false;
									}

									if (item2->ChangeRareAttribute() == true)
									{
										char buf[21];
										snprintf(buf, sizeof(buf), "%u", item2->GetID());
										LogManager::instance().ItemLog(this, item, "CHANGE_RARE_ATTR", buf);

										item->SetCount(item->GetCount() - 1);
									}
									else
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;397]");
									}
								}
								break;

							case ITEM_AUTO_HP_RECOVERY_S:
							case ITEM_AUTO_HP_RECOVERY_M:
							case ITEM_AUTO_HP_RECOVERY_L:
							case ITEM_AUTO_HP_RECOVERY_X:
							case ITEM_AUTO_SP_RECOVERY_S:
							case ITEM_AUTO_SP_RECOVERY_M:
							case ITEM_AUTO_SP_RECOVERY_L:
							case ITEM_AUTO_SP_RECOVERY_X:
							case REWARD_BOX_ITEM_AUTO_SP_RECOVERY_XS:
							case REWARD_BOX_ITEM_AUTO_SP_RECOVERY_S:
							case REWARD_BOX_ITEM_AUTO_HP_RECOVERY_XS:
							case REWARD_BOX_ITEM_AUTO_HP_RECOVERY_S:
								{
									if (CArenaManager::instance().IsArenaMap(GetMapIndex()) == true)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;403]");
										return false;
									}
#ifdef ENABLE_NEWSTUFF
									else if (g_NoPotionsOnPVP && CPVPManager::instance().IsFighting(GetPlayerID()) && !IsAllowedPotionOnPVP(item->GetVnum()))
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
										return false;
									}
#endif

									EAffectTypes type = AFFECT_NONE;

									{
										quest::CQuestManager& q = quest::CQuestManager::instance();
										quest::PC* pPC = q.GetPC(GetPlayerID());
									
										if (pPC != NULL)
										{
											int last_use_time = pPC->GetFlag("auto_recovery.last_use_time");
									
											if (get_global_time() - last_use_time < 1)
											{
												ChatPacket(CHAT_TYPE_INFO, LC_TEXT("WAIT_BUFF"), 1 - (get_global_time() - last_use_time));
												return false;
											}
									
											pPC->SetFlag("auto_recovery.last_use_time", get_global_time());
										}
									}	

									bool isSpecialPotion = false;

									switch (item->GetVnum())
									{
										case ITEM_AUTO_HP_RECOVERY_X:
											isSpecialPotion = true;

										case ITEM_AUTO_HP_RECOVERY_S:
										case ITEM_AUTO_HP_RECOVERY_M:
										case ITEM_AUTO_HP_RECOVERY_L:
										case REWARD_BOX_ITEM_AUTO_HP_RECOVERY_XS:
										case REWARD_BOX_ITEM_AUTO_HP_RECOVERY_S:
											type = AFFECT_AUTO_HP_RECOVERY;
											break;

										case ITEM_AUTO_SP_RECOVERY_X:
											isSpecialPotion = true;

										case ITEM_AUTO_SP_RECOVERY_S:
										case ITEM_AUTO_SP_RECOVERY_M:
										case ITEM_AUTO_SP_RECOVERY_L:
										case REWARD_BOX_ITEM_AUTO_SP_RECOVERY_XS:
										case REWARD_BOX_ITEM_AUTO_SP_RECOVERY_S:
											type = AFFECT_AUTO_SP_RECOVERY;
											break;
									}

									if (AFFECT_NONE == type)
										break;

									if (item->GetCount() > 1)
									{
										int pos = GetEmptyInventory(item->GetSize());

										if (-1 == pos)
										{
											ChatPacket(CHAT_TYPE_INFO, "[LS;1130]");
											break;
										}

										item->SetCount( item->GetCount() - 1 );

										LPITEM item2 = ITEM_MANAGER::instance().CreateItem( item->GetVnum(), 1 );
										item2->AddToCharacter(this, TItemPos(INVENTORY, pos));

										if (item->GetSocket(1) != 0)
										{
											item2->SetSocket(1, item->GetSocket(1));
										}

										item = item2;
									}

									CAffect* pAffect = FindAffect( type );

									if (NULL == pAffect)
									{
										EPointTypes bonus = POINT_NONE;

										if (true == isSpecialPotion)
										{
											if (type == AFFECT_AUTO_HP_RECOVERY)
											{
												bonus = POINT_MAX_HP_PCT;
											}
											else if (type == AFFECT_AUTO_SP_RECOVERY)
											{
												bonus = POINT_MAX_SP_PCT;
											}
										}

										AddAffect( type, bonus, 4, item->GetID(), INFINITE_AFFECT_DURATION, 0, true, false);

										item->Lock(true);
										item->SetSocket(0, true);

										AutoRecoveryItemProcess( type );
									}
									else
									{
										if (item->GetID() == pAffect->dwFlag)
										{
											RemoveAffect( pAffect );

											item->Lock(false);
											item->SetSocket(0, false);
										}
										else
										{
											LPITEM old = FindItemByID( pAffect->dwFlag );

											if (NULL != old)
											{
												old->Lock(false);
												old->SetSocket(0, false);
											}

											RemoveAffect( pAffect );

											EPointTypes bonus = POINT_NONE;

											if (true == isSpecialPotion)
											{
												if (type == AFFECT_AUTO_HP_RECOVERY)
												{
													bonus = POINT_MAX_HP_PCT;
												}
												else if (type == AFFECT_AUTO_SP_RECOVERY)
												{
													bonus = POINT_MAX_SP_PCT;
												}
											}

											AddAffect( type, bonus, 4, item->GetID(), INFINITE_AFFECT_DURATION, 0, true, false);

											item->Lock(true);
											item->SetSocket(0, true);

											AutoRecoveryItemProcess( type );
										}
									}
								}
								break;
#ifdef __AURA_SYSTEM__
							case ITEM_AURA_BOOST_ITEM_VNUM_BASE + ITEM_AURA_BOOST_ERASER:
								{
									LPITEM item2;
									if (!IsValidItemPosition(DestCell) || !(item2 = GetItem(DestCell)))
										return false;

									if (item2->IsExchanging() || item2->IsEquipped())
										return false;

#ifdef ENABLE_SEALBIND_SYSTEM
									if (item2->IsBound())
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;1468]");
										return false;
									}
#endif

									if (item2->GetSocket(ITEM_SOCKET_AURA_BOOST) == 0)
									{
										ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[Aura] There is no boost in your aura costume."));
										return false;
									}

									if (IS_SET(item->GetFlag(), ITEM_FLAG_STACKABLE) && !IS_SET(item->GetAntiFlag(), ITEM_ANTIFLAG_STACK) && item->GetCount() > 1)
										item->SetCount(item->GetCount() - 1);
									else
										ITEM_MANAGER::instance().RemoveItem(item);

									item2->SetSocket(ITEM_SOCKET_AURA_BOOST, 0);
								}
								break;
#endif
						}
						break;

					case USE_CLEAR:
						{
							switch (item->GetVnum())
							{

								case 27874: // Grilled Perch
								default:
									RemoveBadAffect();
									break;
							}
							item->SetCount(item->GetCount() - 1);
						}
						break;

					case USE_INVISIBILITY:
						{
							if (item->GetVnum() == 70026)
							{
								quest::CQuestManager& q = quest::CQuestManager::instance();
								quest::PC* pPC = q.GetPC(GetPlayerID());

								if (pPC != NULL)
								{
									int last_use_time = pPC->GetFlag("mirror_of_disapper.last_use_time");

									if (get_global_time() - last_use_time < 10*60)
									{
										ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[#Unk]You cannot use this item yet."));
										return false;
									}

									pPC->SetFlag("mirror_of_disapper.last_use_time", get_global_time());
								}
							}

							AddAffect(AFFECT_INVISIBILITY, POINT_NONE, 0, AFF_INVISIBILITY, 300, 0, true);
							item->SetCount(item->GetCount() - 1);
						}
						break;

					case USE_POTION_NODELAY:
						{
							if (CArenaManager::instance().IsArenaMap(GetMapIndex()) == true)
							{
								if (quest::CQuestManager::instance().GetEventFlag("arena_potion_limit") > 0)
								{
									ChatPacket(CHAT_TYPE_INFO, "[LS;403]");
									return false;
								}

								switch (item->GetVnum())
								{
									case 70020 :
									case 71018 :
									case 71019 :
									case 71020 :
										if (quest::CQuestManager::instance().GetEventFlag("arena_potion_limit_count") < 10000)
										{
											if (m_nPotionLimit <= 0)
											{
												ChatPacket(CHAT_TYPE_INFO, "[LS;122]");
												return false;
											}
										}
										break;

									default :
										ChatPacket(CHAT_TYPE_INFO, "[LS;403]");
										return false;
								}
							}
#ifdef ENABLE_NEWSTUFF
							else if (g_NoPotionsOnPVP && CPVPManager::instance().IsFighting(GetPlayerID()) && !IsAllowedPotionOnPVP(item->GetVnum()))
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
								return false;
							}
#endif

							bool used = false;

							if (item->GetValue(0) != 0) // HP ���밪 ȸ��
							{
								if (GetHP() < GetMaxHP())
								{
									PointChange(POINT_HP, item->GetValue(0) * (100 + GetPoint(POINT_POTION_BONUS)) / 100);
									EffectPacket(SE_HPUP_RED);
									used = TRUE;
								}
							}

							if (item->GetValue(1) != 0)	// SP ���밪 ȸ��
							{
								if (GetSP() < GetMaxSP())
								{
									PointChange(POINT_SP, item->GetValue(1) * (100 + GetPoint(POINT_POTION_BONUS)) / 100);
									EffectPacket(SE_SPUP_BLUE);
									used = TRUE;
								}
							}

							if (item->GetValue(3) != 0) // HP % ȸ��
							{
								if (GetHP() < GetMaxHP())
								{
									PointChange(POINT_HP, item->GetValue(3) * GetMaxHP() / 100);
									EffectPacket(SE_HPUP_RED);
									used = TRUE;
								}
							}

							if (item->GetValue(4) != 0) // SP % ȸ��
							{
								if (GetSP() < GetMaxSP())
								{
									PointChange(POINT_SP, item->GetValue(4) * GetMaxSP() / 100);
									EffectPacket(SE_SPUP_BLUE);
									used = TRUE;
								}
							}

							if (used)
							{
								if (item->GetVnum() == 50085 || item->GetVnum() == 50086)
								{
									if (test_server)
										ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Used moon cake or seeds."));
									SetUseSeedOrMoonBottleTime();
								}
								if (GetDungeon())
									GetDungeon()->UsePotion(this);

								if (GetWarMap())
									GetWarMap()->UsePotion(this, item);

								m_nPotionLimit--;

								//RESTRICT_USE_SEED_OR_MOONBOTTLE
								item->SetCount(item->GetCount() - 1);
								//END_RESTRICT_USE_SEED_OR_MOONBOTTLE
							}
						}
						break;

					case USE_POTION:
						if (CArenaManager::instance().IsArenaMap(GetMapIndex()) == true)
						{
							if (quest::CQuestManager::instance().GetEventFlag("arena_potion_limit") > 0)
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;403]");
								return false;
							}

							switch (item->GetVnum())
							{
								case 27001 :
								case 27002 :
								case 27003 :
								case 27004 :
								case 27005 :
								case 27006 :
									if (quest::CQuestManager::instance().GetEventFlag("arena_potion_limit_count") < 10000)
									{
										if (m_nPotionLimit <= 0)
										{
											ChatPacket(CHAT_TYPE_INFO, "[LS;122]");
											return false;
										}
									}
									break;

								default :
									ChatPacket(CHAT_TYPE_INFO, "[LS;403]");
									return false;
							}
						}
#ifdef ENABLE_NEWSTUFF
						else if (g_NoPotionsOnPVP && CPVPManager::instance().IsFighting(GetPlayerID()) && !IsAllowedPotionOnPVP(item->GetVnum()))
						{
							ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
							return false;
						}
#endif

						if (item->GetValue(1) != 0)
						{
							if (GetPoint(POINT_SP_RECOVERY) + GetSP() >= GetMaxSP())
							{
								return false;
							}

							PointChange(POINT_SP_RECOVERY, item->GetValue(1) * MIN(200, (100 + GetPoint(POINT_POTION_BONUS))) / 100);
							StartAffectEvent();
							EffectPacket(SE_SPUP_BLUE);
						}

						if (item->GetValue(0) != 0)
						{
							if (GetPoint(POINT_HP_RECOVERY) + GetHP() >= GetMaxHP())
							{
								return false;
							}

							PointChange(POINT_HP_RECOVERY, item->GetValue(0) * MIN(200, (100 + GetPoint(POINT_POTION_BONUS))) / 100);
							StartAffectEvent();
							EffectPacket(SE_HPUP_RED);
						}

						if (GetDungeon())
							GetDungeon()->UsePotion(this);

						if (GetWarMap())
							GetWarMap()->UsePotion(this, item);

						item->SetCount(item->GetCount() - 1);
						m_nPotionLimit--;
						break;

					case USE_POTION_CONTINUE:
						{
							if (item->GetValue(0) != 0)
							{
								AddAffect(AFFECT_HP_RECOVER_CONTINUE, POINT_HP_RECOVER_CONTINUE, item->GetValue(0), 0, item->GetValue(2), 0, true);
							}
							else if (item->GetValue(1) != 0)
							{
								AddAffect(AFFECT_SP_RECOVER_CONTINUE, POINT_SP_RECOVER_CONTINUE, item->GetValue(1), 0, item->GetValue(2), 0, true);
							}
							else
								return false;
						}

						if (GetDungeon())
							GetDungeon()->UsePotion(this);

						if (GetWarMap())
							GetWarMap()->UsePotion(this, item);

						item->SetCount(item->GetCount() - 1);
						break;

					case USE_ABILITY_UP:
						{
							switch (item->GetValue(0))
							{
								case APPLY_MOV_SPEED:
#ifdef ENABLE_AFFECT_RENEWAL
									if (FindAffect(AFFECT_MOVE_SPEED))
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
										return false;
									}
#endif
									if (FindAffect(AFFECT_MOV_SPEED))
									{
										ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Folosesti deja licoare violet"));
										return false;
									}
									AddAffect(AFFECT_MOV_SPEED, POINT_MOV_SPEED, item->GetValue(2), AFF_MOV_SPEED_POTION, item->GetValue(1), 0, true);
#ifdef ENABLE_EFFECT_EXTRAPOT
									EffectPacket(SE_DXUP_PURPLE);
#endif
									break;

								case APPLY_ATT_SPEED:
#ifdef ENABLE_AFFECT_RENEWAL
									if (FindAffect(AFFECT_ATTACK_SPEED))
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
										return false;
									}
#endif
									if (FindAffect(AFFECT_ATT_SPEED))
									{
										ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Folosesti deja licoare verde"));
										return false;
									}
									AddAffect(AFFECT_ATT_SPEED, POINT_ATT_SPEED, item->GetValue(2), AFF_ATT_SPEED_POTION, item->GetValue(1), 0, true);
#ifdef ENABLE_EFFECT_EXTRAPOT
									EffectPacket(SE_SPEEDUP_GREEN);
#endif
									break;

								case APPLY_STR:
									AddAffect(AFFECT_STR, POINT_ST, item->GetValue(2), 0, item->GetValue(1), 0, true);
									break;

								case APPLY_DEX:
									AddAffect(AFFECT_DEX, POINT_DX, item->GetValue(2), 0, item->GetValue(1), 0, true);
									break;

								case APPLY_CON:
									AddAffect(AFFECT_CON, POINT_HT, item->GetValue(2), 0, item->GetValue(1), 0, true);
									break;

								case APPLY_INT:
									AddAffect(AFFECT_INT, POINT_IQ, item->GetValue(2), 0, item->GetValue(1), 0, true);
									break;

								case APPLY_CAST_SPEED:
									AddAffect(AFFECT_CAST_SPEED, POINT_CASTING_SPEED, item->GetValue(2), 0, item->GetValue(1), 0, true);
									break;

								case APPLY_ATT_GRADE_BONUS:
									AddAffect(AFFECT_ATT_GRADE, POINT_ATT_GRADE_BONUS, item->GetValue(2), 0, item->GetValue(1), 0, true);
									break;

								case APPLY_DEF_GRADE_BONUS:
									AddAffect(AFFECT_DEF_GRADE, POINT_DEF_GRADE_BONUS,
											item->GetValue(2), 0, item->GetValue(1), 0, true);
									break;
							}
						}

						if (GetDungeon())
							GetDungeon()->UsePotion(this);

						if (GetWarMap())
							GetWarMap()->UsePotion(this, item);

						item->SetCount(item->GetCount() - 1);
						break;

					case USE_TALISMAN:
						{
							const int TOWN_PORTAL	= 1;
							const int MEMORY_PORTAL = 2;


							// gm_guild_build, oxevent �ʿ��� ��ȯ�� ��ȯ���� �� �����ϰ� ����
							if (GetMapIndex() == 200 || GetMapIndex() == 113)
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;388]");
								return false;
							}

							if (CArenaManager::instance().IsArenaMap(GetMapIndex()) == true)
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
								return false;
							}
#ifdef ENABLE_NEWSTUFF
							else if (g_NoPotionsOnPVP && CPVPManager::instance().IsFighting(GetPlayerID()) && !IsAllowedPotionOnPVP(item->GetVnum()))
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;1205]");
								return false;
							}
#endif

							if (m_pkWarpEvent)
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;389]");
								return false;
							}

							// CONSUME_LIFE_WHEN_USE_WARP_ITEM
							int consumeLife = CalculateConsume(this);

							if (consumeLife < 0)
								return false;
							// END_OF_CONSUME_LIFE_WHEN_USE_WARP_ITEM

							if (item->GetValue(0) == TOWN_PORTAL) // ��ȯ��
							{
								if (item->GetSocket(0) == 0)
								{
									if (!GetDungeon())
										if (!GiveRecallItem(item))
											return false;

									PIXEL_POSITION posWarp;

									if (SECTREE_MANAGER::instance().GetRecallPositionByEmpire(GetMapIndex(), GetEmpire(), posWarp))
									{
										// CONSUME_LIFE_WHEN_USE_WARP_ITEM
										PointChange(POINT_HP, -consumeLife, false);
										// END_OF_CONSUME_LIFE_WHEN_USE_WARP_ITEM

										WarpSet(posWarp.x, posWarp.y);
									}
									else
									{
										sys_err("CHARACTER::UseItem : cannot find spawn position (name %s, %d x %d)", GetName(), GetX(), GetY());
									}
								}
								else
								{
									if (test_server)
										ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You will be brought back to the original place."));

									ProcessRecallItem(item);
								}
							}
							else if (item->GetValue(0) == MEMORY_PORTAL) // ��ȯ����
							{
								if (item->GetSocket(0) == 0)
								{
									if (GetDungeon())
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;391;%s;%s]",
												item->GetName(),
												"");
										return false;
									}

									if (!GiveRecallItem(item))
										return false;
								}
								else
								{
									// CONSUME_LIFE_WHEN_USE_WARP_ITEM
									PointChange(POINT_HP, -consumeLife, false);
									// END_OF_CONSUME_LIFE_WHEN_USE_WARP_ITEM

									ProcessRecallItem(item);
								}
							}
						}
						break;

					case USE_TUNING:
					case USE_DETACHMENT:
						{
							LPITEM item2;

							if (!IsValidItemPosition(DestCell) || !(item2 = GetItem(DestCell)))
								return false;

							if (item2->IsExchanging() || item2->IsEquipped()) // @fixme114
								return false;

#ifdef __SASH_SYSTEM__
							if (item->GetValue(0) == SASH_CLEAN_ATTR_VALUE0)
							{
								if (!CleanSashAttr(item, item2))
									return false;
								
								return true;
							}
#endif

#ifdef __CHANGELOOK_SYSTEM__
							if (item->GetValue(0) == CL_CLEAN_ATTR_VALUE0)
							{
								if (!CleanTransmutation(item, item2))
									return false;
								
								return true;
							}
#endif
							#ifdef PRODOMO_PIATRA_FIX
							if (item2->IsEquipped())
                                return false;
							#endif
							if (item2->GetVnum() >= 28330 && item2->GetVnum() <= 28343) // ����+3
							{
								ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[#Unk] Stones +3 cannot be improved with this item."));
								return false;
							}

							if (item2->GetVnum() >= 28430 && item2->GetVnum() <= 28443)  // ����+4
							{
								if (item->GetVnum() == 71056) // û���Ǽ���
								{
									RefineItem(item, item2);
								}
								else
								{
									ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Stones cannot be improved with  this item."));
								}
							}
							else
							{
								RefineItem(item, item2);
							}
						}
						break;

					case USE_CHANGE_COSTUME_ATTR:
					case USE_RESET_COSTUME_ATTR:
						{
							LPITEM item2;
							if (!IsValidItemPosition(DestCell) || !(item2 = GetItem(DestCell)))
								return false;

							if (item2->IsEquipped())
							{
								BuffOnAttr_RemoveBuffsFromItem(item2);
							}

							if (ITEM_COSTUME != item2->GetType())
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;396]");
								return false;
							}

							if (item2->IsExchanging() || item2->IsEquipped()) // @fixme114
								return false;

							if (item2->GetAttributeSetIndex() == -1)
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;396]");
								return false;
							}

							if (item2->GetAttributeCount() == 0)
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;397]");
								return false;
							}

							switch (item->GetSubType())
							{
								case USE_CHANGE_COSTUME_ATTR:
									item2->ChangeAttribute();
									{
										char buf[21];
										snprintf(buf, sizeof(buf), "%u", item2->GetID());
										LogManager::instance().ItemLog(this, item, "CHANGE_COSTUME_ATTR", buf);
									}
									break;
								case USE_RESET_COSTUME_ATTR:
									item2->ClearAttribute();
									item2->AlterToMagicItem();
									{
										char buf[21];
										snprintf(buf, sizeof(buf), "%u", item2->GetID());
										LogManager::instance().ItemLog(this, item, "RESET_COSTUME_ATTR", buf);
									}
									break;
							}

							ChatPacket(CHAT_TYPE_INFO, "[LS;399]");

							item->SetCount(item->GetCount() - 1);
							break;
						}
#ifdef ENABLE_GLOVE_SYSTEM
				    case USE_ADD_ATTRIBUTE_GLOVE:
				    case USE_CHANGE_ATTRIBUTE_GLOVE:{
				    	CItem* targetItem;
				    	if (!IsValidItemPosition(DestCell) ||
				    		!(targetItem = GetItem(DestCell)))
				    		return false;
				    
				    	switch (item->GetSubType()) {
				    		case USE_ADD_ATTRIBUTE_GLOVE:
				    			if (!UseItemGloveAddAttribute(*item,
				    												*targetItem))
				    				return false;
				    			break;
				    		case USE_CHANGE_ATTRIBUTE_GLOVE:
				    			if (!UseItemGloveChangeAttribute(*item,
				    												*targetItem))
				    				return false;
				    			break;
				    	}
				    }
				    break;
#endif

				    case USE_ADD_ATTRIBUTE_TALISMAN:
				    case USE_CHANGE_ATTRIBUTE_TALISMAN:{
				    	CItem* targetItem;
				    	if (!IsValidItemPosition(DestCell) ||
				    		!(targetItem = GetItem(DestCell)))
				    		return false;
				    
				    	switch (item->GetSubType()) {
				    		case USE_ADD_ATTRIBUTE_TALISMAN:
				    			if (!UseItemTalismanAddAttribute(*item,
				    												*targetItem))
				    				return false;
				    			break;
				    		case USE_CHANGE_ATTRIBUTE_TALISMAN:
				    			if (!UseItemTalismanChangeAttribute(*item,
				    												*targetItem))
				    				return false;
				    			break;
				    	}
				    }
				    break;
					
#ifdef NEW_ATTR_RANFORSARI
					case USE_SET_ATT_COSTUME :
					{
						LPITEM item2;
						if (!IsValidItemPosition(DestCell) || !(item2 = GetItem(DestCell)))
							return false;
 
						if (item2->IsEquipped())
						{
							BuffOnAttr_RemoveBuffsFromItem(item2);
						}
 
						if (item2->IsExchanging() || item2->IsEquipped()) // @fixme114
							return false;
 
						if ((item2->GetType() == ITEM_COSTUME) && (item2->GetSubType() == COSTUME_BODY))
						{
							if (item2->GetAttributeCount() < 2)
							{
								if (item2->HasAttr(item->GetValue(0)))
								{
									ChatPacket(CHAT_TYPE_INFO, "[i] Nu poti adauga acelasi bonus de 2 ori.");
									return false;
								}
								item2->AddAttribute(item->GetValue(0), item->GetValue(1));
								ChatPacket(CHAT_TYPE_INFO, "[i] Ranforsarea a fost adaugata cu succes.");
 
								item->SetCount(item->GetCount() - 1);
							}
							else
							{
								ChatPacket(CHAT_TYPE_INFO, "[i] Poti adauga doar 2 ranforsari pe costum!");
							}
						}
						else
						{
							ChatPacket(CHAT_TYPE_INFO, "[i] Ranforsarea merge adaugata doar pe costum.");
						}
					}
					break;
					
					case USE_SET_ATT_COSTUME_WEAPON:
					{
						LPITEM item2;
						if (!IsValidItemPosition(DestCell) || !(item2 = GetItem(DestCell)))
							return false;
 
						if (item2->IsEquipped())
						{
							BuffOnAttr_RemoveBuffsFromItem(item2);
						}
 
						if (item2->IsExchanging() || item2->IsEquipped()) // @fixme114
							return false;
 
						if ((item2->GetType() == ITEM_COSTUME) && (item2->GetSubType() == COSTUME_WEAPON))
						{
							if (item2->GetAttributeCount() < 2)
							{
								if (item2->HasAttr(item->GetValue(0)))
								{
									ChatPacket(CHAT_TYPE_INFO, "[i] Nu poti adauga acelasi bonus de 2 ori.");
									return false;
								}
								item2->AddAttribute(item->GetValue(0), item->GetValue(1));
								ChatPacket(CHAT_TYPE_INFO, "[i] Ranforsarea a fost adaugata cu succes.");
 
								item->SetCount(item->GetCount() - 1);
							}
							else
							{
								ChatPacket(CHAT_TYPE_INFO, "[i] Poti adauga doar 2 ranforsari pe costum!");
							}
						}
						else
						{
							ChatPacket(CHAT_TYPE_INFO, "[i] Ranforsarea merge adaugata doar pe costum.");
						}
					}
					break;
					
					case USE_SET_ATT_PET :
					{
						LPITEM item2;
						if (!IsValidItemPosition(DestCell) || !(item2 = GetItem(DestCell)))
							return false;
 
						if (item2->IsEquipped())
						{
							BuffOnAttr_RemoveBuffsFromItem(item2);
						}
 
						if (item2->IsExchanging() || item2->IsEquipped()) // @fixme114
							return false;
 
						if ((item2->GetType() == ITEM_COSTUME) && (item2->GetSubType() == COSTUME_PET))
						{
							if (item2->GetAttributeCount() < 2)
							{
								if (item2->HasAttr(item->GetValue(0)))
								{
									ChatPacket(CHAT_TYPE_INFO, "[i]Nu poti adauga de 2 ori acelasi bonus.");
									return false;
								}
								item2->AddAttribute(item->GetValue(0), item->GetValue(1));
								ChatPacket(CHAT_TYPE_INFO, "[i] Ranforsarea a fost adaugata cu succes!");
 
								item->SetCount(item->GetCount() - 1);
							}
							else
							{
								ChatPacket(CHAT_TYPE_INFO, "[i] Poti adauga doar 2 ranforsari pe pet.");
							}
						}
						else
						{
							ChatPacket(CHAT_TYPE_INFO, "[i] Aceasta ranforsare merge adaugata doar pe pet.");
						}
					}
					break;	



					case USE_SET_ATT_MOUNT :
					{
						LPITEM item2;
						if (!IsValidItemPosition(DestCell) || !(item2 = GetItem(DestCell)))
							return false;
 
						if (item2->IsEquipped())
						{
							BuffOnAttr_RemoveBuffsFromItem(item2);
						}
 
						if (item2->IsExchanging() || item2->IsEquipped()) // @fixme114
							return false;
 
						if ((item2->GetType() == ITEM_COSTUME) && (item2->GetSubType() == COSTUME_MOUNT))
						{
							if (item2->GetAttributeCount() < 2)
							{
								if (item2->HasAttr(item->GetValue(0)))
								{
									ChatPacket(CHAT_TYPE_INFO, "[i]Nu poti adauga aceasi ranforsare de 2 ori.");
									return false;
								}
								item2->AddAttribute(item->GetValue(0), item->GetValue(1));
								ChatPacket(CHAT_TYPE_INFO, "[i]Ranforsarea a fost adaugata cu succes!");
 
								item->SetCount(item->GetCount() - 1);
							}
							else
							{
								ChatPacket(CHAT_TYPE_INFO, "[i]Poti adauga doar 2 ranforsari pe mount.");
							}
						}
						else
						{
							ChatPacket(CHAT_TYPE_INFO, "[i]Ranforsarea merge adaugata doar pe mount.");
						}
					}
					break;					

#endif
						//  ACCESSORY_REFINE & ADD/CHANGE_ATTRIBUTES
					case USE_PUT_INTO_BELT_SOCKET:
					case USE_PUT_INTO_RING_SOCKET:
					case USE_PUT_INTO_ACCESSORY_SOCKET:
					case USE_ADD_ACCESSORY_SOCKET:
					case USE_CLEAN_SOCKET:
					case USE_CHANGE_ATTRIBUTE:
					case USE_CHANGE_ATTRIBUTE2 :
					case USE_ADD_ATTRIBUTE:
					case USE_ADD_ATTRIBUTE2:
#ifdef __AURA_SYSTEM__
					case USE_PUT_INTO_AURA_SOCKET:
#endif
						{
							LPITEM item2;
							if (!IsValidItemPosition(DestCell) || !(item2 = GetItem(DestCell)))
								return false;

							if (item2->IsEquipped())
							{
								BuffOnAttr_RemoveBuffsFromItem(item2);
							}

							// [NOTE] �ڽ�Ƭ �����ۿ��� ������ ���� ������ ���� �Ӽ��� �ο��ϵ�, ����簡 ����� ���ƴ޶�� ��û�� �־���.
							// ���� ANTI_CHANGE_ATTRIBUTE ���� ������ Flag�� �߰��Ͽ� ��ȹ �������� �����ϰ� ��Ʈ�� �� �� �ֵ��� �� �����̾�����
							// �׵��� �ʿ������ ��ġ�� ���� �ش޷��� �׳� ���⼭ ����... -_-
							if (ITEM_COSTUME == item2->GetType() || (ITEM_ARMOR == item2->GetType() && ARMOR_GLOVE == item2->GetSubType()))
#ifdef __AURA_SYSTEM__
							if (item->GetSubType() != USE_PUT_INTO_AURA_SOCKET)
#endif
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;396]");
								return false;
							}

							if (item2->IsExchanging() || item2->IsEquipped()) // @fixme114
								return false;

							switch (item->GetSubType())
							{
								case USE_CLEAN_SOCKET:
									{
										int i;
#ifdef ENABLE_EXTENDED_SOCKETS
										for (i = 0; i < ITEM_STONES_MAX_NUM; ++i)
#else
										for (i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
#endif
										{
											if (item2->GetSocket(i) == ITEM_BROKEN_METIN_VNUM)
												break;
										}

#ifdef ENABLE_EXTENDED_SOCKETS
										if (i == ITEM_STONES_MAX_NUM)
#else
										if (i == ITEM_SOCKET_MAX_NUM)
#endif
										{
											ChatPacket(CHAT_TYPE_INFO, "[LS;395]");
											return false;
										}

										int j = 0;

#ifdef ENABLE_EXTENDED_SOCKETS
										for (i = 0; i < ITEM_STONES_MAX_NUM; ++i)
#else
										for (i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
#endif
										{
											if (item2->GetSocket(i) != ITEM_BROKEN_METIN_VNUM && item2->GetSocket(i) != 0)
												item2->SetSocket(j++, item2->GetSocket(i));
										}

#ifdef ENABLE_EXTENDED_SOCKETS
										for (; j < ITEM_STONES_MAX_NUM; ++j)
#else
										for (; j < ITEM_SOCKET_MAX_NUM; ++j)
#endif
										{
											if (item2->GetSocket(j) > 0)
												item2->SetSocket(j, 1);
										}

										{
											char buf[21];
											snprintf(buf, sizeof(buf), "%u", item2->GetID());
											LogManager::instance().ItemLog(this, item, "CLEAN_SOCKET", buf);
										}

										item->SetCount(item->GetCount() - 1);

									}
									break;

								case USE_CHANGE_ATTRIBUTE :
									if (item2->GetAttributeSetIndex() == -1)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;396]");
										return false;
									}

									if (item2->GetAttributeCount() == 0)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;397]");
										return false;
									}

									if (item2->GetType() == ITEM_TALISMAN)
									{
										ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot change attribute with this item."));
										return false;
									}
									
									if ((GM_PLAYER == GetGMLevel()) && (false == test_server) && (g_dwItemBonusChangeTime > 0))
									{
										//
										// Event Flag �� ���� ������ ������ �Ӽ� ������ �� �ð����� ���� ����� �ð��� �귶���� �˻��ϰ�
										// �ð��� ����� �귶�ٸ� ���� �Ӽ����濡 ���� �ð��� ������ �ش�.
										//

										// DWORD dwChangeItemAttrCycle = quest::CQuestManager::instance().GetEventFlag(msc_szChangeItemAttrCycleFlag);
										// if (dwChangeItemAttrCycle < msc_dwDefaultChangeItemAttrCycle)
											// dwChangeItemAttrCycle = msc_dwDefaultChangeItemAttrCycle;
										DWORD dwChangeItemAttrCycle = g_dwItemBonusChangeTime;

										quest::PC* pPC = quest::CQuestManager::instance().GetPC(GetPlayerID());

										if (pPC)
										{
											DWORD dwNowSec = get_global_time();

											DWORD dwLastChangeItemAttrSec = pPC->GetFlag(msc_szLastChangeItemAttrFlag);

											if (dwLastChangeItemAttrSec + dwChangeItemAttrCycle > dwNowSec)
											{
												ChatPacket(CHAT_TYPE_INFO, "[LS;398;%d;%d]",
														dwChangeItemAttrCycle, dwChangeItemAttrCycle - (dwNowSec - dwLastChangeItemAttrSec));
												return false;
											}

											pPC->SetFlag(msc_szLastChangeItemAttrFlag, dwNowSec);
										}
									}

									if (item->GetSubType() == USE_CHANGE_ATTRIBUTE2)
									{
										int aiChangeProb[ITEM_ATTRIBUTE_MAX_LEVEL] =
										{
											0, 0, 30, 40, 3
										};

										item2->ChangeAttribute(aiChangeProb);
									}
									#ifdef ENABLE_LEGENDARY_SWITCHERS
									else if(item->GetVnum() == AVERAGE_DAMAGE_ITEM_VNUM)
										item2->ChangeAttribute(NULL, 1);
									else if(item->GetVnum() == SKILL_DAMAGE_ITEM_VNUM)
										item2->ChangeAttribute(NULL, 2);
									#endif
									else if (item->GetVnum() == 76014)
									{
										int aiChangeProb[ITEM_ATTRIBUTE_MAX_LEVEL] =
										{
											0, 10, 50, 39, 1
										};

										item2->ChangeAttribute(aiChangeProb);
									}

									else
									{
										// ����� Ư��ó��
										// ����� ���簡 �߰� �ȵɰŶ� �Ͽ� �ϵ� �ڵ���.
										if (item->GetVnum() == 71151 || item->GetVnum() == 76023)
										{
											if ((item2->GetType() == ITEM_WEAPON)
												|| (item2->GetType() == ITEM_ARMOR && item2->GetSubType() == ARMOR_BODY))
											{
												bool bCanUse = true;
												for (int i = 0; i < ITEM_LIMIT_MAX_NUM; ++i)
												{
													if (item2->GetLimitType(i) == LIMIT_LEVEL && item2->GetLimitValue(i) > 40)
													{
														bCanUse = false;
														break;
													}
												}
												if (false == bCanUse)
												{
													ChatPacket(CHAT_TYPE_INFO, "[LS;1064]");
													break;
												}
											}
											else
											{
												ChatPacket(CHAT_TYPE_INFO, "[LS;1065]");
												break;
											}
										}
										item2->ChangeAttribute();
									}

									ChatPacket(CHAT_TYPE_INFO, "[LS;399]");
									{
										char buf[21];
										snprintf(buf, sizeof(buf), "%u", item2->GetID());
										LogManager::instance().ItemLog(this, item, "CHANGE_ATTRIBUTE", buf);
									}

									if (VRAJESTE_PERMANENT == false)
									{
										item->SetCount(item->GetCount() - 1);
										break;
									}	
										else
									{
										break;
									}		
									break;
#if defined(__ATTR_6TH_7TH__)
								case USE_CHANGE_ATTRIBUTE2:
									{
										if (item2->GetAttributeSetIndex() == -1)
										{
											ChatPacket(CHAT_TYPE_INFO, "[LS;396]");
											return false;
										}

										if (item2->GetRareAttrCount() == 0)
										{
											ChatPacket(CHAT_TYPE_INFO, "[LS;397]");
											return false;
										}
										
										BYTE successChance = 100;
										if (item->GetVnum() == 72351)
											successChance = 10;
										
										if (number(1, 100) <= successChance)
										{
											if (item2->ChangeRareAttribute())
											{
												ChatPacket(CHAT_TYPE_INFO, "[LS;399]");
												{
													char szBuff[21];
													snprintf(szBuff, sizeof(szBuff), "%u", item2->GetID());
													LogManager::instance().ItemLog(this, item, "CHANGE_ATTRIBUTE2", szBuff);
												}
											}
											else
											{
												ChatPacket(CHAT_TYPE_INFO, "Nu exista bonusuri de schimbat");
											}
										}
										else
										{
											ChatPacket(CHAT_TYPE_INFO, "Imbunatatirea a esuat.");
										}

										item->SetCount(item->GetCount() - 1);
									}
									break;
#endif
								case USE_ADD_ATTRIBUTE :
									if (item2->GetAttributeSetIndex() == -1)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;396]");
										return false;
									}

									if (item2->GetType() == ITEM_TALISMAN)
									{
										ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot add attribute with this item."));
										return false;
									}
									
									if (item2->GetAttributeCount() < 4)
									{
										// ���簡 Ư��ó��
										// ����� ���簡 �߰� �ȵɰŶ� �Ͽ� �ϵ� �ڵ���.
										if (item->GetVnum() == 71152 || item->GetVnum() == 76024)
										{
											if ((item2->GetType() == ITEM_WEAPON)
												|| (item2->GetType() == ITEM_ARMOR && item2->GetSubType() == ARMOR_BODY))
											{
												bool bCanUse = true;
												for (int i = 0; i < ITEM_LIMIT_MAX_NUM; ++i)
												{
													if (item2->GetLimitType(i) == LIMIT_LEVEL && item2->GetLimitValue(i) > 40)
													{
														bCanUse = false;
														break;
													}
												}
												if (false == bCanUse)
												{
													ChatPacket(CHAT_TYPE_INFO, "[LS;1064]");
													break;
												}
											}
											else
											{
												ChatPacket(CHAT_TYPE_INFO, "[LS;1065]");
												break;
											}
										}
										char buf[21];
										snprintf(buf, sizeof(buf), "%u", item2->GetID());

										if (number(1, 100) <= aiItemAttributeAddPercent[item2->GetAttributeCount()])
										{
											item2->AddAttribute();
											ChatPacket(CHAT_TYPE_INFO, "[LS;400]");

											int iAddedIdx = item2->GetAttributeCount() - 1;
											LogManager::instance().ItemLog(
													GetPlayerID(),
													item2->GetAttributeType(iAddedIdx),
													item2->GetAttributeValue(iAddedIdx),
													item->GetID(),
													"ADD_ATTRIBUTE_SUCCESS",
													buf,
													GetDesc()->GetHostName(),
													item->GetOriginalVnum());
										}
										else
										{
											ChatPacket(CHAT_TYPE_INFO, "[LS;401]");
											LogManager::instance().ItemLog(this, item, "ADD_ATTRIBUTE_FAIL", buf);
										}

										if (INTARIRE_PERMANENT == false)
										{
											item->SetCount(item->GetCount() - 1);
											break;
										}
									}
									else
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;402]");
									}
									break;

								case USE_ADD_ATTRIBUTE2 :
									// �ູ�� ����
									// �簡�񼭸� ���� �Ӽ��� 4�� �߰� ��Ų �����ۿ� ���ؼ� �ϳ��� �Ӽ��� �� �ٿ��ش�.
									if (item2->GetAttributeSetIndex() == -1)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;396]");
										return false;
									}

									// �Ӽ��� �̹� 4�� �߰� �Ǿ��� ���� �Ӽ��� �߰� �����ϴ�.
									if (item2->GetAttributeCount() == 4)
									{
										char buf[21];
										snprintf(buf, sizeof(buf), "%u", item2->GetID());

										if (number(1, 100) <= aiItemAttributeAddPercent[item2->GetAttributeCount()])
										{
											short AttributeCount = abs(2 - item->GetAttributeCount());
											for (int i = 0; i < AttributeCount; i++)
												item2->AddAttribute();
											ChatPacket(CHAT_TYPE_INFO, "[LS;400]");

											int iAddedIdx = item2->GetAttributeCount() - 1;
											LogManager::instance().ItemLog(
													GetPlayerID(),
													item2->GetAttributeType(iAddedIdx),
													item2->GetAttributeValue(iAddedIdx),
													item->GetID(),
													"ADD_ATTRIBUTE2_SUCCESS",
													buf,
													GetDesc()->GetHostName(),
													item->GetOriginalVnum());
										}
										else
										{
											ChatPacket(CHAT_TYPE_INFO, "[LS;401]");
											LogManager::instance().ItemLog(this, item, "ADD_ATTRIBUTE2_FAIL", buf);
										}

										item->SetCount(item->GetCount() - 1);
									}
									else if (item2->GetAttributeCount() == 5)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;404]");
									}
									else if (item2->GetAttributeCount() < 4)
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;405]");
									}
									else
									{
										// wtf ?!
										sys_err("ADD_ATTRIBUTE2 : Item has wrong AttributeCount(%d)", item2->GetAttributeCount());
									}
									break;

								case USE_ADD_ACCESSORY_SOCKET:
									{
										char buf[21];
										snprintf(buf, sizeof(buf), "%u", item2->GetID());

										if (item2->IsAccessoryForSocket())
										{
											if (item2->GetAccessorySocketMaxGrade() < ITEM_ACCESSORY_SOCKET_MAX_NUM)
											{
#ifdef ENABLE_ADDSTONE_FAILURE
												if (number(1, 100) <= 50)
#else
												if (1)
#endif
												{
													item2->SetAccessorySocketMaxGrade(item2->GetAccessorySocketMaxGrade() + 1);
													ChatPacket(CHAT_TYPE_INFO, "[LS;406]");
													LogManager::instance().ItemLog(this, item, "ADD_SOCKET_SUCCESS", buf);
												}
												else
												{
													ChatPacket(CHAT_TYPE_INFO, "[LS;407]");
													LogManager::instance().ItemLog(this, item, "ADD_SOCKET_FAIL", buf);
												}

												item->SetCount(item->GetCount() - 1);
											}
											else
											{
												ChatPacket(CHAT_TYPE_INFO, "[LS;408]");
											}
										}
										else
										{
											ChatPacket(CHAT_TYPE_INFO, "[LS;409]");
										}
									}
									break;
								case USE_PUT_INTO_BELT_SOCKET:
								case USE_PUT_INTO_ACCESSORY_SOCKET:
									if (item2->IsAccessoryForSocket() && item->CanPutInto(item2))
									{
										char buf[21];
										snprintf(buf, sizeof(buf), "%u", item2->GetID());

										if (item2->GetAccessorySocketGrade() < item2->GetAccessorySocketMaxGrade())
										{
											if (number(1, 100) <= aiAccessorySocketPutPct[item2->GetAccessorySocketGrade()])
											{
												item2->SetAccessorySocketGrade(item2->GetAccessorySocketGrade() + 1);
												ChatPacket(CHAT_TYPE_INFO, "[LS;410]");
												LogManager::instance().ItemLog(this, item, "PUT_SOCKET_SUCCESS", buf);
											}
											else
											{
												ChatPacket(CHAT_TYPE_INFO, "[LS;411]");
												LogManager::instance().ItemLog(this, item, "PUT_SOCKET_FAIL", buf);
											}

											item->SetCount(item->GetCount() - 1);
										}
										else
										{
											if (item2->GetAccessorySocketMaxGrade() == 0)
												ChatPacket(CHAT_TYPE_INFO, "[LS;412]");
											else if (item2->GetAccessorySocketMaxGrade() < ITEM_ACCESSORY_SOCKET_MAX_NUM)
											{
												ChatPacket(CHAT_TYPE_INFO, "[LS;413]");
												ChatPacket(CHAT_TYPE_INFO, "[LS;415]");
											}
											else
												ChatPacket(CHAT_TYPE_INFO, "[LS;416]");
										}
									}
									else
									{
										ChatPacket(CHAT_TYPE_INFO, "[LS;417]");
									}
									break;
#ifdef __AURA_SYSTEM__
								case USE_PUT_INTO_AURA_SOCKET:
								{
									if (item2->IsAuraBoosterForSocket() && item->CanPutInto(item2))
									{
										char buf[21];
										snprintf(buf, sizeof(buf), "%lu", item2->GetID());

										const BYTE c_bAuraBoostIndex = item->GetOriginalVnum() - ITEM_AURA_BOOST_ITEM_VNUM_BASE;
										item2->SetSocket(ITEM_SOCKET_AURA_BOOST, c_bAuraBoostIndex * 100000000 + item->GetValue(ITEM_AURA_BOOST_TIME_VALUE));

										ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[Aura] Aura booster attached successfully."));

										LogManager::instance().ItemLog(this, item, "PUT_AURA_SOCKET", buf);

										if (IS_SET(item->GetFlag(), ITEM_FLAG_STACKABLE) && !IS_SET(item->GetAntiFlag(), ITEM_ANTIFLAG_STACK) && item->GetCount() > 1)
											item->SetCount(item->GetCount() - 1);
										else
											ITEM_MANAGER::instance().RemoveItem(item, "PUT_AURA_SOCKET_REMOVE");
									}
									else
										ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[Aura] You cannot add aura boost to this item."));
								}
								break;
#endif
							}
							if (item2->IsEquipped())
							{
								BuffOnAttr_AddBuffsFromItem(item2);
							}
						}
						break;
						//  END_OF_ACCESSORY_REFINE & END_OF_ADD_ATTRIBUTES & END_OF_CHANGE_ATTRIBUTES

					case USE_BAIT:
						{

							if (m_pkFishingEvent)
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;418]");
								return false;
							}

							LPITEM weapon = GetWear(WEAR_WEAPON);

							if (!weapon || weapon->GetType() != ITEM_ROD)
								return false;

							if (weapon->GetSocket(2))
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;419;%s]", item->GetName());
							}
							else
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;420;%s]", item->GetName());
							}

							weapon->SetSocket(2, item->GetValue(0));
							item->SetCount(item->GetCount() - 1);
						}
						break;

					case USE_MOVE:
					case USE_TREASURE_BOX:
					case USE_MONEYBAG:
						break;

					case USE_AFFECT :
						{
#ifdef __PREMIUM_PRIVATE_SHOP__
			if (item->GetValue(0) == AFFECT_PREMIUM_PRIVATE_SHOP)
			{
				if (SetPremiumPrivateShopBonus(item->GetValue(3)))
					item->SetCount(item->GetCount() - 1);
				return true;
			}
#endif
#ifdef ENABLE_AFFECT_RENEWAL
							for (int i = AFFECT_DRAGON_GOD_1; i <= AFFECT_DRAGON_GOD_4; ++i)
							{
								if (FindAffect(i, aApplyInfo[item->GetValue(1)].bPointType))
								{
									ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
									return false;
								}
							}
#endif
							if (FindAffect(item->GetValue(0), aApplyInfo[item->GetValue(1)].bPointType))
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
							}
							else
							{

								AddAffect(item->GetValue(0), aApplyInfo[item->GetValue(1)].bPointType, item->GetValue(2), 0, item->GetValue(3), 0, false);
								item->SetCount(item->GetCount() - 1);
							}
						}
						break;

					case USE_CREATE_STONE:
						AutoGiveItem(number(28000, 28013));
						item->SetCount(item->GetCount() - 1);
						break;

					// ���� ���� ��ų�� ������ ó��
					case USE_RECIPE :
						{
							LPITEM pSource1 = FindSpecifyItem(item->GetValue(1));
							DWORD dwSourceCount1 = item->GetValue(2);

							LPITEM pSource2 = FindSpecifyItem(item->GetValue(3));
							DWORD dwSourceCount2 = item->GetValue(4);

							if (dwSourceCount1 != 0)
							{
								if (pSource1 == NULL)
								{
									ChatPacket(CHAT_TYPE_INFO, "[LS;421]");
									return false;
								}
							}

							if (dwSourceCount2 != 0)
							{
								if (pSource2 == NULL)
								{
									ChatPacket(CHAT_TYPE_INFO, "[LS;421]");
									return false;
								}
							}

							if (pSource1 != NULL)
							{
								if (pSource1->GetCount() < dwSourceCount1)
								{
									ChatPacket(CHAT_TYPE_INFO, "[LS;422;%s]", pSource1->GetName());
									return false;
								}

								pSource1->SetCount(pSource1->GetCount() - dwSourceCount1);
							}

							if (pSource2 != NULL)
							{
								if (pSource2->GetCount() < dwSourceCount2)
								{
									ChatPacket(CHAT_TYPE_INFO, "[LS;422;%s]", pSource2->GetName());
									return false;
								}

								pSource2->SetCount(pSource2->GetCount() - dwSourceCount2);
							}

							LPITEM pBottle = FindSpecifyItem(50901);

							if (!pBottle || pBottle->GetCount() < 1)
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;423]");
								return false;
							}

							pBottle->SetCount(pBottle->GetCount() - 1);

							if (number(1, 100) > item->GetValue(5))
							{
								ChatPacket(CHAT_TYPE_INFO, "[LS;424]");
								return false;
							}

							AutoGiveItem(item->GetValue(0));
						}
						break;
				}
			}
			break;
			
			
#ifdef __ENABLE_SHAMAN_SYSTEM__
					case USE_UNLOCK_SHAMAN:
					{
						ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You upgrade your buff successfully"));
						return false;
						// if (!IsLoadedAffect() || item->IsExchanging())
						// {
							// ChatPacket(CHAT_TYPE_INFO, "You cannot actually use it.", this);
							// return false;
						// }
						
						// if (GetShamanSystem() && GetShamanSystem()->UpgradePremium(item)) // item is removed in UpgradePremium function.
						// {
							// ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You upgrade your buff successfully"));
							// return false;
						// }
					}
					break;

#endif
		
		case ITEM_METIN:
			{
				LPITEM item2;

				if (!IsValidItemPosition(DestCell) || !(item2 = GetItem(DestCell)))
					return false;

				if (item2->IsExchanging() || item2->IsEquipped()) // @fixme114
					return false;

				if (item2->GetType() == ITEM_PICK) return false;
				if (item2->GetType() == ITEM_ROD) return false;

				int i;

				for (i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
				{
					DWORD dwVnum;

					if ((dwVnum = item2->GetSocket(i)) <= 2)
						continue;

					TItemTable * p = ITEM_MANAGER::instance().GetTable(dwVnum);

					if (!p)
						continue;

					if (item->GetValue(5) == p->alValues[5])
					{
						ChatPacket(CHAT_TYPE_INFO, "[LS;426]");
						return false;
					}
				}

				if (item2->GetType() == ITEM_ARMOR)
				{
					if (!IS_SET(item->GetWearFlag(), WEARABLE_BODY) || !IS_SET(item2->GetWearFlag(), WEARABLE_BODY))
					{
						ChatPacket(CHAT_TYPE_INFO, "[LS;427]");
						return false;
					}
				}
				else if (item2->GetType() == ITEM_WEAPON)
				{
					if (!IS_SET(item->GetWearFlag(), WEARABLE_WEAPON))
					{
						ChatPacket(CHAT_TYPE_INFO, "[LS;428]");
						return false;
					}
				}
				else
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;431]");
					return false;
				}

				for (i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
					if (item2->GetSocket(i) >= 1 && item2->GetSocket(i) <= 2 && item2->GetSocket(i) >= item->GetValue(2))
					{

#ifdef ENABLE_ADDSTONE_FAILURE
						if (number(1, 100) <= 30)
#else
						if (1)
#endif
						{
							ChatPacket(CHAT_TYPE_INFO, "[LS;1613]");
							item2->SetSocket(i, item->GetVnum());
						}
						else
						{
							ChatPacket(CHAT_TYPE_INFO, "[LS;430]");
							item2->SetSocket(i, ITEM_BROKEN_METIN_VNUM);
						}

						LogManager::instance().ItemLog(this, item2, "SOCKET", item->GetName());
						item->SetCount(item->GetCount() - 1);
						break;
					}

				if (i == ITEM_SOCKET_MAX_NUM)
					ChatPacket(CHAT_TYPE_INFO, "[LS;431]");
			}
			break;

		case ITEM_AUTOUSE:
		case ITEM_MATERIAL:
		case ITEM_SPECIAL:
		case ITEM_TOOL:
		case ITEM_LOTTERY:
			break;

		case ITEM_TOTEM:
			{
				if (!item->IsEquipped())
					EquipItem(item);
			}
			break;

	case ITEM_BLEND:
		if (CBlendItem::instance().FindItem(item->GetVnum()))
		{
			int affect_type = AFFECT_BLEND;
			int apply_type = aApplyInfo[item->GetSocket(0)].bPointType;
			int apply_value = item->GetSocket(1);
			int apply_duration = item->GetSocket(2);

#ifdef ENABLE_AFFECT_RENEWAL
			if (CItemVnumHelper::IsExtendedBlend(item->GetVnum()) == true)
			{
				if ((apply_duration != 0) && UseExtendedBlendAffect(item, affect_type, apply_type, apply_value, apply_duration))
				{
					item->Lock(true);
					item->SetSocket(3, true);
					item->StartBlendExpireEvent();
				}
				else
				{
					item->Lock(false);
					item->SetSocket(3, false);
					item->StopBlendExpireEvent();
				}

				break;
			}
#endif

			if (FindAffect(affect_type, apply_type))
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
			}
			else
			{
				if (FindAffect(AFFECT_EXP_BONUS_EURO_FREE, POINT_RESIST_MAGIC))
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;99]");
				}
				else
				{
#ifdef ENABLE_BATTLE_PASS
					CBattlePass::Instance().RegisterItemMission(MISSION_TYPE_USE_ITEM, 1, this, item);
#endif
#ifdef ENABLE_AFFECT_RENEWAL
					if (SetBlendAffect(item))
					{
						AddAffect(affect_type, apply_type, apply_value, 0, apply_duration, 0, false);
						item->SetCount(item->GetCount() - 1);
					}
#else
					AddAffect(affect_type, apply_type, apply_value, 0, apply_duration, 0, false);
					item->SetCount(item->GetCount() - 1);
#endif
				}
			}
		}
		break;
		
		case ITEM_EXTRACT:
			{
				LPITEM pDestItem = GetItem(DestCell);
				if (NULL == pDestItem)
				{
					return false;
				}
				switch (item->GetSubType())
				{
				case EXTRACT_DRAGON_SOUL:
					if (pDestItem->IsDragonSoul())
					{
						return DSManager::instance().PullOut(this, NPOS, pDestItem, item);
					}
					return false;
				case EXTRACT_DRAGON_HEART:
					if (pDestItem->IsDragonSoul())
					{
						return DSManager::instance().ExtractDragonHeart(this, pDestItem, item);
					}
					return false;
				default:
					return false;
				}
			}
			break;
			
		case ITEM_TOGGLE:
			if (!OnUseItem(this, item))
				return false;
			break;
			
		case ITEM_NONE:
			sys_err("Item type NONE %s", item->GetName());
			break;

		default:
			sys_log(0, "UseItemEx: Unknown type %s %d", item->GetName(), item->GetType());
			return false;
	}

	return true;
}

int g_nPortalLimitTime = 10;

bool CHARACTER::UseItem(TItemPos Cell, TItemPos DestCell)
{
	WORD wCell = Cell.cell;
	BYTE window_type = Cell.window_type;
	LPITEM item;

	if (!CanHandleItem())
		return false;

	if (!IsValidItemPosition(Cell) || !(item = GetItem(Cell)))
			return false;
		
	if (GetUseItemStackAttrFlood() + PASSES_PER_SEC(1) > thecore_pulse())
	{
		ChatPacket(CHAT_TYPE_INFO, "You need to wait 1 second to do that.");
		return false;
	}

	sys_log(0, "%s: USE_ITEM %s (inven %d, cell: %d)", GetName(), item->GetName(), window_type, wCell);

	if (item->IsExchanging())
		return false;
	if (!item->CanUsedBy(this))
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;1004]");
		return false;
	}
	
#ifdef __PREMIUM_PRIVATE_SHOP__
	if (IsEditingPrivateShop())
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(char_item.cpp)tradus:You cannot use items while editing your personal shop."));
		return false;
	}
#endif


#ifdef ENABLE_SWITCHBOT
	if (Cell.IsSwitchbotPosition())
	{
		CSwitchbot* pkSwitchbot = CSwitchbotManager::Instance().FindSwitchbot(GetPlayerID());
		if (pkSwitchbot && pkSwitchbot->IsActive(Cell.cell))
		{
			return false;
		}

		int iEmptyCell = GetEmptyInventory(item->GetSize());

		if (iEmptyCell == -1)
		{
			ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(char_item.cpp)tradus:full inventory."));
			return false;
		}

		MoveItem(Cell, TItemPos(INVENTORY, iEmptyCell), item->GetCount());
		return true;
	}
#endif
	
	if (IsStun())
		return false;
	if (false == FN_check_item_sex(this, item))
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;1005]");
		return false;
	}
	
#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	if (IsSecured())
	{
		ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT("@@(char_item.cpp)tradus:Acest Cont este securizat."));
		return false;
	}
#endif
	if (IS_SUMMON_ITEM(item->GetVnum()))
	{
		if (false == IS_SUMMONABLE_ZONE(GetMapIndex()))
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;97]");
			return false;
		}
		if (CThreeWayWar::instance().IsThreeWayWarMapIndex(GetMapIndex()))
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;433]");
			return false;
		}
		int iPulse = thecore_pulse();

		if (iPulse - GetSafeboxLoadTime() < PASSES_PER_SEC(g_nPortalLimitTime))
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;434;%d]", g_nPortalLimitTime);

			if (test_server)
				ChatPacket(CHAT_TYPE_INFO, "[TestOnly]Pulse %d LoadTime %d PASS %d", iPulse, GetSafeboxLoadTime(), PASSES_PER_SEC(g_nPortalLimitTime));
			return false;
		}

		if (GetExchange() || GetMyShop() || GetShopOwner() || IsOpenSafebox() || IsCubeOpen() || IsAuraRefineWindowOpen())
		{
			ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(char_item.cpp)tradus:[You cannot use a Scroll of the Location while another window is open.]You cannot use a Scroll of the Location while another window is open."));
			return false;
		}

		{
			if (iPulse - GetRefineTime() < PASSES_PER_SEC(g_nPortalLimitTime))
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;437;%d]", g_nPortalLimitTime);
				return false;
			}
		}
		{
			if (iPulse - GetMyShopTime() < PASSES_PER_SEC(g_nPortalLimitTime))
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;438;%d]", g_nPortalLimitTime);
				return false;
			}

		}
		if (item->GetVnum() != 70302)
		{
			PIXEL_POSITION posWarp;

			int x = 0;
			int y = 0;

			double nDist = 0;
			const double nDistant = 5000.0;

			if (item->GetVnum() == 22010)
			{
				x = item->GetSocket(0) - GetX();
				y = item->GetSocket(1) - GetY();
			}

			else if (item->GetVnum() == 22000)
			{
				SECTREE_MANAGER::instance().GetRecallPositionByEmpire(GetMapIndex(), GetEmpire(), posWarp);

				if (item->GetSocket(0) == 0)
				{
					x = posWarp.x - GetX();
					y = posWarp.y - GetY();
				}
				else
				{
					x = item->GetSocket(0) - GetX();
					y = item->GetSocket(1) - GetY();
				}
			}

			nDist = sqrt(pow((float)x,2) + pow((float)y,2));

			if (nDistant > nDist)
			{
				ChatPacket(CHAT_TYPE_INFO, "[LS;439]");
				if (test_server)
					ChatPacket(CHAT_TYPE_INFO, "PossibleDistant %f nNowDist %f", nDistant,nDist);
				return false;
			}
		}

		if (iPulse - GetExchangeTime()  < PASSES_PER_SEC(g_nPortalLimitTime))
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;440;%d]", g_nPortalLimitTime);
			return false;
		}
	}

	//������ ��� ���� �ŷ�â ���� üũ
	if ((item->GetVnum() == 50200) || (item->GetVnum() == 71049))
	{
		if (GetExchange() || GetMyShop() || GetShopOwner() || IsOpenSafebox() || IsCubeOpen() || IsAuraRefineWindowOpen())
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;1440]");
			return false;
		}

	}
	//END_PREVENT_TRADE_WINDOW
		

	// @fixme150 BEGIN
	if (quest::CQuestManager::instance().GetPCForce(GetPlayerID())->IsRunning() == true)
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(char_item.cpp)tradus:You cannot use this item if you're using quests"));
		return false;
	}
	// @fixme150 END

	if (IS_SET(item->GetFlag(), ITEM_FLAG_LOG)) // ��� �α׸� ����� ������ ó��
	{
		DWORD vid = item->GetVID();
		DWORD oldCount = item->GetCount();
		DWORD vnum = item->GetVnum();

		char hint[ITEM_NAME_MAX_LEN + 32 + 1];
		int len = snprintf(hint, sizeof(hint) - 32, "%s", item->GetName());

		if (len < 0 || len >= (int) sizeof(hint) - 32)
			len = (sizeof(hint) - 32) - 1;

		DWORD dwVnum = item->GetVnum();
		bool ret = UseItemEx(item, DestCell);

		if (ret)
		{
			// STACK_ATTR_ITEM
			ITEM_STACK_ATTRIBUTE::instance().StackAttributeByWearIndex(this, STACK_ATTRIBUTE_BY_ITEM, dwVnum);
			// STACK_ATTR_ITEM
		}

		if (NULL == ITEM_MANAGER::instance().FindByVID(vid)) // UseItemEx���� �������� ���� �Ǿ���. ���� �α׸� ����
		{
			LogManager::instance().ItemLog(this, vid, vnum, "REMOVE", hint);
		}
		else if (oldCount != item->GetCount())
		{
			snprintf(hint + len, sizeof(hint) - len, " %u", oldCount - 1);
			LogManager::instance().ItemLog(this, vid, vnum, "USE_ITEM", hint);
		}
		return (ret);
	}
	else
	{
		DWORD dwVnum = item->GetVnum();
		bool ret = UseItemEx(item, DestCell);
		
		if (ret)
		{
			// STACK_ATTR_ITEM
			ITEM_STACK_ATTRIBUTE::instance().StackAttributeByWearIndex(this, STACK_ATTRIBUTE_BY_ITEM, dwVnum);
			// STACK_ATTR_ITEM
		}
	
		return ret;
	}
}


bool CHARACTER::DestroyItem(TItemPos Cell)
{
	LPITEM item = NULL;
	if (!CanHandleItem())
	{
		if (NULL != DragonSoul_RefineWindow_GetOpener())
			ChatPacket(CHAT_TYPE_INFO, "[LS;1069]");
		return false;
	}
	if (IsDead())
		return false;
	if (!IsValidItemPosition(Cell) || !(item = GetItem(Cell)))
		return false;
	if (item->IsExchanging())
		return false;
	if (true == item->isLocked())
		return false;
	if (quest::CQuestManager::instance().GetPCForce(GetPlayerID())->IsRunning() == true)
		return false;
	if (item->GetCount() <= 0)
		return false;
	
#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	if (IsSecured())
	{
		ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT("@@(char_item.cpp)tradus:Acest_cont_este_securizat!"));
		return false;
	}
#endif
	
	SyncQuickslot(QUICKSLOT_TYPE_ITEM, Cell.cell, 255);
	ITEM_MANAGER::instance().RemoveItem(item);
	ChatPacket(CHAT_TYPE_INFO, "[LS;1947;%s]", item->GetName());

	return true;
}

bool CHARACTER::DropItem(TItemPos Cell, WORD bCount)
{
	bCount = abs(bCount);
	LPITEM item = NULL;

	if (!CanHandleItem())
	{
		if (NULL != DragonSoul_RefineWindow_GetOpener())
			ChatPacket(CHAT_TYPE_INFO, "[LS;1069]");
		return false;
	}
#ifdef ENABLE_NEWSTUFF
	if (0 != g_ItemDropTimeLimitValue)
	{
		if (get_dword_time() < m_dwLastItemDropTime+g_ItemDropTimeLimitValue)
		{
			ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(char_item.cpp)tradus:[#Unk]You cannot drop Yang yet"));
			return false;
		}
	}

	m_dwLastItemDropTime = get_dword_time();
#endif
	if (IsDead())
		return false;

	if (!IsValidItemPosition(Cell) || !(item = GetItem(Cell)))
		return false;

	if (item->IsExchanging())
		return false;

	if (true == item->isLocked())
		return false;

	if (quest::CQuestManager::instance().GetPCForce(GetPlayerID())->IsRunning() == true)
		return false;

	if (IS_SET(item->GetAntiFlag(), ITEM_ANTIFLAG_DROP | ITEM_ANTIFLAG_GIVE))
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;442]");
		return false;
	}
	

#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	if (IsSecured())
	{
		ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT("@@(char_item.cpp)tradus:Acest_cont_este_securizat!"));
		return false;
	}
#endif


	if (bCount == 0 || bCount > item->GetCount())
		bCount = item->GetCount();

	SyncQuickslot(QUICKSLOT_TYPE_ITEM, Cell.cell, 255);	// Quickslot ���� ����

	LPITEM pkItemToDrop;

	if (bCount == item->GetCount())
	{
		item->RemoveFromCharacter();
		pkItemToDrop = item;
	}
	else
	{
		if (bCount == 0)
		{
			if (test_server)
				sys_log(0, "[DROP_ITEM] drop item count == 0");
			return false;
		}

		item->SetCount(item->GetCount() - bCount);
		ITEM_MANAGER::instance().FlushDelayedSave(item);

		pkItemToDrop = ITEM_MANAGER::instance().CreateItem(item->GetVnum(), bCount);

		// copy item socket -- by mhh
		FN_copy_item_socket(pkItemToDrop, item);

		char szBuf[51 + 1];
		snprintf(szBuf, sizeof(szBuf), "%u %u", pkItemToDrop->GetID(), pkItemToDrop->GetCount());
		LogManager::instance().ItemLog(this, item, "ITEM_SPLIT", szBuf);
	}

	PIXEL_POSITION pxPos = GetXYZ();

	if (pkItemToDrop->AddToGround(GetMapIndex(), pxPos))
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;443]");
#ifdef ENABLE_NEWSTUFF
		pkItemToDrop->StartDestroyEvent(g_aiItemDestroyTime[ITEM_DESTROY_TIME_DROPITEM]);
#else
		pkItemToDrop->StartDestroyEvent();
#endif

		ITEM_MANAGER::instance().FlushDelayedSave(pkItemToDrop);

		char szHint[32 + 1];
		snprintf(szHint, sizeof(szHint), "%s %u %u", pkItemToDrop->GetName(), pkItemToDrop->GetCount(), pkItemToDrop->GetOriginalVnum());
		LogManager::instance().ItemLog(this, pkItemToDrop, "DROP", szHint);
		//Motion(MOTION_PICKUP);
	}

	return true;
}

bool CHARACTER::DropGold(int gold)
{
	if (gold <= 0 || gold > GetGold())
		return false;

	if (!CanHandleItem())
		return false;

	if (0 != g_GoldDropTimeLimitValue)
	{
		if (get_dword_time() < m_dwLastGoldDropTime+g_GoldDropTimeLimitValue)
		{
			ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(char_item.cpp)tradus:[#Unk]You cannot drop Yang yet"));
			return false;
		}
	}

	m_dwLastGoldDropTime = get_dword_time();

	LPITEM item = ITEM_MANAGER::instance().CreateItem(1, gold);

	if (item)
	{
		PIXEL_POSITION pos = GetXYZ();

		if (item->AddToGround(GetMapIndex(), pos))
		{
			//Motion(MOTION_PICKUP);
			PointChange(POINT_GOLD, -gold, true);

			if (gold > 1000) // õ�� �̻� ����Ѵ�.
				LogManager::instance().CharLog(this, gold, "DROP_GOLD", "");

#ifdef ENABLE_NEWSTUFF
			item->StartDestroyEvent(g_aiItemDestroyTime[ITEM_DESTROY_TIME_DROPGOLD]);
#else
			item->StartDestroyEvent();
#endif
			ChatPacket(CHAT_TYPE_INFO, "[LS;1057;%d]", 150/60);
		}

		Save();
		return true;
	}

	return false;
}

bool CHARACTER::MoveItem(TItemPos Cell, TItemPos DestCell, WORD count)
{
	count = abs(count);
	LPITEM item = NULL;
	
#ifdef ENABLE_DRAGONSOUL_ALCHEMY_PLUS
	if (DragonSoul_IsDeckActivated() && (DestCell.window_type == DRAGON_SOUL_INVENTORY || Cell.window_type == DRAGON_SOUL_INVENTORY))
		ComputePoints();
#endif

	if (!IsValidItemPosition(Cell))
		return false;

	if (!(item = GetItem(Cell)))
		return false;

	if (item->IsExchanging())
		return false;

	if (item->GetCount() < count)
		return false;

#ifdef ENABLE_CUSTOM_INVENTORY
	if (INVENTORY == Cell.window_type && IS_SET(item->GetFlag(), ITEM_FLAG_IRREMOVABLE))
	{
		 if((Cell.cell >= INVENTORY_MAX_NUM && Cell.cell < CUSTOM_INVENTORY_SLOT_START) || Cell.cell >= CUSTOM_INVENTORY_SLOT_END)
			return false;
	}	
#else
	if (INVENTORY == Cell.window_type && Cell.cell >= INVENTORY_MAX_NUM && IS_SET(item->GetFlag(), ITEM_FLAG_IRREMOVABLE))
		return false;

#endif

	if (true == item->isLocked())
		return false;
	
#ifdef ENABLE_CUSTOM_INVENTORY
	if(INVENTORY == Cell.window_type && INVENTORY == DestCell.window_type)
	{
		if(Cell.IsDefaultInventoryPosition() && DestCell.cell == USHRT_MAX)
		{
			WORD wFindCell = GetEmptyInventory(item, 2);
			if (wFindCell != -1)
			{
				DestCell.cell = wFindCell;
			}
			else
			{
				ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Nu ai spatiu suficient in inventarul special."));
				return false;
			}
		}
		
		if(Cell.IsCustomInventoryPosition() && DestCell.cell == USHRT_MAX)
		{
			WORD wFindCell = GetEmptyInventory(item, 1);
			if (wFindCell != -1)
			{
				DestCell.cell = wFindCell;
			}
			else
			{
				ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Nu ai spatiu suficient in inventar."));
				return false;
			}
		}
	}
#endif
	
	if (!IsValidItemPosition(DestCell))
	{
		return false;
	}

	if (!CanHandleItem())
	{
		if (NULL != DragonSoul_RefineWindow_GetOpener())
			ChatPacket(CHAT_TYPE_INFO, "[LS;1069]");
#ifdef __AURA_SYSTEM__
		if (IsAuraRefineWindowOpen())
			ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(char_item.cpp)tradus:[Aura] You cannot move items until the aura window is still opened."));
#endif
		return false;
	}
	
#ifdef ENABLE_CUSTOM_INVENTORY
	if (DestCell.IsCustomInventoryPosition() && DestCell.GetCustomInventoryCategory() != item->GetItemCategory())
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(char_item.cpp)tradus:[Special_Inventory]Nu poti plasa acest obiect aici."));
		return false;
	}
#endif
	
	if (DestCell.IsBeltInventoryPosition() && false == CBeltInventoryHelper::CanMoveIntoBeltInventory(item))
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;1097]");
		return false;
	}
	
#ifdef ENABLE_SWITCHBOT
	if (Cell.IsSwitchbotPosition() && CSwitchbotManager::Instance().IsActive(GetPlayerID(), Cell.cell))
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(char_item.cpp)tradus:cannot move active switchbot item."));
		return false;
	}

	if (DestCell.IsSwitchbotPosition() && !SwitchbotHelper::IsValidItem(item))
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(char_item.cpp)tradus:Invalid item type for switchbot."));
		return false;
	}
#endif

#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	if (IsSecured())
	{
		ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT("@@(char_item.cpp)tradus:Acest_cont_este_securizat!"));
		return false;
	}
#endif

	if (Cell.IsEquipPosition())
	{




		if (!CanUnequipNow(item))
			return false;

#ifdef ENABLE_WEAPON_COSTUME_SYSTEM
		int iWearCell = item->FindEquipCell(this);
		if (iWearCell == WEAR_WEAPON)
		{
			LPITEM costumeWeapon = GetWear(WEAR_COSTUME_WEAPON);
			if (costumeWeapon && !UnequipItem(costumeWeapon))
			{
				ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@(char_item.cpp)tradus:You cannot unequip the costume weapon because there is not enough space"));
				return false;
			}

			if (!IsEmptyItemGrid(DestCell, item->GetSize(), Cell.cell))
				return UnequipItem(item);
		}
#endif
	}

	if (DestCell.IsEquipPosition())
	{
		if (GetItem(DestCell))
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;1092]");

			return false;
		}

		EquipItem(item, DestCell.cell - INVENTORY_MAX_NUM);
	}
	else
	{
		if (item->IsDragonSoul())
		{
			if (item->IsEquipped())
			{
				return DSManager::instance().PullOut(this, DestCell, item);
			}
			else
			{
				if (DestCell.window_type != DRAGON_SOUL_INVENTORY)
				{
					return false;
				}

				if (!DSManager::instance().IsValidCellForThisItem(item, DestCell))
					return false;
			}
		}
		// ��ȥ���� �ƴ� �������� ��ȥ�� �κ��� �� �� ����.
		else if (DRAGON_SOUL_INVENTORY == DestCell.window_type)
			return false;

		LPITEM item2;

		if ((item2 = GetItem(DestCell)) && item != item2 && item2->IsStackable() &&
				!IS_SET(item2->GetAntiFlag(), ITEM_ANTIFLAG_STACK) &&
				item2->GetVnum() == item->GetVnum()) // ��ĥ �� �ִ� �������� ���
		{
			for (int i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
				if (item2->GetSocket(i) != item->GetSocket(i))
					return false;

			if (count == 0)
				count = item->GetCount();

			sys_log(0, "%s: ITEM_STACK %s (window: %d, cell : %d) -> (window:%d, cell %d) count %d", GetName(), item->GetName(), Cell.window_type, Cell.cell,
				DestCell.window_type, DestCell.cell, count);

			count = MIN(g_bItemCountLimit - item2->GetCount(), count);

			item->SetCount(item->GetCount() - count);
			item2->SetCount(item2->GetCount() + count);
			return true;
		}

		if (!IsEmptyItemGrid(DestCell, item->GetSize(), Cell.cell))
			return false;

		if (count == 0 || count >= item->GetCount() || !item->IsStackable() || IS_SET(item->GetAntiFlag(), ITEM_ANTIFLAG_STACK))
		{
			sys_log(0, "%s: ITEM_MOVE %s (window: %d, cell : %d) -> (window:%d, cell %d) count %d", GetName(), item->GetName(), Cell.window_type, Cell.cell,
				DestCell.window_type, DestCell.cell, count);

			item->RemoveFromCharacter();
#if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
			SetItem(DestCell, item, false);
#else
			SetItem(DestCell, item);
#endif		

			if (INVENTORY == Cell.window_type && INVENTORY == DestCell.window_type)
				SyncQuickslot(QUICKSLOT_TYPE_ITEM, Cell.cell, DestCell.cell);
		}
		else if (count < item->GetCount())
		{

			sys_log(0, "%s: ITEM_SPLIT %s (window: %d, cell : %d) -> (window:%d, cell %d) count %d", GetName(), item->GetName(), Cell.window_type, Cell.cell,
				DestCell.window_type, DestCell.cell, count);

			item->SetCount(item->GetCount() - count);
			LPITEM item2 = ITEM_MANAGER::instance().CreateItem(item->GetVnum(), count);

			// copy socket -- by mhh
			FN_copy_item_socket(item2, item);

#if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
			item2->AddToCharacter(this, DestCell, false);
#else
			item2->AddToCharacter(this, DestCell);
#endif

			char szBuf[51+1];
			snprintf(szBuf, sizeof(szBuf), "%u %u %u %u ", item2->GetID(), item2->GetCount(), item->GetCount(), item->GetCount() + item2->GetCount());
			LogManager::instance().ItemLog(this, item, "ITEM_SPLIT", szBuf);
		}
	}

	return true;
}

namespace NPartyPickupDistribute
{
	struct FFindOwnership
	{
		LPITEM item;
		LPCHARACTER owner;

		FFindOwnership(LPITEM item)
			: item(item), owner(NULL)
		{
		}

		void operator () (LPCHARACTER ch)
		{
			if (item->IsOwnership(ch))
				owner = ch;
		}
	};

	struct FCountNearMember
	{
		int		total;
		int		x, y;

		FCountNearMember(LPCHARACTER center )
			: total(0), x(center->GetX()), y(center->GetY())
		{
		}

		void operator () (LPCHARACTER ch)
		{
			if (DISTANCE_APPROX(ch->GetX() - x, ch->GetY() - y) <= PARTY_DEFAULT_RANGE)
				total += 1;
		}
	};

	struct FMoneyDistributor
	{
		int		total;
		LPCHARACTER	c;
		int		x, y;
		int		iMoney;

		FMoneyDistributor(LPCHARACTER center, int iMoney)
			: total(0), c(center), x(center->GetX()), y(center->GetY()), iMoney(iMoney)
		{
		}

		void operator ()(LPCHARACTER ch)
		{
			if (ch!=c)
				if (DISTANCE_APPROX(ch->GetX() - x, ch->GetY() - y) <= PARTY_DEFAULT_RANGE)
				{
#ifdef ENABLE_REMOVE_LIMIT_GOLD
					ch->ChangeGold(iMoney);
#else
					ch->PointChange(POINT_GOLD, iMoney, true);
#endif

					if (iMoney > 1000) // õ�� �̻� ����Ѵ�.
					{
						LOG_LEVEL_CHECK(LOG_LEVEL_MAX, LogManager::instance().CharLog(ch, iMoney, "GET_GOLD", ""));
					}
				}
		}
	};
}

void CHARACTER::GiveGold(INT iAmount)
{
	if (iAmount <= 0)
		return;

	sys_log(0, "GIVE_GOLD: %s %lld", GetName(), iAmount);

	if (GetParty())
	{
		LPPARTY pParty = GetParty();

		long long dwTotal = iAmount;
		long long dwMyAmount = dwTotal;

		NPartyPickupDistribute::FCountNearMember funcCountNearMember(this);
		pParty->ForEachOnlineMember(funcCountNearMember);

		if (funcCountNearMember.total > 1)
		{
			DWORD dwShare = dwTotal / funcCountNearMember.total;
			dwMyAmount -= dwShare * (funcCountNearMember.total - 1);

			NPartyPickupDistribute::FMoneyDistributor funcMoneyDist(this, dwShare);

			pParty->ForEachOnlineMember(funcMoneyDist);
		}

#ifdef ENABLE_REMOVE_LIMIT_GOLD
			ChangeGold(dwMyAmount);
#else
			PointChange(POINT_GOLD, dwMyAmount, true);
#endif
		if (dwMyAmount > 1000) // õ�� �̻� ����Ѵ�.
		{
			LOG_LEVEL_CHECK(LOG_LEVEL_MAX, LogManager::instance().CharLog(this, dwMyAmount, "GET_GOLD", ""));
		}
	}
	else
	{
#ifdef ENABLE_REMOVE_LIMIT_GOLD
			ChangeGold(iAmount);
#else
			PointChange(POINT_GOLD, iAmount, true);
#endif

		if (iAmount > 1000) // õ�� �̻� ����Ѵ�.
		{
			LOG_LEVEL_CHECK(LOG_LEVEL_MAX, LogManager::instance().CharLog(this, iAmount, "GET_GOLD", ""));
		}
	}
}

bool CHARACTER::PickupItem(DWORD dwVID)
{
	LPITEM item = ITEM_MANAGER::instance().FindByVID(dwVID);

	if (IsObserverMode())
		return false;

	if (!item || !item->GetSectree())
		return false;

	if (item->DistanceValid(this))
	{
		// @fixme150 BEGIN
		if (item->GetType() == ITEM_QUEST)
		{
			if (quest::CQuestManager::instance().GetPCForce(GetPlayerID())->IsRunning() == true)
			{
				ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot pickup this item if you're using quests"));
				return false;
			}
		}
		// @fixme150 END

		if (item->IsOwnership(this))
		{
			if (item->GetType() == ITEM_ELK)
			{
				GiveGold(item->GetCount());
				item->RemoveFromGround();

				M2_DESTROY_ITEM(item);

				Save();
			}
			// ����� �������̶��
			else
			{
				if (item->IsStackable() && !IS_SET(item->GetAntiFlag(), ITEM_ANTIFLAG_STACK))
				{
					WORD bCount = item->GetCount();

#ifdef ENABLE_CUSTOM_INVENTORY
				for (int i = 0; i < CUSTOM_INVENTORY_SLOT_END; ++i)
				{
					if(i >= INVENTORY_MAX_NUM && i < CUSTOM_INVENTORY_SLOT_START)
						continue;
#else
				for (int i = 0; i < INVENTORY_MAX_NUM; ++i)
				{
#endif	
						LPITEM item2 = GetInventoryItem(i);

						if (!item2)
							continue;

						if (item2->GetVnum() == item->GetVnum())
						{
							int j;

							for (j = 0; j < ITEM_SOCKET_MAX_NUM; ++j)
								if (item2->GetSocket(j) != item->GetSocket(j))
									break;

							if (j != ITEM_SOCKET_MAX_NUM)
								continue;

							WORD bCount2 = MIN(g_bItemCountLimit - item2->GetCount(), bCount);
							bCount -= bCount2;

							item2->SetCount(item2->GetCount() + bCount2);

							if (bCount == 0)
							{
								#ifdef ENABLE_SPECIAL_DROP_CHAT_RENEWAL
								SendPickupItemPacket(item2->GetVnum(), item->GetCount());
								#else
								ChatPacket(CHAT_TYPE_INFO, "[LS;444;%s]", item2->GetName());
								#endif
								M2_DESTROY_ITEM(item);
								if (item2->GetType() == ITEM_QUEST)
									quest::CQuestManager::instance().PickupItem (GetPlayerID(), item2);
								return true;
							}
						}
					}

					item->SetCount(bCount);
				}
				
				
				int iEmptyCell = GetEmptyInventoryEx(item);
				if (iEmptyCell == -1)
				{
					sys_log(0, "No empty inventory pid %u size %ud itemid %u", GetPlayerID(), item->GetSize(), item->GetID());
					ChatPacket(CHAT_TYPE_INFO, "[LS;445]");
					return false;
				}
			

				item->RemoveFromGround();
				item->AddToCharacter(this, TItemPos(item->GetWindowInventoryEx(), iEmptyCell));

				char szHint[32+1];
				snprintf(szHint, sizeof(szHint), "%s %u %u", item->GetName(), item->GetCount(), item->GetOriginalVnum());
				LogManager::instance().ItemLog(this, item, "GET", szHint);
				#ifdef ENABLE_SPECIAL_DROP_CHAT_RENEWAL
				SendPickupItemPacket(item->GetVnum(), item->GetCount());
				#else
				ChatPacket(CHAT_TYPE_INFO, "[LS;444;%s]", item->GetName());	
				#endif
				if (item->GetType() == ITEM_QUEST)
					quest::CQuestManager::instance().PickupItem (GetPlayerID(), item);
			}

			//Motion(MOTION_PICKUP);
			return true;
		}
		else if (!IS_SET(item->GetAntiFlag(), ITEM_ANTIFLAG_GIVE | ITEM_ANTIFLAG_DROP) && GetParty())
		{
			// �ٸ� ��Ƽ�� ������ �������� �������� �Ѵٸ�
			NPartyPickupDistribute::FFindOwnership funcFindOwnership(item);

			GetParty()->ForEachOnlineMember(funcFindOwnership);

			LPCHARACTER owner = funcFindOwnership.owner;
			// @fixme115
			if (!owner)
				return false;

			int iEmptyCell;

			if (item->IsDragonSoul())
			{
				if (!(owner && (iEmptyCell = owner->GetEmptyDragonSoulInventory(item)) != -1))
				{
					owner = this;

					if ((iEmptyCell = GetEmptyDragonSoulInventory(item)) == -1)
					{
						owner->ChatPacket(CHAT_TYPE_INFO, "[LS;445]");
						return false;
					}
				}
			}
			else
			{
				if (!(owner && (iEmptyCell = owner->GetEmptyInventory(item->GetSize())) != -1))
				{
					owner = this;

					if ((iEmptyCell = GetEmptyInventory(item->GetSize())) == -1)
					{
						owner->ChatPacket(CHAT_TYPE_INFO, "[LS;445]");
						return false;
					}
				}
			}
			item->RemoveFromGround();

			if (item->IsDragonSoul())
				item->AddToCharacter(owner, TItemPos(DRAGON_SOUL_INVENTORY, iEmptyCell));
			else
				item->AddToCharacter(owner, TItemPos(INVENTORY, iEmptyCell));

			char szHint[32+1];
			snprintf(szHint, sizeof(szHint), "%s %u %u", item->GetName(), item->GetCount(), item->GetOriginalVnum());
			LogManager::instance().ItemLog(owner, item, "GET", szHint);

			if (owner == this)
				#ifdef ENABLE_SPECIAL_DROP_CHAT_RENEWAL
				SendPickupItemPacket(item->GetVnum(), item->GetCount());
				#else
				ChatPacket(CHAT_TYPE_INFO, "[LS;444;%s]", item->GetName());
				#endif

			else
			{
				owner->ChatPacket(CHAT_TYPE_INFO, "[LS;446;%s;%s]", GetName(), item->GetName());
				ChatPacket(CHAT_TYPE_INFO, "[LS;449;%s;%s]", owner->GetName(), item->GetName());
			}

			if (item->GetType() == ITEM_QUEST)
				quest::CQuestManager::instance().PickupItem (owner->GetPlayerID(), item);

			return true;
		}
	}

	return false;
}

#ifdef ENABLE_CUSTOM_INVENTORY
bool CHARACTER::SwapItem(WORD bCell, WORD bDestCell)
#else	
bool CHARACTER::SwapItem(BYTE bCell, BYTE bDestCell)
#endif	
{
	if (!CanHandleItem())
		return false;

	TItemPos srcCell(INVENTORY, bCell), destCell(INVENTORY, bDestCell);

	// �ùٸ� Cell ���� �˻�
	// ��ȥ���� Swap�� �� �����Ƿ�, ���⼭ �ɸ�.
	//if (bCell >= INVENTORY_MAX_NUM + WEAR_MAX_NUM || bDestCell >= INVENTORY_MAX_NUM + WEAR_MAX_NUM)
	if (srcCell.IsDragonSoulEquipPosition() || destCell.IsDragonSoulEquipPosition())
		return false;

	// ���� CELL ���� �˻�
	if (bCell == bDestCell)
		return false;

	// �� �� ���â ��ġ�� Swap �� �� ����.
	if (srcCell.IsEquipPosition() && destCell.IsEquipPosition())
		return false;

	LPITEM item1, item2;

	// item2�� ���â�� �ִ� ���� �ǵ���.
	if (srcCell.IsEquipPosition())
	{
		item1 = GetInventoryItem(bDestCell);
		item2 = GetInventoryItem(bCell);
	}
	else
	{
		item1 = GetInventoryItem(bCell);
		item2 = GetInventoryItem(bDestCell);
	}

	if (!item1 || !item2)
		return false;

	if (item1 == item2)
	{
	    sys_log(0, "[WARNING][WARNING][HACK USER!] : %s %d %d", m_stName.c_str(), bCell, bDestCell);
	    return false;
	}

	// item2�� bCell��ġ�� �� �� �ִ��� Ȯ���Ѵ�.
	if (!IsEmptyItemGrid(TItemPos (INVENTORY, item1->GetCell()), item2->GetSize(), item1->GetCell()))
		return false;

	// �ٲ� �������� ���â�� ������
	if (TItemPos(EQUIPMENT, item2->GetCell()).IsEquipPosition())
	{
		uint16_t bEquipCell = item2->GetCell() - INVENTORY_MAX_NUM;
		uint16_t bInvenCell = item1->GetCell();

		// �������� �������� ���� �� �ְ�, ���� ���� �������� ���� ������ ���¿��߸� ����
		if (item2->IsDragonSoul() || item2->GetType() == ITEM_BELT) // @fixme117
		{
			if (false == CanUnequipNow(item2) || false == CanEquipNow(item1))
				return false;
		}

		if (bEquipCell != item1->FindEquipCell(this)) // ���� ��ġ�϶��� ���
			return false;	

		item2->RemoveFromCharacter();

		if (item1->EquipTo(this, bEquipCell))
		{
#if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
			item2->AddToCharacter(this, TItemPos(INVENTORY, bInvenCell), false);
#else
			item2->AddToCharacter(this, TItemPos(INVENTORY, bInvenCell));
#endif
		}
		else
		{
			sys_err("SwapItem cannot equip %s! item1 %s", item2->GetName(), item1->GetName());
		}
	}
	else
	{
		uint16_t bCell1 = item1->GetCell();
		uint16_t bCell2 = item2->GetCell();

		item1->RemoveFromCharacter();
		item2->RemoveFromCharacter();

#if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
		item1->AddToCharacter(this, TItemPos(INVENTORY, bCell2), false);
		item2->AddToCharacter(this, TItemPos(INVENTORY, bCell1), false);
#else
		item1->AddToCharacter(this, TItemPos(INVENTORY, bCell2));
		item2->AddToCharacter(this, TItemPos(INVENTORY, bCell1));
#endif
	}

	return true;
}

bool CHARACTER::UnequipItem(LPITEM item)
{
#ifdef ENABLE_WEAPON_COSTUME_SYSTEM
	int iWearCell = item->FindEquipCell(this);
	if (iWearCell == WEAR_WEAPON)
	{
		LPITEM costumeWeapon = GetWear(WEAR_COSTUME_WEAPON);
		if (costumeWeapon && !UnequipItem(costumeWeapon))
		{
			ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot unequip the costume weapon because there is not enough space"));
			return false;
		}
	}
#endif

	if (false == CanUnequipNow(item))
		return false;

	int pos = GetEmptyInventoryEx(item);

	// HARD CODING
	if (item->GetVnum() == UNIQUE_ITEM_HIDE_ALIGNMENT_TITLE)
		ShowAlignment(true);

	item->RemoveFromCharacter();
	item->AddToCharacter(this, TItemPos(item->GetWindowInventoryEx(), pos), false);
	// if (item->IsDragonSoul())
// #ifdef ENABLE_HIGHLIGHT_SYSTEM
		// item->AddToCharacter(this, TItemPos(DRAGON_SOUL_INVENTORY, pos), false);
// #else
		// item->AddToCharacter(this, TItemPos(DRAGON_SOUL_INVENTORY, pos));
// #endif
	// else
// #if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
		// item->AddToCharacter(this, TItemPos(INVENTORY, pos), false);
// #else
		// item->AddToCharacter(this, TItemPos(INVENTORY, pos));
// #endif

	CheckMaximumPoints();

	return true;
}

//
// @version	05/07/05 Bang2ni - Skill ����� 1.5 �� �̳��� ��� ���� ����
//
bool CHARACTER::EquipItem(LPITEM item, int iCandidateCell)
{
	if (item->IsExchanging())
		return false;

	if (false == item->IsEquipable())
		return false;

	if (false == CanEquipNow(item))
		return false;

	int iWearCell = item->FindEquipCell(this, iCandidateCell);

	if (iWearCell < 0)
		return false;

	// ���𰡸� ź ���¿��� �νõ� �Ա� ����
	if (iWearCell == WEAR_BODY && IsRiding() && (item->GetVnum() >= 11901 && item->GetVnum() <= 11904))
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;883]");
		return false;
	}

	if (iWearCell != WEAR_ARROW && IsPolymorphed())
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;450]");
		return false;
	}
	
	
#ifdef ENABLE_WEDDING_FIX
	LPITEM armor = GetWear(WEAR_BODY);
#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
	if ((item->GetSubType() == UNIQUE_SPECIAL_RIDE) && (iWearCell == WEAR_UNIQUE1 || iWearCell == WEAR_UNIQUE2))
	{
		if (armor && armor->GetVnum() >= 11901 && armor->GetVnum() <= 11904)
		{	
			ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Non puoi cavalcare con uno Smoking o Abito da Sposa."));
			return false;		
		}
	}

#else

	if (iWearCell == WEAR_COSTUME_MOUNT && (item->GetSubType() == COSTUME_MOUNT))
	{
		if (armor && armor->GetVnum() >= 11901 && armor->GetVnum() <= 11904)
		{	
			ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Non puoi cavalcare con uno Smoking o Abito da Sposa."));
			return false;		
		}
	}
#endif

	if (iWearCell == WEAR_COSTUME_BODY && (item->GetSubType() == COSTUME_BODY))
	{
		if (armor && armor->GetVnum() >= 11901 && armor->GetVnum() <= 11904)
		{	
			ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Non puoi usare un Costume con uno Smoking o Abito da Sposa."));
			return false;
		}
	}
	
	LPITEM CostumeBody = GetWear(WEAR_COSTUME_BODY);
	if (CostumeBody)
	{
		if (item->GetVnum() >= 11901 && item->GetVnum() <= 11904)
		{
			ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Devi rimuovere il Costume per usarlo."));
			return false;
		}
	}
#endif
	

	if (FN_check_item_sex(this, item) == false)
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;1005]");
		return false;
	}

	//�ű� Ż�� ���� ���� �� ��뿩�� üũ
	if(item->IsRideItem() && IsRiding())
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;1054]");
		return false;
	}
	
	
	if (item->GetType() == ITEM_ARMOR && item->GetSubType() == ARMOR_BODY && IsRiding())
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Info:> Nu poti schimba armura cand calaresti."));
		return false;
	}
	
	if (item->GetType() == ITEM_COSTUME && item->GetSubType() == COSTUME_BODY && IsRiding())
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Info:> Nu poti schimba costumul cand calaresti."));
		return false;
	}
	

	// ȭ�� �̿ܿ��� ������ ���� �ð� �Ǵ� ��ų ��� 1.5 �Ŀ� ��� ��ü�� ����
	DWORD dwCurTime = get_dword_time();

	if (iWearCell != WEAR_ARROW
		&& (dwCurTime - GetLastAttackTime() <= 1500 || dwCurTime - m_dwLastSkillTime <= 1500))
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;451]");
		return false;
	}
	
    // Unstack objects when equip[ITEM_UNIQUE]
    if (item->IsEquipable() && item->GetType() == ITEM_UNIQUE && item->GetCount() > 1) {
		auto pos = GetEmptyInventory(item->GetSize());
		if (pos == -1) {
			return false;
		}

		auto newItem = ITEM_MANAGER::Instance().CreateItem(item->GetVnum());
		if (!newItem)
			return false;

		item->SetCount(item->GetCount() - 1);
		newItem->AddToCharacter(this, TItemPos(INVENTORY, pos), true);

		item = newItem;
    }

#ifdef __FIX_COSTUM_NUNTA_PESTE_COSTUM_NORMAL__
	if (GetWear(WEAR_BODY) && GetWear(WEAR_BODY)->GetVnum() >= 11901 && GetWear(WEAR_BODY)->GetVnum() <= 11904 && 
		item->GetType() == ITEM_COSTUME && item->GetSubType() == COSTUME_BODY)
    {
        ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot wear a costume as long as you have equipped a wedding costume"));
        return false;
    }
   
    if (GetWear(WEAR_COSTUME_BODY) && item->GetVnum() >= 11901 && item->GetVnum() <= 11904)
    {
        ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot equip a wedding costume as long as you have a costume on"));
        return false;
    }
#endif

#ifdef ENABLE_WEAPON_COSTUME_SYSTEM
	if (iWearCell == WEAR_WEAPON)
	{
		if (item->GetType() == ITEM_WEAPON)
		{
			LPITEM costumeWeapon = GetWear(WEAR_COSTUME_WEAPON);
			if (costumeWeapon && costumeWeapon->GetValue(3) != item->GetSubType() && !UnequipItem(costumeWeapon))
			{
				ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot unequip the costume weapon because there is not enough space"));
				return false;
			}
		}
		else //fishrod/pickaxe
		{
			LPITEM costumeWeapon = GetWear(WEAR_COSTUME_WEAPON);
			if (costumeWeapon && !UnequipItem(costumeWeapon))
			{
				ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot unequip the costume weapon because there is not enough space"));
				return false;
			}
		}
	}
	else if (iWearCell == WEAR_COSTUME_WEAPON)
	{
		if (item->GetType() == ITEM_COSTUME && item->GetSubType() == COSTUME_WEAPON)
		{
			LPITEM pkWeapon = GetWear(WEAR_WEAPON);
			if (!pkWeapon || pkWeapon->GetType() != ITEM_WEAPON || item->GetValue(3) != pkWeapon->GetSubType())
			{
				ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot equip the costume weapon, because you have the wrong weapon equipped"));
				return false;
			}
		}
	}
#endif

	// ��ȥ�� Ư�� ó��
	if (item->IsDragonSoul())
	{
		// ���� Ÿ���� ��ȥ���� �̹� �� �ִٸ� ������ �� ����.
		// ��ȥ���� swap�� �����ϸ� �ȵ�.
		if(GetInventoryItem(INVENTORY_MAX_NUM + iWearCell))
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;1090]");
			return false;
		}

		if (!item->EquipTo(this, iWearCell))
		{
			return false;
		}
	}
	// ��ȥ���� �ƴ�.
	else
	{
		// ������ ���� �������� �ִٸ�,
		if (GetWear(iWearCell) && !IS_SET(GetWear(iWearCell)->GetFlag(), ITEM_FLAG_IRREMOVABLE))
		{
			// �� �������� �ѹ� ������ ���� �Ұ�. swap ���� ���� �Ұ�
			if (item->GetWearFlag() == WEARABLE_ABILITY)
				return false;

			if (false == SwapItem(item->GetCell(), INVENTORY_MAX_NUM + iWearCell))
			{
				return false;
			}
		}
		else
		{
			BYTE bOldCell = item->GetCell();

			if (item->EquipTo(this, iWearCell))
			{
				SyncQuickslot(QUICKSLOT_TYPE_ITEM, bOldCell, iWearCell);
			}
		}
	}

	if (true == item->IsEquipped())
	{
		// ������ ���� ��� ���ĺ��ʹ� ������� �ʾƵ� �ð��� �����Ǵ� ��� ó��.
		if (-1 != item->GetProto()->cLimitRealTimeFirstUseIndex)
		{
			// �� ���̶� ����� ���������� ���δ� Socket1�� ���� �Ǵ��Ѵ�. (Socket1�� ���Ƚ�� ���)
			if (0 == item->GetSocket(1))
			{
				// ��밡�ɽð��� Default ������ Limit Value ���� ����ϵ�, Socket0�� ���� ������ �� ���� ����ϵ��� �Ѵ�. (������ ��)
				long duration = (0 != item->GetSocket(0)) ? item->GetSocket(0) : item->GetProto()->aLimits[(unsigned char)(item->GetProto()->cLimitRealTimeFirstUseIndex)].lValue;

				if (0 == duration)
					duration = 60 * 60 * 24 * 7;

				item->SetSocket(0, time(0) + duration);
				item->StartRealTimeExpireEvent();
			}

			item->SetSocket(1, item->GetSocket(1) + 1);
		}

		if (item->GetVnum() == UNIQUE_ITEM_HIDE_ALIGNMENT_TITLE)
			ShowAlignment(false);

		const DWORD& dwVnum = item->GetVnum();

		// �󸶴� �̺�Ʈ �ʽ´��� ����(71135) ����� ����Ʈ �ߵ�
		if (true == CItemVnumHelper::IsRamadanMoonRing(dwVnum))
		{
			this->EffectPacket(SE_EQUIP_RAMADAN_RING);
		}
		// �ҷ��� ����(71136) ����� ����Ʈ �ߵ�
		else if (true == CItemVnumHelper::IsHalloweenCandy(dwVnum))
		{
			this->EffectPacket(SE_EQUIP_HALLOWEEN_CANDY);
		}
		// �ູ�� ����(71143) ����� ����Ʈ �ߵ�
		else if (true == CItemVnumHelper::IsHappinessRing(dwVnum))
		{
			this->EffectPacket(SE_EQUIP_HAPPINESS_RING);
		}
		// ����� �Ҵ�Ʈ(71145) ����� ����Ʈ �ߵ�
		else if (true == CItemVnumHelper::IsLovePendant(dwVnum))
		{
			this->EffectPacket(SE_EQUIP_LOVE_PENDANT);
		}
		// ITEM_UNIQUE�� ���, SpecialItemGroup�� ���ǵǾ� �ְ�, (item->GetSIGVnum() != NULL)
		//
#ifdef __SASH_SYSTEM__
		else if ((item->GetType() == ITEM_COSTUME) && (item->GetSubType() == COSTUME_SASH))
#ifdef ENABLE_WEDDING_FIX			
			if (armor && armor->GetVnum() != 11901 && armor->GetVnum() != 11902 && armor->GetVnum() != 11903 && armor->GetVnum() != 11904)
				this->EffectPacket(SE_EFFECT_SASH_EQUIP);
			else if (!(armor))
#endif				
				this->EffectPacket(SE_EFFECT_SASH_EQUIP);	
#endif
		else if ((ITEM_UNIQUE == item->GetType() || ITEM_RING == item->GetType()) && 0 != item->GetSIGVnum())
		{
			const CSpecialItemGroup* pGroup = ITEM_MANAGER::instance().GetSpecialItemGroup(item->GetSIGVnum());
			if (NULL != pGroup)
			{
				const CSpecialAttrGroup* pAttrGroup = ITEM_MANAGER::instance().GetSpecialAttrGroup(pGroup->GetAttrVnum(item->GetVnum()));
				if (NULL != pAttrGroup)
				{
					const std::string& std = pAttrGroup->m_stEffectFileName;
					SpecificEffectPacket(std.c_str());
				}
			}
		}

		if (
			(ITEM_UNIQUE == item->GetType() && UNIQUE_SPECIAL_RIDE == item->GetSubType() && IS_SET(item->GetFlag(), ITEM_FLAG_QUEST_USE))
			|| (ITEM_UNIQUE == item->GetType() && UNIQUE_SPECIAL_MOUNT_RIDE == item->GetSubType() && IS_SET(item->GetFlag(), ITEM_FLAG_QUEST_USE))
#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
			|| (ITEM_COSTUME == item->GetType() && COSTUME_MOUNT == item->GetSubType())
#endif
		)
		{
			quest::CQuestManager::instance().UseItem(GetPlayerID(), item, false);
		}

	}

	return true;
}

void CHARACTER::BuffOnAttr_AddBuffsFromItem(LPITEM pItem)
{
	for (size_t i = 0; i < sizeof(g_aBuffOnAttrPoints)/sizeof(g_aBuffOnAttrPoints[0]); i++)
	{
		TMapBuffOnAttrs::iterator it = m_map_buff_on_attrs.find(g_aBuffOnAttrPoints[i]);
		if (it != m_map_buff_on_attrs.end())
		{
			it->second->AddBuffFromItem(pItem);
		}
	}
}

void CHARACTER::BuffOnAttr_RemoveBuffsFromItem(LPITEM pItem)
{
	for (size_t i = 0; i < sizeof(g_aBuffOnAttrPoints)/sizeof(g_aBuffOnAttrPoints[0]); i++)
	{
		TMapBuffOnAttrs::iterator it = m_map_buff_on_attrs.find(g_aBuffOnAttrPoints[i]);
		if (it != m_map_buff_on_attrs.end())
		{
			it->second->RemoveBuffFromItem(pItem);
		}
	}
}

void CHARACTER::BuffOnAttr_ClearAll()
{
	for (TMapBuffOnAttrs::iterator it = m_map_buff_on_attrs.begin(); it != m_map_buff_on_attrs.end(); it++)
	{
		CBuffOnAttributes* pBuff = it->second;
		if (pBuff)
		{
			pBuff->Initialize();
		}
	}
}

void CHARACTER::BuffOnAttr_ValueChange(BYTE bType, BYTE bOldValue, BYTE bNewValue)
{
	TMapBuffOnAttrs::iterator it = m_map_buff_on_attrs.find(bType);

	if (0 == bNewValue)
	{
		if (m_map_buff_on_attrs.end() == it)
			return;
		else
			it->second->Off();
	}
	else if(0 == bOldValue)
	{
		CBuffOnAttributes* pBuff = NULL;
		if (m_map_buff_on_attrs.end() == it)
		{
			switch (bType)
			{
			case POINT_ENERGY:
				{
					static BYTE abSlot[] = { WEAR_BODY, WEAR_HEAD, WEAR_FOOTS, WEAR_WRIST, WEAR_WEAPON, WEAR_NECK, WEAR_EAR, WEAR_SHIELD };
					static std::vector <BYTE> vec_slots (abSlot, abSlot + _countof(abSlot));
					pBuff = M2_NEW CBuffOnAttributes(this, bType, &vec_slots);
				}
				break;
			case POINT_COSTUME_ATTR_BONUS:
				{
					static BYTE abSlot[] = {
						WEAR_COSTUME_BODY,
						WEAR_COSTUME_HAIR,
#ifdef ENABLE_WEAPON_COSTUME_SYSTEM
						WEAR_COSTUME_WEAPON,
#endif
					};
					static std::vector <BYTE> vec_slots (abSlot, abSlot + _countof(abSlot));
					pBuff = M2_NEW CBuffOnAttributes(this, bType, &vec_slots);
				}
				break;
			default:
				break;
			}
			m_map_buff_on_attrs.insert(TMapBuffOnAttrs::value_type(bType, pBuff));

		}
		else
			pBuff = it->second;
		if (pBuff != NULL)
			pBuff->On(bNewValue);
	}
	else
	{
		assert (m_map_buff_on_attrs.end() != it);
		it->second->ChangeBuffValue(bNewValue);
	}
}

/*
LPITEM CHARACTER::FindSpecifyItem(DWORD vnum) const
{
	for (int i = 0; i < INVENTORY_MAX_NUM; ++i)
		if (GetInventoryItem(i) && GetInventoryItem(i)->GetVnum() == vnum)
			return GetInventoryItem(i);

	return NULL;
}*/

LPITEM CHARACTER::FindSpecifyItem(DWORD vnum) const
{
#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	for (int i = 0; i < Inventory_Size(); ++i)
#else
	for (int i = 0; i < INVENTORY_MAX_NUM; ++i)	
#endif
	if (GetInventoryItem(i) && GetInventoryItem(i)->GetVnum() == vnum)
		return GetInventoryItem(i);
	
#ifdef ENABLE_CUSTOM_INVENTORY	
	for (int j = CUSTOM_INVENTORY_SLOT_START; j < CUSTOM_INVENTORY_SLOT_END; ++j)
	{
		if (GetInventoryItem(j) && GetInventoryItem(j)->GetVnum() == vnum)
			return GetInventoryItem(j);	
	}
#endif

	return NULL;
}
/*
LPITEM CHARACTER::FindItemByID(DWORD id) const
{
	for (int i=0 ; i < INVENTORY_MAX_NUM ; ++i)
	{
		if (NULL != GetInventoryItem(i) && GetInventoryItem(i)->GetID() == id)
			return GetInventoryItem(i);
	}

	for (int i=BELT_INVENTORY_SLOT_START; i < BELT_INVENTORY_SLOT_END ; ++i)
	{
		if (NULL != GetInventoryItem(i) && GetInventoryItem(i)->GetID() == id)
			return GetInventoryItem(i);
	}

	return NULL;
}
*/

LPITEM CHARACTER::FindItemByID(DWORD id) const
{
#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	for (int i = 0; i < Inventory_Size(); ++i)
#else
	for (int i = 0; i < INVENTORY_MAX_NUM; ++i)	
#endif
	{
		if (NULL != GetInventoryItem(i) && GetInventoryItem(i)->GetID() == id)
			return GetInventoryItem(i);
	}

	for (int i=BELT_INVENTORY_SLOT_START; i < BELT_INVENTORY_SLOT_END ; ++i)
	{
		if (NULL != GetInventoryItem(i) && GetInventoryItem(i)->GetID() == id)
			return GetInventoryItem(i);
	}
	
#ifdef ENABLE_CUSTOM_INVENTORY	
	for (int j = CUSTOM_INVENTORY_SLOT_START; j < CUSTOM_INVENTORY_SLOT_END; ++j)
	{
		if (NULL != GetInventoryItem(j) && GetInventoryItem(j)->GetID() == id)
			return GetInventoryItem(j);
	}
#endif

	return NULL;
}
int CHARACTER::CountSpecifyItem(DWORD vnum) const
{
	
	int	count = 0;
	LPITEM item;
	
	const LPPRIVATE_SHOP pPrivateShop = CPrivateShopManager::Instance().GetPrivateShop(GetPlayerID());
	
#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	for (int i = 0; i < Inventory_Size(); ++i)
#else
	for (int i = 0; i < INVENTORY_MAX_NUM; ++i)	
#endif
	{
		item = GetInventoryItem(i);
		if (NULL != item && item->GetVnum() == vnum)
		{
#ifdef __PREMIUM_PRIVATE_SHOP__
			if (pPrivateShop)
				if (pPrivateShop->HasItemByID(item->GetID()))
					continue;
#endif
			if (m_pkMyShop && m_pkMyShop->IsSellingItem(item->GetID()))
			{
				continue;
			}
			else
			{
				count += item->GetCount();
			}
		}
	}

#ifdef ENABLE_CUSTOM_INVENTORY
	for (int j = CUSTOM_INVENTORY_SLOT_START; j < CUSTOM_INVENTORY_SLOT_END; ++j)
	{
		// if(j == iExceptionCell)
			// continue;

		item = GetInventoryItem(j);
		if (NULL != item && item->GetVnum() == vnum)
		{
			if (m_pkMyShop && m_pkMyShop->IsSellingItem(item->GetID()))
			{
				continue;
			}
			else
			{
				count += item->GetCount();
			}
		}
	}
#endif	

	return count;
}

void CHARACTER::RemoveSpecifyItem(DWORD vnum, DWORD count)
{
	if (0 == count)
		return;


#ifdef __PREMIUM_PRIVATE_SHOP__
	const LPPRIVATE_SHOP pPrivateShop = CPrivateShopManager::Instance().GetPrivateShop(GetPlayerID());
#endif

#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	for (UINT i = 0; i < Inventory_Size(); ++i)
#else
	for (UINT i = 0; i < INVENTORY_MAX_NUM; ++i)
#endif	
	{
		if (NULL == GetInventoryItem(i))
			continue;

		if (GetInventoryItem(i)->GetVnum() != vnum)
			continue;
		
	#ifdef __PREMIUM_PRIVATE_SHOP__
		if (pPrivateShop)
			if (pPrivateShop->HasItemByID(GetInventoryItem(i)->GetID()))
				continue;
#endif

		//���� ������ ��ϵ� �����̸� �Ѿ��. (���� �������� �Ǹŵɶ� �� �κ����� ���� ��� ����!)
		if(m_pkMyShop)
		{
			bool isItemSelling = m_pkMyShop->IsSellingItem(GetInventoryItem(i)->GetID());
			if (isItemSelling)
				continue;
		}

		if (vnum >= 80003 && vnum <= 80007)
			LogManager::instance().GoldBarLog(GetPlayerID(), GetInventoryItem(i)->GetID(), QUEST, "RemoveSpecifyItem");

		if (count >= GetInventoryItem(i)->GetCount())
		{
			count -= GetInventoryItem(i)->GetCount();
			GetInventoryItem(i)->SetCount(0);

			if (0 == count)
				return;
		}
		else
		{
			GetInventoryItem(i)->SetCount(GetInventoryItem(i)->GetCount() - count);
			return;
		}
	}
	
#ifdef ENABLE_CUSTOM_INVENTORY
	for (UINT j = CUSTOM_INVENTORY_SLOT_START; j < CUSTOM_INVENTORY_SLOT_END; ++j)
	{
		// if(j == iExceptionCell)
			// continue;

		if (NULL == GetInventoryItem(j))
			continue;

		if (GetInventoryItem(j)->GetVnum() != vnum)
			continue;

		if(m_pkMyShop)
		{
			bool isItemSelling = m_pkMyShop->IsSellingItem(GetInventoryItem(j)->GetID());
			if (isItemSelling)
				continue;
		}

		if (count >= GetInventoryItem(j)->GetCount())
		{
			count -= GetInventoryItem(j)->GetCount();
			GetInventoryItem(j)->SetCount(0);

			if (0 == count)
				return;
		}
		else
		{
			GetInventoryItem(j)->SetCount(GetInventoryItem(j)->GetCount() - count);
			return;
		}
	}
#endif

	if (count)
		sys_log(0, "CHARACTER::RemoveSpecifyItem cannot remove enough item vnum %u, still remain %d", vnum, count);
}
/*
int CHARACTER::CountSpecifyTypeItem(BYTE type) const
{
	int	count = 0;

	for (int i = 0; i < INVENTORY_MAX_NUM; ++i)
	{
		LPITEM pItem = GetInventoryItem(i);
		if (pItem != NULL && pItem->GetType() == type)
		{
			count += pItem->GetCount();
		}
	}

	return count;
}*/

int CHARACTER::CountSpecifyTypeItem(BYTE type) const
{
	int	count = 0;
#ifdef __PREMIUM_PRIVATE_SHOP__
	const LPPRIVATE_SHOP pPrivateShop = CPrivateShopManager::Instance().GetPrivateShop(GetPlayerID());
#endif
#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	for (UINT i = 0; i < Inventory_Size(); ++i)
#else
	for (UINT i = 0; i < INVENTORY_MAX_NUM; ++i)
#endif
	{
		LPITEM pItem = GetInventoryItem(i);
		if (pItem != NULL && pItem->GetType() == type)
		{
#ifdef __PREMIUM_PRIVATE_SHOP__
			if (pPrivateShop)
				if (pPrivateShop->HasItemByID(pItem->GetID()))
					continue;
#endif
			
			count += pItem->GetCount();
		}
	}

	return count;
}

void CHARACTER::RemoveSpecifyTypeItem(BYTE type, DWORD count)
{
	if (0 == count)
		return;

	//for (UINT i = 0; i < INVENTORY_MAX_NUM; ++i)
#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	for (UINT i = 0; i < Inventory_Size(); ++i)
#else
	for (UINT i = 0; i < INVENTORY_MAX_NUM; ++i)
#endif	
	{
		if (NULL == GetInventoryItem(i))
			continue;

		if (GetInventoryItem(i)->GetType() != type)
			continue;

		//���� ������ ��ϵ� �����̸� �Ѿ��. (���� �������� �Ǹŵɶ� �� �κ����� ���� ��� ����!)
		if(m_pkMyShop)
		{
			bool isItemSelling = m_pkMyShop->IsSellingItem(GetInventoryItem(i)->GetID());
			if (isItemSelling)
				continue;
		}

		if (count >= GetInventoryItem(i)->GetCount())
		{
			count -= GetInventoryItem(i)->GetCount();
			GetInventoryItem(i)->SetCount(0);

			if (0 == count)
				return;
		}
		else
		{
			GetInventoryItem(i)->SetCount(GetInventoryItem(i)->GetCount() - count);
			return;
		}
	}
}

void CHARACTER::AutoGiveItem(LPITEM item, bool longOwnerShip)
{
	if (NULL == item)
	{
		sys_err("NULL point");
		return;
	}

	if (item->GetOwner())
	{
		sys_err("item %d 's owner exists!", item->GetID());
		return;
	}

	int cell = GetEmptyInventoryEx(item);
	if (cell != -1)
	{
		item->AddToCharacter(this, TItemPos(item->GetWindowInventoryEx(), cell));
		
		LogManager::instance().ItemLog(this, item, "SYSTEM", item->GetName());

		if (item->GetType() == ITEM_USE && item->GetSubType() == USE_POTION)
		{
			TQuickslot * pSlot;
			for (int i = 0; i < QUICKSLOT_MAX_NUM; ++i)
				if (GetQuickslot(i, &pSlot) && pSlot->type == QUICKSLOT_TYPE_NONE)
				{
					TQuickslot slot;
					slot.type = QUICKSLOT_TYPE_ITEM;
					slot.pos = cell;
					SetQuickslot(i, slot);
					break;
				}
		}
	}
	else
	{
		item->AddToGround(GetMapIndex(), GetXYZ());
#ifdef ENABLE_NEWSTUFF
		item->StartDestroyEvent(g_aiItemDestroyTime[ITEM_DESTROY_TIME_AUTOGIVE]);
#else
		item->StartDestroyEvent();
#endif


		if (longOwnerShip)
			item->SetOwnership(this, 300);
		else
			item->SetOwnership(this, 60);
		LogManager::instance().ItemLog(this, item, "SYSTEM_DROP", item->GetName());
	}
}


LPITEM CHARACTER::AutoGiveItem(DWORD dwItemVnum, WORD bCount, int iRarePct, bool bMsg)
{
	TItemTable * p = ITEM_MANAGER::instance().GetTable(dwItemVnum);

	if (!p)
		return NULL;

	DBManager::instance().SendMoneyLog(MONEY_LOG_DROP, dwItemVnum, bCount);

	if (p->dwFlags & ITEM_FLAG_STACKABLE && p->bType != ITEM_BLEND && p->bType != ITEM_TOGGLE) // Toggle Items must be skipped cuz of socket comaparing.
	{
#ifdef ENABLE_CUSTOM_INVENTORY
		for (int i = 0; i < CUSTOM_INVENTORY_SLOT_END; ++i)
		{
			if(i >= INVENTORY_MAX_NUM && i < CUSTOM_INVENTORY_SLOT_START)
				continue;
#else
		for (int i = 0; i < INVENTORY_MAX_NUM; ++i)
		{
#endif
			LPITEM item = GetInventoryItem(i);

			if (!item)
				continue;

			if (item->GetVnum() == dwItemVnum && FN_check_item_socket(item))
			{
				if (IS_SET(p->dwFlags, ITEM_FLAG_MAKECOUNT))
				{
					if (bCount < p->alValues[1])
						bCount = p->alValues[1];
				}

				WORD bCount2 = MIN(g_bItemCountLimit - item->GetCount(), bCount);
				bCount -= bCount2;

				item->SetCount(item->GetCount() + bCount2);

				if (bCount == 0)
				{
					if (bMsg)
						#ifdef ENABLE_SPECIAL_DROP_CHAT_RENEWAL
						SendPickupItemPacket(item->GetVnum(), item->GetCount());
						#else
						ChatPacket(CHAT_TYPE_INFO, "[LS;444;%s]", item->GetName());	
						#endif

					return item;
				}
			}
		}
	}

	LPITEM item = ITEM_MANAGER::instance().CreateItem(dwItemVnum, bCount, 0, true);

	if (!item)
	{
		sys_err("cannot create item by vnum %u (name: %s)", dwItemVnum, GetName());
		return NULL;
	}

	if (item->GetType() == ITEM_BLEND)
	{
		for (int i=0; i < INVENTORY_MAX_NUM; i++)
		{
			LPITEM inv_item = GetInventoryItem(i);

			if (inv_item == NULL) continue;

			if (inv_item->GetType() == ITEM_BLEND)
			{
				if (inv_item->GetVnum() == item->GetVnum())
				{
					if (inv_item->GetSocket(0) == item->GetSocket(0) &&
						inv_item->GetSocket(1) == item->GetSocket(1) &&
#ifndef ENABLE_AFFECT_RENEWAL
						inv_item->GetSocket(2) == item->GetSocket(2) &&
#endif
						inv_item->GetCount() < g_bItemCountLimit)
					{
#ifndef ENABLE_AFFECT_RENEWAL
						inv_item->SetSocket(2, inv_item->GetSocket(2) + item->GetSocket(2));
#else
						inv_item->SetCount(inv_item->GetCount() + item->GetCount());
#endif
						M2_DESTROY_ITEM(item); // FIXME
						return inv_item;
					}
				}
			}
		}
	}

	int iEmptyCell = GetEmptyInventoryEx(item);
	if (iEmptyCell != -1)
	{
		if (bMsg)
			#ifdef ENABLE_SPECIAL_DROP_CHAT_RENEWAL
			SendPickupItemPacket(item->GetVnum(), item->GetCount());
			#else
			ChatPacket(CHAT_TYPE_INFO, "[LS;444;%s]", item->GetName());
			#endif


		item->AddToCharacter(this, TItemPos(item->GetWindowInventoryEx(), iEmptyCell));
		
		LogManager::instance().ItemLog(this, item, "SYSTEM", item->GetName());

		if (item->GetType() == ITEM_USE && item->GetSubType() == USE_POTION)
		{
			TQuickslot * pSlot;
			for (int i = 0; i < QUICKSLOT_MAX_NUM; ++i)
				if (GetQuickslot(i, &pSlot) && pSlot->type == QUICKSLOT_TYPE_NONE)
				{
					TQuickslot slot;
					slot.type = QUICKSLOT_TYPE_ITEM;
					slot.pos = iEmptyCell;
					SetQuickslot(i, slot);
					break;
				}
		}
	}
	else
	{
		item->AddToGround(GetMapIndex(), GetXYZ());
#ifdef ENABLE_NEWSTUFF
		item->StartDestroyEvent(g_aiItemDestroyTime[ITEM_DESTROY_TIME_AUTOGIVE]);
#else
		item->StartDestroyEvent();
#endif
		// ��Ƽ ��� flag�� �ɷ��ִ� �������� ���,
		// �κ��� �� ������ ��� ��¿ �� ���� ����Ʈ���� �Ǹ�,
		// ownership�� �������� ����� ������(300��) �����Ѵ�.
		if (IS_SET(item->GetAntiFlag(), ITEM_ANTIFLAG_DROP))
			item->SetOwnership(this, 300);
		else
			item->SetOwnership(this, 60);
		LogManager::instance().ItemLog(this, item, "SYSTEM_DROP", item->GetName());
	}

	sys_log(0,
		"7: %d %d", dwItemVnum, bCount);
	return item;
}

bool CHARACTER::GiveItem(LPCHARACTER victim, TItemPos Cell)
{
	if (!CanHandleItem())
		return false;

	// @fixme150 BEGIN
	if (quest::CQuestManager::instance().GetPCForce(GetPlayerID())->IsRunning() == true)
	{
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot take this item if you're using quests"));
		return false;
	}
	// @fixme150 END

	LPITEM item = GetItem(Cell);

	if (item && !item->IsExchanging())
	{
		if (victim->CanReceiveItem(this, item))
		{
			victim->ReceiveItem(this, item);
			return true;
		}
	}

	return false;
}

bool CHARACTER::CanReceiveItem(LPCHARACTER from, LPITEM item) const
{
	if (IsPC())
		return false;

	// TOO_LONG_DISTANCE_EXCHANGE_BUG_FIX
	if (DISTANCE_APPROX(GetX() - from->GetX(), GetY() - from->GetY()) > 2000)
		return false;
	// END_OF_TOO_LONG_DISTANCE_EXCHANGE_BUG_FIX

	switch (GetRaceNum())
	{
		case fishing::CAMPFIRE_MOB:
			if (item->GetType() == ITEM_FISH &&
					(item->GetSubType() == FISH_ALIVE || item->GetSubType() == FISH_DEAD))
				return true;
			break;

		case fishing::FISHER_MOB:
			if (item->GetType() == ITEM_ROD)
				return true;
			break;

			// BUILDING_NPC
		case BLACKSMITH_WEAPON_MOB:
		case DEVILTOWER_BLACKSMITH_WEAPON_MOB:
			if (item->GetType() == ITEM_WEAPON &&
					item->GetRefinedVnum())
				return true;
			else
				return false;
			break;

		case BLACKSMITH_ARMOR_MOB:
		case DEVILTOWER_BLACKSMITH_ARMOR_MOB:
			if (item->GetType() == ITEM_ARMOR &&
					(item->GetSubType() == ARMOR_BODY || item->GetSubType() == ARMOR_SHIELD || item->GetSubType() == ARMOR_HEAD) &&
					item->GetRefinedVnum())
				return true;
			else
				return false;
			break;

		case BLACKSMITH_ACCESSORY_MOB:
		case DEVILTOWER_BLACKSMITH_ACCESSORY_MOB:
			if (item->GetType() == ITEM_ARMOR &&
					!(item->GetSubType() == ARMOR_BODY || item->GetSubType() == ARMOR_SHIELD || item->GetSubType() == ARMOR_HEAD) &&
					item->GetRefinedVnum())
				return true;
			else
				return false;
			break;
			// END_OF_BUILDING_NPC

		case BLACKSMITH_MOB:
			if (item->GetRefinedVnum() && item->GetRefineSet() < 500)
			{
				return true;
			}
			else
			{
				return false;
			}

		case BLACKSMITH2_MOB:
			if (item->GetRefineSet() >= 500)
			{
				return true;
			}
			else
			{
				return false;
			}

		case ALCHEMIST_MOB:
			if (item->GetRefinedVnum())
				return true;
			break;

		case 20101:
		case 20102:
		case 20103:
			// �ʱ� ��
			if (item->GetVnum() == ITEM_REVIVE_HORSE_1)
			{
				if (!IsDead())
				{
					from->ChatPacket(CHAT_TYPE_INFO, "[LS;452]");
					return false;
				}
				return true;
			}
			else if (item->GetVnum() == ITEM_HORSE_FOOD_1)
			{
				if (IsDead())
				{
					from->ChatPacket(CHAT_TYPE_INFO, "[LS;453]");
					return false;
				}
				return true;
			}
			else if (item->GetVnum() == ITEM_HORSE_FOOD_2 || item->GetVnum() == ITEM_HORSE_FOOD_3)
			{
				return false;
			}
			break;
		case 20104:
		case 20105:
		case 20106:
			// �߱� ��
			if (item->GetVnum() == ITEM_REVIVE_HORSE_2)
			{
				if (!IsDead())
				{
					from->ChatPacket(CHAT_TYPE_INFO, "[LS;452]");
					return false;
				}
				return true;
			}
			else if (item->GetVnum() == ITEM_HORSE_FOOD_2)
			{
				if (IsDead())
				{
					from->ChatPacket(CHAT_TYPE_INFO, "[LS;453]");
					return false;
				}
				return true;
			}
			else if (item->GetVnum() == ITEM_HORSE_FOOD_1 || item->GetVnum() == ITEM_HORSE_FOOD_3)
			{
				return false;
			}
			break;
		case 20107:
		case 20108:
		case 20109:
			// ���� ��
			if (item->GetVnum() == ITEM_REVIVE_HORSE_3)
			{
				if (!IsDead())
				{
					from->ChatPacket(CHAT_TYPE_INFO, "[LS;452]");
					return false;
				}
				return true;
			}
			else if (item->GetVnum() == ITEM_HORSE_FOOD_3)
			{
				if (IsDead())
				{
					from->ChatPacket(CHAT_TYPE_INFO, "[LS;453]");
					return false;
				}
				return true;
			}
			else if (item->GetVnum() == ITEM_HORSE_FOOD_1 || item->GetVnum() == ITEM_HORSE_FOOD_2)
			{
				return false;
			}
			break;
	}

	//if (IS_SET(item->GetFlag(), ITEM_FLAG_QUEST_GIVE))
	{
		return true;
	}

	return false;
}

void CHARACTER::ReceiveItem(LPCHARACTER from, LPITEM item)
{
	if (IsPC())
		return;

	switch (GetRaceNum())
	{
		case fishing::CAMPFIRE_MOB:
			if (item->GetType() == ITEM_FISH && (item->GetSubType() == FISH_ALIVE || item->GetSubType() == FISH_DEAD))
				fishing::Grill(from, item);
			else
			{
				// TAKE_ITEM_BUG_FIX
				from->SetQuestNPCID(GetVID());
				// END_OF_TAKE_ITEM_BUG_FIX
				quest::CQuestManager::instance().TakeItem(from->GetPlayerID(), GetRaceNum(), item);
			}
			break;

#ifdef __MELEY_LAIR_DUNGEON__
		case MeleyLair::STATUE_VNUM:
			{
				if (MeleyLair::CMgr::instance().IsMeleyMap(from->GetMapIndex()))
					MeleyLair::CMgr::instance().OnKillStatue(item, from, this, from->GetGuild());
			}
			break;
#endif
		
			// DEVILTOWER_NPC
		case DEVILTOWER_BLACKSMITH_WEAPON_MOB:
		case DEVILTOWER_BLACKSMITH_ARMOR_MOB:
		case DEVILTOWER_BLACKSMITH_ACCESSORY_MOB:
			if (item->GetRefinedVnum() != 0 && item->GetRefineSet() != 0 && item->GetRefineSet() < 500)
			{
				from->SetRefineNPC(this);
				from->RefineInformation(item->GetCell(), REFINE_TYPE_MONEY_ONLY);
			}
			else
			{
				from->ChatPacket(CHAT_TYPE_INFO, "[LS;1002]");
			}
			break;
			// END_OF_DEVILTOWER_NPC

		case BLACKSMITH_MOB:
		case BLACKSMITH2_MOB:
		case BLACKSMITH_WEAPON_MOB:
		case BLACKSMITH_ARMOR_MOB:
		case BLACKSMITH_ACCESSORY_MOB:
			if (item->GetRefinedVnum())
			{
				from->SetRefineNPC(this);
				from->RefineInformation(item->GetCell(), REFINE_TYPE_NORMAL);
			}
			else
			{
				from->ChatPacket(CHAT_TYPE_INFO, "[LS;1002]");
			}
			break;

		case 20101:
		case 20102:
		case 20103:
		case 20104:
		case 20105:
		case 20106:
		case 20107:
		case 20108:
		case 20109:
			if (item->GetVnum() == ITEM_REVIVE_HORSE_1 ||
					item->GetVnum() == ITEM_REVIVE_HORSE_2 ||
					item->GetVnum() == ITEM_REVIVE_HORSE_3)
			{
				from->ReviveHorse();
				item->SetCount(item->GetCount()-1);
				from->ChatPacket(CHAT_TYPE_INFO, "[LS;452]");
			}
			else if (item->GetVnum() == ITEM_HORSE_FOOD_1 ||
					item->GetVnum() == ITEM_HORSE_FOOD_2 ||
					item->GetVnum() == ITEM_HORSE_FOOD_3)
			{
				from->FeedHorse();
				from->ChatPacket(CHAT_TYPE_INFO, "[LS;453]");
				item->SetCount(item->GetCount()-1);
				EffectPacket(SE_HPUP_RED);
			}
			break;

		default:
			sys_log(0, "TakeItem %s %d %s", from->GetName(), GetRaceNum(), item->GetName());
			from->SetQuestNPCID(GetVID());
			quest::CQuestManager::instance().TakeItem(from->GetPlayerID(), GetRaceNum(), item);
			break;
	}
}

bool CHARACTER::IsEquipUniqueItem(DWORD dwItemVnum) const
{
	{
		LPITEM u = GetWear(WEAR_UNIQUE1);

		if (u && u->GetVnum() == dwItemVnum)
			return true;
	}

	{
		LPITEM u = GetWear(WEAR_UNIQUE2);

		if (u && u->GetVnum() == dwItemVnum)
			return true;
	}

	// �������� ��� ������(�ߺ�) ������ üũ�Ѵ�.
	if (dwItemVnum == UNIQUE_ITEM_RING_OF_LANGUAGE)
		return IsEquipUniqueItem(UNIQUE_ITEM_RING_OF_LANGUAGE_SAMPLE);

	return false;
}

// CHECK_UNIQUE_GROUP
bool CHARACTER::IsEquipUniqueGroup(DWORD dwGroupVnum) const
{
	{
		LPITEM u = GetWear(WEAR_UNIQUE1);

		if (u && u->GetSpecialGroup() == (int) dwGroupVnum)
			return true;
	}

	{
		LPITEM u = GetWear(WEAR_UNIQUE2);

		if (u && u->GetSpecialGroup() == (int) dwGroupVnum)
			return true;
	}

	return false;
}
// END_OF_CHECK_UNIQUE_GROUP

void CHARACTER::SetRefineMode(int iAdditionalCell)
{
	m_iRefineAdditionalCell = iAdditionalCell;
	m_bUnderRefine = true;
}

void CHARACTER::ClearRefineMode()
{
	m_bUnderRefine = false;
	SetRefineNPC( NULL );
}

bool CHARACTER::GiveItemFromSpecialItemGroup(DWORD dwGroupNum, std::vector<DWORD> &dwItemVnums,
											std::vector<DWORD> &dwItemCounts, std::vector <LPITEM> &item_gets, int &count)
{
	const CSpecialItemGroup* pGroup = ITEM_MANAGER::instance().GetSpecialItemGroup(dwGroupNum);

	if (!pGroup)
	{
		sys_err("cannot find special item group %d", dwGroupNum);
		return false;
	}

	std::vector <int> idxes;
	int n = pGroup->GetMultiIndex(idxes);

	bool bSuccess;

	for (int i = 0; i < n; i++)
	{
		bSuccess = false;
		int idx = idxes[i];
		DWORD dwVnum = pGroup->GetVnum(idx);
#ifdef ENABLE_REMOVE_LIMIT_GOLD	
		int64_t dwCount = (int64_t)pGroup->GetCount(idx);
#else
		DWORD dwCount = pGroup->GetCount(idx);
#endif
		int	iRarePct = pGroup->GetRarePct(idx);
		LPITEM item_get = NULL;
		switch (dwVnum)
		{
			case CSpecialItemGroup::GOLD:
#ifdef ENABLE_REMOVE_LIMIT_GOLD
				ChangeGold(dwCount);
#else
				PointChange(POINT_GOLD, dwCount);
#endif
				LogManager::instance().CharLog(this, dwCount, "TREASURE_GOLD", "");

				bSuccess = true;
				break;
			case CSpecialItemGroup::EXP:
				{
					PointChange(POINT_EXP, dwCount);
					LogManager::instance().CharLog(this, dwCount, "TREASURE_EXP", "");

					bSuccess = true;
				}
				break;

			case CSpecialItemGroup::MOB:
				{
					sys_log(0, "CSpecialItemGroup::MOB %d", dwCount);
					int x = GetX() + number(-500, 500);
					int y = GetY() + number(-500, 500);

					LPCHARACTER ch = CHARACTER_MANAGER::instance().SpawnMob(dwCount, GetMapIndex(), x, y, 0, true, -1);
					if (ch)
						ch->SetAggressive();
					bSuccess = true;
				}
				break;
			case CSpecialItemGroup::SLOW:
				{
					sys_log(0, "CSpecialItemGroup::SLOW %d", -(int)dwCount);
					AddAffect(AFFECT_SLOW, POINT_MOV_SPEED, -(int)dwCount, AFF_SLOW, 300, 0, true);
					bSuccess = true;
				}
				break;
			case CSpecialItemGroup::DRAIN_HP:
				{
					int iDropHP = GetMaxHP()*dwCount/100;
					sys_log(0, "CSpecialItemGroup::DRAIN_HP %d", -iDropHP);
					iDropHP = MIN(iDropHP, GetHP()-1);
					sys_log(0, "CSpecialItemGroup::DRAIN_HP %d", -iDropHP);
					PointChange(POINT_HP, -iDropHP);
					bSuccess = true;
				}
				break;
			case CSpecialItemGroup::POISON:
				{
					AttackedByPoison(NULL);
					bSuccess = true;
				}
				break;

			case CSpecialItemGroup::MOB_GROUP:
				{
					int sx = GetX() - number(300, 500);
					int sy = GetY() - number(300, 500);
					int ex = GetX() + number(300, 500);
					int ey = GetY() + number(300, 500);
					CHARACTER_MANAGER::instance().SpawnGroup(dwCount, GetMapIndex(), sx, sy, ex, ey, NULL, true);

					bSuccess = true;
				}
				break;
			default:
				{
					item_get = AutoGiveItem(dwVnum, dwCount, iRarePct);

					if (item_get)
					{
						bSuccess = true;
					}
				}
				break;
		}

		if (bSuccess)
		{
			dwItemVnums.push_back(dwVnum);
			dwItemCounts.push_back(dwCount);
			item_gets.push_back(item_get);
			count++;

		}
		else
		{
			return false;
		}
	}
	return bSuccess;
}

// NEW_HAIR_STYLE_ADD
bool CHARACTER::ItemProcess_Hair(LPITEM item, int iDestCell)
{
	if (item->CheckItemUseLevel(GetLevel()) == false)
	{
		// ���� ���ѿ� �ɸ�
		ChatPacket(CHAT_TYPE_INFO, "[LS;456]");
		return false;
	}

	DWORD hair = item->GetVnum();

	switch (GetJob())
	{
		case JOB_WARRIOR :
			hair -= 72000; // 73001 - 72000 = 1001 ���� ��� ��ȣ ����
			break;

		case JOB_ASSASSIN :
			hair -= 71250;
			break;

		case JOB_SURA :
			hair -= 70500;
			break;

		case JOB_SHAMAN :
			hair -= 69750;
			break;

		default :
			return false;
			break;
	}

	if (hair == GetPart(PART_HAIR))
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;457]");
		return true;
	}

	item->SetCount(item->GetCount() - 1);

	SetPart(PART_HAIR, hair);
	UpdatePacket();

	return true;
}
// END_NEW_HAIR_STYLE_ADD

bool CHARACTER::ItemProcess_Polymorph(LPITEM item)
{
	if (IsPolymorphed())
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;458]");
		return false;
	}

	if (true == IsRiding())
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;1053]");
		return false;
	}

	DWORD dwVnum = item->GetSocket(0);

	if (dwVnum == 0)
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;460]");
		item->SetCount(item->GetCount()-1);
		return false;
	}

	const CMob* pMob = CMobManager::instance().Get(dwVnum);

	if (pMob == NULL)
	{
		ChatPacket(CHAT_TYPE_INFO, "[LS;460]");
		item->SetCount(item->GetCount()-1);
		return false;
	}

	switch (item->GetVnum())
	{
		case 70104 :
		case 70105 :
		case 70106 :
		case 70107 :
		case 71093 :
			{
				// �а��� ó��
				sys_log(0, "USE_POLYMORPH_BALL PID(%d) vnum(%d)", GetPlayerID(), dwVnum);

				// ���� ���� üũ
				int iPolymorphLevelLimit = MAX(0, 20 - GetLevel() * 3 / 10);
				if (pMob->m_table.bLevel >= GetLevel() + iPolymorphLevelLimit)
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;461]");
					return false;
				}

				int iDuration = GetSkillLevel(POLYMORPH_SKILL_ID) == 0 ? 5 : (5 + (5 + GetSkillLevel(POLYMORPH_SKILL_ID)/40 * 25));
				iDuration *= 60;

				DWORD dwBonus = 0;

				dwBonus = (2 + GetSkillLevel(POLYMORPH_SKILL_ID)/40) * 100;

				AddAffect(AFFECT_POLYMORPH, POINT_POLYMORPH, dwVnum, AFF_POLYMORPH, iDuration, 0, true);
				AddAffect(AFFECT_POLYMORPH, POINT_ATT_BONUS, dwBonus, AFF_POLYMORPH, iDuration, 0, false);
				
				if (IsAffectFlag(AFF_GEOMGYEONG))
					RemoveAffect(SKILL_GEOMKYUNG);
				if (IsAffectFlag(AFF_GWIGUM))
					RemoveAffect(SKILL_GWIGEOM);

				item->SetCount(item->GetCount()-1);
			}
			break;

		case 50322:
			{
				// ����

				// �а��� ó��
				// ����0                ����1           ����2
				// �а��� ���� ��ȣ   ��������        �а��� ����
				sys_log(0, "USE_POLYMORPH_BOOK: %s(%u) vnum(%u)", GetName(), GetPlayerID(), dwVnum);

				if (CPolymorphUtils::instance().PolymorphCharacter(this, item, pMob) == true)
				{
					CPolymorphUtils::instance().UpdateBookPracticeGrade(this, item);
				}
				else
				{
				}
			}
			break;

		default :
			sys_err("POLYMORPH invalid item passed PID(%d) vnum(%d)", GetPlayerID(), item->GetOriginalVnum());
			return false;
	}

	return true;
}

bool CHARACTER::CanDoCube() const
{
	if (m_bIsObserver)	return false;
	if (GetShop())		return false;
	if (GetMyShop())	return false;
	if (m_bUnderRefine)	return false;
	if (IsWarping())	return false;
	if (IsAuraRefineWindowOpen())	return false;
#ifdef __PREMIUM_PRIVATE_SHOP__
	if (IsEditingPrivateShop() || IsShopSearch() || GetMyPrivateShop()) return false;
#endif
	return true;
}

bool CHARACTER::UnEquipSpecialRideUniqueItem()
{
	LPITEM Unique1 = GetWear(WEAR_UNIQUE1);
	LPITEM Unique2 = GetWear(WEAR_UNIQUE2);
#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
	LPITEM MountCostume = GetWear(WEAR_COSTUME_MOUNT);
#endif

	if( NULL != Unique1 )
	{
		if( UNIQUE_GROUP_SPECIAL_RIDE == Unique1->GetSpecialGroup() )
		{
			return UnequipItem(Unique1);
		}
	}

	if( NULL != Unique2 )
	{
		if( UNIQUE_GROUP_SPECIAL_RIDE == Unique2->GetSpecialGroup() )
		{
			return UnequipItem(Unique2);
		}
	}

#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
	if (MountCostume)
		return UnequipItem(MountCostume);
#endif

	return true;
}

void CHARACTER::AutoRecoveryItemProcess(const EAffectTypes type)
{
	if (true == IsDead() || true == IsStun())
		return;

	if (false == IsPC())
		return;

	if (AFFECT_AUTO_HP_RECOVERY != type && AFFECT_AUTO_SP_RECOVERY != type)
		return;

	if (NULL != FindAffect(AFFECT_STUN))
		return;

	{
		const DWORD stunSkills[] = { SKILL_TANHWAN, SKILL_GEOMPUNG, SKILL_BYEURAK, SKILL_GIGUNG };

		for (size_t i=0 ; i < sizeof(stunSkills)/sizeof(DWORD) ; ++i)
		{
			const CAffect* p = FindAffect(stunSkills[i]);

			if (NULL != p && AFF_STUN == p->dwFlag)
				return;
		}
	}

	const CAffect* pAffect = FindAffect(type);
	// const size_t idx_of_amount_of_used = 1;
	// const size_t idx_of_amount_of_full = 2;

	if (NULL != pAffect)
	{
		LPITEM pItem = FindItemByID(pAffect->dwFlag);

		if (NULL != pItem && true == pItem->GetSocket(0))
		{
			if (!CArenaManager::instance().IsArenaMap(GetMapIndex())
#ifdef ENABLE_NEWSTUFF
				&& !(g_NoPotionsOnPVP && CPVPManager::instance().IsFighting(GetPlayerID()) && !IsAllowedPotionOnPVP(pItem->GetVnum()))
#endif
			)
			{
				// const long amount_of_used = pItem->GetSocket(idx_of_amount_of_used);
				// const long amount_of_full = pItem->GetSocket(idx_of_amount_of_full);

				// const int32_t avail = amount_of_full - amount_of_used;

				int32_t amount = 0;

				if (AFFECT_AUTO_HP_RECOVERY == type)
				{
					amount = GetMaxHP() - (GetHP() + GetPoint(POINT_HP_RECOVERY));
				}
				else if (AFFECT_AUTO_SP_RECOVERY == type)
				{
					amount = GetMaxSP() - (GetSP() + GetPoint(POINT_SP_RECOVERY));
				}

				// if (amount > 0)
				// {
					// if (avail > amount)
					// {
						// const int pct_of_used = amount_of_used * 100 / amount_of_full;
						// const int pct_of_will_used = (amount_of_used + amount) * 100 / amount_of_full;

						// bool bLog = false;
						// ��뷮�� 10% ������ �α׸� ����
						// (��뷮�� %����, ���� �ڸ��� �ٲ� ������ �α׸� ����.)
						// if ((pct_of_will_used / 10) - (pct_of_used / 10) >= 1)
							// bLog = true;
						// pItem->SetSocket(idx_of_amount_of_used, amount_of_used + amount, bLog);
					// }
					// else
					// {
						// amount = avail;

						// ITEM_MANAGER::instance().RemoveItem( pItem );
					// }

					// if (AFFECT_AUTO_HP_RECOVERY == type)
					// {
						// PointChange( POINT_HP_RECOVERY, amount );
						// EffectPacket( SE_AUTO_HPUP );
					// }
					// else if (AFFECT_AUTO_SP_RECOVERY == type)
					// {
						// PointChange( POINT_SP_RECOVERY, amount );
						// EffectPacket( SE_AUTO_SPUP );
					// }
				// }
				if (amount > 0)
				{
					if (AFFECT_AUTO_HP_RECOVERY == type)
					{
						PointChange(POINT_HP_RECOVERY, amount);
						EffectPacket(SE_AUTO_HPUP);
					}
					else if (AFFECT_AUTO_SP_RECOVERY == type)
					{
						PointChange(POINT_SP_RECOVERY, amount);
						EffectPacket(SE_AUTO_SPUP);
					}
				}
			}
			else
			{
				pItem->Lock(false);
				pItem->SetSocket(0, false);
				RemoveAffect( const_cast<CAffect*>(pAffect) );
			}
		}
		else
		{
			RemoveAffect( const_cast<CAffect*>(pAffect) );
		}
	}
}

bool CHARACTER::IsValidItemPosition(TItemPos Pos) const
{
	BYTE window_type = Pos.window_type;
	WORD cell = Pos.cell;

	switch (window_type)
	{
	case RESERVED_WINDOW:
		return false;

	case INVENTORY:
	case EQUIPMENT:
		return cell < (INVENTORY_AND_EQUIP_SLOT_MAX);

	case DRAGON_SOUL_INVENTORY:
		return cell < (DRAGON_SOUL_INVENTORY_MAX_NUM);

	case SAFEBOX:
		if (NULL != m_pkSafebox)
			return m_pkSafebox->IsValidPosition(cell);
		else
			return false;

	case MALL:
		if (NULL != m_pkMall)
			return m_pkMall->IsValidPosition(cell);
		else
			return false;
#if defined(__ATTR_6TH_7TH__)
	case ATTR67_ADD:
		return cell < ATTR67_ADD_SLOT_MAX;
#endif
#ifdef ENABLE_SWITCHBOT
	case SWITCHBOT:
		return cell < SWITCHBOT_SLOT_COUNT;
#endif
	default:
		return false;
	}
}


// �����Ƽ� ���� ��ũ��.. exp�� true�� msg�� ����ϰ� return false �ϴ� ��ũ�� (�Ϲ����� verify �뵵���� return ������ �ణ �ݴ�� �̸������� �򰥸� ���� �ְڴ�..)
#define VERIFY_MSG(exp, msg)  \
	if (true == (exp)) { \
			ChatPacket(CHAT_TYPE_INFO, LC_TEXT(msg)); \
			return false; \
	}

/// ���� ĳ������ ���¸� �������� �־��� item�� ������ �� �ִ� �� Ȯ���ϰ�, �Ұ��� �ϴٸ� ĳ���Ϳ��� ������ �˷��ִ� �Լ�
bool CHARACTER::CanEquipNow(const LPITEM item, const TItemPos& srcCell, const TItemPos& destCell) /*const*/
{
	const TItemTable* itemTable = item->GetProto();
	//BYTE itemType = item->GetType();
	//BYTE itemSubType = item->GetSubType();

	switch (GetJob())
	{
		case JOB_WARRIOR:
			if (item->GetAntiFlag() & ITEM_ANTIFLAG_WARRIOR)
				return false;
			break;

		case JOB_ASSASSIN:
			if (item->GetAntiFlag() & ITEM_ANTIFLAG_ASSASSIN)
				return false;
			break;

		case JOB_SHAMAN:
			if (item->GetAntiFlag() & ITEM_ANTIFLAG_SHAMAN)
				return false;
			break;

		case JOB_SURA:
			if (item->GetAntiFlag() & ITEM_ANTIFLAG_SURA)
				return false;
			break;
	}

	for (int i = 0; i < ITEM_LIMIT_MAX_NUM; ++i)
	{
		long limit = itemTable->aLimits[i].lValue;
		switch (itemTable->aLimits[i].bType)
		{
			case LIMIT_LEVEL:
				if (GetLevel() < limit)
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;462]");
					return false;
				}
				break;

#if defined(__CONQUEROR_LEVEL__)
			case LIMIT_CHAMPION:
				if (GetConquerorLevel() < limit)
				{
					ChatPacket(CHAT_TYPE_INFO, "Nivelul tau campion este prea mic pentru a putea purta acest item!");
					return false;
				}
				break;
#endif

			case LIMIT_STR:
				if (GetPoint(POINT_ST) < limit)
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;463]");
					return false;
				}
				break;

			case LIMIT_INT:
				if (GetPoint(POINT_IQ) < limit)
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;464]");
					return false;
				}
				break;

			case LIMIT_DEX:
				if (GetPoint(POINT_DX) < limit)
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;465]");
					return false;
				}
				break;

			case LIMIT_CON:
				if (GetPoint(POINT_HT) < limit)
				{
					ChatPacket(CHAT_TYPE_INFO, "[LS;466]");
					return false;
				}
				break;
		}
	}
	

	if (item->GetWearFlag() & WEARABLE_UNIQUE)
	{
		if ((GetWear(WEAR_UNIQUE1) && GetWear(WEAR_UNIQUE1)->IsSameSpecialGroup(item)) ||
			(GetWear(WEAR_UNIQUE2) && GetWear(WEAR_UNIQUE2)->IsSameSpecialGroup(item)))
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;468]");
			return false;
		}

		if (marriage::CManager::instance().IsMarriageUniqueItem(item->GetVnum()) &&
			!marriage::CManager::instance().IsMarried(GetPlayerID()))
		{
			ChatPacket(CHAT_TYPE_INFO, "[LS;467]");
			return false;
		}

	}

#ifdef __FIX_ITEMS_TYPE_33__
	if (item->GetType() == ITEM_RING) // ring check for two same rings
	{
		LPITEM ringItems[2] = { GetWear(WEAR_RING1), GetWear(WEAR_RING2) };
		for (int i = 0; i < 2; i++)
		{
			if (ringItems[i]) // if that item is equipped
			{
				if (ringItems[i]->GetVnum() == item->GetVnum())
				{
					ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot equip this item twice"));
					return false;
				}
			}
		}
	}
#endif

#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	if (IsSecured())
	{
		ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT("|cffd93c3c[Err]:|h|r_Acest_cont_este_securizat!"));
		return false;
	}
#endif

	return true;
}

/// ���� ĳ������ ���¸� �������� ���� ���� item�� ���� �� �ִ� �� Ȯ���ϰ�, �Ұ��� �ϴٸ� ĳ���Ϳ��� ������ �˷��ִ� �Լ�
bool CHARACTER::CanUnequipNow(const LPITEM item, const TItemPos& srcCell, const TItemPos& destCell) /*const*/
{

	if (ITEM_BELT == item->GetType())
		VERIFY_MSG(CBeltInventoryHelper::IsExistItemInBeltInventory(this), "[1095]You can only discard the belt when there are no longer any items in its inventory.");

	// ������ ������ �� ���� ������
	if (IS_SET(item->GetFlag(), ITEM_FLAG_IRREMOVABLE))
		return false;


	if (IsAuraRefineWindowOpen())
		return false;
	
	int pos = GetEmptyInventoryEx(item);
	VERIFY_MSG( -1 == pos, "[1130]There isn't enough space in your inventory." );

#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	if (IsSecured())
	{
		ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT("|cffd93c3c[Err]:|h|r_Acest_cont_este_securizat!"));
		return false;
	}
#endif

	return true;
}

#ifdef ENABLE_REFINE_ELEMENT
BYTE CHARACTER::GetRefineElementType()
{
	LPITEM pkWeapon = GetWear(WEAR_WEAPON);
	if(!pkWeapon)
		return 0;
	
	if(pkWeapon->GetRefineElementPlus() < 3)
		return 0;

	return pkWeapon->GetRefineElementType();
}

void CHARACTER::ClearRefineElement()
{
	m_sRefineElementSrcCell = -1;
	m_sRefineElementDstCell = -1;
	m_cRefineElementType = -1;
	m_bUnderRefine = false;
}

bool CHARACTER::DoRefineElement(BYTE bArg)
{
	if (!CanHandleItem(true))
	{
		ClearRefineElement();
		return false;
	}
	
	short sRefineElementSrcCell = m_sRefineElementSrcCell;
	short sRefineElementDstCell = m_sRefineElementDstCell;
	char  cRefineElementType = m_cRefineElementType;
	ClearRefineElement();
	
	if(sRefineElementSrcCell < 0 || sRefineElementDstCell < 0 || cRefineElementType < 0)
		return false;

	LPITEM srcItem = GetInventoryItem(sRefineElementSrcCell);
	LPITEM dstItem = GetInventoryItem(sRefineElementDstCell);

	if(!srcItem || !dstItem)
		return false;
	
	if(dstItem->IsExchanging() || dstItem->IsEquipped())
		return false;
	
	if(srcItem->GetType() != ITEM_USE)
		return false;
	
	if(dstItem->GetType() != ITEM_WEAPON)
		return false;
	
	if(dstItem->GetRefineLevel() < ELEMENT_MIN_REFINE_LEVEL)
		return false;
	
	if(srcItem->GetSubType() == USE_ELEMENT_UPGRADE)
	{
		if(cRefineElementType != REFINE_ELEMENT_TYPE_UPGRADE)
			return false;
		
		if(srcItem->GetValue(0) <= REFINE_ELEMENT_CATEGORY_NONE || srcItem->GetValue(0) >= REFINE_ELEMENT_CATEGORY_MAX)
			return false;
		
		if(dstItem->GetRefineElementPlus() == REFINE_ELEMENT_MAX)
			return false;
		
		if(dstItem->GetRefineElementType() > 0 && dstItem->GetRefineElementType() != srcItem->GetValue(0))
			return false;
	}
	else if(srcItem->GetSubType() == USE_ELEMENT_DOWNGRADE)
	{
		if(cRefineElementType != REFINE_ELEMENT_TYPE_DOWNGRADE)
			return false;
	}
	else if(srcItem->GetSubType() == USE_ELEMENT_CHANGE)
	{
		if(cRefineElementType != REFINE_ELEMENT_TYPE_CHANGE)
			return false;
		
		if(bArg <= REFINE_ELEMENT_CATEGORY_NONE || bArg >= REFINE_ELEMENT_CATEGORY_MAX)
			return false;
	}
	else 
		return false;

	uint64_t dwRefineCost = 10000000; // unsigned long long
	int iSuccessProb = 0;
	switch(cRefineElementType)
	{
		case REFINE_ELEMENT_TYPE_UPGRADE:
			dwRefineCost = REFINE_ELEMENT_UPGRADE_YANG;
			iSuccessProb = REFINE_ELEMENT_UPGRADE_PROBABILITY;
			break;
			
		case REFINE_ELEMENT_TYPE_DOWNGRADE:
			dwRefineCost = REFINE_ELEMENT_DOWNGRADE_YANG;
			iSuccessProb = REFINE_ELEMENT_DOWNGRADE_PROBABILITY;
			break;
			
		case REFINE_ELEMENT_TYPE_CHANGE:
			dwRefineCost = REFINE_ELEMENT_CHANGE_YANG;
			iSuccessProb = REFINE_ELEMENT_DOWNGRADE_PROBABILITY;
			break;
			
		default:
			dwRefineCost = 10000000;
			break;
	}

	if (GetGold() < dwRefineCost)
		return false;
	
	// Save and delete after
	BYTE bElementType = srcItem->GetValue(0);
	srcItem->SetCount(srcItem->GetCount() - 1);
#ifdef ENABLE_REMOVE_LIMIT_GOLD
	ChangeGold(-dwRefineCost);
#else
	PointChange(POINT_GOLD, -dwRefineCost);
#endif
	
	int iRandomProb = number(1, 100);
	if(iRandomProb <= iSuccessProb)
	{
		// Succes
		// For upgrade / change
		BYTE bRefineElementType = dstItem->GetRefineElementType();
		BYTE bRefineElementPlus = dstItem->GetRefineElementPlus();
		BYTE bRefineElementBonusValue = dstItem->GetRefineElementBonusValue();
		BYTE bRefineElementAttackValue = dstItem->GetRefineElementAttackValue();
		
		// For downgrade
		BYTE bRefineElementLastIncBonus = dstItem->GetRefineElementLastIncBonus();
		BYTE bRefineElementLastIncAttack = dstItem->GetRefineElementLastIncAttack();
		
		if(cRefineElementType == REFINE_ELEMENT_TYPE_UPGRADE)
		{
			bRefineElementType = bElementType;
			bRefineElementPlus++;
			
			BYTE bIncreaseValue = number(REFINE_ELEMENT_RANDOM_VALUE_MIN, REFINE_ELEMENT_RANDOM_VALUE_MAX);
			bRefineElementBonusValue += bIncreaseValue;
			bRefineElementLastIncBonus = bIncreaseValue;
			
			BYTE bIncreaseAttackValue = number(REFINE_ELEMENT_RANDOM_BONUS_VALUE_MIN, REFINE_ELEMENT_RANDOM_BONUS_VALUE_MAX);
			bRefineElementAttackValue += bIncreaseAttackValue;
			bRefineElementLastIncAttack = bIncreaseAttackValue;
			
			char szRefineElement[10 + 1];
			snprintf(szRefineElement, sizeof(szRefineElement), "%d%d%02d%02d%d%02d", 
				bRefineElementType, bRefineElementPlus, 
				bRefineElementBonusValue, bRefineElementAttackValue, 
				bRefineElementLastIncBonus, bRefineElementLastIncAttack
			);
			
			DWORD dwRefineElement = atoi(szRefineElement);
			dstItem->SetRefineElement(dwRefineElement);
			
			SendRefineElementPacket(sRefineElementSrcCell, sRefineElementDstCell, REFINE_ELEMENT_TYPE_UPGRADE_SUCCES);
		}
		else if(cRefineElementType == REFINE_ELEMENT_TYPE_DOWNGRADE)
		{
			if(bRefineElementPlus == 1)
			{
				dstItem->SetRefineElement(0);
				SendRefineElementPacket(sRefineElementSrcCell, sRefineElementDstCell, REFINE_ELEMENT_TYPE_DOWNGRADE_SUCCES);
			}
			else
			{
				bRefineElementPlus--;
				bRefineElementBonusValue -= bRefineElementLastIncBonus;
				bRefineElementAttackValue -= bRefineElementLastIncAttack;
				bRefineElementLastIncBonus = MINMAX(0, bRefineElementLastIncBonus, bRefineElementBonusValue - 1);
				bRefineElementLastIncAttack = MINMAX(0, bRefineElementLastIncAttack, bRefineElementAttackValue - 1);
				
				char szRefineElement[10 + 1];
				snprintf(szRefineElement, sizeof(szRefineElement), "%d%d%02d%02d%d%02d", 
					bRefineElementType, bRefineElementPlus, 
					bRefineElementBonusValue, bRefineElementAttackValue, 
					bRefineElementLastIncBonus, bRefineElementLastIncAttack
				);
				
				DWORD dwRefineElement = atoi(szRefineElement);
				dstItem->SetRefineElement(dwRefineElement);
				
				SendRefineElementPacket(sRefineElementSrcCell, sRefineElementDstCell, REFINE_ELEMENT_TYPE_DOWNGRADE_SUCCES);
			}
		}
		else if(cRefineElementType == REFINE_ELEMENT_TYPE_CHANGE)
		{
			bRefineElementType = bArg;
				
			char szRefineElement[10 + 1];
			snprintf(szRefineElement, sizeof(szRefineElement), "%d%d%02d%02d%d%02d", 
				bRefineElementType, bRefineElementPlus, 
				bRefineElementBonusValue, bRefineElementAttackValue, 
				bRefineElementLastIncBonus, bRefineElementLastIncAttack
			);
			
			DWORD dwRefineElement = atoi(szRefineElement);
			dstItem->SetRefineElement(dwRefineElement);
			
			SendRefineElementPacket(sRefineElementSrcCell, sRefineElementDstCell, REFINE_ELEMENT_TYPE_CHANGE_SUCCES);
		}
	}
	else
	{
		SendRefineElementPacket(sRefineElementSrcCell, sRefineElementDstCell, REFINE_ELEMENT_TYPE_UPGRADE_FAIL);
	}

	return true;
}

void CHARACTER::SendRefineElementPacket(WORD wSrcCell, WORD wDstCell, BYTE bType)
{
	if(!GetDesc())
		return;
	
	if(!GetDesc()->IsPhase(PHASE_GAME) && !GetDesc()->IsPhase(PHASE_DEAD))
		return;
	
	TPacketGCRefineElement pack;
	pack.bHeader = HEADER_GC_REFINE_ELEMENT;
	pack.wSrcCell = wSrcCell;
	pack.wDstCell = wDstCell;
	pack.bType = bType;
	GetDesc()->Packet(&pack, sizeof(TPacketGCRefineElement));
}

bool CHARACTER::RefineElementInformation(WORD wSrcCell, WORD wDstCell, BYTE bType)
{
	if(wSrcCell >= INVENTORY_MAX_NUM || wDstCell >= INVENTORY_MAX_NUM)
		return false;
		
	if(bType > REFINE_ELEMENT_TYPE_CHANGE)
		return false;

	LPITEM srcItem = GetInventoryItem(wSrcCell);
	LPITEM dstItem = GetInventoryItem(wDstCell);

	if (!srcItem || !dstItem)
		return false;

	SendRefineElementPacket(wSrcCell, wDstCell, bType);
	
	m_cRefineElementType = bType;
	m_sRefineElementSrcCell = wSrcCell;
	m_sRefineElementDstCell = wDstCell;
	m_bUnderRefine = true;
	return true;
}
#endif
#if defined(__ATTR_6TH_7TH__)
LPITEM CHARACTER::GetAttr67AddItem(BYTE byCell) const
{
	return GetItem(TItemPos(ATTR67_ADD, byCell));
}

bool CHARACTER::Attr67Add(const TAttr67AddData kAttr67AddData)
{
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

	// Check if character exists.
	if (!IsPC())
		return false;

	// Check if any window that handles items are open.
	if (GetExchange() || GetShop() || GetMyShop() || IsOpenSafebox() || IsCubeOpen())
		return false;

#if defined(__CHANGE_LOOK_SYSTEM__)
	// Check if the change look window is open.
	if (IsChangeLookOpen())
		return false;
#endif

	// Check if an item is already in the ATTR67_ADD (window).
	if (GetAttr67AddItem())
		return false;

	// Get the regist item.
	LPITEM pkRegistItem = GetItem(TItemPos(INVENTORY, kAttr67AddData.wRegistItemPos));
	if (!pkRegistItem)
		return false;

#if defined(__SOUL_BIND_SYSTEM__)
	// Check if the regist item is sealed.
	if (pkRegistItem->IsSealed())
		return false;
#endif

	// Check if the regist item is locked.
	if (pkRegistItem->isLocked())
		return false;

	// Check if the regist item is equipped.
	if (pkRegistItem->IsEquipped())
		return false;

	// Check the type of the regist item.
	if (pkRegistItem->GetType() != ITEM_ARMOR && pkRegistItem->GetType() != ITEM_WEAPON)
		return false;

	////////////////////////////////////////////////////////////////////////////////////////////////////////////
	// Material Item (Based from the pkRegistItem LevelLimit)
	// @ item.h -> GetAttr67MaterialVnum (Repro.)
	// >

	// Get the material item based on the regist item.
	DWORD dwItemMaterialVnum = pkRegistItem->GetAttr67MaterialVnum();

	// Check if player has this item and the amount used.
	if (!CountSpecifyItem(dwItemMaterialVnum) >= kAttr67AddData.byMaterialCount)
		return false;

	// Remove all material items used from the player.
	RemoveSpecifyItem(dwItemMaterialVnum, kAttr67AddData.byMaterialCount);

	////////////////////////////////////////////////////////////////////////////////////////////////////////////
	// Support Item
	// >

	// Set the default support increase percent in case the support item doesn't exist or wasn't used.
	long lSupportIncreasePct = 0;
	LPITEM pSupportItem = NULL;
	{
		// Check if the support item exists.
		pSupportItem = GetItem(TItemPos(INVENTORY, kAttr67AddData.wSupportItemPos));
		if (pSupportItem)
		{
			// The TOTAL increase percent is set in value(1) of the item.
			lSupportIncreasePct = pSupportItem->GetValue(3);
		}
	}

	// Total success percent.
	float fMaterialPct = float(kAttr67AddData.byMaterialCount * ATTR67_SUCCESS_PER_MATERIAL);
	float fSupportPct = float(lSupportIncreasePct / ATTR67_SUPPORT_MAX_COUNT) * kAttr67AddData.bySupportItemCount;
	float fTotalSuccessPct = fMaterialPct + fSupportPct;

	// Check if support item was used.
	if (pSupportItem)
	{
		if (!CountSpecifyItem(pSupportItem->GetVnum()) >= kAttr67AddData.bySupportItemCount)
			return false;

		// Remove all support items used from player.
		RemoveSpecifyItem(pSupportItem->GetVnum(), kAttr67AddData.bySupportItemCount);
	}

	// Opting to use SetItem instead of MoveItem since moving it is limited within inventories.
	// Remove regist item from character and set item into ATTR67_ADD (window).
	pkRegistItem->RemoveFromCharacter();
	SetItem(TItemPos(ATTR67_ADD, 0), pkRegistItem);
	{
		// Get item from ATTR67_ADD (window).
		LPITEM pkAttr67Add = GetAttr67AddItem();
		if (!pkAttr67Add)
		{
			// TODO: Make a backup of the item in case something goes bad.
			sys_err("CHARACTER::Attr67Add: failed to get regist item from ATTR67_ADD (window).");
			return false;
		}

		// The success percentage is stored in the quest flag for later checking
		// in quest when collecting the item.
		// This way we can preview the result of the add in the quest before and after.
		SetQuestFlag("attr67add_item.success", (number(1, 100) <= fTotalSuccessPct ? 1 : 0));
		// Set the wait time until the character can collect the item.
		SetQuestFlag("attr67add_item.wait_time", get_global_time() + ATTR67_ADD_WAIT_TIME);
		// Set the control of adding the rare attribute to avoid adding it again.
		SetQuestFlag("attr67add_item.add", 0);

		// @ attr67add_collect (handling)
	}

	return true;
}
#endif


#ifdef ENABLE_GLOVE_SYSTEM
bool CHARACTER::UseItemGloveAddAttribute(CItem& item, CItem& targetItem) {
    if (!targetItem.IsGloves() || targetItem.IsExchanging() ||
        targetItem.IsEquipped() || targetItem.GetAttributeSetIndex() == -1)
        return false;

    auto maxAttrCount = 5;
    if (targetItem.GetAttributeCount() >= maxAttrCount) {
        ChatPacket(CHAT_TYPE_INFO, "You cannot add another attribute.");
        return false;
    }

	if (number(1, 100) <= aiItemAttributeAddPercent[targetItem.GetAttributeCount()])
	{
		targetItem.AddAttribute();
		ChatPacket(CHAT_TYPE_INFO, "Upgrade successfully added. ");

		auto addedIndex = targetItem.GetAttributeCount() - 1;
		const std::string hint = std::to_string(targetItem.GetID());
		LogManager::instance().ItemLog(
			GetPlayerID(), targetItem.GetAttributeType(addedIndex),
			targetItem.GetAttributeValue(addedIndex), item.GetID(),
			"ADD_ATTRIBUTE_GLOVE", hint.c_str(), GetDesc()->GetHostName(),
			item.GetOriginalVnum());
	}
	else {
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("�Ӽ� �߰��� �����Ͽ����ϴ�."));
	}
    if (targetItem.IsEquipped())
        BuffOnAttr_RemoveBuffsFromItem(&targetItem);

    item.DecrementCount();
    return true;
}

bool CHARACTER::UseItemGloveChangeAttribute(CItem& item, CItem& targetItem) {
    if (!targetItem.IsGloves() || targetItem.IsExchanging() ||
        targetItem.IsEquipped() || targetItem.GetAttributeSetIndex() == -1 ||
        targetItem.GetAttributeCount() == 0)
        return false;

    if (targetItem.IsEquipped())
        BuffOnAttr_RemoveBuffsFromItem(&targetItem);

    targetItem.ChangeAttribute();

    const std::string hint = std::to_string(targetItem.GetID());
    LogManager::Instance().ItemLog(this, &item, "CHANGE_ATTRIBUTE_GLOVE",
                                   hint.c_str());

    item.DecrementCount();
    return true;
}
#endif

bool CHARACTER::UseItemTalismanAddAttribute(CItem& item, CItem& targetItem) {
    if (!targetItem.IsTalisman() || targetItem.IsExchanging() ||
        targetItem.IsEquipped() || targetItem.GetAttributeSetIndex() == -1)
        return false;

    auto maxAttrCount = 5;
    if (targetItem.GetAttributeCount() >= maxAttrCount) {
        ChatPacket(CHAT_TYPE_INFO, "You cannot add another attribute.");
        return false;
    }

	if (number(1, 100) <= aiItemAttributeAddPercent[targetItem.GetAttributeCount()])
	{
		targetItem.AddAttribute();
		ChatPacket(CHAT_TYPE_INFO, "Upgrade successfully added. ");

		auto addedIndex = targetItem.GetAttributeCount() - 1;
		const std::string hint = std::to_string(targetItem.GetID());
		LogManager::instance().ItemLog(
			GetPlayerID(), targetItem.GetAttributeType(addedIndex),
			targetItem.GetAttributeValue(addedIndex), item.GetID(),
			"ADD_ATTRIBUTE_TALISMAN", hint.c_str(), GetDesc()->GetHostName(),
			item.GetOriginalVnum());
	}
	else {
		ChatPacket(CHAT_TYPE_INFO, LC_TEXT("�Ӽ� �߰��� �����Ͽ����ϴ�."));
	}

    if (targetItem.IsEquipped())
        BuffOnAttr_RemoveBuffsFromItem(&targetItem);

    item.DecrementCount();
    return true;
}

bool CHARACTER::UseItemTalismanChangeAttribute(CItem& item, CItem& targetItem) {
    if (!targetItem.IsTalisman() || targetItem.IsExchanging() ||
        targetItem.IsEquipped() || targetItem.GetAttributeSetIndex() == -1 ||
        targetItem.GetAttributeCount() == 0)
        return false;

    if (targetItem.IsEquipped())
        BuffOnAttr_RemoveBuffsFromItem(&targetItem);

    targetItem.ChangeAttribute();

    const std::string hint = std::to_string(targetItem.GetID());
    LogManager::Instance().ItemLog(this, &item, "CHANGE_ATTRIBUTE_TALISMAN",
                                   hint.c_str());

    item.DecrementCount();
    return true;
}

#ifdef RENEWAL_PICKUP_AFFECT
void CHARACTER::AutoGiveItemNew(LPITEM item, bool printMsg)
{
	if (!item)
		return;

	const DWORD itemVnum = item->GetVnum();
	const WORD realCount = item->GetCount();

	WORD wCount = item->GetCount();

	if (item->IsStackable() && item->GetType() != ITEM_BLEND)
	{
			for (WORD i = 0; i < INVENTORY_AND_EQUIP_SLOT_MAX; ++i)
			{
				LPITEM invItem = GetInventoryItem(i);
				if (!invItem)
					continue;

				if (invItem->GetVnum() == itemVnum)
				{
					BYTE j;
					for (j = 0; j < ITEM_SOCKET_MAX_NUM; ++j)
						if (invItem->GetSocket(j) != item->GetSocket(j))
							break;
					if (j != ITEM_SOCKET_MAX_NUM)
						continue;
					const WORD bCount2 = MIN(g_bItemCountLimit - invItem->GetCount(), wCount);
					if (bCount2 > 0)
					{
						wCount -= bCount2;
						invItem->SetCount(invItem->GetCount() + bCount2);
						if (wCount == 0)
						{
							if (printMsg)
							{
								if (realCount > 1)
									ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Ai primit: x%d%s."), realCount, item->GetName());
								else
									ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Ai primit: %s."), item->GetName());
							}
							M2_DESTROY_ITEM(item);
							return;
						}
					}
				}
			}
			item->SetCount(wCount);
		
	}


	int cell;
	if (item->IsDragonSoul())
	{
		cell = GetEmptyDragonSoulInventory(item);
	}
	else
	{
		cell = GetEmptyInventory(item->GetSize());
	}

	if (cell != -1)
	{
		if (item->IsDragonSoul())
		{
			item->AddToCharacter(this, TItemPos(DRAGON_SOUL_INVENTORY, cell)
			);
			if (printMsg)
			{
				if (realCount > 1)
					ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Ai primit: x%d%s."), realCount, item->GetName());
				else
					ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Ai primit: %s."), item->GetName());
			}
		}
		else
		{
			item->AddToCharacter(this, TItemPos(INVENTORY, cell)
			);
			if (printMsg)
			{
				if (realCount > 1)
					ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Ai primit: x%d%s."), realCount, item->GetName());
				else
					ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Ai primit: %s."), item->GetName());
			}
		}
	}
	else
	{
		if (printMsg && realCount != wCount)
		{
			if (realCount - wCount > 1)
				ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Ai primit: x%d%s."), realCount, item->GetName());
		}

		item->AddToGround(GetMapIndex(), GetXYZ());
#ifdef ENABLE_NEWSTUFF
		item->StartDestroyEvent(g_aiItemDestroyTime[ITEM_DESTROY_TIME_AUTOGIVE]);
#else
		item->StartDestroyEvent();
#endif
		item->SetOwnership(this, 300);
	}
}

bool CHARACTER::CanPickupDirectly()
{
	return FindAffect(AFFECT_PICKUP_ENABLE) != NULL;
}
#endif