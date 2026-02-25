#pragma once
#include "char.h"
enum
{
    SHOP_SEARCH_TYPE_EMPTY,
    SHOP_SEARCH_TYPE_FIND,
    SHOP_SEARCH_TYPE_BUY,
    SHOP_SEARCH_TYPE_BUY_PREMIUM
};
class CShopSearchHelper
{
public:

    static int GetType (DWORD vnum)
    {

        int ret=SHOP_SEARCH_TYPE_EMPTY;
        switch (vnum)
        {
            case 60004:
                ret = SHOP_SEARCH_TYPE_FIND;
                break;
            case 60005:
                ret = SHOP_SEARCH_TYPE_BUY;
                break;
            case 60006:
                ret = SHOP_SEARCH_TYPE_BUY_PREMIUM;
                break;
        }

        return ret;
    }
    static void Open (DWORD vnum,LPCHARACTER ch)
    {
        ch->ChatPacket(CHAT_TYPE_COMMAND,"OpenShopSearch %d",GetType(vnum));
    }
    static void Close (DWORD vnum,LPCHARACTER ch)
    {
        ch->ChatPacket(CHAT_TYPE_COMMAND,"CloseShopSearch %d",GetType(vnum));
    }
    static bool IsGlass (DWORD vnum)
    {
        return GetType(vnum)!=SHOP_SEARCH_TYPE_EMPTY;
    }
};