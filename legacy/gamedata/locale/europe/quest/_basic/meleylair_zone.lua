quest meleylair_zone begin
	state start begin
		when 20419.chat."Leave the shelter!" with MeleyLair.IsMeleyMap() begin
			say_title(mob_name(20419))
			say("")
			say("Do you want to leave the shelter?")
			say("")
			local agree = select("Yes", "No")
			if agree == 1 then
				MeleyLair.Leave()
			else
				setskin(NOWINDOW)
				return
			end
		end
		
		when login with MeleyLair.IsMeleyMap() begin
			MeleyLair.Check()
		end
		
		when 20420.click with MeleyLair.IsMeleyMap() begin
			say_title(mob_name(20420))
			say("")
			local menu = select("Choose the reward", "Close")
			if menu == 1 then
				say_title(mob_name(20420))
				say("")
				if not MeleyLair.CanGetReward() then
					say("Nothing to do yet, just wait and clean you're weapons!")
				else
					say("You defeated Meley, the Queen of the Dragons. Choose a")
					say("reward:")
					local reward_menu = select("Queen Meley's Chest", "Dragon Watcher Chest", "Close")
					if reward_menu == 1 then
						say_title(mob_name(20420))
						say("")
						say("Do you want to get the Queen Meley's Chest?")
						local agree = select("Yes", "No")
						if agree == 1 then
							MeleyLair.Reward(reward_menu)
							say_title(mob_name(20420))
							say("")
							say("Here's your reward.")
						else
							setskin(NOWINDOW)
							return
						end
					elseif reward_menu == 2 then
						say_title(mob_name(20420))
						say("")
						say("Do you want to get the Dragon Watcher Chest?")
						local agree = select("Yes", "No")
						if agree == 1 then
							MeleyLair.Reward(reward_menu)
							say_title(mob_name(20420))
							say("")
							say("Here's your reward.")
						else
							setskin(NOWINDOW)
							return
						end
					else
						setskin(NOWINDOW)
						return
					end
				end
			else
				setskin(NOWINDOW)
				return
			end
		end
		
		when 20388.click."" begin
		end
    end
end