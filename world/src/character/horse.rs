//! The rider's horse: `CHorseRider` (`G/horse_rider.cpp`, `G/horse_rider.h`) and the
//! `horse_state` line of `CHARACTER::SendHorseInfo` (`G/char_horse.cpp:330-385`).
//!
//! The horse is a level from 0 to 30, a health, a stamina, the time its health next drops,
//! and whether it is ridden. A level reads its row of `c_aHorseStat`: the maxima, the race the
//! summoned horse spawns as, and the bonuses a rider takes. `SetHorseLevel` refills both pools
//! and moves the drop time three days on; the health drops by one each time that passes, and a
//! horse whose health reaches 0 dies, which empties its stamina. Feeding adds one health and
//! moves the drop time on; reviving a dead horse refills it.
//!
//! Two events move the stamina: one point every six minutes while ridden, one point back every
//! twelve minutes while not. At most one runs; starting either cancels the other, a dead horse
//! starts neither, and a death cancels both. The horse does not own a clock: it records which
//! event runs ([`Horse::timer`]) and when one was created afresh ([`Horse::take_new_timer`]),
//! and the world schedules it in Pulses.
//!
//! `SendHorseInfo` speaks only while the horse is summoned or ridden, and only when the level
//! or one of the two grades moved since the last line it sent. The horse queues those lines
//! ([`Horse::take_info_lines`]) for the world to send as `horse_state` command chat.

/// `HORSE_MAX_LEVEL` (`G/horse_rider.h:11`).
pub const HORSE_MAX_LEVEL: u8 = 30;
/// `HORSE_HEALTH_DROP_INTERVAL` (`G/horse_rider.cpp:8`): three days, in seconds.
pub const HORSE_HEALTH_DROP_INTERVAL: u32 = 3 * 24 * 60 * 60;
/// `HORSE_STAMINA_CONSUME_INTERVAL` (`G/horse_rider.cpp:9`): six minutes, in seconds.
pub const HORSE_STAMINA_CONSUME_INTERVAL: u32 = 6 * 60;
/// `HORSE_STAMINA_REGEN_INTERVAL` (`G/horse_rider.cpp:10`): twelve minutes, in seconds.
pub const HORSE_STAMINA_REGEN_INTERVAL: u32 = 12 * 60;

/// One row of `c_aHorseStat` (`THorseStat`, `G/horse_rider.h:13-27`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HorseStat {
    /// `iMinLevel`: the rider level the row names.
    pub min_level: u8,
    /// `iNPCRace`: the race the summoned horse spawns as.
    pub npc_race: u32,
    /// `iMaxHealth`.
    pub max_health: i16,
    /// `iMaxStamina`.
    pub max_stamina: i16,
    /// `iST`, the strength a rider takes.
    pub st: i32,
    /// `iDX`.
    pub dx: i32,
    /// `iHT`.
    pub ht: i32,
    /// `iIQ`.
    pub iq: i32,
    /// `iDamMean`.
    pub dam_mean: i32,
    /// `iDamMin`.
    pub dam_min: i32,
    /// `iDamMax`.
    pub dam_max: i32,
    /// `iArmor`.
    pub armor: i32,
}

/// One row, in the source's column order.
const fn row(
    min_level: u8,
    npc_race: u32,
    max_health: i16,
    max_stamina: i16,
    stats: [i32; 4],
    damage: [i32; 3],
    armor: i32,
) -> HorseStat {
    HorseStat {
        min_level,
        npc_race,
        max_health,
        max_stamina,
        st: stats[0],
        dx: stats[1],
        ht: stats[2],
        iq: stats[3],
        dam_mean: damage[0],
        dam_min: damage[1],
        dam_max: damage[2],
        armor,
    }
}

/// `c_aHorseStat` (`G/horse_rider.cpp:15-63`), indexed by the horse level.
pub const HORSE_STATS: [HorseStat; HORSE_MAX_LEVEL as usize + 1] = [
    row(0, 0, 1, 1, [0, 0, 0, 0], [0, 0, 0], 0),
    row(25, 20101, 3, 4, [26, 35, 18, 9], [54, 43, 64], 32),
    row(25, 20101, 4, 4, [27, 36, 18, 9], [55, 44, 66], 33),
    row(25, 20101, 5, 5, [28, 38, 19, 9], [56, 44, 67], 33),
    row(25, 20101, 7, 5, [29, 39, 19, 10], [57, 45, 68], 34),
    row(25, 20101, 8, 6, [30, 40, 20, 10], [58, 46, 69], 34),
    row(25, 20101, 9, 6, [31, 41, 21, 10], [59, 47, 70], 35),
    row(25, 20101, 11, 7, [32, 42, 21, 11], [60, 48, 72], 36),
    row(25, 20101, 12, 7, [33, 44, 22, 11], [61, 48, 73], 36),
    row(25, 20101, 13, 8, [34, 45, 22, 11], [62, 49, 74], 37),
    row(25, 20101, 15, 10, [35, 46, 23, 12], [63, 50, 75], 37),
    row(35, 20104, 18, 30, [40, 53, 27, 13], [69, 55, 82], 41),
    row(35, 20104, 19, 35, [41, 54, 27, 14], [70, 56, 84], 42),
    row(35, 20104, 21, 40, [42, 56, 28, 14], [71, 56, 85], 42),
    row(35, 20104, 22, 50, [43, 57, 28, 14], [72, 57, 86], 43),
    row(35, 20104, 24, 55, [44, 58, 29, 15], [73, 58, 87], 43),
    row(35, 20104, 25, 60, [44, 59, 30, 15], [74, 59, 88], 44),
    row(35, 20104, 27, 65, [45, 60, 30, 15], [75, 60, 90], 45),
    row(35, 20104, 28, 70, [46, 62, 31, 15], [76, 60, 91], 45),
    row(35, 20104, 30, 80, [47, 63, 31, 16], [77, 61, 92], 46),
    row(35, 20104, 32, 100, [48, 64, 32, 16], [78, 62, 93], 46),
    row(50, 20107, 35, 120, [53, 71, 36, 18], [84, 67, 100], 50),
    row(50, 20107, 36, 125, [55, 74, 37, 18], [86, 68, 103], 51),
    row(50, 20107, 37, 130, [57, 76, 38, 19], [88, 70, 105], 52),
    row(50, 20107, 38, 135, [59, 78, 39, 20], [90, 72, 108], 54),
    row(50, 20107, 40, 140, [60, 80, 40, 20], [91, 72, 109], 54),
    row(50, 20107, 42, 145, [61, 81, 40, 20], [92, 73, 110], 55),
    row(50, 20107, 44, 150, [62, 83, 42, 21], [94, 75, 112], 56),
    row(50, 20107, 46, 160, [63, 84, 42, 21], [95, 76, 114], 57),
    row(50, 20107, 48, 170, [65, 87, 43, 22], [97, 77, 116], 58),
    row(50, 20107, 50, 200, [67, 89, 45, 22], [99, 79, 118], 59),
];

/// What the store keeps of a horse (`THorseInfo`, `C/tables.h:473-480`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HorseInfo {
    /// `bLevel`, 0 to 30.
    pub level: u8,
    /// `bRiding`.
    pub riding: bool,
    /// `sStamina`.
    pub stamina: i16,
    /// `sHealth`.
    pub health: i16,
    /// `dwHorseHealthDropTime`: the `get_global_time()` the health next drops at.
    pub health_drop_time: u32,
}

/// Which stamina event runs (`m_eventStaminaRegen`, `m_eventStaminaConsume`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HorseTimer {
    /// Neither runs.
    #[default]
    None,
    /// `horse_stamina_regen_event`, every [`HORSE_STAMINA_REGEN_INTERVAL`] seconds.
    Regen,
    /// `horse_stamina_consume_event`, every [`HORSE_STAMINA_CONSUME_INTERVAL`] seconds.
    Consume,
}

impl HorseTimer {
    /// The event's period in seconds; 0 for none.
    #[must_use]
    pub const fn interval_secs(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Regen => HORSE_STAMINA_REGEN_INTERVAL,
            Self::Consume => HORSE_STAMINA_CONSUME_INTERVAL,
        }
    }
}

/// One `horse_state %d %d %d` line: the level, the health grade and the stamina grade.
pub type HorseInfoLine = [u8; 3];

/// A character's horse (`CHorseRider`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Horse {
    info: HorseInfo,
    /// `m_chHorse != NULL`: whether the horse stands summoned. Not stored.
    summoned: bool,
    timer: HorseTimer,
    /// Whether [`Horse::timer`] was created since the world last scheduled it.
    new_timer: bool,
    /// `m_bSendHorseLevel`, `m_bSendHorseHealthGrade`, `m_bSendHorseStaminaGrade`
    /// (`G/char.cpp:364-366`): what the last `horse_state` line said.
    sent: HorseInfoLine,
    info_lines: Vec<HorseInfoLine>,
}

impl Horse {
    /// `SetHorseData` (`G/horse_rider.cpp:150-153`): the stored row, copied whole.
    #[must_use]
    pub fn from_info(info: HorseInfo) -> Self {
        Self {
            info,
            ..Self::default()
        }
    }

    /// `GetHorseData`: what the save writes.
    #[must_use]
    pub const fn info(&self) -> HorseInfo {
        self.info
    }

    /// `GetHorseLevel`.
    #[must_use]
    pub const fn level(&self) -> u8 {
        self.info.level
    }

    /// `IsHorseRiding`.
    #[must_use]
    pub const fn riding(&self) -> bool {
        self.info.riding
    }

    /// `GetHorseHealth`.
    #[must_use]
    pub const fn health(&self) -> i16 {
        self.info.health
    }

    /// `GetHorseStamina`.
    #[must_use]
    pub const fn stamina(&self) -> i16 {
        self.info.stamina
    }

    /// Whether the horse stands summoned.
    #[must_use]
    pub const fn summoned(&self) -> bool {
        self.summoned
    }

    /// The level's row of `c_aHorseStat`. The store keeps the level within 0 to 30; a wider
    /// one reads the last row rather than past the table.
    #[must_use]
    pub fn stat(&self) -> &'static HorseStat {
        &HORSE_STATS[usize::from(self.info.level.min(HORSE_MAX_LEVEL))]
    }

    /// `GetHorseMaxHealth`.
    #[must_use]
    pub fn max_health(&self) -> i16 {
        self.stat().max_health
    }

    /// `GetHorseMaxStamina`.
    #[must_use]
    pub fn max_stamina(&self) -> i16 {
        self.stat().max_stamina
    }

    /// `GetHorseGrade` (`G/horse_rider.cpp:378-386`): 0 without a horse, else 1 for levels 1
    /// to 10, 2 for 11 to 20 and 3 for 21 to 30.
    #[must_use]
    pub const fn grade(&self) -> u8 {
        if self.info.level == 0 {
            0
        } else {
            (self.info.level - 1) / 10 + 1
        }
    }

    /// The stamina event that runs.
    #[must_use]
    pub const fn timer(&self) -> HorseTimer {
        self.timer
    }

    /// Whether the running event was created since the last call, which the world answers by
    /// scheduling it one period from now.
    pub fn take_new_timer(&mut self) -> bool {
        std::mem::take(&mut self.new_timer)
    }

    /// The `horse_state` lines queued since the last call, oldest first.
    pub fn take_info_lines(&mut self) -> Vec<HorseInfoLine> {
        std::mem::take(&mut self.info_lines)
    }

    /// `SetHorseLevel` (`G/horse_rider.cpp:364-376`): the level clamps to 0 to 30, both pools
    /// refill to the row's maxima, and the drop time moves three days past `now`.
    pub fn set_level(&mut self, level: i32, now: u32) {
        let level = u8::try_from(level.clamp(0, i32::from(HORSE_MAX_LEVEL))).unwrap_or(0);
        self.info.level = level;
        self.info.stamina = self.max_stamina();
        self.info.health = self.max_health();
        self.reset_health_drop_time(now);
        self.send_info();
    }

    /// `EnterHorse` (`G/horse_rider.cpp:87-106`) without its riding branch: a living horse
    /// starts its regeneration, and the drops that came due while away are taken silently.
    /// Answers whether the stored row said the horse was ridden; the flag is cleared, as
    /// legacy's toggle does before `StartRiding` runs again.
    pub fn enter(&mut self, now: u32) -> bool {
        if self.info.level == 0 || self.info.health <= 0 {
            return false;
        }
        let ridden = self.info.riding;
        if ridden {
            self.info.riding = false;
        } else {
            self.start_regen();
        }
        self.check_health_drop_time(now, false);
        ridden
    }

    /// `UpdateHorseDataByLogoff` (`G/horse_rider.cpp:155-162`): a point of stamina back for
    /// every twelve minutes away, with no line.
    pub fn catch_up_logoff(&mut self, away_secs: u32) {
        if self.info.level == 0 {
            return;
        }
        if away_secs >= HORSE_STAMINA_REGEN_INTERVAL {
            let points = away_secs / HORSE_STAMINA_REGEN_INTERVAL;
            self.update_stamina(i32::try_from(points).unwrap_or(i32::MAX), false);
        }
    }

    /// `ReviveHorse` (`G/horse_rider.cpp:108-127`): a dead horse refills, its drop time moves
    /// on, and its regeneration starts. Answers whether it was dead.
    pub fn revive(&mut self, now: u32) -> bool {
        if self.info.level == 0 || self.info.health > 0 {
            return false;
        }
        self.info.health = self.max_health();
        self.info.stamina = self.max_stamina();
        self.reset_health_drop_time(now);
        self.start_regen();
        true
    }

    /// `FeedHorse` (`G/horse_rider.cpp:141-148`): a living horse gains a point of health and
    /// its drop time moves on. Answers whether it was fed.
    pub fn feed(&mut self, now: u32) -> bool {
        if self.info.level == 0 || self.info.health <= 0 {
            return false;
        }
        self.update_health(1, true);
        self.reset_health_drop_time(now);
        true
    }

    /// `do_horse_set_stat`'s two updates (`G/cmd_gm.cpp:3816-3829`): the health moves by
    /// `health - GetHorseHealth()` and then the stamina by `stamina - GetHorseStamina()`,
    /// each through its clamping update, so a health of 0 kills the horse first.
    pub fn set_stat(&mut self, health: i32, stamina: i32) {
        self.update_health(health.wrapping_sub(i32::from(self.info.health)), true);
        self.update_stamina(stamina.wrapping_sub(i32::from(self.info.stamina)), true);
    }

    /// `CHorseRider::StartRiding` (`G/horse_rider.cpp:172-188`): a living horse with stamina
    /// is mounted, its consumption starts and its line goes out. Answers whether it was.
    pub fn start_riding(&mut self) -> bool {
        if self.info.riding
            || self.info.level == 0
            || self.info.health <= 0
            || self.info.stamina <= 0
        {
            return false;
        }
        self.info.riding = true;
        self.start_consume();
        self.send_info();
        true
    }

    /// `CHorseRider::StopRiding` (`G/horse_rider.cpp:190-198`): the rider gets off and the
    /// regeneration starts. Answers whether it was ridden.
    pub fn stop_riding(&mut self) -> bool {
        if !self.info.riding {
            return false;
        }
        self.info.riding = false;
        self.start_regen();
        true
    }

    /// `horse_stamina_regen_event` (`G/horse_rider.cpp:254-283`) when it fires: a dead horse
    /// ends it; else a point of stamina comes back with its line, the event ends at the
    /// maximum, and the drops that came due are taken. Answers whether it runs on.
    pub fn run_regen(&mut self, now: u32) -> bool {
        if self.info.health <= 0 {
            self.end_timer(HorseTimer::Regen);
            return false;
        }
        self.update_stamina(1, true);
        let runs_on = self.info.stamina != self.max_stamina();
        if !runs_on {
            self.end_timer(HorseTimer::Regen);
        }
        self.check_health_drop_time(now, true);
        runs_on && self.timer == HorseTimer::Regen
    }

    /// `horse_stamina_consume_event` (`G/horse_rider.cpp:220-252`) when it fires: a dead horse
    /// ends it; else a point of stamina goes with its line, the event ends at 0, and the drops
    /// that came due are taken. Answers whether it runs on. `UpdateRideTime` is empty in
    /// `CHorseRider` and `CHARACTER` does not override it.
    pub fn run_consume(&mut self, now: u32) -> bool {
        if self.info.health <= 0 {
            self.end_timer(HorseTimer::Consume);
            return false;
        }
        self.update_stamina(-1, true);
        let runs_on = self.info.stamina != 0;
        if !runs_on {
            self.end_timer(HorseTimer::Consume);
        }
        self.check_health_drop_time(now, true);
        runs_on && self.timer == HorseTimer::Consume
    }

    /// The `horse_state` grades of `SendHorseInfo` (`G/char_horse.cpp:332-367`): health 0 for
    /// a dead horse, 1 up to 30%, 2 up to 70%, else 3; stamina 0 up to 10%, 1 up to 30%, 2 up
    /// to 70%, else 3.
    #[must_use]
    pub fn grades(&self) -> HorseInfoLine {
        let (health, max_health) = (i32::from(self.info.health), i32::from(self.max_health()));
        let (stamina, max_stamina) = (i32::from(self.info.stamina), i32::from(self.max_stamina()));
        let health_grade = if health == 0 {
            0
        } else if health * 10 <= max_health * 3 {
            1
        } else if health * 10 <= max_health * 7 {
            2
        } else {
            3
        };
        let stamina_grade = if stamina * 10 <= max_stamina {
            0
        } else if stamina * 10 <= max_stamina * 3 {
            1
        } else if stamina * 10 <= max_stamina * 7 {
            2
        } else {
            3
        };
        [self.info.level, health_grade, stamina_grade]
    }

    /// `ResetHorseHealthDropTime` (`G/horse_rider.cpp:336-339`).
    fn reset_health_drop_time(&mut self, now: u32) {
        self.info.health_drop_time = now.wrapping_add(HORSE_HEALTH_DROP_INTERVAL);
    }

    /// `CheckHorseHealthDropTime` (`G/horse_rider.cpp:341-350`): one point of health for each
    /// interval that came due, the drop time moving on by one interval each.
    fn check_health_drop_time(&mut self, now: u32, send: bool) {
        while self.info.health_drop_time < now {
            self.info.health_drop_time = self
                .info
                .health_drop_time
                .wrapping_add(HORSE_HEALTH_DROP_INTERVAL);
            self.update_health(-1, send);
        }
    }

    /// `UpdateHorseStamina` (`G/horse_rider.cpp:164-176`): the stamina clamps to 0 and the
    /// row's maximum, and a ridden horse with none left is got off.
    fn update_stamina(&mut self, delta: i32, send: bool) {
        let moved = i32::from(self.info.stamina).wrapping_add(delta);
        let clamped = moved.clamp(0, i32::from(self.max_stamina()));
        self.info.stamina = i16::try_from(clamped).unwrap_or(0);
        if self.info.stamina == 0 && self.info.riding {
            let _stopped = self.stop_riding();
        }
        if send {
            self.send_info();
        }
    }

    /// `UpdateHorseHealth` (`G/horse_rider.cpp:352-364`): the health clamps to 0 and the row's
    /// maximum, and a horse at 0 dies again on every update that leaves it there.
    fn update_health(&mut self, delta: i32, send: bool) {
        let moved = i32::from(self.info.health).wrapping_add(delta);
        let clamped = moved.clamp(0, i32::from(self.max_health()));
        self.info.health = i16::try_from(clamped).unwrap_or(0);
        if self.info.level != 0 && self.info.health == 0 {
            self.die();
        }
        if send {
            self.send_info();
        }
    }

    /// `CHorseRider::HorseDie` (`G/horse_rider.cpp:366-371`): the stamina empties and both
    /// events end. `CHARACTER::HorseDie` unsummons too; the world does that on reading
    /// [`Horse::summoned`].
    fn die(&mut self) {
        self.update_stamina(-i32::from(self.info.stamina), true);
        self.timer = HorseTimer::None;
        self.new_timer = false;
    }

    /// `StartStaminaConsumeEvent` (`G/horse_rider.cpp:285-303`).
    fn start_consume(&mut self) {
        self.start_timer(HorseTimer::Consume);
    }

    /// `StartStaminaRegenEvent` (`G/horse_rider.cpp:305-323`).
    fn start_regen(&mut self) {
        self.start_timer(HorseTimer::Regen);
    }

    /// Either start: nothing for a dead or missing horse; else the other event is cancelled,
    /// and this one is created unless it already runs.
    fn start_timer(&mut self, timer: HorseTimer) {
        if self.info.level == 0 || self.info.health <= 0 {
            return;
        }
        if self.timer != timer {
            self.timer = timer;
            self.new_timer = true;
        }
    }

    /// The event clears its own pointer as it returns 0.
    fn end_timer(&mut self, timer: HorseTimer) {
        if self.timer == timer {
            self.timer = HorseTimer::None;
            self.new_timer = false;
        }
    }

    /// `CHARACTER::SendHorseInfo` (`G/char_horse.cpp:330-385`): while summoned or ridden, a
    /// line when the level or a grade moved.
    fn send_info(&mut self) {
        if !self.summoned && !self.info.riding {
            return;
        }
        let line = self.grades();
        if line != self.sent {
            self.info_lines.push(line);
            self.sent = line;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Horse, HorseInfo, HorseTimer, HORSE_HEALTH_DROP_INTERVAL, HORSE_STAMINA_REGEN_INTERVAL,
    };

    const NOW: u32 = 0x6000_0000;

    fn horse_at(level: i32) -> Horse {
        let mut horse = Horse::default();
        horse.set_level(level, NOW);
        horse
    }

    /// `SetHorseLevel` clamps to 0..=30 and refills to the row; the grade follows the decade.
    #[test]
    fn a_level_clamps_refills_and_grades() {
        let horse = horse_at(25);
        assert_eq!((horse.health(), horse.stamina()), (40, 140));
        assert_eq!(horse.grade(), 3);
        assert_eq!(
            horse.info().health_drop_time,
            NOW + HORSE_HEALTH_DROP_INTERVAL
        );
        assert_eq!(horse_at(99).level(), 30);
        assert_eq!(horse_at(-4).level(), 0);
        assert_eq!((horse_at(10).grade(), horse_at(11).grade()), (1, 2));
        assert_eq!(horse_at(0).grade(), 0);
        assert_eq!(
            (horse_at(0).max_health(), horse_at(0).max_stamina()),
            (1, 1)
        );
    }

    /// `/horse_set_stat`: a health of 0 kills, which empties the stamina before the stamina
    /// update sets it, and ends the regeneration.
    #[test]
    fn a_death_empties_the_stamina_before_the_set_lands() {
        let mut horse = horse_at(21);
        assert!(!horse.enter(NOW));
        assert_eq!(horse.timer(), HorseTimer::Regen);
        horse.set_stat(0, 5);
        assert_eq!((horse.health(), horse.stamina()), (0, 5));
        assert_eq!(horse.timer(), HorseTimer::None);
        assert!(!horse.feed(NOW), "a dead horse eats nothing");
        assert!(horse.revive(NOW));
        assert_eq!((horse.health(), horse.stamina()), (35, 120));
        assert_eq!(horse.timer(), HorseTimer::Regen);
        horse.set_stat(-7, 500);
        assert_eq!((horse.health(), horse.stamina()), (0, 120), "both clamp");
    }

    /// The drops that came due while away are taken one interval at a time, and the logoff
    /// catch-up gives a point per twelve minutes, capped at the row.
    #[test]
    fn the_drops_and_the_logoff_catch_up_follow_the_clock() {
        let mut horse = horse_at(5);
        horse.set_stat(8, 1);
        horse.catch_up_logoff(HORSE_STAMINA_REGEN_INTERVAL * 3 + 5);
        assert_eq!(horse.stamina(), 4);
        horse.catch_up_logoff(HORSE_STAMINA_REGEN_INTERVAL - 1);
        assert_eq!(horse.stamina(), 4);
        let later = NOW + 2 * HORSE_HEALTH_DROP_INTERVAL + 1;
        assert!(!horse.enter(later));
        assert_eq!(horse.health(), 6, "two intervals passed");
        assert_eq!(
            horse.info().health_drop_time,
            NOW + 3 * HORSE_HEALTH_DROP_INTERVAL
        );
        let mut starving = horse_at(1);
        starving.enter(NOW + 4 * HORSE_HEALTH_DROP_INTERVAL);
        assert_eq!((starving.health(), starving.stamina()), (0, 0), "it died");
        assert_eq!(starving.timer(), HorseTimer::None);
    }

    /// The regeneration adds a point and ends at the maximum; the consumption takes a point,
    /// gets the rider off at 0 and hands over to the regeneration.
    #[test]
    fn the_events_move_the_stamina_and_hand_over() {
        let mut horse = horse_at(11);
        horse.set_stat(18, 28);
        assert!(!horse.enter(NOW));
        assert!(horse.take_new_timer());
        assert!(horse.run_regen(NOW));
        assert!(!horse.run_regen(NOW), "29 then 30, the maximum");
        assert_eq!((horse.stamina(), horse.timer()), (30, HorseTimer::None));
        horse.set_stat(18, 1);
        assert!(horse.start_riding());
        assert_eq!(horse.timer(), HorseTimer::Consume);
        assert!(horse.take_new_timer());
        assert!(!horse.run_consume(NOW));
        assert!(!horse.riding(), "no stamina left");
        assert_eq!(horse.timer(), HorseTimer::Regen);
        assert!(horse.take_new_timer());
    }

    /// The line speaks only while ridden or summoned, and only on a move of the level or a
    /// grade.
    #[test]
    fn the_state_line_speaks_only_on_a_change_while_ridden() {
        let mut horse = Horse::from_info(HorseInfo {
            level: 21,
            riding: false,
            stamina: 120,
            health: 35,
            health_drop_time: NOW,
        });
        horse.set_stat(30, 120);
        assert!(horse.take_info_lines().is_empty(), "not ridden");
        assert!(horse.start_riding());
        assert_eq!(horse.take_info_lines(), [[21, 3, 3]]);
        horse.set_stat(29, 120);
        assert!(horse.take_info_lines().is_empty(), "still grade 3");
        horse.set_stat(10, 12);
        assert_eq!(horse.take_info_lines(), [[21, 1, 3], [21, 1, 0]]);
    }
}
