//! The characters a map's regen files stand up at boot, and the positions its mini-map lists.
//!
//! # What legacy does
//!
//! `SECTREE_MANAGER::Build` reads each hosted map's four regen files through `regen_load`
//! (`G/regen.cpp:601-710`; [`gamedata::regen`] reads them). For every entry, in file order:
//!
//! - A `REGEN_TYPE_MOB` entry whose mob proto is `CHAR_TYPE_NPC`, `CHAR_TYPE_WARP` or
//!   `CHAR_TYPE_GOTO` is listed with `InsertNPCPosition` at the centre of its box, less the map's
//!   base, whatever its regen time. That list is what `SendNPCPosition` sends a client entering
//!   the map.
//! - An entry with a regen time spawns at once through `regen_spawn` (`G/regen.cpp:325-383`),
//!   which runs `max_count` times, and then draws `number(0, 16)`: its `regen_event` first fires
//!   that many seconds plus its time later (`:679-689`).
//!
//! `regen_spawn` picks the spawner by the entry:
//!
//! - **Anywhere** calls `SpawnMobRandomPosition` (`G/char_manager.cpp:262-380`). Vnums 5001, 5002
//!   and 5004 spawn only under the event flags `japan_regen`, `newyear_mob` and
//!   `independence_day`, and are refused with no draw otherwise. Any other vnum makes up to 2000
//!   tries of `number(1, width / 100 - 1) * 100` and the same for the height, past the map's base,
//!   and stands at the first cell that is not blocked, built on or `BANPK`; then it draws
//!   `number(0, 360)` for its rotation, after `SetProto`.
//! - **A box of one point**, whatever its kind, calls `SpawnMob` there with the entry's direction
//!   as the rotation: `(direction - 1) * 45`, or `number(0, 7) * 45` when the direction is 0,
//!   drawn before the call. A group entry at a point spawns its group vnum as a mob vnum (D4).
//! - **A mob box** calls `SpawnMobRange` (`:473-506`). A stone forces its spawn motion. It tries
//!   16 points of `number(sx, ex)`, `number(sy, ey)` and keeps the first `SpawnMob` places.
//! - **A group box** calls `SpawnGroup` (`:595-667`): each member in turn is a `SpawnMobRange`
//!   in the box, and a member that stands draws four `number(300, 500)` for the next member's box
//!   around it. The group fails, with no further draw, when its leader does not stand; a later
//!   member that does not stand is skipped. The first member is the party's master and holds the
//!   regen; the others join its party. A `ga` entry makes every member aggressive.
//! - **A group-group box** calls `SpawnGroupGroup` (`:581-593`), which draws
//!   `number(1, members)` from the group-group (none when it is missing or empty) and spawns
//!   that group.
//!
//! `SpawnMob` (`G/char_manager.cpp:382-471`) answers nothing for a vnum with no proto. It checks
//! the cell of anything that is not an NPC, warp or goto, and of an ore vein: there must be a
//! sectree, the cell may not be blocked (an ore vein) or blocked or built on (anything else), and
//! it may not be `BANPK`. It answers nothing for a point with no sectree. Only then does it
//! create the character and, when the caller passed no rotation, draw `number(0, 360)`. In
//! `SetProto` an ore vein draws `number(420, 900)`, the seconds before `kill_ore_load_event`
//! destroys it (`G/char.cpp:2563-2570`). An NPC with no empire takes its map's
//! (`GetEmpireFromMapIndex`). `Show` with the spawn motion sets `AFF_SPAWN`, which nothing clears
//! (`G/char.cpp:1884-1913`, D5).
//!
//! # Divergences
//!
//! - **VIDs.** Legacy numbers every character from one counter. The Rewrite names a player by its
//!   store id, so the others are numbered from [`FIRST_NPC_VID`] up, which no player id reaches.
//! - **The sectree.** Legacy keys a sectree by 16 bits of `x / 6400` and `y / 6400`, taken from the
//!   coordinate as a `DWORD`, so a point far outside a map, or a negative one, can land in one of
//!   its sectrees and spawn there. That is a Defect, not reproduced: a point spawns only where the
//!   map's grid builds a sectree ([`MapCells::attribute`]).
//! - **The event flags.** No GM command sets an event flag yet, so the three gated vnums never
//!   spawn anywhere, which is what legacy does with the flags unset. The `test_server` notice an
//!   anywhere spawn sends (`G/char_manager.cpp:375-376`) is not sent; the owner's files have no
//!   anywhere entry that is not gated.

use std::error::Error;
use std::fmt;
use std::ops::RangeInclusive;

use gamedata::map_atlas::MapRegion;
use gamedata::mob_group::MobGroups;
use gamedata::mob_locale_names::MobLocaleNames;
use gamedata::mob_proto::{
    MobProtos, AIFLAG_AGGRESSIVE, CHAR_TYPE_GOTO, CHAR_TYPE_NPC, CHAR_TYPE_STONE, CHAR_TYPE_WARP,
};
use gamedata::records::MobTableRecord;
use gamedata::regen::{RegenEntry, RegenKind};
use gamedata::server_attr::{ATTR_BANPK, ATTR_BLOCK, ATTR_OBJECT};

use crate::cells::MapCells;
use crate::character::{number, Dice};

/// The first VID a non-player takes: above every player id, which is a positive `int`.
pub const FIRST_NPC_VID: u32 = 0x8000_0000;

/// `AFF_SPAWN` (`G/affect.h:206-212`): the affect `Show` sets for the spawn motion, bit
/// `AFF_SPAWN - 1` of the first affect word.
pub const AFF_SPAWN: u32 = 3;

/// The tries `SpawnMobRange` makes to find a point.
const RANGE_TRIES: usize = 16;

/// The tries `SpawnMobRandomPosition` makes to find a cell.
const ANYWHERE_TRIES: usize = 2000;

/// `GetLimitPoint`'s ceiling on a non-player's speeds.
const SPEED_LIMIT: i16 = 250;

/// The ore veins of `mining::info` (`G/mining.cpp:29-50`), which `SpawnMob` treats as monsters.
const ORE_VEINS: [RangeInclusive<u32>; 2] = [20_047..=20_059, 30_301..=30_306];

/// The anywhere vnums an event flag gates (`G/char_manager.cpp:264-289`): `japan_regen`,
/// `newyear_mob` and `independence_day`.
const GATED_ANYWHERE: [u32; 3] = [5001, 5002, 5004];

/// The seconds `kill_ore_load_event` waits: `number(7 * 60, 15 * 60)`.
const ORE_LIFETIME: (i32, i32) = (7 * 60, 15 * 60);

/// The half-sides `SpawnGroup` draws around a member for the next one's box.
const GROUP_SPREAD: (i32, i32) = (300, 500);

/// The seconds `regen_load` adds to an entry's first fire: `number(0, 16)`.
const FIRST_FIRE_JITTER: (i32, i32) = (0, 16);

/// A non-player standing on a map: an NPC, a warp, a goto, a monster or a stone, with what its
/// insert packets carry and what its regen and group made it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Npc {
    /// Its VID.
    pub vid: u32,
    /// Its mob vnum.
    pub vnum: u32,
    /// `GetRaceNum`: the vnum's low 16 bits, the `WORD` the client is sent.
    pub race: u16,
    /// Its proto's `CHAR_TYPE_*`.
    pub char_type: u8,
    /// Its proto's `ON_CLICK_*`: which `m_triggerOnClick` a click on it runs
    /// (`char.cpp:6341-6350`).
    pub on_click: u8,
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
    /// `m_pointsInstant.dwAIFlag`: the proto's AI flags, with `AIFLAG_AGGRESSIVE` added when its
    /// spawn made it aggressive (`SetAggressive`).
    pub ai_flags: u32,
    /// `m_afAffectFlag.bits`, which its insert carries: `AFF_SPAWN` when it was shown with the
    /// spawn motion.
    pub affect_flags: [u32; 2],
    /// The VID of its group's leader, the party master, when it is a follower.
    pub leader: Option<u32>,
    /// The regen it holds (`SetRegen`), by its place in [`MapNpcs::regens`]: a lone character's
    /// and a group leader's.
    pub regen: Option<usize>,
    /// The seconds before `kill_ore_load_event` destroys it, for an ore vein.
    pub ore_lifetime: Option<u32>,
}

impl Npc {
    /// `IsAggressive`.
    #[must_use]
    pub fn is_aggressive(&self) -> bool {
        self.ai_flags & AIFLAG_AGGRESSIVE != 0
    }
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

/// One timed regen entry as boot leaves it: legacy `REGEN` with its `count` and its event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Regen {
    /// The entry.
    pub entry: RegenEntry,
    /// `count`: the characters that hold it and stand, each a lone character or a group leader.
    pub count: u32,
    /// The seconds after boot its `regen_event` first fires: `number(0, 16)` plus its time.
    pub first_fire: u32,
}

/// What one map holds: its characters in spawn order, its mini-map list in file order, and its
/// timed regens in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapNpcs {
    /// The characters standing on the map.
    pub npcs: Vec<Npc>,
    /// The mini-map list.
    pub positions: Vec<NpcPosition>,
    /// The entries with a regen time.
    pub regens: Vec<Regen>,
}

/// What the spawner stood up and what it did not, in characters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpawnReport {
    /// The characters it stood up.
    pub spawned: u64,
    /// Of those, the stones.
    pub stones: u64,
    /// Anywhere characters an event flag holds back.
    pub gated: u64,
    /// Characters whose vnum has no proto, which legacy spawns none of either.
    pub no_proto: u64,
    /// Group spawns whose group or group-group is missing or empty.
    pub no_group: u64,
    /// Characters no point placed, which legacy spawns none of either.
    pub unplaced: u64,
    /// Entries with no regen time, which legacy never spawns.
    pub idle: u64,
}

/// The non-player VIDs, handed out in order from [`FIRST_NPC_VID`].
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

/// Every non-player VID has been handed out.
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

/// `mining::IsVeinOfOre`.
fn is_vein(vnum: u32) -> bool {
    ORE_VEINS.iter().any(|veins| veins.contains(&vnum))
}

/// `GetLimitPoint` of a non-player's speed: 0 to 250.
fn limit_speed(speed: i16) -> u8 {
    u8::try_from(speed.clamp(0, SPEED_LIMIT)).unwrap_or(u8::MAX)
}

/// A rotation `number` drew, which its bounds keep within a `u16`.
fn degrees(draw: i32) -> u16 {
    u16::try_from(draw).unwrap_or_default()
}

/// A count of seconds `number` drew, which its bounds keep positive.
fn seconds(draw: i32) -> u32 {
    u32::try_from(draw).unwrap_or_default()
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

/// Stands up the non-players of each map from its regen entries.
#[derive(Debug)]
pub struct NpcSpawner<'a> {
    protos: &'a MobProtos,
    groups: &'a MobGroups,
    names: &'a MobLocaleNames,
    vids: NpcVids,
    report: SpawnReport,
}

/// The map one `regen_load` spawns on, and what it has stood up so far.
struct Site<'a> {
    region: &'a MapRegion,
    cells: &'a MapCells,
    npcs: Vec<Npc>,
}

/// How one `SpawnMob` call places its character.
#[derive(Clone, Copy)]
struct Place {
    x: i32,
    y: i32,
    z: i32,
    spawn_motion: bool,
    /// The rotation the caller passed, or [`Facing::Drawn`] for `-1`.
    facing: Facing,
}

/// Where a new character's rotation comes from.
#[derive(Clone, Copy)]
enum Facing {
    /// The caller passed it.
    Given(u16),
    /// `SpawnMob` was passed `-1` and draws `number(0, 360)` before `SetProto`.
    Drawn,
    /// `SpawnMobRandomPosition` draws it after `SetProto`.
    Later,
}

/// A box `SpawnMobRange` and `SpawnGroup` draw points from, corners included.
#[derive(Clone, Copy)]
struct Area {
    sx: i32,
    sy: i32,
    ex: i32,
    ey: i32,
}

impl Area {
    fn of(entry: &RegenEntry) -> Self {
        Self {
            sx: entry.sx,
            sy: entry.sy,
            ex: entry.ex,
            ey: entry.ey,
        }
    }
}

impl<'a> NpcSpawner<'a> {
    /// A spawner reading these protos, groups and names, numbering characters from `vids`.
    #[must_use]
    pub fn new(
        protos: &'a MobProtos,
        groups: &'a MobGroups,
        names: &'a MobLocaleNames,
        vids: NpcVids,
    ) -> Self {
        Self {
            protos,
            groups,
            names,
            vids,
            report: SpawnReport::default(),
        }
    }

    /// What every map spawned so far stood up and did not.
    #[must_use]
    pub fn report(&self) -> SpawnReport {
        self.report
    }

    /// `regen_load`'s work for one map: its mini-map list, its first spawn and its regens, entry
    /// by entry, with the map's cells to check each point against.
    ///
    /// # Errors
    ///
    /// Returns [`NpcVidsExhausted`] when the characters outnumber the VIDs left.
    pub fn spawn_map(
        &mut self,
        region: &MapRegion,
        cells: &MapCells,
        entries: &[RegenEntry],
        dice: &mut dyn Dice,
    ) -> Result<MapNpcs, NpcVidsExhausted> {
        let mut site = Site {
            region,
            cells,
            npcs: Vec::new(),
        };
        let mut positions = Vec::new();
        let mut regens = Vec::new();
        for entry in entries.iter().filter(|entry| entry.spawns()) {
            let proto = self.protos.get(vnum_bits(entry.vnum));
            if let (RegenKind::Mob, Some(proto)) = (entry.kind, proto) {
                if is_listed(proto) {
                    positions.push(position(proto, entry, region));
                }
            }
            if entry.time == 0 {
                self.report.idle += 1;
                continue;
            }
            let count = self.regen_spawn(&mut site, entry, regens.len(), dice)?;
            let jitter = seconds(number(dice, FIRST_FIRE_JITTER.0, FIRST_FIRE_JITTER.1));
            regens.push(Regen {
                entry: *entry,
                count,
                first_fire: jitter.saturating_add(entry.time),
            });
        }
        Ok(MapNpcs {
            npcs: site.npcs,
            positions,
            regens,
        })
    }

    /// `regen_spawn(regen, false)`: `max_count` units, each holding regen `regen`. Answers the
    /// units that stood, legacy's `count`.
    fn regen_spawn(
        &mut self,
        site: &mut Site<'_>,
        entry: &RegenEntry,
        regen: usize,
        dice: &mut dyn Dice,
    ) -> Result<u32, NpcVidsExhausted> {
        let units = entry.max_count.max(0).unsigned_abs();
        let vnum = vnum_bits(entry.vnum);
        let mut count = 0;
        for _ in 0..units {
            let stood = if entry.kind == RegenKind::Anywhere {
                self.spawn_mob_random_position(site, vnum, dice)?
            } else if entry.sx == entry.ex && entry.sy == entry.ey {
                let rotation = if entry.direction == 0 {
                    degrees(number(dice, 0, 7) * 45)
                } else {
                    u16::from(entry.direction - 1) * 45
                };
                let place = Place {
                    x: entry.sx,
                    y: entry.sy,
                    z: i32::from(entry.z_section),
                    spawn_motion: false,
                    facing: Facing::Given(rotation),
                };
                let stood = self.spawn_mob(site, vnum, place, dice)?;
                if stood.is_none() && self.protos.get(vnum).is_some() {
                    self.report.unplaced += 1;
                }
                stood
            } else {
                match entry.kind {
                    RegenKind::Group { aggressive } => {
                        self.spawn_group(site, vnum, Area::of(entry), aggressive, dice)?
                    }
                    RegenKind::GroupGroup => {
                        self.spawn_group_group(site, vnum, Area::of(entry), dice)?
                    }
                    // `m` passes its `is_aggressive`, which only `ga` sets, so a mob box is
                    // neither aggressive nor shown with the spawn motion unless it is a stone.
                    _ => self.spawn_mob_range(site, vnum, Area::of(entry), false, dice)?,
                }
            };
            if let Some(at) = stood {
                count += 1;
                site.npcs[at].regen = Some(regen);
            }
        }
        Ok(count)
    }

    /// `SpawnMobRandomPosition`: a gated vnum is refused; anything else tries 2000 cells of the
    /// map.
    fn spawn_mob_random_position(
        &mut self,
        site: &mut Site<'_>,
        vnum: u32,
        dice: &mut dyn Dice,
    ) -> Result<Option<usize>, NpcVidsExhausted> {
        if GATED_ANYWHERE.contains(&vnum) {
            self.report.gated += 1;
            return Ok(None);
        }
        let Some(proto) = self.protos.get(vnum) else {
            self.report.no_proto += 1;
            return Ok(None);
        };
        let region = site.region;
        let width = region.ex.saturating_sub(region.sx) / 100 - 1;
        let height = region.ey.saturating_sub(region.sy) / 100 - 1;
        for _ in 0..ANYWHERE_TRIES {
            let x = (number(dice, 1, width) * 100).saturating_add(region.sx);
            let y = (number(dice, 1, height) * 100).saturating_add(region.sy);
            let Some(attr) = site.cells.attribute(x, y) else {
                continue;
            };
            if attr & (ATTR_BLOCK | ATTR_OBJECT | ATTR_BANPK) != 0 {
                continue;
            }
            // `SetProto` comes before the rotation here, so an ore vein draws its lifetime first.
            let at = self.create(site, proto, (x, y, 0), false, Facing::Later, dice)?;
            site.npcs[at].rotation = degrees(number(dice, 0, 360));
            return Ok(Some(at));
        }
        self.report.unplaced += 1;
        Ok(None)
    }

    /// `SpawnGroupGroup`: one draw picks the group, uniformly (D1).
    fn spawn_group_group(
        &mut self,
        site: &mut Site<'_>,
        vnum: u32,
        area: Area,
        dice: &mut dyn Dice,
    ) -> Result<Option<usize>, NpcVidsExhausted> {
        let members = self.groups.group_group(vnum).unwrap_or_default();
        let Ok(last) = i32::try_from(members.len()) else {
            return Ok(None);
        };
        if last == 0 {
            self.report.no_group += 1;
            return Ok(None);
        }
        let picked = usize::try_from(number(dice, 1, last) - 1).unwrap_or_default();
        // A group-group is spawned with the regen's `is_aggressive`, which only `ga` sets.
        self.spawn_group(site, members[picked], area, false, dice)
    }

    /// `SpawnGroup` at boot, where no stone is selected: the leader in the box, then each member
    /// in a box drawn around the one before it.
    fn spawn_group(
        &mut self,
        site: &mut Site<'_>,
        vnum: u32,
        area: Area,
        aggressive: bool,
        dice: &mut dyn Dice,
    ) -> Result<Option<usize>, NpcVidsExhausted> {
        let groups = self.groups;
        let Some(group) = groups.group(vnum).filter(|_| vnum != 0) else {
            self.report.no_group += 1;
            return Ok(None);
        };
        let mut area = area;
        let mut leader: Option<usize> = None;
        for (i, &member) in group.members.iter().enumerate() {
            let Some(at) = self.spawn_mob_range(site, member, area, false, dice)? else {
                if i == 0 {
                    return Ok(None);
                }
                continue;
            };
            let (x, y) = (site.npcs[at].x, site.npcs[at].y);
            area.sx = x.saturating_sub(number(dice, GROUP_SPREAD.0, GROUP_SPREAD.1));
            area.sy = y.saturating_sub(number(dice, GROUP_SPREAD.0, GROUP_SPREAD.1));
            area.ex = x.saturating_add(number(dice, GROUP_SPREAD.0, GROUP_SPREAD.1));
            area.ey = y.saturating_add(number(dice, GROUP_SPREAD.0, GROUP_SPREAD.1));
            match leader {
                Some(master) => site.npcs[at].leader = Some(site.npcs[master].vid),
                None => leader = Some(at),
            }
            if aggressive {
                site.npcs[at].ai_flags |= AIFLAG_AGGRESSIVE;
            }
        }
        Ok(leader)
    }

    /// `SpawnMobRange`: 16 tries at a point in the box. A stone is shown with the spawn motion.
    fn spawn_mob_range(
        &mut self,
        site: &mut Site<'_>,
        vnum: u32,
        area: Area,
        spawn_motion: bool,
        dice: &mut dyn Dice,
    ) -> Result<Option<usize>, NpcVidsExhausted> {
        let Some(proto) = self.protos.get(vnum) else {
            self.report.no_proto += 1;
            return Ok(None);
        };
        let spawn_motion = spawn_motion || proto.mob_type == CHAR_TYPE_STONE;
        for _ in 0..RANGE_TRIES {
            let x = number(dice, area.sx, area.ex);
            let y = number(dice, area.sy, area.ey);
            let place = Place {
                x,
                y,
                z: 0,
                spawn_motion,
                facing: Facing::Drawn,
            };
            if let Some(at) = self.spawn_mob(site, vnum, place, dice)? {
                return Ok(Some(at));
            }
        }
        self.report.unplaced += 1;
        Ok(None)
    }

    /// `SpawnMob`: the proto, the cell checks, the sectree, then the character.
    fn spawn_mob(
        &mut self,
        site: &mut Site<'_>,
        vnum: u32,
        place: Place,
        dice: &mut dyn Dice,
    ) -> Result<Option<usize>, NpcVidsExhausted> {
        let Some(proto) = self.protos.get(vnum) else {
            self.report.no_proto += 1;
            return Ok(None);
        };
        let Some(attr) = site.cells.attribute(place.x, place.y) else {
            return Ok(None);
        };
        if !is_listed(proto) || is_vein(vnum) {
            let refused = if is_vein(vnum) {
                ATTR_BLOCK
            } else {
                ATTR_BLOCK | ATTR_OBJECT
            };
            if attr & (refused | ATTR_BANPK) != 0 {
                return Ok(None);
            }
        }
        let at = self.create(
            site,
            proto,
            (place.x, place.y, place.z),
            place.spawn_motion,
            place.facing,
            dice,
        )?;
        Ok(Some(at))
    }

    /// `CreateCharacter`, then `SetProto` and `Show`: the VID, the rotation, the ore vein's
    /// lifetime, the empire and the spawn motion.
    fn create(
        &mut self,
        site: &mut Site<'_>,
        proto: &MobTableRecord,
        (x, y, z): (i32, i32, i32),
        spawn_motion: bool,
        facing: Facing,
        dice: &mut dyn Dice,
    ) -> Result<usize, NpcVidsExhausted> {
        let vid = self.vids.take()?;
        let rotation = match facing {
            Facing::Given(given) => given,
            Facing::Drawn => degrees(number(dice, 0, 360)),
            Facing::Later => 0,
        };
        let race = race_of(proto.vnum);
        let ore_lifetime =
            is_vein(u32::from(race)).then(|| seconds(number(dice, ORE_LIFETIME.0, ORE_LIFETIME.1)));
        let empire = if proto.mob_type == CHAR_TYPE_NPC && proto.empire == 0 {
            map_empire(site.region.index)
        } else {
            proto.empire
        };
        let affect_flags = if spawn_motion {
            [1 << (AFF_SPAWN - 1), 0]
        } else {
            [0, 0]
        };
        site.npcs.push(Npc {
            vid,
            vnum: proto.vnum,
            race,
            char_type: proto.mob_type,
            on_click: proto.on_click_type,
            x,
            y,
            z,
            rotation,
            empire,
            moving_speed: limit_speed(proto.moving_speed),
            attack_speed: limit_speed(proto.attack_speed),
            name: self.names.find(u32::from(race)).to_vec(),
            ai_flags: proto.ai_flag,
            affect_flags,
            leader: None,
            regen: None,
            ore_lifetime,
        });
        self.report.spawned += 1;
        if proto.mob_type == CHAR_TYPE_STONE {
            self.report.stones += 1;
        }
        Ok(site.npcs.len() - 1)
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
    use std::sync::Arc;

    use gamedata::map_atlas::MapAtlas;
    use gamedata::mob_proto::{MobProto, CHAR_TYPE_MONSTER, CHAR_TYPE_STONE};
    use gamedata::records::MOB_LOCALE_NAME_BYTES;
    use gamedata::regen;
    use gamedata::server_attr::{self, CellBlock, SectreeGrid, ServerAttr, SECTREE_CELLS};

    use super::*;

    /// Answers the draws a test scripted, in order, and fails a test that draws one more.
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

    /// The monster's AI flags: aggressive, and one bit beside it to show they are carried whole.
    const MONSTER_AI: u32 = AIFLAG_AGGRESSIVE | 0b100;

    fn protos() -> MobProtos {
        let mut smith = proto(20_016, CHAR_TYPE_NPC, 0);
        smith.table.on_click_type = 2;
        let mut monster = proto(101, CHAR_TYPE_MONSTER, 0);
        monster.table.ai_flag = MONSTER_AI;
        let mut wolf = proto(102, CHAR_TYPE_MONSTER, 0);
        wolf.table.moving_speed = 300;
        MobProtos::from_rows(vec![
            smith,
            proto(20_017, CHAR_TYPE_NPC, 3),
            proto(20_018, CHAR_TYPE_WARP, 0),
            proto(20_019, CHAR_TYPE_GOTO, 0),
            proto(20_047, CHAR_TYPE_NPC, 0),
            proto(30_306, CHAR_TYPE_NPC, 0),
            monster,
            wolf,
            proto(8_001, CHAR_TYPE_STONE, 0),
            proto(0x0001_4E30, CHAR_TYPE_NPC, 0),
        ])
    }

    /// Group 500 is three 102s; 501's leader has no proto; 502 is a 102, a member with no proto,
    /// then a 102; 0 is a 102 that `SpawnGroup` never looks up. Group-group 600 picks 500 or 502;
    /// 601 has no member.
    fn groups() -> MobGroups {
        MobGroups::parse(
            b"Group a\n{\n\tVnum\t500\n\tLeader\tx\t102\n\t1\tx\t102\n\t2\tx\t102\n}\n\
              Group zero\n{\n\tVnum\t0\n\tLeader\tx\t102\n}\n\
              Group b\n{\n\tVnum\t501\n\tLeader\tx\t1\n\t1\tx\t102\n}\n\
              Group c\n{\n\tVnum\t502\n\tLeader\tx\t102\n\t1\tx\t1\n\t2\tx\t102\n}\n",
            b"Group gg\n{\n\tVnum\t600\n\t1\t500\t9\n\t2\t502\t1\n}\n\
              Group empty\n{\n\tVnum\t601\n}\n",
        )
        .unwrap()
    }

    fn names() -> MobLocaleNames {
        MobLocaleNames::parse(b"VNUM\tNAME\n20016\tSmith\n20017\tGuard\n").unwrap()
    }

    /// A map from (10000, 20000) to (12000, 22000). Its one sectree runs from (6400, 19200) to
    /// (12800, 25600).
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

    /// The test map's cells, each 0 unless `marks` sets it.
    fn cells(marks: &[((i32, i32), u32)]) -> MapCells {
        let mut block = vec![0; SECTREE_CELLS * SECTREE_CELLS];
        let cell = |at: i32| usize::try_from(at % 6400 / 50).unwrap();
        for &((x, y), attr) in marks {
            block[cell(y) * SECTREE_CELLS + cell(x)] = attr;
        }
        let attr = ServerAttr::from_blocks(1, 1, vec![CellBlock::from_cells(&block)]).unwrap();
        MapCells::new(SectreeGrid::of(&region(1)).unwrap(), Arc::new(attr))
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

    fn boxed(kind: RegenKind, vnum: i32) -> RegenEntry {
        entry(kind, vnum, (10_100, 20_100), (10_300, 20_300))
    }

    fn spawn_on(
        cells: &MapCells,
        entries: &[RegenEntry],
        dice: &mut Script,
    ) -> (MapNpcs, SpawnReport) {
        let (protos, groups, names) = (protos(), groups(), names());
        let mut spawner = NpcSpawner::new(&protos, &groups, &names, NpcVids::default());
        let map = spawner.spawn_map(&region(1), cells, entries, dice).unwrap();
        (map, spawner.report())
    }

    fn spawn(entries: &[RegenEntry], dice: &mut Script) -> (MapNpcs, SpawnReport) {
        spawn_on(&cells(&[]), entries, dice)
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

    /// An NPC at a point stands there with the entry's z and direction, and the proto's speeds,
    /// and holds its regen.
    #[test]
    fn an_npc_at_a_point_stands_as_its_entry_says() {
        let mut at = point(20_016, 10_500, 20_700);
        at.z_section = 7;
        at.direction = 3;
        let (map, report) = spawn(&[at], &mut Script::new(&[0]));
        assert_eq!(
            map.npcs,
            [Npc {
                vid: FIRST_NPC_VID,
                vnum: 20_016,
                race: 20_016,
                char_type: CHAR_TYPE_NPC,
                on_click: 2,
                x: 10_500,
                y: 20_700,
                z: 7,
                rotation: 90,
                empire: 1,
                moving_speed: 100,
                attack_speed: 90,
                name: b"Smith".to_vec(),
                ai_flags: 0,
                affect_flags: [0, 0],
                leader: None,
                regen: Some(0),
                ore_lifetime: None,
            }]
        );
        assert_eq!(report.spawned, 1);
    }

    /// Each timed entry draws `number(0, 16)` after its spawns, and first fires that many seconds
    /// past its time; its count is the units that stood. An idle entry keeps no regen.
    #[test]
    fn a_timed_entry_draws_its_first_fire_after_its_spawns() {
        let mut two = point(20_016, 10_500, 20_700);
        two.max_count = 2;
        two.time = 30;
        let mut idle = point(20_016, 10_500, 20_700);
        idle.time = 0;
        let missing = point(1, 10_500, 20_700);
        let mut dice = Script::new(&[16, 17]);
        let (map, _) = spawn(&[two, idle, missing], &mut dice);
        assert_eq!(
            map.regens,
            [
                Regen {
                    entry: two,
                    count: 2,
                    first_fire: 46,
                },
                Regen {
                    entry: missing,
                    count: 0,
                    first_fire: 60,
                },
            ]
        );
        let held: Vec<Option<usize>> = map.npcs.iter().map(|n| n.regen).collect();
        assert_eq!(held, [Some(0), Some(0)]);
        assert_eq!(dice.left(), 0);
    }

    /// Direction 0 draws `number(0, 7) * 45` for each character; any other names its angle.
    #[test]
    fn the_direction_sets_or_draws_the_rotation() {
        let mut drawn = point(20_016, 10_500, 20_700);
        drawn.direction = 0;
        drawn.max_count = 2;
        let mut last = point(20_016, 10_600, 20_700);
        last.direction = 255;
        let mut dice = Script::new(&[3, 0x7fff_ffff, 0, 0]);
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
        let mut dice = Script::new(&[5, 0]);
        let (map, report) = spawn(&[missing], &mut dice);
        assert!(map.npcs.is_empty());
        assert_eq!(report.no_proto, 1);
        assert_eq!(dice.left(), 0);
    }

    /// A box tries points until a sectree holds one, then draws `number(0, 360)`.
    #[test]
    fn a_box_tries_points_until_a_sectree_holds_one() {
        let boxed = entry(RegenKind::Mob, 20_016, (12_000, 21_000), (13_000, 21_100));
        // 12_000 + 900 is past the sectree's 12_800; 12_000 + 100 is inside it.
        let mut dice = Script::new(&[900, 50, 100, 20, 361 + 42, 0]);
        let (map, report) = spawn(&[boxed], &mut dice);
        assert_eq!(map.npcs.len(), 1);
        let npc = &map.npcs[0];
        assert_eq!((npc.x, npc.y, npc.z, npc.rotation), (12_100, 21_020, 0, 42));
        assert_eq!(report.unplaced, 0);
        assert_eq!(dice.left(), 0);
    }

    /// A box's draws reach both of its far edges, and a draw of its width wraps to its near ones.
    #[test]
    fn a_box_draws_from_edge_to_edge() {
        let boxed = entry(RegenKind::Mob, 20_016, (10_100, 20_100), (10_300, 20_400));
        let mut dice = Script::new(&[200, 300, 0, 0, 201, 301, 0, 0]);
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
        let mut dice = Script::new(&[7, 30, 42, 0, 30, 7, 43, 0]);
        let (map, _) = spawn(&[upright, flat], &mut dice);
        let placed: Vec<(i32, i32, u16)> = map
            .npcs
            .iter()
            .map(|npc| (npc.x, npc.y, npc.rotation))
            .collect();
        assert_eq!(placed, [(11_000, 21_030, 42), (11_030, 21_000, 43)]);
        assert_eq!(dice.left(), 0);
    }

    /// An exception entry marks an area; it spawns, lists, draws and counts nothing.
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

    /// Sixteen points no sectree holds place nothing, and nothing draws a rotation; a point no
    /// sectree holds places nothing either.
    #[test]
    fn a_box_outside_the_sectrees_places_nothing() {
        let outside = entry(RegenKind::Mob, 20_016, (30_000, 21_000), (30_100, 21_100));
        let mut dice = Script::new(&[0; 33]);
        let (map, report) = spawn(&[outside], &mut dice);
        assert!(map.npcs.is_empty());
        assert_eq!(report.unplaced, 1);
        assert_eq!(report.spawned, 0);
        assert_eq!(dice.left(), 0);
        let (_, report) = spawn(&[point(20_016, 12_800, 20_700)], &mut Script::new(&[0]));
        assert_eq!(report.unplaced, 1, "the sectree's right edge is exclusive");
        let (map, _) = spawn(&[point(20_016, 12_799, 20_700)], &mut Script::new(&[0]));
        assert_eq!(map.npcs.len(), 1, "past the map's edge, inside its sectree");
    }

    /// The sixteenth point is the last a box tries, and a seventeenth is never drawn.
    #[test]
    fn the_sixteenth_try_is_the_last() {
        let boxed = entry(RegenKind::Mob, 20_016, (12_000, 21_000), (13_000, 21_100));
        let mut draws = [900_u32, 0].repeat(15);
        draws.extend([100, 20, 42, 0]);
        let mut dice = Script::new(&draws);
        let (map, report) = spawn(&[boxed], &mut dice);
        assert_eq!(report.unplaced, 0);
        assert_eq!((map.npcs[0].x, map.npcs[0].y), (12_100, 21_020));
        assert_eq!(dice.left(), 0);
        let mut draws = [900_u32, 0].repeat(16);
        draws.extend([100, 20]);
        let mut dice = Script::new(&draws);
        let (map, report) = spawn(&[boxed], &mut dice);
        assert!(map.npcs.is_empty());
        assert_eq!(report.unplaced, 1);
        assert_eq!(
            dice.left(),
            1,
            "the entry's own draw follows the sixteenth try"
        );
    }

    /// Every character of `max_count` spawns, and a count that is not positive spawns none but
    /// still draws its first fire.
    #[test]
    fn max_count_characters_spawn() {
        let mut three = point(20_016, 10_500, 20_700);
        three.max_count = 3;
        let mut none = point(20_016, 10_500, 20_700);
        none.max_count = 0;
        let mut negative = point(20_016, 10_500, 20_700);
        negative.max_count = -3;
        let mut dice = Script::new(&[0, 0, 0]);
        let (map, report) = spawn(&[three, none, negative], &mut dice);
        assert_eq!(map.npcs.len(), 3);
        assert_eq!(report.spawned, 3);
        assert_eq!(dice.left(), 0);
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
        let (map, report) = spawn(&entries, &mut Script::new(&[0; 7]));
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

    /// A monster in a box stands like an NPC, with the proto's AI flags and no spawn motion, and
    /// the client sees its speeds limited to 250.
    #[test]
    fn a_monster_in_a_box_stands_with_its_protos_ai() {
        let mut dice = Script::new(&[50, 70, 361 + 10, 5, 0, 0, 0, 0]);
        let entries = [boxed(RegenKind::Mob, 101), boxed(RegenKind::Mob, 102)];
        let (map, report) = spawn(&entries, &mut dice);
        let monster = &map.npcs[0];
        assert_eq!(
            (monster.char_type, monster.x, monster.y, monster.rotation),
            (CHAR_TYPE_MONSTER, 10_150, 20_170, 10)
        );
        assert_eq!(monster.ai_flags, MONSTER_AI);
        assert!(monster.is_aggressive());
        assert_eq!(monster.affect_flags, [0, 0]);
        assert_eq!((monster.leader, monster.regen), (None, Some(0)));
        assert_eq!(map.regens[0].first_fire, 65);
        assert!(!map.npcs[1].is_aggressive());
        assert_eq!(map.npcs[1].moving_speed, 250);
        assert_eq!(report.spawned, 2);
        assert_eq!(dice.left(), 0);
    }

    /// A stone in a box is shown with the spawn motion, which leaves `AFF_SPAWN` set; at a point it
    /// is not, as `SpawnMob` is passed `false` there.
    #[test]
    fn a_stone_in_a_box_keeps_the_spawn_affect() {
        let entries = [boxed(RegenKind::Mob, 8_001), point(8_001, 10_500, 20_700)];
        let mut dice = Script::new(&[0; 5]);
        let (map, report) = spawn(&entries, &mut dice);
        let affects: Vec<[u32; 2]> = map.npcs.iter().map(|n| n.affect_flags).collect();
        assert_eq!(affects, [[0x4, 0], [0, 0]]);
        assert_eq!(report.stones, 2);
        assert_eq!(dice.left(), 0);
    }

    /// A monster is refused on a blocked, built-on or `BANPK` cell; an NPC is not checked; an ore
    /// vein is refused on a blocked or `BANPK` cell but stands on a built-on one, and draws its
    /// lifetime.
    #[test]
    fn the_cells_refuse_what_spawn_mob_checks() {
        let blocked = (10_100, 20_100);
        let built = (10_150, 20_100);
        let banpk = (10_200, 20_100);
        let cells = cells(&[
            (blocked, ATTR_BLOCK),
            (built, ATTR_OBJECT),
            (banpk, ATTR_BANPK),
        ]);
        let mut entries = Vec::new();
        for vnum in [101, 20_016, 20_047] {
            for (x, y) in [blocked, built, banpk] {
                entries.push(point(vnum, x, y));
            }
        }
        // Nine first fires, and the vein's lifetime before the eighth.
        let mut dice = Script::new(&[0, 0, 0, 0, 0, 0, 0, 100, 0, 0]);
        let (map, report) = spawn_on(&cells, &entries, &mut dice);
        let stood: Vec<(u32, i32)> = map.npcs.iter().map(|n| (n.vnum, n.x)).collect();
        assert_eq!(
            stood,
            [
                (20_016, 10_100),
                (20_016, 10_150),
                (20_016, 10_200),
                (20_047, 10_150)
            ]
        );
        assert_eq!(map.npcs[3].ore_lifetime, Some(520));
        assert_eq!(map.npcs[0].ore_lifetime, None);
        assert_eq!(report.unplaced, 5);
        assert_eq!(dice.left(), 0);
    }

    /// An ore vein in a box draws its lifetime after its rotation, as `SetProto` comes after it.
    #[test]
    fn an_ore_vein_draws_its_lifetime_after_its_rotation() {
        let mut dice = Script::new(&[0, 0, 7, 480, 0]);
        let (map, _) = spawn(&[boxed(RegenKind::Mob, 20_047)], &mut dice);
        assert_eq!(map.npcs[0].rotation, 7);
        assert_eq!(map.npcs[0].ore_lifetime, Some(900));
        assert_eq!(dice.left(), 0);
    }

    /// A group stands its leader in the box, then each member in a box of `number(300, 500)` each
    /// way around the one before. The leader holds the regen and the others follow it; `ga` makes
    /// every member aggressive, and the entry counts the group once.
    #[test]
    fn a_group_stands_each_member_around_the_last() {
        let ga = boxed(RegenKind::Group { aggressive: true }, 500);
        let mut draws = vec![0, 0, 0, 0, 0, 0, 0];
        draws.extend([100, 200, 0, 0, 0, 0, 0]);
        draws.extend([0, 0, 0, 200, 200, 200, 200]);
        draws.push(0);
        let mut dice = Script::new(&draws);
        let (map, report) = spawn(&[ga], &mut dice);
        let points: Vec<(i32, i32)> = map.npcs.iter().map(|n| (n.x, n.y)).collect();
        assert_eq!(
            points,
            [(10_100, 20_100), (9_900, 20_000), (9_600, 19_700)],
            "the second within 300 of the first, the third within 300 of the second"
        );
        let leader = Some(FIRST_NPC_VID);
        let follows: Vec<Option<u32>> = map.npcs.iter().map(|n| n.leader).collect();
        assert_eq!(follows, [None, leader, leader]);
        let held: Vec<Option<usize>> = map.npcs.iter().map(|n| n.regen).collect();
        assert_eq!(held, [Some(0), None, None]);
        assert!(map.npcs.iter().all(Npc::is_aggressive));
        assert!(map.npcs.iter().all(|n| n.affect_flags == [0, 0]));
        assert_eq!(map.regens[0].count, 1);
        assert_eq!(report.spawned, 3);
        assert_eq!(dice.left(), 0);
    }

    /// A group whose leader does not stand fails with no further draw; a later member that does
    /// not stand is skipped with no box drawn around it; a missing group draws nothing.
    #[test]
    fn a_group_fails_with_its_leader_and_skips_a_missing_member() {
        let g = RegenKind::Group { aggressive: false };
        let entries = [boxed(g, 501), boxed(g, 502), boxed(g, 503)];
        let mut draws = vec![0];
        draws.extend([0; 7]);
        draws.extend([0; 7]);
        draws.extend([0, 0]);
        let mut dice = Script::new(&draws);
        let (map, report) = spawn(&entries, &mut dice);
        assert_eq!(map.npcs.len(), 2);
        assert!(!map.npcs[1].is_aggressive());
        assert_eq!(map.npcs[1].leader, Some(map.npcs[0].vid));
        let counts: Vec<u32> = map.regens.iter().map(|r| r.count).collect();
        assert_eq!(counts, [0, 1, 0]);
        assert_eq!(report.no_proto, 2, "501's leader and 502's middle member");
        assert_eq!(report.no_group, 1);
        assert_eq!(dice.left(), 0);
    }

    /// A group-group draws `number(1, members)`, whatever the members' probabilities, and spawns
    /// that group; an empty or missing one draws nothing.
    #[test]
    fn a_group_group_picks_one_group_uniformly() {
        let entries = [
            boxed(RegenKind::GroupGroup, 600),
            boxed(RegenKind::GroupGroup, 601),
            boxed(RegenKind::GroupGroup, 602),
        ];
        let mut draws = vec![1];
        draws.extend([0; 14]);
        draws.extend([0, 0, 0]);
        let mut dice = Script::new(&draws);
        let (map, report) = spawn(&entries, &mut dice);
        assert_eq!(map.npcs.len(), 2, "the draw of 2 picks 502, of two members");
        assert!(
            !map.npcs.iter().any(Npc::is_aggressive),
            "a group-group is never `ga`"
        );
        assert_eq!(report.no_group, 2);
        assert_eq!(dice.left(), 0);
    }

    /// The event flags hold the three gated anywhere vnums back with no draw. Any other tries
    /// cells of the whole map and draws its rotation once it stands.
    #[test]
    fn an_anywhere_entry_tries_the_whole_map() {
        let mut gated = entry(RegenKind::Anywhere, 5_001, (0, 0), (0, 0));
        gated.max_count = 2;
        let free = entry(RegenKind::Anywhere, 101, (0, 0), (0, 0));
        let cells = cells(&[((10_100, 20_100), ATTR_BLOCK)]);
        let mut dice = Script::new(&[0, 0, 0, 1, 1, 361 + 30, 0]);
        let (map, report) = spawn_on(&cells, &[gated, free], &mut dice);
        assert_eq!(report.gated, 2);
        let npc = &map.npcs[0];
        assert_eq!((npc.x, npc.y, npc.z, npc.rotation), (10_200, 20_200, 0, 30));
        assert_eq!(npc.regen, Some(1));
        assert_eq!(dice.left(), 0);
    }

    /// `SpawnGroup` refuses vnum 0 before it looks the group up, so a group 0 never stands.
    #[test]
    fn a_group_of_vnum_zero_is_never_looked_up() {
        let g = RegenKind::Group { aggressive: false };
        let mut dice = Script::new(&[0]);
        let (map, report) = spawn(&[boxed(g, 0)], &mut dice);
        assert!(map.npcs.is_empty());
        assert_eq!(report.no_group, 1);
        assert_eq!(dice.left(), 0);
    }

    /// An anywhere entry draws its cell from 1 to one short of the map's width and height, in
    /// cells of 100, and refuses a cell that bans fighting as it refuses a blocked one.
    #[test]
    fn an_anywhere_entry_draws_inside_the_maps_edge_and_shuns_banpk() {
        let free = entry(RegenKind::Anywhere, 101, (0, 0), (0, 0));
        let cells = cells(&[((10_100, 20_100), ATTR_BANPK)]);
        let mut dice = Script::new(&[0, 0, 19, 1, 0, 0]);
        let (map, report) = spawn_on(&cells, &[free], &mut dice);
        let npc = &map.npcs[0];
        assert_eq!(
            (npc.x, npc.y),
            (10_100, 20_200),
            "the draw of 19 wraps to the first cell of 19"
        );
        assert_eq!(report.unplaced, 0);
        assert_eq!(dice.left(), 0);
    }

    /// The ore veins are `mining::info`'s two ranges, both ends included, and nothing beside.
    #[test]
    fn the_ore_veins_are_minings_two_ranges() {
        let veins: Vec<u32> = (20_000..=30_400).filter(|&vnum| is_vein(vnum)).collect();
        let mining: Vec<u32> = (20_047..=20_059).chain(30_301..=30_306).collect();
        assert_eq!(veins, mining);
    }

    /// A group entry at a point is `SpawnMob` of the group's vnum, as legacy calls it.
    #[test]
    fn a_group_at_a_point_spawns_the_mob_of_its_vnum() {
        let group = entry(
            RegenKind::Group { aggressive: true },
            20_017,
            (10_100, 20_100),
            (10_100, 20_100),
        );
        let (map, _) = spawn(&[group], &mut Script::new(&[0]));
        assert_eq!(map.npcs.len(), 1);
        assert!(
            !map.npcs[0].is_aggressive(),
            "a point never makes one aggressive"
        );
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
        let (protos, groups, names) = (protos(), groups(), names());
        let mut spawner = NpcSpawner::new(&protos, &groups, &names, NpcVids::default());
        let map = spawner
            .spawn_map(
                &region(21),
                &cells(&[]),
                &entries,
                &mut Script::new(&[0; 4]),
            )
            .unwrap();
        let empires: Vec<u8> = map.npcs.iter().map(|n| n.empire).collect();
        assert_eq!(empires, [2, 3, 0, 0]);
    }

    /// The race is the vnum's low 16 bits, and the name is the race's, not the proto's.
    #[test]
    fn the_race_and_name_are_the_low_bits() {
        let (map, _) = spawn(
            &[point(0x0001_4E30, 10_500, 20_700)],
            &mut Script::new(&[0]),
        );
        assert_eq!(map.npcs[0].vnum, 0x0001_4E30);
        assert_eq!(map.npcs[0].race, 0x4E30);
        assert_eq!(map.npcs[0].name, b"Smith", "0x4E30 is 20016");
        let (map, _) = spawn(&[point(20_018, 10_500, 20_700)], &mut Script::new(&[0]));
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
        let (protos, groups, names) = (protos(), groups(), names());
        let mut spawner = NpcSpawner::new(&protos, &groups, &names, NpcVids::starting_at(u32::MAX));
        let mut two = point(20_016, 10_500, 20_700);
        two.max_count = 2;
        let refused = spawner.spawn_map(&region(1), &cells(&[]), &[two], &mut Script::new(&[]));
        assert_eq!(refused, Err(NpcVidsExhausted));
        assert_eq!(NpcVidsExhausted.to_string(), "every NPC VID is taken");
    }

    fn owners() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata")
    }

    /// The owner's map 1 stands up its monsters, stones and NPCs, each on a cell `SpawnMob`
    /// allows, and lists its mini-map.
    #[test]
    fn the_owners_first_map_stands_up_its_characters() {
        let root = owners();
        let protos = MobProtos::load(&root.join("proto")).unwrap();
        let names = MobLocaleNames::load(&root.join("locale/europe/country")).unwrap();
        let groups = MobGroups::load(&root.join("locale/europe")).unwrap();
        let map_dir = root.join("locale/europe/map");
        let atlas = MapAtlas::load(&map_dir).unwrap();
        let region = atlas.region(1).unwrap();
        let entries = regen::load_map(&map_dir, region).unwrap();
        let attr = server_attr::load_map(&map_dir, region).unwrap();
        let cells = MapCells::new(SectreeGrid::of(region).unwrap(), Arc::new(attr));
        let mut spawner = NpcSpawner::new(&protos, &groups, &names, NpcVids::default());
        let mut dice = crate::character::Pcg32::new(1, 0);
        let map = spawner
            .spawn_map(region, &cells, &entries, &mut dice)
            .unwrap();
        let report = spawner.report();
        assert_eq!(report.spawned, u64::try_from(map.npcs.len()).unwrap());
        let of_type = |t: u8| map.npcs.iter().filter(|n| n.char_type == t).count();
        // R3's model of the same files, which ignores the cells, expects 3171.93 characters
        // [2977, 3297]: 73 NPC-type, 12 stones, and ten 5004s behind their flag.
        let expected = SpawnReport {
            spawned: 3_177,
            stones: 12,
            gated: 10,
            ..SpawnReport::default()
        };
        assert_eq!(report, expected);
        let counts = [
            of_type(CHAR_TYPE_MONSTER),
            of_type(CHAR_TYPE_NPC),
            of_type(CHAR_TYPE_STONE),
            of_type(CHAR_TYPE_WARP),
        ];
        assert_eq!(counts, [3_092, 72, 12, 1]);
        let veins = map.npcs.iter().filter(|n| n.ore_lifetime.is_some()).count();
        assert_eq!(veins, 20, "stone.txt's twenty group-group 2001 entries");
        for npc in &map.npcs {
            let attr = cells.attribute(npc.x, npc.y).expect("in a sectree");
            if npc.char_type == CHAR_TYPE_MONSTER || npc.char_type == CHAR_TYPE_STONE {
                assert_eq!(attr & (ATTR_BLOCK | ATTR_OBJECT | ATTR_BANPK), 0);
            }
        }
        let stones = map.npcs.iter().filter(|n| n.char_type == CHAR_TYPE_STONE);
        assert!(stones.clone().count() > 0);
        assert!(stones.clone().all(|n| n.affect_flags == [0x4, 0]));
        let first = map
            .npcs
            .iter()
            .find(|n| n.vnum == 20_300)
            .expect("npc.txt's first");
        assert_eq!((first.x, first.y), (471_800, 951_600));
        assert!(map.positions.iter().any(|p| p.name == b"Fierar"));
        assert_eq!(map.positions.len(), 48);
        assert_eq!(
            map.regens.len(),
            entries.iter().filter(|e| e.spawns() && e.time != 0).count()
        );
        assert_eq!(map.regens.len(), 1_118);
    }
}
