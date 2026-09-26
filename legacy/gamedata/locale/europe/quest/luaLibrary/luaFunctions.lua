party_get_member_pids = function()
	local pids = {party.get_member_pids()};
	return pids;
end

get_time_format = function(seconds)
	local finalString = "";
	local timeFormats = {
		{["timeFormat"] = "%S", ["timeString"] = "second", ["epoch"] = 0},
		{["timeFormat"] = "%M", ["timeString"] = "minute", ["epoch"] = 0},
		{["timeFormat"] = "%H", ["timeString"] = "hour", ["epoch"] = 2},
		{["timeFormat"] = "%d", ["timeString"] = "day", ["epoch"] = 1},
		{["timeFormat"] = "%m", ["timeString"] = "month", ["epoch"] = 1},
		{["timeFormat"] = "%Y", ["timeString"] = "years", ["epoch"] = 1970}
	};
	
	for index, data in timeFormats do
		local timeStomp = tonumber(os.date(data["timeFormat"], tonumber(seconds))-data["epoch"]);
		if (timeStomp > 0) then
			local editString = data["timeString"];
			if (timeStomp > 1) then
				editString = string.format("%s%s", editString, "s");
			end
			
			finalString = string.format("%d %s %s", timeStomp, editString, finalString);
		end
	end return finalString;
end

isNextMultiply = function(numToRoundUp, tableNum, multiplyBy)
	local multiple = 0;
	
	for index = 1, tableNum do
		if (multiple * multiplyBy == numToRoundUp) then
			return true;
		end
		multiple = multiple + 1;
	end
	return false;
end

isQuestAvailable = function(isitem)
	if not pc.can_warp() then say("Trebuie sa astepti 10 secunde pentru a continua!") return false; end
	if isitem then if (pc.count_item(item.get_vnum()) == 0) then say("Nu ai obiectul in inventar!") return false; end end
	if pc.is_trade0() then syschat("Inchide fereastra de trade!") return false; end
	if pc.is_busy0() then syschat("Inchide celelalte ferestre!") return false; end 
	-- if pc.get_empty_inventory_count() <= 5 then syschat("Ai inventarul plin. Arunca ceva din inventar.") return false; end
	
	return true;
end -- func

returnCommaString = function(amount)
	local value = amount;
	while true do
		value, k = string.gsub(value, "^(-?%d+)(%d%d%d)", '%1.%2');
		if (k == 0) then break; end
	end return value;
end

regenWriteLine = function(isPointMap)
	local playerMapIndex = pc.get_map_index();
	
	local strFileName = string.format("%s/quest/debug/%s", get_locale_base_path(), playerMapIndex);
	local createFile = io.open(strFileName, "a");
	
	local strRegenData = string.format("m	%d	%d	10	10	0	0	60s	100	1	VNUMPIETRE", pc.get_local_x(), pc.get_local_y());
	
	if (isPointMap) then
		strRegenData = string.format("--------- POINT MAP (%d %d) ---------", pc.get_x(), pc.get_y());
	end
	
	createFile:write(strRegenData, "\n");
	createFile:close();
	
	syschat(strRegenData);
end

writeSyserr = function(strLine)
	local strFileName = string.format("%s/quest/debug/syserr.txt", get_locale_base_path());
	local createFile = io.open(strFileName, "a");
	
	createFile:write(string.format("%s %s", tostring(os.date()), strLine), "\n");
	createFile:close();
end

writeSyslog = function(strLine)
	local strFileName = string.format("%s/quest/debug/syslog.txt", get_locale_base_path());
	local createFile = io.open(strFileName, "a");
	
	createFile:write(string.format("%s %s", tostring(os.date()), strLine), "\n");
	createFile:close();
end

tableHasValue = function(array, value)
	for index in array do
		if (array[index] == value) then
			return true;
		end
	end return false;
end

luaRandom = function(minNumber, maxNumber)
	if (type(minNumber) != "number" or type(maxNumber) != "number") then
		return -1;
	end
	
	if (minNumber >= maxNumber) then
		return -1;
	end
	
	local randomSeedDivison = math.random(2, 7);
	-- Initialize the pseudo random number generator
	math.randomseed(os.time());
	-- popping random numbers to get the real deal;
	math.random(); math.random(); math.random();
	return math.random(minNumber, maxNumber);
end

function tableShuffle(array)
	for index = table.getn(array), 2, -1 do
		local j = math.random(index)
		array[index], array[j] = array[j], array[index];
	end return array;
end
