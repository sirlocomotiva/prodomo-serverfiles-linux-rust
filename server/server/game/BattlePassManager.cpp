#include "stdafx.h"
#include <random>

#ifdef ENABLE_BATTLE_PASS
#include "BattlePassManager.h"
#include "db.h"
#include "config.h"
#include "locale_service.h"

#ifdef min
#undef min
#endif

#ifdef max
#undef max
#endif

#include <rapidjson/document.h>
#include <rapidjson/istreamwrapper.h>
#include <rapidjson/error/en.h>
#include <fstream>
#include "char.h"
#include "item.h"

EBattlePassMissionTypes getMissionType(const std::string& missionTypeString) {
	static const std::map<std::string, EBattlePassMissionTypes> missionTypeMap = {
		{"KILL", MISSION_TYPE_KILL},
		{"SELL", MISSION_TYPE_SELL},
		{"DESTROY", MISSION_TYPE_ITEM_DESTROY},
		{"DROP", MISSION_TYPE_ITEM_DROP},
		{"CRAFT", MISSION_TYPE_CRAFT},
		{"CHEST_OPEN", MISSION_TYPE_CHEST_OPEN},
		{"USE_ITEM", MISSION_TYPE_USE_ITEM},
		{"FINISH_DUNGEON", MISSION_TYPE_FINISH_DUNGEON},
		{"POLYMORPH", MISSION_TYPE_POLYMORPH},
		{"MESSAGES", MISSION_TYPE_MESSAGES},
		{"PLAYTIME", MISSION_TYPE_PLAYTIME}
	};

	auto it = missionTypeMap.find(missionTypeString);
	if (it != missionTypeMap.end()) {
		return it->second;
	}
	// Default value if mission type string is not found
	return MISSION_TYPE_KILL;
}

EBattlePassMissionTypeSpecific getMissionTypeSpecific(const std::string& missionTypeSpecificString) {
    if (missionTypeSpecificString == "SPECIFIC") {
        return MISSION_TYPE_SPECIFIC;
    }
    else if (missionTypeSpecificString == "GLOBAL") {
        return MISSION_TYPE_GLOBAL;
    }

    return MISSION_TYPE_SPECIFIC;
}

EBattlePassMissionTime getMissionTime(const std::string& missionTimeString) {
    if (missionTimeString == "DAILY") {
        return MISSION_TIME_DAILY;
    }
    else if (missionTimeString == "WEEKLY") {
        return MISSION_TIME_WEEKLY;
    }

    return MISSION_TIME_DAILY;
}

CBattlePass::CBattlePass()
{

}

CBattlePass::~CBattlePass()
{

}

void CBattlePass::BootBattlePassMap()
{
	if (g_bAuthServer)
		return;

	missionMap.clear();

	char filename[PATH_MAX] = {};
	snprintf(filename, sizeof(filename), "%s/battlepass.json", LocaleService_GetBasePath().c_str());
	std::ifstream ifs(filename, std::ios::in);

	if (ifs.fail())
		return;

	rapidjson::Document document;
	rapidjson::IStreamWrapper isw(ifs);
	document.ParseStream(isw);

	if (document.HasParseError())
	{
		sys_err("%s parse failed! Error: %s offset: %u", filename, GetParseError_En(document.GetParseError()), document.GetErrorOffset());
		return;
	}

     if (document.HasMember("Settings") && document["Settings"].IsObject()) {
        rapidjson::Value& settings = document["Settings"];

        if (settings.HasMember("maxDailyMissions") && settings["maxDailyMissions"].IsUint())
            battlePassSettings.maxDailyMissions = static_cast<uint8_t>(settings["maxDailyMissions"].GetUint());
        else {
            sys_err("Missing or invalid 'maxDailyMissions' value in JSON.");
            return;
        }

        if (settings.HasMember("maxWeeklyMissions") && settings["maxWeeklyMissions"].IsUint())
            battlePassSettings.maxWeeklyMissions = static_cast<uint8_t>(settings["maxWeeklyMissions"].GetUint());
        else {
            sys_err("Missing or invalid 'maxWeeklyMissions' value in JSON.");
            return;
        }

        if (settings.HasMember("endTime") && settings["endTime"].IsUint())
            battlePassSettings.endTime = settings["endTime"].GetUint();
        else {
            sys_err("Missing or invalid 'endTime' value in JSON.");
            return;
        }

    	if (settings.HasMember("rewardsFree") && settings["rewardsFree"].IsArray()) {
    		rapidjson::Value& rewards = settings["rewardsFree"];

    		for (rapidjson::SizeType i = 0; i < rewards.Size(); i++) {
    			if (rewards[i].IsArray() && rewards[i].Size() == 2 &&
					rewards[i][0].IsUint() && rewards[i][1].IsUint()) {
    				uint32_t rewardId = rewards[i][0].GetUint();
    				uint32_t rewardAmount = rewards[i][1].GetUint();

					battlePassSettings.rewardsFree.push_back(std::make_pair(rewardId, rewardAmount));
					} else {
						sys_err("Invalid reward format at index ", i, " in 'rewards' array.");
						continue;
					}
    		}
    	} else {
    		sys_err("Missing or invalid 'rewards' array in JSON.");
    		return;
    	}

    	if (settings.HasMember("rewardsPremium") && settings["rewardsPremium"].IsArray()) {
    		rapidjson::Value& rewards = settings["rewardsPremium"];

    		for (rapidjson::SizeType i = 0; i < rewards.Size(); i++) {
    			if (rewards[i].IsArray() && rewards[i].Size() == 2 &&
					rewards[i][0].IsUint() && rewards[i][1].IsUint()) {
    				uint32_t rewardId = rewards[i][0].GetUint();
    				uint32_t rewardAmount = rewards[i][1].GetUint();

    				battlePassSettings.rewardsPremium.push_back(std::make_pair(rewardId, rewardAmount));
					} else {
						sys_err("Invalid reward format at index ", i, " in 'rewards' array.");
						continue;
					}
    		}
    	} else {
    		sys_err("Missing or invalid 'rewards' array in JSON.");
    		return;
    	}
    }
    else {
        sys_err("Missing or invalid 'Settings' object in JSON.");
        return;
    }

    if (document.HasMember("missions") && document["missions"].IsArray()) {
        rapidjson::Value& missions = document["missions"];

        for (rapidjson::SizeType i = 0; i < missions.Size(); i++) {
            if (missions[i].IsObject()) {
                TBattlePassParser mission;
                mission.id = i;

                if (missions[i].HasMember("missionType") && missions[i]["missionType"].IsString())
                    mission.missionType = getMissionType(missions[i]["missionType"].GetString());
                else {
                    sys_err("Missing or invalid 'missionType' value in mission ", i, " in JSON.");
                    continue;
                }

                if (missions[i].HasMember("missionSpecific") && missions[i]["missionSpecific"].IsString())
                    mission.missionSpecific = getMissionTypeSpecific(missions[i]["missionSpecific"].GetString());
                else {
                    sys_err("Missing or invalid 'missionSpecific' value in mission ", i, " in JSON.");
                    continue;
                }

                if (missions[i].HasMember("maxProgress") && missions[i]["maxProgress"].IsUint())
                    mission.maxProgress = missions[i]["maxProgress"].GetUint();
                else {
                    sys_err("Missing or invalid 'maxProgress' value in mission ", i, " in JSON.");
                    continue;
                }

                if (missions[i].HasMember("timeInfo") && missions[i]["timeInfo"].IsString())
                    mission.timeInfo = getMissionTime(missions[i]["timeInfo"].GetString());
                else {
                    sys_err("Missing or invalid 'timeInfo' value in mission ", i, " in JSON.");
                    continue;
                }

                if (missions[i].HasMember("points") && missions[i]["points"].IsUint())
                    mission.pointsReward = missions[i]["points"].GetUint();
                else {
                    sys_err("Missing or invalid 'points' value in mission ", i, " in JSON.");
                    continue;
                }

                if (missions[i].HasMember("desc") && missions[i]["desc"].IsString())
                    mission.name = missions[i]["desc"].GetString();
                else {
                    sys_err("Missing or invalid 'desc' value in mission ", i, " in JSON.");
                    continue;
                }

            	//vnums
            	mission.vnums.clear();
            	if (mission.missionSpecific == MISSION_TYPE_SPECIFIC)
            	{
            		rapidjson::Value& vnumArray = missions[i]["vnum"];
            		if (vnumArray.IsArray()) {
            			for (rapidjson::SizeType j = 0; j < vnumArray.Size(); j++) {
            				if (vnumArray[j].IsUint()) {
            					mission.vnums.push_back(vnumArray[j].GetUint());
            				}
            			}
            		}
            	}

                missionMap[i] = mission;
            }
        }
    }
    else {
        sys_err("Missing or invalid 'missions' array in JSON.");
        return;
    }
}

std::vector<TBattlePassParser> CBattlePass::GetMissions(uint8_t timeInfo)
{
	// Prepare a vector to hold all missions
	std::vector<TBattlePassParser> allMissions;
	for (const auto& pair : missionMap) {
		allMissions.push_back(pair.second);
	}

	// Shuffle all missions
	std::random_device rd;
	std::mt19937 gen(rd());
	std::shuffle(allMissions.begin(), allMissions.end(), gen);

	// Select missions that match the timeInfo, up to 5
	std::vector<TBattlePassParser> selectedMissions;
	for (const auto& mission : allMissions) {
		if (mission.timeInfo == timeInfo) {
			selectedMissions.push_back(mission);
			if (selectedMissions.size() == 5) {
				break;
			}
		}
	}

	return selectedMissions;
}


void CBattlePass::RegisterTargetMission(uint8_t mission, uint32_t points, LPCHARACTER me, LPCHARACTER pkTarget)
{
	if (me->GetLevel() < 30)
		return;

	if (me->GetPlayerBattlePass().empty())
		return;

	for (auto& items : me->GetPlayerBattlePass())
	{
		for (auto& missionInfo : items.second)
		{
			const auto& mission_data = missionMap[missionInfo.missionID];
			if (mission_data.missionType == mission)
			{
				if (mission_data.missionSpecific != MISSION_TYPE_SPECIFIC || std::find(mission_data.vnums.begin(), mission_data.vnums.end(), pkTarget->GetRaceNum()) != mission_data.vnums.end())
				{
					me->BattlePassAction(missionInfo.missionID, missionInfo.type, points);
				}
			}
		}
	}
}

void CBattlePass::RegisterItemMission(uint8_t mission, uint32_t points, LPCHARACTER me, LPITEM pkItem)
{
	if (me->GetLevel() < 30)
		return;

	if (me->GetPlayerBattlePass().empty())
		return;

	if (!pkItem)
		return;

	for (auto& items : me->GetPlayerBattlePass())
	{
		for (auto& missionInfo : items.second)
		{
			const auto& mission_data = missionMap[missionInfo.missionID];
			if (mission_data.missionType == mission)
			{
				if (mission_data.missionSpecific != MISSION_TYPE_SPECIFIC || std::find(mission_data.vnums.begin(), mission_data.vnums.end(), pkItem->GetVnum()) != mission_data.vnums.end())
				{
					me->BattlePassAction(missionInfo.missionID, missionInfo.type, points);
				}
			}
		}
	}
}

void CBattlePass::RegisterDefaultMission(uint8_t mission, uint32_t points, LPCHARACTER me)
{
	if (me->GetLevel() < 30)
		return;

	for (auto& items : me->GetPlayerBattlePass())
	{
		for (auto& missionInfo : items.second)
		{
			const auto& mission_data = missionMap[missionInfo.missionID];
			if (mission_data.missionType == mission)
				me->BattlePassAction(missionInfo.missionID, missionInfo.type, points);
		}
	}
}

#endif



