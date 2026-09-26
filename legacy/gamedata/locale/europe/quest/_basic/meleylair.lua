quest meleylair begin
	state start begin
		when 20419.chat."The Sung Mahi dragons?" with pc.get_map_index() == MeleyLair.GetSubMapIndex() begin
			say_title(mob_name(20419))
			say("")
			say("I fear that many will not grab the dawns of day...")
			say("")
			say("Sung-Mahi, the demons god, has come back and also with")
			say("their 3 dragons. They feed with human souls! Once they")
			say("have eaten enough, Sung Mahi powers will grow... and")
			say("that mean the end for all of us. We cannot allow this")
			say("to happen, but it will require something more than just")
			say("a brave warrior.")
			say("")
			say("Only a united guild power can stand up to fight these")
			say("frightening beasts. Can you help us? Do you know any other")
			say("adventuriers brave as you? We have to warn all of them!")
			wait()
			say_title(mob_name(20419))
			say("")
			say("I heard frightening stories about those warriors who want")
			say("to fight the dragons. They never come back...")
		end
		
		when 20419.chat."Fight the Sung Mahi dragons!" with pc.get_map_index() == MeleyLair.GetSubMapIndex() and pc.is_guild_master() begin
			say_title(mob_name(20419))
			say("")
			local registered, channel = MeleyLair.IsRegistered()
			if registered then
				say(string.format("You're guild is already registered! You can access[ENTER]the shelter from CH %d.[ENTER]", channel))
				return
			end
			
			local requirment1, requirment2 = MeleyLair.GetRequirments()
			say("To fight against Sung Mahi dragons you have to register")
			say("the guild.")
			say("")
			say("To register you need following requirments:")
			say(string.format("- Guild must be at least level %d.", requirment1))
			say(string.format("- Guild must have at least %d ladder points.", requirment2))
			say("")
			say("Do you want to register?")
			local menu = select("Yes", "No")
			if menu == 1 then
				local result1, result2 = MeleyLair.Register()
				if result1 == 0 then
					setskin(NOWINDOW)
					return
				end
				
				local resultMsg = {
								[1] = "You're guild is already registered! You can access[ENTER]the shelter from CH %d.",
								[2] = "You're guild is implicated yet in another dungeon[ENTER]try again later!",
								[3] = "You're guild need to be at least level %d to can[ENTER]register it.",
								[4] = "You're guild need to have at least %d ladder points to[ENTER]can register it.",
								[5] = "The guild wasn't registered, dungeon map can't be created[ENTER]please contact our staff.",
								[6] = "You're guild was registered, if you defeat the dragons,[ENTER]%d ladder points will be returned to the guild.",
								[7] = "You can register you're guild in %s.",
				}
				
				say_title(mob_name(20419))
				say("")
				if result1 != 2 and result1 != 5 then
					if result1 == 7 then
						local hours = string.format("%02.f", math.floor(result2 / 3600));
						local minutes = string.format("%02.f", math.floor(result2 / 60 - (hours * 60)));
						local seconds = string.format("%02.f", math.floor(result2 - hours * 3600 - minutes * 60));
						local timeConv = string.format(hours..":"..minutes..":"..seconds)
						say(string.format(resultMsg[result1], timeConv))
					else
						say(string.format(resultMsg[result1], result2))
					end
				else
					say(resultMsg[result1])
				end
			else
				setskin(NOWINDOW)
				return
			end
		end
		
		when 20419.chat."Go inside shelter!" with pc.get_map_index() == MeleyLair.GetSubMapIndex() and pc.has_guild() begin
			say_title(mob_name(20419))
			say("")
			say("Do you want to go inside shelter?")
			say("")
			local agree = select("Yes", "No")
			if agree == 1 then
				local registered, limit = MeleyLair.Enter()
				if not registered and limit == 0 then
					say_title(mob_name(20419))
					say("")
					if pc.is_guild_master() then
						say("You must register the guild to can have access to[ENTER]the shelter.")
					else
						say("The guild leader must register the guild to can[ENTER]have access to the shelter.")
					end
					
					return
				elseif not registered and limit > 0 then
					say_title(mob_name(20419))
					say("")
					say(string.format("You can access the shelter from CH %d.", limit))
					return
				elseif limit == 1 then
					say_title(mob_name(20419))
					say("")
					say(string.format("You can't access the shelter because there are[ENTER]yet %d guild members inside.[ENTER]", MeleyLair.GetPartecipantsLimit()))
					return
				elseif limit == 2 then
					say_title(mob_name(20419))
					say("")
					say("There are a problem, please contact our staff.")
					return
				elseif limit == 3 then
					say_title(mob_name(20419))
					say("")
					say("The dungeon is already finished.")
					return
				elseif limit == 4 then
					say_title(mob_name(20419))
					say("")
					say("To can join the run you must have the combatent")
					say("status checking in guild.")
					return
				end
				
				return
			else
				setskin(NOWINDOW)
				return
			end
		end
    end
end