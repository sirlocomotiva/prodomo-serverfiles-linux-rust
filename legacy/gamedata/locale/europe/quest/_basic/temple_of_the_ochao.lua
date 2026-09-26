quest temple_of_the_ochao begin
    state start begin
		when login with pc.get_map_index() == 209 begin
			temple_ochao.initialize()
		end
		
		when 6311.kill with pc.get_level() >= 95 begin
			local drop = {
							{["vnum"] = 2145, ["count"] = 1, ["chance"] = 60,},
							{["vnum"] = 11295, ["count"] = 1, ["chance"] = 55,},
							{["vnum"] = 11495, ["count"] = 1, ["chance"] = 55,},
							{["vnum"] = 11695, ["count"] = 1, ["chance"] = 55,},
							{["vnum"] = 11895, ["count"] = 1, ["chance"] = 55,},
							{["vnum"] = 21075, ["count"] = 1, ["chance"] = 55,},
							{["vnum"] = 70043, ["count"] = 1, ["chance"] = 45,},
							{["vnum"] = 30609, ["count"] = 1, ["chance"] = 15,},
							{["vnum"] = 30612, ["count"] = 1, ["chance"] = 15,},
							{["vnum"] = 30523, ["count"] = 1, ["chance"] = 40,},
			}
			
			for i, info in ipairs(drop) do
				local generate = number(1, 100)
				if generate <= info.chance then
					game.drop_item_with_ownership(info.vnum, info.count, 300)
				end
			end
		end
		
		when 20408.chat."Intra in Templul Ochao" with pc.get_level() >= 95 begin
			say_title(mob_name(20408))
			say("")
			say("Vrei sa me duci direct in Templul lui")
			say("Ochao?")
			local answer = select("Da", "Nu")
			if answer == 1 then
				pc.warp(853700, 1416400)
			end
		end
		
		when 20415.chat."Calatoreste in Padurea Fermecata" with pc.get_level() >= 95 and pc.get_map_index() == 209 begin
			say_title(mob_name(20415))
			say("")
			say("Doriti sa calatoriti în Padurea Fermecata a")
			say("Zorilor?")
			local answer = select("Da", "Nu")
			if answer == 1 then
				pc.setf("enchanted_forest", "limit_activity", 0)
				pc.warp(816700, 1505000)
			end
		end
	end
end