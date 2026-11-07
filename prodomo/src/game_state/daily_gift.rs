//! The daily gifts (`__DAILY_GIFT_SYSTEM__`, `G/char_daily_gifts.cpp`).
//!
//! The running week pays one gift a day from the `daily_gift.txt` group the
//! `daily_gift_week` flag names. A character at level 50 or above opens the week,
//! and each day is locked until its collect time (the week's start plus one day
//! per day) waits, or is collected. A locked day unlocks early for its cash cost
//! or one ticket (72319); a waiting day collects free. Both need a free inventory
//! cell. The open answers the `GC_DAILY_GIFT` record with the week's gifts, costs
//! and statuses; a collect answers it again.
//!
//! The week and the statuses are the character's (`daily_gift` row): the load
//! installs them, the save writes them. The open and loaded flags are
//! memory-only, and so is the renew time, which the admission reads from the
//! `daily_gift_renew_time` flag (`G/char.cpp:511`). The cash is the account's:
//! the load reads it, the world holds it, and the save writes the delta back.
//!
//! # Not ported
//!
//! Nothing. The record arms, the collect paths, the renew, the GM line and the
//! flag broadcast are all here.
//!
//! # Defects not reproduced
//!
//! - `FindSpecifyItem` answers null when no ticket is held, and the ticket arm
//!   reads its count without checking (`:199-202`), so collecting with the
//!   ticket and none held crashes the core. The Rewrite tells the no-ticket
//!   line, the one a held-out ticket answers.
//! - `GetVnum`, `GetCount` and `GetCost` index the week's days unchecked
//!   (`G/item_manager.h:46-59`), and the open reads all seven, so a week the
//!   file gave fewer than seven days reads past the group. The Rewrite answers
//!   the no-gifts line for a week that is missing or short.
//! - A slot past 6 wraps the day byte (`bSlotIndex + 1`) into a `day%d` column
//!   the table has no such column for (`:234-244`), which errors the query.
//!   The Rewrite ends a collect past day 7 silently.
//! - `m_vecDailyGift` is pushed twice and never read; only its emptiness is
//!   ever asked, beside the two bools that already say it. The Rewrite keeps
//!   the bools, not the vector.

use std::time::{SystemTime, UNIX_EPOCH};

use common::item_slots::usable_inventory_cells;
use common::vid::Vid;
use db::items::RowChange;
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_nested::GcDailyGift;
use world::character::{number, remove_specify_item, MoveDone, MoveKind};

use super::GameState;
use crate::chat_line::Arg;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

/// `DAILY_GIFT_CLOSE` (`C/length.h`): close the week.
pub const DAILY_GIFT_CLOSE: u8 = 0;
/// `DAILY_GIFT_OPEN`: open the week.
pub const DAILY_GIFT_OPEN: u8 = 1;
/// `DAILY_GIFT_COLLECT`: collect the slot's day.
pub const DAILY_GIFT_COLLECT: u8 = 2;
/// `DAILY_GIFT_COLLECT_USE_TICKET`: collect the slot's day with the ticket.
pub const DAILY_GIFT_COLLECT_USE_TICKET: u8 = 3;

/// `DAILY_GIFT_COLLECT_LOCK`: the day waits for its collect time.
pub const DAILY_GIFT_LOCKED: u8 = 0;
/// `DAILY_GIFT_COLLECT_WAIT`: the day collects free.
pub const DAILY_GIFT_WAITING: u8 = 1;
/// `DAILY_GIFT_COLLECT_SUCCESS`: the day is collected.
pub const DAILY_GIFT_COLLECTED: u8 = 2;

/// `DAILY_GIFT_TICKET_ITEM` (`C/length.h`): the ticket that unlocks a locked day.
pub const DAILY_GIFT_TICKET: u32 = 72319;
/// `HasDailyGiftMission` (`G/char_daily_gifts.cpp:80-87`): the level that opens the week.
pub const DAILY_GIFT_MISSION_LEVEL: u8 = 50;

/// One daily-gift request the handler's flag gate let through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DailyGiftStep {
    /// `DAILY_GIFT_CLOSE`.
    Close,
    /// `DAILY_GIFT_OPEN`.
    Open,
    /// `DAILY_GIFT_COLLECT` or `DAILY_GIFT_COLLECT_USE_TICKET`, with the slot.
    Collect {
        /// `bSlotIndex`: the 0-based day.
        slot: u8,
        /// Whether the ticket arm was named.
        ticket: bool,
    },
}

/// A character's daily-gift week: its flags, its stored row, and its cash.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DailyGiftState {
    /// `m_bDailyGiftOpen`: the week is open.
    pub open: bool,
    /// `m_bDailyGiftLoad`: the week is loaded.
    pub loaded: bool,
    /// `m_dwDailyGiftRenewTime`: the renew time the admission read from the flag.
    pub renew_time: u32,
    /// The stored row, if the week was ever opened: the week, the seven statuses
    /// and the renew time.
    pub stored: Option<db::daily_gift::StoredDailyGift>,
    /// The account's cash, which an early unlock spends.
    pub cash: u32,
}

/// `time(0)` as legacy compares it: seconds since the epoch, saturating at `DWORD_MAX`.
fn now_secs() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(u32::MAX, |since| {
            u32::try_from(since.as_secs()).unwrap_or(u32::MAX)
        })
}

/// `ctime` of `secs`, in UTC: `Www Mmm DD HH:MM:SS YYYY` with the trailing newline,
/// the day space-padded as `ctime` pads it.
///
/// Legacy prints local time; the Rewrite has no time zone configured, so it prints
/// UTC. The newline rides the line, as legacy's `%s` carries `ctime`'s.
fn ctime_utc(secs: u32) -> Vec<u8> {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = i64::from(secs) / 86_400;
    let clock = i64::from(secs) % 86_400;
    // Howard Hinnant's civil-from-days: March-based months, so February's leap day
    // lands at the year's end. Day 0 is 1970-03-01 in March numbering.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let day_of_year = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!(
        "{} {} {:2} {:02}:{:02}:{:02} {}\n",
        DAYS[usize::try_from(days.rem_euclid(7)).unwrap_or(0)],
        MONTHS[usize::try_from(month - 1).unwrap_or(0)],
        day,
        clock / 3600,
        clock % 3600 / 60,
        clock % 60,
        year,
    )
    .into_bytes()
}

impl GameState {
    /// The daily-gift week the character online under `vid` plays, empty for one that
    /// entered without a row.
    fn daily_gift_mut(&mut self, vid: Vid) -> &mut DailyGiftState {
        self.daily_gifts.entry(vid).or_default()
    }

    /// Install the stored row and the cash at the admission, and read the renew time
    /// from the flag (`CHARACTER::CHARACTER`, `G/char.cpp:511`).
    pub(super) fn install_daily_gift(
        &mut self,
        vid: Vid,
        stored: Option<db::daily_gift::StoredDailyGift>,
        cash: u32,
    ) {
        let renew_time = self.quests.as_ref().map_or(0, |quests| {
            quests.manager().event_flag(b"daily_gift_renew_time")
        });
        let state = self.daily_gift_mut(vid);
        state.stored = stored;
        state.cash = cash;
        state.renew_time = u32::try_from(renew_time).unwrap_or(u32::MAX);
    }

    /// The stored row, if the week was ever opened, with the cash: what a save writes.
    pub(super) fn kept_daily_gift(
        &self,
        vid: Vid,
    ) -> (Option<db::daily_gift::StoredDailyGift>, u32) {
        self.daily_gifts
            .get(&vid)
            .map_or((None, 0), |state| (state.stored, state.cash))
    }

    /// Whether the `daily_gift_event` flag runs (`CheckDailyGiftStatus`, `:380-386`).
    fn daily_running(&self) -> bool {
        self.quests
            .as_ref()
            .is_some_and(|quests| quests.manager().event_flag(b"daily_gift_event") > 0)
    }

    /// Whether the character online under `vid` is level 50 or above
    /// (`HasDailyGiftMission`, `:80-87`).
    fn daily_mission(&self, vid: Vid) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .is_some_and(|points| points.level() >= DAILY_GIFT_MISSION_LEVEL)
    }

    /// One event flag's value, 0 without quests.
    fn daily_flag(&self, name: &[u8]) -> i32 {
        self.quests
            .as_ref()
            .map_or(0, |quests| quests.manager().event_flag(name))
    }

    /// One `CG_DAILY_GIFT` the flag gate let through. Every action is silent while
    /// the event is off (`CInputMain::DailyGift`, `G/input_main.cpp:3512-3545`).
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when the world does not hold the character.
    pub fn daily_gift(
        &mut self,
        vid: Vid,
        step: DailyGiftStep,
        mover: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?
            .player_id();
        if !self.daily_running() {
            return Ok(Self::daily_silent(
                &self.protos,
                &self.locale,
                pid,
                vid,
                mover,
            ));
        }
        match step {
            DailyGiftStep::Close => {
                let state = self.daily_gift_mut(vid);
                state.open = false;
                state.loaded = false;
                Ok(Self::daily_silent(
                    &self.protos,
                    &self.locale,
                    pid,
                    vid,
                    mover,
                ))
            }
            DailyGiftStep::Open => {
                self.daily_open(vid, mover);
                Ok(Self::daily_silent(
                    &self.protos,
                    &self.locale,
                    pid,
                    vid,
                    mover,
                ))
            }
            DailyGiftStep::Collect { slot, ticket } => {
                let changes = self.daily_collect(vid, slot, ticket, mover);
                let mut answer = Self::daily_silent(&self.protos, &self.locale, pid, vid, mover);
                answer.changes = changes;
                Ok(answer)
            }
        }
    }

    /// An answer that carries nothing: a refused or record-only step.
    fn daily_silent(
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
                kind: MoveKind::Socketed,
                records: Vec::new(),
                changes: Vec::new(),
            },
            mover,
            protos,
            locale,
        )
    }

    /// `LoadDailyGiftWeek` (`G/char_daily_gifts.cpp:21-70`): renew the week if its time
    /// passed, then answer the record, unless the week is already loaded. A week with
    /// no group, or a short one, is told there are no gifts.
    fn daily_open(&mut self, vid: Vid, mover: Mover) {
        self.check_daily_renew(vid);
        {
            let state = self.daily_gift_mut(vid);
            state.open = true;
            if state.loaded {
                if !self.daily_mission(vid) {
                    self.tell(
                        vid,
                        mover,
                        CHAT_TYPE_INFO,
                        b"You cannot collect any reward until you complete the required quest. (Reach level 50)",
                        &[],
                    );
                }
                return;
            }
        }
        let week = u8::try_from(self.daily_flag(b"daily_gift_week")).unwrap_or(0);
        let group = self
            .daily_weeks
            .group(week)
            .filter(|group| group.days.len() == gamedata::daily_gift::DAILY_GIFT_DAY_COUNT);
        let Some(group) = group else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"There are no daily gifts for this week.",
                &[],
            );
            return;
        };
        let group = group.clone();
        let start = u32::try_from(self.daily_flag(b"daily_gift_start_time")).unwrap_or(u32::MAX);
        let mut vnum = [0u32; 7];
        let mut count = [0u32; 7];
        let mut collect_time = [0u32; 7];
        let mut cost = [0u16; 7];
        let mut status = [0u8; 7];
        for (day, gift) in group.days.iter().enumerate() {
            let at = start.saturating_add(86_400 * u32::try_from(day + 1).unwrap_or(u32::MAX));
            vnum[day] = gift.vnum;
            count[day] = gift.count;
            collect_time[day] = at;
            cost[day] = u16::try_from(gift.cost).unwrap_or(u16::MAX);
            status[day] =
                self.daily_status(vid, mover, u8::try_from(day + 1).unwrap_or(u8::MAX), at);
        }
        let cash = self.daily_gifts.get(&vid).map_or(0, |state| state.cash);
        let mut record = Vec::new();
        GcDailyGift {
            header: GcDailyGift::header(),
            cash,
            week,
            vnum,
            count,
            collect_time,
            cost,
            status,
        }
        .encode_into(&mut record);
        let _sent = self.write_to_client(vid, record);
        self.daily_gift_mut(vid).loaded = true;
        if !self.daily_mission(vid) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"You cannot collect any reward until you complete the required quest. (Reach level 50)",
                &[],
            );
        }
    }

    /// `GetDailyGiftStatus` (`G/char_daily_gifts.cpp:232-312`): the day's status. A row
    /// that is not this week's, or whose renew time is not past the week's start, is
    /// replaced by a fresh one with day 1 waiting; a day past its collect time waits,
    /// and the wait is stored when the row counted.
    fn daily_status(&mut self, vid: Vid, mover: Mover, day: u8, collect_time: u32) -> u8 {
        let open = self.daily_gifts.get(&vid).is_some_and(|state| state.open);
        if !open || !self.daily_running() || !self.daily_mission(vid) {
            return DAILY_GIFT_LOCKED;
        }
        let week = u8::try_from(self.daily_flag(b"daily_gift_week")).unwrap_or(0);
        let start =
            i64::from(u32::try_from(self.daily_flag(b"daily_gift_start_time")).unwrap_or(0));
        let renew =
            i64::from(u32::try_from(self.daily_flag(b"daily_gift_renew_time")).unwrap_or(0));
        let counted = self.daily_gifts.get(&vid).is_some_and(|state| {
            state
                .stored
                .is_some_and(|stored| stored.week == week && stored.renew_time > start)
        });
        let mut status;
        if counted {
            let index = usize::from(day.saturating_sub(1));
            status = self
                .daily_gifts
                .get(&vid)
                .and_then(|state| state.stored)
                .map_or(DAILY_GIFT_LOCKED, |stored| {
                    stored.days.get(index).copied().unwrap_or(DAILY_GIFT_LOCKED)
                });
        } else {
            self.daily_gift_mut(vid).stored = Some(db::daily_gift::StoredDailyGift {
                week,
                days: [DAILY_GIFT_WAITING, 0, 0, 0, 0, 0, 0],
                renew_time: renew,
            });
            status = if day == 1 {
                DAILY_GIFT_WAITING
            } else {
                DAILY_GIFT_LOCKED
            };
        }
        if collect_time != 0 && now_secs() > collect_time && status != DAILY_GIFT_COLLECTED {
            if counted {
                let index = usize::from(day.saturating_sub(1));
                if let Some(stored) = self.daily_gift_mut(vid).stored.as_mut() {
                    if let Some(slot) = stored.days.get_mut(index) {
                        *slot = DAILY_GIFT_WAITING;
                    }
                }
            }
            status = DAILY_GIFT_WAITING;
        }
        if self.test_server {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"[TEST] You can collect day %d at %s",
                &[
                    Arg::Int(i64::from(day)),
                    Arg::Text(&ctime_utc(collect_time)),
                ],
            );
        }
        status
    }

    /// `CheckDailyGiftRenewTime` (`G/char_daily_gifts.cpp:89-129`): when the renew time
    /// passed, start the next week, or stop the event when it does not renew.
    fn check_daily_renew(&mut self, vid: Vid) {
        let now = now_secs();
        let renew_time = self
            .daily_gifts
            .get(&vid)
            .map_or(0, |state| state.renew_time);
        if renew_time == 0 || now <= renew_time {
            return;
        }
        if self.daily_flag(b"daily_gift_renew") > 0 {
            let next = now.saturating_add(86_400 * 7).saturating_add(1);
            let week = if self.daily_flag(b"daily_gift_order") > 0 {
                let next_week = u8::try_from(self.daily_flag(b"daily_gift_week"))
                    .unwrap_or(0)
                    .saturating_add(1);
                let groups = u8::try_from(self.daily_weeks.len()).unwrap_or(u8::MAX);
                if next_week > groups {
                    1
                } else {
                    next_week
                }
            } else {
                let groups = i32::try_from(self.daily_weeks.len()).unwrap_or(i32::MAX);
                u8::try_from(number(&mut self.dice, 1, groups)).unwrap_or(1)
            };
            self.set_event_flag(b"daily_gift_week", i32::from(week));
            self.set_event_flag(
                b"daily_gift_start_time",
                i32::try_from(now).unwrap_or(i32::MAX),
            );
            self.set_event_flag(
                b"daily_gift_renew_time",
                i32::try_from(next).unwrap_or(i32::MAX),
            );
            self.daily_gift_mut(vid).renew_time = next;
        } else {
            for name in [
                b"daily_gift_event".as_slice(),
                b"daily_gift_week",
                b"daily_gift_start_time",
                b"daily_gift_renew_time",
                b"daily_gift_renew",
                b"daily_gift_order",
            ] {
                self.set_event_flag(name, 0);
            }
            self.daily_gift_mut(vid).renew_time = 0;
        }
    }

    /// `CollectDailyGift` (`G/char_daily_gifts.cpp:132-230`): collect the slot's day.
    /// A locked day unlocks for its cash cost, or for the ticket when the ticket arm
    /// ran; a waiting day collects free; a collected day is told it already was. A
    /// collect without a free inventory cell is told `[LS;1444]`. A collect answers
    /// the rows to store.
    fn daily_collect(&mut self, vid: Vid, slot: u8, ticket: bool, mover: Mover) -> Vec<RowChange> {
        let held = self
            .daily_gifts
            .get(&vid)
            .is_some_and(|state| state.open || state.loaded);
        if !held {
            return Vec::new();
        }
        if !self.daily_running() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"The event is no longer active.",
                &[],
            );
            return Vec::new();
        }
        if !self.daily_mission(vid) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"You cannot collect any reward until you complete the required quest. (Reach level 50)",
                &[],
            );
            return Vec::new();
        }
        if usize::from(slot) >= gamedata::daily_gift::DAILY_GIFT_DAY_COUNT {
            return Vec::new();
        }
        let week = u8::try_from(self.daily_flag(b"daily_gift_week")).unwrap_or(0);
        let group = self
            .daily_weeks
            .group(week)
            .filter(|group| group.days.len() == gamedata::daily_gift::DAILY_GIFT_DAY_COUNT);
        let Some(group) = group.cloned() else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"There are no daily gifts for this week.",
                &[],
            );
            return Vec::new();
        };
        let day = slot.saturating_add(1);
        let status = self.daily_status(vid, mover, day, 0);
        if status == DAILY_GIFT_LOCKED {
            let gift = group.days[usize::from(slot)];
            return self.daily_collect_locked(vid, slot, ticket, mover, gift);
        }
        match status {
            DAILY_GIFT_WAITING => {
                let gift = group.days[usize::from(slot)];
                if !self.daily_cell_free(vid) {
                    self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;1444]", &[]);
                    return Vec::new();
                }
                let changes = self.daily_give(vid, mover, gift.vnum, gift.count, Vec::new());
                self.daily_mark_collected(vid, slot);
                self.daily_gift_mut(vid).loaded = false;
                self.daily_open(vid, mover);
                changes
            }
            DAILY_GIFT_COLLECTED => {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"You have already recieved this reward.",
                    &[],
                );
                Vec::new()
            }
            _ => {
                self.daily_gift_mut(vid).loaded = false;
                self.daily_open(vid, mover);
                Vec::new()
            }
        }
    }

    /// The locked day's collect (`:160-219`): for the ticket when the ticket arm
    /// ran, else for cash when it covers the cost. The cash branch stays shut
    /// while the ticket arm ran, even when the cash would cover it.
    fn daily_collect_locked(
        &mut self,
        vid: Vid,
        slot: u8,
        ticket: bool,
        mover: Mover,
        gift: gamedata::daily_gift::DailyGiftDay,
    ) -> Vec<RowChange> {
        if ticket {
            if !self.daily_cell_free(vid) {
                self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;1444]", &[]);
                return Vec::new();
            }
            if self.daily_ticket_count(vid) <= 0 {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"You don't have the ticket.",
                    &[],
                );
                return Vec::new();
            }
            let changes = self.daily_take_ticket(vid, mover);
            let changes = self.daily_give(vid, mover, gift.vnum, gift.count, changes);
            self.daily_mark_collected(vid, slot);
            self.daily_gift_mut(vid).loaded = false;
            self.daily_open(vid, mover);
            return changes;
        }
        let cash = self.daily_gifts.get(&vid).map_or(0, |state| state.cash);
        if cash < gift.cost {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Unfortunately you don't have enough cash.",
                &[],
            );
            return Vec::new();
        }
        if !self.daily_cell_free(vid) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;1444]", &[]);
            return Vec::new();
        }
        if let Some(state) = self.daily_gifts.get_mut(&vid) {
            state.cash = state.cash.saturating_sub(gift.cost);
        }
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"You have unlocked the item successfully.",
            &[],
        );
        let changes = self.daily_give(vid, mover, gift.vnum, gift.count, Vec::new());
        self.daily_mark_collected(vid, slot);
        self.daily_gift_mut(vid).loaded = false;
        self.daily_open(vid, mover);
        changes
    }

    /// Whether a size-1 item fits the inventory (`GetEmptyInventory(1)`, `:164,205`).
    fn daily_cell_free(&self, vid: Vid) -> bool {
        self.characters.find_by_vid(vid).is_ok_and(|character| {
            let usable = usable_inventory_cells(character.inven_point());
            character
                .items()
                .find_free_inventory_cell(usable, 1)
                .is_some()
        })
    }

    /// The tickets held (`FindSpecifyItem`, `:198-202`): the first stack's count, 0
    /// with none held.
    fn daily_ticket_count(&self, vid: Vid) -> i32 {
        self.characters
            .find_by_vid(vid)
            .map(|character| {
                let usable = usable_inventory_cells(character.inven_point());
                world::character::count_specify_item(character.items(), usable, DAILY_GIFT_TICKET)
            })
            .unwrap_or(0)
    }

    /// Spend one ticket (`RemoveSpecifyItem`, `:203`), answering the rows to store.
    fn daily_take_ticket(&mut self, vid: Vid, mover: Mover) -> Vec<RowChange> {
        let protos = self.protos.clone();
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return Vec::new();
        };
        let usable = usable_inventory_cells(character.inven_point());
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let done = remove_specify_item(items, slots, &protos, usable, (DAILY_GIFT_TICKET, 1));
        let done = MoveDone {
            kind: MoveKind::Removed,
            records: done.records,
            changes: done.changes,
        };
        let packed = MovedItems::new(pid, vid.raw(), done, mover, &protos, &self.locale);
        for record in packed.records {
            let _sent = self.write_to_client(vid, record);
        }
        packed.changes
    }

    /// Give the day's gift (`AutoGiveItem(vnum, count, -1, false)`, `:170-174`), past
    /// the rows `before` already answered. The give runs silent: `bMsg` is false, so
    /// the pickup notice the shared give packs is left out.
    fn daily_give(
        &mut self,
        vid: Vid,
        mover: Mover,
        vnum: u32,
        count: u32,
        mut before: Vec<RowChange>,
    ) -> Vec<RowChange> {
        let questing = self.quest_running(vid);
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        if let Some((_, mut given)) = self.auto_give(
            vid,
            (vnum, u16::try_from(count).unwrap_or(u16::MAX)),
            -1,
            questing,
        ) {
            given
                .records
                .retain(|record| !matches!(record, world::character::MoveRecord::PickedUp { .. }));
            let packed = MovedItems::new(pid, vid.raw(), given, mover, &self.protos, &self.locale);
            for record in packed.records {
                let _sent = self.write_to_client(vid, record);
            }
            before.extend(packed.changes);
        }
        before
    }

    /// Mark the slot's day collected, storing the wait the collect found.
    fn daily_mark_collected(&mut self, vid: Vid, slot: u8) {
        if let Some(stored) = self.daily_gift_mut(vid).stored.as_mut() {
            if let Some(day) = stored.days.get_mut(usize::from(slot)) {
                *day = DAILY_GIFT_COLLECTED;
            }
        }
    }

    /// `ACMD(do_daily_gift_event)` (`G/cmd_gm.cpp:5061-5152`): with no argument the
    /// usage and, while the event runs, the start and renew dates; else the event
    /// starts for a week or stops, and the start is noticed to every PC.
    pub(super) fn run_daily_gift_event(&mut self, vid: Vid, rest: &[u8], mover: Mover) {
        let (first, back) = crate::command::one_argument(rest);
        let (second, back) = crate::command::one_argument(back);
        let (third, _) = crate::command::one_argument(back);
        let digit = |word: &[u8]| word.first().is_some_and(u8::is_ascii_digit);
        let byte = super::skill_admin::str_to_byte;
        if first.is_empty() {
            self.daily_gift_event_usage(vid, mover);
            return;
        }
        let command = if digit(&first) { byte(&first) } else { 1 };
        let specific = if !second.is_empty() && digit(&second) {
            byte(&second)
        } else {
            0
        };
        let order = !second.is_empty() && second == b"auto";
        let renew = if !third.is_empty() && digit(&third) {
            byte(&third)
        } else {
            0
        };
        if command == 1 {
            if self.daily_running() {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"This event is already active.",
                    &[],
                );
                return;
            }
            let now = now_secs();
            let until = now.saturating_add(86_400 * 7).saturating_add(1);
            let week = if order {
                1
            } else if specific > 0 {
                specific
            } else {
                let groups = i32::try_from(self.daily_weeks.len()).unwrap_or(i32::MAX);
                u8::try_from(number(&mut self.dice, 1, groups)).unwrap_or(1)
            };
            self.set_event_flag(b"daily_gift_event", 1);
            self.set_event_flag(b"daily_gift_week", i32::from(week));
            self.set_event_flag(
                b"daily_gift_start_time",
                i32::try_from(now).unwrap_or(i32::MAX),
            );
            self.set_event_flag(
                b"daily_gift_renew_time",
                i32::try_from(until).unwrap_or(i32::MAX),
            );
            self.set_event_flag(b"daily_gift_renew", i32::from(renew));
            self.set_event_flag(b"daily_gift_order", i32::from(order));
            self.notice_every_pc(b"Daily gifts are now available.", &[]);
        } else {
            for name in [
                b"daily_gift_event".as_slice(),
                b"daily_gift_week",
                b"daily_gift_start_time",
                b"daily_gift_renew_time",
                b"daily_gift_renew",
                b"daily_gift_order",
            ] {
                self.set_event_flag(name, 0);
            }
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"You disabled the daily gift event.",
                &[],
            );
        }
    }

    /// The `/daily_gift_event` usage (`:5070-5094`): the seventeen lines and, while
    /// the event runs, the start and renew dates. The `<renew>` block repeats, as
    /// legacy repeats it.
    fn daily_gift_event_usage(&self, vid: Vid, mover: Mover) {
        for line in [
            "Usage: daily_gift_event <command> <specific_week> <renew>",
            "<command>",
            "    0 = Disable",
            "    1 = Enable",
            "<specific_week>",
            "    ID = ( 0 = Random | # Week table )",
            "    STRING = ( auto = Start week from order )",
            "<renew>",
            "    0 = The event will stop after a week",
            "    1 = The event will continue after a week",
            "        with the same arguments.",
            "<renew>",
            "    0 = The event will stop after a week",
            "    1 = The event will continue after a week",
            "        with the same arguments.",
            "Example:",
            "Command: /daily_gift_event 1 auto 1",
        ] {
            self.tell(vid, mover, CHAT_TYPE_INFO, line.as_bytes(), &[]);
        }
        if self.daily_running() {
            let start = u32::try_from(self.daily_flag(b"daily_gift_start_time")).unwrap_or(0);
            let renew = u32::try_from(self.daily_flag(b"daily_gift_renew_time")).unwrap_or(0);
            self.tell(vid, mover, CHAT_TYPE_INFO, b"", &[]);
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"     Start Date: %s",
                &[Arg::Text(&ctime_utc(start))],
            );
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"     Renew Date: %s",
                &[Arg::Text(&ctime_utc(renew))],
            );
        }
    }

    /// The `daily_gift_event` arm of `SetEventFlag` (`G/questmanager.cpp:1440-1451`):
    /// every character at level 50 or above hears `DailyGiftEvent` with the value.
    pub(super) fn daily_gift_event_arm(&self, value: i32) {
        let mut vids: Vec<Vid> = self.outboxes.keys().copied().collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            if !self.daily_mission(vid) {
                continue;
            }
            if let Some(mover) = self.own_mover(vid) {
                self.tell(
                    vid,
                    mover,
                    protocol::gc_chat::CHAT_TYPE_COMMAND,
                    b"DailyGiftEvent %d",
                    &[Arg::Int(i64::from(value))],
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ctime_utc, DAILY_GIFT_LOCKED, DAILY_GIFT_MISSION_LEVEL};
    use crate::game_state::fixtures::{a_world, enter, points_at, TestClock};
    use common::vid::Vid;

    fn mover() -> crate::item_move::Mover {
        crate::item_move::Mover {
            recently_fought: false,
            empire: 1,
            language: 0,
            pk_mode: 4,
            affect_flags: [0; 2],
            dead: false,
            alignment: 0,
            view_guild: (0, 0),
        }
    }

    #[test]
    fn ctime_prints_the_epoch_and_a_leap_day() {
        assert_eq!(
            ctime_utc(0),
            b"Thu Jan  1 00:00:00 1970\n".to_vec(),
            "the epoch"
        );
        // 2024-02-29 16:14:56 UTC, a Thursday on a leap day.
        assert_eq!(
            ctime_utc(1_709_223_296),
            b"Thu Feb 29 16:14:56 2024\n".to_vec(),
            "the leap day"
        );
        // 2026-11-07 00:00:00 UTC, a Saturday with a single-digit day.
        assert_eq!(
            ctime_utc(1_794_009_600),
            b"Sat Nov  7 00:00:00 2026\n".to_vec(),
            "the single-digit day pads with a space"
        );
    }

    #[test]
    fn the_mission_is_level_50() {
        assert_eq!(DAILY_GIFT_MISSION_LEVEL, 50);
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, 7, (100, 100), 10_000);
        assert!(!world.daily_mission(Vid::new(7)), "level 10");
        world
            .characters
            .find_by_vid_mut(Vid::new(7))
            .expect("the player is in the world")
            .set_points(Some(points_at(50, 10_000, 0)));
        assert!(world.daily_mission(Vid::new(7)), "level 50");
    }

    #[test]
    fn a_closed_week_locks_every_day() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, 7, (100, 100), 10_000);
        assert_eq!(
            world.daily_status(Vid::new(7), mover(), 1, 1),
            DAILY_GIFT_LOCKED,
            "no open, no quests"
        );
        world.daily_gift_mut(Vid::new(7)).open = true;
        assert_eq!(
            world.daily_status(Vid::new(7), mover(), 1, 1),
            DAILY_GIFT_LOCKED,
            "open but the event is off"
        );
    }
}
