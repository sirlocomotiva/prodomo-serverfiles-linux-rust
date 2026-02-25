#pragma once
#include "../common/tables.h"

class CBattlePass : public singleton<CBattlePass>
{
	public:
		CBattlePass();
		~CBattlePass() override;

        void BootBattlePassMap();
		std::vector<TBattlePassParser> GetMissions(uint8_t timeInfo);

		const auto& GetMissionMap() { return missionMap; }

		void RegisterTargetMission(uint8_t mission, uint32_t points, LPCHARACTER me, LPCHARACTER pkTarget);
		void RegisterItemMission(uint8_t mission, uint32_t points, LPCHARACTER me, LPITEM pkItem)
	;
		void RegisterDefaultMission(uint8_t mission, uint32_t points, LPCHARACTER me);

		//Get Settings
		const auto& GetBattlePassSettings() { return battlePassSettings; }

private:
	std::map<uint16_t, TBattlePassParser> missionMap;
	TBattlePassSettings battlePassSettings = {};
};
