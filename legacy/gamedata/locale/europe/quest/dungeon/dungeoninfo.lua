quest dungeoninfo begin
	state start begin
		when login or enter begin
			cmdchat("dungeon_info_qid "..q.getcurrentquestindex())
		end

		when button begin
			cmdchat("getinputbegin")
			local cmd = split(input(cmdchat("dungeon_info_cmd")), "#")
			cmdchat("getinputend")

			local mobIdx = tonumber(cmd[2])

			syschat("input is "..mobIdx)
			
			if cmd[1] == "ENTER" then
				syschat("you can put directly enter dungeon via quest.")
				--if mobIdx == 1093 then
				--	pc.warp(590500,110500)
				--elseif mobIdx == 2598 then
				--	pc.warp(590500,110500)
				--end
			elseif cmd[1] == "WARP" then
				syschat("you can put warp via quest.")
				--if mobIdx == 1093 then
				--	pc.warp(590500,110500)
				--elseif mobIdx == 6418 then
				--	syschat("you can put warp via quest.")
				--elseif mobIdx == 2598 then
				--	pc.warp(590500,110500)
				--end
			elseif cmd[1] == "TEST" then
				syschat("you can put test boss via quest.")
				--if mobIdx == 1093 then
				--	pc.warp(590500,110500)
				--elseif mobIdx == 2598 then
				--	pc.warp(590500,110500)
				--end
			end
		end
	end
end

