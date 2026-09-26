quest battlePass begin
	state start begin
		--when 25162.use with pc.is_gm() begin
		when 25162.use with pc.getf("battlePass","status") == 2 begin
			say_title(item_name(item.get_vnum()))
			say("")
			say("Doriti sa resetati BattlePass-ul?")
			say("Amintiti-va ca il puteti reseta si face altul din nou")
			say("cu acelasi personaj.")
			say("")
			say("Atentie!")
			say("Asigurati-va ca ati terminat deja BattlePass inainte.")
			say("Daca il resetati inainte veti pierde tot progresul si")
			say("toate recompensele pe care nu le-ati revendicat inca...")
			say("")
			say("")
			local x = select("Da", "Nu")
			if x == 1 then
				clear_old_battlepass()
				pc.setf("battlePass","monthIndex", 0)
				pc.setf("battlePass","status", 0)
				command("battle_pass info")
				item.remove()
			end
		end
		when 25150.use or 
			25151.use or 
			25152.use or 
			25153.use or 
			25154.use or 
			25155.use or 
			25156.use or 
			25157.use or 
			25158.use or 
			25159.use or 
			25160.use or 
			25161.use begin
			say_title(item_name(item.get_vnum()))
			say("")
			local month = monthIndex()
			local itemMonth = item.get_value(0)
			if month+1 != itemMonth then
				say("Nu poti folosi acest obiect!")
				return
			end
			local playerMonthIndex = pc.getf("battlePass","monthIndex")
			if playerMonthIndex == itemMonth then
				say("Ai deja Battle Pass activ!")
				return
			else
				if playerMonthIndex < itemMonth then
					say_reward("Battle Pass")
					say("Vrei sa activezi Battle Pass?")
					say("")
					say("Nu uita sa terminati toate misiunile")
					say("inainte de sfarsitul lunii sau iti vei pierde recompensa.")
					say("")
					local x = select("Da", "Nu")
					if x == 1 then
						clear_old_battlepass()
						pc.setf("battlePass","monthIndex", itemMonth)
						pc.setf("battlePass","status", 1)
						command("battle_pass info")
						item.remove()
					else
						return
					end
				end
			end
		end
	end
end

