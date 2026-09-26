quest new7and8th_skills begin
    state start begin
		when 50514.use begin
			say_title(item_name(50514))
			say("")
			if pc.get_skill_group() == 0 then
				say("You didn't start an apprenticeship yet.")
				return
			end
			
			local check_learned = 0
			local help_skill_list = {
										{{236}, {240},},
										{{237}, {241},},
										{{238}, {242},},
										{{239}, {243},},
										{{244},},
			}
			
			local skill_list = help_skill_list[pc.get_job() + 1][pc.get_skill_group()]
			for i = 1, table.getn(skill_list) do
				skill_vnum = skill_list[i]
				if pc.get_skill_level(skill_vnum) > 0 then
					check_learned = 1
				end
			end
			
			if check_learned == 1 then
				say("You already learn one of boost skills.")
				return
			end
			
			help_skill_list_name = {
								[236] = "Sword Spin Boost",
								[237] = "Ambush Boost",
								[238] = "Finger Strike Boost",
								[239] = "Shooting Dragon Boost",
								[240] = "Spirit Strike Boost",
								[241] = "Fire Arrow Boost",
								[242] = "Dark Strike Boost",
								[243] = "Summon Lightning Boost",
								[244] = "Wolf's Breath Boost"
			}
			
			local skill_vnum_list = {}
			local skill_name_list = {}
			for i = 1, table.getn(skill_list) do
				skill_vnum = skill_list[i]
				if pc.get_skill_level(skill_vnum) < 1 then
					table.insert(skill_vnum_list, skill_vnum)
					table.insert(skill_name_list, help_skill_list_name[skill_vnum])
				end
			end
			
			if table.getn(skill_vnum_list) == 0 then
				say("There are no boost skills available.")
				return
			end
			
			table.insert(skill_name_list, "Cancel")
			say("This ancient book seems to be very powerful! It is")
			say("the key to tremendous skills. But first you must")
			say("decide! Fighters can only master a single skill.")
			say("Choose wisely")
			local i = select_table(skill_name_list)
			if i == table.getn(skill_name_list)then
				return
			end
			
			local name = skill_name_list[i]
			local vnum = skill_vnum_list[i]
			say_title(item_name(50514))
			say("")
			say(string.format("You choose to learn %s.", name))
			say("You're sure about your decision?")
			local confirm = select("Yes", "No")
			if confirm == 1 then
				pc.remove_item(item.get_vnum(), 1)
				pc.set_skill_level(vnum, 1)
				return
			end
			
			return
		end
		
		when 50515.use begin
			say_title(item_name(50515))
			say("")
			if pc.get_skill_group() == 0 then
				say("You didn't start an apprenticeship yet.")
				return
			end
			
			local check_learned = 0
			local anti_skill_list = {221, 222, 223, 224, 225, 226, 227, 228, 229}
			for i = 1, table.getn(anti_skill_list) do
				skill_vnum = anti_skill_list[i]
				if pc.get_skill_level(skill_vnum) > 0 then
					check_learned = 1
				end
			end
			
			if check_learned == 1 then
				say("You already learn one of ward skills.")
				return
			end
			
			anti_skill_list_name = {
								[221] = "Sword Spin Ward",
								[222] = "Ambush Ward",
								[223] = "Finger Strike Ward",
								[224] = "Shooting Dragon Ward",
								[225] = "Spirit Strike Ward",
								[226] = "Fire Arrow Ward",
								[227] = "Dark Strike Ward",
								[228] = "Summon Lightning Ward",
								[229] = "Wolf's Breath Ward"
			}
			
			local skill_vnum_list = {}
			local skill_name_list = {}
			for i = 1, table.getn(anti_skill_list) do
				skill_vnum = anti_skill_list[i]
				if pc.get_skill_level(skill_vnum) < 1 then
					table.insert(skill_vnum_list, skill_vnum)
					table.insert(skill_name_list, anti_skill_list_name[skill_vnum])
				end
			end
			
			if table.getn(skill_vnum_list) == 0 then
				say("There are no ward skills available.")
				return
			end
			
			table.insert(skill_name_list, "Cancel")
			say("This ancient book seems to be very powerful! It is")
			say("the key to tremendous skills. But first you must")
			say("decide! Fighters can only master a single skill.")
			say("Choose wisely")
			local i = select_table(skill_name_list)
			if i == table.getn(skill_name_list)then
				return
			end
			
			local name = skill_name_list[i]
			local vnum = skill_vnum_list[i]
			say_title(item_name(50515))
			say("")
			say(string.format("You choose to learn %s.", name))
			say("You're sure about your decision?")
			local confirm = select("Yes", "No")
			if confirm == 1 then
				pc.remove_item(item.get_vnum(), 1)
				pc.set_skill_level(vnum, 1)
				return
			end
			
			return
		end
		
		when 50525.use begin
			say_title(item_name(50525))
			say("")
			if pc.get_skill_group() == 0 then
				say("You didn't start an apprenticeship yet.")
				return
			end
			
			if get_time() < pc.getqf("next_time") then
				if not pc.is_skill_book_no_delay() then
					say("When your training is done, you have to")
					say("rest for 12 to 24 hours.")
					return
				end
			end
			
			GRAND_MASTER_SKILL_LEVEL = 30
			PERFECT_MASTER_SKILL_LEVEL = 40
			local check_learned = 0
			local new_skill_list = {221, 222, 223, 224, 225, 226, 227, 228, 229, 236, 237, 238, 239, 240, 241, 242, 243, 244}
			for i = 1, table.getn(new_skill_list) do
				skill_vnum = new_skill_list[i]
				if pc.get_skill_level(skill_vnum) >= GRAND_MASTER_SKILL_LEVEL and pc.get_skill_level(skill_vnum) < PERFECT_MASTER_SKILL_LEVEL then
					check_learned = 1
				end
			end
			
			if check_learned == 0 then
				say("No skill was learned yet through the skill")
				say("training of the grand masters.")
				return
			end
			
			new_skill_list_name = {
								[221] = "Sword Spin Ward",
								[222] = "Ambush Ward",
								[223] = "Finger Strike Ward",
								[224] = "Shooting Dragon Ward",
								[225] = "Spirit Strike Ward",
								[226] = "Fire Arrow Ward",
								[227] = "Dark Strike Ward",
								[228] = "Summon Lightning Ward",
								[229] = "Wolf's Breath Ward",
								[236] = "Sword Spin Ward",
								[237] = "Ambush Ward",
								[238] = "Finger Strike Ward",
								[239] = "Shooting Dragon Ward",
								[240] = "Spirit Strike Ward",
								[241] = "Fire Arrow Ward",
								[242] = "Dark Strike Ward",
								[243] = "Summon Lightning Ward",
								[244] = "Wolf's Breath Ward"
			}
			
			local skill_vnum_list = {}
			local skill_name_list = {}
			for i = 1, table.getn(new_skill_list) do
				skill_vnum = new_skill_list[i]
				if pc.get_skill_level(skill_vnum) >= GRAND_MASTER_SKILL_LEVEL and pc.get_skill_level(skill_vnum) < PERFECT_MASTER_SKILL_LEVEL then
					table.insert(skill_vnum_list, skill_vnum)
					table.insert(skill_name_list, new_skill_list_name[skill_vnum])
				end
			end
			
			if table.getn(skill_vnum_list) == 0 then
				say("There are no ward or boost skills available.")
				return
			end
			
			say("When your waste your Good/Evils points during")
			say("the training it's possible that you can be")
			say("downgraded, even under the level of a Ronin.")
			say("")
			say("Do you want to continue?")
			local s = select("Yes", "No")	
			if s == 2 then
				return
			end
			
			say_title(item_name(50525))
			say("")
			say("Choose the skill you want to learn:")
			table.insert(skill_name_list, "Cancel")
			local i = select_table(skill_name_list)
			if i == table.getn(skill_name_list)then
				return
			end
			
			local name = skill_name_list[i]
			local vnum = skill_vnum_list[i]
			local level = pc.get_skill_level(vnum)
			local cur_alignment = pc.get_real_alignment()
			local need_alignment = 1000 + 500 * (level - 30)
			
			say_title(item_name(50525))
			say("")
			if cur_alignment <- 19000+need_alignment then
				say_reward("You don't have enough alignment points")
				say_reward("for a Grand Master training.")
				return
			end
			
			if get_time() < pc.getqf("next_time") then
				if pc.is_skill_book_no_delay() then
					pc.remove_skill_book_no_delay()
				else
					say("When your training is done, you have to")
					say("rest for 12 to 24 hours.")
				end
			end
			
			say(string.format("You choose to learn %s.", name))
			say("You're sure about your decision?")
			local confirm = select("Yes", "No")
			if confirm == 1 then
				pc.setqf("next_time", get_time() + 60 * 60 * math.random(12, 24))
				
				say_title(item_name(50525))
				say("")
				if pc.learn_grand_master_skill(vnum) then
					if pc.get_skill_level(vnum) == 40 then
						say(string.format("%s became a perfect Master.", name))
					else
						say(string.format("You raised %s to G%d.", name, level-30+1+1))
					end
					
					say("My body is full of power. Something comes out here!")
					say("You have completed higher level of training successfully.")
				else
					pc.change_alignment(-number(need_alignment / 3, need_alignment / 2))
					say("That did not work. Darn!")
				end
				
				pc.remove_item(item.get_vnum(), 1)
			end
			
			return
		end
		
		when 71000.use begin
			say_title(item_name(71000))
			say("")
			if pc.get_skill_group() == 0 then
				say("You didn't start an apprenticeship yet.")
				return
			end
			
			local check_learned = 0
			local new_skill_list = {221, 222, 223, 224, 225, 226, 227, 228, 229, 236, 237, 238, 239, 240, 241, 242, 243, 244}
			for i = 1, table.getn(new_skill_list) do
				skill_vnum = new_skill_list[i]
				if pc.get_skill_level(skill_vnum) > 0 then
					check_learned = 1
				end
			end
			
			if check_learned == 0 then
				say("You didn't learn any boost or ward skill yet.")
				return
			end
			
			say("You will forgot anything you learn about boost")
			say("and ward, are you sure about your decision?")
			local confirm = select("Yes", "No")
			if confirm == 1 then
				for i = 1, table.getn(new_skill_list) do
					skill_vnum = new_skill_list[i]
					if pc.get_skill_level(skill_vnum) > 0 then
						pc.set_skill_level(skill_vnum, 0)
					end
				end
				
				pc.remove_item(item.get_vnum(), 1)
			end
			
			return
		end
    end
end