// vim:ts=8 sw=4
#ifndef __INC_DB_CACHE_H__
#define __INC_DB_CACHE_H__

#include "../common/cache.h"

class CItemCache : public BaseCache<TPlayerItem>
{
    public:
	CItemCache();
	virtual ~CItemCache();

	void Delete();
	virtual void OnFlush();
};

class CPlayerTableCache : public BaseCache<TPlayerTable>
{
    public:
	CPlayerTableCache();
	virtual ~CPlayerTableCache();

	virtual void OnFlush();

	DWORD GetLastUpdateTime() { return m_lastUpdateTime; }
};
// MYSHOP_PRICE_LIST

class CItemPriceListTableCache : public BaseCache< TItemPriceListTable >
{
    public:

	/// Constructor

	CItemPriceListTableCache(void);
	virtual ~CItemPriceListTableCache();


	void	UpdateList(const TItemPriceListTable* pUpdateList);

	virtual void	OnFlush(void);

    private:

	static const int	s_nMinFlushSec;		///< Minimum cache expire time
};

#ifdef __PREMIUM_PRIVATE_SHOP__
class CPrivateShopCache : public BaseCache<TPrivateShop>
{
public:
	CPrivateShopCache();
	virtual ~CPrivateShopCache();
	void Delete();
	virtual void OnFlush();

	DWORD GetLastUpdateTime() { return m_lastUpdateTime; }
};

class CPrivateShopItemCache : public BaseCache<TPlayerPrivateShopItem>
{
public:
	CPrivateShopItemCache();
	virtual ~CPrivateShopItemCache();
	void Delete();
	virtual void OnFlush();

	DWORD GetLastUpdateTime() { return m_lastUpdateTime; }
};

class CPrivateShopSaleCache : public BaseCache<TPrivateShopSale>
{
public:
	CPrivateShopSaleCache();
	virtual ~CPrivateShopSaleCache();
	void Delete();
	virtual void OnFlush();

	DWORD GetLastUpdateTime() { return m_lastUpdateTime; }
};
#endif

#endif
