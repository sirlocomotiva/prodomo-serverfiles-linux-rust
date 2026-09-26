quest sash begin
	state start begin
		when 60003.chat."What is a Sash?" begin
			say_title(mob_name(60003))
			say("")
			say("The sash offers you two options:")
			say("Combination & Absorption.")
			say("")
			say("You can try combination with sash of same")
			say("grade. Two sash combined can generate a")
			say("sash of a biggest grade.")
			say("")
			say("In a absorption the sash take the bonus")
			say("of a weapon or amour at a rate of your")
			say("sash. The absorption rate varies between")
			say("the sash grade, the weapon or amour")
			say("absorbed will be destroyed.")
			say("")
		end
		
		when 60003.chat."Combination" begin
			say_title(mob_name(60003))
			say("")
			say("Would you like to combine two sashes?")
			say("")
			local confirm = select("Yes", "No")
			if confirm == 2 then
				return
			end
			
			setskin(NOWINDOW)
			pc.open_sash(true)
		end
		
		when 60003.chat."Absorption of bonuses" begin
			say_title(mob_name(60003))
			say("")
			say("Do you want to absorb bonuses from your weapon")
			say("or amour?")
			say("")
			local confirm = select("Yes", "No")
			if confirm == 2 then
				return
			end
			
			setskin(NOWINDOW)
			pc.open_sash(false)
		end
	end
end