#pragma once
#include "../common/tables.h"
#include "../common/length.h"
#include "../libgame/grid.h"
#include "entity.h"
#include <unordered_set>

class CItem;

class CPrivateShop : public CEntity
{
public:
	CPrivateShop();
	virtual ~CPrivateShop();

	bool					Show(long lX, long lY, long lZ, long lMapIndex);
	bool					Initialize(const std::vector<TPlayerPrivateShopItem>& c_vec_shopItem, bool bRespawn = false);
	bool					TransferItems(LPCHARACTER pShopOwner);

	WORD					GetItemCount() { return m_map_shopItem.size(); }
	std::map<WORD, LPITEM>& GetItemContainer() { return m_map_shopItem; }
	void					CleanItems();
	bool					HasItemByID(DWORD itemID);

	LPITEM					GetItem(WORD wPos);
	void					RemoveItem(WORD wPos);
	bool					ItemCheckin(const TPlayerPrivateShopItem& c_rShopItem);
	void					ChangeItemPrice(WORD wPos, long long llGold, DWORD dwCheque);
	void					MoveItem(WORD wPos, WORD wChangePos);
	void					ChangeTitle(const char* c_szTitle);

	void			SetVID(DWORD dwVID) { m_dwVID = dwVID; }
	DWORD			GetVID() { return m_dwVID; }

	void			SetID(DWORD dwID) { m_dwID = dwID; }
	DWORD			GetID() { return m_dwID; }

	void			SetVnum(DWORD dwVnum) { m_dwVnum = dwVnum; }
	DWORD			GetVnum() { return m_dwVnum; }

	void			SetOwnerName(const char* c_szOwnerName) { m_strOwnerName = c_szOwnerName; }
	std::string		GetOwnerName() { return m_strOwnerName; }

	void			SetTitle(const char* c_szTitle) { m_strTitle = c_szTitle; }
	std::string		GetTitle() { return m_strTitle; }

	void			SetTitleType(BYTE bTitleType) { m_bTitleType = bTitleType; }
	BYTE			GetTitleType() { return m_bTitleType; }

	void			SetState(BYTE bState);
	BYTE			GetState() { return m_bState; }

	void			SetPageCount(BYTE bPageCount) { m_bPageCount = bPageCount; }
	BYTE			GetPageCount() { return m_bPageCount; }

	void			SetUnlockedSlots(WORD wUnlockedSlots);
	WORD			GetUnlockedSlots() { return m_wUnlockedSlots; }

	void			SetClosing(bool bIsClosing) { m_bIsClosing = bIsClosing; }
	bool			IsClosing() { return m_bIsClosing; }

	bool			AddShopViewer(LPCHARACTER pShopViewer);
	bool			RemoveShopViewer(LPCHARACTER pShopViewer);
	void			CleanShopViewers();
	void			BroadcastPacket(const void* c_pData, int iSize);

protected:
	void	EncodeInsertPacket(LPENTITY pEntity) override;
	void	EncodeRemovePacket(LPENTITY pEntity) override;

private:
	std::map<WORD, LPITEM>			m_map_shopItem;
	std::unordered_set<LPCHARACTER>	m_set_shopViewer;

	DWORD						m_dwID;
	DWORD						m_dwVID;
	DWORD						m_dwVnum;
	std::string					m_strOwnerName;
	std::string					m_strTitle;
	BYTE						m_bTitleType;
	BYTE						m_bState;
	BYTE						m_bPageCount;
	WORD						m_wUnlockedSlots;
	bool						m_bIsClosing;
};
