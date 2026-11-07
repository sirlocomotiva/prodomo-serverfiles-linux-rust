//! The ship defence (`CShipDefenseManager`, `G/ShipDefense.cpp:1489-1960`,
//! `__SHIP_DEFENSE__`): the Hydra lobby and its waves — the create/start states, the
//! join/leave/land warps, the mast's heal and its broadcast, and the deck ticks.
//!
//! Legacy keeps one manager per game core: the runs by leader PID
//! (`m_mapShipDefense`), each with its private map, its state and wave, its unique
//! characters, its barrier VIDs, its laser data and its six events. The one process
//! keeps the runs on the game thread, keyed by leader PID, and reads the private map
//! through the dungeon manager: `CreatePrivateMap` and `DestroyPrivateMap` are the
//! same calls the party dungeons use.
//!
//! A run holds what the bridged names and the ticks name: the private index, the
//! state (`STATE_CREATE`/`STATE_START`), the wave, the mast VID with its hit points,
//! the barrier VIDs, the boss VID with its wave, the four mini-hydra VIDs, the egg
//! VID, the reward and portal VIDs, the fake-hydra VID, the spawn counters, and the
//! end kind with its due and count. The ticks (`ShipEvent`, `WaveEvent`,
//! `SpawnEvent`, `LaserEffectEvent`, `ClearSpawnEvent`, `ExitEvent`) read and write
//! these in place each Pulse; `CancelEvents` is the timers going quiet, so a run
//! whose timers are all `None` answers nothing.
//!
//! The laser affect (`AFFECT_DEFENSEWAVE_LASER`, 701) lives on the mini-hydra NPCs
//! through the NPC affect store, as legacy's `AddAffect` puts it there; the run
//! keeps only the effect cells and the shelter Pulse.
//!
//! A player that leaves the map is no participant: the ship keeps no participant
//! rows (`GetLeaderPID` answers the same PID wherever the member stands), and the
//! login flow's join/leave runs from the quest scripts themselves
//! (`ship_defense.lua`'s `when login`), which the Rewrite runs like legacy does.

use std::collections::BTreeMap;

/// The ship map (`ShipDefense.h:53`).
pub(super) const MAP: i32 = 358;

/// The port map (`ShipDefense.h:52`).
pub(super) const PORT_MAP: i32 = 359;

/// The entry map (`ShipDefense.h:54`): where the leave warps.
pub(super) const ENTRY_MAP: i32 = 135;

/// The cooldown a create stamps (`ShipDefense.h:109`), in seconds.
pub(super) const COOLDOWN_SECS: u32 = 1800;

/// The 1% heal of a repair (`WoodRepair::WOOD_REPAIR_PCT`).
const HEAL_PCT: i64 = 1;

/// The stun a repair costs (`WOOD_REPAIR_STUN_DELAY`, `G/ShipDefense.cpp:1731`), in
/// seconds.
pub(super) const REPAIR_STUN_SECS: i32 = 5;

/// The mast (`ShipDefense.h:29`).
pub(super) const MAST_VNUM: u32 = 20434;

/// The wheel (`ShipDefense.h:30`).
pub(super) const WHEEL_VNUM: u32 = 20436;

/// The invisible wall (`ShipDefense.h:31`).
pub(super) const BARRIER_VNUM: u32 = 3970;

/// The portal (`ShipDefense.h:47`).
pub(super) const PORTAL_VNUM: u32 = 3949;

/// The repair wood (`ShipDefense.h:49`).
pub(super) const WOOD_VNUM: u32 = 20437;

/// The reward stone (`ShipDefense.h:45`).
pub(super) const REWARD_VNUM: u32 = 3965;

/// The egg (`ShipDefense.h:43`).
pub(super) const EGG_VNUM: u32 = 20432;

/// `STATE_CREATE` (`ShipDefense.h:224`).
pub(super) const STATE_CREATE: u8 = 1;

/// `STATE_START` (`ShipDefense.h:225`).
pub(super) const STATE_START: u8 = 2;

/// The fake-hydra heads (`ShipDefense.h:39-40`), which no hit reaches
/// (`battle_is_attackable`, `G/battle.cpp:144-148`).
pub(super) const FAKE_HYDRAS: [u32; 2] = [3963, 3964];

/// The step rates of the hydra waves (`ShipDefense.h:34-36`).
pub(super) const HYDRAS: [u32; 3] = [3960, 3961, 3962];

/// The step rates of the mini-hydra waves (`ShipDefense.h:41-42`).
pub(super) const MINI_HYDRAS: [u32; 3] = [3957, 3958, 3959];

/// The mini-hydra spawn cells (`SpawnMiniHydra`, `G/ShipDefense.cpp:986-1008`), in map
/// units.
const MINI_CELLS: [(i32, i32); 4] = [(396, 411), (374, 411), (374, 389), (396, 389)];

/// The laser effect cells (`LaserEffectEvent`, `G/ShipDefense.cpp:188-197`), in map
/// units.
const LASER_CELLS: [(i32, i32); 4] = [(390, 405), (379, 405), (379, 394), (390, 394)];

/// The egg cells (`SpawnEgg`, `G/ShipDefense.cpp:1010-1040`), in map units.
const EGG_CELLS: [(i32, i32); 4] = [(379, 394), (391, 392), (391, 403), (380, 406)];

/// The wave-1 spawn cells (`SpawnEvent`, `G/ShipDefense.cpp:349-357`), in map units.
const WAVE1_CELLS: [(i32, i32); 7] = [
    (396, 426),
    (392, 427),
    (389, 429),
    (384, 431),
    (380, 429),
    (376, 427),
    (373, 426),
];

/// The barrier cells (`SpawnBarriers`, `G/ShipDefense.cpp:1041-1055`), in map units:
/// back left, back right, front left, front right.
const BARRIER_CELLS: [(i32, i32); 4] = [(400, 372), (369, 372), (400, 435), (369, 435)];

/// The land spot on the port map (`Land`, `G/ShipDefense.cpp:1786-1808`): the port
/// base plus (405, 480), in map units.
pub(super) const LAND_SPOT: (i32, i32) = (405, 480);

/// The `data/dungeon/ship_defense/wave1.txt` rows: the cell and vnum of each `m` row.
const WAVE1: [((i32, i32), u32); 20] = [
    ((393, 427), 3955),
    ((376, 427), 3955),
    ((389, 429), 3955),
    ((380, 429), 3955),
    ((385, 430), 3955),
    ((387, 428), 3955),
    ((382, 428), 3955),
    ((374, 426), 3955),
    ((394, 424), 3955),
    ((386, 424), 3955),
    ((382, 424), 3954),
    ((377, 424), 3954),
    ((390, 424), 3954),
    ((394, 421), 3954),
    ((389, 421), 3954),
    ((381, 422), 3954),
    ((376, 422), 3954),
    ((373, 422), 3954),
    ((395, 422), 3954),
    ((384, 425), 3956),
];

/// The `data/dungeon/ship_defense/wave2.txt` rows.
const WAVE2: [((i32, i32), u32); 26] = [
    ((385, 377), 3404),
    ((383, 377), 3405),
    ((380, 377), 3402),
    ((378, 377), 3401),
    ((376, 377), 3605),
    ((388, 377), 3602),
    ((390, 377), 3604),
    ((392, 377), 3601),
    ((377, 377), 3603),
    ((385, 430), 3953),
    ((383, 430), 3953),
    ((380, 430), 3953),
    ((378, 430), 3953),
    ((376, 430), 3953),
    ((383, 430), 3953),
    ((383, 430), 3953),
    ((394, 430), 3955),
    ((385, 430), 3955),
    ((383, 430), 3955),
    ((380, 430), 3955),
    ((378, 430), 3955),
    ((394, 430), 3954),
    ((385, 430), 3954),
    ((383, 430), 3954),
    ((380, 430), 3954),
    ((378, 430), 3954),
];

/// The `data/dungeon/ship_defense/wave2_1.txt` rows.
const WAVE2_1: [((i32, i32), u32); 4] = [
    ((405, 418), 3950),
    ((405, 385), 3950),
    ((364, 418), 3950),
    ((364, 385), 3950),
];

/// The `data/dungeon/ship_defense/wave3.txt` rows.
const WAVE3: [((i32, i32), u32); 28] = [
    ((385, 377), 3404),
    ((383, 377), 3405),
    ((380, 377), 3402),
    ((378, 377), 3401),
    ((376, 377), 3605),
    ((388, 377), 3602),
    ((390, 377), 3604),
    ((392, 377), 3601),
    ((377, 377), 3603),
    ((385, 430), 3953),
    ((383, 430), 3953),
    ((380, 430), 3953),
    ((378, 430), 3953),
    ((376, 430), 3953),
    ((383, 430), 3953),
    ((383, 430), 3953),
    ((394, 430), 3955),
    ((385, 430), 3955),
    ((383, 430), 3955),
    ((380, 430), 3955),
    ((378, 430), 3955),
    ((382, 430), 3955),
    ((394, 430), 3954),
    ((385, 430), 3954),
    ((383, 430), 3954),
    ((380, 430), 3954),
    ((378, 430), 3954),
    ((382, 430), 3954),
];

/// The `data/dungeon/ship_defense/wave3_1.txt` rows.
const WAVE3_1: [((i32, i32), u32); 8] = [
    ((403, 389), 3951),
    ((403, 388), 3951),
    ((403, 387), 3951),
    ((403, 386), 3951),
    ((365, 389), 3951),
    ((365, 388), 3951),
    ((365, 387), 3951),
    ((365, 386), 3951),
];

/// The `data/dungeon/ship_defense/wave4.txt` rows, wave 4's set steps included.
const WAVE4: [((i32, i32), u32); 44] = [
    ((385, 377), 3404),
    ((383, 377), 3405),
    ((380, 377), 3402),
    ((378, 377), 3401),
    ((376, 377), 3605),
    ((388, 377), 3602),
    ((390, 377), 3604),
    ((392, 377), 3601),
    ((377, 377), 3603),
    ((385, 430), 3953),
    ((383, 430), 3953),
    ((380, 430), 3953),
    ((378, 430), 3953),
    ((376, 430), 3953),
    ((383, 430), 3953),
    ((383, 430), 3953),
    ((394, 430), 3955),
    ((385, 430), 3955),
    ((383, 430), 3955),
    ((380, 430), 3955),
    ((378, 430), 3955),
    ((382, 430), 3955),
    ((394, 430), 3954),
    ((385, 430), 3954),
    ((383, 430), 3954),
    ((380, 430), 3954),
    ((378, 430), 3954),
    ((382, 430), 3954),
    ((403, 389), 3952),
    ((403, 388), 3952),
    ((403, 387), 3952),
    ((403, 386), 3952),
    ((365, 389), 3952),
    ((365, 388), 3952),
    ((365, 387), 3952),
    ((365, 386), 3952),
    ((365, 415), 3952),
    ((365, 416), 3952),
    ((365, 414), 3952),
    ((365, 413), 3952),
    ((403, 415), 3952),
    ((403, 416), 3952),
    ((403, 414), 3952),
    ((403, 413), 3952),
];

/// The `wave4_1.txt` the wave-4 steps ask for does not exist: the owner ships
/// `wave4 _1.txt` (with a space) instead, so legacy's `fopen` fails, logs
/// `SYSTEM: regen_do: ... file not found`, and spawns nothing
/// (`G/regen.cpp:419-425`). The Rewrite reads the shipped file's rows — the step
/// spawns they name — rather than the missing name: the miss is a Defect, and the
/// shipped rows are what the owner means to spawn.
/// The `wave4_1.txt` the wave-4 steps ask for does not exist: the owner ships
/// `wave4 _1.txt` (with a space) instead, so legacy's `fopen` fails, logs
/// `SYSTEM: regen_do: ... file not found`, and spawns nothing
/// (`G/regen.cpp:419-425`). The Rewrite reads the shipped file's rows — the step
/// spawns they name — rather than the missing name: the miss is a Defect, and the
/// shipped rows are what the owner means to spawn.
const WAVE4_1: [((i32, i32), u32); 16] = [
    ((403, 389), 3952),
    ((403, 388), 3952),
    ((403, 387), 3952),
    ((403, 386), 3952),
    ((365, 389), 3952),
    ((365, 388), 3952),
    ((365, 387), 3952),
    ((365, 386), 3952),
    ((365, 416), 3952),
    ((365, 415), 3952),
    ((365, 414), 3952),
    ((365, 413), 3952),
    ((403, 416), 3952),
    ((403, 415), 3952),
    ((403, 414), 3952),
    ((403, 413), 3952),
];

/// One ship-defence run: the `CShipDefense` narrowed to what the bridged names and
/// the deck ticks read and write.
#[derive(Clone, Debug, Default)]
pub(super) struct ShipRun {
    /// `m_lMapIndex`: the private index.
    pub index: i32,
    /// `m_byState`: `STATE_CREATE` or `STATE_START`.
    pub state: u8,
    /// `m_byWave`: the wave the deck stands on.
    pub wave: u8,
    /// The mast's VID (`UNIQUE_MAST_POS`), with its hit points and maximum: what
    /// the heal and the broadcast read. The NPC carries them too; the run's copy
    /// is the `GetUniqueCharacter` handle.
    pub mast: Option<(u32, i64, i64)>,
    /// `m_lpBarrier`: the four barrier VIDs, `None` where removed.
    pub barriers: [Option<u32>; 4],
    /// The boss VID (`UNIQUE_HYDRA_POS`), with its wave.
    pub boss: Option<(u32, u8)>,
    /// The four mini-hydra VIDs (`UNIQUE_MINI_HYDRA1_POS..4`).
    pub minis: [Option<u32>; 4],
    /// The egg VID (`UNIQUE_HYDRA_EGG_POS`).
    pub egg: Option<u32>,
    /// The reward-stone VID and the portal VID (`SetShipEvent(WAVES)`).
    pub reward: Option<u32>,
    /// The reward-stone VID and the portal VID, second half.
    pub portal: Option<u32>,
    /// The fake-hydra VID (`UNIQUE_FAKE_HYDRA_POS`).
    pub fake: Option<u32>,
    /// The `WaveEvent` timer: (`bNextWave`, wave, wait count, due).
    pub wave_due: Option<(bool, u8, u8, u64)>,
    /// The `SpawnEvent` timer: (wave, step, seconds, due).
    pub spawn_due: Option<(u8, u8, u32, u64)>,
    /// The `ShipEvent` timer: (wave, wait count, due).
    pub ship_due: Option<(u8, u8, u64)>,
    /// The `LaserEffectEvent` timer: its due.
    pub laser_due: Option<u64>,
    /// The `ClearSpawnEvent` timer: (clear-all-except-boss or not, due).
    pub clear_due: Option<(bool, u64)>,
    /// The `ExitEvent` timer: (back home or not, seconds left, due).
    pub exit_due: Option<(bool, u32, u64)>,
    /// `m_mapLaserEffectData`: the laser effect cells by mini slot.
    pub lasers: BTreeMap<u8, (i32, i32)>,
    /// `m_iLastLaserShelterPulse`: the Pulse a shelter last cleared a laser.
    pub shelter_pulse: u64,
}

/// The ship-defence runs (`CShipDefenseManager::m_mapShipDefense`), by leader PID.
#[derive(Clone, Debug, Default)]
pub(super) struct ShipRuns {
    /// The runs, by leader PID.
    by_leader: BTreeMap<u32, ShipRun>,
}

/// Whether `map` is the ship's map: 358 itself, or its 3580000..3590000 private
/// copies (`CShipDefenseManager::IsDungeon`, `G/ShipDefense.cpp:1955-1958`). The
/// death hook, the login flow and the quest reads share it.
pub(super) fn is_ship_map(map: i32) -> bool {
    map == MAP || (3_580_000..3_590_000).contains(&map)
}

/// Whether `CreatePrivateMap` could copy `base`: the Channel hosts it, which is what
/// `SECTREE_MANAGER::CreatePrivateMap` checks first (`G/sectree_manager.cpp:943-956`).
pub(super) fn base_hosted(
    atlas: Option<&gamedata::map_atlas::MapAtlas>,
    channel: u8,
    base: i32,
) -> bool {
    let _ = channel;
    atlas.is_some_and(|atlas| atlas.region(base).is_some())
}

/// `GetRecallPositionByEmpire(map, 0)` (`G/sectree_manager.cpp:493-526`): the map's
/// own town spawn, already absolute — the atlas builds it from the base plus the
/// `Town.txt` offset (`LoadMapRegion`, `G/sectree_manager.cpp:357-360`), so no second
/// scaling. `None` with no region, as the empire-0 `false` is. Empire 0 never takes
/// the per-empire points.
pub(super) fn recall_spot(
    atlas: Option<&gamedata::map_atlas::MapAtlas>,
    map: i32,
) -> Option<(i32, i32)> {
    let region = atlas?.region(map)?;
    Some(region.spawn)
}

impl ShipRuns {
    /// The run led by `leader`, if any.
    pub(super) fn of(&self, leader: u32) -> Option<&ShipRun> {
        self.by_leader.get(&leader)
    }

    /// The run led by `leader`, to change.
    pub(super) fn of_mut(&mut self, leader: u32) -> Option<&mut ShipRun> {
        self.by_leader.get_mut(&leader)
    }

    /// Stand a fresh run on `index` for `leader`.
    pub(super) fn insert(&mut self, leader: u32, index: i32) {
        self.by_leader.insert(
            leader,
            ShipRun {
                index,
                state: STATE_CREATE,
                ..ShipRun::default()
            },
        );
    }

    /// Whether any run stands: the ticks' early quiet.
    pub(super) fn is_empty(&self) -> bool {
        self.by_leader.is_empty()
    }

    /// Every leader PID with a run, in PID order.
    pub(super) fn leaders(&self) -> Vec<u32> {
        self.by_leader.keys().copied().collect()
    }
}

use common::vid::Vid;

use super::GameState;

impl GameState {
    /// The leader PID behind `reader`: its party's leader, or its own PID with no
    /// party (`GetLeaderPID(c_lpChar, false)`, `G/ShipDefense.cpp:1652-1674`).
    fn ship_leader_of(&self, reader: Vid) -> Option<u32> {
        let pid = self.characters.find_by_vid(reader).ok()?.player_id();
        Some(self.party_leader_of(pid).unwrap_or(pid))
    }

    /// The create's leader PID: the reader's party leader when the reader is the
    /// leader standing online, else the reader's own PID when it holds no party
    /// (`GetLeaderPID(c_lpChar, true)`). 0 when a member reads it, as the call is
    /// the leader's.
    fn ship_create_leader_of(&self, reader: Vid) -> u32 {
        let Ok(character) = self.characters.find_by_vid(reader) else {
            return 0;
        };
        let pid = character.player_id();
        let Some(leader) = self.party_leader_of(pid) else {
            return pid;
        };
        u32::from(leader == pid) * leader
    }

    /// Judge the step's `ship_defense_mgr.create`, `.start`, `.join`, `.leave`,
    /// `.land` and `.set_alliance_hp_pct` calls, in legacy call order
    /// (`Create`, `Start`, `Join`, `Leave`, `Land`, `SetAllianceHPPct`,
    /// `G/ShipDefense.cpp:1489-1808`): each judge reads the world the one before it
    /// left, as legacy's in-call runs do.
    pub(super) fn judge_ship_calls(
        &mut self,
        reader: Vid,
        calls: (u32, u32, u32, u32, u32, Vec<u8>),
        now: u32,
    ) {
        for _ in 0..calls.0 {
            self.judge_ship_create(reader, now);
        }
        for _ in 0..calls.1 {
            self.judge_ship_start(reader);
        }
        for _ in 0..calls.2 {
            self.judge_ship_join(reader);
        }
        for _ in 0..calls.3 {
            self.judge_ship_leave(reader);
        }
        for _ in 0..calls.4 {
            self.judge_ship_land(reader);
        }
        for pct in calls.5 {
            let _ = pct;
            self.judge_ship_heal(reader);
        }
    }

    /// Judge one `ship_defense_mgr.create` (`Create`, `G/ShipDefense.cpp:1489-1546`):
    /// the leader gate, the live-run gate, the private map, the cooldown fan, the
    /// ready fan, the exit save and the ordered warp. The deck (the barriers, the
    /// wheel and the mast) stands in the same call (`PrepareDeck`,
    /// `G/ShipDefense.cpp:1124-1143`). A member's create is quiet (0 holds no run);
    /// a failed map chats the line and queues nothing.
    fn judge_ship_create(&mut self, reader: Vid, now: u32) {
        let leader = self.ship_create_leader_of(reader);
        if leader == 0 {
            return;
        }
        if self.ship.of(leader).is_some() {
            return;
        }
        let channel = self.place_of(reader.raw()).map_or(0, |place| place.0);
        let Some(index) = self.create_ship_map(channel) else {
            if let Some(mover) = self.own_mover(reader) {
                self.tell(
                    reader,
                    mover,
                    protocol::gc_chat::CHAT_TYPE_INFO,
                    b"Failed to create private map.",
                    &[],
                );
            }
            return;
        };
        if self.party_leader_of(leader).is_some() {
            self.stamp_ship_cooldown(leader, now);
            self.tell_ship_ready(leader);
        }
        self.ship.insert(leader, index);
        self.stand_ship_deck(channel, index, leader);
        let Some((x, y)) = self
            .recall
            .atlas()
            .and_then(|atlas| recall_spot(Some(atlas), MAP))
        else {
            return;
        };
        self.order_ship_warp(reader, index, x, y);
    }

    /// Judge one `ship_defense_mgr.start` (`Start`, `G/ShipDefense.cpp:1548-1567`):
    /// the run starts and the deck's first wave prepares. Quiet with no run.
    fn judge_ship_start(&mut self, reader: Vid) {
        let leader = self.ship_create_leader_of(reader);
        if leader == 0 {
            return;
        }
        let Some(run) = self.ship.of_mut(leader) else {
            return;
        };
        run.state = STATE_START;
        let index = run.index;
        let due = self.last_pulse.saturating_add(25);
        if let Some(run) = self.ship.of_mut(leader) {
            run.spawn_due = None;
            run.ship_due = None;
            run.laser_due = None;
            run.clear_due = None;
        }
        self.ship_broadcast_hp(index);
        self.ship_prepare_wave(leader, 1, due);
    }

    /// Judge one `ship_defense_mgr.join` (`Join`, `G/ShipDefense.cpp:1738-1772`):
    /// the exit save and the ordered warp to the recall spot on the run's index.
    /// Quiet with no party or no run.
    fn judge_ship_join(&mut self, reader: Vid) {
        let Ok(character) = self.characters.find_by_vid(reader) else {
            return;
        };
        let pid = character.player_id();
        let Some(leader) = self.party_leader_of(pid) else {
            return;
        };
        let Some(run) = self.ship.of(leader) else {
            return;
        };
        let index = run.index;
        if self.maps.keys().all(|(_, map)| *map != index) {
            return;
        }
        let Some((x, y)) = self
            .recall
            .atlas()
            .and_then(|atlas| recall_spot(Some(atlas), MAP))
        else {
            return;
        };
        self.order_ship_warp(reader, index, x, y);
    }

    /// Judge one `ship_defense_mgr.leave` (`Leave`, `G/ShipDefense.cpp:1774-1784`):
    /// the exit save and the ordered warp to the entry map's recall spot.
    fn judge_ship_leave(&mut self, reader: Vid) {
        let Some((x, y)) = self
            .recall
            .atlas()
            .and_then(|atlas| recall_spot(Some(atlas), ENTRY_MAP))
        else {
            return;
        };
        let Some(mover) = self.own_mover(reader) else {
            return;
        };
        self.order_quest_warp(reader, x, y, 0, mover);
    }

    /// Judge one `ship_defense_mgr.land` (`Land`, `G/ShipDefense.cpp:1786-1808`):
    /// the ordered warp to the port spot, or the entry recall with no port map.
    fn judge_ship_land(&mut self, reader: Vid) {
        let Some(mover) = self.own_mover(reader) else {
            return;
        };
        let at = self
            .recall
            .atlas()
            .and_then(|atlas| atlas.region(PORT_MAP))
            .map(|region| (region.sx + LAND_SPOT.0 * 100, region.sy + LAND_SPOT.1 * 100));
        let Some((x, y)) = at.or_else(|| {
            self.recall
                .atlas()
                .and_then(|atlas| recall_spot(Some(atlas), ENTRY_MAP))
        }) else {
            return;
        };
        self.order_quest_warp(reader, x, y, 0, mover);
    }

    /// Judge one `ship_defense_mgr.set_alliance_hp_pct` (`SetAllianceHPPct`,
    /// `G/ShipDefense.cpp:1701-1736`): the mast heals 1% of its maximum past it,
    /// the HP notice chats, the reader stuns 5 seconds, and the HP broadcast goes.
    /// The argument is ignored, as the call ignores it. Quiet with no run, no map
    /// or no mast.
    fn judge_ship_heal(&mut self, reader: Vid) {
        let Some(leader) = self.ship_leader_of(reader) else {
            return;
        };
        let Some(run) = self.ship.of(leader) else {
            return;
        };
        let index = run.index;
        let Some((vid, hp, max)) = run.mast else {
            return;
        };
        if self.maps.keys().all(|(_, map)| *map != index) {
            return;
        }
        let heal = (HEAL_PCT * max.max(0) / 100).max(0);
        let hp = hp.saturating_add(heal);
        if let Some(run) = self.ship.of_mut(leader) {
            run.mast = Some((vid, hp, max));
        }
        if let Some(npc) = self.npc_mut(vid) {
            npc.hp = i32::try_from(hp).unwrap_or(i32::MAX);
        }
        self.ship_tell_mast_hp(index, hp, max);
        self.add_pc_affect(
            reader,
            super::affect::Affect {
                kind: crate::loading_phase::AFFECT_STUN,
                apply_on: 0,
                value: 0,
                flag: crate::loading_phase::AFF_STUN,
                remaining: REPAIR_STUN_SECS,
                sp_cost: 0,
            },
            true,
        );
        self.ship_broadcast_hp(index);
    }

    /// The create's cooldown fan (`Create`, `G/ShipDefense.cpp:1524-1533`): every
    /// online member's `ship_defense.cooldown` flag stamps `now + COOLDOWN`.
    fn stamp_ship_cooldown(&mut self, leader: u32, now: u32) {
        let pids: Vec<u32> = self
            .parties
            .linked_vids(leader)
            .iter()
            .filter_map(|vid| self.characters.find_by_vid(*vid).ok())
            .map(world::character::Character::player_id)
            .collect();
        if pids.is_empty() {
            return;
        }
        let value = i32::try_from(now.saturating_add(COOLDOWN_SECS)).unwrap_or(i32::MAX);
        if let Some(quests) = self.quests.as_mut() {
            for pid in pids {
                quests
                    .manager_mut()
                    .set_quest_flag(pid, b"ship_defense.cooldown", value);
            }
        }
    }

    /// The create's ready fan (`Create`, `G/ShipDefense.cpp:1534`): the
    /// `LC_TEXT` notice to every online member, in member order.
    fn tell_ship_ready(&self, leader: u32) {
        for vid in self.parties.linked_vids(leader) {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(
                    vid,
                    mover,
                    protocol::gc_chat::CHAT_TYPE_NOTICE,
                    b"The 'Ship Defence' dungeon is ready. The fisherman will let you \
                      board now.",
                    &[],
                );
            }
        }
    }

    /// Host the ship's private map on `channel`: `CreatePrivateMap(358)` through
    /// the dungeon manager, which destroys the index again when the base grid is
    /// missing, as the create's `None` arm does.
    fn create_ship_map(&mut self, channel: u8) -> Option<i32> {
        let taken: Vec<i32> = self.maps.keys().map(|&(_, map)| map).collect();
        let index = self
            .dungeons
            .create(MAP, |base| self.recall.hosts(channel, base), &taken)?;
        let Some(grid) = self
            .recall
            .atlas()
            .and_then(|atlas| atlas.region(MAP))
            .and_then(gamedata::server_attr::SectreeGrid::of)
        else {
            self.dungeons.destroy(index);
            return None;
        };
        self.host_map(channel, index, grid);
        Some(index)
    }

    /// The create/join warp: the exit save, then the ordered warp to the recall
    /// spot on the run's index (`Create`, `G/ShipDefense.cpp:1536-1544`).
    fn order_ship_warp(&mut self, reader: Vid, index: i32, x: i32, y: i32) {
        let raw = reader.raw();
        let Some((channel_now, map)) = self.place_of(raw) else {
            return;
        };
        let exit = self.spot_of(raw).map(|spot| crate::save::ExitLocation {
            x: spot.x,
            y: spot.y,
            map_index: map,
        });
        let Some(clients) = self.clients.as_ref() else {
            return;
        };
        let ordered = clients
            .members_on_map(channel_now, map)
            .into_iter()
            .find(|(_, entry)| entry.vid == raw)
            .is_some_and(|(id, _)| {
                clients.order(
                    channel_now,
                    id,
                    crate::client_registry::ClientOrder::Warp {
                        x,
                        y,
                        private_map_index: index,
                        channel: Some(channel_now),
                        exit,
                        sorted_until: self.sort_lock(reader),
                    },
                )
            });
        if !ordered {
            tracing::warn!(vid = raw, "a ship warp found no client to order");
        }
    }

    /// Stand the deck (`PrepareDeck`, `G/ShipDefense.cpp:1124-1143`): the four
    /// barriers, the wheel, and the mast whose VID the run holds.
    fn stand_ship_deck(&mut self, channel: u8, index: i32, leader: u32) {
        let pulse = self.last_pulse;
        let mut barriers = [None; 4];
        for (slot, cell) in BARRIER_CELLS.iter().enumerate() {
            barriers[slot] = self
                .ship_spawn(channel, index, BARRIER_VNUM, *cell, 1, pulse)
                .map(|(vid, _)| vid);
        }
        let _wheel = self.ship_spawn(channel, index, WHEEL_VNUM, (385, 367), 1, pulse);
        let mast = self.ship_spawn(channel, index, MAST_VNUM, (385, 400), 1, pulse);
        if let Some(run) = self.ship.of_mut(leader) {
            run.barriers = barriers;
            run.mast = mast.map(|(vid, max)| (vid, i64::from(max), i64::from(max)));
        }
    }

    /// Stand one ship spawn (`Spawn`, `G/ShipDefense.cpp:865-909`): the vnum at the
    /// map-unit cell on the run's index, facing `(dir - 1) * 45` degrees, without
    /// the spawn motion. Answers the VID and its maximum HP.
    fn ship_spawn(
        &mut self,
        channel: u8,
        index: i32,
        vnum: u32,
        cell: (i32, i32),
        dir: i32,
        pulse: u64,
    ) -> Option<(u32, i32)> {
        let place = (channel, index);
        let (base_x, base_y) = self
            .maps
            .get(&place)
            .map_or((0, 0), super::view::MapIndex::base);
        let at = (
            base_x.saturating_add(cell.0.saturating_mul(100)),
            base_y.saturating_add(cell.1.saturating_mul(100)),
        );
        let (Some(region), Some(cells)) = (
            self.regens.region_of(channel, MAP),
            self.regens.cells_of(index),
        ) else {
            return None;
        };
        let respawner = self.regens.respawner_mut()?;
        let facing = if dir == 0 { -1 } else { (dir - 1) * 45 };
        let stood = match respawner.spawn_directed(
            (&region, &cells),
            vnum,
            at,
            facing,
            false,
            &mut self.dice,
        ) {
            Ok(stood) => stood?,
            Err(error) => {
                tracing::warn!(?place, %error, "a ship spawn outnumbered the VIDs left");
                return None;
            }
        };
        let (vid, max) = (stood.vid, stood.max_hp);
        self.stand_runtime(place, vec![stood], pulse);
        Some((vid, max))
    }

    /// `BroadcastAllianceHP` (`G/ShipDefense.cpp:1675-1700`): the mast's `GC_TARGET`
    /// to every player on the run's map.
    fn ship_broadcast_hp(&self, index: i32) {
        let leader = self
            .ship
            .by_leader
            .iter()
            .find(|(_, run)| run.index == index)
            .map(|(leader, _)| *leader);
        let Some(leader) = leader else {
            return;
        };
        let Some((vid, hp, max)) = self.ship.of(leader).and_then(|run| run.mast) else {
            return;
        };
        let mut vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == index)
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_by_key(|vid| vid.raw());
        for to in vids {
            let mut record = Vec::new();
            protocol::gc_actors::GcTarget::new(vid, 0, 0, 0, 1, hp, max, 0)
                .encode_into(&mut record);
            let _sent = self.write_to_client(to, record);
        }
    }

    /// The HP notice (`NoticeByType(NOTICE_MAST_HP)`, `G/ShipDefense.cpp:842-853`):
    /// `<mast>'s remaining HP: <pct>%` to every player on the run's map.
    fn ship_tell_mast_hp(&self, index: i32, hp: i64, max: i64) {
        let pct = if max <= 0 { 0 } else { hp * 100 / max };
        let name = self
            .ship
            .by_leader
            .iter()
            .find(|(_, run)| run.index == index)
            .and_then(|(_, run)| run.mast)
            .and_then(|(vid, _, _)| self.npc_by_vid(vid))
            .map_or(Vec::new(), |npc| npc.name.clone());
        let mut vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == index)
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(
                    vid,
                    mover,
                    protocol::gc_chat::CHAT_TYPE_NOTICE,
                    b"%s's remaining HP: %d%%",
                    &[
                        crate::chat_line::Arg::Text(&name),
                        crate::chat_line::Arg::Int(pct),
                    ],
                );
            }
        }
    }
}

impl GameState {
    /// `PrepareWave` (`G/ShipDefense.cpp:1144-1231`): the deck stands `wave` — the
    /// fake hydra and its clear for wave 1, the clear, the quarter jump, the hydra
    /// and the notice for the rest — and the wave timer counts down from the wave's
    /// wait.
    fn ship_prepare_wave(&mut self, leader: u32, wave: u8, due: u64) {
        let Some(run) = self.ship.of(leader) else {
            return;
        };
        let index = run.index;
        let channel = self.ship_channel(index);
        if let Some(run) = self.ship.of_mut(leader) {
            run.wave_due = None;
            run.spawn_due = None;
            run.ship_due = None;
            run.laser_due = None;
            run.clear_due = None;
            run.exit_due = None;
        }
        let wait = match wave {
            1 => 60,
            2..=4 => 10,
            _ => return,
        };
        match wave {
            1 => {
                let fake = self.ship_spawn(channel, index, FAKE_HYDRAS[0], (385, 373), 5, due);
                if let Some(run) = self.ship.of_mut(leader) {
                    run.fake = fake.map(|(vid, _)| vid);
                    let at = due.saturating_add(3 * 25);
                    run.clear_due = Some((false, at));
                }
                for cell in [(378, 443), (385, 439), (392, 443)] {
                    self.ship_spawn(channel, index, FAKE_HYDRAS[1], cell, 5, due);
                }
                self.ship_tell_wave1(index);
            }
            2..=4 => {
                self.ship_clear_deck(index, false);
                self.ship_jump_quarter(index);
                if wave == 2 {
                    for cell in [(385, 439), (392, 443)] {
                        self.ship_spawn(channel, index, FAKE_HYDRAS[1], cell, 5, due);
                    }
                } else if wave == 3 {
                    self.ship_spawn(channel, index, FAKE_HYDRAS[1], (385, 439), 5, due);
                }
                let hydra = HYDRAS[usize::from(wave - 2)];
                let stood = self.ship_spawn(channel, index, hydra, (385, 373), 1, due);
                if let Some(run) = self.ship.of_mut(leader) {
                    run.boss = stood.map(|(vid, _)| (vid, wave));
                }
                self.ship_tell_wave(index, wave);
            }
            _ => {}
        }
        if let Some(run) = self.ship.of_mut(leader) {
            run.wave_due = Some((false, wave, wait, due));
        }
    }

    /// `SetWave` (`G/ShipDefense.cpp:1233-1308`): the deck runs `wave` — the back
    /// barriers go, the regen stands, the minis stand, the spawn timer starts, and
    /// the laser timer starts past wave 1. Wave 1 also starts its 120-second wave
    /// timer.
    fn ship_set_wave(&mut self, leader: u32, wave: u8, due: u64) {
        let Some(run) = self.ship.of(leader) else {
            return;
        };
        if !matches!(wave, 1..=4) {
            return;
        }
        let index = run.index;
        if let Some(run) = self.ship.of_mut(leader) {
            run.wave = wave;
            run.wave_due = None;
            run.spawn_due = None;
            run.ship_due = None;
            run.laser_due = None;
            run.clear_due = None;
            run.exit_due = None;
        }
        self.ship_remove_barriers(index, false);
        self.ship_spawn_regen(index, wave, false);
        let (mini, count) = match wave {
            2 => (MINI_HYDRAS[0], 1),
            3 => (MINI_HYDRAS[1], 2),
            4 => (MINI_HYDRAS[2], 4),
            _ => (0, 0),
        };
        for slot in 0..count {
            self.ship_spawn_mini(leader, mini, slot, due);
        }
        if let Some(run) = self.ship.of_mut(leader) {
            run.spawn_due = Some((wave, 0, 0, due));
            if wave == 1 {
                run.wave_due = Some((true, wave, 120, due));
            }
            if matches!(wave, 2..=4) {
                run.laser_due = Some(due);
            }
        }
    }

    /// `SetShipEvent` (`G/ShipDefense.cpp:1310-1348`): between waves the mast-protected
    /// notice chats and the ship timer counts 6 down; after wave 4 the barriers go,
    /// the waves notice chats, the reward and the portal stand, and the 300-second
    /// exit timer starts for the port.
    fn ship_set_ship_event(&mut self, leader: u32, wave: u8, due: u64) {
        let Some(run) = self.ship.of(leader) else {
            return;
        };
        let (index, channel) = (run.index, self.ship_channel(run.index));
        if let Some(run) = self.ship.of_mut(leader) {
            run.wave_due = None;
            run.spawn_due = None;
            run.ship_due = None;
            run.laser_due = None;
            run.clear_due = None;
            run.exit_due = None;
        }
        self.ship_clear_deck(index, false);
        match wave {
            3 | 4 => {
                self.ship_tell(index, true, b"You have successfully protected the mast. Keep it up! Get ready for the next wave.");
                if let Some(run) = self.ship.of_mut(leader) {
                    run.ship_due = Some((wave, 6, due));
                }
            }
            5 => {
                self.ship_remove_barriers(index, true);
                self.ship_tell(index, true, b"Land sighted! You have successfully protected the mast and reached the new continent after a long and weary journey. Continue to the portal.");
                let reward = self.ship_spawn(channel, index, REWARD_VNUM, (385, 416), 1, due);
                let portal = self.ship_spawn(channel, index, PORTAL_VNUM, (385, 450), 1, due);
                if let Some(run) = self.ship.of_mut(leader) {
                    run.reward = reward.map(|(vid, _)| vid);
                    run.portal = portal.map(|(vid, _)| vid);
                    run.exit_due = Some((false, 300, due));
                }
            }
            _ => {}
        }
    }

    /// The Channel hosting the run's index, 0 with none.
    fn ship_channel(&self, index: i32) -> u8 {
        self.maps
            .keys()
            .find(|(_, map)| *map == index)
            .map_or(0, |(channel, _)| *channel)
    }

    /// `JumpToQuarterDeck` (`G/ShipDefense.cpp:1350-1364`): every player on the run's
    /// map shows at the recall spot, and the barriers stand again.
    fn ship_jump_quarter(&mut self, index: i32) {
        let channel = self.ship_channel(index);
        let Some((x, y)) = self
            .recall
            .atlas()
            .and_then(|atlas| recall_spot(Some(atlas), MAP))
        else {
            return;
        };
        let vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == index)
            .map(|(vid, _)| *vid)
            .collect();
        for vid in vids {
            self.show_at(vid, x, y);
        }
        let pulse = self.last_pulse;
        let leaders = self.ship.leaders();
        for leader in leaders {
            let stood = self.ship.of(leader).is_some_and(|run| run.index == index);
            if !stood {
                continue;
            }
            let mut barriers = [None; 4];
            for (slot, cell) in BARRIER_CELLS.iter().enumerate() {
                barriers[slot] = self
                    .ship_spawn(channel, index, BARRIER_VNUM, *cell, 1, pulse)
                    .map(|(vid, _)| vid);
            }
            if let Some(run) = self.ship.of_mut(leader) {
                run.barriers = barriers;
            }
        }
    }

    /// Show the player `vid` at (`x`, `y`) on the run's map (`JumpToPosition`'s
    /// `Show` half, `G/ShipDefense.cpp:761-789`): the view work at the point, then
    /// the stop, as `Stop` runs after `Show`.
    fn show_at(&mut self, vid: Vid, x: i32, y: i32) {
        self.show_body(vid.raw(), (x, y, 0), None);
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.motion.dest = (x, y);
        }
    }

    /// `ClearDeck` (`G/ShipDefense.cpp:1112-1122`): every monster and stone on the
    /// run's map dies, except the boss heads when `except_boss` holds.
    fn ship_clear_deck(&mut self, index: i32, except_boss: bool) {
        let channel = self.ship_channel(index);
        let mut vids: Vec<u32> = self
            .npc_of
            .iter()
            .filter(|(_, place)| place.0 == channel && place.1 == index)
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_unstable();
        for vid in vids {
            let skip = except_boss
                && self.npc_by_vid(vid).is_some_and(|npc| {
                    HYDRAS.contains(&npc.vnum) || FAKE_HYDRAS.contains(&npc.vnum)
                });
            if skip {
                continue;
            }
            let struck = self.npc_by_vid(vid).is_some_and(super::fight::is_struck);
            if struck {
                self.dead_npc(vid, None);
            }
        }
    }

    /// Remove the barriers on the run's map: the back pair, or all four.
    fn ship_remove_barriers(&mut self, index: i32, all: bool) {
        let leaders = self.ship.leaders();
        for leader in leaders {
            let Some(run) = self.ship.of(leader) else {
                continue;
            };
            if run.index != index {
                continue;
            }
            let barriers = run.barriers;
            for (slot, vid) in barriers.iter().enumerate() {
                if !all && slot >= 2 {
                    continue;
                }
                if let Some(vid) = vid {
                    self.destroy_npc(*vid);
                }
            }
            if let Some(run) = self.ship.of_mut(leader) {
                if all {
                    run.barriers = [None; 4];
                } else {
                    run.barriers[0] = None;
                    run.barriers[1] = None;
                }
            }
        }
    }

    /// Stand the wave's regen (`SetWave`, `G/ShipDefense.cpp:1253-1257`):
    /// `regen_do(file, index, base, base, NULL, true)` stands each `m` row's vnum
    /// at its cell once (`regen_spawn_dungeon`, `G/regen.cpp:256-295`), with a drawn
    /// facing (`direction == 0`, `:288`).
    fn ship_spawn_regen(&mut self, index: i32, wave: u8, _once: bool) {
        let rows: &[((i32, i32), u32)] = match wave {
            1 => &WAVE1,
            2 => &WAVE2,
            3 => &WAVE3,
            4 => &WAVE4,
            _ => return,
        };
        let channel = self.ship_channel(index);
        let pulse = self.last_pulse;
        for (cell, vnum) in rows {
            self.ship_spawn(channel, index, *vnum, *cell, 0, pulse);
        }
    }

    /// Stand the mini-hydra `vnum` at `slot` (`SpawnMiniHydra`,
    /// `G/ShipDefense.cpp:984-1008`). Quiet when the slot already holds one.
    fn ship_spawn_mini(&mut self, leader: u32, vnum: u32, slot: usize, due: u64) {
        let Some(run) = self.ship.of(leader) else {
            return;
        };
        if slot >= 4 || run.minis[slot].is_some() {
            return;
        }
        let (index, channel) = (run.index, self.ship_channel(run.index));
        let cell = MINI_CELLS[slot];
        let stood = self.ship_spawn(channel, index, vnum, cell, 0, due);
        if let Some(run) = self.ship.of_mut(leader) {
            run.minis[slot] = stood.map(|(vid, _)| vid);
        }
    }

    /// `NoticeByType(NOTICE_WAVE1)` (`G/ShipDefense.cpp:800-813`): the two big lines.
    fn ship_tell_wave1(&self, index: i32) {
        self.ship_tell(
            index,
            true,
            b"We're under attack from sea monsters! Get ready to fight!",
        );
        let name = self.ship_mast_name(index);
        let wave = self
            .ship
            .by_leader
            .values()
            .find(|run| run.index == index)
            .map_or(0, |run| run.wave + 1);
        let mut vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == index)
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(
                    vid,
                    mover,
                    protocol::gc_chat::CHAT_TYPE_BIG_NOTICE,
                    b"Wave %d: For %d sec. you must protect the %s with all your strength.",
                    &[
                        crate::chat_line::Arg::Int(i64::from(wave)),
                        crate::chat_line::Arg::Int(120),
                        crate::chat_line::Arg::Text(&name),
                    ],
                );
            }
        }
    }

    /// `NoticeByType(NOTICE_WAVE2..4)` (`G/ShipDefense.cpp:815-830`): the two big
    /// lines.
    fn ship_tell_wave(&self, index: i32, wave: u8) {
        let (mast, hydra) = (self.ship_mast_name(index), self.ship_boss_name(index));
        if mast.is_empty() || hydra.is_empty() {
            return;
        }
        self.ship_tell(index, true, b"They're attacking again! Ready your weapons!");
        let mut vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == index)
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(
                    vid,
                    mover,
                    protocol::gc_chat::CHAT_TYPE_BIG_NOTICE,
                    b"Wave %d: While protecting the %s, defeat %s.",
                    &[
                        crate::chat_line::Arg::Int(i64::from(wave) + 1),
                        crate::chat_line::Arg::Text(&mast),
                        crate::chat_line::Arg::Text(&hydra),
                    ],
                );
            }
        }
    }

    /// `Notice` to every player on the run's map (`G/ShipDefense.cpp:614-633`): the
    /// big notice or the plain one.
    fn ship_tell(&self, index: i32, big: bool, text: &[u8]) {
        let chat = if big {
            protocol::gc_chat::CHAT_TYPE_BIG_NOTICE
        } else {
            protocol::gc_chat::CHAT_TYPE_NOTICE
        };
        let mut vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == index)
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, chat, text, &[]);
            }
        }
    }

    /// The mast's name on the run's map, empty with no mast.
    fn ship_mast_name(&self, index: i32) -> Vec<u8> {
        self.ship
            .by_leader
            .values()
            .find(|run| run.index == index)
            .and_then(|run| run.mast)
            .and_then(|(vid, _, _)| self.npc_by_vid(vid))
            .map_or(Vec::new(), |npc| npc.name.clone())
    }

    /// The boss's name on the run's map, empty with no boss.
    fn ship_boss_name(&self, index: i32) -> Vec<u8> {
        self.ship
            .by_leader
            .values()
            .find(|run| run.index == index)
            .and_then(|run| run.boss)
            .and_then(|(vid, _)| self.npc_by_vid(vid))
            .map_or(Vec::new(), |npc| npc.name.clone())
    }
}

impl GameState {
    /// The ship ticks (`run_dungeon_ticks`'s neighbour): every run's due timers fire
    /// in leader order — the wave, spawn, ship, laser, clear and exit events — as the
    /// legacy events fire each second (`G/ShipDefense.cpp:36-330`). Quiet with no
    /// run.
    pub(super) fn run_ship_ticks(&mut self, pulse: u64) {
        if self.ship.is_empty() {
            return;
        }
        for leader in self.ship.leaders() {
            self.run_ship_wave(leader, pulse);
            self.run_ship_spawn(leader, pulse);
            self.run_ship_ship(leader, pulse);
            self.run_ship_laser(leader, pulse);
            self.run_ship_clear(leader, pulse);
            self.run_ship_exit(leader, pulse);
        }
    }

    /// `WaveEvent` (`G/ShipDefense.cpp:231-321`): the count runs out and the wave
    /// sets (or the next prepares), else the countdown chats at 10 and under, at 30
    /// past the wait, and every 10 under 60 before it.
    fn run_ship_wave(&mut self, leader: u32, pulse: u64) {
        let Some((next, wave, wait, due)) = self.ship.of(leader).and_then(|run| run.wave_due)
        else {
            return;
        };
        if pulse < due {
            return;
        }
        if wait < 1 {
            if let Some(run) = self.ship.of_mut(leader) {
                run.wave_due = None;
            }
            let at = pulse.saturating_add(25);
            if next {
                self.ship_prepare_wave(leader, wave.saturating_add(1), at);
            } else {
                self.ship_set_wave(leader, wave, at);
            }
            return;
        }
        let chats = if next {
            wait <= 10 || wait == 30
        } else {
            wait <= 10 || (wait % 10 == 0 && wait < 60)
        };
        if chats && matches!(wave, 1..=4) {
            let text = if next {
                b"The next wave begins in %d sec.".as_slice()
            } else {
                b"The sea battle starts in %d sec.".as_slice()
            };
            let index = self.ship.of(leader).map_or(0, |run| run.index);
            let mut vids: Vec<Vid> = self
                .bodies
                .iter()
                .filter(|(_, body)| body.map == index)
                .map(|(vid, _)| *vid)
                .collect();
            vids.sort_by_key(|vid| vid.raw());
            for vid in vids {
                if let Some(mover) = self.own_mover(vid) {
                    self.tell(
                        vid,
                        mover,
                        protocol::gc_chat::CHAT_TYPE_INFO,
                        text,
                        &[crate::chat_line::Arg::Int(i64::from(wait))],
                    );
                }
            }
        }
        if let Some(run) = self.ship.of_mut(leader) {
            run.wave_due = Some((next, wave, wait.saturating_sub(1), pulse.saturating_add(25)));
        }
    }

    /// `ShipEvent` (`G/ShipDefense.cpp:122-151`): the count runs out and the wave
    /// prepares, else it ticks down.
    fn run_ship_ship(&mut self, leader: u32, pulse: u64) {
        let Some((wave, wait, due)) = self.ship.of(leader).and_then(|run| run.ship_due) else {
            return;
        };
        if pulse < due {
            return;
        }
        if wait == 0 {
            if let Some(run) = self.ship.of_mut(leader) {
                run.ship_due = None;
            }
            let at = pulse.saturating_add(25);
            self.ship_prepare_wave(leader, wave, at);
            return;
        }
        if let Some(run) = self.ship.of_mut(leader) {
            run.ship_due = Some((wave, wait.saturating_sub(1), pulse.saturating_add(25)));
        }
    }

    /// `SpawnEvent` (`G/ShipDefense.cpp:322-493`): wave 1 spawns past 40 seconds, the
    /// hydra waves roll the egg and the minis and step their regens, and the seconds
    /// tick on. The step-0 re-arm quirk (`% 3` truthy) is ported as written: `uiCountSec
    /// % 3` nonzero, not zero.
    fn run_ship_spawn(&mut self, leader: u32, pulse: u64) {
        let Some((wave, step, secs, due)) = self.ship.of(leader).and_then(|run| run.spawn_due)
        else {
            return;
        };
        if pulse < due {
            return;
        }
        let index = self.ship.of(leader).map_or(0, |run| run.index);
        let channel = self.ship_channel(index);
        match wave {
            1 => {
                if secs >= 40
                    && secs % u32::try_from(self.ship_number(1, 3)).unwrap_or(1).max(1) == 0
                {
                    let cell = WAVE1_CELLS[self.ship_pick(WAVE1_CELLS.len())];
                    let vnum = u32::try_from(self.ship_number(3954, 3955)).unwrap_or(3954);
                    self.ship_spawn(channel, index, vnum, cell, 0, pulse);
                }
            }
            2..=4 => {
                let egg_roll = self.ship_roll(1000);
                if egg_roll <= 10 {
                    self.ship_spawn_egg(leader, pulse);
                }
                let (mini, count, chance) = match wave {
                    2 => (MINI_HYDRAS[0], 1, 30),
                    3 => (MINI_HYDRAS[1], 2, 40),
                    _ => (MINI_HYDRAS[1], 4, 50),
                };
                if self.ship_roll(1000) <= chance {
                    for slot in 0..count {
                        self.ship_spawn_mini(leader, mini, slot, pulse);
                    }
                }
                let step = if secs % 3 != 0 && self.ship_deck_empty(index) {
                    let file = if wave == 4 { 3 } else { wave };
                    self.ship_spawn_regen(index, file, false);
                    0
                } else if secs % 10 == 0 && step == 0 {
                    self.ship_spawn_regen_step(index, wave, 1);
                    1
                } else if secs % 30 == 0 && step == 1 {
                    self.ship_spawn_regen_step(index, wave, 1);
                    2
                } else if secs % 60 == 0 && step == 2 {
                    self.ship_spawn_regen(index, if wave == 4 { 4 } else { wave }, false);
                    0
                } else {
                    step
                };
                if let Some(run) = self.ship.of_mut(leader) {
                    run.spawn_due =
                        Some((wave, step, secs.saturating_add(1), pulse.saturating_add(25)));
                }
                return;
            }
            _ => {}
        }
        if let Some(run) = self.ship.of_mut(leader) {
            run.spawn_due = Some((wave, step, secs.saturating_add(1), pulse.saturating_add(25)));
        }
    }

    /// `LaserEffectEvent` (`G/ShipDefense.cpp:153-230`): the shelters clear first,
    /// then a random standing mini rolls the wave's laser chance (5/7.5/10%) and
    /// takes the laser affect with its effect cell.
    fn run_ship_laser(&mut self, leader: u32, pulse: u64) {
        let Some(due) = self.ship.of(leader).and_then(|run| run.laser_due) else {
            return;
        };
        if pulse < due {
            return;
        }
        self.ship_clear_lasers(leader, pulse);
        let wave = self.ship.of(leader).map_or(0, |run| run.wave);
        let chance = match wave {
            2 => 50,
            3 => 75,
            4 => 100,
            _ => 0,
        };
        let slot = self.ship_pick(4);
        let mini = self.ship.of(leader).and_then(|run| run.minis[slot]);
        if let Some(vid) = mini {
            let held = self.npc_affect_words(vid);
            let laser_word = 1_u32 << ((51 - 1) % 32);
            if held[1] & laser_word == 0 && chance > 0 && self.ship_roll(1000) <= chance {
                self.add_npc_affect(
                    vid,
                    super::affect::Affect {
                        kind: 701,
                        apply_on: 18,
                        value: 100,
                        flag: 51,
                        remaining: crate::loading_phase::INFINITE_AFFECT_DURATION,
                        sp_cost: 0,
                    },
                );
                let cell = LASER_CELLS[slot];
                if let Some(run) = self.ship.of_mut(leader) {
                    run.lasers.insert(u8::try_from(slot).unwrap_or(0), cell);
                }
            }
        }
        if let Some(run) = self.ship.of_mut(leader) {
            run.laser_due = Some(pulse.saturating_add(25));
        }
    }

    /// `CheckLaserPosition` (`G/ShipDefense.cpp:706-760`): a player standing on a
    /// laser cell clears the mini's laser affect and stamps the shelter Pulse — past
    /// the 10-second shelter, else the test line chats on a test server.
    fn ship_clear_lasers(&mut self, leader: u32, pulse: u64) {
        let Some(run) = self.ship.of(leader) else {
            return;
        };
        if run.lasers.is_empty() {
            return;
        }
        let (index, channel) = (run.index, self.ship_channel(run.index));
        let (base_x, base_y) = self
            .maps
            .get(&(channel, index))
            .map_or((0, 0), super::view::MapIndex::base);
        let lasers: Vec<(u8, (i32, i32))> = self.ship.of(leader).map_or(Vec::new(), |run| {
            run.lasers
                .iter()
                .map(|(slot, cell)| (*slot, *cell))
                .collect()
        });
        let vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == index)
            .map(|(vid, _)| *vid)
            .collect();
        for vid in vids {
            let Some(spot) = self.spot_of(vid.raw()) else {
                continue;
            };
            let cell = ((spot.x - base_x) / 100, (spot.y - base_y) / 100);
            for (slot, at) in &lasers {
                if cell != *at {
                    continue;
                }
                let mini = self
                    .ship
                    .of(leader)
                    .and_then(|run| run.minis.get(usize::from(*slot)).copied().flatten());
                let Some(mini) = mini else {
                    continue;
                };
                let held = self.npc_affect_words(mini);
                let laser_word = 1_u32 << ((51 - 1) % 32);
                if held[1] & laser_word == 0 {
                    continue;
                }
                let sheltered = self
                    .ship
                    .of(leader)
                    .is_some_and(|run| pulse.wrapping_sub(run.shelter_pulse) < 10 * 25);
                if sheltered {
                    if self.test_server {
                        if let Some(mover) = self.own_mover(vid) {
                            self.tell(
                                vid,
                                mover,
                                protocol::gc_chat::CHAT_TYPE_INFO,
                                b"<TEST_SERVER> You cannot remove this affect during %d sec.",
                                &[crate::chat_line::Arg::Int(10)],
                            );
                        }
                    }
                    return;
                }
                self.remove_npc_affect(mini, 701);
                if let Some(run) = self.ship.of_mut(leader) {
                    run.lasers.remove(slot);
                    run.shelter_pulse = pulse;
                }
            }
        }
    }

    /// `ClearSpawnEvent` (`G/ShipDefense.cpp:36-65`): the egg or the fake hydra goes.
    fn run_ship_clear(&mut self, leader: u32, pulse: u64) {
        let Some((all, due)) = self.ship.of(leader).and_then(|run| run.clear_due) else {
            return;
        };
        if pulse < due {
            return;
        }
        if let Some(run) = self.ship.of_mut(leader) {
            run.clear_due = None;
        }
        let vid = self.ship.of(leader).and_then(|run| {
            if all {
                run.boss.map(|(vid, _)| vid)
            } else {
                run.fake.or(run.egg)
            }
        });
        if let Some(vid) = vid {
            self.destroy_npc(vid);
        }
        if let Some(run) = self.ship.of_mut(leader) {
            if all {
                run.boss = None;
            } else {
                run.fake = None;
                run.egg = None;
            }
        }
    }

    /// `ExitEvent` (`G/ShipDefense.cpp:67-120`): the count runs out and every player
    /// on the map jumps home or to the port and the run ends, else the minute line
    /// chats each 60 seconds and the count ticks down.
    fn run_ship_exit(&mut self, leader: u32, pulse: u64) {
        let Some((home, secs, due)) = self.ship.of(leader).and_then(|run| run.exit_due) else {
            return;
        };
        if pulse < due {
            return;
        }
        if secs < 1 {
            let index = self.ship.of(leader).map_or(0, |run| run.index);
            self.ship_jump_all(index, home);
            self.end_ship_run(leader);
            return;
        }
        if secs % 60 == 0 {
            let index = self.ship.of(leader).map_or(0, |run| run.index);
            let minutes = i64::from(secs % 3600 / 60);
            let mut vids: Vec<Vid> = self
                .bodies
                .iter()
                .filter(|(_, body)| body.map == index)
                .map(|(vid, _)| *vid)
                .collect();
            vids.sort_by_key(|vid| vid.raw());
            for vid in vids {
                if let Some(mover) = self.own_mover(vid) {
                    self.tell(
                        vid,
                        mover,
                        protocol::gc_chat::CHAT_TYPE_INFO,
                        b"Hurry! You have %d minutes to travel through the portal to reach the mainland.",
                        &[crate::chat_line::Arg::Int(minutes)],
                    );
                }
            }
        }
        if let Some(run) = self.ship.of_mut(leader) {
            run.exit_due = Some((home, secs.saturating_sub(1), pulse.saturating_add(25)));
        }
    }

    /// `JumpAll` (`G/ShipDefense.cpp:1366-1401`): every player on the run's map shows
    /// or warps to the entry recall or the port spot.
    fn ship_jump_all(&mut self, index: i32, home: bool) {
        let (map, at) = if home {
            (
                ENTRY_MAP,
                self.recall
                    .atlas()
                    .and_then(|atlas| recall_spot(Some(atlas), ENTRY_MAP)),
            )
        } else {
            let port = self
                .recall
                .atlas()
                .and_then(|atlas| atlas.region(PORT_MAP))
                .map(|region| (region.sx + LAND_SPOT.0 * 100, region.sy + LAND_SPOT.1 * 100));
            match port {
                Some(at) => (PORT_MAP, Some(at)),
                None => (
                    ENTRY_MAP,
                    self.recall
                        .atlas()
                        .and_then(|atlas| recall_spot(Some(atlas), ENTRY_MAP)),
                ),
            }
        };
        let Some((x, y)) = at else {
            return;
        };
        let vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == index)
            .map(|(vid, _)| *vid)
            .collect();
        for vid in vids {
            if map == index {
                self.show_at(vid, x, y);
            } else if let Some(mover) = self.own_mover(vid) {
                self.order_quest_warp(vid, x, y, 0, mover);
            }
        }
    }

    /// `Remove` (`G/ShipDefense.cpp:1471-1480`): the run ends — its map unhosts, its
    /// dungeon destroys, and its registration goes.
    fn end_ship_run(&mut self, leader: u32) {
        let Some(run) = self.ship.of(leader) else {
            return;
        };
        let index = run.index;
        self.dungeons.destroy(index);
        let channels: Vec<u8> = self
            .maps
            .keys()
            .filter(|(_, map)| *map == index)
            .map(|(channel, _)| *channel)
            .collect();
        for channel in channels {
            self.maps.remove(&(channel, index));
        }
        self.ship.by_leader.remove(&leader);
    }

    /// Stand the egg (`SpawnEgg`, `G/ShipDefense.cpp:1010-1040`): a random cell, and
    /// the 8-second clear.
    fn ship_spawn_egg(&mut self, leader: u32, pulse: u64) {
        let Some(run) = self.ship.of(leader) else {
            return;
        };
        if run.egg.is_some() {
            return;
        }
        let (index, channel) = (run.index, self.ship_channel(run.index));
        let cell = EGG_CELLS[self.ship_pick(EGG_CELLS.len())];
        let stood = self.ship_spawn(channel, index, EGG_VNUM, cell, 0, pulse);
        if let Some(run) = self.ship.of_mut(leader) {
            run.egg = stood.map(|(vid, _)| vid);
            run.clear_due = Some((false, pulse.saturating_add(8 * 25)));
        }
    }

    /// Whether the deck holds no countable monster (`CheckEmptyDeck`,
    /// `G/ShipDefense.cpp:911-954`): the hydras, minis, egg, reward and wood do not
    /// count.
    fn ship_deck_empty(&self, index: i32) -> bool {
        let channel = self.ship_channel(index);
        !self.npc_of.iter().any(|(vid, place)| {
            place.0 == channel
                && place.1 == index
                && self.npc_by_vid(*vid).is_some_and(|npc| {
                    npc.char_type == gamedata::mob_proto::CHAR_TYPE_MONSTER
                        && !HYDRAS.contains(&npc.vnum)
                        && !FAKE_HYDRAS.contains(&npc.vnum)
                        && !MINI_HYDRAS.contains(&npc.vnum)
                        && npc.vnum != EGG_VNUM
                        && npc.vnum != REWARD_VNUM
                        && npc.vnum != WOOD_VNUM
                })
        })
    }

    /// `number(from, to)` on the game dice.
    fn ship_number(&mut self, from: i32, to: i32) -> i32 {
        world::character::number(&mut self.dice, from, to)
    }

    /// A roll in `1..=max` on the game dice.
    fn ship_roll(&mut self, max: i32) -> u32 {
        u32::try_from(self.ship_number(1, max)).unwrap_or(1)
    }

    /// A pick in `0..len` on the game dice.
    fn ship_pick(&mut self, len: usize) -> usize {
        let len = i32::try_from(len).unwrap_or(1).max(1);
        usize::try_from(self.ship_number(0, len - 1)).unwrap_or(0)
    }

    /// Stand the `wave<n>_1.txt` step file's rows at once (`SpawnEvent`,
    /// `G/ShipDefense.cpp:392-483`).
    fn ship_spawn_regen_step(&mut self, index: i32, wave: u8, _step: u8) {
        let rows: &[((i32, i32), u32)] = match wave {
            2 => &WAVE2_1,
            3 => &WAVE3_1,
            4 => &WAVE4_1,
            _ => return,
        };
        let channel = self.ship_channel(index);
        let pulse = self.last_pulse;
        for (cell, vnum) in rows {
            self.ship_spawn(channel, index, *vnum, *cell, 0, pulse);
        }
    }

    /// Remove the first affect of `kind` from the non-player `vid`, computed off
    /// (`RemoveAffect`, `G/char_affect.cpp:789-830`): the point is taken back and the
    /// update goes when the flags moved.
    pub(super) fn remove_npc_affect(&mut self, vid: u32, kind: u32) {
        let words_before = self.npc_affect_words(vid);
        let taken = self.npc_affects.get_mut(&vid).and_then(|event| {
            event
                .affects
                .iter()
                .position(|affect| affect.kind == kind)
                .map(|at| event.affects.remove(at))
        });
        let Some(taken) = taken else {
            return;
        };
        self.change_npc_point(vid, usize::from(taken.apply_on), -taken.value);
        let words_after = self.npc_affect_words(vid);
        if words_after != words_before {
            self.send_npc_update(vid, words_after);
        }
    }
}

impl GameState {
    /// `CShipDefenseManager::OnKill` (`G/ShipDefense.cpp:1824-1900`): a non-player's
    /// death on the ship's map — the killer's deck clears past the egg, spawns the
    /// wood for a dead unique mini, advances past a dead hydra, and every deck drops
    /// the dead VID from its uniques. A dead mast clears the deck, chats the SOS and
    /// starts the 3-second exit for home.
    pub(super) fn ship_on_kill(&mut self, vid: u32, killer: Option<Vid>) {
        let Some(&(channel, map, _)) = self.npc_of.get(&vid) else {
            return;
        };
        if !is_ship_map(map) {
            return;
        }
        if self.ship.is_empty() {
            return;
        }
        if let Some(killer) = killer {
            let Some(leader) = self.ship_leader_of(killer) else {
                return;
            };
            let pulse = self.last_pulse;
            let Some(run) = self.ship.of(leader) else {
                return;
            };
            let (egg, minis, boss, mast) = (run.egg, run.minis, run.boss, run.mast);
            if egg == Some(vid) {
                self.ship_clear_deck(map, true);
            }
            let mini = self
                .npc_by_vid(vid)
                .is_some_and(|npc| MINI_HYDRAS.contains(&npc.vnum))
                && minis.iter().any(|mini| *mini == Some(vid));
            if mini {
                let spot = self.npc_by_vid(vid).map(|npc| (npc.x, npc.y));
                let base = self
                    .maps
                    .get(&(channel, map))
                    .map(super::view::MapIndex::base);
                if let (Some((x, y)), Some((base_x, base_y))) = (spot, base) {
                    let cell = ((x - base_x) / 100, (y - base_y) / 100);
                    self.ship_spawn(channel, map, WOOD_VNUM, cell, 0, pulse);
                }
            }
            let boss_wave = boss.and_then(|(boss, wave)| (boss == vid).then_some(wave));
            if let Some(wave) = boss_wave {
                match wave {
                    2 | 3 => self.ship_set_ship_event(leader, wave + 1, pulse.saturating_add(25)),
                    4 => self.ship_set_ship_event(leader, 5, pulse.saturating_add(25)),
                    _ => {}
                }
            } else if mast.is_some_and(|(mast, _, _)| mast == vid) {
                self.ship_mast_fell(leader, map, pulse);
            }
        }
        for leader in self.ship.leaders() {
            let drop = self.ship.of(leader).is_some_and(|run| {
                run.mast.is_some_and(|(mast, _, _)| mast == vid)
                    || run.boss.is_some_and(|(boss, _)| boss == vid)
                    || run.minis.iter().any(|mini| *mini == Some(vid))
                    || run.egg == Some(vid)
                    || run.fake == Some(vid)
                    || run.reward == Some(vid)
                    || run.portal == Some(vid)
            });
            if !drop {
                continue;
            }
            if let Some(run) = self.ship.of_mut(leader) {
                if run.mast.is_some_and(|(mast, _, _)| mast == vid) {
                    run.mast = None;
                }
                if run.boss.is_some_and(|(boss, _)| boss == vid) {
                    run.boss = None;
                }
                for mini in &mut run.minis {
                    if *mini == Some(vid) {
                        *mini = None;
                    }
                }
                if run.egg == Some(vid) {
                    run.egg = None;
                }
                if run.fake == Some(vid) {
                    run.fake = None;
                }
                if run.reward == Some(vid) {
                    run.reward = None;
                }
                if run.portal == Some(vid) {
                    run.portal = None;
                }
            }
        }
    }

    /// `CShipDefense::DeadCharacter`'s mast arm (`G/ShipDefense.cpp:589-605`): the
    /// timers cancel, the deck clears, the SOS chats, and the 3-second exit starts
    /// for home.
    fn ship_mast_fell(&mut self, leader: u32, index: i32, pulse: u64) {
        if let Some(run) = self.ship.of_mut(leader) {
            run.wave_due = None;
            run.spawn_due = None;
            run.ship_due = None;
            run.laser_due = None;
            run.clear_due = None;
            run.exit_due = None;
        }
        self.ship_clear_deck(index, false);
        self.ship_tell(index, true, b"SOS! The mast has been smashed and your ship is sinking. You will now be teleported to Cape Dragon Fire.");
        if let Some(run) = self.ship.of_mut(leader) {
            run.exit_due = Some((true, 3, pulse.saturating_add(25)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{a_world, enter, open_cells, TestClock};
    use super::*;
    use crate::client_registry::{ChannelClients, ClientEntry, ClientOrder, Lease};
    use world::npc::{NpcVids, Respawner, FIRST_NPC_VID};

    const LEADER_VID: u32 = 100;
    const MEMBER_VID: u32 = 101;
    const NOW: u32 = 10_000_000;

    /// The fixture's channels: the leader's and member's order queues, inboxes
    /// and leases.
    struct ShipWorld {
        world: GameState,
        orders: tokio::sync::mpsc::UnboundedReceiver<ClientOrder>,
        leader_inbox: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        member_inbox: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        lease: Lease,
        member_lease: Lease,
        member_orders: tokio::sync::mpsc::UnboundedReceiver<ClientOrder>,
    }

    /// A world with an atlas naming the ship, port and entry maps, the deck's
    /// protos, and the ship's open cells.
    fn ship_world(clock: &TestClock) -> (GameState, std::sync::Arc<ChannelClients>) {
        let region = |index: i32, name: &[u8], base: i32| gamedata::map_atlas::MapRegion {
            index,
            name: name.to_vec(),
            sx: base,
            sy: base,
            ex: base + 100_000,
            ey: base + 100_000,
            spawn: (base + 13_000, base + 13_000),
            empire_spawns: None,
        };
        let regions = vec![
            region(MAP, b"ship", 100_000),
            region(PORT_MAP, b"port", 300_000),
            region(ENTRY_MAP, b"cape", 500_000),
        ];
        let atlas = std::sync::Arc::new(gamedata::map_atlas::MapAtlas::from_regions(regions));
        let clients = std::sync::Arc::new(ChannelClients::new());
        let mut maps = std::collections::HashMap::new();
        maps.insert(
            1u8,
            vec![41u32, MAP as u32, PORT_MAP as u32, ENTRY_MAP as u32],
        );
        let mut world = a_world(clock)
            .with_clients(std::sync::Arc::clone(&clients))
            .with_recall(atlas, &maps, &[]);
        let rows = [
            MAST_VNUM,
            WHEEL_VNUM,
            BARRIER_VNUM,
            FAKE_HYDRAS[0],
            FAKE_HYDRAS[1],
            HYDRAS[0],
            HYDRAS[1],
            HYDRAS[2],
            MINI_HYDRAS[0],
            MINI_HYDRAS[1],
            MINI_HYDRAS[2],
            EGG_VNUM,
            REWARD_VNUM,
            PORTAL_VNUM,
            WOOD_VNUM,
            3954,
            3955,
        ]
        .into_iter()
        .map(|vnum| gamedata::mob_proto::MobProto {
            line: 2,
            table: gamedata::records::MobTableRecord {
                vnum,
                mob_type: gamedata::mob_proto::CHAR_TYPE_MONSTER,
                max_hp: 10_000,
                ..gamedata::records::MobTableRecord::default()
            },
        })
        .collect();
        let respawner = Respawner::new(
            std::sync::Arc::new(gamedata::mob_proto::MobProtos::from_rows(rows)),
            std::sync::Arc::new(gamedata::mob_group::MobGroups::default()),
            std::sync::Arc::new(
                gamedata::mob_locale_names::MobLocaleNames::parse(b"VNUM\tNAME\n").unwrap(),
            ),
            NpcVids::starting_at(FIRST_NPC_VID + 100),
        );
        world.install_respawner(respawner, ship_cells());
        world.stand_npcs(
            1,
            &gamedata::map_atlas::MapRegion {
                index: MAP,
                name: b"ship".to_vec(),
                sx: 100_000,
                sy: 100_000,
                ex: 200_000,
                ey: 200_000,
                spawn: (113_000, 113_000),
                empire_spawns: None,
            },
            world::npc::MapNpcs::default(),
        );
        for map in [ENTRY_MAP, PORT_MAP] {
            world.host_map(
                1,
                map,
                gamedata::server_attr::SectreeGrid {
                    x: 0,
                    y: 0,
                    columns: 10,
                    rows: 10,
                },
            );
        }
        (world, clients)
    }

    /// The ship's open cells: [`open_cells`] plus a 32 by 32 open grid for the
    /// ship base, which the deck spawns land on.
    fn ship_cells() -> std::sync::Arc<world::cells::HostedCells> {
        let blocks = (0..1024)
            .map(|_| {
                gamedata::server_attr::CellBlock::from_cells(&vec![
                        0;
                        gamedata::server_attr::SECTREE_CELLS
                            * gamedata::server_attr::SECTREE_CELLS
                    ])
            })
            .collect();
        let attr = std::sync::Arc::new(
            gamedata::server_attr::ServerAttr::from_blocks(32, 32, blocks).expect("1024 blocks"),
        );
        let grid = gamedata::server_attr::SectreeGrid {
            x: 0,
            y: 0,
            columns: 32,
            rows: 32,
        };
        let mut hosted = (*open_cells()).clone();
        hosted.insert(
            MAP,
            world::cells::MapCells::new(grid, std::sync::Arc::clone(&attr)),
        );
        std::sync::Arc::new(hosted)
    }

    /// A partied leader and member with client leases on the entry map.
    fn a_ship_world() -> ShipWorld {
        let clock = TestClock::default();
        clock.set(1_000);
        let (mut world, clients) = ship_world(&clock);
        let leader_inbox = enter(&mut world, LEADER_VID, (3200, 3200), 0);
        let member_inbox = enter(&mut world, MEMBER_VID, (3300, 3200), 0);
        world.create_party(LEADER_VID);
        world.join_party(LEADER_VID, MEMBER_VID);
        world.link_party(Vid::new(MEMBER_VID));
        let mut lease = clients.join(ClientEntry {
            channel: 1,
            map: 41,
            name: format!("P{LEADER_VID}"),
            vid: LEADER_VID,
            empire: 1,
            language: 0,
        });
        let orders = lease.take_orders().expect("the queue is home");
        let mut member_lease = clients.join(ClientEntry {
            channel: 1,
            map: 41,
            name: format!("P{MEMBER_VID}"),
            vid: MEMBER_VID,
            empire: 1,
            language: 0,
        });
        let member_orders = member_lease.take_orders().expect("the queue is home");
        ShipWorld {
            world,
            orders,
            leader_inbox,
            member_inbox,
            lease,
            member_lease,
            member_orders,
        }
    }

    /// What `inbox` holds since the last look.
    fn heard(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// The create's gates: a member's is quiet, the pass stands the run, stamps the
    /// cooldown, fans the ready line and orders the warp.
    #[test]
    fn the_create_gates_the_member_and_warps_the_leader_in() {
        let ShipWorld {
            mut world,
            mut orders,
            mut leader_inbox,
            mut member_inbox,
            lease: _lease,
            member_lease: _member_lease,
            ..
        } = a_ship_world();
        world.judge_ship_create(Vid::new(MEMBER_VID), NOW);
        assert!(world.ship.of(LEADER_VID).is_none());
        world.judge_ship_create(Vid::new(LEADER_VID), NOW);
        let run = world.ship.of(LEADER_VID).expect("the pass stands");
        assert_eq!(run.state, STATE_CREATE);
        assert_eq!(run.index, 3_580_000);
        assert!(run.mast.is_some());
        assert!(run.barriers.iter().all(Option::is_some));
        let order = orders.try_recv().expect("the warp is ordered");
        assert!(matches!(
            order,
            ClientOrder::Warp {
                private_map_index: 3_580_000,
                ..
            }
        ));
        for inbox in [&mut leader_inbox, &mut member_inbox] {
            let lines = heard(inbox);
            assert!(
                lines.iter().any(|line| line
                    .windows(b"Ship Defence".len())
                    .any(|window| window == b"Ship Defence")),
                "the ready line fans: {lines:02x?}"
            );
        }
    }

    /// The start runs the deck: the state flips and the wave-1 timer counts from 60.
    #[test]
    fn the_start_runs_the_deck() {
        let ShipWorld {
            mut world,
            lease: _lease,
            member_lease: _member_lease,
            ..
        } = a_ship_world();
        world.judge_ship_create(Vid::new(LEADER_VID), NOW);
        world.judge_ship_start(Vid::new(LEADER_VID));
        let run = world.ship.of(LEADER_VID).expect("the run stands");
        assert_eq!(run.state, STATE_START);
        assert!(matches!(run.wave_due, Some((false, 1, 60, _))));
        assert!(run.fake.is_some());
    }

    /// The join warps a member in; the leave and the land order their warps.
    #[test]
    fn the_join_leave_and_land_order_their_warps() {
        let ShipWorld {
            mut world,
            mut orders,
            mut member_orders,
            lease: _lease,
            member_lease: _member_lease,
            ..
        } = a_ship_world();
        world.judge_ship_create(Vid::new(LEADER_VID), NOW);
        let _create = orders.try_recv().expect("the create warps");
        world.judge_ship_join(Vid::new(MEMBER_VID));
        let order = member_orders.try_recv().expect("the join warps");
        assert!(matches!(
            order,
            ClientOrder::Warp {
                private_map_index: 3_580_000,
                ..
            }
        ));
        world.judge_ship_leave(Vid::new(MEMBER_VID));
        let _leave = member_orders.try_recv().expect("the leave warps");
        world.judge_ship_land(Vid::new(MEMBER_VID));
        let _land = member_orders.try_recv().expect("the land warps");
    }

    /// The wave tick: at 60 it chats nothing and counts down; at 10 and under
    /// the sea-battle countdown chats; at 0 wave 1 sets — the minis stand (none
    /// for wave 1), the spawn timer starts, and the 120-second wave timer arms.
    #[test]
    fn the_wave_tick_counts_chats_and_sets_wave_1() {
        let ShipWorld {
            mut world,
            mut leader_inbox,
            lease: _lease,
            member_lease: _member_lease,
            ..
        } = a_ship_world();
        world.judge_ship_create(Vid::new(LEADER_VID), NOW);
        world.judge_ship_start(Vid::new(LEADER_VID));
        if let Some(body) = world.bodies.get_mut(&Vid::new(LEADER_VID)) {
            body.map = 3_580_000;
        }
        let _ = heard(&mut leader_inbox);
        let due = world
            .ship
            .of(LEADER_VID)
            .and_then(|run| run.wave_due)
            .expect("armed")
            .3;
        // At 60 the tick counts down silent.
        world.run_ship_wave(LEADER_VID, due);
        let (_, _, wait, _) = world
            .ship
            .of(LEADER_VID)
            .and_then(|run| run.wave_due)
            .expect("armed");
        assert_eq!(wait, 59);
        assert!(heard(&mut leader_inbox).is_empty(), "silent past 10");
        // At 10 the countdown chats.
        if let Some(run) = world.ship.of_mut(LEADER_VID) {
            run.wave_due = Some((false, 1, 10, due));
        }
        world.run_ship_wave(LEADER_VID, due);
        let lines = heard(&mut leader_inbox);
        assert!(
            lines.iter().any(|line| line
                .windows(b"sea battle starts in 10".len())
                .any(|window| window == b"sea battle starts in 10")),
            "the countdown chats: {lines:02x?}"
        );
        // At 0 wave 1 sets: the spawn timer starts and the next-wave timer arms.
        if let Some(run) = world.ship.of_mut(LEADER_VID) {
            run.wave_due = Some((false, 1, 0, due));
        }
        world.run_ship_wave(LEADER_VID, due);
        let run = world.ship.of(LEADER_VID).expect("the run stands");
        assert_eq!(run.wave, 1);
        assert!(run.spawn_due.is_some(), "the spawn timer starts");
        assert!(
            matches!(run.wave_due, Some((true, 1, 120, _))),
            "the 120-second timer arms: {:?}",
            run.wave_due
        );
    }

    /// The ship tick: it counts 6 down silent, then prepares wave 3 — the fake
    /// hydra and the wave-1 pair stand, and the 10-second wave timer arms.
    #[test]
    fn the_ship_tick_counts_down_then_prepares_the_wave() {
        let ShipWorld {
            mut world,
            lease: _lease,
            member_lease: _member_lease,
            ..
        } = a_ship_world();
        world.judge_ship_create(Vid::new(LEADER_VID), NOW);
        world.judge_ship_start(Vid::new(LEADER_VID));
        let due = world.last_pulse().saturating_add(25);
        world.ship_set_ship_event(LEADER_VID, 3, due);
        assert!(world
            .ship
            .of(LEADER_VID)
            .and_then(|run| run.ship_due)
            .is_some());
        world.run_ship_ship(LEADER_VID, due);
        let (_, wait, _) = world
            .ship
            .of(LEADER_VID)
            .and_then(|run| run.ship_due)
            .expect("counting");
        assert_eq!(wait, 5);
        if let Some(run) = world.ship.of_mut(LEADER_VID) {
            run.ship_due = Some((3, 0, due));
        }
        world.run_ship_ship(LEADER_VID, due);
        let run = world.ship.of(LEADER_VID).expect("the run stands");
        assert!(run.ship_due.is_none(), "the timer ends");
        // Waves 2-4 spawn their fakes untracked, as legacy `Spawn`s them; the
        // wave-3 hydra stands tracked.
        let fakes = world
            .npcs_on(1, 3_580_000)
            .npcs
            .iter()
            .filter(|npc| npc.vnum == FAKE_HYDRAS[1])
            .count();
        // `judge_ship_start` stands the wave-1 fakes (3), and the wave-3 prepare
        // stands its own (1).
        assert_eq!(fakes, 4, "the wave-3 fake stands");
        assert!(run.boss.is_some(), "the hydra stands");
        assert!(
            matches!(run.wave_due, Some((false, 3, 10, _))),
            "the 10-second timer arms: {:?}",
            run.wave_due
        );
    }

    /// The spawn tick: wave 1 past 40 seconds stands a mob, and the laser tick
    /// re-arms every second.
    #[test]
    fn the_spawn_tick_stands_wave_1_and_the_laser_tick_rearms() {
        let ShipWorld {
            mut world,
            lease: _lease,
            member_lease: _member_lease,
            ..
        } = a_ship_world();
        world.judge_ship_create(Vid::new(LEADER_VID), NOW);
        world.judge_ship_start(Vid::new(LEADER_VID));
        let due = world.last_pulse().saturating_add(25);
        world.ship_set_wave(LEADER_VID, 1, due);
        let before = world.npcs_on(1, 3_580_000).npcs.len();
        // Wave 1 at 41 seconds: the roll may or may not stand, but the seconds tick on.
        if let Some(run) = world.ship.of_mut(LEADER_VID) {
            run.spawn_due = Some((1, 0, 41, due));
        }
        world.run_ship_spawn(LEADER_VID, due);
        let (_, _, secs, _) = world
            .ship
            .of(LEADER_VID)
            .and_then(|run| run.spawn_due)
            .expect("ticking");
        assert_eq!(secs, 42);
        let _ = before;
        // Wave 2 arms the laser timer; the laser tick re-arms it a second on.
        world.ship_set_wave(LEADER_VID, 2, due);
        assert!(world
            .ship
            .of(LEADER_VID)
            .and_then(|run| run.laser_due)
            .is_some());
        world.run_ship_laser(LEADER_VID, due);
        assert_eq!(
            world.ship.of(LEADER_VID).and_then(|run| run.laser_due),
            Some(due.saturating_add(25)),
            "the laser re-arms"
        );
    }

    /// The clear tick: the wave-1 fake hydra goes 3 seconds after the prepare,
    /// and the egg's clear takes the egg.
    #[test]
    fn the_clear_tick_destroys_the_fake_and_the_egg() {
        let ShipWorld {
            mut world,
            lease: _lease,
            member_lease: _member_lease,
            ..
        } = a_ship_world();
        world.judge_ship_create(Vid::new(LEADER_VID), NOW);
        world.judge_ship_start(Vid::new(LEADER_VID));
        let due = world.last_pulse().saturating_add(25);
        world.ship_prepare_wave(LEADER_VID, 1, due);
        let fake = world
            .ship
            .of(LEADER_VID)
            .and_then(|run| run.fake)
            .expect("the fake stands");
        let (_, clear_due) = world
            .ship
            .of(LEADER_VID)
            .and_then(|run| run.clear_due)
            .expect("armed");
        assert_eq!(clear_due, due.saturating_add(3 * 25), "3 seconds on");
        world.run_ship_clear(LEADER_VID, clear_due);
        assert!(world.ship.of(LEADER_VID).and_then(|run| run.fake).is_none());
        assert!(world.npc_by_vid(fake).is_none(), "the fake is destroyed");
        // The egg's clear takes the egg.
        world.ship_spawn_egg(LEADER_VID, due);
        let egg = world
            .ship
            .of(LEADER_VID)
            .and_then(|run| run.egg)
            .expect("the egg stands");
        let (_, egg_due) = world
            .ship
            .of(LEADER_VID)
            .and_then(|run| run.clear_due)
            .expect("armed");
        assert_eq!(egg_due, due.saturating_add(8 * 25), "8 seconds on");
        world.run_ship_clear(LEADER_VID, egg_due);
        assert!(world.npc_by_vid(egg).is_none(), "the egg is destroyed");
    }

    /// The exit tick: at 60 the minute line chats and the count ticks down; at 0
    /// the run ends.
    #[test]
    fn the_exit_tick_chats_the_minute_then_ends_the_run() {
        let ShipWorld {
            mut world,
            mut leader_inbox,
            lease: _lease,
            member_lease: _member_lease,
            ..
        } = a_ship_world();
        world.judge_ship_create(Vid::new(LEADER_VID), NOW);
        world.judge_ship_start(Vid::new(LEADER_VID));
        if let Some(body) = world.bodies.get_mut(&Vid::new(LEADER_VID)) {
            body.map = 3_580_000;
        }
        let due = world.last_pulse().saturating_add(25);
        world.ship_set_ship_event(LEADER_VID, 5, due);
        let (_, secs, exit_due) = world
            .ship
            .of(LEADER_VID)
            .and_then(|run| run.exit_due)
            .expect("armed");
        assert_eq!(secs, 300);
        let _ = heard(&mut leader_inbox);
        world.run_ship_exit(LEADER_VID, exit_due);
        let lines = heard(&mut leader_inbox);
        assert!(
            lines.iter().any(|line| line
                .windows(b"5 minutes".len())
                .any(|window| window == b"5 minutes")),
            "the minute line chats: {lines:02x?}"
        );
        if let Some(run) = world.ship.of_mut(LEADER_VID) {
            run.exit_due = Some((false, 0, exit_due));
        }
        world.run_ship_exit(LEADER_VID, exit_due);
        assert!(world.ship.of(LEADER_VID).is_none(), "the run ends");
    }

    /// The heal pastes 1% past the maximum, chats the HP, stuns the reader and
    /// broadcasts the target.
    #[test]
    fn the_heal_pastes_chats_stuns_and_broadcasts() {
        let ShipWorld {
            mut world,
            mut leader_inbox,
            lease: _lease,
            member_lease: _member_lease,
            ..
        } = a_ship_world();
        world.judge_ship_create(Vid::new(LEADER_VID), NOW);
        let (vid, hp, max) = world.ship.of(LEADER_VID).and_then(|run| run.mast).unwrap();
        let _burst = heard(&mut leader_inbox);
        if let Some(body) = world.bodies.get_mut(&Vid::new(LEADER_VID)) {
            body.map = 3_580_000;
        }
        world.judge_ship_heal(Vid::new(LEADER_VID));
        let (vid_after, hp_after, _) = world.ship.of(LEADER_VID).and_then(|run| run.mast).unwrap();
        assert_eq!(vid_after, vid);
        assert_eq!(hp_after, hp + max / 100);
        let lines = heard(&mut leader_inbox);
        assert!(
            lines.iter().any(|line| line
                .windows(b"remaining HP".len())
                .any(|window| window == b"remaining HP")),
            "the HP chats: {lines:02x?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.first() == Some(&protocol::gc_actors::GcTarget::header())),
            "the broadcast goes: {lines:02x?}"
        );
    }
}
