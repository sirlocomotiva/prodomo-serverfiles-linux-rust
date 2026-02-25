#pragma once
#include <unordered_map>
#include <unordered_set>
#include "../common/tables.h"
#include "packet.h"

class CItem;
class CPrivateShop;
class TEMP_BUFFER;
class CPrivateShopManager : public singleton<CPrivateShopManager>
{
public:

	CPrivateShopManager() = default;
	~CPrivateShopManager() = default;

	void				Destroy();

	LPPRIVATE_SHOP		CreatePrivateShop(DWORD dwPID);
	LPPRIVATE_SHOP		GetPrivateShop(DWORD dwPID);
	LPPRIVATE_SHOP		GetPrivateShopByOwnerName(const char* c_szOwnerName);
	LPPRIVATE_SHOP		GetPrivateShopByVID(DWORD dwVID);
	bool				DeletePrivateShop(DWORD dwPID);
	DWORD				AllocVID() { return ++m_dwVIDCount; }

	bool				BuildPrivateShop(LPCHARACTER pShopOwner, const char* c_szTitle, DWORD dwPolyVnum, BYTE bTitleType, BYTE bPageCount, const std::vector<TPrivateShopItem>& c_vec_shopItem);
	void				BuildPrivateShopResult(DWORD dwPID, TPrivateShop* pPrivateShopTable, bool bSuccess);
	void				ClosePrivateShop(LPCHARACTER pShopOwner);

	void				SpawnPrivateShop(TPrivateShop* pTab, const std::vector<TPlayerPrivateShopItem>& c_vec_shopItem);
	void				DespawnPrivateShop(DWORD dwPID);

	bool				StopShopping(LPCHARACTER pShopViewer);

	void				ItemCheckin(LPCHARACTER pOwner, const TPlayerPrivateShopItem* c_pShopItem);
	void				ItemCheckout(LPCHARACTER pOwner, WORD wSrcPos, TItemPos TDstPos);

	bool				ItemTransaction(LPCHARACTER pCustomer, TPlayerPrivateShopItem* c_pShopItem);
	void				SendItemTransaction(const TPlayerPrivateShopItem* c_pShopItem, LPCHARACTER pCustomer);
	void				SendItemTransactionFailedResult(DWORD dwShopID, WORD wPos);
	void				SendItemTransfer(const TPlayerItem* c_pItem);
	void				SendItemExpire(LPITEM pItem);
	void				SendShopSearchUpdate(DWORD dwShopID, BYTE bState, int iSpecificItemPos = -1);

	void				AddSearchItem(LPITEM pItem);
	void				RemoveSearchItem(LPITEM pItem);
	void				SearchItem(LPDESC pCustomerDesc, TPrivateShopSearchFilter& rFilter, bool bUseFilter, TEMP_BUFFER* pBuf = nullptr);

	void				AddMarketItemPrice(DWORD dwVnum, TItemPrice& rItemPrice);
	TItemPrice*			GetMarketItemPrice(DWORD dwVnum);

	void				SendMarketItemPriceData(LPCHARACTER pChar);
	void				SendMarketItemPrice(LPCHARACTER pChar, DWORD dwVnum);

	typedef std::unordered_map<DWORD, std::unique_ptr<CPrivateShop> >	TPrivateShopMap;
	typedef std::unordered_map<DWORD, LPPRIVATE_SHOP>					TPrivateShopVIDMap;
	typedef std::unordered_map<DWORD, TItemPrice>						TMarketPriceMap;

	typedef std::unordered_set<LPITEM>					TItemList;
	typedef std::unordered_map<BYTE, TItemList>			TSubTypeItemMap;
	typedef std::unordered_map<BYTE, TSubTypeItemMap>	TTypeItemMap;

private:
	DWORD				m_dwVIDCount;
	TPrivateShopMap		m_map_privateShop;
	TPrivateShopVIDMap	m_map_privateShopVID;
	TTypeItemMap		m_map_searchItem;
	TMarketPriceMap		m_map_marketItemPrice;
};
