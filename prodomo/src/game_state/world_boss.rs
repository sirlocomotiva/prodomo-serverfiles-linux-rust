//! The world-boss event (`CWorldBoss`, `G/worldboss.h`, `G/worldboss.cpp:1-908`)
//! with its line (`do_worldboss`, `G/cmd_gm.cpp:5204-5237`).
//!
//! Legacy is one process-wide singleton whose event flag (`world_boss_event`)
//! fans out through `SetEventFlag` (`G/questmanager.cpp:1344-1348`). Every
//! Channel core runs its own three timers, but only channel 1 spawns, kills
//! and escapes (`WORLD_BOSS_CHANNEL`, `G/worldboss.h:60`); the other cores'
//! combat timers still count down and still end in a cooldown. The Rewrite is
//! one world hosting every Channel's maps, so the port keeps one state with
//! per-Channel Pulse countdowns and gates the spawn, kill and escape arms on
//! channel 1, as legacy's channel-1 core does.
//!
//! Timers ride the per-owner Pulse countdown pattern the GM arena uses: each
//! arm writes its due Pulse, and the tick runs what is due. Equal-Pulse fires
//! run in arm order (cooldown, then combat, then spawn) where legacy's heap
//! order at equal priority is heap-shape dependent (a Divergence the arena
//! records too). The map and boss draws use the world's own dice; the spawn
//! wait is always 5 seconds, because legacy builds its distribution and never
//! samples it (`:596-597`: `random_time` stays 5).
//!
//! The temp ranking lives in memory. The season bank (`UpdateSeasonRanking`,
//! `:858-884`) is dropped: its only sink is the SQL season table the DB peer
//! flushes hourly or on `flush`, which no game code reads back (the ranking
//! answers come from the temp table), so `/flush` is silent.
//!
//! # Divergences
//!
//! - **The ranking order at equal damage.** Legacy sorts with `std::sort`, the
//!   Rewrite with a stable sort; rows with equal damage may rank differently.
//! - **The kill walk.** Legacy pushes ranking rows in descriptor-set (pointer)
//!   order; the Rewrite walks VIDs in order (V1).
//! - **Channel 1.** Legacy reads `g_bChannel` per core; the Rewrite spawns on
//!   the channel-1 copy of the drawn map and arms the spawn only while the
//!   process hosts channel 1 at all.

use common::vid::Vid;
use world::character::{AutoGiven, MoveDone, MoveKind};

use super::GameState;
use crate::chat_line::Arg;
use crate::command::{one_argument, GM_PLAYER};
use crate::item_move::{MovedItems, Mover};
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};

/// `WORLD_BOSS_CHANNEL` (`G/worldboss.h:60`): only channel 1 spawns.
pub(super) const CHANNEL: u8 = 1;

/// `EState` (`G/worldboss.h:21-30`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum BossState {
    /// `WORLD_BOSS_STATE_NONE`.
    #[default]
    None = 0,
    /// `WORLD_BOSS_STATE_WAIT_FOR_SPAWN`.
    WaitForSpawn = 1,
    /// `WORLD_BOSS_STATE_BOSS_SPAWNED`.
    Spawned = 2,
    /// `WORLD_BOSS_STATE_BOSS_KILLED` (kill and escape both land here).
    Killed = 3,
    /// `WORLD_BOSS_STATE_BREAK_TIME`.
    BreakTime = 4,
    // `WORLD_BOSS_STATE_END_CYCLE` is never assigned, as in legacy, so it has
    // no member.
}

/// Which timer arm a due Pulse belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum BossDue {
    /// `worldboss_cooldown_event`: one shot, then the run time starts.
    Cooldown,
    /// `worldboss_combat_event`: every second of the run.
    Combat,
    /// `worldboss_spawn_event`: one shot, then the boss spawns.
    Spawn,
}

/// One `CG_WORLD_BOSS` ask (`CInputMain::WorldBoss`, `G/input_main.cpp:3553-3570`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldBossStep {
    /// `CG_INFO` (0): the state, the seconds, the damage and the minimum.
    Info,
    /// `CG_REWARD` (1): the loot draw.
    Reward,
    /// `CG_RANKING` (2): the temp ranking.
    Ranking,
}

/// The run's two marks (`m_bBossSpawn`, `m_bBossAttacked`): whether the boss
/// stands and whether it has been hit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct BossMarks {
    /// The boss stands.
    pub(super) spawned: bool,
    /// The boss has been hit.
    pub(super) attacked: bool,
}

/// One temp-ranking row (`TPacketGDTempWorldBossRanking`, `tables.h:2258-2265`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BossRankRow {
    /// The hunter's PID.
    pub(super) pid: u32,
    /// `szPlayerName`, zero-padded to 25.
    pub(super) player: [u8; protocol::gc_world_boss::PLAYER_NAME_LEN],
    /// `szGuildName`, zero-padded to 13.
    pub(super) guild: [u8; protocol::gc_world_boss::GUILD_NAME_LEN],
    /// `bEmpire`.
    pub(super) empire: u8,
    /// `dwRecord`: the total damage.
    pub(super) record: u32,
}

/// The `do_worldboss` usage (`G/cmd_gm.cpp:5213-5215`), one line each.
const USAGE: [&[u8]; 3] = [
    b"Usage: worldboss start",
    b"       worldboss stop",
    b"       worldboss flush",
];

/// `Enable(true)`'s notice (`G/worldboss.cpp:423`).
const STARTED: &[u8] = b"[World boss] Event started";
/// `Enable(false)`'s notice (`:427`).
const COMPLETED: &[u8] = b"[World boss] Event completed";
/// `SetRunTime`'s notice (`:612`).
const COMING_SOON: &[u8] = b"[World boss] Caution: the world boss will appear soon!";
/// `SetRunTime`'s test-server notice (`:607`): the `secconds` typo is legacy's.
const TEST_RUNTIME: &[u8] = b"[Test World boss] The boss will appear in %d secconds.";
/// `Spawn`'s notice format (`:484-486`).
const SPAWNED_FORMAT: &[u8] = b"[World boss] In Channel %d, %s has appeared and is terrorising the area of %s. Let's go hunting!";
/// `Spawn`'s test-server notice format (`:492`).
const TEST_SPAWN_FORMAT: &[u8] = b"[World Boss Test] %s spawn at %d, %d";
/// `Escape`'s notice (`:529`).
const ESCAPED: &[u8] = b"[World boss] The World boss has escaped! Good luck next time.";
/// `Kill`'s notice format (`:545`).
const KILLED_FORMAT: &[u8] =
    b"[World boss] Well done: %s has been defeated and peace has been restored... for now.";
/// The first-damage notice format (`:673`).
const ATTACKED_FORMAT: &[u8] =
    b"[World boss] The world boss in Channel %d has been attacked. Join the hunt now!";
/// `Reward`'s refusals and answers (`:704`, `:795`, `:796`, `:816`).
const REWARD_WAIT: &[u8] = b"[World boss] You can't receive a reward now.";
const REWARD_GIVEN: &[u8] =
    b"[World boss] Well done! Here is your reward. Join in next time as well!";
const REWARD_FULL: &[u8] = b"[World boss] You can't receive a reward now, because you do not have enough space in your inventory.";
/// `Reward`'s loot-level lines (`:747`, `:753`, `:759`, `:765`).
const LOOT_I: &[u8] = b"[World boss] Reward for Loot Level I in the rankings: %d";
const LOOT_II: &[u8] = b"[World boss] Reward for Loot Level II in the rankings: %d";
const LOOT_III: &[u8] = b"[World boss] Reward for Loot Level III in the rankings: %d";
const LOOT_IV: &[u8] = b"[World boss] Reward for Loot Level IV in the rankings: %d";
/// `Reward`'s special item groups (`:748`, `:754`, `:760`, `:766`).
const GROUP_I: i32 = 11001;
const GROUP_II: i32 = 11002;
const GROUP_III: i32 = 11003;
const GROUP_IV: i32 = 11004;
/// `Reward`'s affect values (`:749`, `:755`, `:761`).
const AFFECT_I: i32 = 100;
const AFFECT_II: i32 = 50;
const AFFECT_III: i32 = 25;
/// `Reward`'s affect (`:809`): 7200 s of the mall item bonus.
const REWARD_SECONDS: i32 = 7200;
// `GetRankingPointsByLootLevel` (`:844-856`) has no caller in legacy, so it
// has no port.

/// `s_map_MapName` (`G/worldboss.cpp:19-25`): the map names, English literals
/// in `LC_TEXT`, so the raw bytes speak for every language (the same shape the
/// battle arena's `START_FORMAT` arm uses for its format string).
fn map_name(map: i64) -> &'static [u8] {
    match map {
        61 => b"Mount Sohan",
        62 => b"Doyyumhwan",
        63 => b"Yongbi Desert",
        64 => b"Valley of Seungryong",
        // `strdup("")` for an unknown map (`:32`): the empty name.
        _ => b"",
    }
}

impl GameState {
    /// `do_worldboss` (`G/cmd_gm.cpp:5204-5237`): an empty word answers the
    /// three-line usage; `start` and `stop` set `world_boss_event` to 1 and 0
    /// (which runs `Enable` through the flag bridge, as legacy's
    /// `RequestSetEventFlag` does); `flush` parses and lands nowhere; any
    /// other word is silent. The compare is case-insensitive (`strcasecmp`).
    pub(super) fn run_worldboss_line(&mut self, vid: Vid, rest: &[u8], mover: Mover) {
        let (word, _) = one_argument(rest);
        if word.is_empty() {
            for line in USAGE {
                self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
            }
            return;
        }
        if word.eq_ignore_ascii_case(b"start") {
            self.set_event_flag(b"world_boss_event", 1);
        } else if word.eq_ignore_ascii_case(b"stop") {
            self.set_event_flag(b"world_boss_event", 0);
        } else if word.eq_ignore_ascii_case(b"flush") {
            // The season flush writes the SQL season table, which no game code
            // reads back, so the word parses and lands nowhere.
        }
    }

    /// Run one `CG_WORLD_BOSS` ask: the info, the reward draw, or the ranking.
    /// The records reach the client through the world; the answer carries the
    /// reward's rows to store, or nothing.
    pub(super) fn world_boss_step(
        &mut self,
        vid: Vid,
        step: WorldBossStep,
        mover: Mover,
    ) -> MovedItems {
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        let protos = std::sync::Arc::clone(&self.protos);
        let locale = std::sync::Arc::clone(&self.locale);
        if matches!(step, WorldBossStep::Reward) {
            if let Some(done) = self.world_boss_reward(vid) {
                return MovedItems::new(pid, vid.raw(), done, mover, &protos, &locale);
            }
        } else if matches!(step, WorldBossStep::Info) {
            self.world_boss_info(vid);
        } else {
            self.world_boss_ranking_of(vid);
        }
        Self::world_boss_silent(&protos, &locale, pid, vid, mover)
    }

    /// An answer that carries nothing: a record-only ask.
    fn world_boss_silent(
        protos: &gamedata::item_proto::ItemProtos,
        locale: &std::sync::Arc<gamedata::locale_string::LocaleStrings>,
        pid: u32,
        vid: Vid,
        mover: Mover,
    ) -> MovedItems {
        MovedItems::new(
            pid,
            vid.raw(),
            MoveDone {
                kind: MoveKind::Given,
                records: Vec::new(),
                changes: Vec::new(),
            },
            mover,
            protos,
            locale,
        )
    }

    /// `CWorldBoss::Enable` (`G/worldboss.cpp:414-445`): re-initialise, then
    /// start the cooldown and speak, or speak the completion. Every connected
    /// PC then hears its `worldboss %d` command line, and the quest letter for
    /// `daily_world_boss` runs where the quest exists.
    pub(super) fn world_boss_enable(&mut self, enabled: bool) {
        self.world_boss_initialize();
        if enabled {
            let cooldown = self.world_boss_cooldown_secs();
            self.world_boss_set_cooldown(cooldown);
            self.notice_every_pc(STARTED, &[]);
        } else {
            self.notice_every_pc(COMPLETED, &[]);
        }
        let quest = self
            .quests
            .as_ref()
            .and_then(|quests| quests.manager().host().quest_index(b"daily_world_boss"));
        let mut vids: Vec<Vid> = self.outboxes.keys().copied().collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            let Some(mover) = self.own_mover(vid) else {
                continue;
            };
            if let Some(quest) = quest {
                self.quest_letter(vid, quest);
            }
            let value: i64 = i64::from(enabled);
            self.tell(
                vid,
                mover,
                CHAT_TYPE_COMMAND,
                b"worldboss %d",
                &[Arg::Int(value)],
            );
        }
    }

    /// `CWorldBoss::Initialize` (`G/worldboss.cpp:150-175`): clear the run,
    /// cancel every timer, and drop the temp ranking.
    fn world_boss_initialize(&mut self) {
        self.world_boss_map = 0;
        self.world_boss_vnum = 0;
        self.world_boss_vid = 0;
        self.world_boss_marks.spawned = false;
        self.world_boss_marks.attacked = false;
        self.world_boss_state = BossState::None;
        self.world_boss_cancel();
        self.world_boss_run_left = 0;
        self.world_boss_cooldown_left = 0;
        self.world_boss_ranking.clear();
        self.world_boss_passed = 0;
        self.world_boss_duration = 0;
    }

    /// Cancel every armed world-boss timer (`event_cancel`, `:164-172`).
    fn world_boss_cancel(&mut self) {
        for due in [
            self.world_boss_cooldown_due.take(),
            self.world_boss_combat_due.take(),
            self.world_boss_spawn_due.take(),
        ]
        .into_iter()
        .flatten()
        {
            self.regens.due.remove(&(due, super::regen::Due::WorldBoss));
        }
    }

    /// Arm one world-boss timer (`event_create`, `PASSES_PER_SEC(secs)`).
    fn world_boss_arm(&mut self, arm: BossDue, secs: u32) {
        let when = self.last_pulse.saturating_add(super::regen::pulses(secs));
        match arm {
            BossDue::Cooldown => {
                if let Some(due) = self.world_boss_cooldown_due.replace(when) {
                    self.regens.due.remove(&(due, super::regen::Due::WorldBoss));
                }
            }
            BossDue::Combat => {
                if let Some(due) = self.world_boss_combat_due.replace(when) {
                    self.regens.due.remove(&(due, super::regen::Due::WorldBoss));
                }
            }
            BossDue::Spawn => {
                if let Some(due) = self.world_boss_spawn_due.replace(when) {
                    self.regens.due.remove(&(due, super::regen::Due::WorldBoss));
                }
            }
        }
        self.regens.due.insert((when, super::regen::Due::WorldBoss));
    }

    /// The world-boss timers due on `pulse`: the cooldown one-shot runs the
    /// run time, the combat arm runs its second, and the spawn one-shot spawns.
    /// Equal-Pulse fires run cooldown, combat, then spawn.
    pub(super) fn run_world_boss_tick(&mut self, pulse: u64) {
        if self.world_boss_cooldown_due.is_some_and(|due| due <= pulse) {
            self.world_boss_cooldown_due = None;
            let runtime = self.world_boss_runtime_secs();
            self.world_boss_set_runtime(runtime);
        }
        if self.world_boss_combat_due.is_some_and(|due| due <= pulse) {
            self.world_boss_combat_due = None;
            self.world_boss_combat_second();
        }
        if self.world_boss_spawn_due.is_some_and(|due| due <= pulse) {
            self.world_boss_spawn_due = None;
            self.world_boss_spawn();
        }
    }

    /// `CWorldBoss::SetRunTime` (`G/worldboss.cpp:584-621`): arm the combat
    /// event every second and, on channel 1, the one-shot spawn after 5
    /// seconds; speak the coming-soon notice (and the test-server countdown,
    /// which is `SendNotice`, every PC); land in `WAIT_FOR_SPAWN` and write
    /// the state flag.
    fn world_boss_set_runtime(&mut self, duration: u32) {
        self.world_boss_duration = duration;
        self.world_boss_passed = 0;
        self.world_boss_arm(BossDue::Combat, 1);
        // NOTE: Only spawn the boss on channel 1 (`:594`), but the combat arm
        // above runs everywhere, as each legacy core armed it. The 5 is not a
        // fallback: legacy's `dist` is never sampled (`:596-599`).
        if self.channel_one_hosts() {
            self.world_boss_arm(BossDue::Spawn, 5);
            if self.test_server {
                self.notice_every_pc(TEST_RUNTIME, &[Arg::Int(5)]);
            }
        }
        // `SendNotice` (`:612`): every PC on this core, as `Enable` speaks.
        self.notice_every_pc(COMING_SOON, &[]);
        self.world_boss_cooldown_left = 0;
        self.world_boss_run_left = Self::clock_now().saturating_add(u64::from(duration));
        self.world_boss_set_state(BossState::WaitForSpawn);
    }

    /// `CWorldBoss::SetCooldown` (`G/worldboss.cpp:623-637`): arm the one-shot
    /// cooldown, land in `BREAK_TIME` and write the state flag.
    fn world_boss_set_cooldown(&mut self, duration: u32) {
        self.world_boss_arm(BossDue::Cooldown, duration);
        self.world_boss_cooldown_left = Self::clock_now().saturating_add(u64::from(duration));
        self.world_boss_run_left = 0;
        self.world_boss_set_state(BossState::BreakTime);
    }

    /// `worldboss_combat_event` (`G/worldboss.cpp:72-106`): past the duration
    /// the cooldown starts; at half the duration a spawned boss escapes; else
    /// the count rises and the next second arms.
    fn world_boss_combat_second(&mut self) {
        if self.world_boss_passed > self.world_boss_duration {
            let cooldown = self.world_boss_cooldown_secs();
            self.world_boss_set_cooldown(cooldown);
            return;
        }
        if self.world_boss_passed >= self.world_boss_duration / 2 && self.world_boss_marks.spawned {
            self.world_boss_escape();
        }
        self.world_boss_passed = self.world_boss_passed.saturating_add(1);
        self.world_boss_arm(BossDue::Combat, 1);
    }

    /// `CWorldBoss::Spawn` (`G/worldboss.cpp:447-506`): on channel 1 only, draw
    /// the map and the vnum, stand the boss at a random position, speak the
    /// spawn notice (and the test-server coordinates), clear the temp ranking,
    /// land in `BOSS_SPAWNED` and write the state flag. Off channel 1, or with
    /// the boss already out, or with no table, or with no stand, nothing lands.
    fn world_boss_spawn(&mut self) -> bool {
        if self.world_boss_marks.spawned {
            return false;
        }
        if self.world_boss_table.maps.is_empty() || self.world_boss_table.bosses.is_empty() {
            return false;
        }
        let map_last =
            i32::try_from(self.world_boss_table.maps.len().saturating_sub(1)).unwrap_or(0);
        let boss_last =
            i32::try_from(self.world_boss_table.bosses.len().saturating_sub(1)).unwrap_or(0);
        let map_pick = world::character::number(&mut self.dice, 0, map_last);
        let boss_pick = world::character::number(&mut self.dice, 0, boss_last);
        let map = self
            .world_boss_table
            .maps
            .get(usize::try_from(map_pick).unwrap_or(0))
            .copied()
            .unwrap_or(0);
        let vnum = self
            .world_boss_table
            .bosses
            .get(usize::try_from(boss_pick).unwrap_or(0))
            .copied()
            .unwrap_or(0);
        let places = self.places_hosting(i32::try_from(map).unwrap_or(0));
        let Some(place) = places.into_iter().find(|&(channel, _)| channel == CHANNEL) else {
            return false;
        };
        let before: Vec<u32> = self
            .npcs
            .get(&place)
            .map(|list| list.npcs.iter().map(|npc| npc.vid).collect())
            .unwrap_or_default();
        self.spawn_event_mob_randomly(place, vnum);
        let stood = self
            .npcs
            .get(&place)
            .and_then(|list| list.npcs.iter().find(|npc| !before.contains(&npc.vid)))
            .map(|npc| (npc.vid, npc.name.clone(), npc.x, npc.y));
        let Some((vid, name, x, y)) = stood else {
            return false;
        };
        self.world_boss_map = i32::try_from(map).unwrap_or(0);
        self.world_boss_vnum = vnum;
        self.world_boss_vid = vid;
        self.world_boss_marks.spawned = true;
        self.world_boss_marks.attacked = false;
        self.notice_every_pc(
            SPAWNED_FORMAT,
            &[
                Arg::Int(i64::from(CHANNEL)),
                Arg::Text(&name),
                Arg::Text(map_name(map)),
            ],
        );
        if self.test_server {
            let (mx, my) = (x / 100, y / 100);
            self.notice_every_pc(
                TEST_SPAWN_FORMAT,
                &[
                    Arg::Text(&name),
                    Arg::Int(i64::from(mx)),
                    Arg::Int(i64::from(my)),
                ],
            );
        }
        // `HEADER_GD_CLR_TEMP_WORLD_BOSS_RK` (`:499`): the temp ranking clears.
        self.world_boss_ranking.clear();
        self.world_boss_set_state(BossState::Spawned);
        true
    }

    /// `CWorldBoss::Escape` (`G/worldboss.cpp:508-533`): on channel 1 only, and
    /// only with the boss out, destroy the boss, land in `BOSS_KILLED`, speak
    /// and write the state flag.
    fn world_boss_escape(&mut self) {
        if !self.world_boss_marks.spawned {
            return;
        }
        let vid = self.world_boss_vid;
        if self.npc_by_vid(vid).is_some() {
            self.destroy_npc(vid);
            self.world_boss_marks.spawned = false;
        }
        self.world_boss_set_state(BossState::Killed);
        // `BroadcastNotice` (`:529`): every core's PCs, as `Enable` speaks.
        self.notice_every_pc(ESCAPED, &[]);
    }

    /// `CWorldBoss::Kill` (`G/worldboss.cpp:535-582`): land in `BOSS_KILLED`,
    /// speak, clear every connected PC's `world_boss_reward` flag, rank every
    /// PC on the boss's map past the level and damage bars, and write the
    /// state flag. Only the channel-1 core's kill runs here (the death hook
    /// gates on it). The season bank after the ranking is dropped with `/flush`.
    pub(super) fn world_boss_kill(&mut self, name: &[u8]) {
        self.world_boss_state = BossState::Killed;
        self.world_boss_marks.spawned = false;
        self.notice_every_pc(KILLED_FORMAT, &[Arg::Text(name)]);
        let (map, vid, min_damage, min_level) = (
            self.world_boss_map,
            self.world_boss_vid,
            self.world_boss_min_damage(),
            self.world_boss_min_level(),
        );
        let mut hunters: Vec<(Vid, i32, u8)> = self
            .bodies
            .iter()
            .map(|(&hunter, body)| (hunter, body.map, body.card.empire))
            .collect();
        hunters.sort_by_key(|hunter| hunter.0.raw());
        for (hunter, stood, empire) in hunters {
            let Ok(character) = self.characters.find_by_vid(hunter) else {
                continue;
            };
            let pid = character.player_id();
            if let Some(quests) = self.quests.as_mut() {
                quests
                    .manager_mut()
                    .set_quest_flag(pid, b"world_boss_reward", 0);
            }
            if stood != map {
                continue;
            }
            let level = character
                .points()
                .map_or(0, world::character::Points::level);
            let damage = self
                .npc_combat
                .get(&vid)
                .and_then(|combat| combat.damage.get(&hunter.raw()))
                .map_or(0, |row| u32::try_from(row.total.max(0)).unwrap_or(u32::MAX));
            if u32::from(level) < min_level || damage <= min_damage {
                continue;
            }
            let cname = character.name().as_bytes().to_vec();
            let guild = self
                .guild_of(pid)
                .map_or(Vec::new(), |id| self.guild_name_of(id));
            let mut player = [0u8; protocol::gc_world_boss::PLAYER_NAME_LEN];
            let mut gname = [0u8; protocol::gc_world_boss::GUILD_NAME_LEN];
            let take = cname.len().min(player.len());
            player[..take].copy_from_slice(&cname[..take]);
            let take = guild.len().min(gname.len());
            gname[..take].copy_from_slice(&guild[..take]);
            self.world_boss_ranking.push(BossRankRow {
                pid,
                player,
                guild: gname,
                empire,
                record: damage,
            });
        }
        self.world_boss_set_state(BossState::Killed);
    }

    /// Write the state and its event flag (`RequestSetEventFlag("world_boss_state",
    /// state)` after every transition).
    fn world_boss_set_state(&mut self, state: BossState) {
        self.world_boss_state = state;
        self.set_event_flag(b"world_boss_state", state as i32);
    }

    /// `CWorldBoss::Process`'s `GC_INFO` ask (`G/worldboss.cpp:651-665`): the
    /// state (legacy reads the flag off channel 1; the flag mirrors the state
    /// here, so it reads the state), the seconds left, the asker's total
    /// damage and the minimum damage.
    pub(super) fn world_boss_info(&mut self, asker: Vid) {
        if self.own_mover(asker).is_none() {
            return;
        }
        let state = self.world_boss_state as u32;
        let damage = self.world_boss_damage_of(asker);
        let body = protocol::gc_world_boss::GcWorldBossProcess {
            state,
            run_time_left: u32::try_from(self.world_boss_run_left).unwrap_or(u32::MAX),
            cooldown_left: u32::try_from(self.world_boss_cooldown_left).unwrap_or(u32::MAX),
            total_damage: damage,
            min_damage: self.world_boss_min_damage(),
        };
        let record = protocol::gc_world_boss::encode_info(body);
        let _sent = self.write_to_client(asker, record.to_vec());
    }

    /// `CWorldBoss::Process`'s `GC_DAMAGE` arm (`:668-681`): the first hit
    /// speaks, then the asker's total damage answers.
    pub(super) fn world_boss_damaged(&mut self, asker: Vid) {
        if !self.world_boss_marks.attacked {
            self.notice_every_pc(ATTACKED_FORMAT, &[Arg::Int(i64::from(CHANNEL))]);
            self.world_boss_marks.attacked = true;
        }
        let Some(mover) = self.own_mover(asker) else {
            return;
        };
        let _ = mover;
        let damage = self.world_boss_damage_of(asker);
        let record = protocol::gc_world_boss::encode_damage(damage);
        let _sent = self.write_to_client(asker, record.to_vec());
    }

    /// `CWorldBoss::Reward` (`G/worldboss.cpp:697-819`): the bars, the 3-second
    /// request throttle, the loot level and the special-item-group draw with
    /// the mall-bonus affect. Answers the merged give, or nothing on a refusal
    /// or a failed give (legacy's answer is discarded by the caller, `:3562`;
    /// the full-inventory arm still answers `true` there, `:817`, which is
    /// nothing to store here either).
    ///
    /// The congrats lines speak before the give, so a failed give still spoke
    /// them: only the affect and the quest flag wait for the give. The need
    /// count draws its own roll set, as the give draws its own
    /// (`GetMultiIndex` runs twice, `:768` and in the give, `:9587`). A row
    /// the give cannot land fails the whole give, as legacy's `false` does;
    /// keyword rows land nothing, as `/letters_open`'s draw does (their
    /// systems are not ported). An empty group refuses: legacy's give then
    /// returns an uninitialised `bSuccess` (`:9589`, `:9689`), which is not
    /// reproduced.
    pub(super) fn world_boss_reward(&mut self, asker: Vid) -> Option<MoveDone> {
        let mover = self.own_mover(asker)?;
        if self.world_boss_state == BossState::Spawned {
            self.tell(asker, mover, CHAT_TYPE_INFO, REWARD_WAIT, &[]);
            return None;
        }
        if self.world_boss_request_pulse(asker) > self.last_pulse && !self.is_gm(asker) {
            return None;
        }
        self.set_world_boss_request_pulse(asker);
        if self.world_boss_state != BossState::Killed {
            return None;
        }
        let (pid, position, group, affect, line) = self.world_boss_reward_loot(asker)?;
        let bag = self.item_groups.special.get(&group).cloned()?;
        if bag.group_size() == 0 {
            return None;
        }
        let rolls: Vec<i32> = (0..bag.group_size())
            .map(|_| world::character::number(&mut self.dice, 1, 100))
            .collect();
        let mut need: u8 = 0;
        for index in bag.multi_index(&rolls) {
            let size = bag
                .vnum(index)
                .and_then(|vnum| self.protos.get(vnum))
                .map_or(0, |proto| {
                    u8::try_from(proto.size.max(0)).unwrap_or(u8::MAX)
                });
            need = need.wrapping_add(size);
        }
        if self.free_inventory_of(asker) <= u32::from(need) {
            self.tell(asker, mover, CHAT_TYPE_INFO, REWARD_FULL, &[]);
            return None;
        }
        self.tell(asker, mover, CHAT_TYPE_INFO, REWARD_GIVEN, &[]);
        self.tell(
            asker,
            mover,
            CHAT_TYPE_INFO,
            line,
            &[Arg::Int(i64::try_from(position).unwrap_or(i64::MAX))],
        );
        let rolls: Vec<i32> = (0..bag.group_size())
            .map(|_| world::character::number(&mut self.dice, 1, 100))
            .collect();
        let mut out: Option<MoveDone> = None;
        for index in bag.multi_index(&rolls) {
            let (Some(vnum), Some(count)) = (bag.vnum(index), bag.count(index)) else {
                return None;
            };
            if vnum <= 7 {
                // A keyword row (`EGiveType`): its system is not ported, so the
                // draw lands nothing, as `/letters_open`'s does.
                continue;
            }
            let rare = bag.rare_pct(index).unwrap_or(0);
            let Ok(count) = u16::try_from(count.max(0)) else {
                return None;
            };
            let (outcome, done) = self.auto_give(asker, (vnum, count), rare, false)?;
            match outcome {
                AutoGiven::NoProto | AutoGiven::NotMade => {
                    return None;
                }
                AutoGiven::Merged(_) | AutoGiven::Placed { .. } => {}
                AutoGiven::Unplaced(item) => self.lay_auto_given(asker, item),
            }
            match out.as_mut() {
                Some(out) => {
                    out.records.extend(done.records);
                    out.changes.extend(done.changes);
                }
                None => out = Some(done),
            }
        }
        let out = out?;
        if affect > 0 {
            self.add_pc_affect(
                asker,
                super::affect::Affect {
                    kind: crate::loading_phase::AFFECT_WORLD_BOSS_REWARD,
                    apply_on: u8::try_from(common::point_slot::POINT_MALL_ITEMBONUS)
                        .unwrap_or(u8::MAX),
                    value: affect,
                    flag: 0,
                    remaining: REWARD_SECONDS,
                    sp_cost: 0,
                },
                true,
            );
        }
        if let Some(quests) = self.quests.as_mut() {
            quests
                .manager_mut()
                .set_quest_flag(pid, b"world_boss_reward", 1);
        }
        Some(out)
    }

    /// `Reward`'s bars and loot level (`G/worldboss.cpp:717-766`): the quest
    /// flag, the level and the damage bars, then the asker's rank in the
    /// damage-sorted temp ranking, which names the group, the affect and the
    /// line. Nothing when any bar refuses or the asker never ranked.
    fn world_boss_reward_loot(&self, asker: Vid) -> Option<(u32, usize, i32, i32, &'static [u8])> {
        let pid = self
            .characters
            .find_by_vid(asker)
            .map_or(0, world::character::Character::player_id);
        if self.world_boss_quest_flag(pid, b"world_boss_reward") > 0 {
            return None;
        }
        let level = self
            .characters
            .find_by_vid(asker)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, world::character::Points::level);
        if u32::from(level) < self.world_boss_min_level() {
            return None;
        }
        if self.world_boss_damage_of(asker) < self.world_boss_min_damage() {
            return None;
        }
        let mut sorted = self.world_boss_ranking.clone();
        sorted.sort_by(|left, right| right.record.cmp(&left.record));
        let position = sorted
            .iter()
            .position(|row| row.pid == pid)
            .map(|at| at + 1)?;
        let (group, affect, line) = if (1..=10).contains(&position) {
            (GROUP_I, AFFECT_I, LOOT_I)
        } else if (11..=25).contains(&position) {
            (GROUP_II, AFFECT_II, LOOT_II)
        } else if (26..=100).contains(&position) {
            (GROUP_III, AFFECT_III, LOOT_III)
        } else {
            (GROUP_IV, 0, LOOT_IV)
        };
        Some((pid, position, group, affect, line))
    }

    /// `CWorldBoss::GetRanking` (`G/worldboss.cpp:821-836`): past the 3-second
    /// throttle, answer the temp ranking, sorted by damage.
    pub(super) fn world_boss_ranking_of(&mut self, asker: Vid) {
        if self.world_boss_request_pulse(asker) > self.last_pulse && !self.is_gm(asker) {
            return;
        }
        self.set_world_boss_request_pulse(asker);
        let Some(mover) = self.own_mover(asker) else {
            return;
        };
        let _ = mover;
        let mut sorted = self.world_boss_ranking.clone();
        sorted.sort_by(|left, right| right.record.cmp(&left.record));
        let mut record =
            protocol::gc_world_boss::encode_head(protocol::gc_world_boss::SUB_RANKING).to_vec();
        // `wSize` counts the whole record (`:897`).
        let size = protocol::gc_world_boss::HEAD_LEN
            + sorted.len() * protocol::gc_world_boss::RANKING_ROW_LEN;
        let size = u16::try_from(size).unwrap_or(u16::MAX);
        let [lo, hi] = size.to_le_bytes();
        record[1] = lo;
        record[2] = hi;
        for row in &sorted {
            let encoded = protocol::gc_world_boss::encode_ranking_row(
                protocol::gc_world_boss::GcWorldBossRankingRow {
                    player: row.player,
                    guild: row.guild,
                    empire: row.empire,
                    record: row.record,
                },
            );
            record.extend_from_slice(&encoded);
        }
        let _sent = self.write_to_client(asker, record);
    }

    /// The asker's accumulated damage on the boss (`GetAccumulateDamageByVID`,
    /// `G/char.cpp:11601-11616`): the damage row under its VID in the boss's
    /// combat map.
    fn world_boss_damage_of(&self, asker: Vid) -> u32 {
        self.npc_combat
            .get(&self.world_boss_vid)
            .and_then(|combat| combat.damage.get(&asker.raw()))
            .map_or(0, |row| u32::try_from(row.total.max(0)).unwrap_or(u32::MAX))
    }

    /// The config values. A refused file reads zero: the constructor's
    /// `EConfig` defaults die in its own `memset` (`:134-135`), and the loader
    /// zeroes before it reads (`:240`), so a missing table is all zeros.
    fn world_boss_cooldown_secs(&self) -> u32 {
        self.world_boss_table.config.cooldown
    }

    /// `GetRunTime`.
    fn world_boss_runtime_secs(&self) -> u32 {
        self.world_boss_table.config.runtime
    }

    /// `GetMinDamage` (`G/worldboss.h:72`); the quest
    /// `game.get_worldboss_min_damage` bridge reads this.
    pub(super) fn world_boss_min_damage(&self) -> u32 {
        self.world_boss_table.config.min_damage
    }

    /// The config's minimum level.
    fn world_boss_min_level(&self) -> u32 {
        self.world_boss_table.config.min_level
    }

    /// Whether the process hosts channel 1 at all: legacy's `g_bChannel == 1`
    /// arm reads as hosted here, since the one process holds every Channel.
    fn channel_one_hosts(&self) -> bool {
        self.channel_hosts(CHANNEL)
    }

    /// The wall-clock seconds both `left` fields count in (`time(0)`).
    fn clock_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs())
    }
}

/// The per-PC world-boss request throttle (`GetWorldBossRequestPulse`,
/// `G/char.h`): 3 s past each ask.
impl GameState {
    /// Whether `asker` is throttled.
    fn world_boss_request_pulse(&self, asker: Vid) -> u64 {
        self.world_boss_throttle
            .get(&asker.raw())
            .copied()
            .unwrap_or(0)
    }

    /// Stamp the 3-second throttle (`PASSES_PER_SEC(3)`).
    fn set_world_boss_request_pulse(&mut self, asker: Vid) {
        let when = self.last_pulse.saturating_add(super::regen::pulses(3));
        self.world_boss_throttle.insert(asker.raw(), when);
    }

    /// `IsGM` (`G/char.cpp:6373-6380`): any GM level, or anyone while the
    /// test server is on (the throttle skips GMs, `:708`, `:826`).
    fn is_gm(&self, asker: Vid) -> bool {
        if self.test_server {
            return true;
        }
        self.characters
            .find_by_vid(asker)
            .ok()
            .and_then(|character| character.points())
            .is_some_and(|points| points.gm_level() != GM_PLAYER)
    }

    /// One PC quest flag (`GetQuestFlag("world_boss_reward")`).
    fn world_boss_quest_flag(&self, pid: u32, name: &[u8]) -> i32 {
        self.quests
            .as_ref()
            .and_then(|quests| quests.manager().quest_flag(pid, name))
            .unwrap_or(0)
    }

    /// Free inventory cells (`CountEmptyInventory`): the usable cells minus
    /// the held items.
    fn free_inventory_of(&self, asker: Vid) -> u32 {
        let Ok(character) = self.characters.find_by_vid(asker) else {
            return 0;
        };
        let usable = common::item_slots::usable_inventory_cells(character.inven_point());
        let held = u16::try_from(character.items().occupied().len()).unwrap_or(u16::MAX);
        u32::from(usable.saturating_sub(held))
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
            killer_mode: false,
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

    /// A world with quests (for the event flags), the owner's world-boss
    /// table, the fixture map scheduled (as boot schedules hosted maps, so
    /// the channel-1 arms run), and a caller on it.
    fn boss_world(clock: &TestClock) -> (GameState, UnboundedReceiver<Vec<u8>>) {
        let mut world = a_world(clock);
        world.schedule_map(
            super::super::fixtures::PLACE,
            &super::super::fixtures::region(),
        );
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager =
            quest::manager::Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        world.quests = Some(super::super::quests::Quests::new(
            manager,
            std::sync::Arc::default(),
            MobNamesByLanguage::default(),
        ));
        let bytes = std::fs::read(locale_dir.join("world_boss.txt")).expect("the table loads");
        let table = gamedata::world_boss::read(&bytes).expect("the table reads");
        world.world_boss_table = std::sync::Arc::new(table);
        let inbox = enter_on(
            &mut world,
            super::super::fixtures::PLACE,
            7,
            (3200, 3200),
            820,
        );
        (world, inbox)
    }

    fn flag(world: &GameState, name: &[u8]) -> i32 {
        world
            .quests
            .as_ref()
            .expect("quests loaded")
            .manager()
            .event_flag(name)
    }

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// The line's parse: the three-line usage, the case-insensitive start and
    /// stop through the flag bridge, and the silent flush and unknown word.
    #[test]
    fn the_line_parses_and_the_flag_runs_enable() {
        let clock = TestClock::default();
        let (mut world, mut caller) = boss_world(&clock);
        drained(&mut caller);
        world.run_worldboss_line(Vid::new(7), b"", mover());
        let lines = drained(&mut caller);
        assert_eq!(lines.len(), 3, "the usage speaks three lines");
        assert_eq!(text_of(&lines[0]), b"Usage: worldboss start");
        assert_eq!(text_of(&lines[1]), b"       worldboss stop");
        assert_eq!(text_of(&lines[2]), b"       worldboss flush");

        world.run_worldboss_line(Vid::new(7), b"START", mover());
        assert_eq!(flag(&world, b"world_boss_event"), 1);
        assert_eq!(flag(&world, b"world_boss_state"), 4, "the cooldown lands");
        let lines = drained(&mut caller);
        assert!(
            lines
                .iter()
                .any(|line| text_of(line) == b"[World boss] Event started"),
            "the start notice: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line[0] == 4 && text_of(line) == b"worldboss 1"),
            "the command line: {lines:?}"
        );

        world.run_worldboss_line(Vid::new(7), b"bogus", mover());
        assert!(drained(&mut caller).is_empty(), "an unknown word is silent");
        world.run_worldboss_line(Vid::new(7), b"flush", mover());
        assert!(drained(&mut caller).is_empty(), "the flush is silent");

        world.run_worldboss_line(Vid::new(7), b"Stop", mover());
        assert_eq!(flag(&world, b"world_boss_event"), 0);
        let lines = drained(&mut caller);
        assert!(
            lines
                .iter()
                .any(|line| text_of(line) == b"[World boss] Event completed"),
            "the completion notice: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line[0] == 4 && text_of(line) == b"worldboss 0"),
            "the stop command line: {lines:?}"
        );
    }

    /// The timers: the owner's 1-second cooldown runs the run time, which arms
    /// the combat seconds and the 5-second spawn and speaks both notices; the
    /// combat count rises a second at a time.
    #[test]
    fn the_cooldown_runs_the_run_time_and_the_seconds_count() {
        let clock = TestClock::default();
        let (mut world, mut caller) = boss_world(&clock);
        drained(&mut caller);
        world.run_worldboss_line(Vid::new(7), b"start", mover());
        drained(&mut caller);
        let due = world.world_boss_cooldown_due.expect("the cooldown armed");
        assert_eq!(due, 25, "the owner's 1-second cooldown");
        world.last_pulse = due;
        world.run_world_boss_tick(due);
        assert_eq!(world.world_boss_state, BossState::WaitForSpawn);
        assert_eq!(flag(&world, b"world_boss_state"), 1);
        let lines = drained(&mut caller);
        assert!(
            lines
                .iter()
                .any(|line| text_of(line)
                    == b"[World boss] Caution: the world boss will appear soon!"),
            "the coming-soon notice: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| text_of(line)
                    == b"[Test World boss] The boss will appear in 5 secconds."),
            "the 5-second test notice: {lines:?}"
        );
        let spawn_due = world.world_boss_spawn_due.expect("the spawn armed");
        assert_eq!(spawn_due, due + 125, "five seconds on");
        let combat_due = world.world_boss_combat_due.expect("the combat armed");
        world.last_pulse = combat_due;
        world.run_world_boss_tick(combat_due);
        assert_eq!(world.world_boss_passed, 1);
        assert!(
            world
                .world_boss_combat_due
                .is_some_and(|next| next > combat_due),
            "the next second arms"
        );
    }

    /// The combat arms: at half the duration a spawned boss escapes (the state
    /// lands in `KILLED` and the escape speaks), and past the duration the
    /// cooldown starts again.
    #[test]
    fn half_duration_escapes_a_spawned_boss_and_the_end_cools_down() {
        let clock = TestClock::default();
        let (mut world, mut caller) = boss_world(&clock);
        drained(&mut caller);
        world.world_boss_set_runtime(100);
        drained(&mut caller);
        world.world_boss_marks.spawned = true;
        world.world_boss_vid = 900;
        world.world_boss_passed = 50;
        let due = world.world_boss_combat_due.expect("the combat armed");
        world.run_world_boss_tick(due);
        assert_eq!(world.world_boss_state, BossState::Killed);
        assert_eq!(flag(&world, b"world_boss_state"), 3);
        let lines = drained(&mut caller);
        assert!(
            lines.iter().any(|line| text_of(line)
                == b"[World boss] The World boss has escaped! Good luck next time."),
            "the escape notice: {lines:?}"
        );
        world.world_boss_passed = 101;
        let due = world.world_boss_combat_due.expect("the combat still armed");
        world.run_world_boss_tick(due);
        assert_eq!(world.world_boss_state, BossState::BreakTime);
        assert!(
            world.world_boss_cooldown_due.is_some(),
            "the cooldown re-arms"
        );
    }

    /// The spawn refuses with no channel-1 copy of the map, and stands the
    /// boss on one: the fields land, the spawn notice speaks, the temp ranking
    /// clears and the state flag reads `SPAWNED`.
    #[test]
    fn the_spawn_needs_a_channel_one_map_and_stands_the_boss() {
        let clock = TestClock::default();
        let (mut world, mut caller) = boss_world(&clock);
        drained(&mut caller);
        assert!(!world.world_boss_spawn(), "map 61 hosts nowhere");
        assert!(!world.world_boss_marks.spawned);

        // Host the owner's map on channel 1 with open cells and the real mob
        // protos, so the random stand lands.
        let region = gamedata::map_atlas::MapRegion {
            index: 61,
            name: b"map_n_snowm_01".to_vec(),
            sx: 0,
            sy: 0,
            ex: 6_400,
            ey: 6_400,
            spawn: (0, 0),
            empire_spawns: None,
        };
        world.schedule_map((1, 61), &region);
        let block = gamedata::server_attr::CellBlock::from_cells(
            &vec![0; gamedata::server_attr::SECTREE_CELLS * gamedata::server_attr::SECTREE_CELLS],
        );
        let attr = gamedata::server_attr::ServerAttr::from_blocks(1, 1, vec![block]).unwrap();
        let grid = gamedata::server_attr::SectreeGrid::of(&region).unwrap();
        let mut hosted = world::cells::HostedCells::default();
        hosted.insert(
            61,
            world::cells::MapCells::new(grid, std::sync::Arc::new(attr)),
        );
        let proto_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        let respawner = world::npc::Respawner::new(
            std::sync::Arc::new(
                gamedata::mob_proto::MobProtos::load(&proto_dir).expect("the mob protos load"),
            ),
            std::sync::Arc::new(gamedata::mob_group::MobGroups::default()),
            std::sync::Arc::new(
                gamedata::mob_locale_names::MobLocaleNames::parse(b"VNUM\tNAME\n").unwrap(),
            ),
            world::npc::NpcVids::starting_at(100_000),
        );
        world.install_respawner(respawner, std::sync::Arc::new(hosted));
        assert!(world.world_boss_spawn(), "the boss stands on map 61");
        assert!(world.world_boss_marks.spawned);
        assert_eq!(world.world_boss_map, 61);
        assert_eq!(world.world_boss_vnum, 6418);
        assert_eq!(world.world_boss_state, BossState::Spawned);
        assert_eq!(flag(&world, b"world_boss_state"), 2);
        let stood = world
            .npc_by_vid(world.world_boss_vid)
            .expect("the boss stands");
        assert_eq!(stood.vnum, 6418);
        let lines = drained(&mut caller);
        assert!(
            lines
                .iter()
                .any(|line| text_of(line).starts_with(b"[World boss] In Channel 1, ")),
            "the spawn notice: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| text_of(line).starts_with(b"[World Boss Test] ")),
            "the test-server coordinates: {lines:?}"
        );
        assert!(!world.world_boss_spawn(), "no second boss while one stands");
    }

    /// The kill: the state lands, the notice speaks, every connected PC's
    /// reward flag clears, and the hunters on the boss's map past the bars
    /// rank.
    #[test]
    fn the_kill_ranks_the_hunters_and_clears_their_flags() {
        let clock = TestClock::default();
        let (mut world, mut caller) = boss_world(&clock);
        drained(&mut caller);
        world.world_boss_table = std::sync::Arc::new(gamedata::world_boss::WorldBossTable {
            config: gamedata::world_boss::WorldBossConfig {
                cooldown: 1,
                runtime: 100,
                min_damage: 0,
                min_level: 1,
            },
            maps: vec![41],
            bosses: vec![101],
            ranking: vec![100],
        });
        world.world_boss_map = 41;
        world.world_boss_vid = 900;
        world.world_boss_marks.spawned = true;
        world.world_boss_state = BossState::Spawned;
        world.npc_combat.entry(900).or_default().damage.insert(
            7,
            super::super::fight::NpcDamage {
                total: 500,
                aggro: 0,
            },
        );
        let pid = world
            .characters
            .find_by_vid(Vid::new(7))
            .expect("the caller stands")
            .player_id();
        if let Some(quests) = world.quests.as_mut() {
            quests
                .manager_mut()
                .set_quest_flag(pid, b"world_boss_reward", 1);
        }
        world.world_boss_kill(b"Boss");
        assert_eq!(world.world_boss_state, BossState::Killed);
        assert!(!world.world_boss_marks.spawned);
        assert_eq!(flag(&world, b"world_boss_state"), 3);
        assert_eq!(world.world_boss_ranking.len(), 1);
        assert_eq!(world.world_boss_ranking[0].pid, pid);
        assert_eq!(world.world_boss_ranking[0].record, 500);
        assert!(
            world
                .quests
                .as_ref()
                .expect("quests loaded")
                .manager()
                .quest_flag(pid, b"world_boss_reward")
                .is_none_or(|flag| flag == 0),
            "the reward flag clears"
        );
        let lines = drained(&mut caller);
        assert!(
            lines.iter().any(|line| text_of(line)
                == b"[World boss] Well done: Boss has been defeated and peace has been restored... for now."),
            "the kill notice: {lines:?}"
        );
    }

    /// The damage ask: the first hit speaks, then the asker's total answers;
    /// later hits only answer.
    #[test]
    fn the_first_hit_speaks_and_every_hit_answers_its_damage() {
        let clock = TestClock::default();
        let (mut world, mut caller) = boss_world(&clock);
        drained(&mut caller);
        world.world_boss_vid = 900;
        world.npc_combat.entry(900).or_default().damage.insert(
            7,
            super::super::fight::NpcDamage {
                total: 500,
                aggro: 0,
            },
        );
        world.world_boss_damaged(Vid::new(7));
        let lines = drained(&mut caller);
        assert_eq!(lines.len(), 2, "the notice and the damage answer");
        assert!(
            text_of(&lines[0])
                == b"[World boss] The world boss in Channel 1 has been attacked. Join the hunt now!",
            "the first-hit notice: {:?}",
            text_of(&lines[0])
        );
        assert_eq!(&lines[1][0..4], &[148, 8, 0, 1]);
        assert_eq!(
            protocol::gc_world_boss::decode_damage(&lines[1][4..]),
            Some(500)
        );
        world.world_boss_damaged(Vid::new(7));
        let lines = drained(&mut caller);
        assert_eq!(lines.len(), 1, "later hits only answer");
        assert_eq!(
            protocol::gc_world_boss::decode_damage(&lines[0][4..]),
            Some(500)
        );
    }

    /// The info ask answers the 24-byte record: the state, the seconds, the
    /// damage and the minimum.
    #[test]
    fn the_info_ask_answers_the_state_and_the_damage() {
        let clock = TestClock::default();
        let (mut world, mut caller) = boss_world(&clock);
        drained(&mut caller);
        world.world_boss_vid = 900;
        world.world_boss_state = BossState::Spawned;
        world.npc_combat.entry(900).or_default().damage.insert(
            7,
            super::super::fight::NpcDamage {
                total: 500,
                aggro: 0,
            },
        );
        world.world_boss_info(Vid::new(7));
        let lines = drained(&mut caller);
        assert_eq!(lines.len(), 1);
        assert_eq!(&lines[0][0..4], &[148, 24, 0, 0]);
        let body = protocol::gc_world_boss::decode_process(&lines[0][4..]).expect("the body");
        assert_eq!(body.state, 2);
        assert_eq!(body.total_damage, 500);
        assert_eq!(body.min_damage, 250_000, "the owner's minimum");
    }

    /// The reward gates: a spawned boss refuses with the wait line, any other
    /// unranked state refuses silently, and a ranked hunter with no group
    /// finds nothing to draw.
    #[test]
    fn the_reward_refuses_while_spawned_and_without_a_group() {
        let clock = TestClock::default();
        let (mut world, mut caller) = boss_world(&clock);
        drained(&mut caller);
        world.world_boss_state = BossState::Spawned;
        assert!(world.world_boss_reward(Vid::new(7)).is_none());
        let lines = drained(&mut caller);
        assert_eq!(lines.len(), 1);
        assert_eq!(
            text_of(&lines[0]),
            b"[World boss] You can't receive a reward now."
        );
        world.world_boss_state = BossState::None;
        assert!(world.world_boss_reward(Vid::new(7)).is_none());
        assert!(drained(&mut caller).is_empty(), "other states stay silent");

        world.world_boss_table = std::sync::Arc::new(gamedata::world_boss::WorldBossTable {
            config: gamedata::world_boss::WorldBossConfig {
                cooldown: 1,
                runtime: 100,
                min_damage: 0,
                min_level: 1,
            },
            maps: vec![41],
            bosses: vec![101],
            ranking: vec![100],
        });
        world.world_boss_state = BossState::Killed;
        world.world_boss_vid = 900;
        world.npc_combat.entry(900).or_default().damage.insert(
            7,
            super::super::fight::NpcDamage {
                total: 500,
                aggro: 0,
            },
        );
        let pid = world
            .characters
            .find_by_vid(Vid::new(7))
            .expect("the caller stands")
            .player_id();
        world.world_boss_ranking.push(BossRankRow {
            pid,
            player: [0; protocol::gc_world_boss::PLAYER_NAME_LEN],
            guild: [0; protocol::gc_world_boss::GUILD_NAME_LEN],
            empire: 1,
            record: 500,
        });
        assert!(world.world_boss_reward(Vid::new(7)).is_none());
        assert!(
            drained(&mut caller).is_empty(),
            "group 11001 is absent, so the draw refuses silently"
        );
    }

    /// The 3-second throttle binds a player on a live server: the second ask
    /// in the window stays silent.
    #[test]
    fn the_throttle_binds_a_player_on_a_live_server() {
        let clock = TestClock::default();
        let (mut world, mut caller) = boss_world(&clock);
        world.test_server = false;
        drained(&mut caller);
        world.world_boss_ranking_of(Vid::new(7));
        assert_eq!(drained(&mut caller).len(), 1, "the ranking answers");
        world.world_boss_ranking_of(Vid::new(7));
        assert!(drained(&mut caller).is_empty(), "the second ask waits");
    }
}
