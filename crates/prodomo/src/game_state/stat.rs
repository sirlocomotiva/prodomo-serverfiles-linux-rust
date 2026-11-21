//! The stat commands: `/stat` and `/stat-` (`G/cmd_general.cpp:715-844`).
//!
//! [`GameState::run_stat`] is `ACMD(do_stat)` (`:790-844`) and `ACMD(do_stat_minus)`
//! (`:715-788`): the line's word names one of the four attributes (`st`, `ht`, `dx`,
//! `iq`), the point moves by 1, and the records ride the caller's outbox. The raise
//! needs a stat point and a real point below the cap; the lower needs a reset count
//! and a real point above the job's start. The cap is `g_iStatusPointSetMaxValue`
//! (`game.status_point_set_max_value`, `G/config.cpp:1101-1110`), 90 on the owner's
//! data. The polymorph gate refuses first (`[LS;521]`), then the lock gates the raise
//! and the lower at legacy's places, with the popup it chats.
//!
//! The raises land exactly as legacy writes them: both arrays move, `ComputePoints`
//! runs, the attribute's `PointChange` runs (which recomputes the battle points),
//! then the crossed pool line runs (`do_stat` sends `POINT_MAX_HP` for `iq` and
//! `POINT_MAX_SP` for `ht`, `:836-841`), the stat counter falls, and `ComputePoints`
//! runs again. The lowers land the same way: both arrays move, compute, the
//! attribute's change (`ht` and `iq` carry their own pool's line this time,
//! `:752-767`), the stat counter rises and the reset count falls, and compute again.
//! The stat counter and the reset count are `COUNTERS`: their changes land the real
//! array too. The held row takes the four real attributes and both counters, because
//! the save writes the row, not the points.

use common::point_slot as point;
use common::vid::Vid;

use super::whisper::str_to_int;
use crate::command::one_argument;
use crate::game_state::GameState;

/// What the line moved: the four real attributes and the two counters, for the held
/// row the save writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatHeld {
    /// `POINT_ST`, real.
    pub st: i32,
    /// `POINT_HT`, real.
    pub ht: i32,
    /// `POINT_DX`, real.
    pub dx: i32,
    /// `POINT_IQ`, real.
    pub iq: i32,
    /// `POINT_STAT`.
    pub stat: i32,
    /// `POINT_STAT_RESET_COUNT`.
    pub reset_count: i32,
}

/// What a `t*` line moved: the target's name as typed, for the row the
/// store writes, and the target's new held numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetStatHeld {
    /// The name the line named, as typed.
    pub target: String,
    /// The target's four real attributes and both counters.
    pub held: StatHeld,
}

impl GameState {
    /// `ACMD(do_stat_reset)` (`G/cmd_general.cpp:710-713`): the reset count
    /// rises to 12 through `PointChange`, so the record rides the outbox.
    /// Always runs: legacy reads no word.
    pub(super) fn reset_stat_count(&mut self, vid: Vid) -> Option<StatHeld> {
        let points = self.pc_points_mut(vid.raw())?;
        let missing = 12 - points.get_point(point::POINT_STAT_RESET_COUNT);
        let changed = points
            .point_change(point::POINT_STAT_RESET_COUNT, missing, false, false)
            .unwrap_or_default();
        self.send_point_records(vid, &changed);
        Some(self.stat_held(vid))
    }
    /// `ACMD(do_stat)` or `ACMD(do_stat_minus)`: the line's attribute rises or falls
    /// by 1. Answers the new held numbers, or `None` for a line that runs nothing
    /// (no word, an unknown word, a gate, or a cap).
    pub(super) fn run_stat(&mut self, vid: Vid, rest: &[u8], lower: bool) -> Option<StatHeld> {
        let (word, _) = one_argument(rest);
        if word.is_empty() {
            return None;
        }
        if self
            .points_of(vid.raw())
            .is_some_and(world::character::Points::is_polymorphed)
        {
            use protocol::gc_chat::CHAT_TYPE_INFO;
            let mover = self.own_mover(vid)?;
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;521]", &[]);
            return None;
        }
        // `do_stat` and `do_stat_minus` (`G/cmd_general.cpp:730,805`), after the empty
        // line; the popup chats. `do_stat_val` has no gate.
        if self.is_secured(vid) {
            let mover = self.own_mover(vid)?;
            let _sent = self.write_to_client(vid, self.protected_popup(mover));
            return None;
        }
        let kind = match word.as_slice() {
            b"st" => point::POINT_ST,
            b"ht" => point::POINT_HT,
            b"dx" => point::POINT_DX,
            b"iq" => point::POINT_IQ,
            _ => return None,
        };
        if lower {
            self.lower_stat(vid, kind)
        } else {
            self.raise_stat(vid, kind)
        }
    }

    /// `ACMD(do_stat_val)` (`G/cmd_general.cpp:3699-3750`): the line's
    /// attribute rises by the line's amount for stat points. Answers the new
    /// held numbers, or `None` for a line that runs nothing (no word, an
    /// amount at most 0, an unknown word, a gate, or a cap). The amount is
    /// clamped to the stat pool and to the room below the cap; the landing
    /// is `do_stat`'s, with the amount in place of 1. A polymorphed caller
    /// hears the Romanian line (`[LS;521]`'s sibling, `:3709-3713`).
    pub(super) fn run_stat_val(&mut self, vid: Vid, rest: &[u8]) -> Option<StatHeld> {
        let (word, tail) = one_argument(rest);
        let (amount_word, _) = one_argument(tail);
        let amount = str_to_int(&amount_word);
        if word.is_empty() || amount <= 0 {
            return None;
        }
        if self
            .points_of(vid.raw())
            .is_some_and(world::character::Points::is_polymorphed)
        {
            use protocol::gc_chat::CHAT_TYPE_INFO;
            let mover = self.own_mover(vid)?;
            let line = b"@@(cmd_general)tradus:You cannot change your state as long as you are transformed";
            self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
            return None;
        }
        let kind = match word.as_slice() {
            b"st" => point::POINT_ST,
            b"ht" => point::POINT_HT,
            b"dx" => point::POINT_DX,
            b"iq" => point::POINT_IQ,
            _ => return None,
        };
        self.raise_stat_by(vid, kind, amount)
    }

    /// `ACMD(do_stat_plus_amount)` (`G/cmd_gm.cpp:4068-4162`): the entry's
    /// attribute rises by the line's amount for stat points. Answers the new
    /// held numbers, or `None` for a line that runs nothing. Only the empty
    /// word is silent; every other refusal answers: the polymorph gate sends
    /// `[LS;521]`, an
    /// empty pool sends `There are no stat points left.`, an amount past the
    /// pool sends `Less stat points left.`, and a negative amount sends `You
    /// entered an incorrect value.`. The clamp is a hardcoded 90 on the
    /// instant value (`ENABLE_STATPLUS_NOLIMIT` is not defined), and the
    /// landing is `do_stat`'s without the pool line: both arrays move,
    /// compute, the attribute's change, the pool falls, and compute again.
    /// A clamped-to-zero amount moves nothing and still answers the held row.
    pub(super) fn run_stat_plus(&mut self, vid: Vid, rest: &[u8], kind: usize) -> Option<StatHeld> {
        use protocol::gc_chat::CHAT_TYPE_INFO;
        let (word, _) = one_argument(rest);
        if word.is_empty() {
            return None;
        }
        let mover = self.own_mover(vid)?;
        if self
            .points_of(vid.raw())
            .is_some_and(world::character::Points::is_polymorphed)
        {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;521]", &[]);
            return None;
        }
        let points = self.pc_points_mut(vid.raw())?;
        let pool = points.get_point(point::POINT_STAT);
        if pool <= 0 {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"There are no stat points left.",
                &[],
            );
            return None;
        }
        let mut amount = str_to_int(&word);
        if pool < amount {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Less stat points left.", &[]);
            return None;
        }
        if amount < 0 {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"You entered an incorrect value.",
                &[],
            );
            return None;
        }
        let instant = self.pc_points_mut(vid.raw())?.get_point(kind);
        if amount + instant > 90 {
            amount = 90 - instant;
        }
        if amount == 0 {
            return Some(self.stat_held(vid));
        }
        self.raise_stat_unclamped(vid, kind, amount)
    }

    /// `ACMD(do_set_stat)` (`G/cmd_gm.cpp:4575-4685`): the named character's
    /// entry attribute moves by the line's delta for the named character's
    /// stat points. Answers the target's name and new held numbers, or `None`
    /// for a line that runs nothing. A missing word answers `Invalid
    /// argument`; a name nobody online holds answers `Cannot find
    /// player(%s). ...` with the name twice (the P2P arm through the client
    /// registry: a name online on another Channel names the game-server arm,
    /// else the never-logged-in arm); below the target's job start answers
    /// `Cannot set stat under initial stat`; a delta past the target's pool
    /// answers `Less stat points left.`. The polymorph gate sends `[LS;521]`
    /// when the *target* is polymorphed, and the subcommand gate is dead:
    /// the dispatcher only builds the four attributes. Past 90 the target
    /// clamps to 90 and the delta shrinks with it; a zero or negative delta
    /// lands: the real array takes the target, the instant array moves by the
    /// delta (not to the target), compute runs, the attribute's change runs,
    /// the pool falls by the delta (a negative delta returns points), and
    /// compute runs again. The caller hears `%s's %s change %d to %d` with
    /// the name as typed.
    pub(super) fn run_set_stat(
        &mut self,
        vid: Vid,
        rest: &[u8],
        kind: usize,
    ) -> Option<SetStatHeld> {
        use crate::chat_line::Arg;
        use protocol::gc_chat::CHAT_TYPE_INFO;
        let (name_word, tail) = one_argument(rest);
        let (delta_word, _) = one_argument(tail);
        let mover = self.own_mover(vid)?;
        if name_word.is_empty() || delta_word.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Invalid argument", &[]);
            return None;
        }
        let target = std::str::from_utf8(&name_word)
            .ok()
            .and_then(|name| self.whisper_target(name));
        let Some(target) = target else {
            self.tell_set_stat_miss(vid, mover, &name_word);
            return None;
        };
        // The polymorph gate (`G/cmd_gm.cpp:4606-4610`): the *target's* morph refuses,
        // and the caller hears the line.
        if self
            .points_of(target.raw())
            .is_some_and(world::character::Points::is_polymorphed)
        {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;521]", &[]);
            return None;
        }
        let points = self.pc_points_mut(target.raw())?;
        let current = points.real_point(kind);
        let pool = points.get_point(point::POINT_STAT);
        let mut delta = str_to_int(&delta_word);
        let mut goal = current + delta;
        let floor = i32::from(points.job_start(kind));
        if goal < floor {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Cannot set stat under initial stat",
                &[],
            );
            return None;
        }
        if goal > 90 {
            delta -= goal - 90;
            goal = 90;
        }
        if pool < delta {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Less stat points left.", &[]);
            return None;
        }
        let points = self.pc_points_mut(target.raw())?;
        let instant = points.get_point(kind);
        points.set_split_points(kind, goal, instant + delta);
        let computed = points.compute_points();
        self.send_point_records(target, &computed);
        let points = self.pc_points_mut(target.raw())?;
        let changed = points
            .point_change(kind, 0, false, false)
            .unwrap_or_default();
        self.send_point_records(target, &changed);
        let points = self.pc_points_mut(target.raw())?;
        let spent = points
            .point_change(point::POINT_STAT, -delta, false, false)
            .unwrap_or_default();
        self.send_point_records(target, &spent);
        let points = self.pc_points_mut(target.raw())?;
        let computed = points.compute_points();
        self.send_point_records(target, &computed);
        let shown = match kind {
            point::POINT_HT => "con",
            point::POINT_IQ => "int",
            point::POINT_ST => "str",
            _ => "dex",
        };
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"%s's %s change %d to %d",
            &[
                Arg::Text(&name_word),
                Arg::Text(shown.as_bytes()),
                Arg::Int(i64::from(current)),
                Arg::Int(i64::from(goal)),
            ],
        );
        let held = self.stat_held(target);
        let target =
            std::str::from_utf8(&name_word).map_or_else(|_| String::new(), ToString::to_string);
        Some(SetStatHeld { target, held })
    }

    /// The `t*` cannot-find line: the game-server arm when the name is
    /// online on another Channel, else the never-logged-in arm. Split out so
    /// the line does not trip the line lint.
    fn tell_set_stat_miss(&self, vid: Vid, mover: crate::item_move::Mover, name: &[u8]) {
        use crate::chat_line::Arg;
        use protocol::gc_chat::CHAT_TYPE_INFO;
        let elsewhere = std::str::from_utf8(name).ok().and_then(|name| {
            self.clients
                .as_ref()
                .and_then(|clients| clients.entry_of(name))
                .map(|(_, entry, _)| entry.channel)
        });
        let mine = self.bodies.get(&vid).map_or(0, |body| body.channel);
        let line = if elsewhere.is_some_and(|channel| channel != mine) {
            b"Cannot find player(%s). %s is not in your game server".as_slice()
        } else {
            b"Cannot find player(%s). Perhaps %s doesn't login or exist".as_slice()
        };
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            line,
            &[Arg::Text(name), Arg::Text(name)],
        );
    }

    /// `ACMD(do_stat)`: the attribute rises by 1 for a stat point.
    fn raise_stat(&mut self, vid: Vid, kind: usize) -> Option<StatHeld> {
        self.raise_stat_by(vid, kind, 1)
    }

    /// The raise by `amount`: both arrays move, compute, the attribute's
    /// change, the crossed pool line, the stat counter falls by the amount,
    /// and compute again. The amount is clamped to the pool and to the room
    /// below the cap, as `do_stat_val` clamps it (`:3734-3738`).
    fn raise_stat_by(&mut self, vid: Vid, kind: usize, amount: i32) -> Option<StatHeld> {
        let cap = self.stat_max;
        let points = self.pc_points_mut(vid.raw())?;
        let pool = points.get_point(point::POINT_STAT);
        if pool <= 0 {
            return None;
        }
        let real = points.real_point(kind);
        if real >= cap {
            return None;
        }
        let mut amount = amount.min(pool);
        amount = amount.min(cap - real);
        if amount <= 0 {
            return None;
        }
        points.set_both_points(kind, real + amount);
        let computed = points.compute_points();
        self.send_point_records(vid, &computed);
        let points = self.pc_points_mut(vid.raw())?;
        let changed = points
            .point_change(kind, 0, false, false)
            .unwrap_or_default();
        self.send_point_records(vid, &changed);
        // `do_stat` crosses the pool lines: `iq` sends `POINT_MAX_HP`, `ht` sends
        // `POINT_MAX_SP` (`G/cmd_general.cpp:836-841`), and `do_stat_val` does the
        // same (`:3743-3746`).
        let pool = match kind {
            point::POINT_IQ => Some(point::POINT_MAX_HP),
            point::POINT_HT => Some(point::POINT_MAX_SP),
            _ => None,
        };
        if let Some(pool) = pool {
            let points = self.pc_points_mut(vid.raw())?;
            let pooled = points
                .point_change(pool, 0, false, false)
                .unwrap_or_default();
            self.send_point_records(vid, &pooled);
        }
        let points = self.pc_points_mut(vid.raw())?;
        let spent = points
            .point_change(point::POINT_STAT, -amount, false, false)
            .unwrap_or_default();
        self.send_point_records(vid, &spent);
        let points = self.pc_points_mut(vid.raw())?;
        let computed = points.compute_points();
        self.send_point_records(vid, &computed);
        Some(self.stat_held(vid))
    }

    /// The `*+` landing: `do_stat`'s without the crossed pool line and
    /// without the clamps, which the caller ran (`G/cmd_gm.cpp:4149-4158`).
    /// Both arrays move, compute, the attribute's change, the pool falls by
    /// the amount, and compute again.
    fn raise_stat_unclamped(&mut self, vid: Vid, kind: usize, amount: i32) -> Option<StatHeld> {
        let points = self.pc_points_mut(vid.raw())?;
        let real = points.real_point(kind);
        points.set_both_points(kind, real + amount);
        let computed = points.compute_points();
        self.send_point_records(vid, &computed);
        let points = self.pc_points_mut(vid.raw())?;
        let changed = points
            .point_change(kind, 0, false, false)
            .unwrap_or_default();
        self.send_point_records(vid, &changed);
        let points = self.pc_points_mut(vid.raw())?;
        let spent = points
            .point_change(point::POINT_STAT, -amount, false, false)
            .unwrap_or_default();
        self.send_point_records(vid, &spent);
        let points = self.pc_points_mut(vid.raw())?;
        let computed = points.compute_points();
        self.send_point_records(vid, &computed);
        Some(self.stat_held(vid))
    }

    /// `ACMD(do_stat_minus)`: the attribute falls by 1 and the point returns.
    fn lower_stat(&mut self, vid: Vid, kind: usize) -> Option<StatHeld> {
        let points = self.pc_points_mut(vid.raw())?;
        if points.get_point(point::POINT_STAT_RESET_COUNT) <= 0 {
            return None;
        }
        let floor = match kind {
            point::POINT_ST => i32::from(points.job_start(point::POINT_ST)),
            point::POINT_HT => i32::from(points.job_start(point::POINT_HT)),
            point::POINT_DX => i32::from(points.job_start(point::POINT_DX)),
            _ => i32::from(points.job_start(point::POINT_IQ)),
        };
        let real = points.real_point(kind);
        if real <= floor {
            return None;
        }
        points.set_both_points(kind, real - 1);
        let computed = points.compute_points();
        self.send_point_records(vid, &computed);
        let points = self.pc_points_mut(vid.raw())?;
        let changed = points
            .point_change(kind, 0, false, false)
            .unwrap_or_default();
        self.send_point_records(vid, &changed);
        // `do_stat_minus` sends each pool's own line (`G/cmd_general.cpp:752-767`).
        let pool = match kind {
            point::POINT_HT => Some(point::POINT_MAX_HP),
            point::POINT_IQ => Some(point::POINT_MAX_SP),
            _ => None,
        };
        if let Some(pool) = pool {
            let points = self.pc_points_mut(vid.raw())?;
            let pooled = points
                .point_change(pool, 0, false, false)
                .unwrap_or_default();
            self.send_point_records(vid, &pooled);
        }
        let points = self.pc_points_mut(vid.raw())?;
        let back = points
            .point_change(point::POINT_STAT, 1, false, false)
            .unwrap_or_default();
        self.send_point_records(vid, &back);
        let points = self.pc_points_mut(vid.raw())?;
        let spent = points
            .point_change(point::POINT_STAT_RESET_COUNT, -1, false, false)
            .unwrap_or_default();
        self.send_point_records(vid, &spent);
        let points = self.pc_points_mut(vid.raw())?;
        let computed = points.compute_points();
        self.send_point_records(vid, &computed);
        Some(self.stat_held(vid))
    }

    /// The four real attributes and the two counters the held row takes.
    fn stat_held(&mut self, vid: Vid) -> StatHeld {
        let points = self.pc_points_mut(vid.raw());
        let Some(points) = points else {
            return StatHeld {
                st: 0,
                ht: 0,
                dx: 0,
                iq: 0,
                stat: 0,
                reset_count: 0,
            };
        };
        StatHeld {
            st: points.real_point(point::POINT_ST),
            ht: points.real_point(point::POINT_HT),
            dx: points.real_point(point::POINT_DX),
            iq: points.real_point(point::POINT_IQ),
            stat: points.get_point(point::POINT_STAT),
            reset_count: points.get_point(point::POINT_STAT_RESET_COUNT),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StatHeld;
    use crate::game_state::fixtures::{a_world, enter, TestClock};
    use common::point_slot as point;
    use common::vid::Vid;

    const VID: u32 = 0x6001;

    /// A point slot as the change record carries it: every slot here is below 255.
    fn stat_kind(slot: usize) -> u8 {
        u8::try_from(slot).expect("a point slot fits its record")
    }

    /// How many frames of `kinds` carry `slot`.
    fn count_of(kinds: &[u8], slot: usize) -> usize {
        let kind = stat_kind(slot);
        kinds
            .iter()
            .fold(0, |count, &frame| count + usize::from(frame == kind))
    }

    /// The inbox after one line: every frame is a 25-byte point change.
    fn kinds_of(line: &[Vec<u8>]) -> Vec<u8> {
        for frame in line {
            assert_eq!(frame.len(), 25, "a point change is 25 bytes");
            assert_eq!(&frame[0..4], &17i32.to_le_bytes(), "header 17");
            assert_eq!(&frame[4..8], &VID.to_le_bytes(), "the caller's VID");
        }
        line.iter().map(|frame| frame[8]).collect()
    }

    /// Enter with one stat point and one reset count to spend.
    fn entered(
        world: &mut crate::game_state::GameState,
    ) -> tokio::sync::mpsc::UnboundedReceiver<Vec<u8>> {
        let mut inbox = enter(world, VID, (3200, 3200), 0);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _ = points.point_change(point::POINT_STAT, 1, false, false);
        let _ = points.point_change(point::POINT_STAT_RESET_COUNT, 1, false, false);
        while inbox.try_recv().is_ok() {}
        inbox
    }
    /// The text of one `GC_CHAT` line.
    fn text_of(line: &[u8]) -> Vec<u8> {
        let size = usize::from(u16::from_le_bytes([line[1], line[2]]));
        assert_eq!(size, line.len());
        assert_eq!(line[0], 4, "HEADER_GC_CHAT");
        line[10..].to_vec()
    }

    /// The other character the fixture enters: `P{OTHER}`.
    const OTHER: u32 = 0x6002;

    /// Enter the caller and `P{OTHER}` with five stat points on the other.
    fn entered_pair(
        world: &mut crate::game_state::GameState,
    ) -> (
        tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    ) {
        let mut caller = enter(world, VID, (3200, 3200), 0);
        let mut other = enter(world, OTHER, (3200, 3200), 0);
        let points = world.pc_points_mut(OTHER).expect("the other has points");
        let _ = points.point_change(point::POINT_STAT, 5, false, false);
        while caller.try_recv().is_ok() {}
        while other.try_recv().is_ok() {}
        (caller, other)
    }

    /// The inbox's point changes, whatever their VID.
    fn kinds_any(line: &[Vec<u8>], vid: u32) -> Vec<u8> {
        for frame in line {
            assert_eq!(frame.len(), 25, "a point change is 25 bytes");
            assert_eq!(&frame[0..4], &17i32.to_le_bytes(), "header 17");
            assert_eq!(&frame[4..8], &vid.to_le_bytes(), "the target's VID");
        }
        line.iter().map(|frame| frame[8]).collect()
    }

    /// `tcon P{OTHER} 3` moves the other's constitution by 3 for the other's
    /// points, the records ride the other's outbox, and the caller hears the
    /// confirmation.
    #[test]
    fn a_set_moves_the_named_characters_attribute() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let (mut caller, mut other) = entered_pair(&mut world);
        let set = world
            .run_set_stat(Vid::new(VID), b"P24578 3", point::POINT_HT)
            .expect("five points move by 3");
        assert_eq!(set.target, "P24578", "the name as typed");
        assert_eq!(set.held.ht, 7, "warrior constitution 4 rises to 7");
        assert_eq!(set.held.stat, 2, "three of the other's points spend");
        let mut frames = Vec::new();
        while let Ok(frame) = other.try_recv() {
            frames.push(frame);
        }
        let kinds = kinds_any(&frames, OTHER);
        assert!(
            kinds.contains(&(stat_kind(point::POINT_HT))),
            "the attribute's change rides: {kinds:?}"
        );
        assert!(
            kinds.contains(&(stat_kind(point::POINT_STAT))),
            "the spent points ride: {kinds:?}"
        );
        assert_eq!(
            text_of(&caller.try_recv().expect("the caller hears")),
            b"P24578's con change 4 to 7",
            "the confirmation"
        );
        assert!(caller.try_recv().is_err(), "one line only");
    }

    /// Below the job start the line refuses; past 90 it clamps and the delta
    /// shrinks with it.
    #[test]
    fn a_set_refuses_below_the_start_and_clamps_at_ninety() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let (mut caller, mut other) = entered_pair(&mut world);
        assert!(world
            .run_set_stat(Vid::new(VID), b"P24578 -2", point::POINT_HT)
            .is_none());
        assert_eq!(
            text_of(&caller.try_recv().expect("the floor answers")),
            b"Cannot set stat under initial stat",
            "below the start"
        );
        assert!(other.try_recv().is_err(), "nothing rides");
        let points = world.pc_points_mut(OTHER).expect("the other has points");
        points.set_both_points(point::POINT_HT, 88);
        let _ = points.point_change(point::POINT_STAT, 5, false, false);
        while caller.try_recv().is_ok() {}
        while other.try_recv().is_ok() {}
        let set = world
            .run_set_stat(Vid::new(VID), b"P24578 5", point::POINT_HT)
            .expect("the clamp banks the room");
        assert_eq!(set.held.ht, 90, "eighty-eight takes the two rooms");
        assert_eq!(set.held.stat, 8, "only the room below 90 spends");
    }

    /// A missing word, an unknown name, and a delta past the pool refuse.
    #[test]
    fn a_set_refuses_its_misses() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let (mut caller, mut other) = entered_pair(&mut world);
        assert!(world
            .run_set_stat(Vid::new(VID), b"P24578", point::POINT_HT)
            .is_none());
        assert_eq!(
            text_of(&caller.try_recv().expect("the miss answers")),
            b"Invalid argument",
            "a missing word"
        );
        assert!(world
            .run_set_stat(Vid::new(VID), b"Nobody 3", point::POINT_HT)
            .is_none());
        assert_eq!(
            text_of(&caller.try_recv().expect("the miss answers")),
            b"Cannot find player(Nobody). Perhaps Nobody doesn't login or exist",
            "nobody online"
        );
        assert!(world
            .run_set_stat(Vid::new(VID), b"P24578 9", point::POINT_HT)
            .is_none());
        assert_eq!(
            text_of(&caller.try_recv().expect("the miss answers")),
            b"Less stat points left.",
            "past the other's pool"
        );
        assert!(other.try_recv().is_err(), "no miss rides");
    }

    /// `con+ 3` banks the amount on `POINT_HT` without a pool line, and the
    /// held row answers.
    #[test]
    fn a_plus_banks_the_amount_without_a_pool_line() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = entered(&mut world);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _ = points.point_change(point::POINT_STAT, 4, false, false);
        while inbox.try_recv().is_ok() {}
        let held = world
            .run_stat_plus(Vid::new(VID), b"3", point::POINT_HT)
            .expect("five points raise by 3");
        assert_eq!(held.ht, 7, "warrior constitution 4 rises to 7");
        assert_eq!(held.stat, 2, "three points spend");
        let mut frames = Vec::new();
        while let Ok(frame) = inbox.try_recv() {
            frames.push(frame);
        }
        let kinds = kinds_of(&frames);
        assert!(
            kinds.contains(&(stat_kind(point::POINT_HT))),
            "the attribute's change rides: {kinds:?}"
        );
        assert!(
            kinds.contains(&(stat_kind(point::POINT_STAT))),
            "the spent points ride: {kinds:?}"
        );
        // `ComputePoints` nests each pool line once per compute (two computes),
        // and `ht` adds no crossed line of its own: `do_stat_plus_amount` sends
        // none.
        assert_eq!(
            count_of(&kinds, point::POINT_MAX_SP),
            2,
            "constitution carries no crossed line: {kinds:?}"
        );
    }

    /// The 90 clamp on the instant value: at 88, `5` banks 2 and spends 2.
    #[test]
    fn a_plus_clamps_to_ninety_on_the_instant() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = entered(&mut world);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _ = points.point_change(point::POINT_STAT, 4, false, false);
        points.set_both_points(point::POINT_ST, 88);
        while inbox.try_recv().is_ok() {}
        let held = world
            .run_stat_plus(Vid::new(VID), b"5", point::POINT_ST)
            .expect("the clamp banks the room");
        assert_eq!(held.st, 90, "eighty-eight takes the two rooms");
        assert_eq!(held.stat, 3, "only the room below 90 spends");
    }

    /// At 90 the amount clamps to zero: nothing moves and the held row still
    /// answers.
    #[test]
    fn a_plus_at_the_clamp_moves_nothing() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = entered(&mut world);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _ = points.point_change(point::POINT_STAT, 4, false, false);
        points.set_both_points(point::POINT_ST, 90);
        while inbox.try_recv().is_ok() {}
        let held = world
            .run_stat_plus(Vid::new(VID), b"5", point::POINT_ST)
            .expect("a zero clamp still answers");
        assert_eq!(held.st, 90, "nothing moves");
        assert_eq!(held.stat, 5, "nothing spends");
        assert!(inbox.try_recv().is_err(), "no record rides");
    }

    /// Only the empty word is silent; every other refusal answers.
    #[test]
    fn a_plus_refuses_aloud() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = entered(&mut world);
        assert!(world
            .run_stat_plus(Vid::new(VID), b"", point::POINT_HT)
            .is_none());
        assert!(inbox.try_recv().is_err(), "no word is silent");
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _ = points.point_change(point::POINT_STAT, -1, false, false);
        while inbox.try_recv().is_ok() {}
        assert!(world
            .run_stat_plus(Vid::new(VID), b"3", point::POINT_HT)
            .is_none());
        assert_eq!(
            text_of(&inbox.try_recv().expect("the empty pool answers")),
            b"There are no stat points left.",
            "the empty pool"
        );
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _ = points.point_change(point::POINT_STAT, 2, false, false);
        while inbox.try_recv().is_ok() {}
        assert!(world
            .run_stat_plus(Vid::new(VID), b"5", point::POINT_HT)
            .is_none());
        assert_eq!(
            text_of(&inbox.try_recv().expect("the past-pool amount answers")),
            b"Less stat points left.",
            "past the pool"
        );
        assert!(world
            .run_stat_plus(Vid::new(VID), b"-2", point::POINT_HT)
            .is_none());
        assert_eq!(
            text_of(&inbox.try_recv().expect("the negative amount answers")),
            b"You entered an incorrect value.",
            "the negative"
        );
    }
    /// `do_stat_val` banks the amount: `st 3` spends 3 points and moves by 3.
    #[test]
    fn a_value_banks_the_amount() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = entered(&mut world);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _ = points.point_change(point::POINT_STAT, 4, false, false);
        while inbox.try_recv().is_ok() {}
        let held = world
            .run_stat_val(Vid::new(VID), b"st 3")
            .expect("five points raise by 3");
        assert_eq!(
            held,
            StatHeld {
                st: 9,
                ht: 4,
                dx: 3,
                iq: 3,
                stat: 2,
                reset_count: 1,
            },
            "warrior strength 6 rises to 9 for 3 points"
        );
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        assert_eq!(points.real_point(point::POINT_ST), 9);
        assert_eq!(points.get_point(point::POINT_ST), 9);
        assert_eq!(points.get_point(point::POINT_STAT), 2);
        let mut frames = Vec::new();
        while let Ok(frame) = inbox.try_recv() {
            frames.push(frame);
        }
        let kinds = kinds_of(&frames);
        assert!(
            kinds.contains(&(stat_kind(point::POINT_ST))),
            "the attribute's change rides: {kinds:?}"
        );
        assert!(
            kinds.contains(&(stat_kind(point::POINT_STAT))),
            "the spent points ride: {kinds:?}"
        );
    }

    /// The clamps: past the pool the amount falls to the pool, and past the
    /// cap it falls to the room below the cap.
    #[test]
    fn a_value_clamps_to_the_pool_and_the_cap() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = entered(&mut world);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _ = points.point_change(point::POINT_STAT, 4, false, false);
        while inbox.try_recv().is_ok() {}
        let held = world
            .run_stat_val(Vid::new(VID), b"st 99")
            .expect("the pool clamps the amount");
        assert_eq!(held.st, 11, "six plus the five points");
        assert_eq!(held.stat, 0, "the pool empties");
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _ = points.point_change(point::POINT_STAT, 10, false, false);
        points.set_both_points(point::POINT_DX, 88);
        while inbox.try_recv().is_ok() {}
        let held = world
            .run_stat_val(Vid::new(VID), b"dx 5")
            .expect("the cap clamps the amount");
        assert_eq!(held.dx, 90, "eighty-eight takes the two rooms");
        assert_eq!(held.stat, 8, "only the room below the cap spends");
    }

    /// No word, no amount, a non-positive amount, an unknown word, no pool,
    /// and the cap are all silent.
    #[test]
    fn a_value_refuses_quietly() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = entered(&mut world);
        for line in [
            b"".as_slice(),
            b"st".as_slice(),
            b"st 0".as_slice(),
            b"st -2".as_slice(),
            b"xx 3".as_slice(),
        ] {
            assert!(world.run_stat_val(Vid::new(VID), line).is_none());
            assert!(inbox.try_recv().is_err(), "the line is silent");
        }
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _ = points.point_change(point::POINT_STAT, -1, false, false);
        while inbox.try_recv().is_ok() {}
        assert!(world.run_stat_val(Vid::new(VID), b"st 3").is_none());
        assert!(inbox.try_recv().is_err(), "no point is silent");
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _ = points.point_change(point::POINT_STAT, 5, false, false);
        points.set_both_points(point::POINT_ST, 90);
        while inbox.try_recv().is_ok() {}
        assert!(world.run_stat_val(Vid::new(VID), b"st 3").is_none());
        assert!(inbox.try_recv().is_err(), "the cap is silent");
    }

    /// A raise moves the attribute, spends the point, and answers the held row.
    #[test]
    fn a_raise_moves_the_attribute_and_spends_the_point() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = entered(&mut world);
        let held = world
            .run_stat(Vid::new(VID), b"st", false)
            .expect("one point raises");
        assert_eq!(
            held,
            StatHeld {
                st: 7,
                ht: 4,
                dx: 3,
                iq: 3,
                stat: 0,
                reset_count: 1,
            },
            "warrior strength 6 rises to 7"
        );
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        assert_eq!(points.real_point(point::POINT_ST), 7);
        assert_eq!(points.get_point(point::POINT_ST), 7);
        assert_eq!(points.get_point(point::POINT_STAT), 0);
        let mut frames = Vec::new();
        while let Ok(frame) = inbox.try_recv() {
            frames.push(frame);
        }
        let kinds = kinds_of(&frames);
        assert!(
            kinds.contains(&(stat_kind(point::POINT_ST))),
            "the attribute's change rides: {kinds:?}"
        );
        assert!(
            kinds.contains(&(stat_kind(point::POINT_STAT))),
            "the spent point rides: {kinds:?}"
        );
        // `ComputePoints` nests each pool line once per compute (two computes), and
        // `st`'s own line adds no third: only `ht` and `iq` carry a pool line.
        assert_eq!(
            count_of(&kinds, point::POINT_MAX_HP),
            2,
            "strength carries no pool line of its own: {kinds:?}"
        );
    }

    /// The crossed pool lines: `iq` sends `POINT_MAX_HP`, `ht` sends `POINT_MAX_SP`.
    #[test]
    fn a_raise_crosses_the_pool_lines() {
        for (word, pool) in [
            (b"iq".as_slice(), point::POINT_MAX_HP),
            (b"ht".as_slice(), point::POINT_MAX_SP),
        ] {
            let clock = TestClock::default();
            let mut world = a_world(&clock);
            let mut inbox = entered(&mut world);
            let held = world
                .run_stat(Vid::new(VID), word, false)
                .expect("one point raises");
            assert_eq!(held.stat, 0);
            let mut frames = Vec::new();
            while let Ok(frame) = inbox.try_recv() {
                frames.push(frame);
            }
            let kinds = kinds_of(&frames);
            assert!(
                kinds.contains(&(stat_kind(pool))),
                "{word:?} carries pool {pool}: {kinds:?}"
            );
        }
    }

    /// Without a point, with an unknown word, or at the cap, the line is silent.
    #[test]
    fn a_raise_refuses_quietly() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = entered(&mut world);
        assert!(world.run_stat(Vid::new(VID), b"", false).is_none());
        assert!(world.run_stat(Vid::new(VID), b"xx", false).is_none());
        assert!(inbox.try_recv().is_err(), "no word runs nothing");
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _ = points.point_change(point::POINT_STAT, -1, false, false);
        while inbox.try_recv().is_ok() {}
        assert!(world.run_stat(Vid::new(VID), b"st", false).is_none());
        assert!(inbox.try_recv().is_err(), "no point is silent");
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _ = points.point_change(point::POINT_STAT, 5, false, false);
        points.set_both_points(point::POINT_ST, 90);
        while inbox.try_recv().is_ok() {}
        assert!(world.run_stat(Vid::new(VID), b"st", false).is_none());
        assert!(inbox.try_recv().is_err(), "the cap is silent");
    }

    /// A lower moves the attribute down, returns the point, and spends the reset.
    #[test]
    fn a_lower_returns_the_point_and_spends_the_reset() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = entered(&mut world);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        points.set_both_points(point::POINT_ST, 7);
        while inbox.try_recv().is_ok() {}
        let held = world
            .run_stat(Vid::new(VID), b"st", true)
            .expect("a raised strength lowers");
        assert_eq!(held.st, 6, "back to the warrior start");
        assert_eq!(held.stat, 2, "the point returns beside the held one");
        assert_eq!(held.reset_count, 0, "the reset is spent");
        let mut frames = Vec::new();
        while let Ok(frame) = inbox.try_recv() {
            frames.push(frame);
        }
        let kinds = kinds_of(&frames);
        assert!(kinds.contains(&(stat_kind(point::POINT_ST))), "{kinds:?}");
        assert!(kinds.contains(&(stat_kind(point::POINT_STAT))), "{kinds:?}");
        assert!(
            kinds.contains(&(stat_kind(point::POINT_STAT_RESET_COUNT))),
            "{kinds:?}"
        );
    }

    /// At the job start, or with no reset count, the lower is silent.
    #[test]
    fn a_lower_refuses_quietly() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = entered(&mut world);
        assert!(world.run_stat(Vid::new(VID), b"st", true).is_none());
        assert!(inbox.try_recv().is_err(), "the start is the floor");
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        points.set_both_points(point::POINT_ST, 7);
        let _ = points.point_change(point::POINT_STAT_RESET_COUNT, -1, false, false);
        while inbox.try_recv().is_ok() {}
        assert!(world.run_stat(Vid::new(VID), b"st", true).is_none());
        assert!(inbox.try_recv().is_err(), "no reset is silent");
    }

    /// The lower's own pool lines: `ht` sends `POINT_MAX_HP`, `iq` sends
    /// `POINT_MAX_SP`, uncrossed.
    #[test]
    fn a_lower_sends_each_pools_own_line() {
        for (word, pool, slot) in [
            (b"ht".as_slice(), point::POINT_MAX_HP, point::POINT_HT),
            (b"iq".as_slice(), point::POINT_MAX_SP, point::POINT_IQ),
        ] {
            let clock = TestClock::default();
            let mut world = a_world(&clock);
            let mut inbox = entered(&mut world);
            let points = world.pc_points_mut(VID).expect("the entrant has points");
            let start = points.real_point(slot);
            points.set_both_points(slot, start + 1);
            while inbox.try_recv().is_ok() {}
            world
                .run_stat(Vid::new(VID), word, true)
                .expect("a raised attribute lowers");
            let mut frames = Vec::new();
            while let Ok(frame) = inbox.try_recv() {
                frames.push(frame);
            }
            let kinds = kinds_of(&frames);
            assert!(
                kinds.contains(&(stat_kind(pool))),
                "{word:?} carries pool {pool}: {kinds:?}"
            );
        }
    }

    /// A morphed caller hears `[LS;521]` on `/stat`, `/stat-`, and the `con+` line,
    /// and the Romanian line on `stat_val`, with no point moved and no record but the
    /// line. The morph comes through `SetPolymorph`, so the gate reads the live race.
    #[test]
    fn a_morphed_caller_hears_the_refusal_on_every_stat_line() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = entered(&mut world);
        world.set_polymorph(Vid::new(VID), 101, false);
        while inbox.try_recv().is_ok() {}
        assert!(
            world.run_stat(Vid::new(VID), b"st", false).is_none(),
            "a morphed raise runs nothing"
        );
        assert!(
            world.run_stat(Vid::new(VID), b"st", true).is_none(),
            "a morphed lower runs nothing"
        );
        assert!(
            world
                .run_stat_plus(Vid::new(VID), b"3", point::POINT_HT)
                .is_none(),
            "a morphed con+ runs nothing"
        );
        assert!(
            world.run_stat_val(Vid::new(VID), b"st 3").is_none(),
            "a morphed stat_val runs nothing"
        );
        let mut frames = Vec::new();
        while let Ok(frame) = inbox.try_recv() {
            frames.push(frame);
        }
        let texts: Vec<Vec<u8>> = frames.iter().map(|frame| text_of(frame)).collect();
        assert_eq!(
            texts,
            [
                b"[LS;521]".to_vec(),
                b"[LS;521]".to_vec(),
                b"[LS;521]".to_vec(),
                b"@@(cmd_general)tradus:You cannot change your state as long as you are transformed"
                    .to_vec(),
            ],
            "each line refuses with its own line: {texts:?}"
        );
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        assert_eq!(points.get_point(point::POINT_STAT), 1, "no point moves");
    }

    /// `tcon` on a morphed target refuses with `[LS;521]` to the *caller*, and the
    /// target's points stay. An unmorphed target moves, so the gate reads the target.
    #[test]
    fn a_set_on_a_morphed_target_refuses_to_the_caller() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let (mut caller, mut other) = entered_pair(&mut world);
        world.set_polymorph(Vid::new(OTHER), 101, false);
        while caller.try_recv().is_ok() {}
        while other.try_recv().is_ok() {}
        assert!(
            world
                .run_set_stat(Vid::new(VID), b"P24578 3", point::POINT_HT)
                .is_none(),
            "a morphed target runs nothing"
        );
        let mut frames = Vec::new();
        while let Ok(frame) = caller.try_recv() {
            frames.push(frame);
        }
        assert_eq!(frames.len(), 1, "the caller hears one line");
        assert_eq!(text_of(&frames[0]), b"[LS;521]", "the line is the refusal");
        assert!(other.try_recv().is_err(), "the target hears nothing");
        let points = world.pc_points_mut(OTHER).expect("the other has points");
        assert_eq!(points.get_point(point::POINT_STAT), 5, "no point moves");
    }

    /// The reset fills the count to 12 with one record and answers the held row.
    #[test]
    fn a_reset_fills_the_count_to_twelve() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = entered(&mut world);
        let held = world
            .reset_stat_count(Vid::new(VID))
            .expect("a caller resets");
        assert_eq!(held.reset_count, 12, "the held row takes 12");
        let mut frames = Vec::new();
        while let Ok(frame) = inbox.try_recv() {
            frames.push(frame);
        }
        let kinds = kinds_of(&frames);
        assert_eq!(
            kinds,
            [stat_kind(point::POINT_STAT_RESET_COUNT)],
            "one counter record: {kinds:?}"
        );
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        assert_eq!(
            points.real_point(point::POINT_STAT_RESET_COUNT),
            12,
            "a counter lands the real array too"
        );
    }
}
