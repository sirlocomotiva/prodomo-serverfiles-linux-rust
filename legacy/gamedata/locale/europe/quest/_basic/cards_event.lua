quest cards_event begin
	state start begin
		when login with game.get_event_flag("cards_event") != 0 begin
			cmdchat("cards icon")
		end
		when 20417.chat."Card set" begin
				say_title("Card set")
			say("")
				say("Do you want to trade 24 Okey Collectable Cards")
				say("for an Okey Card Set?")
			say("")
			local s = select("Yes","No")
			if s == 1 and pc.count_item(79505) >= 24 then
				pc.remove_item(79505, 24)
				pc.give_item2(79506)
			elseif s == 1 and pc.count_item(79505) < 24 then
				say_title("Card set")
				say("")
				say("You don't have 24 Okey Collectable Cards!")
				say("")
				if s == 2 then
					return
				end
			end
		end
		when 20417.chat."All rank" begin
			say(pc.get_okay_global_rank())
		end
		when 20417.chat."Rund rank" begin
			say(pc.get_okay_rund_rank())
		end
	end
end