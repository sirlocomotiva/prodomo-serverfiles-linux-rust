quest changelook begin
	state start begin
		when 60003.chat."Transmutation" begin
			say_title(mob_name(60003))
			say("")
			say("Transmutation enables you to permanently change")
			say("the appearance of your equipment. The effect is")
			say("restricted to the level of your equipment,")
			say("however, and the transmutation will be reversed")
			say("if you improve the item. Do you want to continue?")
			say("")
			local confirm = select("Yes", "No")
			if confirm == 2 then
				return
			end
			
			setskin(NOWINDOW)
			pc.open_changelook(true)
		end
	end
end