quest yohara_system begin
	state start begin
		when login or levelup with pc.get_level() >= 120 begin
			set_state("information_new")
		end
	end
	
	state information_new begin
		when letter begin
			local v = find_npc_by_vnum(20433)
			if v != 0 then
				target.vid("Statue", v, "Statue")
			end
			
			send_letter("Statuia Razboinicului")
		end
		
		when button or info begin
			say_title("Devino Campion")
			say("Statuia Razboinicului vrea sa te vada.")
			say("Ar fi bine sa-l vizitezi. ")
			say("Il vei gasin orasul port. ")
			say("")
			say_reward("Urmareste punctul care clipeste pe mini-harta. ")
		end

		when Statue.target.click or 20433.chat."Devino Campion  " begin
			target.delete("Statue")
			
			say_title(mob_name(20433))
			say("")
			say("Salutari: ")
			say("Nu este oare timpul sa depasim limitele? ")
			say("")
			say("Cred ca este momentul. Nu vrei sa devii un Campion? ")
			say("")
			say("Pentru asta trebuie sa invingi Hydra. ")
			say("Daca decizia ta ramane, atunci invinge Hydra. ")
			say("Voi astepta aici. ")
			wait()
			say_title(mob_name(20433))
			say("")
			say("Fii atent, monstrul adancurilor Hydra ")
			say("are in plan sa puna stapanirea pe nava. ")
			say("Du-te si strica-i planurile. ")
			say("")
			say("Sunt sigur ca puterea ta o va fi suficienta ")
			say("pentru a o invinge. ")
			say("")
			set_state("killMonsters_new")
		end

	end

	state killMonsters_new begin
		when letter begin
			send_letter("Statuia Razboinicului")
		end

		when button or info begin
			say_title("Statuia Razboinicului: ")
			say("")
			say("Invinge Hydra. ")
			say("")
		end
		
		when 3965.kill begin
			local n = pc.getqf("kill_count") + 1
			pc.setqf("kill_count", n)
			if n == 1 then
				pc.setqf("kill_count", 0)
				say_title("Statuia Razboinicului: ")
				say("")
				say("Felicitari.")
				say("Te astept unde am vorbit ultima data. ")
				say("")
				set_state(killReward_new)
			end
		end
	end
	
	state killReward_new begin
		when letter begin
			target.vid("Overseer")
		end
		
		when button or info begin
			say_title("Statuia Razboinicului:")
			say("")
			say("Ai invins Hydra. ")
			say("Statuia Razboinicului te asteapta in port.")
			say("")
		end
		
		when 20433.chat."Devino Campion  " begin
			say_title(mob_name(20433))
			say("")
			say("Felicitari sincere.. ")
			say("Ai invins Hydra. Acum ti-ai dovedit puterea.")
			say("Poti avansa la nivelul de campion acum. ")
			say("")
			say_reward("Ai trecut la nivelul de campion")
			pc.set_conqueror(1)
			target.delete("Overseer")
			clear_letter()
			set_state (__COMPLETE__)
		end
	end
	state __COMPLETE__ begin
	end
end
