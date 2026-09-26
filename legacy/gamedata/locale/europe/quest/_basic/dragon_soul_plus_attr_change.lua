quest ds_attr_change begin
	state start begin
		when 20001.chat."Schimbare Atributii Piatra Dragon" with ds.is_qualified() != 0 begin
			say_title(mob_name(20001))
			say ("Vrei sa deschizi schimbarea atributiilor pietrei dragon?")
			local b= select("Da", "Nu")
			if 1==b then
				ds.open_attr_change_window()
			else
			end
		end
	end
end
