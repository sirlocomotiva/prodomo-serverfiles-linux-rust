quest change_empire begin
	state start begin
		when 20090.chat."Empire Exchange"  with game.get_event_flag("c_e") >0  begin

			local count = pc.get_change_empire_count() ;
			if count >= 1 then
				say("You can not change your Empire")
				say("")
				return
			end
			say_reward("Empire Exchange")
			say("")
			say("Escape into another Empire.")
			say("A Guild Leader can not change the Empire...")
			say("You first have to close your guild.")
			say("Married persons have to get a divorce before.")
			say("The cost of an imperial Exchange is 500.000 Gold")
			say("")
			say("Do you really want to change the empire?")
			say("")

			local s = select("Change", "Don't Change")

			if s == 1 then
				change_empire.move_pc()
			end
		end

		when 71054.use begin
			say("You have successfully changed the kingdom,")
			say("The rulers of the new kingdom called you welcome!")

			if get_time() < pc.getqf("next_use_time") then
				say("You can not change the kingdom.")
				say("")

				if pc.is_gm() then
					say("GM為了測試可以設定時間")
					say("")
					local s = select("*奐s設定", "取消")
					if s == 1 then
						say("時間初始化了")
						pc.setqf("next_use_time", 0)
					end
				end

				return
			end

			if change_empire.move_pc() == true then
				pc.setqf("next_use_time", get_time() + 8640 * 7)
			end
		end



		function move_pc()
			if pc.is_engaged() then
				say("You are in a Guild.")
				say("You can not change the kingdom.")
				say("")
				return false
			end

			if pc.is_married() then
				say("You're married.")
				say("You can not change the kingdom.")
				say("")
				return false
			end

			if pc.is_polymorphed() then
				say("You are Transformed.")
				say("You can not change the kingdom.")
				say("")
				return false
			end

			if pc.has_guild() then
				say("You are in a guild.")
				say("You can not change the kingdom.")
				say("")
				return false
			end
			if pc.money < 500000 then
				say("You have not enough Gold.")
				say("To change to the kingdom, you need 500.000 Gold.")
				say("")
				return false
			end
			say("Choose the Empire")
			local s = select("Red", "Yellow", "Blue", "Cancel")
			if 4==s then
				return false 
			end
			say("")
			say_reward("Do you really want to change the empire?")
			say_reward("For treason, there is no excuse!")
			say("")
			local a = select("Change", "Don't Change")
			if 2== a then
				return false
			end

			local ret = pc.change_empire(s)
			local oldempire = pc.get_empire()
			if ret == 999 then
				say("You have successfully changed the empire.")
				say("Please relog in.")
				say("")
				pc.change_gold(-500000)
				pc.remove_item(71054) ;

				char_log(0, "CHANGE_EMPIRE",string.format("%d -> %d", oldempire, s)) 
			
				return  true
			else
				if ret == 1 then
					say("You are already in this Empire.")
					say("Please choose a different Empire.")
					say("")
					say("")
				elseif ret == 2 then
					say("Empire exchange not possible now.")
					say("You can not change the kingdom, because you recently were still in a guild.")
					say("")
					say("")
				elseif ret == 3 then
					say("Empire exchange not possible now.")
					say("You can not change the kingdom, since you were married recently.")
					say("")
				end
				elseif ret == 4 then
					say("Empire exchange not possible now.")
					say("You cannot change the empire because you have an active group.")
					say("")
				end
			end
			return false
		end
	end
end