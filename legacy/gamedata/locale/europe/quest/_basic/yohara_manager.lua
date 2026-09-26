quest yohara_manager begin
	state start begin
		when 20433.chat."Un continent nou"  begin
			say_title(mob_name(20433)..":")
			say("Doresti sa calatoresti pe noul continent Yohara?")
			local s = select("Da","Nu")
			if s == 1 then
				if pc.get_conqueror_level() >= 1 then
					pc.warp(537500,492700)
				else
					say_title(mob_name(20433)..":")
					say_reward("Nivelul tau nu este suficient pentru a incepe aceasta aventura!")
				end
			end
		end
	end
end
