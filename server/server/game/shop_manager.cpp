#include "stdafx.h"
#include "../libgame/grid.h"
#include "constants.h"
#include "utils.h"
#include "config.h"
#include "shop.h"
#include "desc.h"
#include "desc_manager.h"
#include "char.h"
#include "char_manager.h"
#include "item.h"
#include "item_manager.h"
#include "buffer_manager.h"
#include "packet.h"
#include "log.h"
#include "db.h"
#include "questmanager.h"
#include "monarch.h"
#include "mob_manager.h"
#include "locale_service.h"
#include "desc_client.h"
#include "shop_manager.h"
#include "group_text_parse_tree.h"
#include "shopEx.h"
#include <boost/algorithm/string/predicate.hpp>
#include "shop_manager.h"
#include "BattlePassManager.h"
#include <cctype>

CShopManager::CShopManager()
{
}

CShopManager::~CShopManager()
{
	Destroy();
}

bool CShopManager::Initialize(TShopTable * table, int size)
{
	if (!m_map_pkShop.empty())
		return false;

	int i;

	for (i = 0; i < size; ++i, ++table)
	{
		LPSHOP shop = M2_NEW CShop;

		if (!shop->Create(table->dwVnum, table->dwNPCVnum, table->items))
		{
			M2_DELETE(shop);
			continue;
		}

		m_map_pkShop.insert(TShopMap::value_type(table->dwVnum, shop));
		m_map_pkShopByNPCVnum.insert(TShopMap::value_type(table->dwNPCVnum, shop));
	}
	char szShopTableExFileName[256];

	snprintf(szShopTableExFileName, sizeof(szShopTableExFileName),
		"%s/shop_table_ex.txt", LocaleService_GetBasePath().c_str());

#if defined(ENABLE_RENEWAL_SHOPEX)
	return true;
#else
	return ReadShopTableEx(szShopTableExFileName);
#endif
}

void CShopManager::Destroy()
{
#if defined(ENABLE_RENEWAL_SHOPEX)
	for (TShopMap::iterator it = m_map_pkShopByNPCVnum.begin(); it != m_map_pkShopByNPCVnum.end(); ++it)
		delete it->second;
	m_map_pkShopByNPCVnum.clear();
#else
	TShopMap::iterator it = m_map_pkShop.begin();

	while (it != m_map_pkShop.end())
	{
		M2_DELETE(it->second);
		++it;
	}

	m_map_pkShop.clear();
#endif
}

LPSHOP CShopManager::Get(DWORD dwVnum)
{
	TShopMap::const_iterator it = m_map_pkShop.find(dwVnum);

	if (it == m_map_pkShop.end())
		return NULL;

	return (it->second);
}

LPSHOP CShopManager::GetByNPCVnum(DWORD dwVnum)
{
	TShopMap::const_iterator it = m_map_pkShopByNPCVnum.find(dwVnum);

	if (it == m_map_pkShopByNPCVnum.end())
		return NULL;

	return (it->second);
}

/*
 * �������̽� �Լ���
 */

// ���� �ŷ��� ����
bool CShopManager::StartShopping(LPCHARACTER pkChr, LPCHARACTER pkChrShopKeeper, int iShopVnum)
{
	if (pkChr->GetShopOwner() == pkChrShopKeeper)
		return false;
	// this method is only for NPC
	if (pkChrShopKeeper->IsPC())
		return false;

	//PREVENT_TRADE_WINDOW
	if (pkChr->IsOpenSafebox() || pkChr->GetExchange() || pkChr->GetMyShop() || pkChr->IsCubeOpen() || pkChr->IsAuraRefineWindowOpen())
	{
		pkChr->ChatPacket(CHAT_TYPE_INFO, "[LS;876]");
		return false;
	}
	//END_PREVENT_TRADE_WINDOW

	long distance = DISTANCE_APPROX(pkChr->GetX() - pkChrShopKeeper->GetX(), pkChr->GetY() - pkChrShopKeeper->GetY());

	if (distance >= SHOP_MAX_DISTANCE)
	{
		sys_log(1, "SHOP: TOO_FAR: %s distance %d", pkChr->GetName(), distance);
		return false;
	}

	LPSHOP pkShop;

	if (iShopVnum)
		pkShop = Get(iShopVnum);
	else
		pkShop = GetByNPCVnum(pkChrShopKeeper->GetRaceNum());

	if (!pkShop)
	{
		sys_log(1, "SHOP: NO SHOP");
		return false;
	}

	bool bOtherEmpire = false;

	if (pkChr->GetEmpire() != pkChrShopKeeper->GetEmpire())
		bOtherEmpire = true;

	pkShop->AddGuest(pkChr, pkChrShopKeeper->GetVID(), bOtherEmpire);
	pkChr->SetShopOwner(pkChrShopKeeper);
	sys_log(0, "SHOP: START: %s", pkChr->GetName());
	return true;
}

LPSHOP CShopManager::FindPCShop(DWORD dwVID)
{
	TShopMap::iterator it = m_map_pkShopByPC.find(dwVID);

	if (it == m_map_pkShopByPC.end())
		return NULL;

	return it->second;
}

LPSHOP CShopManager::CreatePCShop(LPCHARACTER ch, TShopItemTable * pTable, BYTE bItemCount)
{
	if (FindPCShop(ch->GetVID()))
		return NULL;

	LPSHOP pkShop = M2_NEW CShop;
	pkShop->SetPCShop(ch);
	pkShop->SetShopItems(pTable, bItemCount);

	m_map_pkShopByPC.insert(TShopMap::value_type(ch->GetVID(), pkShop));
	return pkShop;
}

#include <boost/algorithm/string/replace.hpp>
#ifdef OFFLINE_SHOP
extern std::map<int, TShopCost> g_ShopCosts;
bool CShopManager::CreateOfflineShop(LPCHARACTER owner, const char *szSign, const std::vector<TShopItemTable *> pTable, DWORD id)
{
	if (g_ShopCosts.find(id) == g_ShopCosts.end())
	{
		sys_log(0, "CreateOfflineShop:Shop days error %d", id);
		return false;
	}

	DWORD map_index = owner->GetMapIndex();
	long x = (owner->GetX());
	long y = (owner->GetY());
	long z = owner->GetZ();
	char szQuery[4096]={};
	std::string stSign(szSign);
	boost::replace_all(stSign, "%", "%%");
	char szName[SHOP_SIGN_MAX_LEN * 2+1]={0};
	DBManager::instance().EscapeString(szName, sizeof (szName), stSign.c_str(), stSign.length());
	DWORD date_close = get_global_time() + (g_ShopCosts[id].time * g_ShopCosts[id].days);
	std::unique_ptr<SQLMsg> pkMsg(DBManager::instance().DirectQuery("insert into player_shop set item_count=%d,player_id=%d,name='%s',map_index=%d,x=%ld,y=%ld,z=%ld,ip='%s',date=NOW(),date_close=FROM_UNIXTIME(%d),channel=%d",
		pTable.size(), owner->GetPlayerID(), szName, map_index, x, y, z, inet_ntoa(owner->GetDesc()->GetAddr().sin_addr), date_close, g_bChannel));
	SQLResult * pRes = pkMsg->Get();
	stSign.clear();
	memset(&szName, 0, sizeof(szName));
	memset(&szQuery, 0, sizeof(szQuery));
	DWORD shop_id = pRes->uiInsertID;
	if (shop_id>0)
	{
		for (int i = 0; i < pTable.size(); i++)
		{
			LPITEM item = owner->GetItem(pTable[i]->pos);
			if (item)
			{

				char query[1024];
				sprintf(query, "INSERT INTO player_shop_items SET");
				sprintf(query, "%s player_id='%d'", query, owner->GetPlayerID());
				sprintf(query, "%s, date_add=NOW()", query);
				sprintf(query, "%s, shop_id='%d'", query, shop_id);
				sprintf(query, "%s, vnum='%d'", query, item->GetVnum());
				sprintf(query, "%s, count='%d'", query, item->GetCount());
#ifdef __CHANGELOOK_SYSTEM__
				sprintf(query, "%s, look='%d'", query, item->GetTransmutation());
#endif
				sprintf(query, "%s, price='%lld'", query, pTable[i]->price);
				sprintf(query, "%s, display_pos='%u'", query, pTable[i]->display_pos);
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
					else {
						sprintf(query, "%s, applytype%d='%u'", query, ia - 7, attr.bType);
						sprintf(query, "%s, applyvalue%d='%d'", query, ia - 7, attr.sValue);
					}


				}
				std::unique_ptr<SQLMsg> (DBManager::instance().DirectQuery(query));
				memset(&query, 0, sizeof(query));
				ITEM_MANAGER::Instance().RemoveItem(item, "Priv shop");
			}
		}
		StartOfflineShop(shop_id);
		owner->SendShops();
		return true;
	}
	return false;
}

bool CShopManager::StartOfflineShop(DWORD id, bool onboot)
{
#ifdef ENABLE_SHOP_SEARCH
	std::string player_name;
#endif
	std::string name;
	std::string shop_name(LC_TEXT("SHOP_NAME"));
	DWORD pid, time;
	long map_index, x, y, z;
	std::unique_ptr<SQLMsg> pkMsg(DBManager::instance().DirectQuery("SELECT player_shop.name,player_shop.player_id,player.name as player_name,player_shop.map_index,player_shop.x,player_shop.y,player_shop.z,UNIX_TIMESTAMP(player_shop.date_close),player_shop.id from player_shop left join player on player.id=player_shop.player_id where player_shop.id='%d'", id));
	SQLResult * pRes = pkMsg->Get();
	if (pRes->uiNumRows>0)
	{
		MYSQL_ROW row;
		while ((row = mysql_fetch_row(pRes->pSQLResult)) != NULL)
		{
			name = row[0];
			str_to_number(pid, row[1]);
			boost::replace_all(shop_name, "#PLAYER_NAME#", row[2]);
#ifdef ENABLE_SHOP_SEARCH
			player_name=row[2];
#endif
			str_to_number(map_index, row[3]);
			str_to_number(x, row[4]);
			str_to_number(y, row[5]);
			str_to_number(z, row[6]);
			str_to_number(time, row[7]);
		}
	}
	if (map_index <= 0 || x <= 0 || y <= 0)
	{
		sys_err("location is null %d", id);
		return false;
	}

	LPCHARACTER ch = CHARACTER_MANAGER::Instance().SpawnMob(30000, map_index, x, y, z, true, 0, false);
	if (ch)
	{
		ch->SetName(shop_name.c_str());
		ch->SetPrivShop(id);
		ch->SetPrivShopOwner(pid);
		ch->SetShopTime(time);
		ch->Show(map_index, x, y, z);
#ifdef ENABLE_SHOP_SEARCH
		ch->SetShopOwnerName(player_name);
#endif
		ch->OpenShop(id, name.c_str(), onboot);
		ch->StartShopEditModeEvent();
		std::unique_ptr<SQLMsg> (DBManager::instance().DirectQuery("UPDATE player_shop SET shop_vid=%d WHERE id=%d", (DWORD)ch->GetVID(), id));
		LPCHARACTER owner = CHARACTER_MANAGER::instance().FindByPID(ch->GetPrivShopOwner());
		if (owner)
			owner->LoadPrivShops();
		return true;
	}
	return false;
}

LPSHOP CShopManager::CreateNPCShop(LPCHARACTER ch, std::vector<TShopItemTable *> map_shop)
{
	if (FindPCShop(ch->GetPrivShop()))
		return NULL;

	LPSHOP pkShop = M2_NEW CShop;
	pkShop->SetPCShop(ch);
	pkShop->SetPrivShopItems(map_shop);
	pkShop->SetLocked(false);
	m_map_pkShopByPC.insert(TShopMap::value_type(ch->GetPrivShop(), pkShop));
	return pkShop;
}

void CShopManager::DestroyPCShop(LPCHARACTER ch)
{
	LPSHOP pkShop = FindPCShop(ch->IsPrivShop() && ch->GetRaceNum() == 30000 ? ch->GetPrivShop() : ch->GetVID());

	if (!pkShop)
		return;

	//PREVENT_ITEM_COPY;
	ch->SetMyShopTime();
	//END_PREVENT_ITEM_COPY

	m_map_pkShopByPC.erase(ch->IsPrivShop() && ch->GetRaceNum() == 30000 ? ch->GetPrivShop() : ch->GetVID());
	M2_DELETE(pkShop);
}
#else
void CShopManager::DestroyPCShop(LPCHARACTER ch)
{
	LPSHOP pkShop = FindPCShop(ch->GetVID());

	if (!pkShop)
		return;

	//PREVENT_ITEM_COPY;
	ch->SetMyShopTime();
	//END_PREVENT_ITEM_COPY

	m_map_pkShopByPC.erase(ch->GetVID());
	M2_DELETE(pkShop);
}
#endif

// ���� �ŷ��� ����
void CShopManager::StopShopping(LPCHARACTER ch)
{
	LPSHOP shop;

	if (!(shop = ch->GetShop()))
		return;

	//PREVENT_ITEM_COPY;
	ch->SetMyShopTime();
	//END_PREVENT_ITEM_COPY

	shop->RemoveGuest(ch);
	sys_log(0, "SHOP: END: %s", ch->GetName());
}

// ������ ����
void CShopManager::Buy(LPCHARACTER ch, BYTE pos)
{
#ifdef ENABLE_NEWSTUFF
	if (0 != g_BuySellTimeLimitValue)
	{
		if (get_dword_time() < ch->GetLastBuySellTime()+g_BuySellTimeLimitValue)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot discard gold yet."));
			return;
		}
	}

	ch->SetLastBuySellTime(get_dword_time());
#endif
	if (!ch->GetShop())
		return;

	if (!ch->GetShopOwner())
		return;

	if (DISTANCE_APPROX(ch->GetX() - ch->GetShopOwner()->GetX(), ch->GetY() - ch->GetShopOwner()->GetY()) > 2000)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;877]");
		return;
	}
	
#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	if (ch->IsSecured())
	{
		ch->ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT("|cffd93c3c[Err]:|h|r_Acest_cont_este_securizat!"));
		return;
	}
#endif

	CShop* pkShop = ch->GetShop();

	if (!pkShop->IsPCShop())
	{
		//if (pkShop->GetVnum() == 0)
		//	return;
		//const CMob* pkMob = CMobManager::instance().Get(pkShop->GetNPCVnum());
		//if (!pkMob)
		//	return;

		//if (pkMob->m_table.bType != CHAR_TYPE_NPC)
		//{
		//	return;
		//}
	}
	else
	{
	}

	//PREVENT_ITEM_COPY
	ch->SetMyShopTime();
	//END_PREVENT_ITEM_COPY

	int ret = pkShop->Buy(ch, pos);

	if (SHOP_SUBHEADER_GC_OK != ret) // ������ �־����� ������.
	{
		TPacketGCShop pack;

		pack.header	= HEADER_GC_SHOP;
		pack.subheader	= ret;
		pack.size	= sizeof(TPacketGCShop);

		ch->GetDesc()->Packet(&pack, sizeof(pack));
	}
}

void CShopManager::Sell(LPCHARACTER ch, WORD wCell, WORD bCount, BYTE bType)
{
#ifdef ENABLE_NEWSTUFF
	if (0 != g_BuySellTimeLimitValue)
	{
		if (get_dword_time() < ch->GetLastBuySellTime()+g_BuySellTimeLimitValue)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot discard gold yet."));
			return;
		}
	}

	ch->SetLastBuySellTime(get_dword_time());
#endif
	if (!ch->GetShop())
		return;

	if (!ch->GetShopOwner())
		return;

	if (!ch->CanHandleItem())
		return;

	if (ch->GetShop()->IsPCShop())
		return;

	if (DISTANCE_APPROX(ch->GetX()-ch->GetShopOwner()->GetX(), ch->GetY()-ch->GetShopOwner()->GetY())>2000)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;877]");
		return;
	}
	
#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
	if (ch->IsSecured())
	{
		ch->ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT("|cffd93c3c[Err]:|h|r_Acest_cont_este_securizat!"));
		return;
	}
#endif

	LPITEM item = ch->GetInventoryItem(wCell);
	
// #ifdef ENABLE_CUSTOM_INVENTORY
	// if (false == bCell.IsDefaultInventoryPosition() && false == bCell.IsCustomInventoryPosition())
	// {
		// ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Pozitie invalida vanzare obiect."));
		// return;
	// }
	
	// LPITEM item = ch->GetItem(bCell);
// #else
	// LPITEM item = ch->GetInventoryItem(bCell);
// #endif

	if (!item)
		return;

	if (item->IsEquipped() == true)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1059]");
		return;
	}

	if (true == item->isLocked())
	{
		return;
	}

	if (IS_SET(item->GetAntiFlag(), ITEM_ANTIFLAG_SELL))
		return;

	long long dwPrice;

	if (bCount == 0 || bCount > item->GetCount())
		bCount = item->GetCount();

	dwPrice = item->GetShopBuyPrice();

	if (IS_SET(item->GetFlag(), ITEM_FLAG_COUNT_PER_1GOLD))
	{
		if (dwPrice == 0)
			dwPrice = bCount;
		else
			dwPrice = bCount / dwPrice;
	}
	else
		dwPrice *= bCount;

	dwPrice /= 5;

	//���� ���
	DWORD dwTax = 0;
	int iVal = 3;

	{
		dwTax = dwPrice * iVal/100;
		dwPrice -= dwTax;
	}

	if (test_server)
		sys_log(0, "Sell Item price id %d %s itemid %d", ch->GetPlayerID(), ch->GetName(), item->GetID());

	// const long long nTotalMoney = static_cast<long long>(ch->GetGold()) + static_cast<long long>(dwPrice);

	// if (GOLD_MAX_MAX <= nTotalMoney)
	// {
		// sys_err("[OVERFLOW_GOLD] id %u name %s gold %lld", ch->GetPlayerID(), ch->GetName(), ch->GetGold());
		// ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("20??? ???? ??? ?? ????."));
		// return;
	// }

	sys_log(0, "SHOP: SELL: %s item name: %s(x%d):%u price: %lld", ch->GetName(), item->GetName(), bCount, item->GetID(), dwPrice);


	if (iVal > 0)
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;881;%d]", iVal);

#ifdef ENABLE_BATTLE_PASS
	CBattlePass::Instance().RegisterItemMission(MISSION_TYPE_SELL, bCount, ch, item);
#endif
#ifdef ENABLE_REWARD_SYSTEM
	CHARACTER_MANAGER::Instance().DoReward(ch, REWARD_MISSION_SELL_ITEM, item->GetVnum(), bCount);
#endif

	DBManager::instance().SendMoneyLog(MONEY_LOG_SHOP, item->GetVnum(), dwPrice);
	if (bCount == item->GetCount())
		ITEM_MANAGER::instance().RemoveItem(item, "SELL");
	else
		item->SetCount(item->GetCount() - bCount);

	//���� �ý��� : ���� ¡��
	CMonarch::instance().SendtoDBAddMoney(dwTax, ch->GetEmpire(), ch);

#ifdef ENABLE_REMOVE_LIMIT_GOLD
    ch->ChangeGold(dwPrice);
#else
	ch->PointChange(POINT_GOLD, dwPrice, false);
#endif
}

bool CompareShopItemName(const SShopItemTable& lhs, const SShopItemTable& rhs)
{
	TItemTable* lItem = ITEM_MANAGER::instance().GetTable(lhs.vnum);
	TItemTable* rItem = ITEM_MANAGER::instance().GetTable(rhs.vnum);
	if (lItem && rItem)
		return strcmp(lItem->szLocaleName, rItem->szLocaleName) < 0;
	else
		return true;
}

bool ConvertToShopItemTable(IN CGroupNode* pNode, OUT TShopTableEx& shopTable)
{
	if (!pNode->GetValue("vnum", 0, shopTable.dwVnum))
	{
		sys_err("Group %s does not have vnum", pNode->GetNodeName().c_str());
		return false;
	}

	if (!pNode->GetValue("name", 0, shopTable.name))
	{
		sys_err("Group %s does not have name", pNode->GetNodeName().c_str());
		return false;
	}

	if (shopTable.name.length() >= SHOP_TAB_NAME_MAX)
	{
		sys_err("Shop name length must be less than %d. Error in Group %s, name %s", SHOP_TAB_NAME_MAX, pNode->GetNodeName().c_str(), shopTable.name.c_str());
		return false;
	}

	std::string stCoinType;
	if (!pNode->GetValue("cointype", 0, stCoinType))
	{
		stCoinType = "Gold";
	}

	if (boost::iequals(stCoinType, "Gold"))
	{
		shopTable.coinType = SHOP_COIN_TYPE_GOLD;
	}
	else if (boost::iequals(stCoinType, "SecondaryCoin"))
	{
		shopTable.coinType = SHOP_COIN_TYPE_SECONDARY_COIN;
	}
	else
	{
		sys_err("Group %s has undefine cointype(%s)", pNode->GetNodeName().c_str(), stCoinType.c_str());
		return false;
	}

	CGroupNode* pItemGroup = pNode->GetChildNode("items");
	if (!pItemGroup)
	{
		sys_err("Group %s does not have 'group items'", pNode->GetNodeName().c_str());
		return false;
	}

	int itemGroupSize = pItemGroup->GetRowCount();
	std::vector <TShopItemTable> shopItems(itemGroupSize);
	if (itemGroupSize >= SHOP_HOST_ITEM_MAX_NUM)
	{
		sys_err("count(%d) of rows of group items of group %s must be smaller than %d", itemGroupSize, pNode->GetNodeName().c_str(), SHOP_HOST_ITEM_MAX_NUM);
		return false;
	}

	for (int i = 0; i < itemGroupSize; i++)
	{
		if (!pItemGroup->GetValue(i, "vnum", shopItems[i].vnum))
		{
			sys_err("row(%d) of group items of group %s does not have vnum column", i, pNode->GetNodeName().c_str());
			return false;
		}

		if (!pItemGroup->GetValue(i, "count", shopItems[i].count))
		{
			sys_err("row(%d) of group items of group %s does not have count column", i, pNode->GetNodeName().c_str());
			return false;
		}
		if (!pItemGroup->GetValue(i, "price", shopItems[i].price))
		{
			sys_err("row(%d) of group items of group %s does not have price column", i, pNode->GetNodeName().c_str());
			return false;
		}
	}
	std::string stSort;
	if (!pNode->GetValue("sort", 0, stSort))
	{
		stSort = "None";
	}

	if (boost::iequals(stSort, "Asc"))
	{
		std::sort(shopItems.begin(), shopItems.end(), CompareShopItemName);
	}
	else if(boost::iequals(stSort, "Desc"))
	{
		std::sort(shopItems.rbegin(), shopItems.rend(), CompareShopItemName);
	}

	CGrid grid = CGrid(5, 9);
	int iPos;

	msl::refill(shopTable.items);

	for (size_t i = 0; i < shopItems.size(); i++)
	{
		TItemTable * item_table = ITEM_MANAGER::instance().GetTable(shopItems[i].vnum);
		if (!item_table)
		{
			sys_err("vnum(%d) of group items of group %s does not exist", shopItems[i].vnum, pNode->GetNodeName().c_str());
			return false;
		}

		iPos = grid.FindBlank(1, item_table->bSize);

		grid.Put(iPos, 1, item_table->bSize);
		shopTable.items[iPos] = shopItems[i];
	}

	shopTable.byItemCount = shopItems.size();
	return true;
}

#ifdef ENABLE_DISTANCE_SHOPPING
#include "distance_shopping.h"
bool CShopManager::StartDistanceShopping(LPCHARACTER pkChr, int iShopVnum)
{	
	if (!iShopVnum)
		return false;
	
	if (!CDistanceShopping::instance().CanUseDistanceShopping(pkChr))
	{
		pkChr->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("distance_shopping_excluded_map"));
		return false;
	}

	if (pkChr->IsOpenSafebox() || pkChr->GetExchange() || pkChr->GetMyShop() || pkChr->IsCubeOpen())
	{
		pkChr->ChatPacket(CHAT_TYPE_INFO, "[LS;876]");
		return false;
	}
	
	if (pkChr->GetShop())
	{
		StopDistanceShopping(pkChr);
		return false;
	}

	LPSHOP pkShop;
	pkShop = Get(iShopVnum);

	if (!pkShop)
	{
		sys_log(1, "DISTANCE SHOP: NO SHOP");
		return false;
	}

	pkShop->AddGuest(pkChr, pkChr->GetVID(), false, true);
	pkChr->SetShopOwner(pkChr);
	sys_log(0, "DISTANCE SHOP: START: %s", pkChr->GetName());
	return true;
}

void CShopManager::StopDistanceShopping(LPCHARACTER ch)
{
	if (!ch->GetShop())
		return;
	
	LPSHOP pkShop;
	pkShop = ch->GetShop();

	pkShop->RemoveGuest(ch);
	sys_log(0, "DISTANCE SHOP: END: %s", ch->GetName());
}
#endif

#if defined(ENABLE_RENEWAL_SHOPEX)
bool CShopManager::InitializeShopEX(TShopTable* table, int size)
{
	typedef std::multimap <DWORD, TShopTableEx> TMapNPCshop;
	TMapNPCshop map_npcShop;

	for (int i = 0; i < size; ++i, ++table)
	{
		TShopTableEx shopTable;
		shopTable.dwVnum = table->dwVnum;
		shopTable.name = table->szShopName;
		shopTable.coinType = SHOP_COIN_TYPE_GOLD;

		if (shopTable.name.length() >= SHOP_TAB_NAME_MAX) {
			sys_err("Shop name length must be less than %d. Error %s", SHOP_TAB_NAME_MAX, shopTable.name.c_str());
			return false;
		}

		CGrid grid(5, 9);

		memset(&shopTable.items[0], 0, sizeof(shopTable.items));

		for (size_t j = 0; j < table->byItemCount; j++)
		{
			TItemTable* item_table = ITEM_MANAGER::instance().GetTable(table->items[j].vnum);
			if (!item_table) {
				sys_err("vnum(%d) of group items of group %s does not exist.", table->items[j].vnum, shopTable.name.c_str());
				return false;
			}

			int iPos = grid.FindBlank(1, item_table->bSize);
			if (iPos == -1) {
				sys_err("vnum(%d) of group items of group %s there is no space!", table->items[j].vnum, shopTable.name.c_str());
				return false;
			}

			grid.Put(iPos, 1, item_table->bSize);
			shopTable.items[iPos] = table->items[j];
			shopTable.byItemCount++;
		}
		if (m_map_pkShopByNPCVnum.find(table->dwNPCVnum) != m_map_pkShopByNPCVnum.end()) {
			sys_err("NPCVNUM(%d) already used.", table->dwNPCVnum);
			return false;
		}

		map_npcShop.insert(TMapNPCshop::value_type(table->dwNPCVnum, shopTable));
	}

	for (TMapNPCshop::iterator it = map_npcShop.begin(); it != map_npcShop.end(); ++it)
	{
		const DWORD npcVnum = it->first;
		TShopTableEx& table = it->second;

		if (m_map_pkShop.find(table.dwVnum) != m_map_pkShop.end()) {
			sys_err("Shop vnum(%d) already exists.", table.dwVnum);
			return false;
		}

		TShopMap::const_iterator shop_it = m_map_pkShopByNPCVnum.find(npcVnum);
		LPSHOPEX pkShopEx = NULL;

		if (m_map_pkShopByNPCVnum.end() == shop_it) {
			pkShopEx = new CShopEx;
			pkShopEx->Create(0, npcVnum);
			m_map_pkShopByNPCVnum.insert(TShopMap::value_type(npcVnum, pkShopEx));
		}
		else {
			pkShopEx = dynamic_cast <CShopEx*> (shop_it->second);
			if (!pkShopEx) {
				sys_err("NPC(%d) Shop is not extended version.", shop_it->first);
				return false;
			}
		}

		if (pkShopEx->GetTabCount() >= SHOP_TAB_COUNT_MAX) {
			sys_err("ShopEx cannot have tab more than %d.", SHOP_TAB_COUNT_MAX);
			delete pkShopEx;
			return false;
		}

		pkShopEx->AddShopTable(table);
		m_map_pkShop.insert(TShopMap::value_type(table.dwVnum, pkShopEx));
	}

	return true;
}
#endif

bool CShopManager::ReadShopTableEx(const char* stFileName)
{
	// file ���� üũ.
	// ���� ���� ������ ó������ �ʴ´�.
	FILE* fp = fopen(stFileName, "rb");
	if (NULL == fp)
		return true;
	fclose(fp);

	CGroupTextParseTreeLoader loader;
	if (!loader.Load(stFileName))
	{
		sys_err("%s Fail", stFileName);
		return false;
	}

	CGroupNode* pShopNPCGroup = loader.GetGroup("shopnpc");
	if (NULL == pShopNPCGroup)
	{
		sys_err("Group ShopNPC is not exist");
		return false;
	}

	typedef std::multimap <DWORD, TShopTableEx> TMapNPCshop;
	TMapNPCshop map_npcShop;
	for (int i = 0; i < pShopNPCGroup->GetRowCount(); i++)
	{
		DWORD npcVnum;
		std::string shopName;
		if (!pShopNPCGroup->GetValue(i, "npc", npcVnum) || !pShopNPCGroup->GetValue(i, "group", shopName))
		{
			sys_err("Invalid row(%d), group ShopNPC rows must have 'npc', 'group' columns", i);
			return false;
		}
		std::transform(shopName.begin(), shopName.end(), shopName.begin(), (int(*)(int))std::tolower);
		CGroupNode* pShopGroup = loader.GetGroup(shopName.c_str());
		if (!pShopGroup)
		{
			sys_err("Group %s is not exist", shopName.c_str());
			return false;
		}
		TShopTableEx table;
		if (!ConvertToShopItemTable(pShopGroup, table))
		{
			sys_err("Cannot read Group %s", shopName.c_str());
			return false;
		}
		if (m_map_pkShopByNPCVnum.find(npcVnum) != m_map_pkShopByNPCVnum.end())
		{
			sys_err("%d cannot have both original shop and extended shop", npcVnum);
			return false;
		}

		map_npcShop.insert(TMapNPCshop::value_type(npcVnum, table));
	}

	for (TMapNPCshop::iterator it = map_npcShop.begin(); it != map_npcShop.end(); ++it)
	{
		DWORD npcVnum = it->first;
		TShopTableEx& table = it->second;
		if (m_map_pkShop.find(table.dwVnum) != m_map_pkShop.end())
		{
			sys_err("Shop vnum(%d) already exists", table.dwVnum);
			return false;
		}
		TShopMap::iterator shop_it = m_map_pkShopByNPCVnum.find(npcVnum);

		LPSHOPEX pkShopEx = NULL;
		if (m_map_pkShopByNPCVnum.end() == shop_it)
		{
			pkShopEx = M2_NEW CShopEx;
			pkShopEx->Create(0, npcVnum);
			m_map_pkShopByNPCVnum.insert(TShopMap::value_type(npcVnum, pkShopEx));
		}
		else
		{
			pkShopEx = dynamic_cast <CShopEx*> (shop_it->second);
			if (NULL == pkShopEx)
			{
				sys_err("It can't be happend, NPC(%d) Shop is not extended version", shop_it->first);
				return false;
			}
		}

		if (pkShopEx->GetTabCount() >= SHOP_TAB_COUNT_MAX)
		{
			sys_err("ShopEx cannot have tab more than %d", SHOP_TAB_COUNT_MAX);
			return false;
		}

		if (pkShopEx->GetVnum() != 0 && m_map_pkShop.find(pkShopEx->GetVnum()) != m_map_pkShop.end())
		{
			sys_err("Shop vnum(%d) already exist", pkShopEx->GetVnum());
			return false;
		}
		m_map_pkShop.insert(TShopMap::value_type (pkShopEx->GetVnum(), pkShopEx));
		pkShopEx->AddShopTable(table);
	}

	return true;
}
