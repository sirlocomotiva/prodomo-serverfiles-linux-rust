#ifndef __INC_METIN_II_GAME_SHOP_MANAGER_H__
#define __INC_METIN_II_GAME_SHOP_MANAGER_H__

class CShop;
typedef class CShop * LPSHOP;

class CShopManager : public singleton<CShopManager>
{
public:
	typedef std::map<DWORD, CShop *> TShopMap;

public:
	CShopManager();
	virtual ~CShopManager();

	bool	Initialize(TShopTable * table, int size);
#if defined(ENABLE_RENEWAL_SHOPEX)
	bool	InitializeShopEX(TShopTable* table, int size);
#endif
	void	Destroy();

	LPSHOP	Get(DWORD dwVnum);
	LPSHOP	GetByNPCVnum(DWORD dwVnum);

	bool	StartShopping(LPCHARACTER pkChr, LPCHARACTER pkShopKeeper, int iShopVnum = 0);
	void	StopShopping(LPCHARACTER ch);

	void	Buy(LPCHARACTER ch, BYTE pos);
	void 	Sell(LPCHARACTER ch, WORD wCell, WORD bCount = 0, BYTE bType = 0);

	LPSHOP	CreatePCShop(LPCHARACTER ch, TShopItemTable * pTable, BYTE bItemCount);
	LPSHOP	FindPCShop(DWORD dwVID);
	void	DestroyPCShop(LPCHARACTER ch);
#ifdef OFFLINE_SHOP
public:
	bool	StartOfflineShop(DWORD id, bool onboot = false);
	bool	CreateOfflineShop(LPCHARACTER owner, const char *szSign, const std::vector<TShopItemTable*> pTable, DWORD days);
	LPSHOP	CreateNPCShop(LPCHARACTER ch, std::vector<TShopItemTable *> map_shop);
	DWORD	GetShopCount() { return m_map_pkShop.size(); }
#endif

private:
	TShopMap	m_map_pkShop;
	TShopMap	m_map_pkShopByNPCVnum;
	TShopMap	m_map_pkShopByPC;

#ifdef ENABLE_SHOP_SEARCH
public:
	TShopMap GetShops() const { return m_map_pkShopByPC; }
#endif

#ifdef ENABLE_DISTANCE_SHOPPING
	bool	StartDistanceShopping(LPCHARACTER pkChr, int iShopVnum = 0);
	void	StopDistanceShopping(LPCHARACTER ch);
#endif

	bool	ReadShopTableEx(const char* stFileName);
};

#endif
