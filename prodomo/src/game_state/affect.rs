//! The affect event of each character (`m_pkAffectEvent`, `G/char_affect.cpp:143-162`), and the
//! timed affects it counts down (`CHARACTER::AddAffect`, `CAffect`, `G/char_affect.cpp`).
//! The same event serves a non-player that holds timed affects: the countdown only, no
//! recovery and no stamina, and the neighbourhood learns through `UpdatePacket` alone.
//!
//! One event serves one character. Every 25 Pulses it runs the potion recovery, then the
//! stamina refill, then the countdown of each timed affect, and it runs again while any of
//! them is left. It stops when no affect is left, no recovery is left, and stamina is full
//! (`CHARACTER::StartAffectEvent` is started again by a later affect or potion).
//!
//! Legacy's stamina refill runs on the legacy clock, three seconds after the body stopped. The
//! Rewrite counts the same three seconds in Pulses, 75 of them, which the Divergence in
//! `docs/STATUS.md` records.
//!
//! Each change the event makes to the affect flags is sent to the character and to every
//! character that sees it, as `GC_CHARACTER_UPDATE` (`G/char.cpp:1315`).

use common::point_slot as point;
use common::vid::Vid;
use protocol::gc_inventory::HEADER_GC_SYNC_POSITION;
use protocol::gc_nested::{GcAffectAdd, GcAffectElement};
use protocol::gc_position::GcSyncPositionElement;
use protocol::gc_vid::{GcHeaderAndDwordAndByte, HEADER_GC_AFFECT_REMOVE};
use world::character::{is_recovering, update_recovery};

use super::view::EntityKey;
use super::GameState;
use crate::item_move::Mover;
use crate::loading_phase::{
    point_changes, AFFECT_REVIVE_INVISIBLE, AFF_REVIVE_INVISIBLE, AFF_STUN,
    REVIVE_INVISIBLE_SECONDS,
};
use crate::save::PASSES_PER_SEC;

/// The Pulses a stopped body waits before the stamina refills: `3000` ms of legacy's clock at
/// 25 Pulses a second.
const STAMINA_REST_PULSES: u64 = 75;

/// One timed affect of a character, as legacy's `CAffect` keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Affect {
    /// The affect type, `wType`, such as [`AFFECT_REVIVE_INVISIBLE`].
    pub(super) kind: u32,
    /// `bApplyOn`: the point an affect applies to, 0 for none.
    pub(super) apply_on: u8,
    /// `lApplyValue`: what the affect added to its point, taken back when it ends.
    pub(super) value: i32,
    /// `dwFlag`, numbered from 1: the affect flag it sets, or 0 for none.
    pub(super) flag: u32,
    /// `lDuration` in ticks: each tick takes one away, and the affect ends at 0 or below.
    pub(super) remaining: i32,
    /// `lSPCost`: the spell points each tick spends; under them the affect ends
    /// (`ProcessAffect`, `G/char_affect.cpp:410-416`).
    pub(super) sp_cost: i32,
}

impl Affect {
    /// `ReviveInvisible(5)`: `AddAffect(215, POINT_NONE, 0, flag 28, 5, 0, true)`.
    pub(super) const fn revive_invisible() -> Self {
        Self {
            kind: AFFECT_REVIVE_INVISIBLE,
            apply_on: 0,
            value: 0,
            flag: AFF_REVIVE_INVISIBLE,
            remaining: REVIVE_INVISIBLE_SECONDS,
            sp_cost: 0,
        }
    }

    /// A skill buff's affect: `AddAffect(vnum, on, amount, flag, dur, sp_cost)`
    /// (`G/char_skill.cpp:2361-2370`).
    pub(super) fn buff(
        kind: u32,
        on: usize,
        value: i32,
        flag: u32,
        dur: i32,
        sp_cost: i32,
    ) -> Self {
        Self {
            kind,
            apply_on: u8::try_from(on).unwrap_or(u8::MAX),
            value,
            flag,
            remaining: dur,
            sp_cost,
        }
    }
}

/// The affect event of one character, and what it counts down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AffectEvent {
    /// The Pulse the event fires on next.
    pub(super) due: u64,
    /// The order the event was started in. Two events due on one Pulse run in this order.
    pub(super) sequence: u64,
    /// The timed affects, in the order they were added.
    pub(super) affects: Vec<Affect>,
}

/// The affect flag words the affects set. Flag `n` is bit `n - 1` of word `(n - 1) / 32`, so
/// the revive-invisible flag 28 is `0x0800_0000` in word 0. A flag of 0 sets nothing.
#[must_use]
pub(super) fn flags_of(affects: &[Affect]) -> [u32; 2] {
    let mut words = [0_u32; 2];
    for affect in affects {
        let Some(bit) = affect.flag.checked_sub(1) else {
            continue;
        };
        if let Some(word) = usize::try_from(bit / 32)
            .ok()
            .and_then(|index| words.get_mut(index))
        {
            *word |= 1_u32 << (bit % 32);
        }
    }
    words
}

/// One tick of the countdown (`ProcessAffect`): every affect loses a tick, and those at 0 or
/// below are taken out. The taken-out affects come back in the order they were added.
fn count_down(affects: &mut Vec<Affect>) -> Vec<Affect> {
    let mut expired = Vec::new();
    affects.retain_mut(|affect| {
        affect.remaining -= 1;
        if affect.remaining > 0 {
            return true;
        }
        expired.push(*affect);
        false
    });
    expired
}

/// Takes the affects of `kind` out of `affects`, in the order they were added.
pub(super) fn take_affects_of_kind(affects: &mut Vec<Affect>, kind: u32) -> Vec<Affect> {
    let mut taken = Vec::new();
    affects.retain(|affect| {
        if affect.kind != kind {
            return true;
        }
        taken.push(*affect);
        false
    });
    taken
}

/// The `GC_AFFECT_REMOVE` of one affect: its type, then `bApplyOn` (`G/packet.h`).
pub(super) fn affect_remove_frame(affect: &Affect) -> Vec<u8> {
    let mut frame = Vec::new();
    GcHeaderAndDwordAndByte::new(HEADER_GC_AFFECT_REMOVE, affect.kind, affect.apply_on)
        .encode_into(&mut frame);
    frame
}

/// The `GC_AFFECT_ADD` of one affect (`SendAffectAddPacket`, `G/char_affect.cpp:111-128`): the
/// duration is the ticks it has left, and the per-tick spell-point cost it carries.
pub(super) fn affect_add_frame(affect: &Affect) -> Vec<u8> {
    let mut frame = Vec::new();
    GcAffectAdd {
        header: GcAffectAdd::header(),
        element: GcAffectElement {
            affect_type: affect.kind,
            apply_on: affect.apply_on,
            apply_value: affect.value,
            flag: affect.flag,
            duration: affect.remaining,
            sp_cost: affect.sp_cost,
        },
    }
    .encode_into(&mut frame);
    frame
}

impl GameState {
    /// Starts the affect event of `vid` if it has none, due one tick from now.
    pub(super) fn start_affect_event(&mut self, vid: Vid) {
        if self.affect_events.contains_key(&vid) {
            return;
        }
        let due = self.last_pulse.saturating_add(u64::from(PASSES_PER_SEC));
        self.affect_sequence = self.affect_sequence.saturating_add(1);
        let event = AffectEvent {
            due,
            sequence: self.affect_sequence,
            affects: Vec::new(),
        };
        let _started = self.affect_events.insert(vid, event);
    }

    /// Adds `affect` to the event of `vid`, which starts if it has none.
    pub(super) fn add_affect(&mut self, vid: Vid, affect: Affect) {
        self.start_affect_event(vid);
        if let Some(event) = self.affect_events.get_mut(&vid) {
            event.affects.push(affect);
        }
    }

    /// `CHARACTER::AddAffect` on the player under `vid` (`G/char_affect.cpp:668-760`): a
    /// zero duration runs one tick; a moving body stops for the stun. With `over` and an
    /// affect of the same kind, the old row is uncomputed, its remove packet sent, and
    /// the new row is rewritten in place at its old place (`bOverride`); without it the
    /// new row appends beside the old one (two rows may share the kind — `RemoveAffect`
    /// takes all of them). Else the row appends. Then the row computes, the flags
    /// update when the flag is set or either speed moved (a set flag always updates),
    /// the event starts, and the add packet goes to the player. No STUN/SLOW/POISON/263
    /// kind is realtime, and the DB packet is the retired peer's.
    pub(super) fn add_pc_affect(&mut self, vid: Vid, mut affect: Affect, over: bool) {
        if affect.remaining == 0 {
            affect.remaining = 1;
        }
        if affect.flag == AFF_STUN {
            self.stop_for_stun(vid);
        }
        let mov_before = self.move_speed_of(vid.raw());
        let att_before = self.attack_speed_of(vid.raw());
        let retired = over.then_some(()).and_then(|()| {
            self.affect_events.get_mut(&vid).and_then(|event| {
                event
                    .affects
                    .iter_mut()
                    .find(|slot| slot.kind == affect.kind)
                    .map(|slot| {
                        let retired = *slot;
                        *slot = affect;
                        retired
                    })
            })
        });
        let placed = retired.is_some();
        if let Some(retired) = retired {
            self.change_pc_point(vid, usize::from(retired.apply_on), -retired.value, false);
            let frame = affect_remove_frame(&retired);
            let _retired = self.write_to_client(vid, frame);
        }
        self.start_affect_event(vid);
        if !placed {
            if let Some(event) = self.affect_events.get_mut(&vid) {
                event.affects.push(affect);
            }
        }
        self.change_pc_point(vid, usize::from(affect.apply_on), affect.value, false);
        let flags_after = self.affect_flags_of(vid);
        if affect.flag != 0
            || mov_before != self.move_speed_of(vid.raw())
            || att_before != self.attack_speed_of(vid.raw())
        {
            self.send_affect_update(vid, flags_after);
        }
        let frame = affect_add_frame(&affect);
        let _added = self.write_to_client(vid, frame);
    }

    /// `AddAffect`'s stun arm on the player under `vid`: a body whose destination is not
    /// its spot stops where it stands, its fight pose ends (`battle_end`), and the sync
    /// record goes around (`G/char_affect.cpp:698-707`, `G/char.cpp:3828-3846`). The
    /// duration is kept, as the arm never touches it.
    fn stop_for_stun(&mut self, vid: Vid) {
        let Some(spot) = self.spot_of(vid.raw()) else {
            return;
        };
        let stops = self
            .bodies
            .get(&vid)
            .is_some_and(|body| body.motion.dest.0 != spot.x || body.motion.dest.1 != spot.y);
        if !stops {
            return;
        }
        let point = (spot.x, spot.y);
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.motion.dest = point;
            body.motion.start = point;
            body.fighting = false;
        }
        let mut record = vec![HEADER_GC_SYNC_POSITION.value(), 0, 0];
        record.extend_from_slice(&GcSyncPositionElement::new(vid.raw(), spot.x, spot.y).encode());
        let size = u16::try_from(record.len()).unwrap_or(u16::MAX);
        record[1..3].copy_from_slice(&size.to_le_bytes());
        self.packet_around(EntityKey::Character(vid.raw()), &record, None);
    }

    /// `GetPoint(POINT_ATT_SPEED)` of the player under `vid`, or 0 when it is gone.
    fn attack_speed_of(&self, vid: u32) -> i32 {
        self.characters
            .find_by_vid(Vid::new(vid))
            .ok()
            .and_then(|character| character.points())
            .map_or(0, |points| points.get_point(point::POINT_ATT_SPEED))
    }

    /// The affect flag words `vid` has now: the ones its affects set, or 0 with no event.
    pub(super) fn affect_flags_of(&self, vid: Vid) -> [u32; 2] {
        self.affect_events
            .get(&vid)
            .map_or([0; 2], |event| flags_of(&event.affects))
    }

    /// The `GC_CHARACTER_UPDATE` of `vid` with `flags` in its affect words, or `None` when the
    /// character is not online. Its look, language and PK mode come from the world and its body.
    pub(super) fn update_frame(&self, vid: Vid, flags: [u32; 2]) -> Option<Vec<u8>> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let body = self.bodies.get(&vid)?;
        let points = character.points()?;
        let look = world::character::look_of(character.items(), points, &self.protos);
        let mover = Mover {
            recently_fought: false,
            empire: body.card.empire,
            language: body.card.language,
            pk_mode: body.card.pk_mode,
            affect_flags: flags,
            dead: self.is_dead(vid.raw()),
            alignment: i16::try_from(body.card.alignment / 10).unwrap_or(i16::MAX),
        };
        let mut frame = Vec::new();
        crate::item_move::character_update(vid.raw(), &look, mover).encode_into(&mut frame);
        Some(frame)
    }

    /// Sends `vid`'s affect-flag update to the character and to every character that sees it.
    pub(super) fn send_affect_update(&mut self, vid: Vid, flags: [u32; 2]) {
        let Some(frame) = self.update_frame(vid, flags) else {
            return;
        };
        let me = EntityKey::Character(vid.raw());
        self.packet_around(me, &frame, None);
    }

    /// `ch->ReviveInvisible(5)` at entry (`G/char.cpp:7490-7493`): the affect joins the event,
    /// and the new affect flags go to the viewers of the character, which the character's own
    /// update is also sent to. Returns that update, for the character's enter-game burst, or
    /// `None` when the character stands in no sectree: `PacketAround` sends such an entity
    /// nothing, not even its own copy (`G/entity.cpp:95`), so its burst carries none either.
    pub(super) fn enter_revive_invisible(&mut self, vid: Vid) -> Option<Vec<u8>> {
        self.add_affect(vid, Affect::revive_invisible());
        let flags = self.affect_flags_of(vid);
        let frame = self.update_frame(vid, flags)?;
        let me = EntityKey::Character(vid.raw());
        if !self.in_sectree(me) {
            return None;
        }
        self.packet_around(me, &frame, Some(me));
        Some(frame)
    }

    /// `OnMove(true)` (`G/char.cpp:6156`): an accepted attack or combo takes the revive-invisible
    /// affect off. The update goes to the viewers and the character first, then the removal to
    /// the character. The event keeps running and ends on its next run when nothing is left.
    pub(super) fn remove_revive_invisible(&mut self, vid: Vid) {
        let flags_before = self.affect_flags_of(vid);
        let removed = match self.affect_events.get_mut(&vid) {
            Some(event) => take_affects_of_kind(&mut event.affects, AFFECT_REVIVE_INVISIBLE),
            None => Vec::new(),
        };
        if removed.is_empty() {
            return;
        }
        let flags_after = self.affect_flags_of(vid);
        if flags_after != flags_before {
            self.send_affect_update(vid, flags_after);
        }
        for affect in &removed {
            let _delivered = self.write_to_client(vid, affect_remove_frame(affect));
        }
    }

    /// Runs the affect events due by `pulse`, in the order they were started.
    pub(super) fn run_affect_events(&mut self, pulse: u64) {
        let mut due: Vec<(u64, u64, Vid)> = self
            .affect_events
            .iter()
            .filter(|(_, event)| event.due <= pulse)
            .map(|(vid, event)| (event.due, event.sequence, *vid))
            .collect();
        due.sort_unstable_by_key(|&(_, sequence, _)| sequence);
        for (_, _, vid) in due {
            self.run_affect_event(vid, pulse);
        }
    }

    /// One run of the event of `vid`: the recovery, the stamina refill, the countdown, then
    /// the removals and the flag update, and the next run when something is left.
    fn run_affect_event(&mut self, vid: Vid, pulse: u64) {
        let flags_before = self.affect_flags_of(vid);
        let idle = self
            .bodies
            .get(&vid)
            .map(|body| pulse.saturating_sub(body.stop_pulse));
        let Some(points) = self
            .characters
            .find_by_vid_mut(vid)
            .ok()
            .and_then(|character| character.items_and_points_mut().1)
        else {
            let _gone = self.affect_events.remove(&vid);
            return;
        };
        let mut records = update_recovery(points);
        let recovering = is_recovering(points);
        let stamina_max = points.max_stamina();
        if points.stamina() < stamina_max && idle.is_some_and(|idle| idle >= STAMINA_REST_PULSES) {
            records.extend(
                points
                    .point_change(point::POINT_STAMINA, stamina_max, false, false)
                    .unwrap_or_default(),
            );
        }
        let stamina_full = points.stamina() == points.max_stamina();
        let sp = points.sp();
        let Some(event) = self.affect_events.get_mut(&vid) else {
            return;
        };
        // The `lSPCost` drain (`ProcessAffect`, `G/char_affect.cpp:410-416`): each
        // tick spends the cost (the SP gate reads the SP the run started with —
        // the spend never fails); an affect the SP cannot cover ends before the
        // countdown.
        let mut drained = 0;
        let mut starved: Vec<Affect> = Vec::new();
        event.affects.retain(|affect| {
            if affect.sp_cost <= 0 {
                return true;
            }
            if sp - drained < affect.sp_cost {
                starved.push(*affect);
                return false;
            }
            drained += affect.sp_cost;
            true
        });
        let expired = count_down(&mut event.affects);
        let again = recovering || !event.affects.is_empty() || !stamina_full;
        for frame in point_changes(&records, vid.raw()) {
            let _delivered = self.write_to_client(vid, frame);
        }
        // The starved affects end like expired ones (their point back, then the
        // remove packet); the drain spends after them, so a starved SP row cannot
        // cover itself.
        for affect in starved.iter().chain(expired.iter()) {
            self.change_pc_point(vid, usize::from(affect.apply_on), -affect.value, false);
            let _delivered = self.write_to_client(vid, affect_remove_frame(affect));
        }
        if drained > 0 {
            self.change_pc_point(vid, point::POINT_SP, -drained, false);
        }
        let flags_after = self.affect_flags_of(vid);
        if flags_after != flags_before {
            self.send_affect_update(vid, flags_after);
        }
        if again {
            if let Some(event) = self.affect_events.get_mut(&vid) {
                event.due = pulse.saturating_add(u64::from(PASSES_PER_SEC));
            }
        } else {
            let _ended = self.affect_events.remove(&vid);
        }
    }
    /// Starts the affect event of the non-player `vid` if it has none, due one tick from now.
    fn start_npc_affect_event(&mut self, vid: u32) {
        if self.npc_affects.contains_key(&vid) {
            return;
        }
        let due = self.last_pulse.saturating_add(u64::from(PASSES_PER_SEC));
        self.affect_sequence = self.affect_sequence.saturating_add(1);
        let event = AffectEvent {
            due,
            sequence: self.affect_sequence,
            affects: Vec::new(),
        };
        let _started = self.npc_affects.insert(vid, event);
    }

    /// The affect flag words the non-player `vid` shows now: its base words with the ones
    /// its timed affects set, or the base alone with no event. `m_afAffectFlag` holds both
    /// the spawn bit and the timed bits (`G/char.h:1078`); the Rewrite keeps the spawn
    /// words on the `Npc` and ORs the timed ones in at every read.
    pub(super) fn npc_affect_words(&self, vid: u32) -> [u32; 2] {
        let base = self.npc_by_vid(vid).map_or([0; 2], |npc| npc.affect_flags);
        let Some(event) = self.npc_affects.get(&vid) else {
            return base;
        };
        let timed = flags_of(&event.affects);
        [base[0] | timed[0], base[1] | timed[1]]
    }

    /// `CHARACTER::AddAffect` with override on the non-player under `vid`
    /// (`G/char_affect.cpp:668-770`): a zero duration runs one tick; a moving mind stops
    /// for the stun; an affect of the same kind is uncomputed, then the row is rewritten
    /// in place at its old place; else the row is appended. Then the row computes, a
    /// `MOV_SPEED` change rescales the in-flight leg (`SetPoint`), and the update goes
    /// to the neighbourhood when the flag is set or either speed moved (a set flag
    /// always updates). No add or remove packet: both are `GetDesc()` arms, and a
    /// non-player has no desc.
    pub(super) fn add_npc_affect(&mut self, vid: u32, mut affect: Affect) {
        if affect.remaining == 0 {
            affect.remaining = 1;
        }
        if affect.flag == AFF_STUN {
            self.stop_npc_for_stun(vid);
        }
        let mov_before = self.npc_point(vid, point::POINT_MOV_SPEED);
        let att_before = self.npc_point(vid, point::POINT_ATT_SPEED);
        let retired = self.npc_affects.get_mut(&vid).and_then(|event| {
            event
                .affects
                .iter_mut()
                .find(|slot| slot.kind == affect.kind)
                .map(|slot| {
                    let retired = *slot;
                    *slot = affect;
                    retired
                })
        });
        let placed = retired.is_some();
        if let Some(retired) = retired {
            self.change_npc_point(vid, usize::from(retired.apply_on), -retired.value);
        }
        self.start_npc_affect_event(vid);
        if !placed {
            if let Some(event) = self.npc_affects.get_mut(&vid) {
                event.affects.push(affect);
            }
        }
        self.change_npc_point(vid, usize::from(affect.apply_on), affect.value);
        let mov_after = self.npc_point(vid, point::POINT_MOV_SPEED);
        if mov_after != mov_before {
            self.rescale_npc_leg(vid, mov_after);
        }
        let words_after = self.npc_affect_words(vid);
        if affect.flag != 0
            || mov_before != mov_after
            || att_before != self.npc_point(vid, point::POINT_ATT_SPEED)
        {
            self.send_npc_update(vid, words_after);
        }
    }

    /// `SetPoint(POINT_MOV_SPEED)`'s mid-walk arm (`G/char.cpp:3812-3815`): while the mind's
    /// leg is still in flight, `CalculateMoveDuration` re-anchors it — the start snaps to
    /// the spot, the duration is recomputed from the spot to the destination at the new
    /// limited speed, and the start time is now (`:3585-3603`). A leg already arrived
    /// (`now >= start + duration`) keeps its times: the next `StateMove` arrives it.
    fn rescale_npc_leg(&mut self, vid: u32, move_speed: i32) {
        let Some(spot) = self.npc_spot(vid) else {
            return;
        };
        let now = self.clock.now();
        let Some(mind) = self.minds.get_mut(vid) else {
            return;
        };
        let elapsed = now.wrapping_sub(mind.motion.start_ms);
        if elapsed >= mind.motion.duration_ms {
            return;
        }
        mind.motion.start = spot;
        mind.motion.duration_ms = super::motion::move_duration(
            spot,
            mind.motion.dest,
            super::motion::MOTION_SPEED_FALLBACK,
            super::mob_battle::npc_limit_speed(move_speed),
        );
        mind.motion.start_ms = now;
    }

    /// `AddAffect`'s stun arm on the non-player under `vid`: a mind whose destination is
    /// not its spot stops where it stands, its fight pose ends (`battle_end`), and the
    /// sync record goes around (`G/char_affect.cpp:690-701`). The Rewrite keeps no
    /// `m_dwMoveDuration` to leave untouched: `Motion` carries its own.
    fn stop_npc_for_stun(&mut self, vid: u32) {
        let Some(spot) = self.npc_spot(vid) else {
            return;
        };
        let stops = self
            .minds
            .get(vid)
            .is_some_and(|mind| mind.motion.dest.0 != spot.0 || mind.motion.dest.1 != spot.1);
        if !stops {
            return;
        }
        let point = (spot.0, spot.1);
        if let Some(mind) = self.minds.get_mut(vid) {
            mind.motion.dest = point;
            mind.motion.start = point;
            mind.fighting = false;
        }
        let mut record = vec![HEADER_GC_SYNC_POSITION.value(), 0, 0];
        record.extend_from_slice(&GcSyncPositionElement::new(vid, spot.0, spot.1).encode());
        let size = u16::try_from(record.len()).unwrap_or(u16::MAX);
        record[1..3].copy_from_slice(&size.to_le_bytes());
        self.packet_around(EntityKey::Npc(vid), &record, None);
    }

    /// `GetPoint` of the non-player under `vid`, or 0 when it is gone.
    fn npc_point(&self, vid: u32, slot: usize) -> i32 {
        self.npc_by_vid(vid).map_or(0, |npc| npc.fight.point(slot))
    }

    /// `PointChange` of the non-player under `vid`: the slot moves, with no packet — the
    /// caller sends the one update for the whole change. Slot 0 (`POINT_NONE`) changes
    /// nothing but still lands in the row, as `ComputeAffect` still writes the field.
    fn change_npc_point(&mut self, vid: u32, slot: usize, value: i32) {
        if slot == point::POINT_NONE {
            return;
        }
        if let Some(npc) = self.npc_mut(vid) {
            npc.fight.add(slot, value);
        }
    }

    /// `UpdatePacket` of the non-player under `vid` with `words` in its affect words
    /// (`G/char.cpp:1277-1340`): its limited live speeds, and 0 for every worn part,
    /// guild, mount, premium and alignment — a monster wears and owns nothing. Sent to
    /// the neighbourhood only, as the victim has no desc.
    pub(super) fn send_npc_update(&mut self, vid: u32, words: [u32; 2]) {
        let Some(npc) = self.npc_by_vid(vid) else {
            return;
        };
        let look = world::character::CharacterLook {
            parts: [0; world::character::PARTS],
            moving_speed: u8::try_from(super::mob_battle::npc_limit_speed(
                npc.fight.point(point::POINT_MOV_SPEED),
            ))
            .unwrap_or(u8::MAX),
            attack_speed: u8::try_from(super::mob_battle::npc_limit_speed(
                npc.fight.point(point::POINT_ATT_SPEED),
            ))
            .unwrap_or(u8::MAX),
            level: u32::from(npc.fight.level),
            conqueror_level: 0,
            refine_element_type: 0,
        };
        let mover = Mover {
            recently_fought: false,
            empire: npc.empire,
            language: 0,
            pk_mode: 0,
            affect_flags: words,
            dead: npc.dead,
            alignment: 0,
        };
        let mut frame = Vec::new();
        crate::item_move::character_update(vid, &look, mover).encode_into(&mut frame);
        self.packet_around(EntityKey::Npc(vid), &frame, None);
    }

    /// Runs the affect events of the non-players due by `pulse`, in the order they were
    /// started. Shares the sequence with the players' events: a monster's and a player's
    /// event due on one Pulse run in start order.
    pub(super) fn run_npc_affect_events(&mut self, pulse: u64) {
        let mut due: Vec<(u64, u64, u32)> = self
            .npc_affects
            .iter()
            .filter(|(_, event)| event.due <= pulse)
            .map(|(vid, event)| (event.due, event.sequence, *vid))
            .collect();
        due.sort_unstable_by_key(|&(_, sequence, _)| sequence);
        for (_, _, vid) in due {
            self.run_npc_affect_event(vid, pulse);
        }
    }

    /// One run of the affect event of the non-player `vid`: the countdown, then the
    /// removals and the flag update, and the next run while any affect is left. No
    /// recovery, no stamina, no `CheckMaximumPoints`: nothing here touches a maximum,
    /// and a monster holds neither recovery points nor stamina.
    fn run_npc_affect_event(&mut self, vid: u32, pulse: u64) {
        let words_before = self.npc_affect_words(vid);
        let mov_before = self.npc_point(vid, point::POINT_MOV_SPEED);
        let att_before = self.npc_point(vid, point::POINT_ATT_SPEED);
        let Some(event) = self.npc_affects.get_mut(&vid) else {
            return;
        };
        let expired = count_down(&mut event.affects);
        let again = !event.affects.is_empty();
        // `ComputeAffect(pkAff, false)` on each expired affect (`ProcessAffect`): the point
        // is taken back, with no remove packet for a non-player, and the flags update
        // once, after all of them.
        for affect in &expired {
            self.change_npc_point(vid, usize::from(affect.apply_on), -affect.value);
        }
        let mov_after = self.npc_point(vid, point::POINT_MOV_SPEED);
        if mov_after != mov_before {
            self.rescale_npc_leg(vid, mov_after);
        }
        let words_after = self.npc_affect_words(vid);
        if words_after != words_before
            || mov_before != mov_after
            || att_before != self.npc_point(vid, point::POINT_ATT_SPEED)
        {
            self.send_npc_update(vid, words_after);
        }
        if again {
            if let Some(event) = self.npc_affects.get_mut(&vid) {
                event.due = pulse.saturating_add(u64::from(PASSES_PER_SEC));
            }
        } else {
            let _ended = self.npc_affects.remove(&vid);
        }
    }

    /// `ClearAffect(true)` on the non-player under `vid` (`G/char_battle.cpp:1370`): kinds
    /// 209/210/211 go — none is in the no-clear tables — each computed off. `update`
    /// sends the one `UpdatePacket` for the clear; death clears without one, before
    /// `GC_DEAD`.
    pub(super) fn clear_npc_affects(&mut self, vid: u32, update: bool) {
        let words_before = self.npc_affect_words(vid);
        let taken = match self.npc_affects.get_mut(&vid) {
            Some(event) => std::mem::take(&mut event.affects),
            None => Vec::new(),
        };
        if taken.is_empty() {
            return;
        }
        for affect in &taken {
            self.change_npc_point(vid, usize::from(affect.apply_on), -affect.value);
        }
        let _ended = self.npc_affects.remove(&vid);
        if update {
            self.rescale_npc_leg(vid, self.npc_point(vid, point::POINT_MOV_SPEED));
            let words_after = self.npc_affect_words(vid);
            if words_after != words_before {
                self.send_npc_update(vid, words_after);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{count_down, flags_of, Affect};
    use crate::game_loop::PulseProcessor;
    use crate::game_state::fixtures::{a_world, enter_on, TestClock, PLACE};
    use crate::game_state::GameState;
    use crate::loading_phase::AFFECT_REVIVE_INVISIBLE;
    use common::vid::Vid;

    #[test]
    fn the_revive_invisible_flag_is_bit_twenty_seven_of_word_zero() {
        let affects = [Affect::revive_invisible()];
        assert_eq!(flags_of(&affects), [0x0800_0000, 0]);
    }

    #[test]
    fn a_flag_past_thirty_two_sets_the_second_word() {
        let affect = Affect {
            flag: 33,
            ..Affect::revive_invisible()
        };
        assert_eq!(flags_of(&[affect]), [0, 1]);
    }

    #[test]
    fn a_flag_of_zero_sets_nothing_and_no_affect_sets_nothing() {
        let affect = Affect {
            flag: 0,
            ..Affect::revive_invisible()
        };
        assert_eq!(flags_of(&[affect]), [0, 0]);
        assert_eq!(flags_of(&[]), [0, 0]);
    }

    #[test]
    fn the_countdown_ends_the_revive_invisible_affect_on_the_fifth_tick() {
        let mut affects = vec![Affect::revive_invisible()];
        for _ in 0..4 {
            assert!(count_down(&mut affects).is_empty());
            assert_eq!(affects.len(), 1);
        }
        let expired = count_down(&mut affects);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].kind, AFFECT_REVIVE_INVISIBLE);
        assert_eq!(expired[0].remaining, 0);
        assert!(affects.is_empty());
    }

    #[test]
    fn a_treeless_entrant_gets_no_update_in_its_burst_and_its_affect_still_starts() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        // Map 77 is hosted by no index, so the body is placed treeless (V4).
        let _inbox = enter_on(&mut world, (PLACE.0, 77), 1, (5, 5), 12_345);
        assert_eq!(world.enter_revive_invisible(Vid::new(1)), None);
        assert_eq!(world.affect_flags_of(Vid::new(1)), [0x0800_0000, 0]);
    }

    /// The stamina `vid` has now, and the maximum it refills to.
    fn stamina_of(world: &GameState, vid: Vid) -> (i32, i32) {
        let points = world
            .characters()
            .find_by_vid(vid)
            .expect("the entrant is in the world")
            .points()
            .expect("the entrant has points");
        (points.stamina(), points.max_stamina())
    }

    #[test]
    fn the_stamina_refill_comes_on_the_seventy_fifth_idle_pulse_and_not_before() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let vid = Vid::new(1);
        // The body stops on Pulse 0 and its event starts there, due on 25, 50 and 75.
        let _inbox = enter_on(&mut world, PLACE, 1, (5, 5), 10);
        world.start_affect_event(vid);
        world.run_affect_events(25);
        world.run_affect_events(50);
        let (stamina, max) = stamina_of(&world, vid);
        assert!(stamina < max, "the entrant starts below its maximum");
        assert_eq!(
            stamina, 10,
            "50 idle Pulses is short of the 75 the refill waits"
        );
        world.run_affect_events(75);
        assert_eq!(
            stamina_of(&world, vid),
            (max, max),
            "75 idle Pulses refills it"
        );
    }

    #[test]
    fn a_body_stopped_a_pulse_late_is_refilled_on_the_hundredth_pulse_not_the_seventy_fifth() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let vid = Vid::new(1);
        // The event starts on Pulse 0, due on 25, 50, 75 and 100. The body stops on Pulse 1, so
        // its idle counts at those runs are 24, 49, 74 and 99.
        world.start_affect_event(vid);
        world.process_pulse(1);
        let _inbox = enter_on(&mut world, PLACE, 1, (5, 5), 10);
        world.run_affect_events(25);
        world.run_affect_events(50);
        world.run_affect_events(75);
        let (stamina, max) = stamina_of(&world, vid);
        assert_eq!(
            stamina, 10,
            "74 idle Pulses is short of the 75 the refill waits"
        );
        assert!(stamina < max, "the entrant is still below its maximum");
        world.run_affect_events(100);
        assert_eq!(
            stamina_of(&world, vid),
            (max, max),
            "99 idle Pulses refills it"
        );
    }

    #[test]
    fn two_revives_that_end_on_one_pulse_send_their_updates_in_the_order_they_started() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut watcher = enter_on(&mut world, PLACE, 3, (5, 5), 10_000);
        let _started_first = enter_on(&mut world, PLACE, 2, (6, 5), 10_000);
        let _started_second = enter_on(&mut world, PLACE, 1, (7, 5), 10_000);
        // Both start on Pulse 0, due on 25 and so on, and end together on the fifth tick, 125.
        // The VID order is the reverse of the order they started in.
        let _burst = world.enter_revive_invisible(Vid::new(2));
        let _burst = world.enter_revive_invisible(Vid::new(1));
        while watcher.try_recv().is_ok() {}
        for pulse in [25, 50, 75, 100, 125] {
            world.run_affect_events(pulse);
        }
        let updated: Vec<u32> = std::iter::from_fn(|| watcher.try_recv().ok())
            .filter(|frame| frame.first() == Some(&19))
            .map(|frame| u32::from_le_bytes(frame[1..5].try_into().expect("four bytes")))
            .collect();
        assert_eq!(updated, [2, 1], "the revive that started first ends first");
    }

    #[test]
    fn each_started_event_takes_the_next_sequence_number() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        world.start_affect_event(Vid::new(1));
        world.start_affect_event(Vid::new(2));
        let first = world.affect_events[&Vid::new(1)].sequence;
        let second = world.affect_events[&Vid::new(2)].sequence;
        assert!(
            second > first,
            "a later event runs after an earlier one due on the same Pulse"
        );
    }

    #[test]
    fn two_affects_that_end_together_leave_in_the_order_they_were_added() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter_on(&mut world, PLACE, 1, (5, 5), 10_000);
        let vid = Vid::new(1);
        // Two affects of different kinds, both on their last tick, so one run ends both.
        world.add_affect(
            vid,
            Affect {
                kind: 7,
                apply_on: 1,
                value: 0,
                flag: 0,
                remaining: 1,
                sp_cost: 0,
            },
        );
        world.add_affect(
            vid,
            Affect {
                kind: 9,
                apply_on: 2,
                value: 0,
                flag: 0,
                remaining: 1,
                sp_cost: 0,
            },
        );
        while inbox.try_recv().is_ok() {}
        world.run_affect_events(25);
        let removed: Vec<(u32, u8)> = std::iter::from_fn(|| inbox.try_recv().ok())
            .filter(|frame| frame.first() == Some(&127))
            .map(|frame| {
                let kind = u32::from_le_bytes(frame[1..5].try_into().expect("four bytes"));
                (kind, frame[5])
            })
            .collect();
        assert_eq!(
            removed,
            [(7, 1), (9, 2)],
            "the affects leave in the order they were added"
        );
    }

    /// A 20-second slow ends on its 20th tick: the move speed is back, with the remove
    /// and the flag update.
    #[test]
    fn a_slow_that_ends_gives_the_move_speed_back_with_a_remove_and_an_update() {
        use crate::loading_phase::{AFFECT_SLOW, AFF_SLOW, SLOW_MOV_SPEED, SLOW_TICKS};
        use common::point_slot as point;
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter_on(&mut world, PLACE, 1, (5, 5), 10_000);
        let vid = Vid::new(1);
        let before = world.move_speed_of(1);
        world.add_pc_affect(
            vid,
            Affect {
                kind: AFFECT_SLOW,
                apply_on: u8::try_from(point::POINT_MOV_SPEED).unwrap(),
                value: SLOW_MOV_SPEED,
                flag: AFF_SLOW,
                remaining: SLOW_TICKS,
                sp_cost: 0,
            },
            true,
        );
        assert_eq!(world.move_speed_of(1), (before - 30).max(0));
        while inbox.try_recv().is_ok() {}
        for tick in 1..=20 {
            world.run_affect_events(u64::try_from(tick).unwrap() * 25);
        }
        assert_eq!(world.move_speed_of(1), before);
        let heard: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert!(heard.iter().any(|frame| frame.first() == Some(&0x7f)));
        assert!(heard.iter().any(|frame| frame.first() == Some(&19)));
    }

    /// An added stun stops a moving body at its spot: the start and destination are
    /// the spot, the fight pose ends, and the sync record goes around.
    #[test]
    fn a_stun_stops_a_moving_body_at_its_spot_with_a_sync_record() {
        use crate::game_state::fixtures::enter;
        use crate::loading_phase::{AFFECT_STUN, AFF_STUN, STUN_TICKS};
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut watcher = enter_on(&mut world, PLACE, 2, (6, 5), 10_000);
        let _entrant = enter(&mut world, 1, (5, 5), 10_000);
        assert!(world.goto(1, 50, 50));
        let vid = Vid::new(1);
        let spot = world.spot_of(1).unwrap();
        world.add_pc_affect(
            vid,
            Affect {
                kind: AFFECT_STUN,
                apply_on: 0,
                value: 0,
                flag: AFF_STUN,
                remaining: STUN_TICKS,
                sp_cost: 0,
            },
            true,
        );
        let body = world.bodies.get(&vid).unwrap();
        assert_eq!(body.motion.dest, (spot.x, spot.y));
        assert_eq!(body.motion.start, (spot.x, spot.y));
        assert!(!body.fighting);
        let heard: Vec<Vec<u8>> = std::iter::from_fn(|| watcher.try_recv().ok()).collect();
        assert!(heard.iter().any(|frame| frame.first() == Some(&0x05)));
    }

    /// A stun added over a running stun rewrites it in place: one row with the new
    /// duration, one remove and one add.
    #[test]
    fn a_stun_over_a_running_stun_rewrites_it_in_place() {
        use crate::loading_phase::{AFFECT_STUN, AFF_STUN, STUN_TICKS};
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter_on(&mut world, PLACE, 1, (5, 5), 10_000);
        let vid = Vid::new(1);
        let stun = Affect {
            kind: AFFECT_STUN,
            apply_on: 0,
            value: 0,
            flag: AFF_STUN,
            remaining: 1,
            sp_cost: 0,
        };
        world.add_pc_affect(vid, stun, true);
        world.run_affect_events(25);
        // The one-tick stun ran its tick; re-add it running, then overwrite it.
        world.add_pc_affect(vid, stun, true);
        while inbox.try_recv().is_ok() {}
        world.add_pc_affect(
            vid,
            Affect {
                remaining: STUN_TICKS,
                ..stun
            },
            true,
        );
        let event = world.affect_events.get(&vid).unwrap();
        let stuns: Vec<Affect> = event
            .affects
            .iter()
            .filter(|affect| affect.kind == AFFECT_STUN)
            .copied()
            .collect();
        assert_eq!(stuns.len(), 1);
        assert_eq!(stuns[0].remaining, STUN_TICKS);
        let heard: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        let removes = heard
            .iter()
            .filter(|frame| frame.first() == Some(&0x7f))
            .count();
        let adds = heard
            .iter()
            .filter(|frame| frame.first() == Some(&0x7e))
            .count();
        assert_eq!((removes, adds), (1, 1));
    }

    /// A stood NPC for the non-player affect tests: race 101 at (3200, 3200).
    fn stood_npc(world: &mut GameState, vid: u32) {
        use crate::game_state::fixtures::{open_cells, region};
        use gamedata::mob_group::MobGroups;
        use gamedata::mob_locale_names::MobLocaleNames;
        use gamedata::mob_proto::{MobProtos, CHAR_TYPE_MONSTER};
        use std::sync::Arc;
        use world::npc::{MapNpcs, Npc, NpcVids, Respawner, FIRST_NPC_VID};
        let respawner = Respawner::new(
            Arc::new(MobProtos::from_rows(Vec::new())),
            Arc::new(MobGroups::default()),
            Arc::new(MobLocaleNames::parse(b"VNUM\tNAME\n").unwrap()),
            NpcVids::starting_at(FIRST_NPC_VID + 100),
        );
        world.install_respawner(respawner, open_cells());
        world.stand_npcs(
            PLACE.0,
            &region(),
            MapNpcs {
                npcs: vec![Npc {
                    vid,
                    vnum: 101,
                    race: 101,
                    char_type: CHAR_TYPE_MONSTER,
                    x: 3200,
                    y: 3200,
                    hp: 100,
                    max_hp: 100,
                    ..Npc::default()
                }],
                positions: Vec::new(),
                regens: Vec::new(),
            },
        );
    }

    /// A non-player slow ticks down and gives the speed back on its last tick with one
    /// neighbourhood update; the event ends with the last affect.
    #[test]
    fn a_npc_slow_ticks_down_and_ends_its_event() {
        use crate::loading_phase::{AFFECT_SLOW, AFF_SLOW, SLOW_MOV_SPEED};
        use common::point_slot as point;
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        stood_npc(&mut world, 1000);
        let mut inbox = enter_on(&mut world, PLACE, 1, (3100, 3200), 10_000);
        world.add_npc_affect(
            1000,
            Affect {
                kind: AFFECT_SLOW,
                apply_on: u8::try_from(point::POINT_MOV_SPEED).unwrap(),
                value: SLOW_MOV_SPEED,
                flag: AFF_SLOW,
                remaining: 2,
                sp_cost: 0,
            },
        );
        assert_eq!(
            world
                .npc_by_vid(1000)
                .unwrap()
                .fight
                .point(point::POINT_MOV_SPEED),
            -30
        );
        while inbox.try_recv().is_ok() {}
        let due = world.npc_affects[&1000].due;
        world.run_npc_affect_events(due);
        assert_eq!(world.npc_affects[&1000].affects[0].remaining, 1);
        assert_eq!(
            world
                .npc_by_vid(1000)
                .unwrap()
                .fight
                .point(point::POINT_MOV_SPEED),
            -30
        );
        while inbox.try_recv().is_ok() {}
        let due = world.npc_affects[&1000].due;
        world.run_npc_affect_events(due);
        assert_eq!(
            world
                .npc_by_vid(1000)
                .unwrap()
                .fight
                .point(point::POINT_MOV_SPEED),
            0
        );
        assert!(!world.npc_affects.contains_key(&1000));
        let heard: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert_eq!(
            heard
                .iter()
                .filter(|frame| frame.first() == Some(&19))
                .count(),
            1,
            "one update: {heard:02x?}"
        );
        assert!(
            heard.iter().all(|frame| frame.first() != Some(&0x7e)),
            "no add for a non-player: {heard:02x?}"
        );
    }

    /// A non-player stun over a running stun rewrites it in place: one row, no add or
    /// remove, the words unchanged.
    #[test]
    fn a_npc_stun_over_a_running_stun_rewrites_it_in_place() {
        use crate::loading_phase::{AFFECT_STUN, AFF_STUN};
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        stood_npc(&mut world, 1000);
        let mut inbox = enter_on(&mut world, PLACE, 1, (3100, 3200), 10_000);
        let stun = Affect {
            kind: AFFECT_STUN,
            apply_on: 0,
            value: 0,
            flag: AFF_STUN,
            remaining: 4,
            sp_cost: 0,
        };
        world.add_npc_affect(1000, stun);
        let words = world.npc_affect_words(1000);
        while inbox.try_recv().is_ok() {}
        world.add_npc_affect(
            1000,
            Affect {
                remaining: 2,
                ..stun
            },
        );
        let event = world.npc_affects.get(&1000).unwrap();
        assert_eq!(event.affects.len(), 1);
        assert_eq!(event.affects[0].remaining, 2);
        assert_eq!(world.npc_affect_words(1000), words);
        let heard: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert!(
            heard
                .iter()
                .all(|frame| !matches!(frame.first(), Some(0x7e | 0x7f))),
            "no add or remove: {heard:02x?}"
        );
    }
}
