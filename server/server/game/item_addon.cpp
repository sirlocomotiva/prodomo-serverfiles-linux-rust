#include "stdafx.h"
#include "constants.h"
#include "utils.h"
#include "item.h"
#include "item_addon.h"
#include "char_manager.h"

CItemAddonManager::CItemAddonManager()
{
}

CItemAddonManager::~CItemAddonManager()
{
}

void CItemAddonManager::ApplyAddonTo(int iAddonType, LPITEM pItem)
{
	if (!pItem)
	{
		sys_err("ITEM pointer null");
		return;
	}

	// TODO 일단 하드코딩으로 평타 스킬 수치 변경만 경우만 적용받게한다.

	int iSkillBonus = MINMAX(-30, (int) (gauss_random(0, 5) + 0.5f), 30);
	int iNormalHitBonus = 0;
	if (abs(iSkillBonus) <= 20)
		iNormalHitBonus = -2 * iSkillBonus + abs(number(-8, 8) + number(-8, 8)) + number(1, 4);
	else
		iNormalHitBonus = -2 * iSkillBonus + number(1, 5);


	pItem->AddAttribute(APPLY_NORMAL_HIT_DAMAGE_BONUS, iNormalHitBonus);
	pItem->AddAttribute(APPLY_SKILL_DAMAGE_BONUS, iSkillBonus);

	pItem->AddAttribute(APPLY_NORMAL_HIT_DAMAGE_BONUS, iNormalHitBonus);
	pItem->AddAttribute(APPLY_SKILL_DAMAGE_BONUS, iSkillBonus);
	
#ifdef ENABLE_REWARD_SYSTEM
	if (iNormalHitBonus >= 45)
	{
		LPCHARACTER ch = pItem->GetOwner();
		if (ch)
			CHARACTER_MANAGER::Instance().DoReward(ch, REWARD_MISSION_AVERAGE_BONUS, iNormalHitBonus, 1);
	}
#endif

	
}
