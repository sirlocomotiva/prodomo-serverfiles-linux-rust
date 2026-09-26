quest ds_give_qualification begin
	state start begin
		when login begin
			ds.give_qualification()
		end
	end
end