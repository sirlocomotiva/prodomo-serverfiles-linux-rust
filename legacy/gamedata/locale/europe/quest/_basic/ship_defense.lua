quest ship_defense begin
	state start begin
		function in_dungeon(idx)
			return idx >= 358 * 10000 and idx < (358 + 1) * 10000
		end

		-- function main_quest_complete() -- Check past missions.
			-- if 0 > 0 then
				-- local main_quest = pc.getf("main_quest_meley_trail", "__status")
				-- if main_quest != nil and main_quest == main_quest_meley_trail.__COMPLETE__ then
					-- return true
				-- end
				-- return false
			-- end
			-- return true
		-- end

		function check_requirements(remove_ticket)

			if ship_defense_mgr.need_party() or party.is_party() then

				local pids = { party.get_member_pids() }
				local noQuestMembers = {}
				local notEnoughLevelMembers = {}
				local noTicketMembers = {}
				-- local questCheck = false
				local ticketCheck = true
				local levelCheck = true
				local ticketGroup = {71174, 1}

				-- Check if the player has a party.
				if not party.is_party() then
					syschat("Puteti incepe calatoria doar cu un grup.")
					return false
				end

				-- Check if the player is a party leader.
				if not party.is_leader() then
					say("Doar liderul grupului poate inregistra.")
					return false
				end

				for i, pid in next, pids, nil do
					q.begin_other_pc_block(pid)

					-- Check past mission of the player.
					-- if not ship_defense.main_quest_complete() then
						-- table.insert(noQuestMembers, pc.get_name())
						-- questCheck = false
					-- end

					-- Check level of the player.
					if pc.get_level() < 90 then
						table.insert(notEnoughLevelMembers, pc.get_name())
						levelCheck = false
					end

					-- Check tickets of the player.
					if ship_defense_mgr.need_ticket() then
						local hasTicket = false
						for idx = 1, table.getn(ticketGroup), 2 do
							if pc.count_item(ticketGroup[idx]) >= ticketGroup[idx + 1] then
								hasTicket = true
								break
							end
						end
						if not hasTicket then
							table.insert(noTicketMembers, pc.get_name())
							ticketCheck = false
						end
					end

					q.end_other_pc_block()
				end

				-- if not questCheck then
					-- say("Cel putin un jucator nu a finalizat misiunea[ENTER]inca.")
					-- for i, name in next, noQuestMembers, nil do
						-- say(color(1, 1, 0), " " .. name)
					-- end

					-- if not levelCheck then
						-- wait()
					-- else
						-- return false
					-- end
				-- end

				if not levelCheck then
					say("Cel putin un jucator nu indeplineste[ENTER]cerintele de nivel.")
					for i, name in next, notEnoughLevelMembers, nil do
						say(color(1, 1, 0), " " .. name)
					end

					if not ticketCheck then
						wait()
					else
						return false
					end
				end

				if ship_defense_mgr.need_ticket() then
					if not ticketCheck then
						say("Cel putin un jucator nu are Permis de[ENTER]Trecere.")
						for i, name in next, noTicketMembers, nil do
							say(color(1, 1, 0), " " .. name)
						end
						return false
					end
				end

				if party.get_near_count() < table.getn(pids) then
					say("Cel putin un jucator este prea departe de restul grupului.")
					return false
				end

				if ship_defense_mgr.need_ticket() and remove_ticket then
					for i, pid in next, pids, nil do
						q.begin_other_pc_block(pid)

						for i = 1, table.getn(ticketGroup), 2 do
							if pc.count_item(ticketGroup[i]) >= ticketGroup[i + 1] then
								pc.remove_item(ticketGroup[i], ticketGroup[i + 1])
								break
							end
						end

						q.end_other_pc_block()
					end
				end

			else -- need_party == 0 || has_part == 0

				-- Check tickets of the player.
				if ship_defense_mgr.need_ticket() then
					local hasTicket = false
					for idx = 1, table.getn(ticketGroup), 2 do
						if pc.count_item(ticketGroup[idx]) >= ticketGroup[idx + 1] then
							hasTicket = true
							break
						end
					end
					if not hasTicket then
						table.insert(noTicketMembers, pc.get_name())
						ticketCheck = false
					end

					if not ticketCheck then
						say("Cel putin un jucator nu are Permis de[ENTER]Trecere.")
						for i, name in next, noTicketMembers, nil do
							say(color(1, 1, 0), " " .. name)
						end
						return false
					end

					if remove_ticket then
						for i = 1, table.getn(ticketGroup), 2 do
							if pc.count_item(ticketGroup[i]) >= ticketGroup[i + 1] then
								pc.remove_item(ticketGroup[i], ticketGroup[i + 1])
								break
							end
						end
					end
				end
			end

			return true
		end

		when login begin
			if pc.get_map_index() == 358 then
				-- Check if the ship defense is created or running.
				if ship_defense_mgr.is_created() or ship_defense_mgr.is_running() then
					ship_defense_mgr.join()
				else
					ship_defense_mgr.leave()
				end
			end
		end

		when 20433.chat."Return" begin
			say("Doresti sa te teleportezi inapoi in oras?")
			if select("Da", "Nu") == 1 then
				warp_to_village()
			else
				say("Perfect. Spune-mi cand doresti sa revii in oras.")
			end
		end

		--when 20433.chat."Rankings" begin
		--	setskin(NOWINDOW)
		--	game.open_ranking(1, 15)
		--end

		when 9009.chat."Apararea Navei!" with pc.get_map_index() == 135 begin
			if game.get_event_flag("ship_defense") != 1 then
				say("Apararea navei va fi disponibila in curand.")
				return
			end

			if ship_defense_mgr.require_cooldown() then
				if pc.getqf("cooldown") > get_time() and not ship_defense_mgr.can_join() then
					say(string.format("Nu poti intra inca. Timp Ramas: %s", wait_time_to_str(pc.getqf("cooldown"))))
					-- if pc.is_gm() then
						-- if select("<GM> Reset Cooldown", "Cancel") == 1 then
							-- say("Done.")
							-- pc.setqf("cooldown", 0)
							-- wait()
						-- end
					-- else
						-- return
					-- end
					return
				end
			end

			say("Vrei sa-ti incepi calatoria in cautarea[ENTER]Hydra acum?")
			if select("Yes", "No") == 1 then

				-- Check if the player is mounting. ## intra pe un cont de player te rog
				if pc.is_mount() then
					say("Monturile nu sunt permise la bord. Descaleca inainte[ENTER]sa va imbarcati pe nava.")
					return
				end

				-- Check if the ship defense is created or running.
				if ship_defense_mgr.is_created() or ship_defense_mgr.is_running() then
					ship_defense_mgr.join()
					return
				end

				local remove_ticket = false
				if ship_defense.check_requirements(remove_ticket) then
					ship_defense_mgr.create()
				end
			end
		end

		when 20436.chat."Incepe Apararea Navei" with ship_defense.in_dungeon(pc.get_map_index()) begin
			say("Vrei sa pornesti si sa incepi calatoria?")
			if select("Da", "Nu") == 1 then

				-- Check if the ship defense is running.
				if ship_defense_mgr.is_running() then
					say("Apararea Navei este deja pornita.")
					return
				end

				local remove_ticket = true
				if ship_defense.check_requirements(remove_ticket) then
					say("Ce se întâmpla?! Marea devine din ce in ce mai agitata.. Se[ENTER]pregateste o furtuna?")
					ship_defense_mgr.start()
				end
			end
		end

		when 20436.chat."Opreste Calatoria" with ship_defense.in_dungeon(pc.get_map_index()) begin
			if ship_defense_mgr.is_running() then
				say("Vrei sa te intorci?")
			else
				say("Doresti sa pararesti Apararea Navei?")
			end
			if select("Da", "Nu") == 1 then
				ship_defense_mgr.leave()
			end
		end

		when 3949.click with ship_defense.in_dungeon(pc.get_map_index()) begin
			if ship_defense_mgr.is_running() then
				ship_defense_mgr.land()
			end
		end

		when 20437.kill with ship_defense.in_dungeon(pc.get_map_index()) begin
			if ship_defense_mgr.spawn_wood_repair() then
				game.drop_item(31107, 1)
			end
		end

		when 20434.take with ship_defense.in_dungeon(pc.get_map_index()) begin
			if ship_defense_mgr.spawn_wood_repair() then
				if item.get_vnum() == 31107 then
					ship_defense_mgr.set_alliance_hp_pct(1)
					item.remove()
				end
			end
		end
	end
end
