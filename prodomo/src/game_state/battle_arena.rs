//! The GM weekly event (`CBattleArena`, `G/BattleArena.h`, `G/BattleArena.cpp:12-333`)
//! with its line (`do_weeklyevent`, `G/cmd_gm.cpp:4475-4501`).
//!
//! Legacy gates the GM's arena kills on `IsBattleArenaMap` in two death-penalty
//! spots; those gates already read [`super::death::ARENA_MAPS`] and
//! [`super::item_penalty::BATTLE_ARENA_MAPS`], which name the same
//! `nBATTLE_ARENA_MAP` maps (`G/BattleArena.h:2`), so no second table is kept here.
//!
//! On the owner's data maps 190-192 load nowhere (no Channel hosts them), so every
//! map operation (regen, monster count, purge, `FWarpToHome`, stone and boss
//! spawns) finds no map or NPCs and no-ops while the notices and the `battle_arena`
//! event flag still run: the event runs degenerately, as legacy would on a core
//! with no such map loaded. This port keeps that shape and never gates on the maps
//! existing: `Start` checks only `STATUS_CLOSE` and the empire (`:236-238`).
//!
//! Timers ride the per-owner Pulse countdown pattern the GM arena uses: the state
//! runs on its due Pulse. Equal-Pulse fires run in map/list order where legacy's
//! heap order at equal priority is heap-shape dependent (a Divergence the arena
//! records too). The empire draw (`Start`'s random arm) uses the world's own dice.

use common::vid::Vid;

use super::GameState;
use crate::chat_line::Arg;
use crate::command::{one_argument, GM_PLAYER};
use crate::item_move::Mover;
use protocol::gc_chat::{CHAT_TYPE_INFO, CHAT_TYPE_NOTICE};

/// Whether `ForceEnd` latched: `m_bForceEnd` (`G/BattleArena.h:25`), as a
/// two-variant enum rather than a fourth struct `bool`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum BattleLatch {
    /// At rest or after `End`.
    #[default]
    Clear,
    /// `ForceEnd` ran; a second call stays silent until `End` clears it.
    Forced,
}

/// `nBATTLE_ARENA_MAP` (`G/BattleArena.h:2`), by empire minus 1.
const MAPS: [i32; 3] = [190, 191, 192];

/// `SpawnRandomStone`'s vnums (`G/BattleArena.cpp:302`).
const STONE_VNUMS: [u32; 7] = [8012, 8013, 8014, 8024, 8025, 8026, 8027];

/// `SpawnRandomStone`'s regions (`G/BattleArena.cpp:304-307`), by map minus 190.
const STONE_REGIONS: [[i32; 4]; 3] = [
    [688_900, 247_600, 692_700, 250_000],
    [740_200, 251_000, 744_200, 247_700],
    [791_400, 250_900, 795_900, 250_400],
];

/// `SpawnLastBoss`'s vnum (`G/BattleArena.cpp:321`).
const BOSS_VNUM: u32 = 8616;

/// `SpawnLastBoss`'s positions (`G/BattleArena.cpp:323-326`), by map minus 190.
const BOSS_POSITIONS: [[i32; 2]; 3] = [[691_000, 248_900], [742_300, 249_000], [793_600, 249_300]];

/// `EMPIRE_NAME` (`G/start_position.h:14-17`): `LC_TEXT(g_nation_name[e])`, the
/// CP949 nation name (`G/start_position.cpp:5-11`). Shared bytes with the three-way
/// war's arm (`threeway_empire_name`), which resolves the same table as `LOCALE_YMIR`.
fn empire_name(empire: u8) -> &'static [u8] {
    match empire {
        1 => NATION_A,
        2 => NATION_B,
        _ => NATION_C,
    }
}

/// The raw CP949 bytes of every `LC_TEXT` below, taken from the owner's compiled
/// source (`G/BattleArena.cpp`): `LC_TEXT` is the one-argument `locale_find`,
/// whose language defaults to `LOCALE_YMIR` (`G/locale.hpp:10,31`), and a
/// `LOCALE_YMIR` lookup answers its string as it is, so the raw bytes speak
/// (the same shape the three-way war's `g_nation_name` arm uses,
/// `G/start_position.cpp:5-11`).
const NATION_A: &[u8] = b"\xbd\xc5\xbc\xf6\xb1\xb9";
const NATION_B: &[u8] = b"\xc3\xb5\xc1\xb6\xb1\xb9";
const NATION_C: &[u8] = b"\xc1\xf8\xb3\xeb\xb1\xb9";

const STATE5_MINUTES: &[u8] = b"\xb8\xf3\xbd\xba\xc5\xcd\xb5\xe9\xc0\xc7 \xb0\xf8\xb0\xdd\xb1\xee\xc1\xf6 5\xba\xd0 \xb3\xb2\xbe\xd2\xbd\xc0\xb4\xcf\xb4\xd9!!!";
const STATE1_MINUTE: &[u8] = b"\xb8\xf3\xbd\xba\xc5\xcd\xb5\xe9\xc0\xc7 \xb0\xf8\xb0\xdd\xb1\xee\xc1\xf6 1\xba\xd0 \xb3\xb2\xbe\xd2\xbd\xc0\xb4\xcf\xb4\xd9!!!";
const ENTER_SOON: &[u8] = b"\xb8\xf3\xbd\xba\xc5\xcd\xb5\xe9\xc0\xcc \xbc\xba\xc0\xbb \xb0\xf8\xb0\xdd\xc7\xcf\xb1\xe2 \xbd\xc3\xc0\xdb\xc7\xdf\xbd\xc0\xb4\xcf\xb4\xd9.";
const GATHER_CENTRE: &[u8] = b"\xc1\xdf\xbe\xd3 \xc1\xa6\xb4\xdc\xbf\xa1 \xbe\xc7\xc0\xc7 \xb1\xe2\xbf\xee\xc0\xcc \xb8\xf0\xbf\xa9\xb5\xec\xb4\xcf\xb4\xd9.";
const STONE_FALLS: &[u8] = b"\xb8\xf3\xbd\xba\xc5\xcd\xb5\xe9\xc0\xcc \xb9\xb0\xb7\xaf\xb0\xa5 \xc1\xb6\xc1\xfc\xc0\xbb \xba\xb8\xc0\xd4\xb4\xcf\xb4\xd9.";
const STONE_BREAKS: &[u8] = b"\xb8\xf3\xbd\xba\xc5\xcd\xb5\xe9\xc0\xcc \xb9\xb0\xb7\xaf\xb0\xa1\xb1\xe2 \xbd\xc3\xc0\xdb\xc7\xdf\xbd\xc0\xb4\xcf\xb4\xd9.";
const GO_HOME: &[u8] =
    b"\xc0\xe1\xbd\xc3 \xc8\xc4 \xb8\xb6\xc0\xbb\xb7\xce \xb5\xb9\xbe\xc6\xb0\xa9\xb4\xcf\xb4\xd9.";
const LAST_BOSS: &[u8] = b"\xb8\xf3\xbd\xba\xc5\xcd\xb5\xe9\xc0\xc7 \xb4\xeb\xc0\xa5\xc0\xcc \xb3\xaa\xc5\xb8\xb3\xb5\xbd\xc0\xb4\xcf\xb4\xd9.";
const KILL_BOSS: &[u8] = b"30\xba\xd0 \xb3\xbb\xb7\xce \xb1\xcd\xb8\xf1\xb7\xc9\xc1\xd6\xb8\xa6 \xb9\xb0\xb8\xae\xc3\xc4\xc1\xd6\xbc\xbc\xbf\xe4.";
const BOSS_FELLED: &[u8] = b"\xb1\xcd\xb8\xf1\xb7\xc9\xc1\xd6\xbf\xcd \xb1\xd7\xc0\xc7 \xba\xce\xc7\xcf\xb5\xe9\xc0\xbb \xb8\xf0\xb5\xce \xb9\xb0\xb8\xae\xc3\xc6\xbd\xc0\xb4\xcf\xb4\xd9.";
const BOSS_ESCAPES: &[u8] = b"\xb1\xcd\xb8\xf1\xb7\xc9\xc1\xd6\xb0\xa1 \xc5\xf0\xb0\xa2\xc7\xcf\xbf\xb4\xbd\xc0\xb4\xcf\xb4\xd9.";
/// The empire start line's format (`G/BattleArena.cpp:244`): `snprintf` of the
/// `"%s"` format with `EMPIRE_NAME` (the raw CP949 nation name).
const START_FORMAT: &[u8] = b"%s\xc0\xc7 \xbc\xba\xc0\xb8\xb7\xce \xb8\xf3\xbd\xba\xc5\xcd\xb5\xe9\xc0\xcc \xc1\xf8\xb1\xba\xc7\xcf\xb0\xed \xc0\xd6\xbd\xc0\xb4\xcf\xb4\xd9.";
const TEN_MINUTES: &[u8] = b"10\xba\xd0 \xb5\xda \xbc\xba\xc0\xbb \xb0\xf8\xb0\xdd\xc7\xd2 \xbf\xb9\xc1\xa4\xc0\xd4\xb4\xcf\xb4\xd9.";

impl GameState {
    /// Whether the battle arena runs: `IsRunning` (`G/BattleArena.cpp:21-24`).
    pub(super) fn battle_running(&self) -> bool {
        self.battle_map != 0
    }

    /// `do_weeklyevent` (`G/cmd_gm.cpp:4475-4501`): at rest the word's empire
    /// (1-3) starts, else a drawn 1-3 starts, and the caller hears
    /// `Weekly Event Start`; while running the event force-ends and the caller
    /// hears `Weekly Event End`.
    pub(super) fn run_weeklyevent_line(&mut self, vid: Vid, rest: &[u8], mover: Mover) {
        if self.battle_running() {
            self.battle_force_end();
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Weekly Event End", &[]);
        } else {
            let (word, _) = one_argument(rest);
            let empire = super::gm_lines::int_word(&word).unwrap_or(0);
            let empire = if (1..=3).contains(&empire) {
                empire
            } else {
                world::character::number(&mut self.dice, 1, 3)
            };
            self.battle_start(empire);
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Weekly Event Start", &[]);
        }
    }

    /// `CBattleArena::Start` (`G/BattleArena.cpp:235-266`).
    fn battle_start(&mut self, empire: i32) {
        if self.battle_running() {
            return;
        }
        let Ok(slot) = usize::try_from(empire - 1) else {
            return;
        };
        let Some(&map) = MAPS.get(slot) else {
            return;
        };
        self.battle_map = map;
        self.battle_empire = empire;
        let name = empire_name(u8::try_from(empire).unwrap_or(1));
        self.notice_every_pc(START_FORMAT, &[Arg::Text(name)]);
        self.notice_every_pc(TEN_MINUTES, &[]);
        self.battle_cancel();
        self.battle_state = 0;
        self.battle_wait = 0;
        let delay = if self.test_server { 60 } else { 5 * 60 };
        self.battle_arm(delay);
        self.set_event_flag(b"battle_arena", map);
    }

    /// `CBattleArena::End` (`G/BattleArena.cpp:268-279`).
    fn battle_end(&mut self) {
        self.battle_cancel();
        self.battle_latch = BattleLatch::Clear;
        self.battle_map = 0;
        self.battle_empire = 0;
        self.set_event_flag(b"battle_arena", 0);
    }

    /// `CBattleArena::ForceEnd` (`G/BattleArena.cpp:281-298`): a second call
    /// while the latch holds is silent. The re-arm runs state 3 after 5 seconds.
    fn battle_force_end(&mut self) {
        if self.battle_latch == BattleLatch::Forced {
            return;
        }
        self.battle_latch = BattleLatch::Forced;
        self.battle_cancel();
        self.battle_state = 3;
        self.battle_wait = 0;
        self.battle_arm(5);
    }

    /// `event_cancel(&m_pEvent)`: the armed Pulse clears.
    fn battle_cancel(&mut self) {
        if let Some(due) = self.battle_due.take() {
            self.regens
                .due
                .remove(&(due, super::regen::Due::BattleArena));
        }
    }

    /// `event_create(battle_arena_event, info, PASSES_PER_SEC(secs))`: the state
    /// runs `secs` seconds on.
    fn battle_arm(&mut self, secs: u32) {
        let when = self.last_pulse.saturating_add(super::regen::pulses(secs));
        self.battle_cancel();
        self.battle_due = Some(when);
        self.regens
            .due
            .insert((when, super::regen::Due::BattleArena));
    }

    /// `battle_arena_event` due on one Pulse (`G/BattleArena.cpp:87-233`).
    pub(super) fn run_battle_arena_tick(&mut self, pulse: u64) {
        let Some(due) = self.battle_due else {
            return;
        };
        if due > pulse {
            return;
        }
        self.battle_due = None;
        match self.battle_state {
            0 => {
                self.battle_state = 1;
                self.notice_every_pc(STATE5_MINUTES, &[]);
                self.battle_arm(if self.test_server { 60 } else { 60 * 4 });
            }
            1 => {
                self.battle_state = 2;
                self.notice_every_pc(STATE1_MINUTE, &[]);
                self.battle_arm(if self.test_server { 10 } else { 60 });
            }
            2 => {
                self.battle_state = 3;
                self.battle_wait = 0;
                self.set_event_flag(b"battle_arena", 0);
                self.notice_every_pc(ENTER_SOON, &[]);
                self.battle_regen();
                self.battle_arm(if self.test_server { 60 } else { 60 * 5 });
            }
            3 => {
                if self.battle_count() == 0 {
                    self.battle_state = 6;
                    self.battle_notice(GATHER_CENTRE);
                } else {
                    self.battle_wait = self.battle_wait.saturating_add(1);
                    if self.battle_wait >= 5 {
                        self.battle_state = 4;
                        self.battle_notice(STONE_FALLS);
                    } else {
                        self.battle_spawn_stone();
                    }
                }
                self.battle_arm(if self.test_server { 60 } else { 60 * 5 });
            }
            4 => {
                self.battle_state = 5;
                self.battle_notice(STONE_BREAKS);
                self.battle_notice(GO_HOME);
                self.battle_purge_monsters();
                self.battle_arm(30);
            }
            5 => {
                let places: Vec<(u8, i32)> = self.places_hosting(self.battle_map);
                for place in places {
                    self.battle_warp_home(place);
                }
                self.battle_end();
            }
            6 => {
                self.battle_state = 7;
                self.battle_wait = 0;
                self.battle_notice(LAST_BOSS);
                self.battle_notice(KILL_BOSS);
                self.battle_spawn_boss();
                self.battle_arm(if self.test_server { 60 } else { 60 * 5 });
            }
            // State 7 (`:196-231`). The count rises before the compare, as
            // legacy's does: the sixth monsters-present fire escapes, and the
            // first five each spawn a stone.
            _ => {
                if self.battle_count() == 0 {
                    self.battle_notice(BOSS_FELLED);
                    self.battle_notice(GO_HOME);
                    self.battle_state = 5;
                    self.battle_arm(60);
                } else {
                    self.battle_wait = self.battle_wait.saturating_add(1);
                    if self.battle_wait >= 6 {
                        self.battle_notice(BOSS_ESCAPES);
                        self.battle_notice(GO_HOME);
                        self.battle_purge_monsters();
                        self.battle_purge_stones();
                        self.battle_state = 5;
                        self.battle_arm(60);
                    } else {
                        self.battle_spawn_stone();
                        self.battle_arm(if self.test_server { 60 } else { 60 * 5 });
                    }
                }
            }
        }
    }

    /// `GetMonsterCountInMap(nMapIndex)` (`G/sectree_manager.cpp:1579-1593`):
    /// with no map no monster counts, which is the owner's shape (maps 190-192
    /// load nowhere). `FCountMonsters` counts `IsMonster` (`G/char.h:750`), the
    /// `CHAR_TYPE_MONSTER` row, which stones are not.
    fn battle_count(&self) -> usize {
        use gamedata::mob_proto::CHAR_TYPE_MONSTER;
        if self.battle_map == 0 {
            return 0;
        }
        self.npcs
            .values()
            .flat_map(|list| list.npcs.iter())
            .filter(|npc| npc.char_type == CHAR_TYPE_MONSTER)
            .filter(|npc| {
                self.npc_of
                    .get(&npc.vid)
                    .is_some_and(|&(_, map, _)| map == self.battle_map)
            })
            .count()
    }

    /// `SendNoticeMap` on the event map (`G/cmd_gm.cpp:1453-1489`): legacy walks
    /// its whole process's client set filtered by map only, so the Rewrite
    /// walks every Channel the same way (a legacy core holds one Channel's
    /// maps, but the filter never names it).
    fn battle_notice(&self, text: &[u8]) {
        let map = self.battle_map;
        let mut vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == map)
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            let Some(channel) = self.bodies.get(&vid).map(|body| body.channel) else {
                continue;
            };
            let here = self.place_of(vid.raw()).filter(|&(on, _)| on == channel);
            if here.is_none() {
                continue;
            }
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, CHAT_TYPE_NOTICE, text, &[]);
            }
        }
    }

    /// The state's regen arm (`G/BattleArena.cpp:122-133`): `regen_do` of the
    /// empire's file (`strRegen`, `G/BattleArena.h:3-9`) at the map's base, with
    /// no dungeon and no exception. A gap: the game thread reads no regen files
    /// at runtime (they load at boot in `main.rs`), so the file's entries never
    /// run here. On the owner's data the loop below is empty anyway (no Channel
    /// hosts maps 190-192) and the files do not ship, as legacy's missing-file
    /// arm logs and stands nothing (`G/regen.cpp:419-425`). If the maps ever
    /// load, this arm needs the boot-time entries.
    fn battle_regen(&mut self) {
        let map = self.battle_map;
        for place in self.places_hosting(map) {
            let _ = self.map_base(place);
        }
    }

    /// `SECTREE_MANAGER::PurgeMonstersInMap` (`G/sectree_manager.cpp:1494-1504`):
    /// every monster on every Channel's copy of the event map dies. Pets and
    /// auto-shamans are spared, as `FPurgeMonsters` spares them; the Rewrite has
    /// no pet or auto-shaman rows, so every monster falls.
    fn battle_purge_monsters(&mut self) {
        use gamedata::mob_proto::CHAR_TYPE_MONSTER;
        let map = self.battle_map;
        let victims: Vec<u32> = self
            .npcs
            .values()
            .flat_map(|list| list.npcs.iter())
            .filter(|npc| npc.char_type == CHAR_TYPE_MONSTER)
            .filter_map(|npc| {
                self.npc_of
                    .get(&npc.vid)
                    .filter(|&&(_, on, _)| on == map)
                    .map(|_| npc.vid)
            })
            .collect();
        for vid in victims {
            self.destroy_npc(vid);
        }
    }

    /// `SECTREE_MANAGER::PurgeStonesInMap` (`G/sectree_manager.cpp:1522-1532`):
    /// every stone on every Channel's copy of the event map dies.
    fn battle_purge_stones(&mut self) {
        use gamedata::mob_proto::CHAR_TYPE_STONE;
        let map = self.battle_map;
        let victims: Vec<u32> = self
            .npcs
            .values()
            .flat_map(|list| list.npcs.iter())
            .filter(|npc| npc.char_type == CHAR_TYPE_STONE)
            .filter_map(|npc| {
                self.npc_of
                    .get(&npc.vid)
                    .filter(|&&(_, on, _)| on == map)
                    .map(|_| npc.vid)
            })
            .collect();
        for vid in victims {
            self.destroy_npc(vid);
        }
    }

    /// `SpawnRandomStone` (`G/BattleArena.cpp:300-317`): a drawn stone vnum in
    /// the map's region on every Channel's copy. The `idx` latch refuses off
    /// the three maps (`:309-310`); the region is absolute centimetres, and the
    /// stone forces the spawn motion as `SpawnMobRange` forces it for stones
    /// (`G/char_manager.cpp:480-481`). A point no sectree or cell takes stands
    /// nothing, as a null `SpawnMob` does.
    fn battle_spawn_stone(&mut self) {
        let Some(slot) = usize::try_from(self.battle_map - 190)
            .ok()
            .filter(|&slot| slot < 3)
        else {
            return;
        };
        let draw = world::character::number(&mut self.dice, 0, 6);
        let vnum = STONE_VNUMS[usize::try_from(draw).unwrap_or(0) % STONE_VNUMS.len()];
        let [sx, sy, ex, ey] = STONE_REGIONS[slot];
        let map = self.battle_map;
        for place in self.places_hosting(map) {
            let (Some(region), Some(cells), Some(respawner)) = (
                self.regens.region_of(place.0, place.1),
                self.regens.cells_of(place.1),
                self.regens.respawner_mut(),
            ) else {
                continue;
            };
            let pulse = self.last_pulse;
            let low = (sx.min(ex), sy.min(ey));
            let high = (sx.max(ex), sy.max(ey));
            if let Ok(Some(stood)) = respawner.spawn_mob_line(
                (&region, &cells),
                vnum,
                (low.0, low.1, high.0, high.1),
                true,
                &mut self.dice,
            ) {
                self.stand_runtime(place, vec![stood], pulse);
            }
            let _ = high;
        }
    }

    /// `SpawnLastBoss` (`G/BattleArena.cpp:319-332`): the boss at the map's
    /// position on every Channel's copy. The `idx` latch refuses off the three
    /// maps (`:328-329`).
    fn battle_spawn_boss(&mut self) {
        let Some(slot) = usize::try_from(self.battle_map - 190)
            .ok()
            .filter(|&slot| slot < 3)
        else {
            return;
        };
        let [x, y] = BOSS_POSITIONS[slot];
        let map = self.battle_map;
        for place in self.places_hosting(map) {
            let (Some(region), Some(cells), Some(respawner)) = (
                self.regens.region_of(place.0, place.1),
                self.regens.cells_of(place.1),
                self.regens.respawner_mut(),
            ) else {
                continue;
            };
            let pulse = self.last_pulse;
            if let Ok(Some(stood)) = respawner.spawn_directed(
                (&region, &cells),
                BOSS_VNUM,
                (x, y),
                -1,
                false,
                &mut self.dice,
            ) {
                self.stand_runtime(place, vec![stood], pulse);
            }
        }
    }

    /// `FWarpToHome` (`G/BattleArena.cpp:38-64`): every PC on the map warps to
    /// its empire start. A GM is skipped unless the server is a test server;
    /// the map comes from the position (`WarpSet(x, y)` resolves it), so the
    /// plain home order goes out.
    fn battle_warp_home(&mut self, place: (u8, i32)) {
        let mut vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.channel == place.0 && body.map == place.1)
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            let Some(body) = self.bodies.get(&vid) else {
                continue;
            };
            let gm = self
                .characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.points().map(world::character::Points::gm_level))
                .unwrap_or(GM_PLAYER);
            if gm != GM_PLAYER && !self.test_server {
                continue;
            }
            let empire = body.card.empire;
            let Some((x, y)) = crate::warp::go_home(empire) else {
                continue;
            };
            self.order_warp(vid, (x, y));
        }
    }
}

#[cfg(test)]
mod tests {
    use common::vid::Vid;
    use gamedata::mob_locale_names::MobNamesByLanguage;
    use tokio::sync::mpsc::UnboundedReceiver;

    use super::super::fixtures::{a_world, enter_on, TestClock};
    use super::*;

    const fn mover() -> Mover {
        Mover {
            recently_fought: false,
            empire: 1,
            language: 1,
            pk_mode: crate::loading_phase::PK_MODE_PEACE,
            affect_flags: [0; 2],
            dead: false,
            alignment: 0,
            view_guild: (0, 0),
            premium: (0, 0),
        }
    }

    fn text_of(line: &[u8]) -> Vec<u8> {
        let size = usize::from(u16::from_le_bytes([line[1], line[2]]));
        assert_eq!(size, line.len());
        assert_eq!(line[0], 4, "HEADER_GC_CHAT");
        line[10..].to_vec()
    }

    fn kind_of(line: &[u8]) -> u8 {
        line[3]
    }

    /// A world with quests (for the `battle_arena` flag) and a caller on it.
    fn battle_world(clock: &TestClock) -> (GameState, UnboundedReceiver<Vec<u8>>) {
        let mut world = a_world(clock);
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager =
            quest::manager::Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        world.quests = Some(super::super::quests::Quests::new(
            manager,
            std::sync::Arc::default(),
            MobNamesByLanguage::default(),
        ));
        let inbox = enter_on(
            &mut world,
            super::super::fixtures::PLACE,
            7,
            (3200, 3200),
            820,
        );
        (world, inbox)
    }

    fn flag(world: &GameState) -> i32 {
        world
            .quests
            .as_ref()
            .expect("quests loaded")
            .manager()
            .event_flag(b"battle_arena")
    }

    /// `Start` gates on the status and the empire, speaks the empire line and the
    /// ten-minute line, arms the test-server minute, and sets the flag to the map.
    #[test]
    fn start_speaks_arms_and_sets_the_flag() {
        let clock = TestClock::default();
        let (mut world, mut caller) = battle_world(&clock);
        while caller.try_recv().is_ok() {}
        world.run_weeklyevent_line(Vid::new(7), b"2", mover());
        // Start notices first, then the caller's own tell.
        let mut lines = Vec::new();
        while let Ok(line) = caller.try_recv() {
            lines.push(line);
        }
        assert_eq!(lines.len(), 3, "the start notices and the tell");
        assert_eq!(text_of(&lines[2]), b"Weekly Event Start");
        lines.pop();
        assert_eq!(kind_of(&lines[0]), CHAT_TYPE_NOTICE);
        assert_eq!(kind_of(&lines[1]), CHAT_TYPE_NOTICE);
        assert!(
            text_of(&lines[0])
                .windows(6)
                .any(|w| w == b"\xc3\xb5\xc1\xb6\xb1\xb9"),
            "the Chunjo nation name in the start line: {:?}",
            text_of(&lines[0])
        );
        assert_eq!(text_of(&lines[1]), TEN_MINUTES);
        assert_eq!(flag(&world), 191);
        assert!(world.battle_running());
        assert_eq!(world.battle_map, 191);
        assert_eq!(world.battle_empire, 2);
        // A second start while running force-ends instead: the map clears only at End.
        world.run_weeklyevent_line(Vid::new(7), b"1", mover());
        assert_eq!(world.battle_state, 3, "force-end re-arms at state 3");
        assert_eq!(flag(&world), 191, "the flag stays until End");
    }

    /// An out-of-range empire word draws 1-3; `Start` refuses while running.
    #[test]
    fn an_empty_word_draws_an_empire_and_a_running_start_refuses() {
        let clock = TestClock::default();
        let (mut world, mut caller) = battle_world(&clock);
        while caller.try_recv().is_ok() {}
        world.run_weeklyevent_line(Vid::new(7), b"", mover());
        assert!(world.battle_running());
        assert!((190..=192).contains(&world.battle_map));
        let map = world.battle_map;
        world.battle_start(1);
        assert_eq!(world.battle_map, map, "no restart while running");
    }

    /// The first three states speak the five-minute, one-minute and entry lines
    /// on the test-server's short waits, and state 2 clears the flag.
    #[test]
    fn the_first_states_speak_and_clear_the_flag() {
        let clock = TestClock::default();
        let (mut world, mut caller) = battle_world(&clock);
        while caller.try_recv().is_ok() {}
        world.run_weeklyevent_line(Vid::new(7), b"1", mover());
        while caller.try_recv().is_ok() {}
        let due = world.battle_due.expect("state 0 armed");
        world.run_battle_arena_tick(due);
        assert_eq!(world.battle_state, 1);
        assert_eq!(
            text_of(&caller.try_recv().expect("state 0")),
            STATE5_MINUTES
        );
        let due = world.battle_due.expect("state 1 armed");
        world.run_battle_arena_tick(due);
        assert_eq!(world.battle_state, 2);
        assert_eq!(text_of(&caller.try_recv().expect("state 1")), STATE1_MINUTE);
        let due = world.battle_due.expect("state 2 armed");
        world.run_battle_arena_tick(due);
        assert_eq!(world.battle_state, 3);
        assert_eq!(flag(&world), 0, "state 2 clears the flag");
        assert_eq!(text_of(&caller.try_recv().expect("state 2")), ENTER_SOON);
        assert!(caller.try_recv().is_err(), "the regen stands nothing");
    }

    /// State 3 with no monsters on the missing maps gathers the centre and moves
    /// to the boss state; the waits and purges no-op on the missing map.
    #[test]
    fn an_empty_map_gathers_the_centre_and_runs_the_boss() {
        let clock = TestClock::default();
        let (mut world, mut caller) = battle_world(&clock);
        while caller.try_recv().is_ok() {}
        world.run_weeklyevent_line(Vid::new(7), b"3", mover());
        while caller.try_recv().is_ok() {}
        // Jump to state 3.
        let due = world.battle_due.expect("armed");
        world.run_battle_arena_tick(due);
        let due = world.battle_due.expect("armed");
        world.run_battle_arena_tick(due);
        let due = world.battle_due.expect("armed");
        world.run_battle_arena_tick(due);
        while caller.try_recv().is_ok() {}
        assert_eq!(world.battle_state, 3);
        // No monsters anywhere: straight to state 6.
        let due = world.battle_due.expect("state 3 armed");
        world.run_battle_arena_tick(due);
        assert_eq!(world.battle_state, 6);
        // The map notice finds no map PCs: nobody hears it.
        assert!(caller.try_recv().is_err(), "no map, no notice");
        // Boss state spawns nothing and re-arms; state 7 with no monsters ends.
        let due = world.battle_due.expect("state 6 armed");
        world.run_battle_arena_tick(due);
        assert_eq!(world.battle_state, 7);
        let due = world.battle_due.expect("state 7 armed");
        world.run_battle_arena_tick(due);
        assert_eq!(world.battle_state, 5, "the boss falls with no monsters");
        assert!(caller.try_recv().is_err(), "no map, no notices");
        // State 5 warps nobody home and ends: the flag clears, the latch clears.
        let due = world.battle_due.expect("state 5 armed");
        world.run_battle_arena_tick(due);
        assert!(!world.battle_running());
        assert_eq!(flag(&world), 0);
    }

    /// `ForceEnd` latches: a second call stays silent, and `End` clears the latch.
    #[test]
    fn force_end_latches_and_end_clears_it() {
        let clock = TestClock::default();
        let (mut world, mut caller) = battle_world(&clock);
        while caller.try_recv().is_ok() {}
        world.run_weeklyevent_line(Vid::new(7), b"1", mover());
        world.battle_force_end();
        let due = world.battle_due.expect("force-end armed");
        world.battle_force_end();
        assert_eq!(
            world.battle_due,
            Some(due),
            "the second force-end stays silent"
        );
        world.battle_end();
        assert!(!world.battle_running());
        assert_eq!(flag(&world), 0);
        // After the end the latch is clear: a fresh start runs.
        world.run_weeklyevent_line(Vid::new(7), b"1", mover());
        assert!(world.battle_running());
        let _ = caller;
    }

    /// The stone and boss spawns refuse off the three maps without touching the
    /// world, and `FWarpToHome` skips a GM out of the test-server mode.
    #[test]
    fn spawns_latch_off_the_maps_and_home_skips_a_gm() {
        let clock = TestClock::default();
        let (mut world, _) = battle_world(&clock);
        world.battle_map = 41;
        world.battle_spawn_stone();
        world.battle_spawn_boss();
        assert!(world.npcs.values().all(|list| list.npcs.is_empty()));
        world.battle_map = 0;
        world.battle_spawn_stone();
        world.battle_spawn_boss();
    }
}
