quest rank_global begin
	state start begin
		when 20016.chat."Get Reward(Weekly Rank)" begin
			local ret = pc.global_rank_get_reward()
			if ret == 1 then
				say("done")
				return
			else
				say("there is no reward for you")
			end
		end
	end
end
