//! The NPCs a map's regen files stand up at boot, and the positions its mini-map lists.
//!
//! # What legacy does
//!
//! `SECTREE_MANAGER::Build` reads each hosted map's four regen files through `regen_load`
//! (`G/regen.cpp:601-720`; [`gamedata::regen`] reads them). For every entry, in file order:
//!
//! - A `REGEN_TYPE_MOB` entry whose mob proto is `CHAR_TYPE_NPC`, `CHAR_TYPE_WARP` or
//!   `CHAR_TYPE_GOTO` is listed with `InsertNPCPosition` at the centre of its box, less the map's
//!   base, whatever its regen time. That list is what `SendNPCPosition` sends a client entering
//!   the map.
//! - An entry with a regen time spawns at once through `regen_spawn` (`G/regen.cpp:325-383`),
//!   which runs `max_count` times. An anywhere entry calls `SpawnMobRandomPosition`. An entry
//!   whose box is one point calls `SpawnMob` there, whatever its kind, with the entry's direction
//!   as the rotation: `(direction - 1) * 45`, or `number(0, 7) * 45` when the direction is 0. A
//!   mob entry with a box calls `SpawnMobRange`, which tries 16 points of `number(sx, ex)`,
//!   `number(sy, ey)` and keeps the first `SpawnMob` places. A group entry with a box calls
//!   `SpawnGroup` or `SpawnGroupGroup`.
//!
//! `SpawnMob` (`G/char_manager.cpp:382-471`) answers nothing for a vnum with no proto. It checks
//! the cell attributes of anything that is not an NPC, warp or goto, and of an ore vein
//! (`mining::IsVeinOfOre`). It answers nothing for a point with no sectree. Only then does it
//! create the character and, when the caller passed no rotation, draw `number(0, 360)`. The
//! proto gives the level, the empire and the speeds, and an NPC with no empire takes its map's
//! (`GetEmpireFromMapIndex`).
//!
//! # What this module ports
//!
//! The NPCs: the characters of type NPC, warp and goto, ore veins excepted, which stand where
//! they spawn and never move. It keeps legacy's draws in legacy's order, so the same numbers
//! place the same NPCs. Monsters, stones, groups, anywhere spawns and ore veins are counted in
//! [`SpawnReport::unported`] and not spawned: they need the cell attributes and the monster AI.
//!
//! # Divergences
//!
//! - **VIDs.** Legacy numbers every character from one counter. The Rewrite names a player by its
//!   store id, so NPCs are numbered from [`FIRST_NPC_VID`] up, which no player id reaches.
//! - **No regen event.** Legacy schedules a regen event after the first spawn, which respawns a
//!   character that died. An NPC cannot die, and nothing here is killed, so no event is kept and
//!   its `number(0, 16)` is not drawn.
//! - **The sectree.** Legacy keys a sectree by 16 bits of `x / 6400` and `y / 6400`, taken from the
//!   coordinate as a `DWORD`, so a point far outside a map, or a negative one, can land in one of
//!   its sectrees and spawn there. That is a Defect, not reproduced: a point spawns only when the
//!   map's region holds it ([`MapRegion::contains`]). The owner's files place no NPC outside its
//!   map.

use std::error::Error;
use std::fmt;
use std::ops::RangeInclusive;

use gamedata::map_atlas::MapRegion;
use gamedata::mob_locale_names::MobLocaleNames;
use gamedata::mob_proto::{MobProtos, CHAR_TYPE_GOTO, CHAR_TYPE_NPC, CHAR_TYPE_WARP};
use gamedata::records::MobTableRecord;
use gamedata::regen::{RegenEntry, RegenKind};

use crate::character::{number, Dice};

/// The first VID an NPC takes: above every player id, which is a positive `int`.
pub const FIRST_NPC_VID: u32 = 0x8000_0000;

/// The tries `SpawnMobRange` makes to find a point.
const RANGE_TRIES: usize = 16;

/// `GetLimitPoint`'s ceiling on a non-player's speeds.
const SPEED_LIMIT: i16 = 250;

/// The ore veins of `mining::info` (`G/mining.cpp:29-50`), which `SpawnMob` treats as monsters.
const ORE_VEINS: [RangeInclusive<u32>; 2] = [20_047..=20_059, 30_301..=30_306];

/// An NPC standing on a map, with what its insert packets carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Npc {
    /// Its VID.
    pub vid: u32,
    /// Its mob vnum.
    pub vnum: u32,
    /// `GetRaceNum`: the vnum's low 16 bits, the `WORD` the client is sent.
    pub race: u16,
    /// Its proto's `CHAR_TYPE_*`.
    pub char_type: u8,
    /// Its x position.
    pub x: i32,
    /// Its y position.
    pub y: i32,
    /// Its z: the entry's `z_section` at a point, 0 in a box.
    pub z: i32,
    /// Its rotation in degrees.
    pub rotation: u16,
    /// Its empire.
    pub empire: u8,
    /// Its moving speed, limited to 0 to 250.
    pub moving_speed: u8,
    /// Its attack speed, limited to 0 to 250.
    pub attack_speed: u8,
    /// `GetName`: the `LOCALE_YMIR` mob name of its race.
    pub name: Vec<u8>,
}

/// A mini-map entry: `npc_info` of `InsertNPCPosition`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpcPosition {
    /// Its proto's `CHAR_TYPE_*`.
    pub char_type: u8,
    /// Its proto's `szLocaleName`, up to the first NUL.
    pub name: Vec<u8>,
    /// Its x position, from the map's base.
    pub x: i32,
    /// Its y position, from the map's base.
    pub y: i32,
}

/// What one map holds: its NPCs in spawn order and its mini-map list in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapNpcs {
    /// The NPCs standing on the map.
    pub npcs: Vec<Npc>,
    /// The mini-map list.
    pub positions: Vec<NpcPosition>,
}

/// What the spawner did not stand up, in characters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpawnReport {
    /// The NPCs it stood up.
    pub spawned: u64,
    /// Characters legacy spawns that this port does not: monsters, stones, ore veins, groups
    /// and anywhere spawns.
    pub unported: u64,
    /// Characters whose vnum has no proto, which legacy spawns none of either.
    pub no_proto: u64,
    /// NPCs no point placed inside the map, which legacy spawns none of either.
    pub unplaced: u64,
    /// Entries with no regen time, which legacy never spawns.
    pub idle: u64,
}

/// The NPC VIDs, handed out in order from [`FIRST_NPC_VID`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpcVids {
    next: Option<u32>,
}

impl Default for NpcVids {
    fn default() -> Self {
        Self::starting_at(FIRST_NPC_VID)
    }
}

impl NpcVids {
    /// A counter whose first VID is `first`.
    #[must_use]
    pub const fn starting_at(first: u32) -> Self {
        Self { next: Some(first) }
    }

    /// The next VID.
    ///
    /// # Errors
    ///
    /// Returns [`NpcVidsExhausted`] once `u32::MAX` has been handed out.
    pub fn take(&mut self) -> Result<u32, NpcVidsExhausted> {
        let vid = self.next.ok_or(NpcVidsExhausted)?;
        self.next = vid.checked_add(1);
        Ok(vid)
    }
}

/// Every NPC VID has been handed out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NpcVidsExhausted;

impl fmt::Display for NpcVidsExhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("every NPC VID is taken")
    }
}

impl Error for NpcVidsExhausted {}

/// `GetEmpireFromMapIndex` (`G/sectree_manager.cpp:1135-1166`): the empire a map belongs to.
#[must_use]
pub fn map_empire(map: i32) -> u8 {
    match map {
        1..=20 | 184 | 185 | 190 => 1,
        21..=40 | 186 | 187 | 191 => 2,
        41..=60 | 188 | 189 | 192 => 3,
        _ => 0,
    }
}

/// Whether `regen_load` lists a proto on the mini-map: an NPC, warp or goto.
fn is_listed(proto: &MobTableRecord) -> bool {
    matches!(
        proto.mob_type,
        CHAR_TYPE_NPC | CHAR_TYPE_WARP | CHAR_TYPE_GOTO
    )
}

/// Whether `SpawnMob` places a proto without its cell checks: one it lists that is not an ore
/// vein.
fn is_ported(proto: &MobTableRecord) -> bool {
    is_listed(proto) && !ORE_VEINS.iter().any(|veins| veins.contains(&proto.vnum))
}

/// `GetLimitPoint` of a non-player's speed: 0 to 250.
fn limit_speed(speed: i16) -> u8 {
    u8::try_from(speed.clamp(0, SPEED_LIMIT)).unwrap_or(u8::MAX)
}

/// A rotation `number` drew, which its bounds keep within a `u16`.
fn degrees(draw: i32) -> u16 {
    u16::try_from(draw).unwrap_or_default()
}

/// A `DWORD` vnum from the `int` the regen file holds.
fn vnum_bits(vnum: i32) -> u32 {
    u32::from_le_bytes(vnum.to_le_bytes())
}

/// `GetRaceNum`: the vnum's low 16 bits.
fn race_of(vnum: u32) -> u16 {
    let [low, high, ..] = vnum.to_le_bytes();
    u16::from_le_bytes([low, high])
}

/// A fixed name field up to its first NUL, as `strlcpy` reads it.
fn c_name(field: &[u8]) -> Vec<u8> {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    field[..end].to_vec()
}

/// Stands up the NPCs of each map from its regen entries.
#[derive(Debug)]
pub struct NpcSpawner<'a> {
    protos: &'a MobProtos,
    names: &'a MobLocaleNames,
    vids: NpcVids,
    report: SpawnReport,
}

/// Where one `SpawnMob` call puts its character.
struct Spot<'a> {
    region: &'a MapRegion,
    x: i32,
    y: i32,
    z: i32,
}

impl<'a> NpcSpawner<'a> {
    /// A spawner reading these protos and names, numbering NPCs from `vids`.
    #[must_use]
    pub fn new(protos: &'a MobProtos, names: &'a MobLocaleNames, vids: NpcVids) -> Self {
        Self {
            protos,
            names,
            vids,
            report: SpawnReport::default(),
        }
    }

    /// What every map spawned so far did not stand up.
    #[must_use]
    pub fn report(&self) -> SpawnReport {
        self.report
    }

    /// `regen_load`'s work for one map: its mini-map list and its first spawn, entry by entry.
    ///
    /// # Errors
    ///
    /// Returns [`NpcVidsExhausted`] when the NPCs outnumber the VIDs left.
    pub fn spawn_map(
        &mut self,
        region: &MapRegion,
        entries: &[RegenEntry],
        dice: &mut dyn Dice,
    ) -> Result<MapNpcs, NpcVidsExhausted> {
        let mut map = MapNpcs::default();
        for entry in entries.iter().filter(|entry| entry.spawns()) {
            let proto = self.protos.get(vnum_bits(entry.vnum));
            if let (RegenKind::Mob, Some(proto)) = (entry.kind, proto) {
                if is_listed(proto) {
                    map.positions.push(position(proto, entry, region));
                }
            }
            if entry.time == 0 {
                self.report.idle += 1;
                continue;
            }
            self.regen_spawn(region, entry, dice, &mut map.npcs)?;
        }
        Ok(map)
    }

    /// `regen_spawn(regen, false)`: `max_count` characters.
    fn regen_spawn(
        &mut self,
        region: &MapRegion,
        entry: &RegenEntry,
        dice: &mut dyn Dice,
        npcs: &mut Vec<Npc>,
    ) -> Result<(), NpcVidsExhausted> {
        let count = entry.max_count.max(0).unsigned_abs();
        if entry.kind == RegenKind::Anywhere {
            self.report.unported += u64::from(count);
            return Ok(());
        }
        if entry.sx == entry.ex && entry.sy == entry.ey {
            let spot = Spot {
                region,
                x: entry.sx,
                y: entry.sy,
                z: i32::from(entry.z_section),
            };
            for _ in 0..count {
                let rotation = if entry.direction == 0 {
                    degrees(number(dice, 0, 7) * 45)
                } else {
                    u16::from(entry.direction - 1) * 45
                };
                self.spawn_mob(entry.vnum, &spot, Some(rotation), dice, npcs)?;
            }
            return Ok(());
        }
        if entry.kind != RegenKind::Mob {
            self.report.unported += u64::from(count);
            return Ok(());
        }
        for _ in 0..count {
            self.spawn_mob_range(region, entry, dice, npcs)?;
        }
        Ok(())
    }

    /// `SpawnMobRange`: 16 tries at a point in the entry's box.
    fn spawn_mob_range(
        &mut self,
        region: &MapRegion,
        entry: &RegenEntry,
        dice: &mut dyn Dice,
        npcs: &mut Vec<Npc>,
    ) -> Result<(), NpcVidsExhausted> {
        let Some(proto) = self.protos.get(vnum_bits(entry.vnum)) else {
            self.report.no_proto += 1;
            return Ok(());
        };
        if !is_ported(proto) {
            self.report.unported += 1;
            return Ok(());
        }
        for _ in 0..RANGE_TRIES {
            let x = number(dice, entry.sx, entry.ex);
            let y = number(dice, entry.sy, entry.ey);
            if region.contains(x, y) {
                let spot = Spot { region, x, y, z: 0 };
                return self.spawn_mob(entry.vnum, &spot, None, dice, npcs);
            }
        }
        self.report.unplaced += 1;
        Ok(())
    }

    /// `SpawnMob` of a character with no cell checks: the proto, the sectree, then the NPC.
    fn spawn_mob(
        &mut self,
        vnum: i32,
        spot: &Spot<'_>,
        rotation: Option<u16>,
        dice: &mut dyn Dice,
        npcs: &mut Vec<Npc>,
    ) -> Result<(), NpcVidsExhausted> {
        let Some(proto) = self.protos.get(vnum_bits(vnum)) else {
            self.report.no_proto += 1;
            return Ok(());
        };
        if !is_ported(proto) {
            self.report.unported += 1;
            return Ok(());
        }
        if !spot.region.contains(spot.x, spot.y) {
            self.report.unplaced += 1;
            return Ok(());
        }
        let vid = self.vids.take()?;
        let rotation = rotation.unwrap_or_else(|| degrees(number(dice, 0, 360)));
        let empire = if proto.mob_type == CHAR_TYPE_NPC && proto.empire == 0 {
            map_empire(spot.region.index)
        } else {
            proto.empire
        };
        let race = race_of(proto.vnum);
        npcs.push(Npc {
            vid,
            vnum: proto.vnum,
            race,
            char_type: proto.mob_type,
            x: spot.x,
            y: spot.y,
            z: spot.z,
            rotation,
            empire,
            moving_speed: limit_speed(proto.moving_speed),
            attack_speed: limit_speed(proto.attack_speed),
            name: self.names.find(u32::from(race)).to_vec(),
        });
        self.report.spawned += 1;
        Ok(())
    }
}

/// `InsertNPCPosition`'s entry: the box's centre, less the map's base.
///
/// The centre lies between the box's placed corners, which are the file's corners plus the
/// base, so taking the base back off cannot leave the `int` range.
fn position(proto: &MobTableRecord, entry: &RegenEntry, region: &MapRegion) -> NpcPosition {
    let (x, y) = entry.centre();
    NpcPosition {
        char_type: proto.mob_type,
        name: c_name(&proto.locale_name),
        x: x.saturating_sub(region.sx),
        y: y.saturating_sub(region.sy),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};

    use gamedata::map_atlas::MapAtlas;
    use gamedata::mob_proto::{MobProto, CHAR_TYPE_MONSTER, CHAR_TYPE_STONE};
    use gamedata::records::MOB_LOCALE_NAME_BYTES;
    use gamedata::regen;

    use super::*;

    /// Answers the draws a test scripts, in order, and fails a test that draws one more.
    struct Script(VecDeque<u32>);

    impl Script {
        fn new(draws: &[u32]) -> Self {
            Self(draws.iter().copied().collect())
        }

        fn left(&self) -> usize {
            self.0.len()
        }
    }

    impl Dice for Script {
        fn random31(&mut self) -> u32 {
            self.0.pop_front().expect("a draw the test scripted")
        }
    }

    fn name_field(name: &[u8]) -> [u8; MOB_LOCALE_NAME_BYTES] {
        let mut field = [0; MOB_LOCALE_NAME_BYTES];
        field[..name.len()].copy_from_slice(name);
        field
    }

    fn proto(vnum: u32, mob_type: u8, empire: u8) -> MobProto {
        MobProto {
            line: 2,
            table: MobTableRecord {
                vnum,
                mob_type,
                empire,
                moving_speed: 100,
                attack_speed: 90,
                locale_name: name_field(b"Proto name"),
                ..MobTableRecord::default()
            },
        }
    }

    fn protos() -> MobProtos {
        MobProtos::from_rows(vec![
            proto(20_016, CHAR_TYPE_NPC, 0),
            proto(20_017, CHAR_TYPE_NPC, 3),
            proto(20_018, CHAR_TYPE_WARP, 0),
            proto(20_019, CHAR_TYPE_GOTO, 0),
            proto(20_047, CHAR_TYPE_NPC, 0),
            proto(30_306, CHAR_TYPE_NPC, 0),
            proto(101, CHAR_TYPE_MONSTER, 0),
            proto(8_001, CHAR_TYPE_STONE, 0),
            proto(0x0001_4E30, CHAR_TYPE_NPC, 0),
        ])
    }

    fn names() -> MobLocaleNames {
        MobLocaleNames::parse(b"VNUM\tNAME\n20016\tSmith\n20017\tGuard\n").unwrap()
    }

    fn region(index: i32) -> MapRegion {
        MapRegion {
            index,
            name: b"test".to_vec(),
            sx: 10_000,
            sy: 20_000,
            ex: 12_000,
            ey: 22_000,
            spawn: (0, 0),
            empire_spawns: None,
        }
    }

    fn entry(kind: RegenKind, vnum: i32, (sx, sy): (i32, i32), (ex, ey): (i32, i32)) -> RegenEntry {
        RegenEntry {
            kind,
            sx,
            sy,
            ex,
            ey,
            z_section: 0,
            direction: 1,
            time: 60,
            max_count: 1,
            vnum,
        }
    }

    fn point(vnum: i32, x: i32, y: i32) -> RegenEntry {
        entry(RegenKind::Mob, vnum, (x, y), (x, y))
    }

    fn spawn(entries: &[RegenEntry], dice: &mut Script) -> (MapNpcs, SpawnReport) {
        let (protos, names) = (protos(), names());
        let mut spawner = NpcSpawner::new(&protos, &names, NpcVids::default());
        let map = spawner.spawn_map(&region(1), entries, dice).unwrap();
        (map, spawner.report())
    }

    /// `GetEmpireFromMapIndex`, at every edge of its table.
    #[test]
    fn a_map_belongs_to_the_empire_legacy_names() {
        for (map, empire) in [
            (0, 0),
            (1, 1),
            (20, 1),
            (21, 2),
            (40, 2),
            (41, 3),
            (60, 3),
            (61, 0),
            (183, 0),
            (184, 1),
            (185, 1),
            (186, 2),
            (187, 2),
            (188, 3),
            (189, 3),
            (190, 1),
            (191, 2),
            (192, 3),
            (193, 0),
            (-1, 0),
        ] {
            assert_eq!(map_empire(map), empire, "map {map}");
        }
    }

    /// An NPC at a point stands there with the entry's z and direction, and the proto's speeds.
    #[test]
    fn an_npc_at_a_point_stands_as_its_entry_says() {
        let mut at = point(20_016, 10_500, 20_700);
        at.z_section = 7;
        at.direction = 3;
        let (map, report) = spawn(&[at], &mut Script::new(&[]));
        assert_eq!(
            map.npcs,
            [Npc {
                vid: FIRST_NPC_VID,
                vnum: 20_016,
                race: 20_016,
                char_type: CHAR_TYPE_NPC,
                x: 10_500,
                y: 20_700,
                z: 7,
                rotation: 90,
                empire: 1,
                moving_speed: 100,
                attack_speed: 90,
                name: b"Smith".to_vec(),
            }]
        );
        assert_eq!(report.spawned, 1);
    }

    /// Direction 0 draws `number(0, 7) * 45` for each character; any other names its angle.
    #[test]
    fn the_direction_sets_or_draws_the_rotation() {
        let mut drawn = point(20_016, 10_500, 20_700);
        drawn.direction = 0;
        drawn.max_count = 2;
        let mut last = point(20_016, 10_600, 20_700);
        last.direction = 255;
        let mut dice = Script::new(&[3, 0x7fff_ffff]);
        let (map, _) = spawn(&[drawn, last], &mut dice);
        let rotations: Vec<u16> = map.npcs.iter().map(|n| n.rotation).collect();
        assert_eq!(rotations, [135, 315, 254 * 45]);
        let vids: Vec<u32> = map.npcs.iter().map(|n| n.vid).collect();
        assert_eq!(vids, [FIRST_NPC_VID, FIRST_NPC_VID + 1, FIRST_NPC_VID + 2]);
        assert_eq!(dice.left(), 0);
    }

    /// A point draws its rotation before `SpawnMob` looks at the proto, as the argument is.
    #[test]
    fn a_point_draws_its_rotation_even_for_a_character_it_does_not_spawn() {
        let mut missing = point(1, 10_500, 20_700);
        missing.direction = 0;
        let mut dice = Script::new(&[5]);
        let (map, report) = spawn(&[missing], &mut dice);
        assert!(map.npcs.is_empty());
        assert_eq!(report.no_proto, 1);
        assert_eq!(dice.left(), 0);
    }

    /// A box tries points until one is inside the map, then draws `number(0, 360)`.
    #[test]
    fn a_box_tries_points_until_the_map_holds_one() {
        let boxed = entry(RegenKind::Mob, 20_016, (11_500, 21_000), (12_500, 21_100));
        // 11_500 + 900 = 12_400 is past the map; 11_500 + 100 = 11_600 is inside.
        let mut dice = Script::new(&[900, 50, 100, 20, 361 + 42]);
        let (map, report) = spawn(&[boxed], &mut dice);
        assert_eq!(map.npcs.len(), 1);
        let npc = &map.npcs[0];
        assert_eq!((npc.x, npc.y, npc.z, npc.rotation), (11_600, 21_020, 0, 42));
        assert_eq!(report.unplaced, 0);
        assert_eq!(dice.left(), 0);
    }

    /// A box's draws reach both of its far edges, and a draw of its width wraps to its near ones.
    #[test]
    fn a_box_draws_from_edge_to_edge() {
        let boxed = entry(RegenKind::Mob, 20_016, (10_100, 20_100), (10_300, 20_400));
        let mut dice = Script::new(&[200, 300, 0, 201, 301, 0]);
        let (map, _) = spawn(&[boxed, boxed], &mut dice);
        let points: Vec<(i32, i32)> = map.npcs.iter().map(|npc| (npc.x, npc.y)).collect();
        assert_eq!(points, [(10_300, 20_400), (10_100, 20_100)]);
        assert_eq!(dice.left(), 0);
    }

    /// A box that is a line on one axis is a box: it draws both coordinates and a rotation.
    #[test]
    fn a_line_is_a_box_not_a_point() {
        let upright = entry(RegenKind::Mob, 20_016, (11_000, 21_000), (11_000, 21_100));
        let flat = entry(RegenKind::Mob, 20_016, (11_000, 21_000), (11_100, 21_000));
        let mut dice = Script::new(&[7, 30, 42, 30, 7, 43]);
        let (map, _) = spawn(&[upright, flat], &mut dice);
        let placed: Vec<(i32, i32, u16)> = map
            .npcs
            .iter()
            .map(|npc| (npc.x, npc.y, npc.rotation))
            .collect();
        assert_eq!(placed, [(11_000, 21_030, 42), (11_030, 21_000, 43)]);
        assert_eq!(dice.left(), 0);
    }

    /// An exception entry marks an area; it spawns, lists and counts nothing.
    #[test]
    fn an_exception_entry_spawns_nothing() {
        let exception = entry(
            RegenKind::Exception,
            20_016,
            (10_100, 20_100),
            (10_100, 20_100),
        );
        let (map, report) = spawn(&[exception], &mut Script::new(&[]));
        assert_eq!(map, MapNpcs::default());
        assert_eq!(report, SpawnReport::default());
    }

    /// Sixteen points outside the map place nothing, and nothing draws a rotation.
    #[test]
    fn a_box_outside_the_map_places_nothing() {
        let outside = entry(RegenKind::Mob, 20_016, (30_000, 21_000), (30_100, 21_100));
        let mut dice = Script::new(&[0; 32]);
        let (map, report) = spawn(&[outside], &mut dice);
        assert!(map.npcs.is_empty());
        assert_eq!(report.unplaced, 1);
        assert_eq!(report.spawned, 0);
        assert_eq!(dice.left(), 0);
        let (_, report) = spawn(&[point(20_016, 12_000, 20_700)], &mut Script::new(&[]));
        assert_eq!(report.unplaced, 1, "the region's right edge is exclusive");
    }

    /// The sixteenth point is the last a box tries, and a seventeenth is never drawn.
    #[test]
    fn the_sixteenth_try_is_the_last() {
        let boxed = entry(RegenKind::Mob, 20_016, (11_500, 21_000), (12_500, 21_100));
        let mut draws = [900_u32, 0].repeat(15);
        draws.extend([100, 20, 42]);
        let mut dice = Script::new(&draws);
        let (map, report) = spawn(&[boxed], &mut dice);
        assert_eq!(report.unplaced, 0);
        assert_eq!((map.npcs[0].x, map.npcs[0].y), (11_600, 21_020));
        assert_eq!(dice.left(), 0);
        let mut draws = [900_u32, 0].repeat(16);
        draws.extend([100, 20]);
        let mut dice = Script::new(&draws);
        let (map, report) = spawn(&[boxed], &mut dice);
        assert!(map.npcs.is_empty());
        assert_eq!(report.unplaced, 1);
        assert_eq!(dice.left(), 2, "a seventeenth try is not made");
    }

    /// Every character of `max_count` spawns, and a count that is not positive spawns none.
    #[test]
    fn max_count_characters_spawn() {
        let mut three = point(20_016, 10_500, 20_700);
        three.max_count = 3;
        let mut none = point(20_016, 10_500, 20_700);
        none.max_count = 0;
        let mut negative = point(20_016, 10_500, 20_700);
        negative.max_count = -3;
        let (map, report) = spawn(&[three, none, negative], &mut Script::new(&[]));
        assert_eq!(map.npcs.len(), 3);
        assert_eq!(report.spawned, 3);
    }

    /// The mini-map lists a mob entry of an NPC, warp or goto, time or not, and nothing else.
    #[test]
    fn the_mini_map_lists_what_regen_load_lists() {
        let mut idle = entry(RegenKind::Mob, 20_018, (10_100, 20_100), (10_300, 20_401));
        idle.time = 0;
        let entries = [
            idle,
            point(20_019, 11_000, 21_000),
            point(20_047, 11_000, 21_000),
            point(101, 11_000, 21_000),
            entry(
                RegenKind::Group { aggressive: false },
                20_016,
                (10_100, 20_100),
                (10_100, 20_100),
            ),
            point(1, 11_000, 21_000),
            entry(
                RegenKind::Exception,
                20_016,
                (10_100, 20_100),
                (10_100, 20_100),
            ),
        ];
        let (map, report) = spawn(&entries, &mut Script::new(&[]));
        let listed: Vec<(u8, i32, i32)> = map
            .positions
            .iter()
            .map(|p| (p.char_type, p.x, p.y))
            .collect();
        assert_eq!(
            listed,
            [
                (CHAR_TYPE_WARP, 200, 250),
                (CHAR_TYPE_GOTO, 1_000, 1_000),
                (CHAR_TYPE_NPC, 1_000, 1_000),
            ]
        );
        assert_eq!(map.positions[0].name, b"Proto name");
        assert_eq!(report.idle, 1);
    }

    /// Monsters, stones, ore veins, groups and anywhere entries are counted, not spawned.
    #[test]
    fn what_this_port_does_not_spawn_is_counted() {
        let mut anywhere = entry(RegenKind::Anywhere, 20_016, (0, 0), (0, 0));
        anywhere.max_count = 4;
        let entries = [
            point(101, 11_000, 21_000),
            point(8_001, 11_000, 21_000),
            point(20_047, 11_000, 21_000),
            point(30_306, 11_000, 21_000),
            entry(RegenKind::Mob, 101, (10_100, 20_100), (10_300, 20_300)),
            entry(RegenKind::Mob, 20_047, (10_100, 20_100), (10_300, 20_300)),
            entry(
                RegenKind::Group { aggressive: true },
                20_016,
                (10_100, 20_100),
                (10_300, 20_300),
            ),
            entry(
                RegenKind::GroupGroup,
                20_016,
                (10_100, 20_100),
                (10_300, 20_300),
            ),
            anywhere,
        ];
        let (map, report) = spawn(&entries, &mut Script::new(&[]));
        assert!(map.npcs.is_empty());
        assert_eq!(report.unported, 12);
        let boxed = entry(RegenKind::Mob, 1, (10_100, 20_100), (10_300, 20_300));
        let (_, report) = spawn(&[boxed], &mut Script::new(&[]));
        assert_eq!(report.no_proto, 1, "a box with no proto draws nothing");
    }

    /// The ore veins are `mining::info`'s two ranges, both ends included, and nothing beside.
    #[test]
    fn the_ore_veins_are_minings_two_ranges() {
        let vein = |vnum| !is_ported(&proto(vnum, CHAR_TYPE_NPC, 0).table);
        let veins: Vec<u32> = (20_000..=30_400).filter(|&vnum| vein(vnum)).collect();
        let mining: Vec<u32> = (20_047..=20_059).chain(30_301..=30_306).collect();
        assert_eq!(veins, mining);
    }

    /// A group entry at a point is `SpawnMob` of the group's vnum, as legacy calls it.
    #[test]
    fn a_group_at_a_point_spawns_the_mob_of_its_vnum() {
        let group = entry(
            RegenKind::Group { aggressive: false },
            20_017,
            (10_100, 20_100),
            (10_100, 20_100),
        );
        let (map, _) = spawn(&[group], &mut Script::new(&[]));
        assert_eq!(map.npcs.len(), 1);
        assert!(map.positions.is_empty());
    }

    /// An NPC with no empire takes its map's; one with an empire, a warp or a goto keeps its own.
    #[test]
    fn an_npc_with_no_empire_takes_its_maps() {
        let entries = [
            point(20_016, 10_500, 20_700),
            point(20_017, 10_500, 20_700),
            point(20_018, 10_500, 20_700),
            point(20_019, 10_500, 20_700),
        ];
        let (protos, names) = (protos(), names());
        let mut spawner = NpcSpawner::new(&protos, &names, NpcVids::default());
        let map = spawner
            .spawn_map(&region(21), &entries, &mut Script::new(&[]))
            .unwrap();
        let empires: Vec<u8> = map.npcs.iter().map(|n| n.empire).collect();
        assert_eq!(empires, [2, 3, 0, 0]);
    }

    /// The race is the vnum's low 16 bits, and the name is the race's, not the proto's.
    #[test]
    fn the_race_and_name_are_the_low_bits() {
        let (map, _) = spawn(&[point(0x0001_4E30, 10_500, 20_700)], &mut Script::new(&[]));
        assert_eq!(map.npcs[0].vnum, 0x0001_4E30);
        assert_eq!(map.npcs[0].race, 0x4E30);
        assert_eq!(map.npcs[0].name, b"Smith", "0x4E30 is 20016");
        let (map, _) = spawn(&[point(20_018, 10_500, 20_700)], &mut Script::new(&[]));
        assert_eq!(map.npcs[0].name, gamedata::mob_locale_names::NO_NAME);
    }

    /// Speeds are limited to 0 to 250, as `GetLimitPoint` limits a non-player's.
    #[test]
    fn speeds_are_limited() {
        assert_eq!(limit_speed(i16::MIN), 0);
        assert_eq!(limit_speed(-1), 0);
        assert_eq!(limit_speed(0), 0);
        assert_eq!(limit_speed(250), 250);
        assert_eq!(limit_speed(251), 250);
        assert_eq!(limit_speed(i16::MAX), 250);
    }

    /// A name field is read up to its first NUL, or whole when it has none.
    #[test]
    fn a_name_field_ends_at_its_nul() {
        assert_eq!(c_name(b"ab\0cd"), b"ab");
        assert_eq!(c_name(b"abcd"), b"abcd");
        assert_eq!(c_name(b"\0"), b"");
    }

    /// The VIDs run out at `u32::MAX`, and a map that needs one more is refused.
    #[test]
    fn the_vids_run_out() {
        let mut vids = NpcVids::starting_at(u32::MAX);
        assert_eq!(vids.take(), Ok(u32::MAX));
        assert_eq!(vids.take(), Err(NpcVidsExhausted));
        let (protos, names) = (protos(), names());
        let mut spawner = NpcSpawner::new(&protos, &names, NpcVids::starting_at(u32::MAX));
        let mut two = point(20_016, 10_500, 20_700);
        two.max_count = 2;
        let refused = spawner.spawn_map(&region(1), &[two], &mut Script::new(&[]));
        assert_eq!(refused, Err(NpcVidsExhausted));
        assert_eq!(NpcVidsExhausted.to_string(), "every NPC VID is taken");
    }

    fn owners() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata")
    }

    /// The owner's map 1 stands up its NPCs, all inside the map, and lists its mini-map.
    #[test]
    fn the_owners_first_map_stands_up_its_npcs() {
        let root = owners();
        let protos = MobProtos::load(&root.join("proto")).unwrap();
        let names = MobLocaleNames::load(&root.join("locale/europe/country")).unwrap();
        let map_dir = root.join("locale/europe/map");
        let atlas = MapAtlas::load(&map_dir).unwrap();
        let region = atlas.region(1).unwrap();
        let entries = regen::load_map(&map_dir, region).unwrap();
        let mut spawner = NpcSpawner::new(&protos, &names, NpcVids::default());
        let mut dice = crate::character::Pcg32::new(1, 0);
        let map = spawner.spawn_map(region, &entries, &mut dice).unwrap();
        let report = spawner.report();
        assert!(!map.npcs.is_empty());
        assert_eq!(report.spawned, u64::try_from(map.npcs.len()).unwrap());
        assert_eq!(report.unplaced, 0);
        assert!(map.npcs.iter().all(|n| region.contains(n.x, n.y)));
        assert!(map
            .npcs
            .iter()
            .all(|n| n.empire == 1 || n.char_type != CHAR_TYPE_NPC));
        let first = &map.npcs[0];
        assert_eq!((first.vnum, first.x, first.y), (20_300, 471_800, 951_600));
        assert!(map.positions.iter().any(|p| p.name == b"Fierar"));
        let expected = SpawnReport {
            spawned: 48,
            unported: 1_079,
            ..SpawnReport::default()
        };
        assert_eq!(
            report, expected,
            "npc.txt's 48 mob entries, and the rest counted"
        );
        assert_eq!(map.positions.len(), 48);
    }
}
