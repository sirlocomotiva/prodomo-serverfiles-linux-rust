#include "stdafx.h"

#include <fmt/format.h>

#include "ClientManager.h"
#include "Cache.h"
#include "Main.h"
#include "QID.h"
#include "PrivateShop.h"
#include "PrivateShopUtils.h"
#include "../libgame/grid.h"

extern int g_test_server;
extern bool g_bHotBackup;
extern int g_log;

///////////////////////////////////////////////////////////////////////////////////////
//  Query Results
///////////////////////////////////////////////////////////////////////////////////////
void CClientManager::RESULT_PRIVATE_SHOP_LOAD(CPeer* pPeer, MYSQL_RES* pRes, DWORD dwHandle, DWORD dwPID)
{
	TPrivateShop TShop {};

	if (!CreatePrivateShopTableFromRes(pRes, TShop))
		return;

	// Send data to the player
	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_LOAD;
	pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(TPrivateShop));
	pPeer->Encode(&bSubHeader, sizeof(BYTE));
	pPeer->Encode(&TShop, sizeof(TPrivateShop));

	// Add table to cache
	PutPrivateShopCache(&TShop);

	sys_log(0, "PRIVATE_SHOP_LOAD: pid %u", dwPID);

	// Spawn the shop if none was found, fetch item data from cache as it was already loaded
	if (!GetPrivateShop(dwPID))
	{
		std::vector<TPlayerPrivateShopItem> vec_privateShopItem;
		WORD wCount = 0;
		auto it = m_map_pPrivateShopItemCacheSetPtr.find(dwPID);

		if (it == m_map_pPrivateShopItemCacheSetPtr.end())
			return;

		TPrivateShopItemCacheSet* pSet = it->second.get();
		auto it_set = pSet->begin();
		vec_privateShopItem.resize(pSet->size());

		while (it_set != pSet->end())
		{
			CPrivateShopItemCache* c = *it_set++;
			TPlayerPrivateShopItem* p = c->Get();

			if (p->dwVnum)
				memcpy(&vec_privateShopItem[wCount++], p, sizeof(TPlayerPrivateShopItem));
		}

		LPPRIVATE_SHOP pPrivateShop = PrivateShopCreate(&TShop, vec_privateShopItem);
		pPrivateShop = PrivateShopSpawn(dwPID);

		if (!pPrivateShop)
		{
			sys_err("Cannot spawn private shop %u", dwPID);
			return;
		}

		pPrivateShop->SetOwnerHandle(dwHandle);
		pPrivateShop->BindOwnerPeerHandle(pPeer->GetHandle());
	}
}

void CClientManager::RESULT_PRIVATE_SHOP_ITEM_LOAD(CPeer* pPeer, MYSQL_RES* pRes, DWORD dwHandle, DWORD dwPID)
{
	static std::vector<TPlayerPrivateShopItem> s_vec_privateShopItem;
	CreatePrivateShopItemTableFromRes(pRes, &s_vec_privateShopItem, dwPID);

	WORD wCount = s_vec_privateShopItem.size();

	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_ITEM_LOAD;
	pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(WORD) + sizeof(TPlayerPrivateShopItem) * wCount);
	pPeer->Encode(&bSubHeader, sizeof(BYTE));
	pPeer->EncodeWORD(wCount);

	CreatePrivateShopItemCacheSet(dwPID);

	sys_log(0, "PRIVATE_SHOP_ITEM_LOAD: count %u pid %u", wCount, dwPID);

	if (wCount)
	{
#ifdef ENABLE_PET_GROWTH_SYSTEM
		for (uint32_t i = 0; i < s_vec_privateShopItem.size(); ++i)
		{
			const auto it = m_map_itemTableByVnum.find(s_vec_privateShopItem[i].dwVnum);

			if (it == m_map_itemTableByVnum.end())
				continue;

			if (it->second->bType == ITEM_PET && ((it->second->bSubType == PET_UPBRINGING) || (it->second->bSubType == PET_BAG)))
			{
				uint32_t col = 0;

				std::string petColumns = fmt::format("id, name, pet_vnum, pet_level, pet_stage, pet_exp, hp, sp, def, pet_skill0, pet_skillLevel0, pet_skill1, pet_skillLevel1, pet_skill2, pet_skillLevel2, pet_skill3, pet_skillLevel3, pet_skill4, pet_skillLevel4");
				std::string petQuery = fmt::format("SELECT {} FROM pet_cache{} WHERE id = {}", petColumns, GetTablePostfix(), s_vec_privateShopItem[i].alSockets[2]);

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

						std::memcpy(&s_vec_privateShopItem[i].pet, &table, sizeof(TPetGrowthCache));
					}
				}
			}
		}
#endif
		pPeer->Encode(&s_vec_privateShopItem[0], sizeof(TPlayerPrivateShopItem) * wCount);

		for (DWORD i = 0; i < wCount; ++i)
			PutPrivateShopItemCache(&s_vec_privateShopItem[i], true);
	}
}

void CClientManager::RESULT_PRIVATE_SHOP_SALE_LOAD(CPeer* pPeer, MYSQL_RES* pRes, DWORD dwHandle, DWORD dwPID)
{
	static std::vector<TPrivateShopSale> s_vec_privateShopSale;
	CreatePrivateShopSaleTableFromRes(pRes, &s_vec_privateShopSale, dwPID);

	WORD wCount = s_vec_privateShopSale.size();

	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_SALE_LOAD;
	pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(WORD) + sizeof(TPrivateShopSale) * wCount);
	pPeer->Encode(&bSubHeader, sizeof(BYTE));
	pPeer->EncodeWORD(wCount);

	CreatePrivateShopSaleCacheSet(dwPID);

	sys_log(0, "RESULT_PRIVATE_SHOP_SALE_LOAD: count %u pid %u", wCount, dwPID);

	if (wCount)
	{
#ifdef ENABLE_PET_GROWTH_SYSTEM
		for (uint32_t i = 0; i < s_vec_privateShopSale.size(); ++i)
		{
			const auto it = m_map_itemTableByVnum.find(s_vec_privateShopSale[i].TItem.dwVnum);

			if (it == m_map_itemTableByVnum.end())
				continue;

			if (it->second->bType == ITEM_PET && ((it->second->bSubType == PET_UPBRINGING) || (it->second->bSubType == PET_BAG)))
			{
				uint32_t col = 0;

				std::string petColumns = fmt::format("id, name, pet_vnum, pet_level, pet_stage, pet_exp, hp, sp, def, pet_skill0, pet_skillLevel0, pet_skill1, pet_skillLevel1, pet_skill2, pet_skillLevel2, pet_skill3, pet_skillLevel3, pet_skill4, pet_skillLevel4");
				std::string petQuery = fmt::format("SELECT {} FROM pet_cache{} WHERE id = {}", petColumns, GetTablePostfix(), s_vec_privateShopSale[i].TItem.alSockets[2]);

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

						std::memcpy(&s_vec_privateShopSale[i].TItem.pet, &table, sizeof(TPetGrowthCache));
					}
				}
			}
		}
#endif
		pPeer->Encode(&s_vec_privateShopSale[0], sizeof(TPrivateShopSale) * wCount);

		for (DWORD i = 0; i < wCount; ++i)
			PutPrivateShopSaleCache(&s_vec_privateShopSale[i], true);
	}
}

///////////////////////////////////////////////////////////////////////////////////////
//  Private Shop Cache
///////////////////////////////////////////////////////////////////////////////////////
void CClientManager::FlushPrivateShopCache(DWORD dwPID)
{
	auto it = m_map_privateShopCache.find(dwPID);

	if (it != m_map_privateShopCache.end())
	{
		CPrivateShopCache* c = it->second.get();

		c->Flush();
		m_map_privateShopCache.erase(it);
	}
}

CPrivateShopCache* CClientManager::GetPrivateShopCache(DWORD dwPID)
{
	auto it = m_map_privateShopCache.find(dwPID);

	if (it == m_map_privateShopCache.end())
		return nullptr;

	return it->second.get();
}

void CClientManager::PutPrivateShopCache(TPrivateShop* pTable)
{
	CPrivateShopCache* c;

	c = GetPrivateShopCache(pTable->dwOwner);

	if (!c)
	{
		std::unique_ptr<CPrivateShopCache> upShopCache = std::make_unique<CPrivateShopCache>();
		c = upShopCache.get();

		m_map_privateShopCache.emplace(pTable->dwOwner, std::move(upShopCache));
	}

	c->Put(pTable);
}

bool CClientManager::DeletePrivateShopCache(DWORD dwPID)
{
	CPrivateShopCache* c = GetPrivateShopCache(dwPID);

	if (!c)
		return false;

	c->Delete();
	return true;
}

void CClientManager::UpdatePrivateShopCache()
{
	if (m_iCacheFlushCount >= m_iCacheFlushCountLimit)
		return;

	auto it = m_map_privateShopCache.begin();

	while (it != m_map_privateShopCache.end())
	{
		CPrivateShopCache* c = (it++)->second.get();

		if (c->CheckFlushTimeout())
		{
			if (g_test_server)
				sys_log(0, "UpdatePrivateShopCache ==> Flush() owner %d", c->Get()->dwOwner);

			c->Flush();

			if (++m_iCacheFlushCount >= m_iCacheFlushCountLimit)
				break;
		}
	}
}

CClientManager::TPrivateShopItemCacheSet* CClientManager::GetPrivateShopItemCacheSet(DWORD dwPID)
{
	auto it = m_map_pPrivateShopItemCacheSetPtr.find(dwPID);

	if (it == m_map_pPrivateShopItemCacheSetPtr.end())
		return nullptr;

	return it->second.get();
}

void CClientManager::CreatePrivateShopItemCacheSet(DWORD dwPID)
{
	if (m_map_pPrivateShopItemCacheSetPtr.find(dwPID) != m_map_pPrivateShopItemCacheSetPtr.end())
		return;

	std::unique_ptr<TPrivateShopItemCacheSet> upSet = std::make_unique<TPrivateShopItemCacheSet>();
	m_map_pPrivateShopItemCacheSetPtr.emplace(dwPID, std::move(upSet));

	if (g_log)
		sys_log(0, "PRIVATE_SHOP_ITEM_CACHE: new cache %u", dwPID);
}

void CClientManager::FlushPrivateShopItemCacheSet(DWORD dwPID)
{
	auto it = m_map_pPrivateShopItemCacheSetPtr.find(dwPID);

	if (it == m_map_pPrivateShopItemCacheSetPtr.end())
	{
		sys_log(0, "FLUSH_PRIVATE_SHOP_ITEM_CACHESET : No PrivateShopItemCacheSet pid(%d)", dwPID);
		return;
	}

	TPrivateShopItemCacheSet* pSet = it->second.get();
	auto it_set = pSet->begin();

	while (it_set != pSet->end())
	{
		CPrivateShopItemCache* c = *it_set++;
		c->Flush();

		m_map_privateShopItemCache.erase(c->Get()->dwID);
	}

	pSet->clear();
	m_map_pPrivateShopItemCacheSetPtr.erase(it);

	if (g_log)
		sys_log(0, "FLUSH_PRIVATE_SHOP_ITEM_CACHESET : Deleted pid(%d)", dwPID);
}

bool CClientManager::DeletePrivateShopItemCacheSet(DWORD dwPID)
{
	auto it = m_map_pPrivateShopItemCacheSetPtr.find(dwPID);

	if (it == m_map_pPrivateShopItemCacheSetPtr.end())
	{
		sys_log(0, "DELETE_PRIVATE_SHOP_ITEM_CACHESET : No PrivateShopItemCacheSet pid(%d)", dwPID);
		return false;
	}

	TPrivateShopItemCacheSet* pSet = it->second.get();
	auto it_set = pSet->begin();

	while (it_set != pSet->end())
	{
		CPrivateShopItemCache* c = *it_set++;
		c->Delete();
		c->Flush();

		m_map_privateShopItemCache.erase(c->Get()->dwID);
	}

	pSet->clear();
	m_map_pPrivateShopItemCacheSetPtr.erase(it);

	if (g_log)
		sys_log(0, "DELETE_PRIVATE_SHOP_ITEM_CACHESET : Deleted pid(%d)", dwPID);

	return true;
}

CPrivateShopItemCache* CClientManager::GetPrivateShopItemCache(DWORD dwID)
{
	auto it = m_map_privateShopItemCache.find(dwID);

	if (it == m_map_privateShopItemCache.end())
		return nullptr;

	return it->second.get();
}

void CClientManager::PutPrivateShopItemCache(TPlayerPrivateShopItem* pNew, bool bSkipQuery)
{
	CPrivateShopItemCache* c;

	c = GetPrivateShopItemCache(pNew->dwID);
	if (!c)
	{
		if (g_log)
			sys_log(0, "PRIVATE_SHOP_ITEM_CACHE: PutPrivateShopItemCache ==> New CPrivateShopItemCache id%d vnum%d new owner%d", pNew->dwID, pNew->dwVnum, pNew->dwOwner);

		std::unique_ptr< CPrivateShopItemCache> upPrivateShopItemCache = std::make_unique<CPrivateShopItemCache>();
		c = upPrivateShopItemCache.get();

		m_map_privateShopItemCache.emplace(pNew->dwID, std::move(upPrivateShopItemCache));
	}
	else
	{
		if (g_log)
			sys_log(0, "PRIVATE_SHOP_ITEM_CACHE: PutPrivateShopItemCache ==> Have Cache");
		if (pNew->dwOwner != c->Get()->dwOwner)
		{
			auto it = m_map_pPrivateShopItemCacheSetPtr.find(c->Get()->dwOwner);

			if (it != m_map_pPrivateShopItemCacheSetPtr.end())
			{
				if (g_log)
					sys_log(0, "PRIVATE_SHOP_ITEM_CACHE: delete owner %u id %u new owner %u", c->Get()->dwOwner, c->Get()->dwID, pNew->dwOwner);

				TPrivateShopItemCacheSet* pSet = it->second.get();
				pSet->erase(c);
			}
		}
	}
	c->Put(pNew, bSkipQuery);

	auto it = m_map_pPrivateShopItemCacheSetPtr.find(c->Get()->dwOwner);

	if (it != m_map_pPrivateShopItemCacheSetPtr.end())
	{
		if (g_log)
			sys_log(0, "PRIVATE_SHOP_ITEM_CACHE: save %u id %u", c->Get()->dwOwner, c->Get()->dwID);
		else
			sys_log(1, "PRIVATE_SHOP_ITEM_CACHE: save %u id %u", c->Get()->dwOwner, c->Get()->dwID);

		TPrivateShopItemCacheSet* pSet = it->second.get();
		pSet->insert(c);
	}
	else
	{
		if (g_log)
			sys_log(0, "PRIVATE_SHOP_ITEM_CACHE: direct save %u id %u", c->Get()->dwOwner, c->Get()->dwID);
		else
			sys_log(1, "PRIVATE_SHOP_ITEM_CACHE: direct save %u id %u", c->Get()->dwOwner, c->Get()->dwID);

		c->OnFlush();
	}
}

bool CClientManager::DeletePrivateShopItemCache(DWORD dwID)
{
	CPrivateShopItemCache* c = GetPrivateShopItemCache(dwID);

	if (!c)
		return false;

	c->Delete();
	return true;
}

void CClientManager::UpdatePrivateShopItemCache()
{
	if (m_iCacheFlushCount >= m_iCacheFlushCountLimit)
		return;

	auto it = m_map_privateShopItemCache.begin();

	while (it != m_map_privateShopItemCache.end())
	{
		CPrivateShopItemCache* c = (it++)->second.get();

		if (c->CheckFlushTimeout())
		{
			if (g_test_server)
				sys_log(0, "UpdatePrivateShopItemCache ==> Flush() vnum %d owner %d", c->Get()->dwVnum, c->Get()->dwOwner);

			c->Flush();

			if (++m_iCacheFlushCount >= m_iCacheFlushCountLimit)
				break;
		}
	}
}

void CClientManager::UpdatePrivateShopItemCacheSet(DWORD dwPID)
{
	auto it = m_map_pPrivateShopItemCacheSetPtr.find(dwPID);

	if (it == m_map_pPrivateShopItemCacheSetPtr.end())
	{
		if (g_test_server)
			sys_log(0, "UPDATE_PRIVATE_SHOP_ITEM_CACHESET : UpdatePrivateShopItemCacheSet ==> No PrivateShopItemCacheSet pid(%d)", dwPID);
		return;
	}

	TPrivateShopItemCacheSet* pSet = it->second.get();
	auto it_set = pSet->begin();

	while (it_set != pSet->end())
	{
		CPrivateShopItemCache* c = *it_set++;
		c->Flush();
	}

	if (g_log)
		sys_log(0, "UPDATE_PRIVATE_SHOP_ITEM_CACHESET : UpdatePrivateShopItemCacheSet pid(%d)", dwPID);
}

CClientManager::TPrivateShopSaleCacheSet* CClientManager::GetPrivateShopSaleCacheSet(DWORD dwPID)
{
	auto it = m_map_pPrivateShopSaleCacheSetPtr.find(dwPID);

	if (it == m_map_pPrivateShopSaleCacheSetPtr.end())
		return nullptr;

	return it->second.get();
}

void CClientManager::CreatePrivateShopSaleCacheSet(DWORD dwPID)
{
	if (m_map_pPrivateShopSaleCacheSetPtr.find(dwPID) != m_map_pPrivateShopSaleCacheSetPtr.end())
		return;

	std::unique_ptr<TPrivateShopSaleCacheSet> upSet = std::make_unique<TPrivateShopSaleCacheSet>();
	m_map_pPrivateShopSaleCacheSetPtr.emplace(dwPID, std::move(upSet));

	if (g_log)
		sys_log(0, "PRIVATE_SHOP_SALE_CACHE: new cache %u", dwPID);
}

void CClientManager::FlushPrivateShopSaleCacheSet(DWORD dwPID)
{
	auto it = m_map_pPrivateShopSaleCacheSetPtr.find(dwPID);

	if (it == m_map_pPrivateShopSaleCacheSetPtr.end())
	{
		sys_log(0, "FLUSH_PRIVATE_SHOP_SALE_CACHESET : No PrivateShopSaleCacheSet pid(%d)", dwPID);
		return;
	}

	TPrivateShopSaleCacheSet* pSet = it->second.get();
	auto it_set = pSet->begin();

	while (it_set != pSet->end())
	{
		CPrivateShopSaleCache* c = *it_set++;
		c->Flush();

		m_map_privateShopSaleCache.erase(c->Get()->dwID);
	}

	pSet->clear();
	m_map_pPrivateShopSaleCacheSetPtr.erase(it);

	if (g_log)
		sys_log(0, "FLUSH_PRIVATE_SHOP_SALE_CACHESET : Deleted pid(%d)", dwPID);
}

bool CClientManager::DeletePrivateShopSaleCacheSet(DWORD dwPID)
{
	auto it = m_map_pPrivateShopSaleCacheSetPtr.find(dwPID);

	if (it == m_map_pPrivateShopSaleCacheSetPtr.end())
	{
		sys_log(0, "DELETE_PRIVATE_SHOP_SALE_CACHESET : No PrivateShopSaleCacheSet pid(%d)", dwPID);
		return false;
	}

	TPrivateShopSaleCacheSet* pSet = it->second.get();
	auto it_set = pSet->begin();

	while (it_set != pSet->end())
	{
		CPrivateShopSaleCache* c = *it_set++;
		c->Delete();
		c->Flush();

		m_map_privateShopSaleCache.erase(c->Get()->dwID);
	}

	pSet->clear();
	m_map_pPrivateShopSaleCacheSetPtr.erase(it);

	if (g_log)
		sys_log(0, "DELETE_PRIVATE_SHOP_SALE_CACHESET : Deleted pid(%d)", dwPID);

	return true;
}

CPrivateShopSaleCache* CClientManager::GetPrivateShopSaleCache(DWORD dwID)
{
	auto it = m_map_privateShopSaleCache.find(dwID);

	if (it == m_map_privateShopSaleCache.end())
		return nullptr;

	return it->second.get();
}

void CClientManager::PutPrivateShopSaleCache(TPrivateShopSale* pNew, bool bSkipQuery)
{
	CPrivateShopSaleCache* c;

	c = GetPrivateShopSaleCache(pNew->dwID);
	if (!c)
	{
		if (g_log)
			sys_log(0, "PRIVATE_SHOP_SALE_CACHE: PutPrivateShopSaleCache ==> New CPrivateShopSaleCache id%d vnum%d new owner%d", pNew->dwID, pNew->TItem.dwVnum, pNew->dwOwner);

		std::unique_ptr< CPrivateShopSaleCache> upPrivateShopSaleCache = std::make_unique<CPrivateShopSaleCache>();
		c = upPrivateShopSaleCache.get();

		m_map_privateShopSaleCache.emplace(pNew->dwID, std::move(upPrivateShopSaleCache));
	}
	else
	{
		if (g_log)
			sys_log(0, "PRIVATE_SHOP_SALE_CACHE: PutPrivateShopSaleCache ==> Have Cache");

		if (pNew->dwOwner != c->Get()->dwOwner)
		{
			auto it = m_map_pPrivateShopSaleCacheSetPtr.find(c->Get()->dwOwner);

			if (it != m_map_pPrivateShopSaleCacheSetPtr.end())
			{
				if (g_log)
					sys_log(0, "PRIVATE_SHOP_SALE_CACHE: delete owner %u id %u new owner %u", c->Get()->dwOwner, c->Get()->dwID, pNew->dwOwner);

				TPrivateShopSaleCacheSet* pSet = it->second.get();
				pSet->erase(c);
			}
		}
	}
	c->Put(pNew, bSkipQuery);

	auto it = m_map_pPrivateShopSaleCacheSetPtr.find(c->Get()->dwOwner);

	if (it != m_map_pPrivateShopSaleCacheSetPtr.end())
	{
		if (g_log)
			sys_log(0, "PRIVATE_SHOP_SALE_CACHE: save %u id %u", c->Get()->dwOwner, c->Get()->dwID);
		else
			sys_log(1, "PRIVATE_SHOP_SALE_CACHE: save %u id %u", c->Get()->dwOwner, c->Get()->dwID);

		TPrivateShopSaleCacheSet* pSet = it->second.get();
		pSet->insert(c);
	}
	else
	{
		if (g_log)
			sys_log(0, "PRIVATE_SHOP_SALE_CACHE: direct save %u id %u", c->Get()->dwOwner, c->Get()->dwID);
		else
			sys_log(1, "PRIVATE_SHOP_SALE_CACHE: direct save %u id %u", c->Get()->dwOwner, c->Get()->dwID);

		c->OnFlush();
	}
}

bool CClientManager::DeletePrivateShopSaleCache(DWORD dwID)
{
	CPrivateShopSaleCache* c = GetPrivateShopSaleCache(dwID);

	if (!c)
		return false;

	c->Delete();
	return true;
}

void CClientManager::UpdatePrivateShopSaleCache()
{
	if (m_iCacheFlushCount >= m_iCacheFlushCountLimit)
		return;

	auto it = m_map_privateShopSaleCache.begin();

	while (it != m_map_privateShopSaleCache.end())
	{
		CPrivateShopSaleCache* c = (it++)->second.get();

		if (c->CheckFlushTimeout())
		{
			if (g_test_server)
				sys_log(0, "UpdatePrivateShopSaleCache ==> Flush() id %d owner %d", c->Get()->dwID, c->Get()->dwOwner);

			c->Flush();

			if (++m_iCacheFlushCount >= m_iCacheFlushCountLimit)
				break;
		}
	}
}

void CClientManager::UpdatePrivateShopSaleCacheSet(DWORD dwPID)
{
	auto it = m_map_pPrivateShopSaleCacheSetPtr.find(dwPID);

	if (it == m_map_pPrivateShopSaleCacheSetPtr.end())
	{
		if (g_test_server)
			sys_log(0, "UPDATE_PRIVATE_SHOP_SALE_CACHESET: UpdatePrivateShopSaleCacheSet ==> No PrivateShopSaleCacheSet pid(%d)", dwPID);
		return;
	}

	TPrivateShopSaleCacheSet* pSet = it->second.get();
	auto it_set = pSet->begin();

	while (it_set != pSet->end())
	{
		CPrivateShopSaleCache* c = *it_set++;
		c->Flush();
	}

	if (g_log)
		sys_log(0, "UPDATE_PRIVATE_SHOP_SALE_CACHESET: UpdatePrivateShopSaleCacheSet pid(%d)", dwPID);
}

void CClientManager::AddMarketItemPrice(TPrivateShopSale& rSale)
{
	TItemPrice TPrice {};
	TPrice.llGold = rSale.TItem.TPrice.llGold;
	TPrice.dwCheque = rSale.TItem.TPrice.dwCheque;

	m_map_marketItemPriceList[rSale.TItem.dwVnum].push_back(TPrice);
}

TItemPrice* CClientManager::GetMarketItemPrice(DWORD dwVnum)
{
	auto it = m_map_marketItemPrice.find(dwVnum);

	if (it == m_map_marketItemPrice.end())
		return nullptr;

	return &it->second;
}

void CClientManager::UpdateMarketItemPrice()
{
	static std::vector<TMarketItemPrice>	vec_updatedMarketItemPrices;
	vec_updatedMarketItemPrices.clear();

	for (const auto& kv : m_map_marketItemPriceList)
	{
		const DWORD dwVnum = kv.first;
		const TMarketItemPriceList& rMarketItemPriceList = kv.second;
		size_t sItemCount = rMarketItemPriceList.size();

		TItemPrice TAvgPrice {};

		for (const auto& rItemPrice : rMarketItemPriceList)
		{
			TAvgPrice.llGold += rItemPrice.llGold;
			TAvgPrice.dwCheque += rItemPrice.dwCheque;
		}

		TItemPrice* pCurrentItemPrice = GetMarketItemPrice(dwVnum);
		if (pCurrentItemPrice)
		{
			TAvgPrice.llGold += pCurrentItemPrice->llGold;
			TAvgPrice.dwCheque += pCurrentItemPrice->dwCheque;
			++sItemCount;
		}

		TAvgPrice.llGold /= sItemCount;
		TAvgPrice.dwCheque /= sItemCount;

		TMarketItemPrice TNewPrice{};
		TNewPrice.dwVnum = dwVnum;
		TNewPrice.TPrice = TAvgPrice;

		auto it = m_map_marketItemPrice.find(dwVnum);
		if (it == m_map_marketItemPrice.end())
			m_map_marketItemPrice.emplace(dwVnum, TAvgPrice);
		else
			it->second = TAvgPrice;

		vec_updatedMarketItemPrices.push_back(TNewPrice);
	}

	// Push updated prices to the game cores
	for (const auto& pPeer : m_peerList)
	{
		if (!pPeer->GetChannel())
			continue;

		BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_MARKET_ITEM_PRICE_DATA_UPDATE;
		WORD wCount = vec_updatedMarketItemPrices.size();

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(WORD) + sizeof(TMarketItemPrice) * vec_updatedMarketItemPrices.size());
		pPeer->Encode(&bSubHeader, sizeof(BYTE));
		pPeer->Encode(&wCount, sizeof(WORD));

		for (const auto& rMarketItemPrice : vec_updatedMarketItemPrices)
			pPeer->Encode(&rMarketItemPrice, sizeof(TMarketItemPrice));
	}

	m_map_marketItemPriceList.clear();
}

///////////////////////////////////////////////////////////////////////////////////////
//  Private Shop Management
///////////////////////////////////////////////////////////////////////////////////////
LPPRIVATE_SHOP CClientManager::CreatePrivateShop(DWORD dwPID)
{
	auto it = m_map_privateShop.find(dwPID);

	if (it != m_map_privateShop.end())
	{
		sys_err("Duplicated private shop with id %d", dwPID);
		m_map_privateShop.erase(it);
	}

	std::unique_ptr<CPrivateShop> upPrivateShop = std::make_unique<CPrivateShop>();
	LPPRIVATE_SHOP pPrivateShop = upPrivateShop.get();

	m_map_privateShop.emplace(dwPID, std::move(upPrivateShop));

	return pPrivateShop;
}

bool CClientManager::DeletePrivateShop(DWORD dwPID)
{
	auto it = m_map_privateShop.find(dwPID);

	if (it == m_map_privateShop.end())
	{
		sys_err("Could not find private shop with id %d", dwPID);
		return false;
	}

	m_map_privateShop.erase(it);
	return true;
}

LPPRIVATE_SHOP CClientManager::GetPrivateShop(DWORD dwPID)
{
	auto it = m_map_privateShop.find(dwPID);

	if (it == m_map_privateShop.end())
		return nullptr;

	return it->second.get();
}

bool CClientManager::InitializePrivateShopMarketItemPrice()
{
	char szQuery[512]{};
	snprintf(szQuery, sizeof(szQuery),
		"SELECT vnum, gold, cheque FROM private_shop_sale_history%s WHERE DATEDIFF(time, FROM_UNIXTIME(%u)) < %u",
		GetTablePostfix(), time(0), VALID_MARKET_PRICE_DAY_INTERVAL);

	std::unique_ptr<SQLMsg> pkMsg(CDBManager::instance().DirectQuery(szQuery));
	SQLResult* pRes = pkMsg->Get();

	if (!m_map_marketItemPrice.empty())
	{
		sys_log(0, "RELOAD: private shop market item price");
		m_map_marketItemPrice.clear();
	}

	static TMarketItemPriceListMap map_marketItemPriceList;
	map_marketItemPriceList.clear();

	MYSQL_ROW data;
	if (pRes->uiNumRows > 0)
	{
		while ((data = mysql_fetch_row(pRes->pSQLResult)))
		{
			int col = 0;
			DWORD dwVnum = 0;
			TItemPrice TPrice{};

			str_to_number(dwVnum, data[col++]);
			str_to_number(TPrice.llGold, data[col++]);
			str_to_number(TPrice.dwCheque, data[col++]);

			map_marketItemPriceList[dwVnum].push_back(TPrice);
		}
	}

	if (map_marketItemPriceList.empty())
		return true;

	for (const auto& kv : map_marketItemPriceList)
	{
		const DWORD dwVnum = kv.first;
		const TMarketItemPriceList& rMarketItemPriceList = kv.second;

		TItemPrice TAvgPrice{};

		for (const auto& rItemPrice : rMarketItemPriceList)
		{
			TAvgPrice.llGold += rItemPrice.llGold;
			TAvgPrice.dwCheque += rItemPrice.dwCheque;
		}

		TAvgPrice.llGold /= rMarketItemPriceList.size();
		TAvgPrice.dwCheque /= rMarketItemPriceList.size();

		m_map_marketItemPrice.emplace(dwVnum, TAvgPrice);
	}

	return true;
}

void CClientManager::ProcessPrivateShopPacket(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	const BYTE bSubHeader = *reinterpret_cast<const BYTE*>(c_szData);
	c_szData += sizeof(BYTE);

	switch (bSubHeader)
	{
		/* Core Mechanics */
		case PRIVATE_SHOP_GD_SUBHEADER_LOGOUT:
			PrivateShopStartPremiumEvent(*(DWORD*)c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_CREATE:
			PrivateShopBuild(pPeer, dwHandle, c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_CLOSE:
			PrivateShopClose(pPeer, dwHandle, c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_DELETE:
			PrivateShopDelete(pPeer, dwHandle, c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_DESPAWN:
			PrivateShopDespawn(pPeer, dwHandle, c_szData);
			break;

		/* Requests */
		case PRIVATE_SHOP_GD_SUBHEADER_WITHDRAW_REQUEST:
			PrivateShopWithdrawRequest(pPeer, dwHandle, c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_MODIFY_REQUEST:
			PrivateShopModifyRequest(pPeer, dwHandle, c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_BUY_REQUEST:
			PrivateShopBuyRequest(pPeer, dwHandle, c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_ITEM_PRICE_CHANGE_REQUEST:
			PrivateShopItemPriceChangeRequest(pPeer, dwHandle, c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_ITEM_MOVE_REQUEST:
			PrivateShopItemMoveRequest(pPeer, dwHandle, c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_ITEM_CHECKIN_REQUEST:
			PrivateShopItemCheckinRequest(pPeer, dwHandle, c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_ITEM_CHECKIN_UPDATE:
			PrivateShopItemCheckinUpdate(pPeer, dwHandle, c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_ITEM_CHECKOUT_REQUEST:
			PrivateShopItemCheckoutRequest(pPeer, dwHandle, c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_ITEM_CHECKOUT_UPDATE:
			PrivateShopItemCheckoutUpdate(pPeer, dwHandle, c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_TITLE_CHANGE_REQUEST:
			PrivateShopTitleChangeRequest(pPeer, dwHandle, c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_WARP_REQUEST:
			PrivateShopWarpRequest(pPeer, dwHandle, c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_SLOT_UNLOCK_REQUEST:
			PrivateShopSlotUnlockRequest(pPeer, dwHandle, c_szData);
			break;

		/* Actions */
		case PRIVATE_SHOP_GD_SUBHEADER_WITHDRAW:
			PrivateShopWithdraw(c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_BUY:
			PrivateShopBuy(pPeer, dwHandle, c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_FAILED_BUY:
			PrivateShopFailedBuy(c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_ITEM_TRANSFER:
			PrivateShopItemTransfer((TPlayerItem*)c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_ITEM_DELETE:
			PrivateShopItemDelete(c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_ITEM_EXPIRE:
			PrivateShopItemExpire(c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_PREMIUM_TIME_UPDATE:
			PrivateShopPremiumTimeUpdate(c_szData);
			break;

		case PRIVATE_SHOP_GD_SUBHEADER_INIT:
			//PrivateShopPeerSpawn(pPeer);
			break;

		default:
			sys_err("Unknown subheader %d", bSubHeader);
			break;
	}
}

LPPRIVATE_SHOP CClientManager::PrivateShopSpawn(DWORD dwShopID)
{
	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(dwShopID);

	// If shop does not exist, fetch its data first
	if (!pPrivateShop)
	{
		TPrivateShop									privateShopTable{};
		static std::vector<TPlayerPrivateShopItem>		vec_privateShopItem;

		if (!PrivateShopFetchData(dwShopID, privateShopTable, vec_privateShopItem))
		{
			sys_err("Cannot fetch data for private shop %u", dwShopID);
			return nullptr;
		}

		pPrivateShop = PrivateShopCreate(&privateShopTable, vec_privateShopItem);
		if (!pPrivateShop)
		{
			sys_err("Failed to create private shop %u", dwShopID);
			return nullptr;
		}
	}

	CPeer* pShopPeer = GetPrivateShopPeer(pPrivateShop->GetChannel(), pPrivateShop->GetPort());
	if (!pShopPeer)
	{
		sys_err("Cannot find peer for private shop %u", dwShopID);
		return nullptr;
	}

	pPrivateShop->BindShopPeerHandle(pShopPeer->GetHandle());
	PrivateShopGameSpawn(pPrivateShop);
	return pPrivateShop;
}

LPPRIVATE_SHOP CClientManager::PrivateShopCreate(TPrivateShop* pTable, const std::vector<TPlayerPrivateShopItem>& c_vec_shopItem)
{
	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(pTable->dwOwner);
	if (!pPrivateShop)
	{
		pPrivateShop = CreatePrivateShop(pTable->dwOwner);
		sys_log(0, "Re-creating private shop for owner %u", pTable->dwOwner);
	}

	// Bind data to the newly created shop
	pPrivateShop->SetTable(pTable);
	pPrivateShop->RemoveAllItem();
	pPrivateShop->SetUnlockedSlots(pTable->wUnlockedSlots);

	for (const auto& c_rShopItem : c_vec_shopItem)
	{
		TItemTable* pItemTable = GetItemTable(c_rShopItem.dwVnum);
		if (!pItemTable)
		{
			sys_err("Unable find item proto table vnum at %u", c_rShopItem.dwVnum);

			// Remove previously added items
			pPrivateShop->RemoveAllItem();
			return nullptr;
		}

		if (!pPrivateShop->SetItem(&c_rShopItem, pItemTable))
		{
			sys_err("Cannot set item on private shop %u pos %u vnum %u", pTable->dwOwner, c_rShopItem.wPos, c_rShopItem.dwVnum);

			// Remove previously added items
			pPrivateShop->RemoveAllItem();
			return nullptr;
		}

		sys_log(0, "PRIVATE_SHOP: Added item %u pos %u vnum %u to private shop %u", c_rShopItem.dwID, c_rShopItem.wPos, c_rShopItem.dwVnum, pTable->dwOwner);
	}

	sys_log(0, "PRIVATE_SHOP: Private shop %u successfully created", pTable->dwOwner);
	return pPrivateShop;
}

void CClientManager::PrivateShopBuild(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	TPrivateShop* pTable = (TPrivateShop*)c_szData;
	c_szData += sizeof(TPrivateShop);

	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_CREATE_RESULT;

	TPacketDGPrivateShopCreateResult subPacket{};
	memcpy(&subPacket.privateShopTable, pTable, sizeof(subPacket.privateShopTable));

	sys_log(0, "PRIVATE_SHOP: Building private shop %u", pTable->dwOwner);

	// Check if the shop already exists
	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(pTable->dwOwner);
	if (pPrivateShop)
	{
		subPacket.bSuccess = false;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(TPacketDGPrivateShopCreateResult));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));
		pPeer->Encode(&subPacket, sizeof(TPacketDGPrivateShopCreateResult));

		sys_log(0, "PRIVATE_SHOP: Private shop %u already created", pTable->dwOwner);
		return;
	}

	auto pCacheSet = GetPrivateShopItemCacheSet(pTable->dwOwner);
	if (pCacheSet && pCacheSet->size())
	{
		subPacket.bSuccess = false;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(TPacketDGPrivateShopCreateResult));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));
		pPeer->Encode(&subPacket, sizeof(TPacketDGPrivateShopCreateResult));

		sys_log(0, "PRIVATE_SHOP: Private shop %u cannot be built as private shop item cache already exists", pTable->dwOwner);
		return;
	}

	// Initialize items
	const WORD wCount = *reinterpret_cast<const WORD*>(c_szData);
	c_szData += sizeof(WORD);

	TPlayerPrivateShopItem* pItem = (TPlayerPrivateShopItem*)c_szData;

	static std::vector<TPlayerPrivateShopItem> s_vec_privateShopItem;
	s_vec_privateShopItem.resize(wCount);

	for (int i = 0; i < wCount; ++i, ++pItem)
	{
		TPlayerPrivateShopItem& rPrivateShopItem = s_vec_privateShopItem.at(i);
		memcpy(&rPrivateShopItem, pItem, sizeof(TPlayerPrivateShopItem));
	}

	// Create private shop instance
	pPrivateShop = PrivateShopCreate(pTable, s_vec_privateShopItem);
	if (!pPrivateShop)
	{
		subPacket.bSuccess = false;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(TPacketDGPrivateShopCreateResult));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));
		pPeer->Encode(&subPacket, sizeof(TPacketDGPrivateShopCreateResult));

		sys_log(0, "PRIVATE_SHOP: Cannot build private shop for owner pid %u", pTable->dwOwner);
		return;
	}

	// Bind peer data to the instance
	pPrivateShop->BindShopPeerHandle(pPeer->GetHandle());
	pPrivateShop->BindOwnerPeerHandle(pPeer->GetHandle());
	pPrivateShop->SetOwnerHandle(dwHandle);

	// Setup cache
	PutPrivateShopCache(pTable);
	CreatePrivateShopItemCacheSet(pPrivateShop->GetOwner());

	for (auto& c_rShopItem : s_vec_privateShopItem)
	{
		// Delete item from the player's cache and remove it from item's table
		if (!DeleteItemCache(c_rShopItem.dwID))
		{
			char szQuery[64];
			snprintf(szQuery, sizeof(szQuery), "DELETE FROM item%s WHERE id=%u", GetTablePostfix(), c_rShopItem.dwID);
			CDBManager::instance().AsyncQuery(szQuery);
		}

		// Insert the item to private shop item table
		PutPrivateShopItemCache(&c_rShopItem);
	}

	// Send result back to the game core
	subPacket.bSuccess = true;

	pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(TPacketDGPrivateShopCreateResult));
	pPeer->Encode(&bSubHeader, sizeof(BYTE));
	pPeer->Encode(&subPacket, sizeof(TPacketDGPrivateShopCreateResult));

	sys_log(0, "PRIVATE_SHOP: Private shop %u successfully built", pTable->dwOwner);
}

void CClientManager::PrivateShopClose(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	const DWORD dwPID = *reinterpret_cast<const DWORD*>(c_szData);
	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_DESTROY;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(dwPID);
	if (!pPrivateShop)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NO_SHOP;
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop %u", dwPID);
		return;
	}

	if (pPrivateShop->GetGold() || pPrivateShop->GetCheque())
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_CLOSE_RESULT_BALANCE_AVAILABLE;
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		return;
	}

	pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
	pPeer->Encode(&bSubHeader, sizeof(BYTE));

	sys_log(0, "PRIVATE_SHOP: Close request forwarded to game for private shop %u", dwPID);
}

void CClientManager::PrivateShopDelete(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	const DWORD dwPID = *reinterpret_cast<const DWORD*>(c_szData);

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(dwPID);
	if (!pPrivateShop)
	{
		sys_err("Cannot find private shop %u", dwPID);
		return;
	}

	// Delete private shop cache and SQL record
	if (!DeletePrivateShopCache(dwPID))
	{
		char szQuery[64];
		snprintf(szQuery, sizeof(szQuery), "DELETE FROM private_shop%s WHERE owner_id=%u", GetTablePostfix(), dwPID);

		CDBManager::instance().AsyncQuery(szQuery);
	}

	// Delete private shop item cache and SQL record
	if (!DeletePrivateShopItemCacheSet(dwPID))
	{
		char szQuery[64];
		snprintf(szQuery, sizeof(szQuery), "DELETE FROM private_shop_item%s WHERE owner_id=%u", GetTablePostfix(), dwPID);

		CDBManager::instance().AsyncQuery(szQuery);
	}

	// Delete shop data
	DeletePrivateShop(dwPID);
	sys_log(0, "PRIVATE_SHOP: Deleted private shop with id %u", dwPID);
}

void CClientManager::PrivateShopDespawn(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	const DWORD dwPID = *reinterpret_cast<const DWORD*>(c_szData);

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(dwPID);
	if (!pPrivateShop)
	{
		sys_err("Cannot find private shop %u", dwPID);
		return;
	}

	if (IsPrivateShopPremiumEvent(dwPID))
		PrivateShopEndPremiumEvent(dwPID);

	DeletePrivateShop(dwPID);

	sys_log(0, "PRIVATE_SHOP: Despawning private shop with id %u (Game request)", dwPID);
}

void CClientManager::PrivateShopWithdrawRequest(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	const DWORD dwPID = *reinterpret_cast<const DWORD*>(c_szData);
	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_WITHDRAW;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(dwPID);
	if (!pPrivateShop)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NO_SHOP;
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop %u", dwPID);
		return;
	}

	if (!pPrivateShop->GetGold() && !pPrivateShop->GetCheque())
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_WITHDRAW_RESULT_NO_BALANCE;
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		return;
	}

	TPacketDGPrivateShopWithdraw subPacket {};
	subPacket.llGold = pPrivateShop->GetGold();
	subPacket.dwCheque = pPrivateShop->GetCheque();

	pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(TPacketDGPrivateShopWithdraw));
	pPeer->Encode(&bSubHeader, sizeof(BYTE));
	pPeer->Encode(&subPacket, sizeof(TPacketDGPrivateShopWithdraw));

	sys_log(0, "PRIVATE_SHOP: Withdraw request forwarded to game for private shop %u", dwPID);
}

void CClientManager::PrivateShopItemCheckinUpdate(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	TPlayerPrivateShopItem* pItem = (TPlayerPrivateShopItem*)c_szData;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(pItem->dwOwner);
	if (!pPrivateShop)
	{
		sys_err("Cannot find private shop %u", pItem->dwOwner);
		return;
	}

	TItemTable* pItemTable = GetItemTable(pItem->dwVnum);
	if (!pItemTable)
	{
		sys_err("Unable find item proto table vnum at %u", pItem->dwVnum);
		return;
	}

	if (!pPrivateShop->SetItem(pItem, pItemTable))
	{
		sys_err("Item checkin update failed for private shop %u", pItem->dwVnum);
		return;
	}

	// Delete item from the player's cache and remove it from item's table
	if (!DeleteItemCache(pItem->dwID))
	{
		char szQuery[64];
		snprintf(szQuery, sizeof(szQuery), "DELETE FROM item%s WHERE id=%u", GetTablePostfix(), pItem->dwID);
		CDBManager::instance().AsyncQuery(szQuery);
	}

	// Insert the item to private shop item table
	PutPrivateShopItemCache(pItem);

	// Add the item from the private shop on game core
	CPeer* pShopPeer = GetPeer(pPrivateShop->GetShopPeerHandle());
	if (pShopPeer)
	{
		BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_ADD_ITEM;
		DWORD dwShopID = pPrivateShop->GetOwner();

		pShopPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(DWORD) + sizeof(TPlayerPrivateShopItem));
		pShopPeer->Encode(&bSubHeader, sizeof(BYTE));
		pShopPeer->Encode(&dwShopID, sizeof(DWORD));
		pShopPeer->Encode(pItem, sizeof(TPlayerPrivateShopItem));
	}
	else
	{
		sys_err("Cannot find private shop peer %u", pItem->dwOwner);
	}

	sys_log(0, "PRIVATE_SHOP: Item checkin on private shop %u pos %u vnum %u gold %lld won %u", pItem->dwOwner, pItem->wPos, pItem->dwVnum, pItem->TPrice.llGold, pItem->TPrice.dwCheque);
}

void CClientManager::PrivateShopItemCheckoutUpdate(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	const WORD wPos = *reinterpret_cast<const WORD*>(c_szData);
	c_szData += sizeof(WORD);

	TPlayerItem* pItem = (TPlayerItem*)c_szData;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(pItem->owner);
	if (!pPrivateShop)
	{
		sys_err("Cannot find private shop %u", pItem->owner);
		return;
	}

	// Remove the item from private shop and private shop item table
	pPrivateShop->RemoveItem(wPos);

	// Save the item to item table
	PutItemCache(pItem);

	// Remove the item from the private shop on game core
	CPeer* pShopPeer = GetPeer(pPrivateShop->GetShopPeerHandle());
	if (pShopPeer)
	{
		BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_REMOVE_ITEM;
		DWORD dwShopID = pPrivateShop->GetOwner();

		pShopPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(DWORD) + sizeof(WORD));
		pShopPeer->Encode(&bSubHeader, sizeof(BYTE));
		pShopPeer->Encode(&dwShopID, sizeof(DWORD));
		pShopPeer->Encode(&wPos, sizeof(WORD));
	}
	else
	{
		sys_err("Cannot find private shop peer %u", pItem->owner);
	}

	sys_log(0, "PRIVATE_SHOP: Item checkout on private shop %u item %u vnum %u pos %u", pItem->owner,  pItem->id, pItem->vnum, wPos);

	// No items left but stash still exists
	if (!pPrivateShop->GetItemCount() && (pPrivateShop->GetGold() || pPrivateShop->GetCheque()))
	{
		sys_log(0, "PRIVATE_SHOP: Despawning item empty private shop %u by item checkout", pItem->owner);
		PrivateShopGameDespawn(pPrivateShop);
	}

	// No items and no stash
	else if (!pPrivateShop->GetItemCount() && !pPrivateShop->GetGold() && !pPrivateShop->GetCheque())
	{
		sys_log(0, "PRIVATE_SHOP: Destroying empty private shop %u by item checkout", pItem->owner);
		PrivateShopGameDespawn(pPrivateShop);
		PrivateShopDestroy(pPrivateShop);
	}
}

void CClientManager::PrivateShopWithdraw(const char* c_szData)
{
	const DWORD dwPID = *reinterpret_cast<const DWORD*>(c_szData);

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(dwPID);
	if (!pPrivateShop)
	{
		sys_err("No private shop was found for withrawal for player %d", dwPID);
		return;
	}

	sys_log(0, "PRIVATE_SHOP: Withdrawn private shop %u gold %lld cheque %u", dwPID, pPrivateShop->GetGold(), pPrivateShop->GetCheque());
	pPrivateShop->Withdraw();

	// Destroy shop if there are no items left in the shop
	if (!pPrivateShop->GetItemCount())
	{
		sys_log(0, "PRIVATE_SHOP: Destroying empty withdrawn private shop %u", dwPID);
		PrivateShopDestroy(pPrivateShop);
	}
}

void CClientManager::PrivateShopModifyRequest(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	const DWORD dwPID = *reinterpret_cast<const DWORD*>(c_szData);
	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_STATE_UPDATE;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(dwPID);
	if (!pPrivateShop || pPrivateShop->GetState() == STATE_CLOSED)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NO_SHOP;
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop %u", dwPID);
		return;
	}

	CPeer* pShopPeer = GetPeer(pPrivateShop->GetShopPeerHandle());
	if (!pShopPeer)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_SHOP_NOT_AVAILABLE;
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop peer for private shop %u", dwPID);
		return;
	}

	// Update state of the shop
	BYTE bState = pPrivateShop->GetState();
	if (bState == STATE_OPEN)
	{
		pPrivateShop->ChangeState(STATE_MODIFY);
		bState = STATE_MODIFY;

		sys_log(0, "PRIVATE_SHOP: State changed to modify for private shop %u", dwPID);
	}
	else if (bState == STATE_MODIFY)
	{
		pPrivateShop->ChangeState(STATE_OPEN);
		bState = STATE_OPEN;

		sys_log(0, "PRIVATE_SHOP: State changed to open for private shop %u", dwPID);
	}

	//Send the result back to owner (and shop if it is on the same peer)

	TPacketDGPrivateShopStateUpdate subPacket {};
	subPacket.dwPID = dwPID;
	subPacket.bState = bState;

	pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(TPacketDGPrivateShopStateUpdate));
	pPeer->Encode(&bSubHeader, sizeof(BYTE));
	pPeer->Encode(&subPacket, sizeof(TPacketDGPrivateShopStateUpdate));

	// Not needed to be sent twice if both player&shop are on the same peer
	if (pPeer != pShopPeer)
	{
		pShopPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(TPacketDGPrivateShopStateUpdate));
		pShopPeer->Encode(&bSubHeader, sizeof(BYTE));
		pShopPeer->Encode(&subPacket, sizeof(TPacketDGPrivateShopStateUpdate));
	}
}

void CClientManager::PrivateShopBuyRequest(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	TPacketGDPrivateShopBuyRequest* p = (TPacketGDPrivateShopBuyRequest*)c_szData;
	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_BUY_REQUEST;

	for (int i = 0; i < SELECTED_ITEM_MAX_NUM; ++i)
	{
		if (!p->aSelectedItems[i].dwShopID)
			break;

		LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(p->aSelectedItems[i].dwShopID);
		if (!pPrivateShop)
		{
			bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NO_SHOP;

			pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
			pPeer->Encode(&bSubHeader, sizeof(BYTE));

			sys_err("Cannot find private shop %u", p->aSelectedItems[i].dwShopID);
			return;
		}

		// Player cannot buy an item from the private shop while it is being modified
		if (pPrivateShop->GetState() == STATE_MODIFY)
		{
			bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_BUY_RESULT_MODIFY_STATE;

			pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
			pPeer->Encode(&bSubHeader, sizeof(BYTE));
			return;
		}

		// Check if the item exists
		TPrivateShopItemInfo* pItemInfo = pPrivateShop->GetItem(p->aSelectedItems[i].wPos);
		if (!pItemInfo || !pItemInfo->bAvailable)
		{
			bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_BUY_RESULT_FALSE_ITEM;

			pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
			pPeer->Encode(&bSubHeader, sizeof(BYTE));

			sys_err("Cannot find item on position %u for private shop %u", p->aSelectedItems[i].wPos, p->aSelectedItems[i].dwShopID);
			return;
		}

		// Check if the price corresponds
		if (pItemInfo->TItem.TPrice.llGold != p->aSelectedItems[i].TPrice.llGold || pItemInfo->TItem.TPrice.dwCheque != p->aSelectedItems[i].TPrice.dwCheque)
		{
			bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_BUY_RESULT_FALSE_PRICE;

			pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
			pPeer->Encode(&bSubHeader, sizeof(BYTE));

			sys_err("Item price mismatch on private shop %u pos %u game(gold %lld, cheque %u) db(gold %lld, cheque %u)",
				p->aSelectedItems[i].dwShopID, p->aSelectedItems[i].wPos, p->aSelectedItems[i].TPrice.llGold,
				p->aSelectedItems[i].TPrice.dwCheque, pItemInfo->TItem.TPrice.llGold, pItemInfo->TItem.TPrice.dwCheque);
			return;
		}

		// Check player's balance if he can afford the item
		if (p->llGoldBalance < pItemInfo->TItem.TPrice.llGold)
		{
			bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_BUY_RESULT_NO_GOLD;

			pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
			pPeer->Encode(&bSubHeader, sizeof(BYTE));
			return;
		}

		p->llGoldBalance -= pItemInfo->TItem.TPrice.llGold;

#ifdef ENABLE_PRIVATE_SHOP_CHEQUE
		if (p->dwChequeBalance < pItemInfo->TItem.TPrice.dwCheque)
		{
			bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_BUY_RESULT_NO_CHEQUE;

			pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
			pPeer->Encode(&bSubHeader, sizeof(BYTE));
			return;
		}

		p->dwChequeBalance -= pItemInfo->TItem.TPrice.dwCheque;
#endif
		// Check if the shop's peer is available
		CPeer* pShopPeer = GetPeer(pPrivateShop->GetShopPeerHandle());
		if (!pShopPeer)
		{
			bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_SHOP_NOT_AVAILABLE;

			pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
			pPeer->Encode(&bSubHeader, sizeof(BYTE));

			sys_err("Cannot find private shop peer for private shop %u", p->aSelectedItems[i].dwShopID);
			return;
		}
	}

	// Mark the item unavailable in case someone else also wants to buy it while the transaction is in process
	for (int i = 0; i < SELECTED_ITEM_MAX_NUM; ++i)
	{
		if (!p->aSelectedItems[i].dwShopID)
			break;

		LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(p->aSelectedItems[i].dwShopID);
		TPrivateShopItemInfo* pItemInfo = pPrivateShop->GetItem(p->aSelectedItems[i].wPos);

		pItemInfo->bAvailable = false;
	}

	// Forward the request to the game
	TPacketDGPrivateShopBuyRequest subPacket {};
	subPacket.dwCustomerPID = p->dwCustomerPID;

	for (int i = 0; i < SELECTED_ITEM_MAX_NUM; ++i)
	{
		if (!p->aSelectedItems[i].dwShopID)
			break;

		LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(p->aSelectedItems[i].dwShopID);
		TPrivateShopItemInfo* pItemInfo = pPrivateShop->GetItem(p->aSelectedItems[i].wPos);

		subPacket.arRequestedItems[i] = pItemInfo->TItem;

		sys_log(0, "PRIVATE_SHOP: Buy request forwarded to game for private shop %u customer %u item pos %u item %u yang %lld won %u",
			p->aSelectedItems[i].dwShopID, p->dwCustomerPID, p->aSelectedItems[i].wPos, pItemInfo->TItem.dwVnum,
			p->aSelectedItems[i].TPrice.llGold, p->aSelectedItems[i].TPrice.dwCheque);
	}

	pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(TPacketDGPrivateShopBuyRequest));
	pPeer->Encode(&bSubHeader, sizeof(BYTE));
	pPeer->Encode(&subPacket, sizeof(TPacketDGPrivateShopBuyRequest));
}

void CClientManager::PrivateShopItemPriceChangeRequest(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	TPacketPrivateShopItemPriceChange* p = (TPacketPrivateShopItemPriceChange*)c_szData;
	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_ITEM_PRICE_CHANGE;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(p->dwShopID);
	if (!pPrivateShop)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NO_SHOP;
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop %u", p->dwShopID);
		return;
	}

	// Player cannot edit shop's content if its not set to modify state
	if (pPrivateShop->GetState() != STATE_MODIFY)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NOT_MODIFY_STATE;
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));
		return;
	}

	CPeer* pShopPeer = GetPeer(pPrivateShop->GetShopPeerHandle());
	if (!pShopPeer)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_SHOP_NOT_AVAILABLE;
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop peer for private shop %u", p->dwShopID);
		return;
	}

	// If item price change was successful, forward the packet to owner & shop
	if (pPrivateShop->ChangeItemPrice(p->wPos, p->TPrice.llGold, p->TPrice.dwCheque))
	{
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(TPacketPrivateShopItemPriceChange));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));
		pPeer->Encode(p, sizeof(TPacketPrivateShopItemPriceChange));

		// Not needed to be sent twice if both player&shop are on the same peer
		if (pPeer != pShopPeer)
		{
			pShopPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(TPacketPrivateShopItemPriceChange));
			pShopPeer->Encode(&bSubHeader, sizeof(BYTE));
			pShopPeer->Encode(p, sizeof(TPacketPrivateShopItemPriceChange));
		}

		sys_log(0, "PRIVATE_SHOP: Changed item price on pos %u private shop %u -> gold %lld won %u", p->wPos, p->dwShopID, p->TPrice.llGold, p->TPrice.dwCheque);
	}
	else
	{
		BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_BUY_RESULT_FALSE_ITEM;
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot change item price on private shop %u item pos %u", p->dwShopID, p->wPos);
	}
}

void CClientManager::PrivateShopItemMoveRequest(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	TPacketPrivateShopItemMove* p = (TPacketPrivateShopItemMove*)c_szData;
	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_ITEM_MOVE;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(p->dwShopID);
	if (!pPrivateShop)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NO_SHOP;
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop %u", p->dwShopID);
		return;
	}

	// Player cannot edit shop's content if its not set to modify state
	if (pPrivateShop->GetState() != STATE_MODIFY)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NOT_MODIFY_STATE;
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));
		return;
	}

	CPeer* pShopPeer = GetPeer(pPrivateShop->GetShopPeerHandle());
	if (!pShopPeer)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_SHOP_NOT_AVAILABLE;
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop peer for private shop %u", p->dwShopID);
		return;
	}

	// If item move was successful, forward the packet to owner & shop
	TPrivateShopItemInfo* pItemInfo = pPrivateShop->MoveItem(p->wPos, p->wChangePos);
	if (pItemInfo)
	{
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(TPacketPrivateShopItemMove));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));
		pPeer->Encode(p, sizeof(TPacketPrivateShopItemMove));

		// Not needed to be sent twice if both player&shop are on the same peer
		if (pPeer != pShopPeer)
		{
			pShopPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(TPacketPrivateShopItemMove));
			pShopPeer->Encode(&bSubHeader, sizeof(BYTE));
			pShopPeer->Encode(p, sizeof(TPacketPrivateShopItemMove));
		}

		sys_log(0, "PRIVATE_SHOP: Moved item %u from pos %u -> %u on private shop %u", pItemInfo->TItem.dwID, p->wPos, p->wChangePos, p->dwShopID);
	}
	else
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_CANNOT_MOVE_ITEM;
		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot move item on private shop %u item pos %u", p->dwShopID, p->wPos);
	}
}

void CClientManager::PrivateShopItemCheckinRequest(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	TPacketGDPrivateShopItemCheckin* p = (TPacketGDPrivateShopItemCheckin*)c_szData;
	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_ITEM_CHECKIN_REQ;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(p->dwShopID);
	if (!pPrivateShop)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NO_SHOP;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop %u", p->dwShopID);
		return;
	}

	// Player cannot edit shop's content if its not set to modify state
	if (pPrivateShop->GetState() != STATE_MODIFY)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NOT_MODIFY_STATE;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));
		return;
	}

	CPeer* pShopPeer = GetPeer(pPrivateShop->GetShopPeerHandle());
	if (!pShopPeer)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_SHOP_NOT_AVAILABLE;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop peer for private shop %u", p->dwShopID);
		return;
	}

	TItemTable* pItemTable = GetItemTable(p->TItem.dwVnum);
	if (!pItemTable)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_ITEM_CHECKIN_FALSE_ITEM;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Unable find item proto table vnum at %u", p->TItem.dwVnum);
		return;
	}

	bool bFoundPos = false;
	int iFoundPos = -1;
	if (p->iPos > -1 && p->iPos < PRIVATE_SHOP_HOST_ITEM_MAX_NUM)
	{
		// Try to place an item on the position
		CGrid* pGrid = pPrivateShop->GetGridByPosition(p->iPos);
		WORD wGridPos = p->iPos - (p->iPos / PRIVATE_SHOP_PAGE_ITEM_MAX_NUM) * PRIVATE_SHOP_PAGE_ITEM_MAX_NUM;;

		if (pGrid->IsEmpty(wGridPos, 1, pItemTable->bSize))
		{
			bFoundPos = true;
			iFoundPos = p->iPos;
		}
	}

	// If position was not determined or selected position
	// is invalid, try to find an empty one
	if (!bFoundPos)
	{
		for (BYTE i = 0; i < PRIVATE_SHOP_PAGE_MAX_NUM; ++i)
		{
			CGrid* pGrid = pPrivateShop->GetGridByPage(i);
			iFoundPos = pGrid->FindBlank(1, pItemTable->bSize);

			// Position was found
			if (iFoundPos != -1)
			{
				iFoundPos += i * PRIVATE_SHOP_PAGE_ITEM_MAX_NUM;
				bFoundPos = true;
				break;
			}

		}
	}

	// Proceed with the item check in
	if (!bFoundPos)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NO_AVAILABLE_SPACE;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_log(0, "Cannot find suitable position for item checkin on private shop %u vnum %u", p->dwShopID, p->TItem.dwVnum);
		return;
	}

	p->TItem.wPos = iFoundPos;

	pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(TPacketGDPrivateShopItemCheckin));
	pPeer->Encode(&bSubHeader, sizeof(BYTE));
	pPeer->Encode(p, sizeof(TPacketGDPrivateShopItemCheckin));

	sys_log(0, "PRIVATE_SHOP: Item check-in request forwarded to game for private shop %u item %u vnum %u pos %u", p->dwShopID, p->TItem.dwID, p->TItem.dwVnum, iFoundPos);
}

void CClientManager::PrivateShopItemCheckoutRequest(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	TPacketGDPrivateShopItemCheckout* p = (TPacketGDPrivateShopItemCheckout*)c_szData;
	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_ITEM_CHECKOUT_REQ;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(p->dwPID);
	if (!pPrivateShop)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NO_SHOP;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop %u", p->dwPID);
		return;
	}

	// Player cannot edit shop's content if its not set to modify state
	if (pPrivateShop->GetState() != STATE_MODIFY)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NOT_MODIFY_STATE;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));
		return;
	}

	CPeer* pShopPeer = GetPeer(pPrivateShop->GetShopPeerHandle());
	if (!pShopPeer)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_SHOP_NOT_AVAILABLE;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop peer for private shop %u", p->dwPID);
		return;
	}

	// Check if the item exists
	TPrivateShopItemInfo* pItemInfo = pPrivateShop->GetItem(p->wSrcPos);
	if (!pItemInfo)
	{
		sys_err("Could not find item info on pos %u shop %u", p->wSrcPos, p->dwPID);

		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_BUY_RESULT_FALSE_ITEM;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));
		return;
	}

	bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_ITEM_CHECKOUT_REQ;
	TPacketDGPrivateShopItemCheckout subPacket{};
	subPacket.wSrcPos = p->wSrcPos;
	subPacket.TDstPos = p->TDstPos;

	pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(TPacketDGPrivateShopItemCheckout));
	pPeer->Encode(&bSubHeader, sizeof(BYTE));
	pPeer->Encode(&subPacket, sizeof(TPacketDGPrivateShopItemCheckout));

	sys_log(0, "PRIVATE_SHOP: Item check-out request forwarded to game for private shop %u item %u vnum %u pos %u", p->dwPID, p->TItem.dwID, p->TItem.dwVnum, p->wSrcPos);
}

void CClientManager::PrivateShopTitleChangeRequest(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	TPacketPrivateShopTitleChange* p = (TPacketPrivateShopTitleChange*)c_szData;
	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_TITLE_CHANGE;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(p->dwPID);
	if (!pPrivateShop)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NO_SHOP;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop %u", p->dwPID);
		return;
	}

	// Player cannot edit shop's content if its not set to modify state
	if (pPrivateShop->GetState() != STATE_MODIFY)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NOT_MODIFY_STATE;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));
		return;
	}

	CPeer* pShopPeer = GetPeer(pPrivateShop->GetShopPeerHandle());
	if (!pShopPeer)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_SHOP_NOT_AVAILABLE;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop peer for private shop %u", p->dwPID);
		return;
	}

	// Change title on the SQL
	pPrivateShop->ChangeTitle(p->szTitle);

	// Change title on the owner's interface and on the shop
	pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(TPacketPrivateShopTitleChange));
	pPeer->Encode(&bSubHeader, sizeof(BYTE));
	pPeer->Encode(p, sizeof(TPacketPrivateShopTitleChange));

	if (pPeer != pShopPeer)
	{
		pShopPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(TPacketPrivateShopTitleChange));
		pShopPeer->Encode(&bSubHeader, sizeof(BYTE));
		pShopPeer->Encode(p, sizeof(TPacketPrivateShopTitleChange));
	}

	sys_log(0, "PRIVATE_SHOP: Title changed on private shop %u (%s)", p->dwPID, p->szTitle);
}

void CClientManager::PrivateShopWarpRequest(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	TPacketGDPrivateShopWarpReq* p = (TPacketGDPrivateShopWarpReq*)c_szData;
	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_WARP;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(p->dwPID);
	if (!pPrivateShop)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NO_SHOP;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop %u", p->dwPID);
		return;
	}

	CPeer* pShopPeer = GetPeer(pPrivateShop->GetShopPeerHandle());
	if (!pShopPeer)
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NO_SHOP;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Cannot find private shop peer %u", p->dwPID);
		return;
	}

	if (pShopPeer->GetChannel() != p->bChannel ||
		pShopPeer->GetListenPort() != p->wListenPort ||
		!pShopPeer->GetMap(p->dwMapIndex))
	{
		bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_NO_SHOP;

		pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE));
		pPeer->Encode(&bSubHeader, sizeof(BYTE));

		sys_err("Data mismatch for private shop %u", p->dwPID);
		return;
	}

	// Start the warp process
	TPacketDGPrivateShopWarp subPacket {};
	subPacket.lAddr = inet_addr(pShopPeer->GetPublicIP());
	subPacket.wListenPort = pShopPeer->GetListenPort();

	pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(TPacketDGPrivateShopWarp));
	pPeer->Encode(&bSubHeader, sizeof(BYTE));
	pPeer->Encode(&subPacket, sizeof(TPacketDGPrivateShopWarp));
}

void CClientManager::PrivateShopSlotUnlockRequest(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	TPacketGDPrivateShopSlotUnlockReq* p = (TPacketGDPrivateShopSlotUnlockReq*)c_szData;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(p->dwPID);
	if (!pPrivateShop)
	{
		sys_err("Could not find shop with id %d", p->dwPID);
		return;
	}
	int iAvailableSlots = PRIVATE_SHOP_LOCKED_SLOT_MAX_NUM - pPrivateShop->GetUnlockedSlots();
	if (iAvailableSlots <= 0)
	{
		sys_err("Shop %d tried to unlock a slot with %d available unlocked slots", p->dwPID, iAvailableSlots);
		return;
	}

	// Prevent overflow
	WORD wIncreasedSlotCount = std::min<WORD>(p->wCount, iAvailableSlots);
	pPrivateShop->IncreaseUnlockedSlots(wIncreasedSlotCount);

	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_UNLOCK_SLOT_RES;

	TPacketGDPrivateShopSlotUnlockRes subPacket{};
	subPacket.dwShopID = p->dwPID;
	subPacket.wUnlockedSlots = pPrivateShop->GetUnlockedSlots();

	pPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, dwHandle, sizeof(BYTE) + sizeof(TPacketGDPrivateShopSlotUnlockRes));
	pPeer->Encode(&bSubHeader, sizeof(BYTE));
	pPeer->Encode(&subPacket, sizeof(TPacketGDPrivateShopSlotUnlockRes));

	CPeer* pShopPeer = GetPeer(pPrivateShop->GetShopPeerHandle());
	if (pShopPeer && pShopPeer != pPeer)
	{
		pShopPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(TPacketGDPrivateShopSlotUnlockRes));
		pShopPeer->Encode(&bSubHeader, sizeof(BYTE));
		pShopPeer->Encode(&subPacket, sizeof(TPacketGDPrivateShopSlotUnlockRes));
	}

	sys_log(0, "PRIVATE_SHOP: Increased available space by %u slots on private shop %u", p->wCount, pPrivateShop->GetOwner());
}

void CClientManager::PrivateShopBuy(CPeer* pPeer, DWORD dwHandle, const char* c_szData)
{
	TPacketGDPrivateShopBuy* p = (TPacketGDPrivateShopBuy*)c_szData;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(p->TItem.dwOwner);
	if (!pPrivateShop)
	{
		sys_err("Could not find shop with id %d", p->TItem.dwOwner);
		return;
	}

	TPrivateShopItemInfo* pItemInfo = pPrivateShop->GetItem(p->TItem.wPos);
	if (!pItemInfo)
	{
		sys_err("Could not find item info on pos %u shop %u", p->TItem.wPos, p->TItem.dwOwner);
		return;
	}

	// Create sale object
	TPrivateShopSale TSale{};
	TSale.dwOwner = p->TItem.dwOwner;
	TSale.dwCustomer = p->dwCustomer;
	strlcpy(TSale.szCustomerName, p->szCustomerName, sizeof(TSale.szCustomerName));
	TSale.tTime = p->tTime;
	std::memcpy(&TSale.TItem, &p->TItem, sizeof(TSale.TItem));

	// Lets assign a hashed ID for this specific sale
	TSale.AssignID();

	// Item was already removed on game core
	// Send notification to the private shop owner if online
	CPeer* pOwnerPeer = GetPeer(pPrivateShop->GetOwnerPeerHandle());
	if (pOwnerPeer)
	{
		// Balance Update
		BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_SALE_UPDATE;

		pOwnerPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, pPrivateShop->GetOwnerHandle(), sizeof(BYTE) + sizeof(TPrivateShopSale));
		pOwnerPeer->Encode(&bSubHeader, sizeof(BYTE));
		pOwnerPeer->Encode(&TSale, sizeof(TPrivateShopSale));
	}

	pPrivateShop->UpdateBalance(pItemInfo->TItem.TPrice.llGold, pItemInfo->TItem.TPrice.dwCheque);
	pPrivateShop->SetSale(TSale);
	pPrivateShop->RemoveItem(p->TItem.wPos);

	// Remove the item if shop is not spawned on the same core as the item was bought on
	if (pPeer->GetHandle() != pPrivateShop->GetShopPeerHandle())
	{
		CPeer* pShopPeer = GetPeer(pPrivateShop->GetShopPeerHandle());
		if (pShopPeer)
		{
			BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_REMOVE_ITEM;
			DWORD dwShopID = pPrivateShop->GetOwner();

			pShopPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(DWORD) + sizeof(WORD));
			pShopPeer->Encode(&bSubHeader, sizeof(BYTE));
			pShopPeer->Encode(&dwShopID, sizeof(DWORD));
			pShopPeer->Encode(&p->TItem.wPos, sizeof(WORD));
		}
		else
		{
			sys_err("Cannot find private shop peer %u", p->TItem.dwOwner);
		}
	}

	sys_log(0, "PRIVATE_SHOP:	Item bought from private shop %u pos %u gold %lld won %u by pid %u", pPrivateShop->GetOwner(), p->TItem.wPos, p->TItem.TPrice.llGold, p->TItem.TPrice.dwCheque, p->dwCustomer);

	// No items left but stash still exists
	if (!pPrivateShop->GetItemCount() && (pPrivateShop->GetGold() || pPrivateShop->GetCheque()))
	{
		sys_log(0, "PRIVATE_SHOP: Despawning item empty private shop %u by item sale", p->TItem.dwOwner);
		PrivateShopGameDespawn(pPrivateShop);
	}

	// No items and no stash
	else if (!pPrivateShop->GetItemCount() && !pPrivateShop->GetGold() && !pPrivateShop->GetCheque())
	{
		sys_log(0, "PRIVATE_SHOP: Destroying empty private shop %u by item sale", p->TItem.dwOwner);
		PrivateShopGameDespawn(pPrivateShop);
		PrivateShopDestroy(pPrivateShop);
	}
}

void CClientManager::PrivateShopFailedBuy(const char* c_szData)
{
	TPacketGDPrivateShopFailedBuy* p = (TPacketGDPrivateShopFailedBuy*)c_szData;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(p->dwShopID);
	if (!pPrivateShop)
	{
		sys_err("Could not find shop with id %u", p->dwShopID);
		return;
	}

	TPrivateShopItemInfo* pItemInfo = pPrivateShop->GetItem(p->wPos);
	if (!pItemInfo)
	{
		sys_err("Could not find item info on pos %u shop %u", p->wPos, p->dwShopID);
		return;
	}

	pItemInfo->bAvailable = true;

	sys_log(0, "PRIVATE_SHOP: Unlocking item on private shop %u pos %u", p->dwShopID, p->wPos);
}

void CClientManager::PrivateShopItemTransfer(TPlayerItem* pTItem)
{

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(pTItem->owner); // Currently this function is triggered by the shop owner whose id is the same as shop's one
	if (!pPrivateShop)
	{
		sys_err("Could not find shop with id %d", pTItem->owner);
		return;
	}

	pPrivateShop->RemoveItemByID(pTItem->id);

	if (!DeletePrivateShopItemCache(pTItem->id))
	{
		char szQuery[64];
		snprintf(szQuery, sizeof(szQuery), "DELETE FROM private_shop_item%s WHERE id=%u", GetTablePostfix(), pTItem->id);
		CDBManager::instance().AsyncQuery(szQuery);
	}

	PutItemCache(pTItem);

	sys_log(1, "PRIVATE_SHOP: Safe item transfer for private shop %u item %u", pTItem->owner, pTItem->id);
}

void CClientManager::PrivateShopItemDelete(const char* c_szData)
{
	TPacketGDPrivateShopItemDelete* p = (TPacketGDPrivateShopItemDelete*)c_szData;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(p->dwShopID);
	if (!pPrivateShop)
	{
		sys_err("Could not find shop with id %d", p->dwShopID);
		return;
	}

	if (!DeletePrivateShopItemCache(p->dwItemID))
	{
		char szQuery[64];
		snprintf(szQuery, sizeof(szQuery), "DELETE FROM private_shop_item%s WHERE id=%u", GetTablePostfix(), p->dwItemID);
		CDBManager::instance().AsyncQuery(szQuery);
	}

	pPrivateShop->RemoveItemByID(p->dwItemID);
	sys_log(0, "PRIVATE_SHOP: Deleting item on private shop %u item %u", p->dwShopID, p->dwItemID);
}

void CClientManager::PrivateShopItemExpire(const char* c_szData)
{
	TPacketGDPrivateShopItemExpire* p = (TPacketGDPrivateShopItemExpire*)c_szData;

	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(p->dwShopID);
	if (!pPrivateShop)
	{
		sys_err("Could not find shop with id %d", p->dwShopID);
		return;
	}

	pPrivateShop->RemoveItem(p->wPos);

	CPeer* pOwnerPeer = GetPeer(pPrivateShop->GetOwnerPeerHandle());
	if (pOwnerPeer)
	{
		BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_ITEM_EXPIRE;

		pOwnerPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, pPrivateShop->GetOwnerHandle(), sizeof(BYTE) + sizeof(WORD));
		pOwnerPeer->Encode(&bSubHeader, sizeof(BYTE));
		pOwnerPeer->Encode(&p->wPos, sizeof(WORD));
	}

	sys_log(0, "PRIVATE_SHOP: Removing expired item on private shop %u pos %u", p->dwShopID, p->wPos);

	// No items left but stash still exists
	if (!pPrivateShop->GetItemCount() && (pPrivateShop->GetGold() || pPrivateShop->GetCheque()))
	{
		sys_log(0, "PRIVATE_SHOP: Despawning item empty private shop %u by item expire", p->dwShopID);
		PrivateShopGameDespawn(pPrivateShop);
	}

	// No items and no stash
	else if (!pPrivateShop->GetItemCount() && !pPrivateShop->GetGold() && !pPrivateShop->GetCheque())
	{
		sys_log(0, "PRIVATE_SHOP: Destroying empty private shop %u by item expire", p->dwShopID);
		PrivateShopGameDespawn(pPrivateShop);
		PrivateShopDestroy(pPrivateShop);
	}
}

void CClientManager::PrivateShopPremiumTimeUpdate(const char* c_szData)
{
	TPacketGDPrivateShopPremiumTimeUpdate* p = (TPacketGDPrivateShopPremiumTimeUpdate*)c_szData;

	CLoginData* pLoginData = GetLoginDataByAID(p->dwAID);

	if (!pLoginData)
	{
		sys_err("Could not find login data with account id %d", p->dwAID);
		return;
	}

	time_t tNewPremiumTime = p->tPremiumTime + time(0);

	pLoginData->SetPremium(PREMIUM_PRIVATE_SHOP, tNewPremiumTime);

	char szQuery[512];

	for (int i = 1; i <= PLAYER_PER_ACCOUNT; ++i)
	{
		sprintf(szQuery, "SELECT pid%u FROM player_index%s WHERE id=%u", i, GetTablePostfix(), p->dwAID);
		std::unique_ptr<SQLMsg> pMsg(CDBManager::instance().DirectQuery(szQuery));
		MYSQL_RES* pRes = pMsg->Get()->pSQLResult;

		if (!pRes)
		{
			sys_err("Failed to fetch player index %u result for account %u", i, p->dwAID);
			return;
		}

		auto iRow = mysql_num_rows(pRes);
		if (iRow <= 0)
		{
			sys_err("Failed to fetch player index %u columns for account %u", i, p->dwAID);
			return;
		}

		MYSQL_ROW row = mysql_fetch_row(pRes);
		DWORD dwPID = 0;
		str_to_number(dwPID, row[0]);

		LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(dwPID);
		if (pPrivateShop)
		{
			pPrivateShop->UpdatePremiumTime(tNewPremiumTime);
			sys_log(0, "PRIVATE_SHOP: Updating premium time for private shop %u -> %u", p->dwPID, tNewPremiumTime);
		}
	}

	sprintf(szQuery, "UPDATE account SET premium_privateshop_expire = FROM_UNIXTIME(%u) WHERE id = %u LIMIT 1", tNewPremiumTime, p->dwAID);
	sys_log(0, "PREMIUM PRIVATE SHOP CHANGED ACCOUNT %u DURATION %u", p->dwAID, tNewPremiumTime);
	CDBManager::Instance().AsyncQuery(szQuery, SQL_ACCOUNT);


	sys_log(0, "PRIVATE_SHOP: Updating premium time for aid %u -> %u", p->dwAID, tNewPremiumTime);
}

void CClientManager::PrivateShopStartPremiumEvent(DWORD dwPID)
{
	LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(dwPID);
	if (!pPrivateShop)
	{
		sys_err("Could not find shop with id %d", dwPID);
		return;
	}

	// Return if the private shop has already been listed for the premium time privilege
	for (auto it = m_list_privateShopPremium.begin(); it != m_list_privateShopPremium.end(); ++it)
	{
		LPPRIVATE_SHOP pPrivateShop = *it;

		if (pPrivateShop && pPrivateShop->GetOwner() == dwPID)
			return;
	}

	// Set non-premium private shop's premium time as 30 seconds to avoid despawning it on each warp ("a bit" longer for the potato pc users)
	if (pPrivateShop->GetPremiumTime() < time(0))
		pPrivateShop->SetPremiumTime(time(0) + 30);

	// Update owner's peer to null
	pPrivateShop->BindOwnerPeerHandle(0);
	pPrivateShop->SetOwnerHandle(0);

	m_list_privateShopPremium.emplace_back(pPrivateShop);

	sys_log(0, "PRIVATE_SHOP: Starting premium time event on private shop %u", dwPID);
}

void CClientManager::PrivateShopEndPremiumEvent(DWORD dwPID)
{
	for (auto it = m_list_privateShopPremium.begin(); it != m_list_privateShopPremium.end(); ++it)
	{
		LPPRIVATE_SHOP pPrivateShop = *it;

		if (pPrivateShop && pPrivateShop->GetOwner() == dwPID)
		{
			m_list_privateShopPremium.erase(it);
			sys_log(0, "PRIVATE_SHOP: Ending premium time event on private shop %u", dwPID);
			return;
		}
	}
}

void CClientManager::UpdatePrivateShopPremiumEvent()
{
	for (auto it = m_list_privateShopPremium.begin(); it != m_list_privateShopPremium.end(); )
	{
		LPPRIVATE_SHOP pPrivateShop = *it;

		if (!pPrivateShop)
		{
			m_list_privateShopPremium.erase(it++);
			continue;
		}

		if (pPrivateShop->GetPremiumTime() < time(0))
		{
			DWORD dwShopID = pPrivateShop->GetOwner();

			CPeer* pShopPeer = GetPeer(pPrivateShop->GetShopPeerHandle());
			if (pShopPeer)
			{
				BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_DESPAWN;

				pShopPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(DWORD));
				pShopPeer->Encode(&bSubHeader, sizeof(BYTE));
				pShopPeer->Encode(&dwShopID, sizeof(DWORD));
			}

			m_list_privateShopPremium.erase(it++);
			DeletePrivateShop(dwShopID);

			sys_log(0, "PRIVATE_SHOP: Premium time event ended on private shop %u", dwShopID);
			return;
		}

		++it;
	}
}

bool CClientManager::IsPrivateShopPremiumEvent(DWORD dwPID)
{
	for (auto it = m_list_privateShopPremium.begin(); it != m_list_privateShopPremium.end(); ++it)
	{
		LPPRIVATE_SHOP pPrivateShop = *it;

		if (pPrivateShop && pPrivateShop->GetOwner() == dwPID)
		{
			return true;
		}
	}

	return false;
}

// Called upon last item checkout / stash withdrawal
void CClientManager::PrivateShopDestroy(LPPRIVATE_SHOP pPrivateShop)
{
	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_CLOSE;

	CPeer* pOwnerPeer = GetPeer(pPrivateShop->GetOwnerPeerHandle());
	DWORD dwPID = pPrivateShop->GetOwner();

	// Shop instance is already destroyed on the game core
	if (pOwnerPeer)
	{
		pOwnerPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, pPrivateShop->GetOwnerHandle(), sizeof(BYTE));
		pOwnerPeer->Encode(&bSubHeader, sizeof(BYTE));
	}

	// Delete private shop cache and SQL record
	if (GetPrivateShopCache(dwPID))
	{
		DeletePrivateShopCache(dwPID);
	}
	else
	{
		char szQuery[64];
		snprintf(szQuery, sizeof(szQuery), "DELETE FROM private_shop%s WHERE owner_id=%u", GetTablePostfix(), dwPID);

		CDBManager::instance().AsyncQuery(szQuery);
	}

	// Delete private shop item cache and SQL record
	if (GetPrivateShopItemCacheSet(dwPID))
	{
		DeletePrivateShopItemCacheSet(dwPID);
	}
	else
	{
		char szQuery[64];
		snprintf(szQuery, sizeof(szQuery), "DELETE FROM private_shop_item%s WHERE owner_id=%u", GetTablePostfix(), dwPID);

		CDBManager::instance().AsyncQuery(szQuery);
	}

	DWORD dwShopID = pPrivateShop->GetOwner();

	// @note: Shouldn't happen as we already despawn the private shop by PrivateShopGameDespawn
	{
		if (IsPrivateShopPremiumEvent(dwShopID))
			PrivateShopEndPremiumEvent(dwShopID);

		if (GetPrivateShop(dwPID))
			DeletePrivateShop(dwShopID);
	}

	sys_log(0, "PRIVATE_SHOP: Destroyed private shop %u", dwShopID);
}

// Called upon last item checkout / last item sale
void CClientManager::PrivateShopGameDespawn(LPPRIVATE_SHOP pPrivateShop)
{
	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_STATE_UPDATE;

	CPeer* pOwnerPeer = GetPeer(pPrivateShop->GetOwnerPeerHandle());
	CPeer* pShopPeer = GetPeer(pPrivateShop->GetShopPeerHandle());

	TPacketDGPrivateShopStateUpdate subPacket {};
	subPacket.dwPID = pPrivateShop->GetOwner();;
	subPacket.bState = STATE_CLOSED;

	pPrivateShop->ChangeState(STATE_CLOSED);

	if (pOwnerPeer)
	{
		pOwnerPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, pPrivateShop->GetOwnerHandle(), sizeof(BYTE) + sizeof(TPacketDGPrivateShopStateUpdate));
		pOwnerPeer->Encode(&bSubHeader, sizeof(BYTE));
		pOwnerPeer->Encode(&subPacket, sizeof(TPacketDGPrivateShopStateUpdate));

		sys_log(0, "PRIVATE_SHOP: Sending despawn update to owner %u", subPacket.dwPID);
	}
	else
	{
		// Flush private shop data immediately and delete the instance if owner is offline
		if (GetPrivateShopCache(pPrivateShop->GetOwner()))
			FlushPrivateShopCache(pPrivateShop->GetOwner());

		if (GetPrivateShopItemCacheSet(pPrivateShop->GetOwner()))
			FlushPrivateShopItemCacheSet(pPrivateShop->GetOwner());

		if (IsPrivateShopPremiumEvent(pPrivateShop->GetOwner()))
			PrivateShopEndPremiumEvent(pPrivateShop->GetOwner());

		DeletePrivateShop(pPrivateShop->GetOwner());

		sys_log(0, "PRIVATE_SHOP: Deleting despawned private shop %u", subPacket.dwPID);
	}

	if (pShopPeer && pShopPeer != pOwnerPeer)
	{
		pShopPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(TPacketDGPrivateShopStateUpdate));
		pShopPeer->Encode(&bSubHeader, sizeof(BYTE));
		pShopPeer->Encode(&subPacket, sizeof(TPacketDGPrivateShopStateUpdate));
	}
}

void CClientManager::PrivateShopGameSpawn(LPPRIVATE_SHOP pPrivateShop)
{
	CPeer* pShopPeer = GetPeer(pPrivateShop->GetShopPeerHandle());
	if (!pShopPeer)
	{
		sys_err("Cannot find peer for private shop %u", pPrivateShop->GetOwner());
		return;
	}

	if (!pPrivateShop->GetItemCount())
	{
		sys_log(0, "PRIVATE_SHOP: Skipping spawning of empty private shop %u", pPrivateShop->GetOwner());
		return;
	}

	BYTE bSubHeader = PRIVATE_SHOP_DG_SUBHEADER_SPAWN;
	WORD wCount = pPrivateShop->GetItemCount();

	pShopPeer->EncodeHeader(HEADER_DG_PRIVATE_SHOP, 0, sizeof(BYTE) + sizeof(TPrivateShop) + sizeof(WORD) + wCount * sizeof(TPlayerPrivateShopItem));
	pShopPeer->Encode(&bSubHeader, sizeof(BYTE));
	pShopPeer->Encode(&pPrivateShop->GetTable(), sizeof(TPrivateShop));
	pShopPeer->Encode(&wCount, sizeof(WORD));

	auto& itemContainer = pPrivateShop->GetItemContainer();
	for (const auto& c_rShopItem : itemContainer)
		pShopPeer->Encode(&c_rShopItem.TItem, sizeof(TPlayerPrivateShopItem));

	sys_log(0, "PRIVATE_SHOP: Spawning private shop %u premium_time %u", pPrivateShop->GetOwner(), pPrivateShop->GetPremiumTime());
}

bool CClientManager::PrivateShopFetchData(DWORD dwShopID, TPrivateShop& rTable, std::vector<TPlayerPrivateShopItem>& c_vec_shopItem)
{
	// Private Shop Information
	{
		std::unique_ptr<SQLMsg> pMsg(CDBManager::instance().DirectQuery(GetPrivateShopQuery(dwShopID)));
		MYSQL_RES* pRes = pMsg->Get()->pSQLResult;

		if (!CreatePrivateShopTableFromRes(pRes, rTable))
			return false;
	}
	// Items
	{
		std::unique_ptr<SQLMsg> pMsg(CDBManager::instance().DirectQuery(GetPrivateShopItemQuery(dwShopID)));
		MYSQL_RES* pRes = pMsg->Get()->pSQLResult;

		CreatePrivateShopItemTableFromRes(pRes, &c_vec_shopItem, dwShopID);
	}

	return true;
}

void CClientManager::PrivateShopPeerSpawn(CPeer* pPeer)
{
	size_t sCount = 0;

	/*
		SPAWNING PRIVATE SHOPS WITH OWNERS BEING ONLINE
		GAME CORE CRASH/SHUTDOWN PROCEDURE
	*/
	if (m_map_privateShop.size())
	{
		sys_log(0, "RESPAWNING UNAVAILABLE PRIVATE SHOPS (CHANNEL %u PORT %u)", pPeer->GetChannel(), pPeer->GetListenPort());

		for (auto it = m_map_privateShop.begin(); it != m_map_privateShop.end(); )
		{
			LPPRIVATE_SHOP pPrivateShop = it->second.get();

			// Skip private shops that were not affected by this core crash/shutdown
			if (pPrivateShop->GetState() != STATE_UNAVAILABLE)
			{
				++it;
				continue;
			}

			// Delete shop data if owner is not online
			bool bDeleteShop = true;

			// Check if peer's port and channel matches private shop's
			if (pPrivateShop->GetTable().wPort == pPeer->GetListenPort()
				&& pPrivateShop->GetTable().bChannel == pPeer->GetChannel())

			{
				// Find if out if shop's owner is playing
				for (const auto& kv : m_map_pkLoginData)
				{
					CLoginData* pLoginData = kv.second;
					if (pPrivateShop->GetOwner() == pLoginData->GetLastPlayerID() && pLoginData->IsPlay())
					{
						bDeleteShop = false;

						// UNAVAILABLE state will be overwritten when assigning table
						if (pPrivateShop->GetItemCount())
						{
							pPrivateShop->ChangeState(STATE_OPEN);
							pPrivateShop->BindShopPeerHandle(pPeer->GetHandle());
							PrivateShopGameSpawn(pPrivateShop);
						}
						++sCount;
					}
				}
			}

			it = bDeleteShop ? m_map_privateShop.erase(it) : ++it;
		}
	}

	/*
		SPAWNING THE REST OF THE SHOPS (WITH PREMIUM TIME)
		DATABASE CRASH/SHUTDOWN PROCEDURE
	*/
	{
		static std::vector<DWORD> vec_suitablePrivateShops;

		// @note: Select only shops that are in either open or modify states and are placed on this specific game core
		char szQuery[256 + 1] {};

#ifdef ENABLE_PRIVATE_SHOP_PREMIUM_TIME
		snprintf(szQuery, sizeof(szQuery),
			"SELECT owner_id "
			"FROM private_shop%s "
			"WHERE premium_time > %u "
			"AND state+0 > %u AND channel=%u AND port=%u"
			, GetTablePostfix(), time(0), STATE_CLOSED, pPeer->GetChannel(), pPeer->GetListenPort());
#else
		snprintf(szQuery, sizeof(szQuery),
			"SELECT owner_id "
			"FROM private_shop%s "
			"WHERE state+0 > %u AND channel=%u AND port=%u"
			, GetTablePostfix(), STATE_CLOSED, pPeer->GetChannel(), pPeer->GetListenPort());
#endif

		sys_log(0, "RESPAWNING PRIVATE SHOPS (CHANNEL %u PORT %u)", pPeer->GetChannel(), pPeer->GetListenPort());

		std::unique_ptr<SQLMsg> pMsg(CDBManager::instance().DirectQuery(szQuery));
		MYSQL_RES* pRes = pMsg->Get()->pSQLResult;

		if (!pRes)
		{
			sys_log(0, "FAILED TO FETCH#1 PRIVATE SHOPS (CHANNEL %u PORT %u)", pPeer->GetChannel(), pPeer->GetListenPort());
			return;
		}

		auto iRow = mysql_num_rows(pRes);
		if (iRow <= 0)
		{
			sys_log(0, "NO PRIVATE SHOPS FOUND (CHANNEL %u PORT %u)", pPeer->GetChannel(), pPeer->GetListenPort());
			return;
		}

		vec_suitablePrivateShops.resize(iRow);

		for (int i = 0; i < iRow; ++i)
		{
			MYSQL_ROW row = mysql_fetch_row(pRes);
			DWORD& rShopID = vec_suitablePrivateShops.at(i);

			str_to_number(rShopID, row[0]);
		}

		TPrivateShop									privateShopTable{};
		static std::vector<TPlayerPrivateShopItem>		vec_privateShopItem;

		for (const auto& dwShopID : vec_suitablePrivateShops)
		{
			// Skip private shops that have already been respawned
			LPPRIVATE_SHOP pPrivateShop = GetPrivateShop(dwShopID);
			if (pPrivateShop && pPrivateShop->GetState() != STATE_UNAVAILABLE)
				continue;

			PrivateShopSpawn(dwShopID);
#ifdef ENABLE_PRIVATE_SHOP_PREMIUM_TIME
			PrivateShopStartPremiumEvent(dwShopID);
#endif
			++sCount;
		}
	}

	sys_log(0, "SPAWNED %u PRIVATE SHOPS (CHANNEL %u PORT %u)", sCount, pPeer->GetChannel(), pPeer->GetListenPort());
}

CPeer* CClientManager::GetPrivateShopPeer(BYTE bChannel, WORD wListenPort)
{
	for (const auto& pPeer : m_peerList)
	{
		if (pPeer->GetChannel() == bChannel && pPeer->GetListenPort() == wListenPort)
			return pPeer;
	}

	return nullptr;
}

TItemTable* CClientManager::GetItemTable(DWORD dwVnum)
{
	auto it = m_map_itemTableByVnum.find(dwVnum);
	if (it != m_map_itemTableByVnum.end())
		return it->second;

	for (const auto& pItemTable : m_vec_itemVnumRange)
	{
		if ((pItemTable->dwVnum < dwVnum) &&
			dwVnum < (pItemTable->dwVnum + pItemTable->dwVnumRange))
		{
			return pItemTable;
		}
	}

	return nullptr;
}
