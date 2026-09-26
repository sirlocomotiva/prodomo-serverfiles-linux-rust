quest event_info begin
	state start begin
		when login begin
			cmdchat(string.format("event_info %d %d %d", game.get_event_flag("letters_event"), game.get_event_flag("cards_event"), game.get_event_flag("enable_fish_event")))
		end
	end
end