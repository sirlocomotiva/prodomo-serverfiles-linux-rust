dungeonFunctions = {};

ClearDungeon = function(revivedBoolean)
	if (pc.in_dungeon()) then
		d.clear_regen();
		d.kill_all();
		
		if (revivedBoolean) then
			d.kill_all();
		end
	end
end

npcGlobalArray = {};
isDungeonNPCblocked = function()
	local mapIndex = pc.get_map_index();
	
	for index in npcGlobalArray do
		if (npcGlobalArray[index][1] == mapIndex) then
			return true;
		end
	end return false;
end

blockDungeonNPC = function()
	local playerName = pc.get_name(); local mapIndex = pc.get_map_index();
	return table.insert(npcGlobalArray, {mapIndex, playerName});
end

getDungeonRemaining = function()
	local playerID = pc.get_player_id();
	local stringData = {
		{["dungeonString"] = string.format("demonTowerWait_%d", playerID)},
		{["dungeonString"] = string.format("spidernest_cooldown_%d", playerID)},
		{["dungeonString"] = string.format("devilsCatacombWait_%d", playerID)},
		{["dungeonString"] = string.format("dragonlair_cooldown_%d", playerID)},
		{["dungeonString"] = string.format("blazingWait_%d", playerID)},
		{["dungeonString"] = string.format("nemeresWait_%d", playerID)},
	};
	
	for index in stringData do
		local valueFlag = game.get_event_flag(stringData[index]["dungeonString"]);
		--cmdchat(string.format("returnDungeonTimers %d %d", index - 1, valueFlag));
	end
end

--[[
    Useful Dungeon functions.
   
    Refrain from using these inside a timer.
    Will lead to crashes due to pc instance being called while not existing.
]]
 
GetPartyMapIndex = function()
    return party.getf("map_index");
end -- function

GetLastPlayerDungeonInstanceIndex = function()
    return pc.getf("dungeon_data", "last_instance_index");
end -- function
 
IsPartyInDungeon = function()
    return d.find(GetPartyMapIndex());
end -- function

WasPlayerInDungeon = function()
    return d.find(GetLastPlayerDungeonInstanceIndex());
end -- function
 
GetPartyDungeonFloor = function()
    return d.getf_from_map_index("dungeon_floor", GetPartyMapIndex());
end -- function

GetPlayerDungeonFloor = function()
    return d.getf_from_map_index("dungeon_floor", GetLastPlayerDungeonInstanceIndex());
end -- function
 
IsSamePartyLeaderDungeon = function()
    return d.getf_from_map_index("party_leader_pid", GetPartyMapIndex()) == party.get_leader_pid();
end -- function

IsSameDungeonOwner = function()
    return d.getf_from_map_index("player_pid", GetLastPlayerDungeonInstanceIndex()) == pc.get_player_id();
end -- function

IsPartyOverTime = function()
    return d.getf_from_map_index("block_rejoin", GetPartyMapIndex()) == 1;
end -- function

IsPlayerOverTime = function()
    return d.getf_from_map_index("block_rejoin", GetLastPlayerDungeonInstanceIndex()) == 1;
end -- function

SetPartyMapIndex = function()
    party.setf("map_index", d.get_map_index());
end -- function

SetPlayerMapIndex = function()
    pc.setf("dungeon_data", "last_instance_index", d.get_map_index());
end -- function
 
SetPartyLeaderPid = function()
    d.setf("party_leader_pid", party.get_leader_pid());
end -- function

SetPlayerPid = function()
    d.setf("player_pid", pc.get_player_id());
end -- function
 
IncreaseDungeonFloor = function()
    d.setf("dungeon_floor", d.getf("dungeon_floor")+1);
end -- function

CanReEnterPartyDungeon = function()
	return party.is_party() and IsPartyInDungeon() and IsSamePartyLeaderDungeon() and not IsPartyOverTime();
end -- function

CanReEnterSoloDungeon = function()
	return not party.is_party() and WasPlayerInDungeon() and IsSameDungeonOwner() and not IsPlayerOverTime();
end -- function

dungeonFunctions.setWaitTime = function(pointedArray)
	local data = pointedArray;
	local timeData = data["timeData"]["timeToWaitData"];
	
	local timeToWaitData = timeData["timeToWait"];
	if (timeData["isRequire"]) then
		local strFlagTime = string.format("%s", timeData["waitingString"]);
		return pc.setf("dungeonManager", strFlagTime, get_global_time() + timeToWaitData);
	end
end

dungeonFunctions.checkRequire = function(pointedArray)
	local data = pointedArray;
	
	local levelRequire = data["levelRequire"];
	local requireItem = data["itemRequire"];
	local groupRequire = data["groupRequire"];
	local timeData = data["timeData"]["timeToWaitData"];
	
	local requireArray = {["minLevel"] = {{}, true}, ["maxLevel"] = {{}, true}, ["itemRequire"] = {{}, true}, ["timeRequire"] = {{}, true}};
	local isPartyCheck = false;
	if (data["groupRequire"]["isRequire"] and party.is_party()) then
		local partyGroupIds = party_get_member_pids();
		
		if (not party.is_leader()) then
			say("Trebuie sa fi liderul grupei pentru a intra.")
			return false;
		end
		
		if (party.get_near_count() < groupRequire["minimumPartyMemebers"]) then
			say(string.format("Trebuie sa fi intr-un grup de minim %d jucatori.", groupRequire["minimumPartyMemebers"]))
			return false;
		end
		
		for index, value in ipairs(partyGroupIds) do
			q.begin_other_pc_block(value);
				if ((pc.get_level() < levelRequire["minimumLevel"]) and levelRequire["isRequire"]) then
					table.insert(requireArray["minLevel"][1], pc.get_name());
					requireArray["minLevel"][2] = false;
				end
				
				if ((pc.get_level() > levelRequire["maximumLevel"]) and levelRequire["isRequire"]) then
					table.insert(requireArray["maxLevel"][1], pc.get_name());
					requireArray["maxLevel"][2] = false;
				end
				
				if ((pc.count_item(requireItem["itemVnum"]) < requireItem["itemCount"]) and requireItem["isRequire"]) then
					table.insert(requireArray["itemRequire"][1], pc.get_name());
					requireArray["itemRequire"][2] = false;
				end
				
				local strFlagTime = string.format("%s", timeData["waitingString"]);
				local flagValue = pc.getf("dungeonManager", strFlagTime);
				local localTime = get_global_time();
				
				if ((flagValue > localTime) and timeData["isRequire"]) then
					table.insert(requireArray["timeRequire"][1], string.format("%s - %s", pc.get_name(), get_time_format(flagValue - localTime)));
					requireArray["timeRequire"][2] = false;
				end
			q.end_other_pc_block();
		end isPartyCheck = true;
	end
	
	if (not isPartyCheck) then
		if ((pc.get_level() < levelRequire["minimumLevel"]) and levelRequire["isRequire"]) then
			table.insert(requireArray["minLevel"][1], pc.get_name());
			requireArray["minLevel"][2] = false;
		end
		
		if ((pc.get_level() > levelRequire["maximumLevel"]) and levelRequire["isRequire"]) then
			table.insert(requireArray["maxLevel"][1], pc.get_name());
			requireArray["maxLevel"][2] = false;
		end
		
		if ((pc.count_item(requireItem["itemVnum"]) < requireItem["itemCount"]) and requireItem["isRequire"]) then
			table.insert(requireArray["itemRequire"][1], pc.get_name());
			requireArray["itemRequire"][2] = false;
		end
		
		local playerPID = pc.get_player_id();
		local strFlagTime = string.format("%s", timeData["waitingString"]);
		local flagValue = pc.getf("dungeonManager", strFlagTime);
		local localTime = get_global_time();
		
		if ((flagValue > localTime) and timeData["isRequire"]) then
			table.insert(requireArray["timeRequire"][1], string.format("%s - %s", pc.get_name(), get_time_format(flagValue - localTime)));
			requireArray["timeRequire"][2] = false;
		end
	end
	
	if (not requireArray["minLevel"][2]) then
		say(string.format("Pentru a intra in temnita nivelul minim este %d.", levelRequire["minimumLevel"]))
		for index in requireArray["minLevel"][1] do
			say(string.format("- %s", requireArray["minLevel"][1][index]));
		end return false;
	end
	
	if (not requireArray["maxLevel"][2]) then
		say(string.format("Pentru a intra in temnita nivelul maxim este %d.", levelRequire["maximumLevel"]))
		for index in requireArray["maxLevel"][1] do
			say(string.format("- %s", requireArray["maxLevel"][1][index]));
		end return false;
	end
	
	if (not requireArray["itemRequire"][2]) then
		say(string.format("Pentru a intra in temnita trebuie sa deti %d - %s.", requireItem["itemCount"], c_item_name(requireItem["itemVnum"])))
		for index in requireArray["itemRequire"][1] do
			say(string.format("- %s", requireArray["itemRequire"][1][index]));
		end return false;
	end
	
	if (not requireArray["timeRequire"][2]) then
		say(string.format("Pentru a intra in temnita timpul de asteptare este %s.", get_time_format(timeData["timeToWait"])))
		for index in requireArray["timeRequire"][1] do
			say(string.format("- %s", requireArray["timeRequire"][1][index]));
		end return false;
	end return true;
end

dungeonFunctions.warpToDungeon = function(pointedArray)
	local data = pointedArray;
	
	local enterData = data["teleportData"]["insideDungeon"];
	local requireItem = data["itemRequire"];
	
	if (data["groupRequire"]["isRequire"] and party.is_party()) then
		if (requireItem["isRequire"]) then
			local partyGroupIds = party_get_member_pids();
			for index, value in ipairs(partyGroupIds) do
				q.begin_other_pc_block(value);
				pc.remove_item(requireItem["itemVnum"], requireItem["itemCount"]);
				q.end_other_pc_block();
			end
		end return d.new_jump_party(enterData["insideMapIndex"], enterData["enterPositions"]["x"], enterData["enterPositions"]["y"]);
	end
	
	if (not party.is_party()) then
		if (requireItem["isRequire"]) then
			pc.remove_item(requireItem["itemVnum"], requireItem["itemCount"]);
		end
		return d.new_jump(enterData["insideMapIndex"], enterData["enterPositions"]["x"]*100, enterData["enterPositions"]["y"]*100);
	end
end