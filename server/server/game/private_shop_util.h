#pragma once
#include "../common/length.h"
#include "../common/tables.h"
#include "packet.h"

#include <sstream>
#include <iomanip>
#include <locale>

template<class T>
std::string format_number(T value)
{
	struct Numpunct : public std::numpunct<char>
	{
	protected:
		virtual char do_thousands_sep() const { return '.'; }
		virtual std::string do_grouping() const { return "\03"; }
	};

	std::stringstream ss{};
	ss.imbue({ std::locale(), new Numpunct });
	ss << std::fixed << value;

	return ss.str();
}

extern bool CanBuildPrivateShop(LPCHARACTER ch);
extern bool CheckTradeWindows(LPCHARACTER ch);
extern int GetEmptyInventory(LPCHARACTER pChar, LPITEM pItem);

/*		LPITEM -> TPlayerItem		*/
extern void CopyItemData(LPITEM pItem, TPlayerItem& rTargetTable);

/*		LPITEM -> TPlayerPrivateShopItem	*/
extern void CopyItemData(LPITEM pItem, TPlayerPrivateShopItem& rTargetTable);

/*		LPITEM -> TPrivateShopItemData	*/
extern void CopyItemData(LPITEM pItem, TPrivateShopItemData& rTargetTable);

/*		LPITEM -> TPrivateShopSearchData	*/
extern void CopyItemData(LPITEM pItem, TPrivateShopSearchData& rTargetTable);

/*		TPlayerPrivateShopItem -> LPITEM	*/
extern void CopyItemData(const TPlayerPrivateShopItem& rSourceTable, LPITEM pItem, LPCHARACTER pOwner = nullptr);

/*		TPlayerPrivateShopItem -> TPrivateShopItemData		*/
extern void CopyItemData(const TPlayerPrivateShopItem& rSourceTable, TPrivateShopItemData& rTargetTable);

/*		TPrivateShopSale -> TPrivateShopSaleData		*/
extern void CopyItemData(const TPrivateShopSale& rSourceTable, TPrivateShopSaleData& rTargetTable);

/*		TPrivateShopSale -> TPrivateShopItemData		*/
extern void CopyItemData(const TPrivateShopSale& rSourceTable, TPrivateShopItemData& rTargetTable);
