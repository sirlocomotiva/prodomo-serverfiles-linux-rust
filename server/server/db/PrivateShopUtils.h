#pragma once
#include "stdafx.h"
#include "ClientManager.h"
#include "Main.h"

static const char* GetPrivateShopQuery(DWORD dwOwner)
{
	static char szQuery[QUERY_MAX_LEN + 1]{};
	snprintf(szQuery, sizeof(szQuery),
		"SELECT owner_id, owner_name, state+0, title, title_type, vnum, x, y, map_index, channel, port, gold, cheque, page_count, premium_time, unlocked_slots "
		"FROM private_shop%s "
		"WHERE owner_id = %u ",
		GetTablePostfix(), dwOwner);

	return szQuery;
}

static const char* GetPrivateShopItemQuery(DWORD dwOwner)
{
	static char szQuery[QUERY_MAX_LEN] {};
	snprintf(szQuery, sizeof(szQuery),
		"SELECT id, pos, count, vnum, gold, cheque, checkin, "
#ifdef ENABLE_PRIVATE_SHOP_CHANGE_LOOK
        "transmutation, "
#endif
#ifdef ENABLE_PRIVATE_SHOP_REFINE_ELEMENT
        "refine_element, "
#endif
		"socket0, "
		"socket1, "
		"socket2, "
#ifdef ENABLE_PRIVATE_SHOP_SOCKET5
        "socket3, "
		"socket4, "
#endif
#ifdef ENABLE_PRIVATE_SHOP_APPLY_RANDOM
        "apply_type0, apply_value0, apply_path0, "
        "apply_type1, apply_value1, apply_path1, "
        "apply_type2, apply_value2, apply_path2, "
        "apply_type3, apply_value3, apply_path3, "
#endif
		"attrtype0, attrvalue0, "
		"attrtype1, attrvalue1, "
		"attrtype2, attrvalue2, "
		"attrtype3, attrvalue3, "
		"attrtype4, attrvalue4, "
		"attrtype5, attrvalue5, "
		"attrtype6, attrvalue6 "
        "FROM private_shop_item%s WHERE owner_id = %d",
		GetTablePostfix(), dwOwner);

	return szQuery;
}

static const char* GetPrivateShopItemSaleQuery(DWORD dwOwner)
{
	static char szQuery[QUERY_MAX_LEN + 1]{};
	snprintf(szQuery, sizeof(szQuery),
		"SELECT id,customer_id,customer_name,UNIX_TIMESTAMP(time),item_id,count,vnum,gold,cheque,checkin, "
#ifdef ENABLE_PRIVATE_SHOP_CHANGE_LOOK
        "transmutation, "
#endif
#ifdef ENABLE_PRIVATE_SHOP_REFINE_ELEMENT
        "refine_element, "
#endif
		"socket0,socket1,socket2, "
#ifdef ENABLE_PRIVATE_SHOP_SOCKET5
        "socket3, "
		"socket4, "
#endif
#ifdef ENABLE_PRIVATE_SHOP_APPLY_RANDOM
        "apply_type0, apply_value0, apply_path0, "
        "apply_type1, apply_value1, apply_path1, "
        "apply_type2, apply_value2, apply_path2, "
        "apply_type3, apply_value3, apply_path3, "
#endif
		"attrtype0,attrvalue0,attrtype1,attrvalue1,attrtype2,attrvalue2,attrtype3,attrvalue3,attrtype4,attrvalue4,attrtype5,attrvalue5,attrtype6,attrvalue6 "
		"FROM private_shop_sale_history%s WHERE owner_id=%d and DATEDIFF(FROM_UNIXTIME(%u), time) < %u",
		GetTablePostfix(), dwOwner, time(0), VALID_SALE_DAY_INTERVAL);

	return szQuery;
}

static bool CreatePrivateShopTableFromRes(MYSQL_RES* pRes, TPrivateShop& rTable)
{
	if (!pRes)
		return false;

	auto iRow = mysql_num_rows(pRes);
	if (iRow <= 0)
		return false;

	MYSQL_ROW row = mysql_fetch_row(pRes);
	int col = 0;

	str_to_number(rTable.dwOwner, row[col++]);
	strlcpy(rTable.szOwnerName, row[col++], sizeof(rTable.szOwnerName));
	str_to_number(rTable.bState, row[col++]);
	strlcpy(rTable.szTitle, row[col++], sizeof(rTable.szTitle));
	str_to_number(rTable.bTitleType, row[col++]);
	str_to_number(rTable.dwVnum, row[col++]);
	str_to_number(rTable.lX, row[col++]);
	str_to_number(rTable.lY, row[col++]);
	str_to_number(rTable.lMapIndex, row[col++]);
	str_to_number(rTable.bChannel, row[col++]);
	str_to_number(rTable.wPort, row[col++]);
	str_to_number(rTable.llGold, row[col++]);
	str_to_number(rTable.dwCheque, row[col++]);
	str_to_number(rTable.bPageCount, row[col++]);
	str_to_number(rTable.tPremiumTime, row[col++]);
	str_to_number(rTable.wUnlockedSlots, row[col++]);

	return true;
}

static bool CreatePrivateShopItemTableFromRes(MYSQL_RES* pRes, std::vector<TPlayerPrivateShopItem>* pVec, DWORD dwPID)
{
	if (!pRes)
	{
		pVec->clear();
		return false;
	}

	auto iRow = mysql_num_rows(pRes);
	if (iRow <= 0)
	{
		pVec->clear();
		return false;
	}

	pVec->resize(iRow);

	for (int i = 0; i < iRow; ++i)
	{
		MYSQL_ROW row = mysql_fetch_row(pRes);
		TPlayerPrivateShopItem& item = pVec->at(i);

		int cur = 0;

		str_to_number(item.dwID, row[cur++]);
		str_to_number(item.wPos, row[cur++]);
		str_to_number(item.dwCount, row[cur++]);
		str_to_number(item.dwVnum, row[cur++]);
		str_to_number(item.TPrice.llGold, row[cur++]);
		str_to_number(item.TPrice.dwCheque, row[cur++]);
		str_to_number(item.tCheckin, row[cur++]);
#ifdef ENABLE_PRIVATE_SHOP_CHANGE_LOOK
        str_to_number(item.dwTransmutationVnum, row[cur++]);
#endif
#ifdef ENABLE_PRIVATE_SHOP_REFINE_ELEMENT
        str_to_number(item.dwRefineElement, row[cur++]);
#endif
		str_to_number(item.alSockets[0], row[cur++]);
		str_to_number(item.alSockets[1], row[cur++]);
		str_to_number(item.alSockets[2], row[cur++]);
#ifdef ENABLE_PRIVATE_SHOP_SOCKET5
		str_to_number(item.alSockets[3], row[cur++]);
		str_to_number(item.alSockets[4], row[cur++]);
#endif

#ifdef ENABLE_PRIVATE_SHOP_APPLY_RANDOM
        for (int j = 0; j < ITEM_APPLY_MAX_NUM; j++)
        {
            str_to_number(item.aApplyRandom[j].bType, row[cur++]);
            str_to_number(item.aApplyRandom[j].sValue, row[cur++]);
            str_to_number(item.aApplyRandom[j].bPath, row[cur++]);
        }
#endif

		for (int j = 0; j < ITEM_ATTRIBUTE_MAX_NUM; j++)
		{
			str_to_number(item.aAttr[j].bType, row[cur++]);
			str_to_number(item.aAttr[j].sValue, row[cur++]);
		}

		item.dwOwner = dwPID;
#ifdef ENABLE_PET_GROWTH_SYSTEM
		auto vnum = item.dwVnum;
		if (vnum >= 55701 && vnum <= 55712)
		{
			uint32_t col = 0;

			std::string petColumns = fmt::format("id, name, pet_vnum, pet_level, pet_stage, pet_exp, hp, sp, def, pet_skill0, pet_skillLevel0, pet_skill1, pet_skillLevel1, pet_skill2, pet_skillLevel2, pet_skill3, pet_skillLevel3, pet_skill4, pet_skillLevel4");
			std::string petQuery = fmt::format("SELECT {} FROM pet_cache{} WHERE id = {}", petColumns, GetTablePostfix(), item.alSockets[2]);

			std::unique_ptr<SQLMsg> msgGrowthPet(CDBManager::Instance().DirectQuery(petQuery.c_str()));
			const SQLResult* resGrowthPet = msgGrowthPet->Get();
			if (resGrowthPet && resGrowthPet->uiNumRows > 0)
			{
				for (size_t k = 0; k < resGrowthPet->uiNumRows; ++k)
				{
					const auto row = mysql_fetch_row(resGrowthPet->pSQLResult);
					if (!row || !row[0])
						continue;

					col = 0;

					TPetGrowthCache table = {};
					table.dwItemID = static_cast<uint16_t>(std::stoi(row[col++]));
					strlcpy(table.szName, row[col++], sizeof(table.szName));
					table.dwPetVnum = static_cast<uint16_t>(std::stoi(row[col++]));
					table.level = static_cast<uint16_t>(std::stoi(row[col++]));
					table.stage = static_cast<uint16_t>(std::stoi(row[col++]));
					table.exp = static_cast<uint32_t>(std::stoi(row[col++]));

					table.bonuses[0] = static_cast<uint16_t>(std::stoi(row[col++]));
					table.bonuses[1] = static_cast<uint16_t>(std::stoi(row[col++]));
					table.bonuses[2] = static_cast<uint16_t>(std::stoi(row[col++]));

					for (auto g = 0; g < PET_MAX_SKILLS; g++)
					{
						table.skill[g] = static_cast<int16_t>(std::stoi(row[col++]));
						table.skillLevel[g] = static_cast<uint16_t>(std::stoi(row[col++]));
					}

					std::memcpy(&item.pet, &table, sizeof(TPetGrowthCache));
				}
			}
		}
#endif
	}
	return true;
}

static bool CreatePrivateShopSaleTableFromRes(MYSQL_RES* pRes, std::vector<TPrivateShopSale>* pVec, DWORD dwPID)
{
	if (!pRes)
	{
		pVec->clear();
		return false;
	}

	auto iRow = mysql_num_rows(pRes);
	if (iRow <= 0)
	{
		pVec->clear();
		return false;
	}

	pVec->resize(iRow);

	/*
		id,customer_id,customer_name,time,item_id,count,vnum,gold,cheque,socket0,socket1,socket2,
		attrtype0,attrvalue0,attrtype1,attrvalue1,attrtype2,attrvalue2,attrtype3,attrvalue3,attrtype4,attrvalue4,attrtype5,attrvalue5,attrtype6,attrvalue6
	*/
	for (int i = 0; i < iRow; ++i)
	{
		MYSQL_ROW row = mysql_fetch_row(pRes);
		TPrivateShopSale& sale = pVec->at(i);

		int cur = 0;
		str_to_number(sale.dwID, row[cur++]);
		str_to_number(sale.dwCustomer, row[cur++]);
		strlcpy(sale.szCustomerName, row[cur++], sizeof(sale.szCustomerName));
		str_to_number(sale.tTime, row[cur++]);

		str_to_number(sale.TItem.dwID, row[cur++]);
		str_to_number(sale.TItem.dwCount, row[cur++]);
		str_to_number(sale.TItem.dwVnum, row[cur++]);
		str_to_number(sale.TItem.TPrice.llGold, row[cur++]);
		str_to_number(sale.TItem.TPrice.dwCheque, row[cur++]);
		str_to_number(sale.TItem.tCheckin, row[cur++]);
#ifdef ENABLE_PRIVATE_SHOP_CHANGE_LOOK
        str_to_number(sale.TItem.dwTransmutationVnum, row[cur++]);
#endif
#ifdef ENABLE_PRIVATE_SHOP_REFINE_ELEMENT
        str_to_number(sale.TItem.dwRefineElement, row[cur++]);
#endif
		str_to_number(sale.TItem.alSockets[0], row[cur++]);
		str_to_number(sale.TItem.alSockets[1], row[cur++]);
		str_to_number(sale.TItem.alSockets[2], row[cur++]);
#ifdef ENABLE_PRIVATE_SHOP_SOCKET5
		str_to_number(sale.TItem.alSockets[3], row[cur++]);
		str_to_number(sale.TItem.alSockets[4], row[cur++]);
#endif
#ifdef ENABLE_PRIVATE_SHOP_APPLY_RANDOM
        for (int j = 0; j < ITEM_APPLY_MAX_NUM; j++)
        {
            str_to_number(sale.TItem.aApplyRandom[j].bType, row[cur++]);
            str_to_number(sale.TItem.aApplyRandom[j].sValue, row[cur++]);
            str_to_number(sale.TItem.aApplyRandom[j].bPath, row[cur++]);
        }
#endif
		for (int j = 0; j < ITEM_ATTRIBUTE_MAX_NUM; j++)
		{
			str_to_number(sale.TItem.aAttr[j].bType, row[cur++]);
			str_to_number(sale.TItem.aAttr[j].sValue, row[cur++]);
		}

		sale.dwOwner = dwPID;
		sale.TItem.dwOwner = dwPID;
	}

	return true;
}
