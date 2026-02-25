#include "stdafx.h"
#include "utils.h"
#include "config.h"
#include "char.h"
#include "desc.h"
#include "sectree_manager.h"
#include "packet.h"
#include "protocol.h"
#include "log.h"
#include "skill.h"
#include "unique_item.h"
#include "profiler.h"
#include "marriage.h"
#include "item_addon.h"
#include "dev_log.h"
#include "locale_service.h"
#include "item.h"
#include "item_manager.h"
#include "affect.h"
#include "DragonSoul.h"
#include "buff_on_attributes.h"
#include "belt_inventory_helper.h"
#include "ItemUtils.h"
#include "../common/VnumHelper.h"
#include "../common/prodomodefines.h"
#ifdef ENABLE_SHOP_SEARCH
	#include "shop_search.h"
#endif
#include "char_manager.h"
#ifdef __PREMIUM_PRIVATE_SHOP__
#include "private_shop_manager.h"
#include "private_shop.h"
#endif


CItem::CItem(DWORD dwVnum)
	: m_dwVnum(dwVnum), m_bWindow(0), m_dwID(0), m_bEquipped(false), m_dwVID(0), m_wCell(0), m_dwCount(0), m_dwTransmutation(0), m_lFlag(0), m_dwLastOwnerPID(0),
	m_bExchanging(false), m_pkDestroyEvent(NULL), m_pkExpireEvent(NULL), m_pkUniqueExpireEvent(NULL),
	m_pkTimerBasedOnWearExpireEvent(NULL), m_pkRealTimeExpireEvent(NULL),
   	m_pkAccessorySocketExpireEvent(NULL), m_pkOwnershipEvent(NULL), m_dwOwnershipPID(0), m_bSkipSave(false), m_isLocked(false),
	m_dwMaskVnum(0), m_dwSIGVnum (0), m_pkAuraBoostSocketExpireEvent(NULL),
#ifdef ENABLE_REFINE_ELEMENT
	m_dwRefineElement(0)
#endif
#ifdef __PREMIUM_PRIVATE_SHOP__
	, m_pPrivateShop(nullptr), m_llGoldPrice(0), m_dwChequePrice(0), m_tPrivateShopCheckin(0)
#endif
#ifdef ENABLE_AFFECT_RENEWAL
	, m_pkBlendUseEvent(nullptr)
#endif
{
	memset( &m_alSockets, 0, sizeof(m_alSockets) );
	memset( &m_aAttr, 0, sizeof(m_aAttr) );
}

CItem::~CItem()
{
	Destroy();
}

void CItem::Initialize()
{
	CEntity::Initialize(ENTITY_ITEM);

	m_bWindow = RESERVED_WINDOW;
	m_pOwner = NULL;
	m_dwID = 0;
	m_bEquipped = false;
	m_dwVID = m_wCell = m_dwCount = m_lFlag = 0;
#ifdef __CHANGELOOK_SYSTEM__
	m_dwTransmutation = 0;
#endif

	m_pProto = NULL;
	m_bExchanging = false;
#ifdef ENABLE_REFINE_ELEMENT
	m_dwRefineElement = 0;
#endif
	memset(&m_alSockets, 0, sizeof(m_alSockets));
	memset(&m_aAttr, 0, sizeof(m_aAttr));
	m_pkDestroyEvent = NULL;
	m_pkOwnershipEvent = NULL;
	m_dwOwnershipPID = 0;
	m_pkUniqueExpireEvent = NULL;
	m_pkTimerBasedOnWearExpireEvent = NULL;
	m_pkRealTimeExpireEvent = NULL;
#ifdef ENABLE_AFFECT_RENEWAL
	m_pkBlendUseEvent = nullptr;
#endif
	m_pkAccessorySocketExpireEvent = NULL;

	m_bSkipSave = false;
	m_dwLastOwnerPID = 0;

#ifdef __PREMIUM_PRIVATE_SHOP__
	m_pPrivateShop = nullptr;
	m_llGoldPrice = 0;
	m_dwChequePrice = 0;
	m_tPrivateShopCheckin = 0;
#endif

#ifdef __AURA_SYSTEM__
	m_pkAuraBoostSocketExpireEvent = NULL;
#endif
}

void CItem::Destroy()
{
	event_cancel(&m_pkDestroyEvent);
	event_cancel(&m_pkOwnershipEvent);
	event_cancel(&m_pkUniqueExpireEvent);
	event_cancel(&m_pkTimerBasedOnWearExpireEvent);
	event_cancel(&m_pkRealTimeExpireEvent);
	event_cancel(&m_pkAccessorySocketExpireEvent);
#ifdef __AURA_SYSTEM__
	event_cancel(&m_pkAuraBoostSocketExpireEvent);
#endif
#ifdef ENABLE_AFFECT_RENEWAL
	event_cancel(&m_pkBlendUseEvent);
#endif
	CEntity::Destroy();

	if (GetSectree())
		GetSectree()->RemoveEntity(this);
}

EVENTFUNC(item_destroy_event)
{
	item_event_info* info = dynamic_cast<item_event_info*>( event->info );

	if ( info == NULL )
	{
		sys_err( "item_destroy_event> <Factor> Null pointer" );
		return 0;
	}

	LPITEM pkItem = info->item;

	if (pkItem->GetOwner())
		sys_err("item_destroy_event: Owner exist. (item %s owner %s)", pkItem->GetName(), pkItem->GetOwner()->GetName());

	pkItem->SetDestroyEvent(NULL);
	M2_DESTROY_ITEM(pkItem);
	return 0;
}

void CItem::SetDestroyEvent(LPEVENT pkEvent)
{
	m_pkDestroyEvent = pkEvent;
}

void CItem::StartDestroyEvent(int iSec)
{
	if (m_pkDestroyEvent)
		return;

	item_event_info* info = AllocEventInfo<item_event_info>();
	info->item = this;

	SetDestroyEvent(event_create(item_destroy_event, info, PASSES_PER_SEC(iSec)));
}

void CItem::EncodeInsertPacket(LPENTITY ent)
{
	LPDESC d;

	if (!(d = ent->GetDesc()))
		return;

	const PIXEL_POSITION & c_pos = GetXYZ();

	struct packet_item_ground_add pack;

	pack.bHeader	= HEADER_GC_ITEM_GROUND_ADD;
	pack.x		= c_pos.x;
	pack.y		= c_pos.y;
	pack.z		= c_pos.z;
	pack.dwVnum		= GetVnum();
	pack.dwVID		= m_dwVID;
	//pack.count	= m_dwCount;

	d->Packet(&pack, sizeof(pack));

	if (m_pkOwnershipEvent != NULL)
	{
		item_event_info * info = dynamic_cast<item_event_info *>(m_pkOwnershipEvent->info);

		if ( info == NULL )
		{
			sys_err( "CItem::EncodeInsertPacket> <Factor> Null pointer" );
			return;
		}

		TPacketGCItemOwnership p;

		p.bHeader = HEADER_GC_ITEM_OWNERSHIP;
		p.dwVID = m_dwVID;
		strlcpy(p.szName, info->szOwnerName, sizeof(p.szName));

		d->Packet(&p, sizeof(TPacketGCItemOwnership));
	}
}

void CItem::EncodeRemovePacket(LPENTITY ent)
{
	LPDESC d;

	if (!(d = ent->GetDesc()))
		return;

	struct packet_item_ground_del pack;

	pack.bHeader	= HEADER_GC_ITEM_GROUND_DEL;
	pack.dwVID		= m_dwVID;

	d->Packet(&pack, sizeof(pack));
	sys_log(2, "Item::EncodeRemovePacket %s to %s", GetName(), ((LPCHARACTER) ent)->GetName());
}

void CItem::SetProto(const TItemTable * table)
{
	assert(table != NULL);
	m_pProto = table;
	SetFlag(m_pProto->dwFlags);
}

void CItem::UsePacketEncode(LPCHARACTER ch, LPCHARACTER victim, struct packet_item_use *packet)
{
	if (!GetVnum())
		return;

	packet->header 	= HEADER_GC_ITEM_USE;
	packet->ch_vid 	= ch->GetVID();
	packet->victim_vid 	= victim->GetVID();
	packet->Cell = TItemPos(GetWindow(), m_wCell);
	packet->vnum	= GetVnum();
}

void CItem::RemoveFlag(long bit)
{
	REMOVE_BIT(m_lFlag, bit);
}

void CItem::AddFlag(long bit)
{
	SET_BIT(m_lFlag, bit);
}

void CItem::UpdatePacket()
{
	if (!m_pOwner || !m_pOwner->GetDesc())
		return;

#ifdef ENABLE_SWITCHBOT
	if (m_bWindow == SWITCHBOT)
		return;
#endif

	TPacketGCItemUpdate pack;

	pack.header = HEADER_GC_ITEM_UPDATE;
	pack.Cell = TItemPos(GetWindow(), m_wCell);
	pack.count	= m_dwCount;
#ifdef __CHANGELOOK_SYSTEM__
	pack.transmutation = m_dwTransmutation;
#endif
#ifdef ENABLE_REFINE_ELEMENT
	pack.dwRefineElement = m_dwRefineElement;
#endif
	for (int i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
		pack.alSockets[i] = m_alSockets[i];

	thecore_memcpy(pack.aAttr, GetAttributes(), sizeof(pack.aAttr));
	sys_log(2, "UpdatePacket %s -> %s", GetName(), m_pOwner->GetName());
	m_pOwner->GetDesc()->Packet(&pack, sizeof(pack));
}

DWORD CItem::GetCount()
{
	if (GetType() == ITEM_ELK) return MIN(m_dwCount, INT_MAX);
	else
	{
		return MIN(m_dwCount, g_bItemCountLimit);
	}
}

#ifdef ENABLE_REFINE_ELEMENT
void CItem::SetRefineElement(DWORD dwRefineElement)
{
	m_dwRefineElement = dwRefineElement;
	UpdatePacket();
	Save();
}
#endif

bool CItem::SetCount(DWORD count)
{
	if (GetType() == ITEM_ELK)
	{
		m_dwCount = MIN(count, INT_MAX);
	}
	else
	{
		m_dwCount = MIN(count, g_bItemCountLimit);
	}

	if (count == 0 && m_pOwner)
	{
		if (GetSubType() == USE_ABILITY_UP || GetSubType() == USE_POTION || GetVnum() == 70020)
		{
			LPCHARACTER pOwner = GetOwner();
			WORD wCell = GetCell();

			RemoveFromCharacter();

			if (!IsDragonSoul())
			{
				LPITEM pItem = pOwner->FindSpecifyItem(GetVnum());

				if (NULL != pItem)
				{
					pOwner->ChainQuickslotItem(pItem, QUICKSLOT_TYPE_ITEM, wCell);
				}
				else
				{
					pOwner->SyncQuickslot(QUICKSLOT_TYPE_ITEM, wCell, 255);
				}
			}

			M2_DESTROY_ITEM(this);
		}
		else
		{
			if (!IsDragonSoul())
			{
				m_pOwner->SyncQuickslot(QUICKSLOT_TYPE_ITEM, m_wCell, 255);
			}
			M2_DESTROY_ITEM(RemoveFromCharacter());
		}

		return false;
	}

	UpdatePacket();

	Save();
	return true;
}

#ifdef __CHANGELOOK_SYSTEM__
void CItem::SetTransmutation(DWORD dwVnum, bool bLog)
{
	m_dwTransmutation = dwVnum;
	UpdatePacket();
	Save();
}
#endif

LPITEM CItem::RemoveFromCharacter()
{
	if (!m_pOwner)
	{
		sys_err("Item::RemoveFromCharacter owner null");
		return (this);
	}

	LPCHARACTER pOwner = m_pOwner;

	if (m_bEquipped)	// �����Ǿ��°�?
	{
		Unequip();
		//pOwner->UpdatePacket();

		SetWindow(RESERVED_WINDOW);
		Save();
		return (this);
	}
	else
	{
		if (GetWindow() != SAFEBOX && GetWindow() != MALL
#if defined(__ATTR_6TH_7TH__)
			&& GetWindow() != ATTR67_ADD
#endif
			)
		{
			if (IsDragonSoul())
			{
				if (m_wCell >= DRAGON_SOUL_INVENTORY_MAX_NUM)
					sys_err("CItem::RemoveFromCharacter: pos >= DRAGON_SOUL_INVENTORY_MAX_NUM");
				else
					pOwner->SetItem(TItemPos(m_bWindow, m_wCell), NULL);
			}
#ifdef ENABLE_SWITCHBOT
			else if (m_bWindow == SWITCHBOT)
			{
				if (m_wCell >= SWITCHBOT_SLOT_COUNT)
				{
					sys_err("CItem::RemoveFromCharacter: pos >= SWITCHBOT_SLOT_COUNT");
				}
				else
				{
					pOwner->SetItem(TItemPos(SWITCHBOT, m_wCell), NULL);
				}
			}
#endif
			else
			{
				TItemPos cell(INVENTORY, m_wCell);

#ifdef ENABLE_CUSTOM_INVENTORY
				if (false == cell.IsDefaultInventoryPosition() && false == cell.IsBeltInventoryPosition() && false == cell.IsCustomInventoryPosition())
#else
				if (false == cell.IsDefaultInventoryPosition() && false == cell.IsBeltInventoryPosition())
#endif
				{
					sys_err("CItem::RemoveFromCharacter: Invalid Item Position");
				}
				else
				{
					pOwner->SetItem(cell, NULL);
				}
			}
		}
#if defined(__ATTR_6TH_7TH__)
		if (GetWindow() == ATTR67_ADD)
			pOwner->SetItem(TItemPos(ATTR67_ADD, 0), NULL);
#endif
		m_pOwner = NULL;
		m_wCell = 0;

		SetWindow(RESERVED_WINDOW);
		Save();
		return (this);
	}
}
#if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
bool CItem::AddToCharacter(LPCHARACTER ch, TItemPos Cell, bool bHighlight)
#else
bool CItem::AddToCharacter(LPCHARACTER ch, TItemPos Cell)
#endif
{
	assert(GetSectree() == NULL);
	assert(m_pOwner == NULL);
	WORD pos = Cell.cell;
	BYTE window_type = Cell.window_type;

	if (INVENTORY == window_type)
	{
		if (m_wCell >= INVENTORY_MAX_NUM && BELT_INVENTORY_SLOT_START > m_wCell)
		{
			sys_err("CItem::AddToCharacter: cell overflow: %s to %s cell %d", m_pProto->szName, ch->GetName(), m_wCell);
			return false;
		}
	}
	else if (DRAGON_SOUL_INVENTORY == window_type)
	{
		if (m_wCell >= DRAGON_SOUL_INVENTORY_MAX_NUM)
		{
			sys_err("CItem::AddToCharacter: cell overflow: %s to %s cell %d", m_pProto->szName, ch->GetName(), m_wCell);
			return false;
		}
	}
#ifdef ENABLE_SWITCHBOT
	else if (SWITCHBOT == window_type)
	{
		if (m_wCell >= SWITCHBOT_SLOT_COUNT)
		{
			sys_err("CItem::AddToCharacter:switchbot cell overflow: %s to %s cell %d", m_pProto->szName, ch->GetName(), m_wCell);
			return false;
		}
	}
#endif


	if (bHighlight)
		bHighlight = this->GetLastOwnerPID() != ch->GetPlayerID();
	
	
#ifdef ENABLE_REWARD_SYSTEM
	if (GetLastOwnerPID() != ch->GetPlayerID())
		CHARACTER_MANAGER::Instance().DoReward(ch, REWARD_MISSION_FIRST_ITEM, GetVnum(), 1);
#endif
	
	if (ch->GetDesc())
		m_dwLastOwnerPID = ch->GetPlayerID();
	
#ifdef __SASH_SYSTEM__
	if ((GetType() == ITEM_COSTUME) && (GetSubType() == COSTUME_SASH) && (GetSocket(SASH_ABSORPTION_SOCKET) == 0))
	{
		long lVal = GetValue(SASH_GRADE_VALUE_FIELD);
		switch (lVal)
		{
			case 2:
				{
					lVal = SASH_GRADE_2_ABS;
				}
				break;
			case 3:
				{
					lVal = SASH_GRADE_3_ABS;
				}
				break;
			case 4:
				{
					lVal = number(SASH_GRADE_4_ABS_MIN, SASH_GRADE_4_ABS_MAX_COMB);
				}
				break;
			default:
				{
					lVal = SASH_GRADE_1_ABS;
				}
				break;
		}
		
		SetSocket(SASH_ABSORPTION_SOCKET, lVal);
	}
#endif


	event_cancel(&m_pkDestroyEvent);

#if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
	ch->SetItem(TItemPos(window_type, pos), this, bHighlight);
#else
	ch->SetItem(TItemPos(window_type, pos), this);
#endif
	m_pOwner = ch;

	Save();
	return true;
}

LPITEM CItem::RemoveFromGround()
{
	if (GetSectree())
	{
		SetOwnership(NULL);

		GetSectree()->RemoveEntity(this);

		ViewCleanup();

		Save();
	}

	return (this);
}

bool CItem::AddToGround(long lMapIndex, const PIXEL_POSITION & pos, bool skipOwnerCheck)
{
	if (0 == lMapIndex)
	{
		sys_err("wrong map index argument: %d", lMapIndex);
		return false;
	}

	if (GetSectree())
	{
		sys_err("sectree already assigned");
		return false;
	}

	if (!skipOwnerCheck && m_pOwner)
	{
		sys_err("owner pointer not null");
		return false;
	}

	LPSECTREE tree = SECTREE_MANAGER::instance().Get(lMapIndex, pos.x, pos.y);

	if (!tree)
	{
		return false;
	}

	//tree->Touch();

	SetWindow(GROUND);
	SetXYZ(pos.x, pos.y, pos.z);
	tree->InsertEntity(this);
	UpdateSectree();
	Save();
	return true;
}

bool CItem::DistanceValid(LPCHARACTER ch)
{
	if (!GetSectree())
		return false;

	int iDist = DISTANCE_APPROX(GetX() - ch->GetX(), GetY() - ch->GetY());

	if (iDist > 300)
		return false;

	return true;
}

bool CItem::CanUsedBy(LPCHARACTER ch)
{
	// Anti flag check
	switch (ch->GetJob())
	{
		case JOB_WARRIOR:
			if (GetAntiFlag() & ITEM_ANTIFLAG_WARRIOR)
				return false;
			break;

		case JOB_ASSASSIN:
			if (GetAntiFlag() & ITEM_ANTIFLAG_ASSASSIN)
				return false;
			break;

		case JOB_SHAMAN:
			if (GetAntiFlag() & ITEM_ANTIFLAG_SHAMAN)
				return false;
			break;

		case JOB_SURA:
			if (GetAntiFlag() & ITEM_ANTIFLAG_SURA)
				return false;
			break;
	}

	return true;
}
// cdf26213a150dc3ecb610f18f6b38b46
int CItem::FindEquipCell(LPCHARACTER ch, int iCandidateCell)
{
	if ((0 == GetWearFlag() || ITEM_TOTEM == GetType()) && ITEM_COSTUME != GetType() && ITEM_DS != GetType() && ITEM_SPECIAL_DS != GetType() && ITEM_RING != GetType() && ITEM_BELT != GetType())
		return -1;
	// cdf26213a150dc3ecb610f18f6b38b46
	// ��ȥ�� ������ WEAR�� ó���� ���� ���(WEAR�� �ִ� 32������ �����ѵ� ��ȥ���� �߰��ϸ� 32�� �Ѵ´�.)
	// �κ��丮�� Ư�� ��ġ((INVENTORY_MAX_NUM + WEAR_MAX_NUM)���� (INVENTORY_MAX_NUM + WEAR_MAX_NUM + DRAGON_SOUL_DECK_MAX_NUM * DS_SLOT_MAX - 1)����)��
	// ��ȥ�� �������� ����.
	// return �� ����, INVENTORY_MAX_NUM�� �� ������,
	// ���� WearCell�� INVENTORY_MAX_NUM�� ���� return �ϱ� ����.
	// cdf26213a150dc3ecb610f18f6b38b46
	if (GetType() == ITEM_DS || GetType() == ITEM_SPECIAL_DS)
	{
		if (iCandidateCell < 0)
		{
			return WEAR_MAX_NUM + GetSubType();
		}
		else
		{
			for (int i = 0; i < DRAGON_SOUL_DECK_MAX_NUM; i++)
			{
				if (WEAR_MAX_NUM + i * DS_SLOT_MAX + GetSubType() == iCandidateCell)
				{
					return iCandidateCell;
				}
			}
			return -1;
		}
	}
	else if (GetType() == ITEM_TALISMAN)
	{
		if (GetWearFlag() & WEARABLE_FIRE)
			return WEAR_TALISMAN_FIRE;
		else if (GetWearFlag() & WEARABLE_ICE)
			return WEAR_TALISMAN_ICE;
		else if (GetWearFlag() & WEARABLE_EARTH)
			return WEAR_TALISMAN_EARTH;
		else if (GetWearFlag() & WEARABLE_DARK)
			return WEAR_TALISMAN_DARK;
		else if (GetWearFlag() & WEARABLE_WIND)
			return WEAR_TALISMAN_WIND;
		else if (GetWearFlag() & WEARABLE_ELEC)
			return WEAR_TALISMAN_ELEC;
	}
	
	else if (GetType() == ITEM_COSTUME)
	{
		if (GetSubType() == COSTUME_BODY)
			return WEAR_COSTUME_BODY;
		else if (GetSubType() == COSTUME_HAIR)
			return WEAR_COSTUME_HAIR;
#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
		else if (GetSubType() == COSTUME_MOUNT)
			return WEAR_COSTUME_MOUNT;
#endif
#ifdef __SASH_SYSTEM__
		else if (GetSubType() == COSTUME_SASH)
			return WEAR_COSTUME_SASH;
#endif
#ifdef ENABLE_WEAPON_COSTUME_SYSTEM
		else if (GetSubType() == COSTUME_WEAPON)
			return WEAR_COSTUME_WEAPON;
#endif
#ifdef __AURA_SYSTEM__
		else if (GetSubType() == COSTUME_AURA)
			return WEAR_COSTUME_AURA;
#endif
#ifdef ENABLE_PET_COSTUME_SYSTEM
		else if (GetSubType() == COSTUME_PET)
			return WEAR_COSTUME_PET;
#endif
	}
	else if (GetType() == ITEM_RING)
	{
		if (ch->GetWear(WEAR_RING1))
			return WEAR_RING2;
		
		return WEAR_RING1;
	}
	else if (GetType() == ITEM_BELT)
		return WEAR_BELT;
	else if (GetWearFlag() & WEARABLE_BODY)
		return WEAR_BODY;
	else if (GetWearFlag() & WEARABLE_HEAD)
		return WEAR_HEAD;
	else if (GetWearFlag() & WEARABLE_FOOTS)
		return WEAR_FOOTS;
	else if (GetWearFlag() & WEARABLE_WRIST)
		return WEAR_WRIST;
	else if (GetWearFlag() & WEARABLE_WEAPON)
		return WEAR_WEAPON;
	else if (GetWearFlag() & WEARABLE_SHIELD)
		return WEAR_SHIELD;
	else if (GetWearFlag() & WEARABLE_NECK)
		return WEAR_NECK;
	else if (GetWearFlag() & WEARABLE_EAR)
		return WEAR_EAR;
	else if (GetWearFlag() & WEARABLE_ARROW)
		return WEAR_ARROW;
#ifdef ENABLE_GLOVE_SYSTEM
	else if (GetWearFlag() & WEARABLE_GLOVE)
		return WEAR_GLOVE;
#endif
	else if (GetWearFlag() & WEARABLE_UNIQUE)
	{
		if (ch->GetWear(WEAR_UNIQUE1))
			return WEAR_UNIQUE2;
		else
			return WEAR_UNIQUE1;
	}

	// ���� ����Ʈ�� ���� �������� �����°����� �ѹ� ������ ���� �E�� ����.
	else if (GetWearFlag() & WEARABLE_ABILITY)
	{
		if (!ch->GetWear(WEAR_ABILITY1))
		{
			return WEAR_ABILITY1;
		}
		else if (!ch->GetWear(WEAR_ABILITY2))
		{
			return WEAR_ABILITY2;
		}
		else if (!ch->GetWear(WEAR_ABILITY3))
		{
			return WEAR_ABILITY3;
		}
		else if (!ch->GetWear(WEAR_ABILITY4))
		{
			return WEAR_ABILITY4;
		}
		else if (!ch->GetWear(WEAR_ABILITY5))
		{
			return WEAR_ABILITY5;
		}
		else if (!ch->GetWear(WEAR_ABILITY6))
		{
			return WEAR_ABILITY6;
		}
		else if (!ch->GetWear(WEAR_ABILITY7))
		{
			return WEAR_ABILITY7;
		}
		else if (!ch->GetWear(WEAR_ABILITY8))
		{
			return WEAR_ABILITY8;
		}
		else
		{
			return -1;
		}
	}
	return -1;
}
void CItem::ModifyPoints(bool bAdd)
{
	int accessoryGrade;

	// ����� ���ʸ� ������ �����Ų��.
	if (false == IsAccessoryForSocket())
	{
		{
			// ������ �Ӽ���ȭ�� ���Ǵ� ��� �������� �ʴ´� (ARMOR_WRIST ARMOR_NECK ARMOR_EAR)
			for (int i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
			{
				DWORD dwVnum;

				if ((dwVnum = GetSocket(i)) <= 2)
					continue;

				TItemTable * p = ITEM_MANAGER::instance().GetTable(dwVnum);

				if (!p)
				{
					//sys_err("cannot find table by vnum %u", dwVnum);
					continue;
				}

				if (ITEM_METIN == p->bType)
				{
					//m_pOwner->ApplyPoint(p->alValues[0], bAdd ? p->alValues[1] : -p->alValues[1]);
					for (int i = 0; i < ITEM_APPLY_MAX_NUM; ++i)
					{
						if (p->aApplies[i].bType == APPLY_NONE)
							continue;

						if (p->aApplies[i].bType == APPLY_SKILL)
							m_pOwner->ApplyPoint(p->aApplies[i].bType, bAdd ? p->aApplies[i].lValue : p->aApplies[i].lValue ^ 0x00800000);
						else
							m_pOwner->ApplyPoint(p->aApplies[i].bType, bAdd ? p->aApplies[i].lValue : -p->aApplies[i].lValue);
					}
				}
			}
		}

		accessoryGrade = 0;
	}
	else
	{
		accessoryGrade = MIN(GetAccessorySocketGrade(), ITEM_ACCESSORY_SOCKET_MAX_NUM);
	}
	
#ifdef __SASH_SYSTEM__
	if ((GetType() == ITEM_COSTUME) && (GetSubType() == COSTUME_SASH) && (GetSocket(SASH_ABSORBED_SOCKET)))
	{
		TItemTable * pkItemAbsorbed = ITEM_MANAGER::instance().GetTable(GetSocket(SASH_ABSORBED_SOCKET));
		if (pkItemAbsorbed)
		{
			if ((pkItemAbsorbed->bType == ITEM_ARMOR) && (pkItemAbsorbed->bSubType == ARMOR_BODY))
			{
				long lDefGrade = pkItemAbsorbed->alValues[1] + long(pkItemAbsorbed->alValues[5] * 2);
				double dValue = lDefGrade * GetSocket(SASH_ABSORPTION_SOCKET);
				dValue = (double)dValue / 100;
				dValue = (double)dValue + .5;
				lDefGrade = (long) dValue;
				if ((pkItemAbsorbed->alValues[1] > 0) && (lDefGrade <= 0) || (pkItemAbsorbed->alValues[5] > 0) && (lDefGrade < 1))
					lDefGrade += 1;
				else if ((pkItemAbsorbed->alValues[1] > 0) || (pkItemAbsorbed->alValues[5] > 0))
					lDefGrade += 1;
				
				m_pOwner->ApplyPoint(APPLY_DEF_GRADE_BONUS, bAdd ? lDefGrade : -lDefGrade);
				
				long lDefMagicBonus = pkItemAbsorbed->alValues[0];
				dValue = lDefMagicBonus * GetSocket(SASH_ABSORPTION_SOCKET);
				dValue = (double)dValue / 100;
				dValue = (double)dValue + .5;
				lDefMagicBonus = (long) dValue;
				if ((pkItemAbsorbed->alValues[0] > 0) && (lDefMagicBonus < 1))
					lDefMagicBonus += 1;
				else if (pkItemAbsorbed->alValues[0] > 0)
					lDefMagicBonus += 1;
				
				m_pOwner->ApplyPoint(APPLY_MAGIC_DEF_GRADE, bAdd ? lDefMagicBonus : -lDefMagicBonus);
			}
			else if (pkItemAbsorbed->bType == ITEM_WEAPON)
			{
				long lAttGrade = pkItemAbsorbed->alValues[4] + pkItemAbsorbed->alValues[5];
				if (pkItemAbsorbed->alValues[3] > pkItemAbsorbed->alValues[4])
					lAttGrade = pkItemAbsorbed->alValues[3] + pkItemAbsorbed->alValues[5];
				
#ifdef ENABLE_REFINE_ELEMENT
				if(GetRefineElementType())
				{
					long lAttPlus = GetRefineElementAttackValue();
					if(lAttPlus > 0)
						lAttGrade += lAttPlus;
				}
#endif
				
				double dValue = lAttGrade * GetSocket(SASH_ABSORPTION_SOCKET);
				dValue = (double)dValue / 100;
				dValue = (double)dValue + .5;
				lAttGrade = (long) dValue;
				if (((pkItemAbsorbed->alValues[3] > 0) && (lAttGrade < 1)) || ((pkItemAbsorbed->alValues[4] > 0) && (lAttGrade < 1)))
					lAttGrade += 1;
				else if ((pkItemAbsorbed->alValues[3] > 0) || (pkItemAbsorbed->alValues[4] > 0))
					lAttGrade += 1;
				
				m_pOwner->ApplyPoint(APPLY_ATT_GRADE_BONUS, bAdd ? lAttGrade : -lAttGrade);
				
				long lAttMagicGrade = pkItemAbsorbed->alValues[2] + pkItemAbsorbed->alValues[5];
				if (pkItemAbsorbed->alValues[1] > pkItemAbsorbed->alValues[2])
					lAttMagicGrade = pkItemAbsorbed->alValues[1] + pkItemAbsorbed->alValues[5];
				
				dValue = lAttMagicGrade * GetSocket(SASH_ABSORPTION_SOCKET);
				dValue = (double)dValue / 100;
				dValue = (double)dValue + .5;
				lAttMagicGrade = (long) dValue;
				if (((pkItemAbsorbed->alValues[1] > 0) && (lAttMagicGrade < 1)) || ((pkItemAbsorbed->alValues[2] > 0) && (lAttMagicGrade < 1)))
					lAttMagicGrade += 1;
				else if ((pkItemAbsorbed->alValues[1] > 0) || (pkItemAbsorbed->alValues[2] > 0))
					lAttMagicGrade += 1;
				
				m_pOwner->ApplyPoint(APPLY_MAGIC_ATT_GRADE, bAdd ? lAttMagicGrade : -lAttMagicGrade);
				
#ifdef ENABLE_REFINE_ELEMENT
				if(GetRefineElementType())
				{
					long lBonusValue = GetRefineElementBonusValue();
					double dValue = lBonusValue * GetSocket(SASH_ABSORPTION_SOCKET);
					dValue = (double)dValue / 100;
					dValue = (double)dValue + .5;
					lBonusValue = (long) dValue;
					if(lBonusValue > 0)
						// daten mortii tai
						m_pOwner->ApplyPoint(APPLY_ENCHANT_ELECT + (GetRefineElementType() - 1), bAdd ? lBonusValue : -lBonusValue);
				}
#endif
			}
		}
	}
#endif


	for (int i = 0; i < ITEM_APPLY_MAX_NUM; ++i)
	{
#ifdef __SASH_SYSTEM__
		if ((m_pProto->aApplies[i].bType == APPLY_NONE) && (GetType() != ITEM_COSTUME) && (GetSubType() != COSTUME_SASH))
#else
		if (m_pProto->aApplies[i].bType == APPLY_NONE)
#endif
			continue;

#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
		if(IsMountItem())
			continue;
#endif

		BYTE bType = m_pProto->aApplies[i].bType;
		long value = m_pProto->aApplies[i].lValue;

		std::vector<int> rarityValues2 = {0, 20, 50};
		int rarity = int(GetSocket(4));
		if (rarity && GetType() == ITEM_ARMOR)
		{
			long calc = value * int(rarityValues2[rarity]) / 100;
			value += calc;
		}

#ifdef __SASH_SYSTEM__
		if ((GetType() == ITEM_COSTUME) && (GetSubType() == COSTUME_SASH))
		{
			TItemTable * pkItemAbsorbed = ITEM_MANAGER::instance().GetTable(GetSocket(SASH_ABSORBED_SOCKET));
			if (pkItemAbsorbed)
			{
				if (pkItemAbsorbed->aApplies[i].bType == APPLY_NONE)
					continue;
				
				bType = pkItemAbsorbed->aApplies[i].bType;
				value = pkItemAbsorbed->aApplies[i].lValue;
				if (value < 0)
					continue;
				
				double dValue = value * GetSocket(SASH_ABSORPTION_SOCKET);
				dValue = (double)dValue / 100;
				dValue = (double)dValue + .5;
				value = (long) dValue;
				if ((pkItemAbsorbed->aApplies[i].lValue > 0) && (value <= 0))
					value += 1;
			}
			else
				continue;
		}
#endif
		if (bType != APPLY_SKILL)
		{
			if (0 != accessoryGrade && i < ITEM_APPLY_MAX_NUM - 1)
				value += MAX(accessoryGrade, value * aiAccessorySocketEffectivePct[accessoryGrade] / 100);
			
			m_pOwner->ApplyPoint(bType, bAdd ? value : -value);
		}
		else
			m_pOwner->ApplyPoint(bType, bAdd ? value : value ^ 0x00800000);
	}
	
	
	#ifdef __AURA_SYSTEM__
	float fAuraDrainPer = 0.0f;
	if (m_pProto->bType == ITEM_COSTUME && m_pProto->bSubType == COSTUME_AURA)
	{
		const long c_lLevelSocket = GetSocket(ITEM_SOCKET_AURA_CURRENT_LEVEL);
		const long c_lDrainSocket = GetSocket(ITEM_SOCKET_AURA_DRAIN_ITEM_VNUM);
		const long c_lBoostSocket = GetSocket(ITEM_SOCKET_AURA_BOOST);

		BYTE bCurLevel = (c_lLevelSocket / 100000) - 1000;
		BYTE bBoostIndex = c_lBoostSocket / 100000000;

		TItemTable* pBoosterProto = ITEM_MANAGER::instance().GetTable(ITEM_AURA_BOOST_ITEM_VNUM_BASE + bBoostIndex);
		fAuraDrainPer = (1.0f * bCurLevel / 10.0f) / 100.0f;
		if (pBoosterProto)
			fAuraDrainPer += 1.0f * pBoosterProto->alValues[ITEM_AURA_BOOST_PERCENT_VALUE] / 100.0f;

		TItemTable* p = NULL;
		if (c_lDrainSocket != 0)
			p = ITEM_MANAGER::instance().GetTable(c_lDrainSocket);

		if (p != NULL && (ITEM_ARMOR == p->bType && (ARMOR_SHIELD == p->bSubType || ARMOR_WRIST == p->bSubType || ARMOR_NECK == p->bSubType || ARMOR_EAR == p->bSubType)))
		{
			for (int i = 0; i < ITEM_APPLY_MAX_NUM; ++i)
			{
				if (p->aApplies[i].bType == APPLY_NONE || /*p->aApplies[i].bType == APPLY_MOUNT || */p->aApplies[i].bType == APPLY_MOV_SPEED || p->aApplies[i].lValue <= 0)
					continue;

				float fValue = p->aApplies[i].lValue * fAuraDrainPer;
				int iValue = static_cast<int>((fValue < 1.0f) ? ceilf(fValue) : truncf(fValue));
				if (p->aApplies[i].bType == APPLY_SKILL)
					m_pOwner->ApplyPoint(p->aApplies[i].bType, bAdd ? iValue : iValue ^ 0x00800000);
				else
					m_pOwner->ApplyPoint(p->aApplies[i].bType, bAdd ? iValue : -iValue);
			}
		}
	}
#endif
	
	// 초승달의 반지, 할로윈 사탕, 행복의 반지, 영원한 사랑의 펜던트의 경우
	// 기존의 하드 코딩으로 강제로 속성을 부여했지만,
	// 그 부분을 제거하고 special item group 테이블에서 속성을 부여하도록 변경하였다.
	// 하지만 하드 코딩되어있을 때 생성된 아이템이 남아있을 수도 있어서 특수처리 해놓는다.
	// 이 아이템들의 경우, 밑에 ITEM_UNIQUE일 때의 처리로 속성이 부여되기 때문에,
	// 아이템에 박혀있는 attribute는 적용하지 않고 넘어간다.
	if (true == CItemVnumHelper::IsRamadanMoonRing(GetVnum()) || true == CItemVnumHelper::IsHalloweenCandy(GetVnum())
		|| true == CItemVnumHelper::IsHappinessRing(GetVnum()) || true == CItemVnumHelper::IsLovePendant(GetVnum()))
	{
		// Do not anything.
	}
	else
	{
		for (int i = 0; i < ITEM_ATTRIBUTE_MAX_NUM; ++i)
		{
			if (GetAttributeType(i))
			{
				const TPlayerItemAttribute& ia = GetAttribute(i);
				long lValue = ia.sValue;
				/*if (ia.bType == APPLY_SKILL)
					m_pOwner->ApplyPoint(ia.bType, bAdd ? value : value ^ 0x00800000);
				else
					m_pOwner->ApplyPoint(ia.bType, bAdd ? value : -value);*/
				
#ifdef __AURA_SYSTEM__
				if (ia.bType == APPLY_SKILL)
				{
					if (m_pProto->bType == ITEM_COSTUME && m_pProto->bSubType == COSTUME_AURA)
					{
						if (ia.sValue <= 0)
							continue;

						float fValue = ia.sValue * fAuraDrainPer;
						lValue = static_cast<int>((fValue < 1.0f) ? ceilf(fValue) : truncf(fValue));
						//m_pOwner->ApplyPoint(ia.bType, bAdd ? iValue : iValue ^ 0x00800000);
					}
					/*else
						m_pOwner->ApplyPoint(ia.bType, bAdd ? ia.sValue : ia.sValue ^ 0x00800000);*/
				}
				else
				{
					if (m_pProto->bType == ITEM_COSTUME && m_pProto->bSubType == COSTUME_AURA)
					{
						if (ia.sValue <= 0)
							continue;

						float fValue = ia.sValue * fAuraDrainPer;
						lValue = static_cast<int>((fValue < 1.0f) ? ceilf(fValue) : truncf(fValue));
						//m_pOwner->ApplyPoint(ia.bType, bAdd ? iValue : -iValue);
					}
					/*else
						m_pOwner->ApplyPoint(ia.bType, bAdd ? ia.sValue : -ia.sValue);*/
				}
#else
				/*if (ia.bType == APPLY_SKILL)
					m_pOwner->ApplyPoint(ia.bType, bAdd ? ia.sValue : ia.sValue ^ 0x00800000);
				else
					m_pOwner->ApplyPoint(ia.bType, bAdd ? ia.sValue : -ia.sValue);*/
#endif
				//long sValue = ia.sValue;
#ifdef __SASH_SYSTEM__
				if ((GetType() == ITEM_COSTUME) && (GetSubType() == COSTUME_SASH))
				{
					double dValue = lValue * GetSocket(SASH_ABSORPTION_SOCKET);
					dValue = (double)dValue / 100;
					dValue = (double)dValue + .5;
					lValue = (long) dValue;
					if ((ia.sValue > 0) && (lValue <= 0))
						lValue += 1;
				}
#endif
#ifdef ENABLE_DRAGONSOUL_ALCHEMY_PLUS
				//long value = ia.sValue;
				
				if (m_pOwner->DragonSoul_IsDeckActivated() && GetType() == ITEM_DS)
				{
					if (m_pOwner->DragonSoul_IsActiveFullDeck())
					{
						int grade_idx = (GetVnum() / 1000) % 10;
						lValue = lValue + (lValue * grade_idx / 10);
					}
				}
				
				/*if (ia.bType == APPLY_SKILL)
					m_pOwner->ApplyPoint(ia.bType, bAdd ? value : value ^ 0x00800000);
				else
					m_pOwner->ApplyPoint(ia.bType, bAdd ? value : -value);*/
#else
				/*if (ia.bType == APPLY_SKILL)
					m_pOwner->ApplyPoint(ia.bType, bAdd ? ia.sValue : ia.sValue ^ 0x00800000);
				else
					m_pOwner->ApplyPoint(ia.bType, bAdd ? ia.sValue : -ia.sValue);	*/
#endif

				//std::vector<int> RARITY_VALUES = {-10, -5, 0, 3, 5, 10};
				//int rarity = int(GetSocket(RARITY_ITEM_SOCKET));
				//long calc = lValue * int(RARITY_VALUES[rarity]) / 100;
				//switch (GetType())
				//{
					//case ITEM_ARMOR:
					// ITEM_WEAPON:
						// += calc;
				//}

				if (ia.bType == APPLY_SKILL)
					m_pOwner->ApplyPoint(ia.bType, bAdd ? lValue : lValue ^ 0x00800000);
				else
					m_pOwner->ApplyPoint(ia.bType, bAdd ? lValue : -lValue);
			}
		}
	}
#ifdef ENABLE_REFINE_ELEMENT
	if(m_pProto->bType == ITEM_WEAPON && GetRefineElementType())
	{
		long lAttPlus = GetRefineElementAttackValue();
		if(lAttPlus > 0)
			m_pOwner->ApplyPoint(APPLY_ATT_GRADE_BONUS, bAdd ? lAttPlus : -lAttPlus);
		
		long lBonusValue = GetRefineElementBonusValue();
		if(lBonusValue > 0)
			// daten mortii tei
			m_pOwner->ApplyPoint(APPLY_ENCHANT_ELECT + (GetRefineElementType() - 1), bAdd ? lBonusValue : -lBonusValue);
	}
#endif

	if (m_pProto->bType == ITEM_ARMOR && GetSocket(4) > 0)
	{
		std::vector<int> rarityValues2 = {0, 20, 50};
		int rarity = int(GetSocket(4));

		long lDefGrade = GetValue(1) + long(GetValue(5) * 2);
		long calc = lDefGrade * int(rarityValues2[rarity]) / 100;

		m_pOwner->ApplyPoint(APPLY_DEF_GRADE_BONUS, bAdd ? calc : -calc);
	}
	switch (m_pProto->bType)
	{
		case ITEM_PICK:
		case ITEM_ROD:
			{
				if (bAdd)
				{
					if (m_wCell == INVENTORY_MAX_NUM + WEAR_WEAPON)
						m_pOwner->SetPart(PART_WEAPON, GetVnum());
				}
				else
				{
					if (m_wCell == INVENTORY_MAX_NUM + WEAR_WEAPON)
						m_pOwner->SetPart(PART_WEAPON, 0);
				}
			}
			break;

		case ITEM_WEAPON:
			{
#ifdef ENABLE_WEAPON_COSTUME_SYSTEM
				if (0 != m_pOwner->GetWear(WEAR_COSTUME_WEAPON))
					break;
#endif

				if (bAdd)
				{
					if (m_wCell == INVENTORY_MAX_NUM + WEAR_WEAPON)
#ifdef __CHANGELOOK_SYSTEM__
					{
						DWORD dwRes = GetTransmutation() != 0 ? GetTransmutation() : GetVnum();
						m_pOwner->SetPart(PART_WEAPON, dwRes);
					}
#else
					m_pOwner->SetPart(PART_WEAPON, GetVnum());
#endif
				}
				else
				{
					if (m_wCell == INVENTORY_MAX_NUM + WEAR_WEAPON)
						m_pOwner->SetPart(PART_WEAPON, 0);
				}
			}
			break;

		case ITEM_ARMOR:
			{
				// �ڽ��� body�� �԰��ִٸ� armor�� ���� �Դ� ��� ���� ���־� ������ �ָ� �� ��.
				if (0 != m_pOwner->GetWear(WEAR_COSTUME_BODY))
					break;

				if (GetSubType() == ARMOR_BODY || GetSubType() == ARMOR_HEAD || GetSubType() == ARMOR_FOOTS || GetSubType() == ARMOR_SHIELD)
				{
					if (bAdd)
					{
						if (GetProto()->bSubType == ARMOR_BODY)
						{
#ifdef __CHANGELOOK_SYSTEM__
							DWORD dwRes = GetTransmutation() != 0 ? GetTransmutation() : GetVnum();
							m_pOwner->SetPart(PART_MAIN, dwRes);
#else
							m_pOwner->SetPart(PART_MAIN, GetVnum());
#endif
						}
					}
					else
					{
						if (GetProto()->bSubType == ARMOR_BODY)
							m_pOwner->SetPart(PART_MAIN, m_pOwner->GetOriginalPart(PART_MAIN));
					}
				}
			}
			break;

		// �ڽ��� ������ �Ծ��� �� ĳ���� parts ���� ����. ���� ��Ÿ�ϴ�� �߰���..
		case ITEM_COSTUME:
			{
				DWORD toSetValue = this->GetVnum();
				EParts toSetPart = PART_MAX_NUM;

				if (GetSubType() == COSTUME_BODY)
				{
					toSetPart = PART_MAIN;
					if (false == bAdd)
					{
						const CItem* pArmor = m_pOwner->GetWear(WEAR_BODY);
						toSetValue = (NULL != pArmor) ? pArmor->GetVnum() : m_pOwner->GetOriginalPart(PART_MAIN);
#ifdef __CHANGELOOK_SYSTEM__
						if (pArmor)
							toSetValue = pArmor->GetTransmutation() != 0 ? pArmor->GetTransmutation() : pArmor->GetVnum();
#endif
					}
#ifdef __CHANGELOOK_SYSTEM__
					else
						toSetValue = GetTransmutation() != 0 ? GetTransmutation() : GetVnum();
#endif
				}

				else if (GetSubType() == COSTUME_HAIR)
				{
					toSetPart = PART_HAIR;
					if (!bAdd)
						toSetValue = 0;
					else
					{
#ifdef __CHANGELOOK_SYSTEM__
						DWORD dwTransmutation = GetTransmutation();
						if (dwTransmutation != 0)
						{
							TItemTable* pItemTable = ITEM_MANAGER::instance().GetTable(dwTransmutation);
							toSetValue = (pItemTable != NULL) ? pItemTable->alValues[3] : GetValue(3);
						}
						else
							toSetValue = GetValue(3);
#else
						toSetValue = GetValue(3);
#endif
					}
				}
#ifdef __AURA_SYSTEM__
				else if (GetSubType() == COSTUME_AURA)
				{
					if (m_wCell == INVENTORY_MAX_NUM + WEAR_COSTUME_AURA)
						m_pOwner->SetPart(PART_AURA, bAdd ? GetVnum() : m_pOwner->GetOriginalPart(PART_AURA));
				}
#endif				


#ifdef __SASH_SYSTEM__
				else if (GetSubType() == COSTUME_SASH)
				{

#ifdef __CHANGELOOK_SYSTEM__					
					DWORD dwTransmutation = GetTransmutation();
					if (dwTransmutation != 0)
						toSetValue = GetTransmutation();
#endif	

					toSetValue -= 85000;
					if (GetSocket(SASH_ABSORPTION_SOCKET) >= SASH_EFFECT_FROM_ABS)
						toSetValue += 2000;
					
					toSetValue = (bAdd == true) ? toSetValue : 0;
					toSetPart = PART_SASH;
				}
#endif

#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
				else if (GetSubType() == COSTUME_MOUNT)
				{
					// not need to do a thing in here
				}
#endif

// #ifdef ENABLE_WEAPON_COSTUME_SYSTEM
				else if (GetSubType() == COSTUME_WEAPON)
				{
					toSetPart = PART_WEAPON;
					if (false == bAdd)
					{
						const CItem* pWeapon = m_pOwner->GetWear(WEAR_WEAPON);
						toSetValue = (NULL != pWeapon) ? pWeapon->GetVnum() : m_pOwner->GetOriginalPart(PART_WEAPON);
// #ifdef __CHANGELOOK_SYSTEM__
						if (pWeapon)
							toSetValue = pWeapon->GetTransmutation() != 0 ? pWeapon->GetTransmutation() : pWeapon->GetVnum();
// #endif
					}
					else {
						toSetValue = GetTransmutation() != 0 ? GetTransmutation() : GetVnum();
					}
// #ifdef __CHANGELOOK_SYSTEM__
				}
// #endif
// #endif

				if (PART_MAX_NUM != toSetPart)
				{
					m_pOwner->SetPart((BYTE)toSetPart, toSetValue);
					m_pOwner->UpdatePacket();
				}
			}
			break;
		case ITEM_UNIQUE:
			{
				if (0 != GetSIGVnum())
				{
					const CSpecialItemGroup* pItemGroup = ITEM_MANAGER::instance().GetSpecialItemGroup(GetSIGVnum());
					if (NULL == pItemGroup)
						break;
					DWORD dwAttrVnum = pItemGroup->GetAttrVnum(GetVnum());
					const CSpecialAttrGroup* pAttrGroup = ITEM_MANAGER::instance().GetSpecialAttrGroup(dwAttrVnum);
					if (NULL == pAttrGroup)
						break;
					for (itertype (pAttrGroup->m_vecAttrs) it = pAttrGroup->m_vecAttrs.begin(); it != pAttrGroup->m_vecAttrs.end(); it++)
					{
						m_pOwner->ApplyPoint(it->apply_type, bAdd ? it->apply_value : -it->apply_value);
					}
				}
			}
			break;
			
	   case ITEM_RING:
			{
				if (0 != GetSIGVnum())
				{
					const CSpecialItemGroup* pItemGroup = ITEM_MANAGER::instance().GetSpecialItemGroup(GetSIGVnum());
					if (NULL == pItemGroup)
						break;
					DWORD dwAttrVnum = pItemGroup->GetAttrVnum(GetVnum());
					const CSpecialAttrGroup* pAttrGroup = ITEM_MANAGER::instance().GetSpecialAttrGroup(dwAttrVnum);
					if (NULL == pAttrGroup)
						break;
					for (auto it = pAttrGroup->m_vecAttrs.begin(); it != pAttrGroup->m_vecAttrs.end(); it++)
					{
						m_pOwner->ApplyPoint(it->apply_type, bAdd ? it->apply_value : -it->apply_value);
					}
				}
			}
			break;
	}
}

bool CItem::IsEquipable() const
{
	switch (this->GetType())
	{
	case ITEM_COSTUME:
	case ITEM_ARMOR:
	case ITEM_WEAPON:
	case ITEM_ROD:
	case ITEM_PICK:
	case ITEM_UNIQUE:
	case ITEM_DS:
	case ITEM_SPECIAL_DS:
	case ITEM_RING:
	case ITEM_BELT:
	case ITEM_TALISMAN:
		return true;
	}

	return false;
}


#define ENABLE_IMMUNE_FIX
// return false on error state
bool CItem::EquipTo(LPCHARACTER ch, BYTE bWearCell)
{
	if (!ch)
	{
		sys_err("EquipTo: nil character");
		return false;
	}

	// ��ȥ�� ���� index�� WEAR_MAX_NUM ���� ŭ.
	if (IsDragonSoul())
	{
		if (bWearCell < WEAR_MAX_NUM || bWearCell >= WEAR_MAX_NUM + DRAGON_SOUL_DECK_MAX_NUM * DS_SLOT_MAX)
		{
			sys_err("EquipTo: invalid dragon soul cell (this: #%d %s wearflag: %d cell: %d)", GetOriginalVnum(), GetName(), GetSubType(), bWearCell - WEAR_MAX_NUM);
			return false;
		}
	}
	else
	{
		if (bWearCell >= WEAR_MAX_NUM)
		{
			sys_err("EquipTo: invalid wear cell (this: #%d %s wearflag: %d cell: %d)", GetOriginalVnum(), GetName(), GetWearFlag(), bWearCell);
			return false;
		}
	}

	if (ch->GetWear(bWearCell))
	{
		sys_err("EquipTo: item already exist (this: #%d %s cell: %d %s)", GetOriginalVnum(), GetName(), bWearCell, ch->GetWear(bWearCell)->GetName());
		return false;
	}

	if (GetOwner())
		RemoveFromCharacter();

	ch->SetWear(bWearCell, this); // ���⼭ ��Ŷ ����

	m_pOwner = ch;
	m_bEquipped = true;
	m_wCell	= INVENTORY_MAX_NUM + bWearCell;

#ifndef ENABLE_IMMUNE_FIX
	DWORD dwImmuneFlag = 0;

	for (int i = 0; i < WEAR_MAX_NUM; ++i)
	{
		if (m_pOwner->GetWear(i))
		{
			// m_pOwner->ChatPacket(CHAT_TYPE_INFO, "unequip immuneflag(%u)", m_pOwner->GetWear(i)->m_pProto->dwImmuneFlag); // always 0
			SET_BIT(dwImmuneFlag, m_pOwner->GetWear(i)->m_pProto->dwImmuneFlag);
		}
	}

	m_pOwner->SetImmuneFlag(dwImmuneFlag);
#endif

	if (IsDragonSoul())
	{
		DSManager::instance().ActivateDragonSoul(this);
	}
	else
	{
		ModifyPoints(true);
		StartUniqueExpireEvent();
		if (-1 != GetProto()->cLimitTimerBasedOnWearIndex)
			StartTimerBasedOnWearExpireEvent();

		// ACCESSORY_REFINE
		StartAccessorySocketExpireEvent();
#ifdef __AURA_SYSTEM__
		StartAuraBoosterSocketExpireEvent();
#endif
		// END_OF_ACCESSORY_REFINE
	}

	ch->BuffOnAttr_AddBuffsFromItem(this);

#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
	if (IsMountItem())
		ch->MountSummon(this);
#endif

#ifdef ENABLE_PET_COSTUME_SYSTEM
	if (IsPetItem())
		ch->PetSummon(this);
#endif

#ifdef __NEW_SET_BONUS__
	if (m_pOwner->IsNewSetNeedRefresh(GetVnum()))
		m_pOwner->ComputePoints();
	else
	{
		m_pOwner->ComputeBattlePoints();
		m_pOwner->UpdatePacket();
	}
#else
	m_pOwner->ComputeBattlePoints();
	m_pOwner->UpdatePacket();
#endif

	Save();

	return (true);
}

bool CItem::Unequip()
{
	if (!m_pOwner || GetCell() < INVENTORY_MAX_NUM)
	{
		// ITEM_OWNER_INVALID_PTR_BUG
		sys_err("%s %u m_pOwner %p, GetCell %d",
				GetName(), GetID(), get_pointer(m_pOwner), GetCell());
		// END_OF_ITEM_OWNER_INVALID_PTR_BUG
		return false;
	}

	if (this != m_pOwner->GetWear(GetCell() - INVENTORY_MAX_NUM))
	{
		sys_err("m_pOwner->GetWear() != this");
		return false;
	}

#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
	if (IsMountItem())
		m_pOwner->MountUnsummon(this);
#endif

#ifdef ENABLE_PET_COSTUME_SYSTEM
	if (IsPetItem())
		m_pOwner->PetUnsummon(this);
#endif

	//�ű� �� ������ ���Ž� ó��
	if (IsRideItem())
		ClearMountAttributeAndAffect();

	if (IsDragonSoul())
	{
		DSManager::instance().DeactivateDragonSoul(this);
	}
	else
	{
		ModifyPoints(false);
	}

	StopUniqueExpireEvent();

	if (-1 != GetProto()->cLimitTimerBasedOnWearIndex)
		StopTimerBasedOnWearExpireEvent();

	// ACCESSORY_REFINE
	StopAccessorySocketExpireEvent();
#ifdef __AURA_SYSTEM__
	StopAuraBoosterSocketExpireEvent();
#endif
	// END_OF_ACCESSORY_REFINE


	m_pOwner->BuffOnAttr_RemoveBuffsFromItem(this);

	m_pOwner->SetWear(GetCell() - INVENTORY_MAX_NUM, NULL);

#ifndef ENABLE_IMMUNE_FIX
	DWORD dwImmuneFlag = 0;

	for (int i = 0; i < WEAR_MAX_NUM; ++i)
	{
		if (m_pOwner->GetWear(i))
		{
			// m_pOwner->ChatPacket(CHAT_TYPE_INFO, "unequip immuneflag(%u)", m_pOwner->GetWear(i)->m_pProto->dwImmuneFlag); // always 0
			SET_BIT(dwImmuneFlag, m_pOwner->GetWear(i)->m_pProto->dwImmuneFlag);
		}
	}

	m_pOwner->SetImmuneFlag(dwImmuneFlag);
#endif
#ifdef __NEW_SET_BONUS__
	if (m_pOwner->IsNewSetNeedRefresh(GetVnum()))
		m_pOwner->ComputePoints();
	else
	{
		m_pOwner->ComputeBattlePoints();
		m_pOwner->UpdatePacket();
	}
#else
	m_pOwner->ComputeBattlePoints();
	m_pOwner->UpdatePacket();
#endif

	m_pOwner = NULL;
	m_wCell = 0;
	m_bEquipped	= false;

	return true;
}

long CItem::GetValue(DWORD idx)
{
	assert(idx < ITEM_VALUES_MAX_NUM);
	return GetProto()->alValues[idx];
}

void CItem::SetExchanging(bool bOn)
{
	m_bExchanging = bOn;
}

void CItem::Save()
{
	if (m_bSkipSave)
		return;
	
	ITEM_MANAGER::instance().DelayedSave(this);
}

bool CItem::CreateSocket(BYTE bSlot, BYTE bGold)
{
	assert(bSlot < ITEM_SOCKET_MAX_NUM);

	if (m_alSockets[bSlot] != 0)
	{
		sys_err("Item::CreateSocket : socket already exist %s %d", GetName(), bSlot);
		return false;
	}

	if (bGold)
		m_alSockets[bSlot] = 2;
	else
		m_alSockets[bSlot] = 1;

	UpdatePacket();

	Save();
	return true;
}
void CItem::SetSockets(const long * c_al)
{
	thecore_memcpy(m_alSockets, c_al, sizeof(m_alSockets));
	Save();
}

void CItem::SetSocket(int i, long v, bool bLog)
{
	assert(i < ITEM_SOCKET_MAX_NUM);
	m_alSockets[i] = v;
	UpdatePacket();
	Save();
	if (bLog)
	{
#ifdef ENABLE_NEWSTUFF
		if (g_iDbLogLevel>=LOG_LEVEL_MAX)
#endif
		LogManager::instance().ItemLog(i, v, 0, GetID(), "SET_SOCKET", "", "", GetOriginalVnum());
	}
}

#ifdef ENABLE_REMOVE_LIMIT_GOLD
unsigned long long CItem::GetGold()
{
	if (IS_SET(GetFlag(), ITEM_FLAG_COUNT_PER_1GOLD))
	{
		if (GetProto()->dwGold == 0)
			return GetCount();
		else
			return GetCount() / GetProto()->dwGold;
	}
	else
		return GetProto()->dwGold;
}

unsigned long long CItem::GetShopBuyPrice()
{
	return GetProto()->dwShopBuyPrice;
}
#else
int CItem::GetGold()
{
	if (IS_SET(GetFlag(), ITEM_FLAG_COUNT_PER_1GOLD))
	{
		if (GetProto()->dwGold == 0)
			return GetCount();
		else
			return GetCount() / GetProto()->dwGold;
	}
	else
		return GetProto()->dwGold;
}

int CItem::GetShopBuyPrice()
{
	return GetProto()->dwShopBuyPrice;
}
#endif

bool CItem::IsOwnership(LPCHARACTER ch)
{
	if (!m_pkOwnershipEvent)
		return true;

	return m_dwOwnershipPID == ch->GetPlayerID() ? true : false;
}

EVENTFUNC(ownership_event)
{
	item_event_info* info = dynamic_cast<item_event_info*>( event->info );

	if ( info == NULL )
	{
		sys_err( "ownership_event> <Factor> Null pointer" );
		return 0;
	}

	LPITEM pkItem = info->item;

	pkItem->SetOwnershipEvent(NULL);

	TPacketGCItemOwnership p;

	p.bHeader	= HEADER_GC_ITEM_OWNERSHIP;
	p.dwVID	= pkItem->GetVID();
	p.szName[0]	= '\0';

	pkItem->PacketAround(&p, sizeof(p));
	return 0;
}

void CItem::SetOwnershipEvent(LPEVENT pkEvent)
{
	m_pkOwnershipEvent = pkEvent;
}

void CItem::SetOwnership(LPCHARACTER ch, int iSec)
{
	if (!ch)
	{
		if (m_pkOwnershipEvent)
		{
			event_cancel(&m_pkOwnershipEvent);
			m_dwOwnershipPID = 0;

			TPacketGCItemOwnership p;

			p.bHeader	= HEADER_GC_ITEM_OWNERSHIP;
			p.dwVID	= m_dwVID;
			p.szName[0]	= '\0';

			PacketAround(&p, sizeof(p));
		}
		return;
	}

	if (m_pkOwnershipEvent)
		return;

	if (iSec <= 10)
		iSec = 30;

	m_dwOwnershipPID = ch->GetPlayerID();

	item_event_info* info = AllocEventInfo<item_event_info>();
	strlcpy(info->szOwnerName, ch->GetName(), sizeof(info->szOwnerName));
	info->item = this;

	SetOwnershipEvent(event_create(ownership_event, info, PASSES_PER_SEC(iSec)));

	TPacketGCItemOwnership p;

	p.bHeader = HEADER_GC_ITEM_OWNERSHIP;
	p.dwVID = m_dwVID;
	strlcpy(p.szName, ch->GetName(), sizeof(p.szName));

	PacketAround(&p, sizeof(p));
}

int CItem::GetSocketCount()
{
	for (int i = 0; i < ITEM_SOCKET_MAX_NUM; i++)
	{
		if (GetSocket(i) == 0)
			return i;
	}
	return ITEM_SOCKET_MAX_NUM;
}

bool CItem::AddSocket()
{
	int count = GetSocketCount();
	if (count == ITEM_SOCKET_MAX_NUM)
		return false;
	m_alSockets[count] = 1;
	return true;
}

void CItem::AlterToSocketItem(int iSocketCount)
{
	if (iSocketCount >= ITEM_SOCKET_MAX_NUM)
	{
		sys_log(0, "Invalid Socket Count %d, set to maximum", ITEM_SOCKET_MAX_NUM);
		iSocketCount = ITEM_SOCKET_MAX_NUM;
	}

	for (int i = 0; i < iSocketCount; ++i)
		SetSocket(i, 1);
}

void CItem::AlterToMagicItem()
{
	int idx = GetAttributeSetIndex();

	if (idx < 0)
		return;

	//      Appeariance Second Third
	// Weapon 50        20     5
	// Armor  30        10     2
	// Acc    20        10     1

	int iSecondPct;
	int iThirdPct;

	switch (GetType())
	{
		case ITEM_WEAPON:
			iSecondPct = 20;
			iThirdPct = 5;
			break;

		case ITEM_ARMOR:
		case ITEM_COSTUME:
			if (GetSubType() == ARMOR_BODY)
			{
				iSecondPct = 10;
				iThirdPct = 2;
			}
#ifdef __AURA_SYSTEM__
				else if (GetType() == ITEM_COSTUME && GetSubType() == COSTUME_AURA)
				{
					iSecondPct = 0;
					iThirdPct = 0;
				}
#endif
			else
			{
				iSecondPct = 10;
				iThirdPct = 1;
			}
			break;

		default:
			return;
	}

	// 100% Ȯ���� ���� �Ӽ� �ϳ�
	PutAttribute(aiItemMagicAttributePercentHigh);

	if (number(1, 100) <= iSecondPct)
		PutAttribute(aiItemMagicAttributePercentLow);

	if (number(1, 100) <= iThirdPct)
		PutAttribute(aiItemMagicAttributePercentLow);
}

DWORD CItem::GetRefineFromVnum()
{
	return ITEM_MANAGER::instance().GetRefineFromVnum(GetVnum());
}

int CItem::GetRefineLevel()
{
	const char* name = GetBaseName();
	char* p = const_cast<char*>(strrchr(name, '+'));

	if (!p)
		return 0;

	int	rtn = 0;
	str_to_number(rtn, p+1);

	const char* locale_name = GetName();
	p = const_cast<char*>(strrchr(locale_name, '+'));

	if (p)
	{
		int	locale_rtn = 0;
		str_to_number(locale_rtn, p+1);
		if (locale_rtn != rtn)
		{
			sys_err("refine_level_based_on_NAME(%d) is not equal to refine_level_based_on_LOCALE_NAME(%d).", rtn, locale_rtn);
		}
	}

	return rtn;
}

bool CItem::IsPolymorphItem()
{
	return GetType() == ITEM_POLYMORPH;
}
EVENTFUNC(unique_expire_event)
{
	item_event_info* info = dynamic_cast<item_event_info*>( event->info );

	if ( info == NULL )
	{
		sys_err( "unique_expire_event> <Factor> Null pointer" );
		return 0;
	}

	LPITEM pkItem = info->item;

	if (pkItem->GetValue(2) == 0)
	{
		if (pkItem->GetSocket(ITEM_SOCKET_UNIQUE_REMAIN_TIME) <= 1)
		{
			sys_log(0, "UNIQUE_ITEM: expire %s %u", pkItem->GetName(), pkItem->GetID());
			pkItem->SetUniqueExpireEvent(NULL);
			
			
#ifdef __PREMIUM_PRIVATE_SHOP__
			if (pkItem->GetPrivateShop())
			{
				sys_log(0, "%s PRIVATE_SHOP: ITEM UNIQUE_EXPIRE id %u vnum %u cell %u", pkItem->GetPrivateShop()->GetOwnerName().c_str(),
					pkItem->GetID(), pkItem->GetVnum(), pkItem->GetCell());
				CPrivateShopManager::Instance().SendItemExpire(pkItem);
				return 0;
			}
#endif
			
			ITEM_MANAGER::instance().RemoveItem(pkItem, "UNIQUE_EXPIRE");
			return 0;
		}
		else
		{
			pkItem->SetSocket(ITEM_SOCKET_UNIQUE_REMAIN_TIME, pkItem->GetSocket(ITEM_SOCKET_UNIQUE_REMAIN_TIME) - 1);
			return PASSES_PER_SEC(60);
		}
	}
	else
	{
		time_t cur = get_global_time();

		if (pkItem->GetSocket(ITEM_SOCKET_UNIQUE_REMAIN_TIME) <= cur)
		{
			pkItem->SetUniqueExpireEvent(NULL);
			
			
#ifdef __PREMIUM_PRIVATE_SHOP__
			if (pkItem->GetPrivateShop())
			{
				sys_log(0, "%s PRIVATE_SHOP: ITEM UNIQUE_EXPIRE id %u vnum %u cell %u", pkItem->GetPrivateShop()->GetOwnerName().c_str(),
					pkItem->GetID(), pkItem->GetVnum(), pkItem->GetCell());
				CPrivateShopManager::Instance().SendItemExpire(pkItem);
				return 0;
			}
#endif
			
			
			ITEM_MANAGER::instance().RemoveItem(pkItem, "UNIQUE_EXPIRE");
			return 0;
		}
		else
		{
			// ���� ���� �ð��� �����۵��� ���������ϰ� ������� �ʴ� ���װ� �־�
			// ����
			// by rtsummit
			if (pkItem->GetSocket(ITEM_SOCKET_UNIQUE_REMAIN_TIME) - cur < 600)
				return PASSES_PER_SEC(pkItem->GetSocket(ITEM_SOCKET_UNIQUE_REMAIN_TIME) - cur);
			else
				return PASSES_PER_SEC(600);
		}
	}
}

// �ð� �ĺ���
// timer�� ������ ���� �ð� �����ϴ� ���� �ƴ϶�,
// timer�� ��ȭ�� ���� timer�� ������ �ð� ��ŭ �ð� ������ �Ѵ�.
EVENTFUNC(timer_based_on_wear_expire_event)
{
	item_event_info* info = dynamic_cast<item_event_info*>( event->info );

	if ( info == NULL )
	{
		sys_err( "expire_event <Factor> Null pointer" );
		return 0;
	}

	LPITEM pkItem = info->item;
	
	BYTE bPassesPerSec = 60;
	
	if (pkItem->GetType() == ITEM_TOGGLE)
		bPassesPerSec = 1;
	
	int remain_time = pkItem->GetSocket(ITEM_SOCKET_REMAIN_SEC) - processing_time/passes_per_sec;
	if (remain_time <= 0)
	{
		sys_log(0, "ITEM EXPIRED : expired %s %u", pkItem->GetName(), pkItem->GetID());
		pkItem->SetTimerBasedOnWearExpireEvent(NULL);
		pkItem->SetSocket(ITEM_SOCKET_REMAIN_SEC, 0);

		// �ϴ� timer based on wear ��ȥ���� �ð� �� �Ǿ��ٰ� ������ �ʴ´�.
		if (pkItem->IsDragonSoul())
		{
			DSManager::instance().DeactivateDragonSoul(pkItem);
		}
		else
		{
			ITEM_MANAGER::instance().RemoveItem(pkItem, "TIMER_BASED_ON_WEAR_EXPIRE");
		}
		return 0;
	}
	pkItem->SetSocket(ITEM_SOCKET_REMAIN_SEC, remain_time);
	return PASSES_PER_SEC (MIN (bPassesPerSec, remain_time));
}

void CItem::SetUniqueExpireEvent(LPEVENT pkEvent)
{
	m_pkUniqueExpireEvent = pkEvent;
}

void CItem::SetTimerBasedOnWearExpireEvent(LPEVENT pkEvent)
{
	m_pkTimerBasedOnWearExpireEvent = pkEvent;
}

EVENTFUNC(real_time_expire_event)
{
	const item_vid_event_info* info = reinterpret_cast<const item_vid_event_info*>(event->info);

	if (NULL == info)
		return 0;

	const LPITEM item = ITEM_MANAGER::instance().FindByVID( info->item_vid );

	if (NULL == item)
		return 0;
	
	const time_t current = get_global_time();

	if (current > item->GetSocket(0))
	{
		
#ifdef __PREMIUM_PRIVATE_SHOP__
		if (item->GetPrivateShop())
		{
			sys_log(0, "%s PRIVATE_SHOP: ITEM EXPIRE id %u vnum %u cell %u", item->GetPrivateShop()->GetOwnerName().c_str(),
				item->GetID(), item->GetVnum(), item->GetCell());
			CPrivateShopManager::Instance().SendItemExpire(item);
			return 0;
		}
#endif

#ifdef __CHANGELOOK_SYSTEM__
		if (item->isLocked())
		{
			if (LPCHARACTER ch = item->GetOwner())
				ch->ClearChangeLookWindow();
		}
#endif
		ITEM_MANAGER::instance().RemoveItem(item, "REAL_TIME_EXPIRE");
		return 0;
	}

	return PASSES_PER_SEC(1);
}

void CItem::StartRealTimeExpireEvent()
{
	if (m_pkRealTimeExpireEvent)
		return;
	
	for (int i=0 ; i < ITEM_LIMIT_MAX_NUM ; i++)
	{
		if (LIMIT_REAL_TIME == GetProto()->aLimits[i].bType || LIMIT_REAL_TIME_START_FIRST_USE == GetProto()->aLimits[i].bType)
		{
			item_vid_event_info* info = AllocEventInfo<item_vid_event_info>();
			info->item_vid = GetVID();

			m_pkRealTimeExpireEvent = event_create( real_time_expire_event, info, PASSES_PER_SEC(1));

			sys_log(0, "REAL_TIME_EXPIRE: StartRealTimeExpireEvent");

			return;
		}
	}
}

#ifdef ENABLE_AFFECT_RENEWAL
EVENTFUNC(blend_use_expire_event)
{
	item_event_info* info = dynamic_cast<item_event_info*>(event->info);

	if (info == nullptr)
	{
		sys_err("expire_event <Factor> Null pointer");
		return 0;
	}

	LPITEM pkItem = info->item;
	int remain_time = pkItem->GetSocket(2) - processing_time / passes_per_sec;
	if (remain_time <= 0)
	{
		pkItem->SetSocket(2, 0);
		pkItem->SetSocket(3, false);
		pkItem->Lock(false);
		// ITEM_MANAGER::instance().RemoveItem(pkItem, "BLEND_USE_EXPIRE");
		pkItem->SetCount(pkItem->GetCount() - 1);
		return 0;
	}

	pkItem->SetSocket(2, remain_time, false);
	return PASSES_PER_SEC(MIN(1, remain_time));
}

void CItem::StartBlendExpireEvent()
{
	if (m_pkBlendUseEvent)
		return;

	if (IsRealTimeItem())
		return;

	if (!IsBlendItem())
		return;

	int iSec = GetSocket(2);

	if (iSec >= 8640000)
		return;

	item_event_info* info = AllocEventInfo<item_event_info>();
	info->item = this;

	m_pkBlendUseEvent = event_create(blend_use_expire_event, info, PASSES_PER_SEC(1));
}

void CItem::StopBlendExpireEvent()
{
	if (!m_pkBlendUseEvent)
		return;

	int remain_time = GetSocket(2) - event_processing_time(m_pkBlendUseEvent) / passes_per_sec;

	SetSocket(2, remain_time, false);
	event_cancel(&m_pkBlendUseEvent);

	ITEM_MANAGER::instance().SaveSingleItem(this);
}
#endif

bool CItem::IsRealTimeItem()
{
	if(!GetProto())
		return false;
	for (int i=0 ; i < ITEM_LIMIT_MAX_NUM ; i++)
	{
		if (LIMIT_REAL_TIME == GetProto()->aLimits[i].bType)
			return true;
	}
	return false;
}

void CItem::StartUniqueExpireEvent()
{
	if (GetType() != ITEM_UNIQUE)
		return;

	if (m_pkUniqueExpireEvent)
		return;

	//�Ⱓ�� �������� ��� �ð��� �������� �������� �ʴ´�
	if (IsRealTimeItem())
		return;

	// HARD CODING
	if (GetVnum() == UNIQUE_ITEM_HIDE_ALIGNMENT_TITLE)
		m_pOwner->ShowAlignment(false);

	int iSec = GetSocket(ITEM_SOCKET_UNIQUE_SAVE_TIME);

	if (iSec == 0)
		iSec = 60;
	else
		iSec = MIN(iSec, 60);

	SetSocket(ITEM_SOCKET_UNIQUE_SAVE_TIME, 0);

	item_event_info* info = AllocEventInfo<item_event_info>();
	info->item = this;

	SetUniqueExpireEvent(event_create(unique_expire_event, info, PASSES_PER_SEC(iSec)));
}

// �ð� �ĺ���
// timer_based_on_wear_expire_event ���� ����
void CItem::StartTimerBasedOnWearExpireEvent()
{
	if (m_pkTimerBasedOnWearExpireEvent)
		return;

	//�Ⱓ�� �������� ��� �ð��� �������� �������� �ʴ´�
	if (IsRealTimeItem())
		return;
	
	if (-1 == GetProto()->cLimitTimerBasedOnWearIndex)
		return;

	int iSec = GetSocket(0);

	// ���� �ð��� �д����� ���� ����...
	if (0 != iSec)
	{
		iSec %= 60;
		if (0 == iSec)
			iSec = 60;
		if (GetType() == ITEM_TOGGLE)
			iSec = 1;
	}

	item_event_info* info = AllocEventInfo<item_event_info>();
	info->item = this;

	SetTimerBasedOnWearExpireEvent(event_create(timer_based_on_wear_expire_event, info, PASSES_PER_SEC(iSec)));
}

void CItem::StopUniqueExpireEvent()
{
	if (!m_pkUniqueExpireEvent)
		return;

	if (GetValue(2) != 0) // ���ӽð��� �̿��� �������� UniqueExpireEvent�� �ߴ��� �� ����.
		return;

	// HARD CODING
	if (GetVnum() == UNIQUE_ITEM_HIDE_ALIGNMENT_TITLE)
		m_pOwner->ShowAlignment(true);

	SetSocket(ITEM_SOCKET_UNIQUE_SAVE_TIME, event_time(m_pkUniqueExpireEvent) / passes_per_sec);
	event_cancel(&m_pkUniqueExpireEvent);

	ITEM_MANAGER::instance().SaveSingleItem(this);
}

void CItem::StopTimerBasedOnWearExpireEvent()
{
	if (!m_pkTimerBasedOnWearExpireEvent)
		return;

	int remain_time = GetSocket(ITEM_SOCKET_REMAIN_SEC) - event_processing_time(m_pkTimerBasedOnWearExpireEvent) / passes_per_sec;

	SetSocket(ITEM_SOCKET_REMAIN_SEC, remain_time);
	event_cancel(&m_pkTimerBasedOnWearExpireEvent);

	ITEM_MANAGER::instance().SaveSingleItem(this);
}


void CItem::ApplyAddon(int iAddonType)

{
	CItemAddonManager::instance().ApplyAddonTo(iAddonType, this);
}

int CItem::GetSpecialGroup() const
{
	return ITEM_MANAGER::instance().GetSpecialGroupFromItem(GetVnum());
}

//
// �Ǽ����� ���� ó��.
//
bool CItem::IsAccessoryForSocket()
{
	return (m_pProto->bType == ITEM_ARMOR && (m_pProto->bSubType == ARMOR_WRIST || m_pProto->bSubType == ARMOR_NECK || m_pProto->bSubType == ARMOR_EAR)) ||
		(m_pProto->bType == ITEM_BELT);				// 2013�� 2�� ���� �߰��� '��Ʈ' �������� ��� ��ȹ������ �Ǽ����� ���� �ý����� �״�� �̿����ڰ� ��.
}

void CItem::SetAccessorySocketGrade(int iGrade)
{
	SetSocket(0, MINMAX(0, iGrade, GetAccessorySocketMaxGrade()));

	int iDownTime = aiAccessorySocketDegradeTime[GetAccessorySocketGrade()];

	//if (test_server)
	//	iDownTime /= 60;

	SetAccessorySocketDownGradeTime(iDownTime);
}

void CItem::SetAccessorySocketMaxGrade(int iMaxGrade)
{
	SetSocket(1, MINMAX(0, iMaxGrade, ITEM_ACCESSORY_SOCKET_MAX_NUM));
}

void CItem::SetAccessorySocketDownGradeTime(DWORD time)
{
	SetSocket(2, time);

	if (test_server && GetOwner())
		GetOwner()->ChatPacket(CHAT_TYPE_INFO, "[LS;469;%s;%d]", GetName(), time);
}

EVENTFUNC(accessory_socket_expire_event)
{
	item_vid_event_info* info = dynamic_cast<item_vid_event_info*>( event->info );

	if ( info == NULL )
	{
		sys_err( "accessory_socket_expire_event> <Factor> Null pointer" );
		return 0;
	}

	LPITEM item = ITEM_MANAGER::instance().FindByVID(info->item_vid);

	if (item->GetAccessorySocketDownGradeTime() <= 1)
	{
degrade:
		item->SetAccessorySocketExpireEvent(NULL);
		item->AccessorySocketDegrade();
		return 0;
	}
	else
	{
		int iTime = item->GetAccessorySocketDownGradeTime() - 60;

		if (iTime <= 1)
			goto degrade;

		item->SetAccessorySocketDownGradeTime(iTime);

		if (iTime > 60)
			return PASSES_PER_SEC(60);
		else
			return PASSES_PER_SEC(iTime);
	}
}

void CItem::StartAccessorySocketExpireEvent()
{
	if (!IsAccessoryForSocket())
		return;

	if (m_pkAccessorySocketExpireEvent)
		return;

	if (GetAccessorySocketMaxGrade() == 0)
		return;

	if (GetAccessorySocketGrade() == 0)
		return;

	int iSec = GetAccessorySocketDownGradeTime();
	SetAccessorySocketExpireEvent(NULL);

	if (iSec <= 1)
		iSec = 5;
	else
		iSec = MIN(iSec, 60);

	item_vid_event_info* info = AllocEventInfo<item_vid_event_info>();
	info->item_vid = GetVID();

	SetAccessorySocketExpireEvent(event_create(accessory_socket_expire_event, info, PASSES_PER_SEC(iSec)));
}

void CItem::StopAccessorySocketExpireEvent()
{
	if (!m_pkAccessorySocketExpireEvent)
		return;

	if (!IsAccessoryForSocket())
		return;

	int new_time = GetAccessorySocketDownGradeTime() - (60 - event_time(m_pkAccessorySocketExpireEvent) / passes_per_sec);

	event_cancel(&m_pkAccessorySocketExpireEvent);

	if (new_time <= 1)
	{
		AccessorySocketDegrade();
	}
	else
	{
		SetAccessorySocketDownGradeTime(new_time);
	}
}

bool CItem::IsRideItem()
{
	if (ITEM_UNIQUE == GetType() && UNIQUE_SPECIAL_RIDE == GetSubType())
		return true;
	if (ITEM_UNIQUE == GetType() && UNIQUE_SPECIAL_MOUNT_RIDE == GetSubType())
		return true;
#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
	if (ITEM_COSTUME == GetType() && COSTUME_MOUNT == GetSubType())
		return true;
#endif
	return false;
}

bool CItem::IsRamadanRing()
{
	if (GetVnum() == UNIQUE_ITEM_RAMADAN_RING)
		return true;
	return false;
}

#ifdef ENABLE_PET_COSTUME_SYSTEM
bool CItem::IsPetItem()
{
	if (GetType() == ITEM_COSTUME && GetSubType() == COSTUME_PET)
		return true;

	return false;
}
#endif

#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
bool CItem::IsMountItem()
{
	if (GetType() == ITEM_COSTUME && GetSubType() == COSTUME_MOUNT)
		return true;

	return false;
}

bool CItem::IsPermanentMount()
{
	bool bIsPermanent = true;
	for (int i = 0; i < ITEM_LIMIT_MAX_NUM; ++i)
	{
		if (m_pProto->aLimits[i].bType != LIMIT_NONE)
		{
			bIsPermanent = false;
			break;
		}
	}

	return bIsPermanent;
}
#endif
//663a36e18e9ece5e6f3a4b76e2bb950e
void CItem::ClearMountAttributeAndAffect()
{
	LPCHARACTER ch = GetOwner();

	ch->RemoveAffect(AFFECT_MOUNT);
	ch->RemoveAffect(AFFECT_MOUNT_BONUS);

	ch->MountVnum(0);

	ch->PointChange(POINT_ST, 0);
	ch->PointChange(POINT_DX, 0);
	ch->PointChange(POINT_HT, 0);
	ch->PointChange(POINT_IQ, 0);
}

// fixme
// �̰� ������ �Ⱦ���... �ٵ� Ȥ�ó� �; ���ܵ�.
// by rtsummit
bool CItem::IsNewMountItem()
{
	switch(GetVnum())
	{
		case 76000: case 76001: case 76002: case 76003:
		case 76004: case 76005: case 76006: case 76007:
		case 76008: case 76009: case 76010: case 76011:
		case 76012: case 76013: case 76014:
			return true;
	}
	return false;
}

void CItem::SetAccessorySocketExpireEvent(LPEVENT pkEvent)
{
	m_pkAccessorySocketExpireEvent = pkEvent;
}

void CItem::AccessorySocketDegrade()
{
	if (GetAccessorySocketGrade() > 0)
	{
		LPCHARACTER ch = GetOwner();

		if (ch)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1948;%s]", GetName());
		}

		ModifyPoints(false);
		SetAccessorySocketGrade(GetAccessorySocketGrade()-1);
		ModifyPoints(true);

		int iDownTime = aiAccessorySocketDegradeTime[GetAccessorySocketGrade()];

		if (test_server)
			iDownTime /= 60;

		SetAccessorySocketDownGradeTime(iDownTime);

		if (iDownTime)
			StartAccessorySocketExpireEvent();
	}
}

// ring�� item�� ���� �� �ִ��� ���θ� üũ�ؼ� ����
static const bool CanPutIntoRing(LPITEM ring, LPITEM item)
{
	//const DWORD vnum = item->GetVnum();
	return false;
}

bool CItem::CanPutInto(LPITEM item)
{
	if (item->GetType() == ITEM_BELT)
		return this->GetSubType() == USE_PUT_INTO_BELT_SOCKET;

	else if(item->GetType() == ITEM_RING)
		return CanPutIntoRing(item, this);
	
#ifdef __AURA_SYSTEM__
	else if (item->GetType() == ITEM_COSTUME && item->GetSubType() == COSTUME_AURA)
		return this->GetSubType() == USE_PUT_INTO_AURA_SOCKET;
#endif

	else if (item->GetType() != ITEM_ARMOR)
		return false;
	
	DWORD vnum = item->GetVnum();

	struct JewelAccessoryInfo
	{
		DWORD jewel;
		DWORD wrist;
		DWORD neck;
		DWORD ear;
	};
	const static JewelAccessoryInfo infos[] = {
		{ 50634, 14420, 16220, 17220 },
		{ 50635, 14500, 16500, 17500 },
		{ 50636, 14520, 16520, 17520 },
		{ 50637, 14540, 16540, 17540 },
		{ 50638, 14560, 16560, 17560 },
		{ 50639, 14570, 16570, 17570 },
	};

	DWORD item_type = (item->GetVnum() / 10) * 10;
	for (size_t i = 0; i < sizeof(infos) / sizeof(infos[0]); i++)
	{
		const JewelAccessoryInfo& info = infos[i];
		switch(item->GetSubType())
		{
		case ARMOR_WRIST:
			if (info.wrist == item_type)
			{
				if (info.jewel == GetVnum())
				{
					return true;
				}
				else
				{
					return false;
				}
			}
			break;
		case ARMOR_NECK:
			if (info.neck == item_type)
			{
				if (info.jewel == GetVnum())
				{
					return true;
				}
				else
				{
					return false;
				}
			}
			break;
		case ARMOR_EAR:
			if (info.ear == item_type)
			{
				if (info.jewel == GetVnum())
				{
					return true;
				}
				else
				{
					return false;
				}
			}
			break;
		}
	}
	if (item->GetSubType() == ARMOR_WRIST)
		vnum -= 14000;
	else if (item->GetSubType() == ARMOR_NECK)
		vnum -= 16000;
	else if (item->GetSubType() == ARMOR_EAR)
		vnum -= 17000;
	else
		return false;

	DWORD type = vnum / 20;

	if (type < 0 || type > 11)
	{
		type = (vnum - 170) / 20;

		if (50623 + type != GetVnum())
			return false;
		else
			return true;
	}
	else if (item->GetVnum() >= 16210 && item->GetVnum() <= 16219)
	{
		if (50625 != GetVnum())
			return false;
		else
			return true;
	}
	else if (item->GetVnum() >= 16230 && item->GetVnum() <= 16239)
	{
		if (50626 != GetVnum())
			return false;
		else
			return true;
	}

	return 50623 + type == GetVnum();
}


bool CItem::CheckItemUseLevel(int nLevel)
{
	for (int i = 0; i < ITEM_LIMIT_MAX_NUM; ++i)
	{
		if (this->m_pProto->aLimits[i].bType == LIMIT_LEVEL)
		{
			if (this->m_pProto->aLimits[i].lValue > nLevel) return false;
			else return true;
		}
	}
	return true;
}


bool CItem::CheckItemUseLevelChampion(int nLevel)
{
	for (int i = 0; i < ITEM_LIMIT_MAX_NUM; ++i)
	{
		if (this->m_pProto->aLimits[i].bType == LIMIT_CHAMPION)
		{
			if (this->m_pProto->aLimits[i].lValue > nLevel) return false;
			else return true;
		}
	}
	return true;
}


long CItem::FindApplyValue(BYTE bApplyType)
{
	if (m_pProto == NULL)
		return 0;

	for (int i = 0; i < ITEM_APPLY_MAX_NUM; ++i)
	{
		if (m_pProto->aApplies[i].bType == bApplyType)
			return m_pProto->aApplies[i].lValue;
	}

	return 0;
}

void CItem::CopySocketTo(LPITEM pItem)
{
	for (int i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
	{
		pItem->m_alSockets[i] = m_alSockets[i];
	}
}

int CItem::GetAccessorySocketGrade()
{
   	return MINMAX(0, GetSocket(0), GetAccessorySocketMaxGrade());
}

int CItem::GetAccessorySocketMaxGrade()
{
   	return MINMAX(0, GetSocket(1), ITEM_ACCESSORY_SOCKET_MAX_NUM);
}

int CItem::GetAccessorySocketDownGradeTime()
{
	return MINMAX(0, GetSocket(2), aiAccessorySocketDegradeTime[GetAccessorySocketGrade()]);
}

void CItem::AttrLog()
{
	const char * pszIP = NULL;

	if (GetOwner() && GetOwner()->GetDesc())
		pszIP = GetOwner()->GetDesc()->GetHostName();

	for (int i = 0; i < ITEM_SOCKET_MAX_NUM; ++i)
	{
		if (m_alSockets[i])
		{
#ifdef ENABLE_NEWSTUFF
			if (g_iDbLogLevel>=LOG_LEVEL_MAX)
#endif
			LogManager::instance().ItemLog(i, m_alSockets[i], 0, GetID(), "INFO_SOCKET", "", pszIP ? pszIP : "", GetOriginalVnum());
		}
	}

	for (int i = 0; i<ITEM_ATTRIBUTE_MAX_NUM; ++i)
	{
		int	type	= m_aAttr[i].bType;
		int value	= m_aAttr[i].sValue;

		if (type)
		{
#ifdef ENABLE_NEWSTUFF
			if (g_iDbLogLevel>=LOG_LEVEL_MAX)
#endif
			LogManager::instance().ItemLog(i, type, value, GetID(), "INFO_ATTR", "", pszIP ? pszIP : "", GetOriginalVnum());
		}
	}
}

int CItem::GetLevelLimit()
{
	for (int i = 0; i < ITEM_LIMIT_MAX_NUM; ++i)
	{
		if (this->m_pProto->aLimits[i].bType == LIMIT_LEVEL)
		{
			return this->m_pProto->aLimits[i].lValue;
		}
	}
	return 0;
}


int CItem::GetLevelChampionLimit()
{
	for (int i = 0; i < ITEM_LIMIT_MAX_NUM; ++i)
	{
		if (this->m_pProto->aLimits[i].bType == LIMIT_CHAMPION)
		{
			return this->m_pProto->aLimits[i].lValue;
		}
	}
	return 0;
}

bool CItem::OnAfterCreatedItem()
{
#ifdef ENABLE_AFFECT_RENEWAL
	if (IsBlendItem())
	{
		if (GetSocket(3) == 1)
		{
			Lock(true);
			StartBlendExpireEvent();
		}
	}
#endif

	if (GetType() == ITEM_TOGGLE && this->GetOwner() && this->GetOwner()->IsPC())
	{
		OnLoadItem(this->GetOwner(), this);
	}
	
	if (-1 != this->GetProto()->cLimitRealTimeFirstUseIndex)
	{
		if (0 != GetSocket(1))
		{
			StartRealTimeExpireEvent();
		}
	}

	return true;
}

bool CItem::IsDragonSoul()
{
	return GetType() == ITEM_DS;
}

int CItem::GiveMoreTime_Per(float fPercent)
{
	if (IsDragonSoul())
	{
		DWORD duration = DSManager::instance().GetDuration(this);
		DWORD remain_sec = GetSocket(ITEM_SOCKET_REMAIN_SEC);
		DWORD given_time = fPercent * duration / 100u;
		if (remain_sec == duration)
			return false;
		if ((given_time + remain_sec) >= duration)
		{
			SetSocket(ITEM_SOCKET_REMAIN_SEC, duration);
			return duration - remain_sec;
		}
		else
		{
			SetSocket(ITEM_SOCKET_REMAIN_SEC, given_time + remain_sec);
			return given_time;
		}
	}
	// �켱 ��ȥ���� ���ؼ��� �ϵ��� �Ѵ�.
	else
		return 0;
}

int CItem::GiveMoreTime_Fix(DWORD dwTime)
{
	if (IsDragonSoul())
	{
		DWORD duration = DSManager::instance().GetDuration(this);
		DWORD remain_sec = GetSocket(ITEM_SOCKET_REMAIN_SEC);
		if (remain_sec == duration)
			return false;
		if ((dwTime + remain_sec) >= duration)
		{
			SetSocket(ITEM_SOCKET_REMAIN_SEC, duration);
			return duration - remain_sec;
		}
		else
		{
			SetSocket(ITEM_SOCKET_REMAIN_SEC, dwTime + remain_sec);
			return dwTime;
		}
	}
	// �켱 ��ȥ���� ���ؼ��� �ϵ��� �Ѵ�.
	else
		return 0;
}


int	CItem::GetDuration()
{
	if(!GetProto())
		return -1;

	for (int i=0 ; i < ITEM_LIMIT_MAX_NUM ; i++)
	{
		if (LIMIT_REAL_TIME == GetProto()->aLimits[i].bType)
			return GetProto()->aLimits[i].lValue;
	}

	if (GetProto()->cLimitTimerBasedOnWearIndex >= 0)
	{
		BYTE cLTBOWI = GetProto()->cLimitTimerBasedOnWearIndex;
		return GetProto()->aLimits[cLTBOWI].lValue;
	}

	return -1;
}

bool CItem::IsSameSpecialGroup(const LPITEM item) const
{
	// ���� VNUM�� ���ٸ� ���� �׷��� ������ ����
	if (this->GetVnum() == item->GetVnum())
		return true;

	if (GetSpecialGroup() && (item->GetSpecialGroup() == GetSpecialGroup()))
		return true;

	return false;
}

#ifdef __AURA_SYSTEM__
bool CItem::IsAuraBoosterForSocket()
{
	if (m_pProto->bType == ITEM_COSTUME && m_pProto->bSubType == COSTUME_AURA)
		return this->GetSocket(ITEM_SOCKET_AURA_BOOST) == 0;

	return false;
}

EVENTFUNC(aura_boost_socket_expire_event)
{
	item_event_info* info = dynamic_cast<item_event_info*>(event->info);
	if (info == NULL)
	{
		sys_err("aura_boost_socket_expire_event> <Factor> Null pointer");
		return 0;
	}

	LPITEM pkAura = info->item;
	const long c_lBoosterSocket = pkAura->GetSocket(ITEM_SOCKET_AURA_BOOST);
	const long c_lBoosterIndex = c_lBoosterSocket / 100000000;
	if (c_lBoosterIndex <= ITEM_AURA_BOOST_ERASER || c_lBoosterIndex >= ITEM_AURA_BOOST_MAX)
		return 0;

	DWORD dwBoosterItemVnum = ITEM_AURA_BOOST_ITEM_VNUM_BASE + c_lBoosterIndex;
	TItemTable* pBoosterProto = ITEM_MANAGER::instance().GetTable(dwBoosterItemVnum);
	if (!pBoosterProto)
		return 0;

	if (pBoosterProto->alValues[ITEM_AURA_BOOST_UNLIMITED_VALUE] == 1)
		return 0;

	long lBoostTimeSec = (c_lBoosterSocket % 100000000) - processing_time / passes_per_sec;
	if (lBoostTimeSec <= 0)
	{
		sys_log(0, "AURA BOOST EXPIRED : expired %s %lu", pkAura->GetName(), pkAura->GetID());
		pkAura->SetAuraBoosterSocketExpireEvent(NULL);
		pkAura->ModifyPoints(false);
		pkAura->SetSocket(ITEM_SOCKET_AURA_BOOST, 0);
		pkAura->ModifyPoints(true);
		if (pkAura->GetOwner())
		{
			pkAura->GetOwner()->ComputeBattlePoints();
			pkAura->GetOwner()->UpdatePacket();
		}

		LogManager::instance().ItemLog(pkAura->GetOwner(), pkAura, "AURA_BOOST_EXPIRED", pkAura->GetName());

		return 0;
	}

	pkAura->SetSocket(ITEM_SOCKET_AURA_BOOST, c_lBoosterIndex * 100000000 + lBoostTimeSec);
	return PASSES_PER_SEC(MIN(60, lBoostTimeSec));
}

void CItem::StartAuraBoosterSocketExpireEvent()
{
	if (!(m_pProto->bType == ITEM_COSTUME && m_pProto->bSubType == COSTUME_AURA))
		return;

	if (GetSocket(ITEM_SOCKET_AURA_BOOST) == 0)
		return;

	if (m_pkAuraBoostSocketExpireEvent)
		return;

	const long c_lBoosterSocket = GetSocket(ITEM_SOCKET_AURA_BOOST);
	const long c_lBoosterIndex = c_lBoosterSocket / 100000000;
	if (c_lBoosterIndex <= ITEM_AURA_BOOST_ERASER || c_lBoosterIndex >= ITEM_AURA_BOOST_MAX)
		return;

	DWORD dwBoosterItemVnum = ITEM_AURA_BOOST_ITEM_VNUM_BASE + c_lBoosterIndex;
	TItemTable* pBoosterProto = ITEM_MANAGER::instance().GetTable(dwBoosterItemVnum);
	if (!pBoosterProto)
		return;

	if (pBoosterProto->alValues[ITEM_AURA_BOOST_UNLIMITED_VALUE] == 1)
		return;

	long lBoostTimeSec = c_lBoosterSocket % 100000000;
	if (0 != lBoostTimeSec)
	{
		lBoostTimeSec %= 60;
		if (0 == lBoostTimeSec)
			lBoostTimeSec = 60;
	}

	item_event_info* info = AllocEventInfo<item_event_info>();
	info->item = this;

	SetAuraBoosterSocketExpireEvent(event_create(aura_boost_socket_expire_event, info, PASSES_PER_SEC(lBoostTimeSec)));
}

void CItem::StopAuraBoosterSocketExpireEvent()
{
	if (!m_pkAuraBoostSocketExpireEvent)
		return;

	const long c_lBoosterSocket = GetSocket(ITEM_SOCKET_AURA_BOOST);
	const long c_lBoosterIndex = c_lBoosterSocket / 100000000;
	if (c_lBoosterIndex <= ITEM_AURA_BOOST_ERASER || c_lBoosterIndex >= ITEM_AURA_BOOST_MAX)
		return;

	DWORD dwBoosterItemVnum = ITEM_AURA_BOOST_ITEM_VNUM_BASE + c_lBoosterIndex;
	TItemTable* pBoosterProto = ITEM_MANAGER::instance().GetTable(dwBoosterItemVnum);
	if (!pBoosterProto)
		return;

	long lBoostTimeSec = (c_lBoosterSocket % 100000000) - event_processing_time(m_pkAuraBoostSocketExpireEvent) / passes_per_sec;
	SetSocket(ITEM_SOCKET_AURA_BOOST, c_lBoosterIndex * 100000000 + lBoostTimeSec);
	event_cancel(&m_pkAuraBoostSocketExpireEvent);

	ITEM_MANAGER::instance().SaveSingleItem(this);
}

void CItem::SetAuraBoosterSocketExpireEvent(LPEVENT pkEvent)
{
	m_pkAuraBoostSocketExpireEvent = pkEvent;
}
#endif

const char* CItem::GetName()
{
    return m_pProto ? LC_LOCALE_ITEM_TEXT(GetVnum(), LOCALE_DEFAULT) : NULL;
}



#ifdef ENABLE_CUSTOM_INVENTORY
bool CItem::IsCustomCategory(BYTE bCategory) const
{
    DWORD dwCategory_0_Items[] = {
		70800, 71178, 71203, 71205, 71207, 71209, 71211, 71213, 71215, 71217, 55034, 55035, 55036, 55037, 55038, 55003, 55004, 55005, 55010, 55011 ,55012, 55013, 55014, 55015, 55016, 55017, 55018, 55019, 55020, 55021, 55022, 55023, 55024, 55025, 55026, 55027, 50301, 50302, 50303	 
    };

    DWORD dwCategory_2_Items[] = { 25040, 27987, 28982, 28980, 28985, 28986, 28981, 28983, 28984, 28987, 70602, 70603,
	
	// 27 July
	28995, 28996
	// 27 July
	
	};

    DWORD dwCategory_3_Items[] = {
        54705, 54702, 54703, 30270, 
    };
	
    DWORD dwCategory_3_UnallowedItems[] = { 70600, 70800, 71178, 71203, 71205, 71207, 71209, 71211, 71213, 71215, 71217, 49377 };
    
    DWORD dwCategory_4_Items[] = {
		70102, 72001, 72002, 72003, 71015, 72049, 72050, 70005, 39002, 72303, 79507, 38058, 71181, 71180, 72004, 72005, 72006, 76037, 79508, 70043, 71016,
		50815, 50816, 50817, 50818, 50819, 50820
    };
	
    DWORD dwCategory_4_UnallowedItems[] = {
		50301, 50302, 50303, 70038, 79900, 50289, 72326, 72341, 79998, 79999, 72723, 72727, 71300, 80014, 80015, 80016, 80017, 79507, 79508,
		79700, 79701, 79702, 79703, 79704, 79705, 27987, 72321
    };
	
	DWORD dwCategory_5_Items[] = { 90000, 70063, 70064 };
	
    if(bCategory == 0)
    {
        if(GetType() == ITEM_SKILLBOOK)
            return true;
        
        for (int j = 0; j < sizeof(dwCategory_0_Items) / sizeof(dwCategory_0_Items[0]); j++)
        {
            if(dwCategory_0_Items[j] == GetVnum())
                return true;
        }
    }
    else if(bCategory == 1)
    {
        if(GetType() == ITEM_METIN && GetSubType() == METIN_NORMAL)
            return true;
    }
    else if(bCategory == 2)
    {
        for (int j = 0; j < sizeof(dwCategory_2_Items) / sizeof(dwCategory_2_Items[0]); j++)
        {
            if(dwCategory_2_Items[j] == GetVnum())
                return true;
        }

        if(GetType() == ITEM_MATERIAL && GetSubType() == MATERIAL_LEATHER)
            return true;
    }
    else if(bCategory == 3)
    {
        for (int j = 0; j < sizeof(dwCategory_3_UnallowedItems) / sizeof(dwCategory_3_UnallowedItems[0]); j++)
        {
            if(dwCategory_3_UnallowedItems[j] == GetVnum())
                return false;
        }	
		
		if(GetType() == ITEM_GIFTBOX)
			return true;	
		
		// if(GetType() == ITEM_TREASURE_BOX || GetType() == ITEM_TREASURE_KEY)
			// return true;	
		
        for (int j = 0; j < sizeof(dwCategory_3_Items) / sizeof(dwCategory_3_Items[0]); j++)
        {
            if(dwCategory_3_Items[j] == GetVnum())
                return true;
        }
    }	
    else if(bCategory == 4)
    {
        for (int j = 0; j < sizeof(dwCategory_4_UnallowedItems) / sizeof(dwCategory_4_UnallowedItems[0]); j++)
        {
            if(dwCategory_4_UnallowedItems[j] == GetVnum())
                return false;
        }	
			
        if(GetType() == ITEM_USE && (GetSubType() == USE_CHANGE_ATTRIBUTE || GetSubType() == USE_ADD_ATTRIBUTE))
            return true;
		
        if(GetType() == ITEM_USE && (GetSubType() == USE_ADD_ATTRIBUTE2))
            return true;	

		if(GetType() == ITEM_USE && GetSubType() == USE_CLEAN_SOCKET)
			return true;

		if(GetType() == ITEM_POLYMORPH)
			return true;

		if(GetType() == ITEM_USE && GetSubType() == USE_AFFECT)
			return true;

		if(GetType() == ITEM_USE && GetSubType() == USE_SPECIAL) // Auto Potions
			return true;
		
        for (int j = 0; j < sizeof(dwCategory_4_Items) / sizeof(dwCategory_4_Items[0]); j++)
        {
            if(dwCategory_4_Items[j] == GetVnum())
			{
				if (71083 == GetVnum())
					continue;
				
                return true;
			}
        }		
    }
	
	else if(bCategory == 5)
	{
		if(GetType() == ITEM_COSTUME && GetSubType() == COSTUME_BODY)
			return true;
		
		if(GetType() == ITEM_COSTUME && GetSubType() == COSTUME_HAIR)
			return true;

		if(GetType() == ITEM_COSTUME && GetSubType() == COSTUME_SASH)
			return true;

// #ifdef ENABLE_PENDANT
		// if(GetType() == ITEM_ARMOR && GetSubType() == ARMOR_PENDANT)
			// return true;
// #endif	

// #ifdef __WEAPON_COSTUME_SYSTEM__
		// if(GetType() == ITEM_COSTUME && GetSubType() == COSTUME_WEAPON)
			// return true;
// #endif

		// for (int j = 0; j < sizeof(dwCategory_5_Items) / sizeof(dwCategory_5_Items[0]); j++)
		// {
			// if(dwCategory_5_Items[j] == GetVnum())
				// return true;
		// }	
	}	
    
    return false;
}
int CItem::GetItemCategory() const
{
    for(int i = 0; i < CUSTOM_INVENTORY_CATEGORY_NUM; i++)
    {
        if(IsCustomCategory(i))
            return i;
    }

    return -1;
}
#endif



#if defined(__ATTR_6TH_7TH__)
DWORD CItem::GetAttr67MaterialVnum()
{
	// NOTE: Webzen has made this value return from the item_proto.
	// With a new column "67AttrMaterial" that indicates the VNUM of the fragment.
	// Instead of adding a new column to the item_proto and to avoid compatibility
	// issues, simply hard coding the VNUM of the fragment should be enough.

	//return m_pProto->dwAttr67MaterialVnum;

#if defined(__PENDANT_SYSTEM__)
	if (GetType() == ITEM_ARMOR && GetSubType() == ARMOR_PENDANT)
		return 39079; // ��u������A ��i��o��A��i ( 105 )
#endif

	return CItemVnumHelper::GetAttr67MaterialVnumByLevel(GetLevelLimit());
}
#endif

const TItemLimit* CItem::FindLimit(uint8_t type) const
{
	for (const auto& limit : m_pProto->aLimits) {
		if (limit.bType == type)
			return &limit;
	}

	return nullptr;
}

int CItem::GetWindowInventoryEx()
{
	if (IsDragonSoul())
		return DRAGON_SOUL_INVENTORY;

	return INVENTORY;
}