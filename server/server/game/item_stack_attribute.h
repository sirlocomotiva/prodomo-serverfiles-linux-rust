#ifndef __INC_ITEM_STACK_ATTRIBUTE__
#define __INC_ITEM_STACK_ATTRIBUTE__

#include "../common/length.h"
#include <boost/unordered_map.hpp>

class CHARACTER;
class CItem;

enum ETypeStackAttribute
{
	STACK_ATTRIBUTE_BY_MONSTER, // TYPE 0
	STACK_ATTRIBUTE_BY_STONE,	// TYPE 1
	STACK_ATTRIBUTE_BY_BOSS,	// TYPE 2
	STACK_ATTRIBUTE_BY_ITEM,	// TYPE 3
	STACK_ATTRIBUTE_BY_FISH,	// TYPE 4

	MAX_STACK_ATTRIBUTE,
};

class ITEM_STACK_ATTRIBUTE : public singleton<ITEM_STACK_ATTRIBUTE>
{
	public:
		ITEM_STACK_ATTRIBUTE();
		~ITEM_STACK_ATTRIBUTE();

		void	Initialize();
		bool	ReadItemStackAttrFile(const char * c_pszFileName);
		
		// Item Func
		void	AddStackAttribute(LPCHARACTER ch, int iType, DWORD bCell, int iVnumSet = 0, int iValue = 1);	
		void	SetStackAttributeIndex(LPCHARACTER ch, DWORD bCell, int iType, int iBonus, int iValue = 1);
		
		// Cell Item Func
		void	StackAttributeByWearIndex(LPCHARACTER ch, int iType, int iVnumSet = 0);

	protected:
		struct SInfoItem
		{
			int ItemVnum;
			int	TypeBonus[3];
			int	BonusType[3];
			int	BonusRewardProgress[3];
			int	BonusMaxStack[3];
			int	GetBonusVnum[3];
		};

		std::map<int, SInfoItem> map_stack_attr;
};

#endif
