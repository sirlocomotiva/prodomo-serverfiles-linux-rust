quest system_gaya_0 begin
	state start begin
		when 20095.chat."Pietre Strãlucitoare" begin
			say("")
			say("Doreºti sã transformi pietrele strãlucitoare")
			say("în Gaya ? ")
			say("Pentru a începe am nevoie de minim 5 pietre strãlucitoare.")
		end

		when 20095.take with pc.count_item(30400) >= 5 begin
			say("")
			say("Am nevoie de 5 pietre strãlucitoare ºi de pietre spirit ")
			say("+0, +1, +2, sau +3 pentru a le transforma în Gaya. ")
			say("Desigur, am nevoie ºi de 50.000 yang.  ")
			say("")
			say("ªansa de reuºitã este de 60%, ")
			say("Doreºti sã continuãm ? ")
			say("")
			local option = select("Da ","Nu ")
			if option == 1 then
				 game.open_gaya()
			end
		end

		when 20095.chat."Ce bonusuri îmi aduce Gaya? " begin
			say("")
			say("Nu ai auzit încã de magazinul Gaya? ")
			say("Nu? Oh atunci trebuie sã îþi povestesc.. ")
			say("De la magazinul gaya poþi achiziþiona anumite obiecte folositoare! ")
			say("Doreºti sã vizualizezi magazinul ? ")
			say("")
			local option = select("Da ","Nu ")
			if option == 1 then
				game.open_gaya_market()
			end
		end
	end
end
