// @ Grimmjock

#include "stdafx.h"
#include <boost/algorithm/string.hpp>
#include "constants.h"
#include "item.h"
#include "item_manager.h"
#include "unique_item.h"
#include "char.h"
#include "config.h"
#include "text_file_loader.h"
#include "item_stack_attribute.h"
#include "char_manager.h"

ITEM_STACK_ATTRIBUTE::ITEM_STACK_ATTRIBUTE()
{
}

ITEM_STACK_ATTRIBUTE::~ITEM_STACK_ATTRIBUTE()
{
	map_stack_attr.clear();
}

// PATH_LOCALE_STACK_ATTRIBUTE_CONFIGURATION
static const char* c_pszPath = "locale/germany/item_stack_attribute.txt"; // Edit the path.

void ITEM_STACK_ATTRIBUTE::Initialize()
{
	if (g_bAuthServer)
		return;

	ReadItemStackAttrFile(c_pszPath);
}

bool ITEM_STACK_ATTRIBUTE::ReadItemStackAttrFile(const char * c_pszFileName)
{
	// Clear map Attributes
	map_stack_attr.clear();
	
	CTextFileLoader loader;

	if (!loader.Load(c_pszFileName))
		return false;
	
	std::string stName;
	
	for (DWORD i = 0; i < loader.GetChildNodeCount(); ++i)
	{
		loader.SetChildNode(i);
		loader.GetCurrentNodeName(&stName);
		
		int iVnum = 0;
		
		if (!loader.GetTokenInteger("vnum", &iVnum))
		{
			sys_err("ReadItemStackAttrFile : Syntax error %s : no vnum, node %s", c_pszFileName, stName.c_str());
			loader.SetParentNode();
			return false;
		}
		
		SInfoItem sNewItem;
		sNewItem.ItemVnum = iVnum;
		
		TTokenVector * pTok;
		
		for (int k = 1; k < 4; ++k)
		{
			char buf[4];
			snprintf(buf, sizeof(buf), "%d", k);
	
			if (loader.GetTokenVector(buf, &pTok))
			{
				int GetBonusVnum = 0;
				str_to_number(GetBonusVnum, pTok->at(0).c_str());
				
				int TypeBonus = 0;
				str_to_number(TypeBonus, pTok->at(1).c_str());
				
				int BonusType = 0;
				str_to_number(BonusType, pTok->at(2).c_str());

				int BonusRewardProgress = 0;
				str_to_number(BonusRewardProgress, pTok->at(3).c_str());
	
				int BonusMaxStack = 0;
				str_to_number(BonusMaxStack, pTok->at(4).c_str());
	
				sNewItem.GetBonusVnum[k - 1] = GetBonusVnum;
				sNewItem.TypeBonus[k - 1] = TypeBonus;
				sNewItem.BonusType[k - 1] = BonusType;
				sNewItem.BonusRewardProgress[k - 1] = BonusRewardProgress;
				sNewItem.BonusMaxStack[k - 1] = BonusMaxStack;

				continue;
			}
	
			break;
		}
		
		map_stack_attr.emplace(iVnum, sNewItem);
		
		loader.SetParentNode();
	}
	
	return true;
}

/// iVnumSet = It's like for iType(monster,stone,boss) GetRaceNum() and for (item_use) item->GetVnum()
void ITEM_STACK_ATTRIBUTE::StackAttributeByWearIndex(LPCHARACTER ch, int iType, int iVnumSet)
{
	if (ch == NULL)
		return;
	
	/// SYNTAX :: AddStackAttribute(ch, iType, HERE ADD WEAR FROM LENGTH.H (enum EWearPositions), iVnumSet);
	AddStackAttribute(ch, iType, WEAR_COSTUME_WEAPON, iVnumSet);
	AddStackAttribute(ch, iType, WEAR_COSTUME_PET, iVnumSet);
}

void ITEM_STACK_ATTRIBUTE::AddStackAttribute(LPCHARACTER ch, int iType, DWORD bCell, int iVnumSet, int iValue)
{
	if (ch == NULL)
		return;
	
	LPITEM pItem = ch->GetWear(bCell);
	
	if (pItem == NULL)
		return;
	
	if (pItem)
	{
		/////////////////////// ITEM NOT FOUND IN MAP ///////////////////////
		if (map_stack_attr.find(pItem->GetVnum()) == map_stack_attr.end())
			return;
		/////////////////////// ITEM NOT FOUND IN MAP ///////////////////////
		
		if (pItem->GetVnum() == map_stack_attr[pItem->GetVnum()].ItemVnum)
		{
			if (map_stack_attr[pItem->GetVnum()].GetBonusVnum[0] == iVnumSet || map_stack_attr[pItem->GetVnum()].GetBonusVnum[0] == 0)
				SetStackAttributeIndex(ch, bCell, iType, 1, iValue);
			
			if (map_stack_attr[pItem->GetVnum()].GetBonusVnum[1] == iVnumSet || map_stack_attr[pItem->GetVnum()].GetBonusVnum[1] == 0)
				SetStackAttributeIndex(ch, bCell, iType, 2, iValue);
			
			if (map_stack_attr[pItem->GetVnum()].GetBonusVnum[2] == iVnumSet || map_stack_attr[pItem->GetVnum()].GetBonusVnum[2] == 0)
				SetStackAttributeIndex(ch, bCell, iType, 3, iValue);
		}
	}
}

void ITEM_STACK_ATTRIBUTE::SetStackAttributeIndex(LPCHARACTER ch, DWORD bCell, int iType, int iBonus, int iValue)
{
	if (ch == NULL)
		return;
	
	LPITEM pItem = ch->GetWear(bCell);
	
	if (pItem == NULL)
		return;
	
	if (map_stack_attr.find(pItem->GetVnum()) == map_stack_attr.end())
		return;
	
	// PREVENT_UNEQUIP_BONUS_HACK
	ch->SetUseItemStackAttrFlood(thecore_pulse());
	// PREVENT_UNEQUIP_BONUS_HACK
	
	if (map_stack_attr[pItem->GetVnum()].TypeBonus[0] == iType && iBonus == 1 && pItem->GetAttributeValue(1) < map_stack_attr[pItem->GetVnum()].BonusMaxStack[0])
	{	
		pItem->SetForceAttribute(1, 0, pItem->GetAttributeValue(1) + iValue);
	
		int iCurrentValue = pItem->GetAttributeValue(0); 

		pItem->SetForceAttribute(0, map_stack_attr[pItem->GetVnum()].BonusType[0], pItem->GetAttributeValue(1) / map_stack_attr[pItem->GetVnum()].BonusRewardProgress[0]);

		int iNextValue = pItem->GetAttributeValue(0); 
		if (iCurrentValue != iNextValue)
			ch->ApplyPoint(map_stack_attr[pItem->GetVnum()].BonusType[0], 1);
	}
	
	else if (map_stack_attr[pItem->GetVnum()].TypeBonus[1] == iType && iBonus == 2 && pItem->GetAttributeValue(3) < map_stack_attr[pItem->GetVnum()].BonusMaxStack[1])
	{
		pItem->SetForceAttribute(3, 0, pItem->GetAttributeValue(3) + iValue);
		
		int iCurrentValue = pItem->GetAttributeValue(2); 
				
		pItem->SetForceAttribute(2, map_stack_attr[pItem->GetVnum()].BonusType[1], pItem->GetAttributeValue(3) / map_stack_attr[pItem->GetVnum()].BonusRewardProgress[1]);
	
		int iNextValue = pItem->GetAttributeValue(2); 
		if (iCurrentValue != iNextValue)
			ch->ApplyPoint(map_stack_attr[pItem->GetVnum()].BonusType[1], 1);
	}
	
	else if (map_stack_attr[pItem->GetVnum()].TypeBonus[2] == iType && iBonus == 3 && pItem->GetAttributeValue(5) < map_stack_attr[pItem->GetVnum()].BonusMaxStack[2])
	{
		pItem->SetForceAttribute(5, 0, pItem->GetAttributeValue(5) + iValue);
		
		int iCurrentValue = pItem->GetAttributeValue(4); 

		pItem->SetForceAttribute(4, map_stack_attr[pItem->GetVnum()].BonusType[2], pItem->GetAttributeValue(5) / map_stack_attr[pItem->GetVnum()].BonusRewardProgress[2]);
		
		int iNextValue = pItem->GetAttributeValue(4); 
		if (iCurrentValue != iNextValue)
			ch->ApplyPoint(map_stack_attr[pItem->GetVnum()].BonusType[2], 1);
	}
}
