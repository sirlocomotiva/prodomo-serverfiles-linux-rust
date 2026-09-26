-- init table (clear in case reload)
DungeonInformationLibrary = {};

DungeonInformationLibrary.LoadDungeonInformation = function()
	-- Creating dungeonInformationTable
	DungeonInformationLibrary["dungeonData"] = {};
	
	table.insert(DungeonInformationLibrary["dungeonData"],
		{
			["dungeonID"] = nil,
			["dungeonHandle"] = "DUNGEON_INFO_DEMON_TOWER",
			
			["teleportData"] = {
				["insideDungeon"] = {["insideMapIndex"] = 66, ["enterPositions"] = {["x"] = 2243, ["y"] = 7037}},
				["outsideDungeon"] = {["insideMapIndex"] = 65, ["enterPositions"] = {["x"] = 5903, ["y"] = 1110}}
			},
			
			["timeData"] = {
				["timeToCompleteData"] = {["isRequire"] = true, ["timeToComplete"] = time_min_to_sec(30)},
				["timeToWaitData"] = {["isRequire"] = false, ["timeToWait"] = time_min_to_sec(10), ["waitingString"] = "demonTime"}
			},
			
			["levelRequire"] = {["isRequire"] = false, ["minimumLevel"] = 45, ["maximumLevel"] = 120},
			["itemRequire"] = {["isRequire"] = false, ["itemVnum"] = 0, ["itemCount"] = 0},
			["groupRequire"] = {["isRequire"] = false, ["minimumPartyMemebers"] = 2},
		}
	);
	
	if (table.getn(DungeonInformationLibrary["dungeonData"]) < 1) then
		sys_err("Failed to LoadDungeonInformation.");
		return false;
	end
	
	-- completing info
	for index, value in ipairs(DungeonInformationLibrary["dungeonData"]) do
		-- setting dungeon id
		value["dungeonID"] = index;
		-- setting dungeon handle
		_G[value["dungeonHandle"]] = index;
	end
end

DungeonInformationLibrary.ReturnDungeonInformation = function(DungeonHandle)
	if (not DungeonInformationLibrary["dungeonData"][DungeonHandle]) then
		sys_err(string.format("Failed to ReturnDungeonInformation -> Dungeon Handle %s.", DungeonHandle));
		return nil;
	end
	
	return DungeonInformationLibrary["dungeonData"][DungeonHandle];
end

DungeonInformationLibrary.IsInDungeon = function(DungeonHandle)
	if (not DungeonInformationLibrary["dungeonData"][DungeonHandle]) then
		sys_err(string.format("Failed to ReturnDungeonInformation:IsInDungeon -> Dungeon Handle %s.", DungeonHandle));
		return nil;
	end
	
	local playerMapIndex, dungeonMapIndex = pc.get_map_index(), DungeonInformationLibrary["dungeonData"][DungeonHandle]["teleportData"]["insideDungeon"]["insideMapIndex"];
	return (pc.in_dungeon() and (playerMapIndex >= dungeonMapIndex*10000) and (playerMapIndex < (dungeonMapIndex+1)*10000));
end

DungeonInformationLibrary.LoadDungeonInformation();