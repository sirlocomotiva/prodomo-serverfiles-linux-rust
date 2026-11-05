//! The rider's horse in the world: the enter-game horse lines of `CInputLogin::Entergame`
//! (`EnterHorse`, `G/input_login.cpp:647-650`, and the `__FIX_NIVEL_CAL__` refresh,
//! `:887-897`), the two stamina events (`G/horse_rider.cpp:220-283`), the `horse_state`
//! lines `SendHorseInfo` sends, and the GM lines `/horse_state`, `/horse_level` and
//! `/horse_set_stat` (`G/cmd_gm.cpp:3745-3833`). The horse's own rules are
//! [`world::character::Horse`].
//!
//! On entering, a character with a horse runs `EnterHorse`: a living horse starts its
//! regeneration and takes the health drops that came due while away. The owner's build then
//! runs `SetHorseLevel(GetHorseLevel())` (`__FIX_NIVEL_CAL__`, `C/prodomodefines.h:95`), which
//! refills both pools, moves the drop time three days on and plants the horse skill (130) at
//! the horse level, and sends byte 76: every login with a horse finds it fed and rested.
//!
//! `/horse_level <name> <level>` clamps the level to 0..30, tells the caller, and runs the
//! same `SetHorseLevel` on the named character, then `ComputePoints` and byte 76.
//! `/horse_set_stat <hp> <stamina>` moves the caller's horse through the two clamping updates,
//! health first, so a health of 0 kills the horse and empties its stamina before the stamina
//! lands. `/horse_state` prints the level and both pools with their percentages.
//!
//! # Not ported
//!
//! - `HEADER_GD_REQ_HORSE_NAME` at entry (`G/input_login.cpp:891-892`): the horse names wait
//!   for the name slice.

use common::vid::Vid;
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};
use world::character::{HorseLine as Said, HorseTimer};

use super::skill_admin::{str_to_wrapping_int, SetSkillLanded, SetSkillOtherLanded};
use super::GameState;
use crate::chat_line::Arg;
use crate::game_loop_messages::{GameCommand, HorseLine};
use crate::save::PASSES_PER_SEC;

/// `SKILL_HORSE` (`G/skill.h:152`): the skill `SetHorseLevel` plants at the horse level.
pub(super) const SKILL_HORSE: u8 = 130;

impl GameState {
    /// Run one GM horse line ([`GameCommand::HorseLine`]) and answer what `/horse_level`
    /// planted, for the store.
    pub(super) fn apply_horse_line(&mut self, command: GameCommand) {
        let GameCommand::HorseLine {
            vid,
            line,
            rest,
            reply,
        } = command
        else {
            return;
        };
        let rest = rest.as_slice();
        let landed = match line {
            HorseLine::State => {
                self.run_horse_state(vid);
                None
            }
            HorseLine::Level => self.run_horse_level(vid, rest),
            HorseLine::SetStat => {
                self.run_horse_set_stat(vid, rest);
                None
            }
            HorseLine::Summon => {
                self.horse_summon(vid, true, 0);
                None
            }
            HorseLine::Unsummon => {
                self.horse_unsummon(vid, true);
                None
            }
            HorseLine::Back => {
                self.user_horse_back(vid);
                None
            }
            HorseLine::Ride => {
                self.run_horse_ride(vid);
                None
            }
            HorseLine::UserRide => {
                self.user_horse_ride(vid);
                None
            }
            // `do_unmount` (`G/cmd_general.cpp:2313-2368`): no mount costume is ever worn
            // (each carries the first-use timer the use refuses) and no unique is, so
            // `UnEquipSpecialRideUniqueItem` answers true and the line is `pc.unmount`'s.
            HorseLine::Unmount => {
                self.unmount(vid);
                None
            }
        };
        if reply.send(landed).is_err() {
            tracing::debug!(?vid, "nobody was left to hear a horse line's answer");
        }
    }

    /// The enter-game horse lines: `EnterHorse`, then the `__FIX_NIVEL_CAL__` refresh with its
    /// byte 76, for a character with a horse. The refresh's horse skill is not stored: every
    /// login plants it again from the stored horse level.
    pub(super) fn enter_horse(&mut self, vid: Vid) {
        let now = Self::global_time();
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        let horse = points.horse_mut();
        let level = horse.level();
        if level == 0 {
            return;
        }
        if horse.enter(now) {
            // A row stored while ridden mounts again (`StartRiding`, `G/horse_rider.
            // cpp:96-101`), past the FIX refresh like `EnterHorse` runs it.
            self.start_riding(vid);
        }
        self.sync_horse_timer(vid);
        let _planted = self.plant_horse_level(vid, i32::from(level));
        self.send_skill_levels(vid);
    }

    /// `CHARACTER::SetHorseLevel` (`G/char_horse.cpp:396-400`): the horse takes the level and
    /// the horse skill follows it (`SetSkillLevel(SKILL_HORSE, GetHorseLevel())`). Answers the
    /// planted skill for the store; `None` without points.
    pub(super) fn plant_horse_level(&mut self, vid: Vid, level: i32) -> Option<SetSkillLanded> {
        let now = Self::global_time();
        let horse = self.pc_points_mut(vid.raw())?.horse_mut();
        horse.set_level(level, now);
        let landed = horse.level();
        self.flush_horse(vid);
        let state = self.pc_skills.entry(vid.raw()).or_default();
        state.levels[usize::from(SKILL_HORSE)] = landed;
        Some(SetSkillLanded {
            vnum: SKILL_HORSE,
            level: landed,
            next_read: state.next_read[usize::from(SKILL_HORSE)],
        })
    }

    /// `do_horse_state` (`G/cmd_gm.cpp:3745-3751`): four lines on the caller's horse. The
    /// percentages divide by the row's maxima, which are 1 without a horse.
    fn run_horse_state(&mut self, vid: Vid) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        let horse = points.horse();
        let (health, max_health) = (i64::from(horse.health()), i64::from(horse.max_health()));
        let (stamina, max_stamina) = (i64::from(horse.stamina()), i64::from(horse.max_stamina()));
        let level = i64::from(horse.level());
        self.tell(vid, mover, CHAT_TYPE_INFO, b"Horse Information:", &[]);
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"    Level  %d",
            &[Arg::Int(level)],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"    Health %d/%d (%d%%)",
            &[
                Arg::Int(health),
                Arg::Int(max_health),
                Arg::Int(health * 100 / max_health),
            ],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"    Stam   %d/%d (%d%%)",
            &[
                Arg::Int(stamina),
                Arg::Int(max_stamina),
                Arg::Int(stamina * 100 / max_stamina),
            ],
        );
    }

    /// `do_horse_level` (`G/cmd_gm.cpp:3753-3798`): the usage for a missing word, `This
    /// character does not exist.` for a name no one online carries, else the caller hears the
    /// clamped level and the named character's horse takes it, with `ComputePoints` and byte 76
    /// on its client.
    fn run_horse_level(&mut self, vid: Vid, rest: &[u8]) -> Option<SetSkillOtherLanded> {
        let mover = self.own_mover(vid)?;
        let (first, back) = crate::command::one_argument(rest);
        let (second, _) = crate::command::one_argument(back);
        if first.is_empty() || second.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"usage : /horse_level <name> <level>",
                &[],
            );
            return None;
        }
        let victim = std::str::from_utf8(&first)
            .ok()
            .and_then(|name| self.whisper_target(name));
        let Some(victim) = victim else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"This character does not exist.",
                &[],
            );
            return None;
        };
        let level = str_to_wrapping_int(&second).clamp(0, 30);
        let shown = self
            .characters
            .find_by_vid(victim)
            .map_or_else(|_| String::new(), |found| found.name().to_string());
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"horse level set (%s: %d)",
            &[Arg::Text(shown.as_bytes()), Arg::Int(i64::from(level))],
        );
        let set = self.plant_horse_level(victim, level)?;
        self.sync_horse_named(victim);
        let computed = self.pc_points_mut(victim.raw())?.compute_points();
        self.send_point_records(victim, &computed);
        self.send_skill_levels(victim);
        Some(SetSkillOtherLanded { target: shown, set })
    }

    /// `do_horse_set_stat` (`G/cmd_gm.cpp:3815-3833`): both words move the caller's horse
    /// through its two updates; a missing word answers the usage.
    fn run_horse_set_stat(&mut self, vid: Vid, rest: &[u8]) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let (first, back) = crate::command::one_argument(rest);
        let (second, _) = crate::command::one_argument(back);
        if first.is_empty() || second.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Usage : /horse_set_stat <hp> <stamina>",
                &[],
            );
            return;
        }
        let (health, stamina) = (str_to_wrapping_int(&first), str_to_wrapping_int(&second));
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        points.horse_mut().set_stat(health, stamina);
        self.settle_horse(vid);
    }

    /// After a step that moved the horse: its stamina event follows; a ride the update
    /// ended finishes stopping, a death unsummons the summoned horse
    /// (`CHARACTER::HorseDie`, `G/char_horse.cpp:300-305`), and the queued lines go out.
    pub(super) fn settle_horse(&mut self, vid: Vid) {
        self.sync_horse_timer(vid);
        let stopped = self
            .pc_points_mut(vid.raw())
            .is_some_and(|points| points.horse_mut().take_stopped());
        if stopped {
            self.finish_riding_stop(vid);
        }
        let died = self
            .pc_points_mut(vid.raw())
            .is_some_and(|points| points.horse_mut().take_died());
        if died {
            self.horse_unsummon(vid, false);
        }
        self.flush_horse(vid);
    }

    /// The manager's answer for the rider before a compute: `true` when it holds a
    /// non-empty name (`G/char.cpp:2828-2832`).
    pub(super) fn sync_horse_named(&mut self, vid: Vid) {
        let named = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|rider| self.horse_names.get(&rider.player_id()))
            .is_some_and(|name| !name.is_empty());
        if let Some(points) = self.pc_points_mut(vid.raw()) {
            points.horse_mut().set_named(named);
        }
    }

    /// Schedule the horse's stamina event after a step that may have started, switched or
    /// ended it: a fresh event fires one period from now, a running one keeps its Pulse, and a
    /// horse with none leaves the schedule.
    pub(super) fn sync_horse_timer(&mut self, vid: Vid) {
        let pulse = self.last_pulse;
        let Some(horse) = self
            .pc_points_mut(vid.raw())
            .map(world::character::Points::horse_mut)
        else {
            self.horse_timers.remove(&vid);
            return;
        };
        let timer = horse.timer();
        let fresh = horse.take_new_timer();
        if timer == HorseTimer::None {
            self.horse_timers.remove(&vid);
        } else if fresh || !self.horse_timers.contains_key(&vid) {
            let period = u64::from(timer.interval_secs()) * u64::from(PASSES_PER_SEC);
            self.horse_timers.insert(vid, pulse + period);
        }
    }

    /// The horse events due at `pulse` (`horse_stamina_regen_event`,
    /// `horse_stamina_consume_event`): each fires, and runs on one period later unless it
    /// ended or handed over to the other.
    pub(super) fn run_horse_timers(&mut self, pulse: u64) {
        let due: Vec<Vid> = self
            .horse_timers
            .iter()
            .filter_map(|(vid, due)| (pulse >= *due).then_some(*vid))
            .collect();
        let now = Self::global_time();
        for vid in due {
            let Some(horse) = self
                .pc_points_mut(vid.raw())
                .map(world::character::Points::horse_mut)
            else {
                self.horse_timers.remove(&vid);
                continue;
            };
            let timer = horse.timer();
            let runs_on = match timer {
                HorseTimer::Regen => horse.run_regen(now),
                HorseTimer::Consume => horse.run_consume(now),
                HorseTimer::None => false,
            };
            if runs_on {
                let period = u64::from(timer.interval_secs()) * u64::from(PASSES_PER_SEC);
                self.horse_timers.insert(vid, pulse + period);
            }
            self.settle_horse(vid);
        }
    }

    /// Send the command lines the horse queued: `horse_state %d %d %d` (`SendHorseInfo`,
    /// `G/char_horse.cpp:369-376`) and `hide_horse_state` (`ClearHorseInfo`, `:315-326`).
    pub(super) fn flush_horse(&mut self, vid: Vid) {
        let Some(lines) = self
            .pc_points_mut(vid.raw())
            .map(|points| points.horse_mut().take_info_lines())
        else {
            return;
        };
        if lines.is_empty() {
            return;
        }
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        for line in lines {
            match line {
                Said::State([level, health, stamina]) => self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_COMMAND,
                    b"horse_state %d %d %d",
                    &[
                        Arg::Int(i64::from(level)),
                        Arg::Int(i64::from(health)),
                        Arg::Int(i64::from(stamina)),
                    ],
                ),
                Said::Hide => self.tell(vid, mover, CHAT_TYPE_COMMAND, b"hide_horse_state", &[]),
            }
        }
    }
}
