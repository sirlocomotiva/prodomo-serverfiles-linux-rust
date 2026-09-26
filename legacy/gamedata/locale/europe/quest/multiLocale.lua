-- #################################################################
-- ##                                                             ##
-- ##                      MULTI_LOCALE.LUA                       ##
-- ##                                                             ##
-- #################################################################

--[[
 This file replaces the old locale.lua since it's a static file with
 one big array we are not able to transalte most of them unless we
 make a function to get the player's locale.
 
 Below are some functions that do the job.
 will be stored into them and isn't possible to change then 
]]

-- locale.map_name
map_name = function(map_index)
	local mapName = {
		[61] = gameforge[LC()][2691],
		[62] = gameforge[LC()][2692],
		[63] = gameforge[LC()][2693],
		[64] = gameforge[LC()][2694],
	}
	return skillName[map_index]
end

-- locale.empire_names
empire_names = function(empire)
	local empireName = {
		[0] = gameforge[LC()][2477],
		[1] = gameforge[LC()][2478],
		[2] = gameforge[LC()][2479],
		[3] = gameforge[LC()][2480],
	}
	return empireName[empire]
end

-- locale.GM_SKILL_NAME_DICT
get_skill_name_by_vnum = function(vnum)
	local skillName = {
		-- Warrior Skill Group 1
		[1] = gameforge[LC()][2513],
		[2] = gameforge[LC()][2524],
		[3] = gameforge[LC()][2535],
		[4] = gameforge[LC()][2546],
		[5] = gameforge[LC()][2557],
	}
	return skillName[vnum]
end
