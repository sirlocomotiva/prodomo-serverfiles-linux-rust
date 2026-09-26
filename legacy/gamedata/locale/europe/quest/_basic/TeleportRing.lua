quest teleportRing begin
	state start begin
		when 70058.use begin		
			TeleportRing.MainWindow(item.get_vnum());
		end
	end
end
