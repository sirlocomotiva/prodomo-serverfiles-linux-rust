fn main() {
    let dump = std::fs::read(\"legacy/sql/gamedata/player.sql\").unwrap();
    let skills = gamedata::guild_skill_data::GuildSkills::from_dump(&dump).unwrap();
    println!(\"vnums={:?} refused={:?}\", skills.vnums(), skills.refused());
    for v in skills.vnums() {
        let s = skills.get(v).unwrap();
        println!(\"{v} max={} flag={:#x} on={:?} on2={:?}\", s.record.max_level, s.record.flag, String::from_utf8_lossy(&s.record.point_on).trim_end_matches('\0'), String::from_utf8_lossy(&s.record.point_on2).trim_end_matches('\0'));
    }
}
