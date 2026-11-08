//! GM arena duels (`G/arena.cpp:1-589,951-1055`, `G/cmd_gm.cpp:3957-4060`).
//! Observer admission is unreachable on the owner's quests (ledger 469); legacy's
//! observer packet/chat senders are commented out. Timers use the same per-owner
//! Pulse countdown pattern as `timed_exit`. Equal-Pulse fires run in arena order,
//! ready before timeout, unlike legacy's heap-shape-dependent event order.
//! Timeout sends the empty opponent list to both players: legacy mistakenly sends
//! it twice to A (`G/arena.cpp:447-448`), a Defect, not a Quirk.

use super::GameState;
use crate::command::one_argument;
use crate::{chat_line::Arg, game_loop_messages::GmLine, item_move::Mover};
use common::vid::Vid;
use protocol::gc_chat::{CHAT_TYPE_INFO, CHAT_TYPE_NOTICE};
use protocol::gc_duel_start::GcDuelStart;

const TEN_SECONDS: u64 = 250;
type Slot = ((u8, u32), usize);

#[derive(Debug, Clone)]
pub(super) struct Arena {
    start: [(i32, i32); 2],
    pids: [u32; 2],
    return_channels: [u8; 2],
    set_count: u32,
    set_points: [u32; 2],
    /// `SetArena` is installed by ready state 0, not by `StartDuel`.
    bound: bool,
    ready: Option<(u64, u8)>,
    timeout: Option<(u64, u8)>,
}

impl Arena {
    fn new(config: &quest::host::Arena) -> Self {
        Self {
            start: [config.start_a, config.start_b],
            pids: [0; 2],
            return_channels: [1; 2],
            set_count: 0,
            set_points: [0; 2],
            bound: false,
            ready: None,
            timeout: None,
        }
    }
    fn clear(&mut self) {
        self.pids = [0; 2];
        self.return_channels = [1; 2];
        self.set_count = 0;
        self.set_points = [0; 2];
        self.bound = false;
        self.ready = None;
        self.timeout = None;
    }
}

impl GameState {
    pub(super) fn is_arena_map(&self, map: i32) -> bool {
        self.arenas
            .contains_key(&(0, u32::from_ne_bytes(map.to_ne_bytes())))
    }

    pub(super) fn in_arena(&self, vid: Vid) -> bool {
        self.arena_member(vid)
            .is_some_and(|slot| self.arenas[&slot.0][slot.1].bound)
    }

    pub(super) fn arena_event_flag(&self, name: &[u8]) -> i32 {
        self.quests
            .as_ref()
            .map_or(0, |quests| quests.manager().event_flag(name))
    }

    pub(super) fn arena_potion_count(&self, vid: Vid) -> i32 {
        self.arena_potions
            .get(&vid)
            .copied()
            .unwrap_or_else(|| self.arena_event_flag(b"arena_potion_limit_count"))
    }

    pub(super) fn set_arena_potion_count(&mut self, vid: Vid, count: i32) {
        self.arena_potions.insert(vid, count);
    }

    pub(super) fn consume_arena_potion(&mut self, vid: Vid) {
        let count = self.arena_potion_count(vid).wrapping_sub(1);
        self.set_arena_potion_count(vid, count);
    }

    /// Reachable login arms (`G/input_login.cpp:791-839`). No owner quest adds an
    /// observer; `RegisterObserverPtr` cannot create one (`G/arena.cpp:1094-1097`).
    pub(super) fn enter_arena(&mut self, vid: Vid) {
        let Some((_, map)) = self
            .place_of(vid.raw())
            .filter(|(_, map)| self.is_arena_map(*map))
        else {
            return;
        };
        if self
            .arena_member(vid)
            .is_some_and(|slot| slot.0 .1 == u32::from_ne_bytes(map.to_ne_bytes()))
        {
            self.arena_packet([Some(vid), None], false);
            if self.is_horse_riding(vid.raw()) {
                self.stop_riding(vid);
                self.horse_unsummon(vid, false);
            }
            self.arena_leave_party(vid, false);
        } else if self
            .points_of(vid.raw())
            .is_some_and(|points| points.gm_level() == crate::command::GM_PLAYER)
        {
            // `MEMBER_NO`: a GM_PLAYER is sent to its empire start (`:834-838`). Legacy's
            // `WarpSet` resolves the target's address through the P2P map table, which no
            // single core here holds. The character's own Channel serves the target in the
            // owner's deployment (every Channel hosts every empire start), so `None` names
            // it; otherwise the lowest Channel hosting the target is named, and a target
            // no Channel hosts refuses as a position no map serves.
            let body = self
                .bodies
                .get(&vid)
                .map(|body| (body.channel, body.card.empire));
            if let Some((channel, empire)) = body {
                if let Some(point) = crate::warp::go_home(empire) {
                    let index = self.recall.atlas().and_then(|atlas| {
                        atlas.index_at(point.0, point.1).filter(|&index| index != 0)
                    });
                    match index {
                        Some(index) if self.recall.hosts(channel, index) => {
                            self.order_warp(vid, point);
                        }
                        Some(index) => {
                            let mut hosts: Vec<u8> = (1..=common::config::SHARED_CHANNEL)
                                .filter(|&channel| self.recall.hosts(channel, index))
                                .collect();
                            hosts.sort_unstable();
                            if let Some(&channel) = hosts.first() {
                                self.arena_return_warp(vid, point, channel);
                            }
                        }
                        None => {}
                    }
                }
            }
        }
    }

    pub(super) fn load_arenas(&mut self, configs: &[quest::host::Arena]) {
        for config in configs {
            let map = u32::from_ne_bytes(config.map_index.to_ne_bytes());
            let slots = self.arenas.entry((0, map)).or_default();
            // CheckArea rejects only the exact ordered pair, not shared endpoints.
            let word = |value: i32| {
                let bytes = value.to_le_bytes();
                i32::from(u16::from_le_bytes([bytes[0], bytes[1]]))
            };
            let narrow = |(x, y): (i32, i32)| (word(x), word(y));
            let mut arena = Arena::new(config);
            arena.start = [narrow(config.start_a), narrow(config.start_b)];
            if !slots.iter().any(|old| old.start == arena.start) {
                slots.push(arena);
            }
        }
    }

    pub(super) fn run_arena_line(
        &mut self,
        vid: Vid,
        line: GmLine,
        rest: &[u8],
        mover: Mover,
    ) -> Option<super::gm_lines::GmRows> {
        match line {
            GmLine::ShowArenaList => {
                for ((_, map), arenas) in &self.arenas {
                    for arena in arenas {
                        self.tell(
                            vid,
                            mover,
                            CHAT_TYPE_INFO,
                            b"ArenaMapInfo Map: %d stA(%d, %d) stB(%d, %d)",
                            &[
                                Arg::Int(i64::from(*map)),
                                Arg::Int(arena.start[0].0.into()),
                                Arg::Int(arena.start[0].1.into()),
                                Arg::Int(arena.start[1].0.into()),
                                Arg::Int(arena.start[1].1.into()),
                            ],
                        );
                    }
                }
            }
            GmLine::ArenaDuel => self.arena_duel_line(vid, rest, mover),
            GmLine::EndArenaDuel => self.arena_end_line(vid, rest, mover),
            GmLine::EndAllArenaDuel => {
                let slots = self.arena_slots();
                for slot in slots {
                    self.end_arena(slot);
                }
            }
            _ => {}
        }
        None
    }

    fn arena_find_name(&self, name: &[u8]) -> Option<Vid> {
        self.bodies.iter().find_map(|(vid, body)| {
            body.card
                .name
                .as_bytes()
                .eq_ignore_ascii_case(name)
                .then_some(*vid)
        })
    }

    fn arena_duel_line(&mut self, vid: Vid, rest: &[u8], mover: Mover) {
        let (a, rest) = one_argument(rest);
        let (b, rest) = one_argument(rest);
        let (sets, rest) = one_argument(rest);
        let (minutes, _) = one_argument(rest);
        let (Some(a), Some(b)) = (self.arena_find_name(&a), self.arena_find_name(&b)) else {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"There are no sparrers.", &[]);
            return;
        };
        let (sets, minutes) = duel_numbers(&sets, &minutes);
        // Self-duels and already registered participants are an unsafe legacy defect:
        // two timer owners warp and reset the same character inconsistently.
        if a == b || [a, b].iter().any(|v| self.arena_member(*v).is_some()) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"There is a problem starting Dalian.",
                &[],
            );
            return;
        }
        for player in [a, b] {
            self.remove_good_affects(player);
            self.remove_bad_affect(player);
        }
        for player in [a, b] {
            self.arena_quit_party(player);
        }
        let slot = self
            .arena_slots()
            .into_iter()
            .find(|slot| self.arenas[&slot.0][slot.1].pids == [0; 2]);
        let success = if let Some(slot) = slot {
            self.start_arena(slot, [a, b], sets, minutes);
            true
        } else {
            false
        };
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            if success {
                b"Dalian started successfully."
            } else {
                b"There is a problem starting Dalian."
            },
            &[],
        );
    }

    fn arena_end_line(&mut self, vid: Vid, rest: &[u8], mover: Mover) {
        let (name, _) = one_argument(rest);
        let Some(player) = self.arena_find_name(&name) else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"This character does not exist.",
                &[],
            );
            return;
        };
        let slot = self.arena_member(player);
        if let Some(slot) = slot {
            self.end_arena(slot);
        }
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            if slot.is_some() {
                b"Dalian forced shutdown successful"
            } else {
                b"Dalian Force Shutdown Failed"
            },
            &[],
        );
    }

    fn arena_slots(&self) -> Vec<Slot> {
        self.arenas
            .iter()
            .flat_map(|(map, slots)| (0..slots.len()).map(move |i| (*map, i)))
            .collect()
    }

    fn arena_member(&self, vid: Vid) -> Option<Slot> {
        let pid = self.characters.find_by_vid(vid).ok()?.player_id();
        self.arenas.iter().find_map(|(map, slots)| {
            slots
                .iter()
                .position(|arena| arena.pids.contains(&pid))
                .map(|index| (*map, index))
        })
    }

    fn arena_players(&self, slot: Slot) -> [Option<Vid>; 2] {
        self.arenas[&slot.0][slot.1].pids.map(|pid| {
            self.characters
                .find_by_pid(pid)
                .ok()
                .map(world::character::Character::vid)
                .filter(|vid| self.bodies.contains_key(vid))
        })
    }

    /// `CArenaManager::CanAttack` (`G/arena.cpp:646-689`): only the registered
    /// opposing PIDs on the same arena map. Registration, not countdown state,
    /// controls this override of PK modes and the grade-one horse gate.
    pub(super) fn arena_can_attack(&self, attacker: Vid, victim: Vid) -> bool {
        attacker != victim && self.arena_combat_slot(attacker, victim).is_some()
    }

    fn arena_combat_slot(&self, a: Vid, b: Vid) -> Option<Slot> {
        let place = self.place_of(a.raw())?;
        if self.place_of(b.raw())? != place {
            return None;
        }
        let pids = [a, b].map(|vid| {
            self.characters
                .find_by_vid(vid)
                .map_or(0, world::character::Character::player_id)
        });
        if pids.contains(&0) {
            return None;
        }
        let key = (0, u32::from_ne_bytes(place.1.to_ne_bytes()));
        self.arenas
            .get(&key)?
            .iter()
            .position(|arena| arena.pids.contains(&pids[0]) && arena.pids.contains(&pids[1]))
            .map(|at| (key, at))
    }

    /// `OnDead`'s map/list lookup (`G/arena.cpp:692-723`). The character's regular
    /// death still runs; the true result suppresses item drop and its dead event.
    pub(super) fn arena_on_dead(&mut self, killer: Vid, victim: Vid) -> bool {
        let Some(slot) = self.arena_combat_slot(killer, victim) else {
            return false;
        };
        let pid = self
            .characters
            .find_by_vid(killer)
            .expect("online killer")
            .player_id();
        self.score_arena(slot, pid);
        true
    }

    /// `CArena::OnDead` (`G/arena.cpp:726-864`); observers remain unreachable.
    fn score_arena(&mut self, slot: Slot, killer_pid: u32) {
        let players = self.arena_players(slot);
        let restart = if players.iter().all(Option::is_some) {
            let restart = self.score_arena_present(slot, players, killer_pid);
            let count = self.arena_event_flag(b"arena_potion_limit_count");
            for vid in players.into_iter().flatten() {
                self.set_arena_potion_count(vid, count);
            }
            restart
        } else {
            self.arena_tell(players, CHAT_TYPE_NOTICE, OPPONENT_PROBLEM, &[]);
            false
        };
        self.arena_tell(
            players,
            CHAT_TYPE_INFO,
            if restart { NEXT_ROUND } else { RETURN_TOWN },
            &[],
        );
        self.arenas.get_mut(&slot.0).expect("arena map")[slot.1].ready =
            Some((self.last_pulse + TEN_SECONDS, if restart { 3 } else { 2 }));
    }

    fn score_arena_present(&mut self, slot: Slot, players: [Option<Vid>; 2], killer: u32) -> bool {
        let arena = &mut self.arenas.get_mut(&slot.0).expect("arena map")[slot.1];
        let Some(side) = arena.pids.iter().position(|pid| *pid == killer) else {
            return false;
        };
        arena.set_points[side] = arena.set_points[side].wrapping_add(1);
        let points = arena.set_points;
        let restart = points[side] < arena.set_count;
        let names = players.map(|vid| {
            self.bodies[&vid.expect("present duelist")]
                .card
                .name
                .as_bytes()
        });
        self.arena_tell(
            players,
            if restart {
                CHAT_TYPE_INFO
            } else {
                CHAT_TYPE_NOTICE
            },
            if restart { ROUND_WIN } else { DUEL_WIN },
            &[Arg::Text(names[side])],
        );
        if restart {
            self.arena_tell(
                players,
                CHAT_TYPE_NOTICE,
                b"%s %d : %d %s",
                &[
                    Arg::Text(names[0]),
                    Arg::Int(points[0].into()),
                    Arg::Int(points[1].into()),
                    Arg::Text(names[1]),
                ],
            );
        }
        restart
    }

    pub(super) fn start_arena(&mut self, slot: Slot, players: [Vid; 2], sets: u32, minutes: u64) {
        let pids = players.map(|vid| {
            self.characters
                .find_by_vid(vid)
                .expect("online duelist")
                .player_id()
        });
        let channels = players.map(|vid| {
            self.bodies
                .get(&vid)
                .map_or(1, |body| if body.channel == 99 { 1 } else { body.channel })
        });
        let arena = &mut self.arenas.get_mut(&slot.0).expect("arena map")[slot.1];
        arena.pids = pids;
        arena.return_channels = channels;
        arena.set_count = sets;
        arena.ready = Some((self.last_pulse + TEN_SECONDS, 0));
        arena.timeout = Some((self.last_pulse + minutes * 60 * 25, 0));
        let start = arena.start;
        for (vid, (x, y)) in players.into_iter().zip(start) {
            self.order_warp(vid, (x * 100, y * 100));
            self.arena_refill(vid);
        }
    }

    fn arena_refill(&mut self, vid: Vid) {
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        let hp = points.max_hp() - points.hp();
        let sp = points.max_sp() - points.sp();
        self.change_pc_point(vid, common::point_slot::POINT_HP, hp, false);
        self.change_pc_point(vid, common::point_slot::POINT_SP, sp, false);
    }

    fn arena_tell(&self, players: [Option<Vid>; 2], kind: u8, key: &[u8], args: &[Arg<'_>]) {
        for vid in players.into_iter().flatten() {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, kind, key, args);
            }
        }
    }

    fn arena_packet(&self, players: [Option<Vid>; 2], start: bool) {
        for (i, vid) in players.iter().enumerate() {
            if let Some(vid) = vid {
                let opponents = if start {
                    players[1 - i].map(|v| vec![v.raw()]).unwrap_or_default()
                } else {
                    Vec::new()
                };
                let mut bytes = Vec::new();
                GcDuelStart::new(opponents)
                    .encode_into(&mut bytes)
                    .expect("at most one opponent");
                let _sent = self.write_to_client(*vid, bytes);
            }
        }
    }

    fn end_arena(&mut self, slot: Slot) {
        let players = self.arena_players(slot);
        let return_channels = self.arenas[&slot.0][slot.1].return_channels;
        self.arenas.get_mut(&slot.0).expect("arena map")[slot.1].clear();
        for (vid, channel) in players.into_iter().zip(return_channels) {
            let Some(vid) = vid else { continue };
            let changed = self
                .bodies
                .get(&vid)
                .is_some_and(|body| body.card.pk_mode != crate::loading_phase::PK_MODE_PEACE);
            if let Some(body) = self.bodies.get_mut(&vid) {
                body.card.pk_mode = crate::loading_phase::PK_MODE_PEACE;
            }
            if changed {
                let flags = self.affect_flags_of(vid);
                self.send_affect_update(vid, flags);
            }
            self.stand(vid);
            self.start_pc_recovery(vid);
            self.arena_refill(vid);
            if let Some(body) = self.bodies.get(&vid) {
                let empire = usize::from(body.card.empire);
                if let Some(point) = ARENA_RETURN.get(empire) {
                    self.arena_return_warp(vid, *point, channel);
                }
            }
        }
    }

    fn arena_return_warp(&self, vid: Vid, (x, y): (i32, i32), channel: u8) {
        let Some(body) = self.bodies.get(&vid) else {
            return;
        };
        let Some(clients) = self.clients.as_ref() else {
            return;
        };
        if let Some((id, _)) = clients
            .members_on_map(body.channel, body.map)
            .into_iter()
            .find(|(_, entry)| entry.vid == vid.raw())
        {
            let _ordered = clients.order(
                body.channel,
                id,
                crate::client_registry::ClientOrder::Warp {
                    x,
                    y,
                    private_map_index: 0,
                    channel: Some(channel),
                    exit: None,
                    sorted_until: self.sort_lock(vid),
                },
            );
        }
    }

    pub(super) fn arena_disconnect(&mut self, vid: Vid) {
        if let Some(slot) = self.arena_member(vid) {
            let other = self
                .arena_players(slot)
                .map(|player| player.filter(|v| *v != vid));
            self.arena_tell(other, CHAT_TYPE_INFO, DISCONNECTED, &[]);
            self.end_arena(slot);
        }
    }

    pub(super) fn run_arena_ticks(&mut self, pulse: u64) {
        let mut previous = None;
        loop {
            let next = match previous {
                None => self.arenas.keys().next().copied(),
                Some(key) => self
                    .arenas
                    .range((std::ops::Bound::Excluded(key), std::ops::Bound::Unbounded))
                    .next()
                    .map(|(key, _)| *key),
            };
            let Some(key) = next else { break };
            previous = Some(key);
            for at in 0..self.arenas[&key].len() {
                let slot = (key, at);
                let ready = self.arenas[&key][at].ready;
                if let Some((_, state)) = ready.filter(|(due, _)| *due <= pulse) {
                    self.arenas.get_mut(&key).expect("arena map")[at].ready = None;
                    self.fire_arena_ready(slot, state, pulse);
                }
                let timeout = self.arenas[&key][at].timeout;
                if let Some((_, state)) = timeout.filter(|(due, _)| *due <= pulse) {
                    self.arenas.get_mut(&key).expect("arena map")[at].timeout = None;
                    self.fire_arena_timeout(slot, state, pulse);
                }
            }
        }
    }

    fn arena_present(&mut self, slot: Slot) -> Option<[Option<Vid>; 2]> {
        let players = self.arena_players(slot);
        if players.iter().any(Option::is_none) {
            self.arena_tell(players, CHAT_TYPE_INFO, MISSING, &[]);
            self.end_arena(slot);
            None
        } else {
            Some(players)
        }
    }

    fn fire_arena_ready(&mut self, slot: Slot, state: u8, pulse: u64) {
        let Some(players) = self.arena_present(slot) else {
            return;
        };
        match state {
            0 => {
                self.arenas.get_mut(&slot.0).expect("arena map")[slot.1].bound = true;
                let count = self
                    .quests
                    .as_ref()
                    .map_or(0, |q| q.manager().event_flag(b"arena_potion_limit_count"));
                if count > 10000 {
                    self.arena_tell(players, CHAT_TYPE_INFO, UNLIMITED, &[]);
                } else {
                    for vid in players.into_iter().flatten() {
                        self.set_arena_potion_count(vid, count);
                    }
                    self.arena_tell(players, CHAT_TYPE_INFO, POTIONS, &[Arg::Int(count.into())]);
                }
                self.arena_tell(players, CHAT_TYPE_INFO, COUNTDOWN, &[]);
                self.arenas.get_mut(&slot.0).expect("arena map")[slot.1].ready =
                    Some((pulse + TEN_SECONDS, 1));
            }
            1 => {
                self.arena_tell(players, CHAT_TYPE_INFO, BEGUN, &[]);
                self.arena_packet(players, true);
            }
            2 => self.end_arena(slot),
            3 => {
                let start = self.arenas[&slot.0][slot.1].start;
                for (vid, (x, y)) in players.into_iter().flatten().zip(start) {
                    let _shown = self.show_body(vid.raw(), (x * 100, y * 100, 0), None);
                    self.write_phase(vid, crate::loading_phase::PHASE_GAME);
                    self.stand(vid);
                    self.start_pc_recovery(vid);
                    self.arena_refill(vid);
                    self.restart_at_same_pos(vid);
                }
                self.arena_packet(players, true);
                self.arena_tell(players, CHAT_TYPE_INFO, BEGUN, &[]);
            }
            _ => {
                self.arena_tell(players, CHAT_TYPE_INFO, BROKEN, &[]);
                self.end_arena(slot);
            }
        }
    }

    fn fire_arena_timeout(&mut self, slot: Slot, state: u8, pulse: u64) {
        let Some(players) = self.arena_present(slot) else {
            return;
        };
        match state {
            0 => {
                self.arena_tell(players, CHAT_TYPE_NOTICE, TIMEOUT, &[]);
                self.arena_tell(players, CHAT_TYPE_NOTICE, RETURN, &[]);
                self.arena_packet(players, false);
                self.arenas.get_mut(&slot.0).expect("arena map")[slot.1].timeout =
                    Some((pulse + TEN_SECONDS, 1));
            }
            1 => self.end_arena(slot),
            _ => {}
        }
    }
}

fn duel_numbers(sets: &[u8], minutes: &[u8]) -> (u32, u64) {
    let sets = super::gm_lines::int_word(sets).unwrap_or(0);
    let sets = if sets < 0 { 1 } else { sets.min(5) };
    let minutes = super::gm_lines::int_word(minutes).unwrap_or(5).max(5);
    (
        u32::try_from(sets).expect("nonnegative sets"),
        u64::try_from(minutes).expect("positive minutes"),
    )
}

const ARENA_RETURN: [(i32, i32); 4] = [
    (0, 0),
    (347_600, 882_700),
    (138_600, 236_600),
    (857_200, 251_800),
];
const MISSING: &[u8] = b"\xb4\xeb\xb7\xc3\x20\xbb\xf3\xb4\xeb\xb0\xa1\x20\xbb\xe7\xb6\xf3\xc1\xae\x20\xb4\xeb\xb7\xc3\xc0\xbb\x20\xc1\xbe\xb7\xe1\xc7\xd5\xb4\xcf\xb4\xd9\x2e";
const UNLIMITED: &[u8] =
    b"\xb9\xb0\xbe\xe0\x20\xc1\xa6\xc7\xd1\xc0\xcc\x20\xbe\xf8\xbd\xc0\xb4\xcf\xb4\xd9\x2e";
const POTIONS: &[u8] = b"\xb9\xb0\xbe\xe0\xc0\xbb\x20\x25\x64\x20\xb0\xb3\x20\xb1\xee\xc1\xf6\x20\xbb\xe7\xbf\xeb\x20\xb0\xa1\xb4\xc9\xc7\xd5\xb4\xcf\xb4\xd9\x2e";
const COUNTDOWN: &[u8] = b"\x31\x30\xc3\xca\xb5\xda\x20\xb4\xeb\xb7\xc3\xc0\xcc\x20\xbd\xc3\xc0\xdb\xb5\xcb\xb4\xcf\xb4\xd9\x2e";
const BEGUN: &[u8] =
    b"\xb4\xeb\xb7\xc3\xc0\xcc\x20\xbd\xc3\xc0\xdb\xb5\xc7\xbe\xfa\xbd\xc0\xb4\xcf\xb4\xd9\x2e";
const BROKEN: &[u8] = b"\xb4\xeb\xb7\xc3\xc0\xe5\x20\xb9\xae\xc1\xa6\xb7\xce\x20\xc0\xce\xc7\xcf\xbf\xa9\x20\xb4\xeb\xb7\xc3\xc0\xbb\x20\xc1\xbe\xb7\xe1\xc7\xd5\xb4\xcf\xb4\xd9\x2e";
const TIMEOUT: &[u8] = b"\xb4\xeb\xb7\xc3\x20\xbd\xc3\xb0\xa3\x20\xc3\xca\xb0\xfa\xb7\xce\x20\xb4\xeb\xb7\xc3\xc0\xbb\x20\xc1\xdf\xb4\xdc\xc7\xd5\xb4\xcf\xb4\xd9\x2e";
const RETURN: &[u8] = b"\x31\x30\xc3\xca\xb5\xda\x20\xb8\xb6\xc0\xbb\xb7\xce\x20\xc0\xcc\xb5\xbf\xc7\xd5\xb4\xcf\xb4\xd9\x2e";
const DISCONNECTED: &[u8] = b"\xbb\xf3\xb4\xeb\xb9\xe6\x20\xc4\xb3\xb8\xaf\xc5\xcd\xb0\xa1\x20\xc1\xa2\xbc\xd3\xc0\xbb\x20\xc1\xbe\xb7\xe1\xc7\xcf\xbf\xa9\x20\xb4\xeb\xb7\xc3\xc0\xbb\x20\xc1\xdf\xc1\xf6\xc7\xd5\xb4\xcf\xb4\xd9\x2e";

const OPPONENT_PROBLEM: &[u8] = b"\xbb\xf3\xb4\xeb\xb9\xe6\x20\xc4\xb3\xb8\xaf\xc5\xcd\xc0\xc7\x20\xb9\xae\xc1\xa6\xb7\xce\x20\xc0\xce\xc7\xcf\xbf\xa9\x20\xb4\xeb\xb7\xc3\xc0\xbb\x20\xc1\xbe\xb7\xe1\xc7\xd5\xb4\xcf\xb4\xd9\x2e";
const ROUND_WIN: &[u8] =
    b"\x25\x73\x20\xb4\xd4\xc0\xcc\x20\xbd\xc2\xb8\xae\xc7\xcf\xbf\xb4\xbd\xc0\xb4\xcf\xb4\xd9\x2e";
const DUEL_WIN: &[u8] = b"\x25\x73\x20\xb4\xd4\xc0\xcc\x20\xb4\xeb\xb7\xc3\xbf\xa1\xbc\xad\x20\xbd\xc2\xb8\xae\xc7\xcf\xbf\xb4\xbd\xc0\xb4\xcf\xb4\xd9\x2e";
const NEXT_ROUND: &[u8] = b"\x31\x30\xc3\xca\xb5\xda\x20\xb4\xd9\xc0\xbd\x20\xc6\xc7\xc0\xbb\x20\xbd\xc3\xc0\xdb\xc7\xd5\xb4\xcf\xb4\xd9\x2e";
const RETURN_TOWN: &[u8] = b"\x31\x30\xc3\xca\xb5\xda\x20\xb8\xb6\xc0\xbb\xb7\xce\x20\xb5\xc7\xb5\xb9\xbe\xc6\xb0\xa9\xb4\xcf\xb4\xd9\x2e";
#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::fixtures::{a_world, enter, TestClock};

    fn config(map: i32, a: (i32, i32), b: (i32, i32)) -> quest::host::Arena {
        quest::host::Arena {
            map_index: map,
            start_a: a,
            start_b: b,
        }
    }

    #[test]
    fn arena_table_preserves_map_order_pair_order_and_exact_duplicate_refusal() {
        let mut world = GameState::default();
        world.load_arenas(&[
            config(0x1234, (0x2345, 0x3456), (0x4567, 0x5678)),
            config(2, (3, 4), (5, 6)),
            config(2, (3, 4), (5, 6)),
            config(2, (5, 6), (3, 4)),
        ]);
        assert_eq!(
            world.arena_slots(),
            vec![((0, 2), 0), ((0, 2), 1), ((0, 0x1234), 0)]
        );
        assert_eq!(
            world.arenas[&(0, 0x1234)][0].start,
            [(0x2345, 0x3456), (0x4567, 0x5678)]
        );
    }

    #[test]
    fn arena_duel_argument_defaults_and_bounds_match_legacy() {
        assert_eq!(duel_numbers(b"", b""), (0, 5));
        assert_eq!(duel_numbers(b"-7", b"2"), (1, 5));
        assert_eq!(duel_numbers(b"9", b"291"), (5, 291));
        assert_eq!(duel_numbers(b"3tail", b"6tail"), (3, 6));
    }

    fn active() -> (
        GameState,
        [tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>; 2],
    ) {
        let mut world = a_world(&TestClock::default());
        let mut a = enter(&mut world, 0x1234, (3200, 3200), 820);
        let mut b = enter(&mut world, 0x2345, (3400, 3200), 820);
        world.load_arenas(&[config(41, (32, 32), (34, 32))]);
        world.start_arena(((0, 41), 0), [Vid::new(0x1234), Vid::new(0x2345)], 3, 5);
        while a.try_recv().is_ok() {}
        while b.try_recv().is_ok() {}
        (world, [a, b])
    }

    fn packets(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        let mut packets = Vec::new();
        while let Ok(packet) = inbox.try_recv() {
            if packet.first() == Some(&40) {
                packets.push(packet);
            }
        }
        packets
    }

    #[test]
    fn arena_ready_countdown_sends_opponent_golden_vids_and_stores_sets() {
        let (mut world, mut inboxes) = active();
        let slot = ((0, 41), 0);
        assert_eq!(world.arenas[&slot.0][0].set_count, 3);
        assert_eq!(world.arenas[&slot.0][0].set_points, [0, 0]);
        assert!(!world.arenas[&slot.0][0].bound);
        world.run_arena_ticks(249);
        assert_eq!(world.arenas[&slot.0][0].ready, Some((250, 0)));
        world.run_arena_ticks(250);
        assert!(world.arenas[&slot.0][0].bound);
        assert_eq!(world.arenas[&slot.0][0].ready, Some((500, 1)));
        world.run_arena_ticks(500);
        assert_eq!(
            packets(&mut inboxes[0]),
            vec![vec![40, 7, 0, 0x45, 0x23, 0, 0]]
        );
        assert_eq!(
            packets(&mut inboxes[1]),
            vec![vec![40, 7, 0, 0x34, 0x12, 0, 0]]
        );
        assert_eq!(world.arenas[&slot.0][0].ready, None);
    }

    #[test]
    fn arena_timeout_clears_both_opponents_then_ends_after_ten_seconds() {
        let (mut world, mut inboxes) = active();
        world.run_arena_ticks(250);
        world.run_arena_ticks(500);
        for inbox in &mut inboxes {
            let _ = packets(inbox);
        }
        world.run_arena_ticks(7500);
        for inbox in &mut inboxes {
            assert_eq!(packets(inbox), vec![vec![40, 3, 0]]);
        }
        assert_eq!(world.arenas[&(0, 41)][0].timeout, Some((7750, 1)));
        world.run_arena_ticks(7749);
        assert_ne!(world.arenas[&(0, 41)][0].pids, [0, 0]);
        world.run_arena_ticks(7750);
        assert_eq!(world.arenas[&(0, 41)][0].pids, [0, 0]);
        assert_eq!(world.arenas[&(0, 41)][0].timeout, None);
        assert!(!world.arenas[&(0, 41)][0].bound);
        assert_eq!(world.arenas[&(0, 41)][0].set_points, [0, 0]);
    }

    #[test]
    fn arena_self_and_duplicate_members_are_refused_without_affect_side_effects() {
        let (mut world, _inboxes) = active();
        let a = Vid::new(0x1234);
        let mover = world.own_mover(a).unwrap();
        world.arena_duel_line(a, b"P4660 P4660 3 5", mover);
        world.arena_duel_line(a, b"P4660 P9029 3 5", mover);
        assert_eq!(world.arenas[&(0, 41)][0].pids, [0x1234, 0x2345]);
        assert_eq!(world.arenas[&(0, 41)][0].ready, Some((250, 0)));
        assert_eq!(
            ARENA_RETURN,
            [
                (0, 0),
                (347_600, 882_700),
                (138_600, 236_600),
                (857_200, 251_800)
            ]
        );
    }

    #[test]
    fn arena_restart_end_disconnect_and_missing_player_clear_events() {
        let (mut world, mut inboxes) = active();
        world.fire_arena_ready(((0, 41), 0), 3, 1);
        assert_eq!(
            packets(&mut inboxes[0]),
            vec![vec![40, 7, 0, 0x45, 0x23, 0, 0]]
        );
        world.fire_arena_ready(((0, 41), 0), 2, 1);
        assert_eq!(world.arenas[&(0, 41)][0].pids, [0, 0]);
        world.start_arena(((0, 41), 0), [Vid::new(0x1234), Vid::new(0x2345)], 5, 5);
        world.arena_disconnect(Vid::new(0x2345));
        assert_eq!(world.arenas[&(0, 41)][0].set_count, 0);
        world.start_arena(((0, 41), 0), [Vid::new(0x1234), Vid::new(0x2345)], 5, 5);
        world.bodies.remove(&Vid::new(0x2345));
        world.run_arena_ticks(250);
        assert_eq!(world.arenas[&(0, 41)][0].ready, None);
        assert_eq!(world.arenas[&(0, 41)][0].timeout, None);
    }
    fn chats(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<(u8, Vec<u8>)> {
        let mut lines = Vec::new();
        while let Ok(bytes) = inbox.try_recv() {
            if bytes.first() == Some(&4) {
                assert_eq!(
                    usize::from(u16::from_le_bytes([bytes[1], bytes[2]])),
                    bytes.len()
                );
                lines.push((bytes[3], bytes[10..].to_vec()));
            }
        }
        lines
    }

    fn english(world: &mut GameState) {
        let country = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe/country");
        world.locale =
            std::sync::Arc::new(gamedata::locale_string::LocaleStrings::load(&country).unwrap());
        for body in world.bodies.values_mut() {
            body.card.language = 1;
        }
    }

    #[test]
    fn arena_attackability_truth_table_ignores_countdown_and_pk_policy() {
        let (mut world, _inboxes) = active();
        let a = Vid::new(0x1234);
        let b = Vid::new(0x2345);
        let _third = enter(&mut world, 0x3456, (3300, 3200), 820);
        assert!(world.arena_can_attack(a, b));
        assert!(world.arena_can_attack(b, a));
        assert!(!world.arena_can_attack(a, a));
        assert!(!world.arena_can_attack(a, Vid::new(0x3456)));
        assert!(!world.arena_can_attack(a, Vid::new(0x4567)));
        world.bodies.get_mut(&b).unwrap().channel = 2;
        assert!(!world.arena_can_attack(a, b));
        world.bodies.get_mut(&b).unwrap().channel = 1;
        world.bodies.get_mut(&b).unwrap().map = 42;
        assert!(!world.arena_can_attack(a, b));
        world.bodies.get_mut(&a).unwrap().map = 42;
        assert!(!world.arena_can_attack(a, b));
        world.bodies.get_mut(&a).unwrap().map = 41;
        world.bodies.get_mut(&b).unwrap().map = 41;
        world.arenas.get_mut(&(0, 41)).unwrap()[0].ready = Some((250, 3));
        assert!(world.arena_can_attack(a, b));
        world.end_arena(((0, 41), 0));
        assert!(!world.arena_can_attack(a, b));
    }

    #[test]
    fn arena_death_lookup_refuses_outsiders_and_other_maps_but_keeps_self_membership() {
        let (mut world, _inboxes) = active();
        let a = Vid::new(0x1234);
        let b = Vid::new(0x2345);
        let _other = enter(&mut world, 0x3456, (3300, 3200), 820);
        assert!(!world.arena_on_dead(Vid::new(0x3456), a));
        assert!(!world.arena_on_dead(Vid::new(0x4567), a));
        world.bodies.get_mut(&b).unwrap().map = 42;
        assert!(!world.arena_on_dead(a, b));
        world.bodies.get_mut(&b).unwrap().map = 41;
        assert!(world.arena_on_dead(a, a));
        assert_eq!(world.arenas[&(0, 41)][0].set_points, [1, 0]);
    }

    #[test]
    fn arena_each_side_scores_rounds_and_wins_with_golden_lines() {
        for side in 0..2 {
            let (mut world, mut inboxes) = active();
            english(&mut world);
            world.arenas.get_mut(&(0, 41)).unwrap()[0].set_count = 2;
            let players = [Vid::new(0x1234), Vid::new(0x2345)];
            assert!(world.arena_on_dead(players[side], players[1 - side]));
            let winner = if side == 0 { "P4660" } else { "P9029" };
            let score = if side == 0 {
                b"P4660 1 : 0 P9029".as_slice()
            } else {
                b"P4660 0 : 1 P9029".as_slice()
            };
            for inbox in &mut inboxes {
                assert_eq!(
                    chats(inbox),
                    vec![
                        (1, format!("{winner} has won.").into_bytes()),
                        (2, score.to_vec()),
                        (1, b"The next round will begin in 10 seconds.".to_vec())
                    ]
                );
            }
            assert_eq!(world.arenas[&(0, 41)][0].ready, Some((250, 3)));
            assert!(world.arena_on_dead(players[side], players[1 - side]));
            for inbox in &mut inboxes {
                assert_eq!(
                    chats(inbox),
                    vec![
                        (2, format!("{winner} has won the duel.").into_bytes()),
                        (
                            1,
                            b"You will be teleported into the city in 10 seconds.".to_vec()
                        )
                    ]
                );
            }
            assert_eq!(world.arenas[&(0, 41)][0].ready, Some((250, 2)));
            assert_eq!(world.arenas[&(0, 41)][0].timeout, Some((7500, 0)));
        }
    }

    #[test]
    fn arena_zero_sets_finishes_first_kill_and_foreign_killer_finishes_without_score() {
        for (sets, killer) in [(0, 0x1234), (3, 0x4567)] {
            let (mut world, mut inboxes) = active();
            english(&mut world);
            world.arenas.get_mut(&(0, 41)).unwrap()[0].set_count = sets;
            world.score_arena(((0, 41), 0), killer);
            for inbox in &mut inboxes {
                let lines = chats(inbox);
                assert_eq!(
                    lines.last().unwrap(),
                    &(
                        1,
                        b"You will be teleported into the city in 10 seconds.".to_vec()
                    )
                );
                assert_eq!(lines.len(), if sets == 0 { 2 } else { 1 });
            }
            assert_eq!(
                world.arenas[&(0, 41)][0].set_points,
                if sets == 0 { [1, 0] } else { [0, 0] }
            );
            assert_eq!(world.arenas[&(0, 41)][0].ready, Some((250, 2)));
        }
    }

    #[test]
    fn arena_missing_duelist_branches_notice_survivors_and_schedule_return() {
        for missing in [vec![0], vec![1], vec![0, 1]] {
            let (mut world, mut inboxes) = active();
            english(&mut world);
            let ids = [Vid::new(0x1234), Vid::new(0x2345)];
            for side in &missing {
                world.bodies.remove(&ids[*side]);
            }
            world.score_arena(((0, 41), 0), 0x1234);
            for (side, inbox) in inboxes.iter_mut().enumerate() {
                let expected = if missing.contains(&side) {
                    Vec::new()
                } else {
                    vec![
                        (
                            2,
                            b"Your combatant has got some problems. The duel is being cancelled."
                                .to_vec(),
                        ),
                        (
                            1,
                            b"You will be teleported into the city in 10 seconds.".to_vec(),
                        ),
                    ]
                };
                assert_eq!(chats(inbox), expected);
            }
            assert_eq!(world.arenas[&(0, 41)][0].ready, Some((250, 2)));
            assert_eq!(world.arenas[&(0, 41)][0].set_points, [0, 0]);
        }
    }
    #[test]
    fn arena_login_member_sends_empty_opponent_list_without_binding_or_advancing_timer() {
        let (mut world, mut inboxes) = active();
        let vid = Vid::new(0x1234);
        let map = world.bodies[&vid].map;
        world.enter_arena(vid);
        assert_eq!(packets(&mut inboxes[0]), [vec![40, 3, 0]]);
        assert!(!world.in_arena(vid));
        assert_eq!(
            world.arenas[&(0, u32::try_from(map).unwrap())][0].ready,
            Some((250, 0))
        );
        world.fire_arena_ready(((0, 41), 0), 0, 250);
        assert!(world.in_arena(vid));
    }

    #[test]
    fn arena_potion_count_resets_at_ready_and_after_each_scored_round() {
        let (mut world, _) = active();
        let vid = Vid::new(0x1234);
        world.set_arena_potion_count(vid, 0x1234);
        world.consume_arena_potion(vid);
        assert_eq!(world.arena_potion_count(vid), 0x1233);
        world.fire_arena_ready(((0, 41), 0), 0, 250);
        assert_eq!(world.arena_potion_count(vid), 0);
        world.set_arena_potion_count(vid, 0x2345);
        world.arena_on_dead(vid, Vid::new(0x2345));
        assert_eq!(world.arena_potion_count(vid), 0);
    }
}
