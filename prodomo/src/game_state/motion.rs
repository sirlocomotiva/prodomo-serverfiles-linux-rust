//! How a player's body walks, as `CHARACTER::Goto`, `Move`, `Sync`, `Stop` and `StateMove` move
//! it (`G/char.cpp:3384-3622`, `G/char_state.cpp:782-904`).
//!
//! A `FUNC_MOVE` only sets a destination and a duration. Every Pulse, [`GameState::step_motion`]
//! puts each moving body at the point the elapsed share of its duration reaches; every sixteenth
//! Pulse it also recomputes the body's view and measures the body's own trade. Every other move
//! function moves the body at once. Moving a body changes its sectree but never its view, as
//! legacy's `Sync` does (`G/char.cpp:3437-3455`).
//!
//! The motion speed is legacy's fall-back of 300 for every body (V2): the `.msa` motions that
//! give each race and weapon its own are not loaded yet.
//!
//! Everything here runs on the game thread only (ADR-0002).

use std::cmp::Ordering;
use std::fmt;
use std::time::Duration;

use common::cfloat::{f32_to_i32, i32_to_f32, u32_to_f32};
use common::point_slot::POINT_MOV_SPEED;
use common::vid::Vid;
use world::character::{Character, EXCHANGE_MAX_DISTANCE};

use protocol::cg_move::CgMove;

use super::view::{EntityKey, MapIndex, Spot};
use super::GameState;
use crate::client_live::{BootLiveClock, LiveClock};
use crate::game_loop_messages::{EnterPlace, GameCommand, GroundPlace};
use crate::loading_phase::{PcCard, PK_MODE_PEACE};
use crate::movement::{
    judge_move, move_relay, MoveContext, MoveDisposition, MoveOutcome, MoveRefusal, FUNC_ATTACK,
    FUNC_COMBO,
};
use crate::sync_position::distance_approx;
use tracing::{debug, info, warn};

/// The motion speed `GetMoveMotionSpeed` falls back to when the race has no motion for the
/// mode (`G/char.cpp:3576`): every body's, until the motions are loaded (V2).
pub(super) const MOTION_SPEED_FALLBACK: f32 = 300.0;

/// The mask `StateMove` samples the view and the trade on: `(thecore_pulse() & 15) == 0`
/// (`G/char_state.cpp:795`).
const SAMPLE_PULSE_MASK: u64 = 15;

/// `FUNC_ATTACK` and `FUNC_COMBO`'s `OnMove(true)` blocks an equip this long after
/// (`G/char_item.cpp:8416-8422`).
pub(super) const RECENT_ATTACK_MS: u32 = 1500;

/// The world's clock: `get_dword_time()` for the motion, and the span a sync owner holds.
pub(super) struct WorldClock(Box<dyn LiveClock + Send>);

impl WorldClock {
    /// The world's clock reading `clock`.
    pub(super) fn new(clock: Box<dyn LiveClock + Send>) -> Self {
        Self(clock)
    }

    /// `get_dword_time()`: milliseconds, wrapping as a `DWORD`.
    pub(super) fn now(&self) -> u32 {
        self.0.now()
    }

    /// The time since the clock's start, which does not wrap.
    pub(super) fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

impl Default for WorldClock {
    fn default() -> Self {
        Self::new(Box::new(BootLiveClock::new()))
    }
}

impl fmt::Debug for WorldClock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WorldClock")
    }
}

/// `m_posStart`, `m_posDest`, `m_dwMoveStartTime`, `m_dwMoveDuration`, and whether the body is
/// in the Move state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Motion {
    pub(super) start: (i32, i32),
    pub(super) dest: (i32, i32),
    pub(super) start_ms: u32,
    pub(super) duration_ms: u32,
    pub(super) moving: bool,
}

impl Motion {
    /// A body standing at `point`, as `Show` leaves it.
    const fn standing(point: (i32, i32)) -> Self {
        Self {
            start: point,
            dest: point,
            start_ms: 0,
            duration_ms: 0,
            moving: false,
        }
    }

    /// `StateMove`'s point at `now` (`G/char_state.cpp:782-791`), and whether it is the arrival.
    /// `None` when the rate is not a number (0 / 0), which legacy turns into `INT_MIN` and the
    /// Rewrite skips (D1).
    pub(super) fn at(&self, now: u32) -> Option<((i32, i32), bool)> {
        let elapsed = u32_to_f32(now.wrapping_sub(self.start_ms));
        let mut rate = elapsed / u32_to_f32(self.duration_ms);
        if rate.is_nan() {
            return None;
        }
        if rate > 1.0 {
            rate = 1.0;
        }
        let axis = |from: i32, to: i32| {
            f32_to_i32(i32_to_f32(to.saturating_sub(from)) * rate + i32_to_f32(from))
        };
        let point = (
            axis(self.start.0, self.dest.0),
            axis(self.start.1, self.dest.1),
        );
        // After the clamp, `>= 1.0` is legacy's `1.0f == fRate` (`:897`).
        Some((point, rate >= 1.0))
    }
}

/// A player's body: what the world knows of a player standing on a map, apart from its spot,
/// which lives in the map's index.
#[derive(Debug)]
pub(super) struct Body {
    /// The Channel it stands on.
    pub(super) channel: u8,
    /// The map it stands on.
    pub(super) map: i32,
    /// `GetRotation()` in degrees, 0.0 until the first move.
    pub(super) rotation: f32,
    pub(super) motion: Motion,
    /// `m_dwLastAttackTime`, stamped by an accepted ATTACK or COMBO (`G/char.cpp:6148-6179`).
    pub(super) last_attack: Option<u32>,
    /// The `SyncPosition` hack count (`m_iSyncHackCount`).
    pub(super) sync_hack_count: u32,
    /// `m_dwLastSyncTime`, on the world clock's unwrapped span.
    pub(super) last_sync: Option<Duration>,
    /// `m_pkSyncOwner` by VID, and when it claimed this body.
    pub(super) sync_owner: Option<(u32, Duration)>,
    /// What its insert records say that walking does not change.
    pub(super) card: PcCard,
    /// The Pulse the body last stopped moving on, which the stamina refill counts three seconds
    /// from (`m_dwStopTime`, in Pulses; see [`crate::game_state::affect`]).
    pub(super) stop_pulse: u64,
}

impl Body {
    /// A body that has just entered at `point` on `map` of `channel`, standing.
    pub(super) const fn new(
        channel: u8,
        map: i32,
        point: (i32, i32),
        card: PcCard,
        stop_pulse: u64,
    ) -> Self {
        Self {
            channel,
            map,
            rotation: 0.0,
            motion: Motion::standing(point),
            last_attack: None,
            sync_hack_count: 0,
            last_sync: None,
            sync_owner: None,
            card,
            stop_pulse,
        }
    }
}

/// `CalculateDuration(iSpd, iDur)` (`G/utils.cpp:189-201`): a duration scaled by a speed, where
/// 100 is the duration itself.
pub(super) fn calculate_duration(speed: i32, duration: i32) -> i32 {
    let i = 100_i64 - i64::from(speed);
    let factor = match i.cmp(&0) {
        Ordering::Greater => 100 + i,
        Ordering::Less => 10_000 / (100 - i),
        Ordering::Equal => 100,
    };
    // The limit holds the speed to 0..200, so the product fits and the fall-back is never taken.
    i32::try_from(i64::from(duration) * factor / 100).unwrap_or(i32::MAX)
}

/// `DISTANCE_SQRT(dx, dy)` (`G/utils.h:12-15`): the squares and the root in `float`.
pub(super) fn distance_sqrt(dx: i32, dy: i32) -> f32 {
    let fx = i32_to_f32(dx);
    let fy = i32_to_f32(dy);
    (fx * fx + fy * fy).sqrt()
}

/// `CalculateMoveDuration`'s `m_dwMoveDuration` (`G/char.cpp:3585-3603`).
pub(super) fn move_duration(
    start: (i32, i32),
    dest: (i32, i32),
    motion_speed: f32,
    move_speed: i32,
) -> u32 {
    let distance = distance_sqrt(
        start.0.saturating_sub(dest.0),
        start.1.saturating_sub(dest.1),
    );
    let millis = f32_to_i32((distance / motion_speed) * 1000.0);
    // `m_dwMoveDuration` is a `DWORD` assigned from an `int`.
    u32::from_ne_bytes(calculate_duration(move_speed, millis).to_ne_bytes())
}

/// `GetDegreeFromPositionXY(sx, sy, ex, ey)` with `(dx, dy) = (ex - sx, ey - sy)`
/// (`G/vector.cpp:25-51`): the angle from north, clockwise, in `float`.
///
/// `Normalize` adds the double `1.0e-12` under the root; for an integer vector that is not zero
/// the sum is at least 1, so adding it in `float` rounds to the same value, and a zero vector
/// gives 90 in both.
pub(super) fn rotation_to_xy(dx: i32, dy: i32) -> f32 {
    let x = i32_to_f32(dx);
    let y = i32_to_f32(dy);
    let length = (x * x + y * y + 1.0e-12_f32).sqrt();
    let (nx, ny) = (x / length, y / length);
    // `_PI` is `(float) 3.141592654f`, which is `f32::consts::PI` exactly.
    let degrees = ny.acos() * (180.0_f32 / std::f32::consts::PI);
    if nx < 0.0 {
        360.0 - degrees
    } else {
        degrees
    }
}

/// `IsWalking()` for a player: `m_bNowWalking || GetStamina() <= 0` (`G/char.h:951`).
/// `m_bNowWalking` changes only through the stamina arm of `PointChange`, which is not ported,
/// so a player walks exactly when its stamina is spent (V3). A player with no points runs.
pub(super) fn is_walking(character: &Character) -> bool {
    character
        .points()
        .is_some_and(|points| points.stamina() <= 0)
}

/// Logs what one `CG_MOVE` of the player under `vid` did.
fn log_move(vid: Vid, outcome: Option<&MoveOutcome>) {
    match outcome {
        None => debug!(?vid, "MOVE from a character with no body; ignoring"),
        Some(MoveOutcome::Ignore(reason)) => {
            info!(?vid, reason, "MOVE consumed without a record");
        }
        Some(MoveOutcome::Refuse(MoveRefusal::CannotMove)) => {
            info!(?vid, "MOVE refused: cannot move");
        }
        Some(&MoveOutcome::Refuse(MoveRefusal::InvalidFunction { function })) => {
            warn!(?vid, function, "MOVE refused: invalid function byte");
        }
        Some(&MoveOutcome::Refuse(MoveRefusal::TooFar { distance })) => {
            warn!(?vid, distance, "MOVE refused: too far");
        }
        Some(MoveOutcome::Refuse(MoveRefusal::Dead)) => {
            info!(?vid, "MOVE refused: dead character");
        }
        Some(MoveOutcome::Accept(_)) => {}
    }
}

impl GameState {
    /// The Channel and map the body under `vid` stands on.
    pub(super) fn place_of(&self, vid: u32) -> Option<(u8, i32)> {
        self.bodies
            .get(&Vid::new(vid))
            .map(|body| (body.channel, body.map))
    }

    /// The PK mode the body under `vid` shows, or `PK_MODE_PEACE` with no body.
    pub(super) fn pk_mode_of(&self, vid: u32) -> u8 {
        self.bodies
            .get(&Vid::new(vid))
            .map_or(PK_MODE_PEACE, |body| body.card.pk_mode)
    }

    /// Where the body under `vid` stands.
    pub(super) fn spot_of(&self, vid: u32) -> Option<Spot> {
        let place = self.place_of(vid)?;
        self.maps.get(&place)?.spot(EntityKey::Character(vid))
    }

    /// `GetLimitPoint(POINT_MOV_SPEED)` of the character under `vid`, or 0 with no points.
    pub(super) fn move_speed_of(&self, vid: u32) -> i32 {
        self.characters
            .find_by_vid(Vid::new(vid))
            .ok()
            .and_then(Character::points)
            .map_or(0, |points| points.limit_point(POINT_MOV_SPEED))
    }

    /// `CHARACTER::Show` for a player's body (`G/char.cpp:1847-1917`): the view work, then
    /// `m_posDest = m_posStart` at the point, with the records delivered after both, so an
    /// insert reads the body standing. The entrant's own records go to `reply` when one is
    /// given. Returns false, changing nothing, when no sectree holds the point.
    pub(super) fn show_body(
        &mut self,
        vid: u32,
        at: (i32, i32, i32),
        reply: Option<&mut Vec<Vec<u8>>>,
    ) -> bool {
        let Some(place) = self.place_of(vid) else {
            return false;
        };
        let radius = self.view_radius;
        let mut effects = Vec::new();
        let Some(index) = self.maps.get_mut(&place) else {
            return false;
        };
        if !index.show(EntityKey::Character(vid), at, radius, &mut effects) {
            return false;
        }
        if let Some(body) = self.bodies.get_mut(&Vid::new(vid)) {
            body.motion.start = (at.0, at.1);
            body.motion.dest = (at.0, at.1);
        }
        self.deliver(&effects, reply.map(|buffer| (vid, buffer)));
        true
    }

    /// Places the body of a player the world has just admitted, `PlayerLoad`'s `Show`
    /// (`G/input_login.cpp:590`), and returns the records its own client was sent. A map the
    /// world was given no grid for has no sectree, and a point no sectree holds stands the body
    /// there with no view, sent only its own insert (V4).
    pub(super) fn place_body(&mut self, vid: Vid, place: EnterPlace, card: PcCard) -> Vec<Vec<u8>> {
        let key = (place.channel, place.map);
        self.maps.entry(key).or_insert_with(MapIndex::treeless);
        self.bodies.insert(
            vid,
            Body::new(
                place.channel,
                place.map,
                (place.x, place.y),
                card,
                self.last_pulse,
            ),
        );
        // `Show` starts the affect event of a PC below its maximum stamina (`G/char.cpp:1874-1875`).
        let below_max = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(world::character::Character::points)
            .is_some_and(|points| points.stamina() < points.max_stamina());
        if below_max {
            self.start_affect_event(vid);
        }
        let at = (place.x, place.y, place.z);
        let mut records = Vec::new();
        if !self.show_body(vid.raw(), at, Some(&mut records)) {
            let mut effects = Vec::new();
            if let Some(index) = self.maps.get_mut(&key) {
                index.place_treeless(EntityKey::Character(vid.raw()), at, &mut effects);
            }
            self.deliver(&effects, Some((vid.raw(), &mut records)));
        }
        records
    }

    /// Takes the body of a leaving player off its map (`G/char.cpp:786-789`): every viewer is
    /// sent its removal, and the body leaves the movers. Returns where it stood, as (map, x, y).
    pub(super) fn remove_body(&mut self, vid: Vid) -> Option<(i32, i32, i32)> {
        let spot = self.spot_of(vid.raw());
        let body = self.bodies.remove(&vid)?;
        self.movers.remove(&vid.raw());
        let mut effects = Vec::new();
        if let Some(index) = self.maps.get_mut(&(body.channel, body.map)) {
            index.remove(EntityKey::Character(vid.raw()), &mut effects);
        }
        self.deliver(&effects, None);
        spot.map(|spot| (body.map, spot.x, spot.y))
    }

    /// `place` with the point of the body under `vid`, which is where the character stands: the
    /// descriptor's copy is only what its last save wrote. A character with no body keeps it.
    pub(super) fn live_place(&self, vid: Vid, place: GroundPlace) -> GroundPlace {
        self.spot_of(vid.raw()).map_or(place, |spot| GroundPlace {
            x: spot.x,
            y: spot.y,
            ..place
        })
    }

    /// Where the body under `vid` stands, as (map, x, y).
    pub(super) fn kept_place(&self, vid: Vid) -> Option<(i32, i32, i32)> {
        let map = self.place_of(vid.raw())?.1;
        self.spot_of(vid.raw()).map(|spot| (map, spot.x, spot.y))
    }

    /// Runs one `Move`, `SyncPosition` or `Relay` command, and logs what a move did. A move's
    /// or a relay's `settled` is dropped only after it ran, when its records are queued.
    pub(super) fn apply_motion(&mut self, command: GameCommand) {
        match command {
            GameCommand::Move {
                vid,
                record,
                settled,
            } => {
                log_move(vid, self.run_move(vid, &record).as_ref());
                drop(settled);
            }
            GameCommand::SyncPosition { vid, packet, reply } => {
                let result = self.sync_positions(vid, &packet);
                if reply.send(result).is_err() {
                    debug!(?vid, "nobody was left to hear a sync answer");
                }
            }
            GameCommand::Relay {
                vid,
                records,
                settled,
            } => {
                self.relay(vid, &records);
                drop(settled);
            }
            _ => debug_assert!(false, "only motion commands reach apply_motion"),
        }
    }

    /// `CInputMain::Move` (`G/input_main.cpp:1757-1891`) for the player under `vid`: the
    /// judgement, then the body's change, then the relay to its view without itself. `None`
    /// when it has no body; the outcome is the caller's to log.
    ///
    /// A move refused for its distance (or a dead mover) shows the body where it stands and
    /// stops it, which re-sends its view (`:1786-1811`); the other refusals and an ignored
    /// move change nothing and send nothing.
    pub(super) fn run_move(&mut self, vid: Vid, record: &CgMove) -> Option<MoveOutcome> {
        let raw = vid.raw();
        let (_, map) = self.place_of(raw)?;
        let spot = self.spot_of(raw)?;
        let context = MoveContext {
            map,
            x: spot.x,
            y: spot.y,
            // The Rewrite has no mount yet, so nothing is riding.
            riding: false,
            // The Rewrite has no death state yet.
            dead: false,
            // The Rewrite has no stun affect and no private shop, so `CanMove` always holds.
            can_move: true,
            move_speed: self.move_speed_of(raw),
        };
        let outcome = judge_move(record, &context);
        self.apply_judged_move(vid, spot, record, &outcome);
        Some(outcome)
    }

    /// What `CInputMain::Move` does after each judgement, for the body standing at `spot`.
    fn apply_judged_move(&mut self, vid: Vid, spot: Spot, record: &CgMove, outcome: &MoveOutcome) {
        let raw = vid.raw();
        match outcome {
            MoveOutcome::Ignore(_)
            | MoveOutcome::Refuse(MoveRefusal::CannotMove | MoveRefusal::InvalidFunction { .. }) => {
            }
            MoveOutcome::Refuse(MoveRefusal::TooFar { .. } | MoveRefusal::Dead) => {
                let _shown = self.show_body(raw, (spot.x, spot.y, spot.z), None);
                self.stop(raw);
            }
            MoveOutcome::Accept(accepted) => {
                let now = self.clock.now();
                if let Some(body) = self.bodies.get_mut(&vid) {
                    // `OnMove(true)` stamps the attack time (`:1839-1840`).
                    if matches!(record.function, FUNC_ATTACK | FUNC_COMBO) {
                        body.last_attack = Some(now);
                    }
                    body.rotation = accepted.rotation;
                    // `ResetStopTime()` on every accepted function, which starts the stamina
                    // refill's wait (`input_main.cpp:1833`, `:1872`). A zero-speed `FUNC_MOVE`
                    // returns before it and is `Ignore` in `judge_move`, so never gets here.
                    body.stop_pulse = self.last_pulse;
                }
                if matches!(record.function, FUNC_ATTACK | FUNC_COMBO) {
                    self.remove_revive_invisible(vid);
                }
                match accepted.disposition {
                    MoveDisposition::Goto { x, y } => {
                        let _set = self.goto(raw, x, y);
                    }
                    MoveDisposition::Step { x, y } => {
                        let _moved = self.move_to(raw, x, y);
                        self.stop(raw);
                    }
                }
                // `GetCurrentMoveDuration()` after the `Goto`, stale when the destination did
                // not change (`G/char.cpp:3478-3490`).
                let duration = self
                    .bodies
                    .get(&vid)
                    .map_or(0, |body| body.motion.duration_ms);
                let me = EntityKey::Character(raw);
                self.packet_around(me, &move_relay(record, raw, duration).encode(), Some(me));
            }
        }
    }

    /// `CHARACTER::Goto` (`G/char.cpp:3471-3510`). Returns true when a new destination was set;
    /// the relay reads the duration after it.
    pub(super) fn goto(&mut self, vid: u32, x: i32, y: i32) -> bool {
        let Some(spot) = self.spot_of(vid) else {
            return false;
        };
        let now = self.clock.now();
        let move_speed = self.move_speed_of(vid);
        let Some(body) = self.bodies.get_mut(&Vid::new(vid)) else {
            return false;
        };
        if spot.x == x && spot.y == y {
            return false;
        }
        if body.motion.dest == (x, y) {
            // `:3478-3490`: the Move state again, with the stale start, time and duration.
            body.motion.moving = true;
            self.movers.insert(vid);
            return false;
        }
        body.motion.dest = (x, y);
        body.motion.start = (spot.x, spot.y);
        body.motion.duration_ms =
            move_duration(body.motion.start, (x, y), MOTION_SPEED_FALLBACK, move_speed);
        body.motion.start_ms = now;
        body.motion.moving = true;
        self.movers.insert(vid);
        true
    }

    /// `CHARACTER::Move` (`G/char.cpp:3610-3622`): the same point returns true at once, with
    /// nothing changed; any other point is a `Sync`. `OnMove` is not ported (no attack state).
    pub(super) fn move_to(&mut self, vid: u32, x: i32, y: i32) -> bool {
        if self
            .spot_of(vid)
            .is_some_and(|spot| spot.x == x && spot.y == y)
        {
            return true;
        }
        self.sync_body(vid, x, y)
    }

    /// `CHARACTER::Sync` (`G/char.cpp:3384-3458`): the body turns toward the point and stands
    /// there with z 0, changing its sectree but not its view. A point no sectree holds keeps the
    /// spot and returns false (`__FIX_KICK_HACK__`). The same point is no exception: the body
    /// turns to 90 degrees, `GetDegreeFromPositionXY` of a zero vector, and z becomes 0.
    pub(super) fn sync_body(&mut self, vid: u32, x: i32, y: i32) -> bool {
        let key = EntityKey::Character(vid);
        let Some(place) = self.place_of(vid) else {
            return false;
        };
        let radius = self.view_radius;
        let Some(index) = self.maps.get_mut(&place) else {
            return false;
        };
        let Some(spot) = index.spot(key) else {
            return false;
        };
        if index.tree_at(x, y).is_none() {
            return false;
        }
        // `SetRotationToXY` (`:3413`) comes after both early returns.
        let rotation = rotation_to_xy(x.saturating_sub(spot.x), y.saturating_sub(spot.y));
        let mut effects = Vec::new();
        let moved = index.move_body(key, x, y, radius, &mut effects);
        if let Some(body) = self.bodies.get_mut(&Vid::new(vid)) {
            body.rotation = rotation;
        }
        // Only a treeless body moved into a sectree has effects (V4).
        self.deliver(&effects, None);
        moved
    }

    /// `CHARACTER::Stop` (`G/char.cpp:3460-3469`): Idle, and the start and destination at the
    /// spot. The duration is kept.
    pub(super) fn stop(&mut self, vid: u32) {
        let spot = self.spot_of(vid);
        if let Some(body) = self.bodies.get_mut(&Vid::new(vid)) {
            body.motion.moving = false;
            if let Some(spot) = spot {
                body.motion.start = (spot.x, spot.y);
                body.motion.dest = (spot.x, spot.y);
            }
        }
        self.movers.remove(&vid);
    }

    /// `StateMove`'s arrival (`G/char_state.cpp:897-904`): Idle, with nothing else changed.
    /// `StopStaminaConsume` is not ported (V3).
    fn arrive(&mut self, vid: u32) {
        if let Some(body) = self.bodies.get_mut(&Vid::new(vid)) {
            body.motion.moving = false;
        }
        self.movers.remove(&vid);
    }

    /// `CHARACTER_MANAGER::Update` for the moving players (`G/char_manager.cpp:678-745`), the
    /// last step of a Pulse: each mover, in VID order (V1), runs `StateMove`.
    pub(super) fn step_motion(&mut self, pulse: u64) {
        let now = self.clock.now();
        let sample = pulse & SAMPLE_PULSE_MASK == 0;
        let movers: Vec<u32> = self.movers.iter().copied().collect();
        for vid in movers {
            let Some(motion) = self.bodies.get(&Vid::new(vid)).map(|body| body.motion) else {
                continue;
            };
            let step = motion.at(now);
            if let Some((point, _)) = step {
                // `Move(x, y)` (`:793`); a not-a-number rate skips only this (D1).
                let _moved = self.move_to(vid, point.0, point.1);
            }
            if sample {
                self.sample_view(vid);
                self.check_trade_distance(vid);
            }
            if let Some((_, true)) = step {
                self.arrive(vid);
            }
        }
    }

    /// `UpdateSectree()` for a moving player (`G/char_state.cpp:797`), delivered.
    fn sample_view(&mut self, vid: u32) {
        let Some(place) = self.place_of(vid) else {
            return;
        };
        let radius = self.view_radius;
        let mut effects = Vec::new();
        if let Some(index) = self.maps.get_mut(&place) {
            index.update_sectree(EntityKey::Character(vid), radius, &mut effects);
        }
        self.deliver(&effects, None);
    }

    /// `StateMove`'s exchange test (`G/char_state.cpp:799-808`): the mover's own trade ends when
    /// the other side stands `EXCHANGE_MAX_DISTANCE` or farther away. There is no map test.
    fn check_trade_distance(&mut self, vid: u32) {
        let Some(partner) = self.trade_partner(Vid::new(vid)) else {
            return;
        };
        let (Some(mine), Some(theirs)) = (self.spot_of(vid), self.spot_of(partner.raw())) else {
            // Unreachable: a departure ends its trade before the body goes.
            return;
        };
        let distance = distance_approx(
            mine.x.saturating_sub(theirs.x),
            mine.y.saturating_sub(theirs.y),
        );
        if distance >= EXCHANGE_MAX_DISTANCE {
            self.cancel_trade(Vid::new(vid));
        }
    }

    /// Whether the character under `vid` attacked within the last 1500 ms of the world clock.
    pub(super) fn attacked_recently(&self, vid: Vid) -> bool {
        let now = self.clock.now();
        self.bodies
            .get(&vid)
            .and_then(|body| body.last_attack)
            .is_some_and(|at| now.wrapping_sub(at) <= RECENT_ATTACK_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::fixtures::{a_world, enter, points, TestClock, PLACE};
    use super::super::view_encode::encode_pc_insert;
    use crate::game_loop_messages::{Kept, RelayScope};
    use crate::movement::{FUNC_MAX_NUM, FUNC_MOVE, FUNC_WAIT};

    fn drained(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// A move record of `function` toward `(x, y)`, turned 18 times 5 degrees.
    const fn a_move(function: u8, x: i32, y: i32) -> CgMove {
        CgMove::new(function, 0, 18, x, y, 77)
    }

    #[test]
    fn calculate_duration_matches_legacy_for_speed_0_100_150_200() {
        // i = 100 - speed: 100 -> 200%; 0 -> 100%; -50 -> 10000 / 150 = 66%; -100 -> 50%.
        assert_eq!(calculate_duration(0, 1000), 2000);
        assert_eq!(calculate_duration(100, 1000), 1000);
        assert_eq!(calculate_duration(150, 1000), 660);
        assert_eq!(calculate_duration(200, 1000), 500);
        assert_eq!(calculate_duration(99, 1000), 1010);
        assert_eq!(calculate_duration(101, 999), 989);
        assert_eq!(calculate_duration(100, -7), -7);
    }

    #[test]
    fn a_move_duration_is_distance_over_motion_speed_in_float_then_scaled_by_speed() {
        assert_eq!(move_duration((0, 0), (1600, 0), 300.0, 100), 5333);
        assert_eq!(move_duration((0, 0), (100, 100), 300.0, 100), 471);
        assert_eq!(move_duration((100, 100), (0, 0), 300.0, 100), 471);
        assert_eq!(move_duration((0, 0), (1600, 0), 300.0, 200), 2666);
        assert_eq!(move_duration((0, 0), (1600, 0), 300.0, 0), 10666);
        assert_eq!(move_duration((5, 5), (5, 5), 300.0, 100), 0);
    }

    #[test]
    fn distance_sqrt_squares_in_float_before_the_root() {
        // 16777217 is not a float; its square in f32 is 2^48, and the root 2^24 exactly. In
        // f64 the root would be 16777217.
        assert_eq!(
            distance_sqrt(16_777_217, 0).to_bits(),
            16_777_216.0_f32.to_bits()
        );
        assert_eq!(distance_sqrt(3, 4).to_bits(), 5.0_f32.to_bits());
        assert_eq!(distance_sqrt(-3, -4).to_bits(), 5.0_f32.to_bits());
        // 46341^2 overflows an i32 but not a float.
        assert_eq!(distance_sqrt(46_341, 0).to_bits(), 46_341.0_f32.to_bits());
    }

    #[test]
    fn rotation_to_xy_is_90_for_a_zero_vector_and_mirrors_for_negative_x() {
        assert_eq!(rotation_to_xy(0, 0).to_bits(), 90.0_f32.to_bits());
        assert_eq!(rotation_to_xy(0, 10).to_bits(), 0.0_f32.to_bits());
        assert_eq!(rotation_to_xy(10, 0).to_bits(), 90.0_f32.to_bits());
        assert_eq!(rotation_to_xy(-10, 0).to_bits(), 270.0_f32.to_bits());
        assert_eq!(rotation_to_xy(0, -10).to_bits(), 180.0_f32.to_bits());
        let east = rotation_to_xy(7, 3);
        let west = rotation_to_xy(-7, 3);
        assert!((east + west - 360.0).abs() < 1.0e-3, "{east} {west}");
        assert!(east > 0.0 && east < 90.0, "{east}");
    }

    #[test]
    fn the_rate_is_clamped_and_arrival_is_the_clamped_one() {
        let motion = Motion {
            start: (0, 0),
            dest: (1000, -1000),
            start_ms: 100,
            duration_ms: 1000,
            moving: true,
        };
        assert_eq!(motion.at(100), Some(((0, 0), false)));
        assert_eq!(motion.at(600), Some(((500, -500), false)));
        assert_eq!(motion.at(1099), Some(((999, -999), false)));
        assert_eq!(motion.at(1100), Some(((1000, -1000), true)));
        assert_eq!(motion.at(5000), Some(((1000, -1000), true)));
    }

    #[test]
    fn a_zero_duration_on_its_start_millisecond_skips_the_pulse_and_arrives_on_the_next() {
        let motion = Motion {
            start: (0, 0),
            dest: (10, 10),
            start_ms: 40,
            duration_ms: 0,
            moving: true,
        };
        assert_eq!(motion.at(40), None);
        assert_eq!(motion.at(41), Some(((10, 10), true)));
    }

    #[test]
    fn a_zero_duration_arrives_on_the_first_later_millisecond() {
        let motion = Motion {
            start: (3, 4),
            dest: (3, 4),
            start_ms: 7,
            duration_ms: 0,
            moving: true,
        };
        assert_eq!(motion.at(8), Some(((3, 4), true)));
    }

    #[test]
    fn the_interpolated_point_is_float_times_rate_plus_start_truncated() {
        // 1/3 of 10 is 3.33..: truncation gives 3 from the start at 0 and -3 toward -10.
        let motion = Motion {
            start: (0, 0),
            dest: (10, -10),
            start_ms: 0,
            duration_ms: 3,
            moving: true,
        };
        assert_eq!(motion.at(1), Some(((3, -3), false)));
        // A start of 16777217 is rounded to a float before the sum: legacy lands on 16777216.
        let far = Motion {
            start: (16_777_217, 0),
            dest: (16_777_219, 0),
            start_ms: 0,
            duration_ms: 1000,
            moving: true,
        };
        assert_eq!(far.at(0), Some(((16_777_216, 0), false)));
    }

    #[test]
    fn the_elapsed_time_wraps_as_a_dword() {
        let motion = Motion {
            start: (0, 0),
            dest: (1000, 0),
            start_ms: u32::MAX - 99,
            duration_ms: 1000,
            moving: true,
        };
        // 100 ms before the wrap and 400 after: 500 elapsed.
        assert_eq!(motion.at(400), Some(((500, 0), false)));
    }

    #[test]
    fn the_motion_speed_is_the_legacy_fallback_300() {
        assert_eq!(MOTION_SPEED_FALLBACK.to_bits(), 300.0_f32.to_bits());
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, 7, (3200, 3200), 820);
        assert!(world.goto(7, 3200 + 1600, 3200));
        let body = &world.bodies[&Vid::new(7)];
        assert_eq!(body.motion.duration_ms, 5333);
    }

    #[test]
    fn a_goto_to_the_current_point_changes_nothing() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, 7, (3200, 3200), 820);
        let before = world.bodies[&Vid::new(7)].motion;
        assert!(!world.goto(7, 3200, 3200));
        assert_eq!(world.bodies[&Vid::new(7)].motion, before);
        assert!(world.movers.is_empty());
    }

    #[test]
    fn a_goto_starts_at_the_current_point_and_stamps_the_clock() {
        let clock = TestClock::default();
        clock.set(1234);
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, 7, (3200, 3200), 820);
        assert!(world.goto(7, 3300, 3300));
        let motion = world.bodies[&Vid::new(7)].motion;
        assert_eq!(
            motion,
            Motion {
                start: (3200, 3200),
                dest: (3300, 3300),
                start_ms: 1234,
                duration_ms: 471,
                moving: true,
            }
        );
        assert!(world.movers.contains(&7));
        // `Goto` does not move the body.
        let spot = world.spot_of(7).unwrap();
        assert_eq!((spot.x, spot.y), (3200, 3200));
    }

    #[test]
    fn a_goto_to_the_same_destination_keeps_the_stale_start_and_duration() {
        let clock = TestClock::default();
        clock.set(100);
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, 7, (3200, 3200), 820);
        assert!(world.goto(7, 4800, 3200));
        clock.set(2100);
        world.step_motion(1);
        world.stop(7);
        // Stop put the destination at the spot; a goto back to the old destination is new.
        assert!(world.goto(7, 4800, 3200));
        world.arrive(7);
        let stale = world.bodies[&Vid::new(7)].motion;
        clock.set(9000);
        // Arrived, not stopped: the destination stands, so the Move state comes back stale.
        assert!(!world.goto(7, 4800, 3200));
        let again = world.bodies[&Vid::new(7)].motion;
        assert_eq!(
            again,
            Motion {
                moving: true,
                ..stale
            }
        );
        assert!(world.movers.contains(&7));
    }

    #[test]
    fn stop_keeps_the_duration_and_resets_start_and_destination() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, 7, (3200, 3200), 820);
        assert!(world.goto(7, 4800, 3200));
        clock.set(2667);
        world.step_motion(1);
        world.stop(7);
        let motion = world.bodies[&Vid::new(7)].motion;
        assert_eq!(motion.start, (4000, 3200));
        assert_eq!(motion.dest, (4000, 3200));
        assert_eq!(motion.duration_ms, 5333);
        assert!(!motion.moving);
        assert!(!world.movers.contains(&7));
    }

    #[test]
    fn a_sync_to_a_point_no_sectree_holds_keeps_the_spot() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, 7, (3200, 3200), 820);
        assert!(!world.sync_body(7, -5, 3200));
        let spot = world.spot_of(7).unwrap();
        assert_eq!((spot.x, spot.y), (3200, 3200));
        assert_eq!(
            world.bodies[&Vid::new(7)].rotation.to_bits(),
            0.0_f32.to_bits()
        );
    }

    /// A body outside every sectree (V4) synced into one fills its view, and both sides are
    /// told: the player in range is sent the mover's insert, and the mover the player's.
    #[test]
    fn a_treeless_body_synced_into_a_sectree_and_its_view_are_sent_each_other() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut seven = enter(&mut world, 7, (3200, 3200), 820);
        let mut eight = enter(&mut world, 8, (-5, 3200), 820);
        assert!(
            drained(&mut seven).is_empty(),
            "8 entered outside every sectree"
        );
        let _ = drained(&mut eight);
        assert!(world.sync_body(8, 3100, 3200));
        let insert = |of: u32, to: u32| {
            let body = &world.bodies[&Vid::new(of)];
            let spot = world.spot_of(of).expect("a spot");
            encode_pc_insert(
                of,
                body,
                spot,
                &points(820),
                Some((to, false)),
                clock.now(),
                [0; 2],
            )
            .to_records
        };
        let holds = |records: &[Vec<u8>], run: &[Vec<u8>]| {
            records.windows(run.len()).any(|window| window == run)
        };
        let (to_seven, to_eight) = (drained(&mut seven), drained(&mut eight));
        assert!(holds(&to_seven, &insert(8, 7)), "{to_seven:02x?}");
        assert!(holds(&to_eight, &insert(7, 8)), "{to_eight:02x?}");
    }

    #[test]
    fn a_sync_sets_z_to_zero_and_turns_toward_the_target() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, 7, (3200, 3200), 820);
        let place = world.place_of(7).unwrap();
        let mut effects = Vec::new();
        assert!(world.maps.get_mut(&place).unwrap().show(
            EntityKey::Character(7),
            (3200, 3200, 55),
            10_500,
            &mut effects
        ));
        assert_eq!(world.spot_of(7).unwrap().z, 55);
        assert!(world.sync_body(7, 3100, 3200));
        let spot = world.spot_of(7).unwrap();
        assert_eq!((spot.x, spot.y, spot.z), (3100, 3200, 0));
        assert_eq!(
            world.bodies[&Vid::new(7)].rotation.to_bits(),
            270.0_f32.to_bits()
        );
        // A `Move` to the same point changes nothing, not even the rotation or z.
        world.bodies.get_mut(&Vid::new(7)).unwrap().rotation = 1.0;
        assert!(world.maps.get_mut(&place).unwrap().show(
            EntityKey::Character(7),
            (3100, 3200, 55),
            10_500,
            &mut effects
        ));
        assert!(world.move_to(7, 3100, 3200));
        assert_eq!(
            world.bodies[&Vid::new(7)].rotation.to_bits(),
            1.0_f32.to_bits()
        );
        assert_eq!(world.spot_of(7).unwrap().z, 55);
        // A `Sync` to the same point turns the body to 90 degrees and sets z to 0.
        assert!(world.sync_body(7, 3100, 3200));
        assert_eq!(
            world.bodies[&Vid::new(7)].rotation.to_bits(),
            90.0_f32.to_bits()
        );
        let spot = world.spot_of(7).unwrap();
        assert_eq!((spot.x, spot.y, spot.z), (3100, 3200, 0));
    }

    #[test]
    fn the_movers_step_in_vid_order_and_sample_on_pulses_divisible_by_16() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        // Sectree columns 0 and 2: neither is around the other, and 12000 is out of range.
        let mut far = enter(&mut world, 9, (3200, 3200), 820);
        let mut near = enter(&mut world, 8, (15_200, 3200), 820);
        let drain = |inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>| {
            std::iter::from_fn(|| inbox.try_recv().ok()).count()
        };
        assert_eq!((drain(&mut far), drain(&mut near)), (0, 0));
        assert!(world.goto(9, 4800, 3200));
        assert!(world.goto(8, 12_200, 3200));
        assert_eq!(world.movers.iter().copied().collect::<Vec<_>>(), [8, 9]);
        clock.set(10_000);
        world.step_motion(1);
        // Both arrived, 7400 apart in neighbouring columns; with no sample neither sees the other.
        assert!(world.movers.is_empty());
        assert_eq!(world.spot_of(8).map(|spot| spot.x), Some(12_200));
        assert_eq!((drain(&mut far), drain(&mut near)), (0, 0));
        assert!(world.goto(9, 4900, 3200));
        clock.set(10_100);
        world.step_motion(15);
        assert_eq!((drain(&mut far), drain(&mut near)), (0, 0));
        world.step_motion(16);
        // 9's sample finds 8: 9 gets 8's pair, and 8 gets 9's pair, MOVE and walk mode.
        assert_eq!(drain(&mut far), 2);
        assert_eq!(drain(&mut near), 4);
    }

    /// A move the body cannot make (`CanMove`, `G/input_main.cpp:1759-1760`) changes nothing and
    /// sends nothing, as an invalid function does (`:1764-1768`). Nothing makes a body unable to
    /// move yet (no stun, no private shop), so the judged outcome is given directly.
    #[test]
    fn a_move_the_body_cannot_make_changes_and_sends_nothing() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut mover = enter(&mut world, 7, (3200, 3200), 820);
        let mut viewer = enter(&mut world, 8, (3300, 3200), 820);
        assert!(world.goto(7, 4800, 3200));
        let _ = (drained(&mut mover), drained(&mut viewer));
        let before = world.bodies[&Vid::new(7)].motion;
        let spot = world.spot_of(7).unwrap();
        for refusal in [
            MoveRefusal::CannotMove,
            MoveRefusal::InvalidFunction { function: 9 },
        ] {
            let outcome = MoveOutcome::Refuse(refusal);
            world.apply_judged_move(Vid::new(7), spot, &a_move(FUNC_MOVE, 3300, 3200), &outcome);
        }
        assert!(drained(&mut mover).is_empty());
        assert!(drained(&mut viewer).is_empty());
        assert_eq!(world.bodies[&Vid::new(7)].motion, before);
        assert_eq!(world.spot_of(7), Some(spot));
        assert!(world.movers.contains(&7), "still walking");
    }

    /// A walk that ends on a sampled Pulse is sampled at its last point before it arrives
    /// (`G/char_state.cpp:793-904`): the mover meets the player standing near its destination.
    #[test]
    fn a_walk_ending_on_a_sampled_pulse_is_sampled_at_its_last_point() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut mover = enter(&mut world, 9, (3200, 3200), 820);
        let mut waiting = enter(&mut world, 8, (15_200, 3200), 820);
        let _ = (drained(&mut mover), drained(&mut waiting));
        assert!(world.goto(9, 12_200, 3200));
        clock.set(60_000);
        world.step_motion(16);
        assert!(world.movers.is_empty(), "9 arrived on this Pulse");
        assert_eq!(drained(&mut mover).len(), 2, "8's pair");
        assert_eq!(
            drained(&mut waiting).len(),
            2,
            "9's pair, with no walk left"
        );
    }

    #[test]
    fn walking_is_stamina_at_or_below_zero() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _one = enter(&mut world, 7, (3200, 3200), 0);
        let _two = enter(&mut world, 8, (3200, 3200), 1);
        let _three = enter(&mut world, 9, (3200, 3200), -1);
        let walking = |vid: u32| is_walking(world.characters.find_by_vid(Vid::new(vid)).unwrap());
        assert!(walking(7));
        assert!(!walking(8));
        assert!(walking(9));
    }

    #[test]
    fn an_attack_blocks_an_equip_for_1500_ms_and_not_1501() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, 7, (3200, 3200), 820);
        assert!(!world.attacked_recently(Vid::new(7)));
        clock.set(u32::MAX - 499);
        world.bodies.get_mut(&Vid::new(7)).unwrap().last_attack = Some(world.clock.now());
        clock.set(1000);
        assert!(world.attacked_recently(Vid::new(7)));
        clock.set(1001);
        assert!(!world.attacked_recently(Vid::new(7)));
    }

    /// An accepted ATTACK or COMBO stamps the world clock on the body, and an equip is held
    /// back up to 1500 ms after it and not 1501 (`G/char_item.cpp:8416-8422`); a walk stamps
    /// nothing.
    #[test]
    fn an_attack_step_blocks_an_equip_for_1500_ms_and_not_1501() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, 7, (3200, 3200), 820);
        clock.set(40_000);
        let walk = world.run_move(Vid::new(7), &a_move(FUNC_MOVE, 3300, 3200));
        assert!(matches!(walk, Some(MoveOutcome::Accept(_))));
        assert!(!world.attacked_recently(Vid::new(7)), "a walk is no attack");
        // Each function at its own millisecond, so each stamp is its own.
        for (function, at) in [(FUNC_ATTACK, 50_000), (FUNC_COMBO, 60_000)] {
            clock.set(at);
            let hit = world.run_move(Vid::new(7), &a_move(function, 3300, 3200));
            assert!(matches!(hit, Some(MoveOutcome::Accept(_))));
            assert_eq!(world.bodies[&Vid::new(7)].last_attack, Some(at));
            clock.set(at + 1500);
            assert!(
                world.attacked_recently(Vid::new(7)),
                "{function} 1500 ms ago"
            );
            clock.set(at + 1501);
            assert!(
                !world.attacked_recently(Vid::new(7)),
                "{function} 1501 ms ago"
            );
        }
    }

    /// A move refused for its distance shows the body where it stands, a `ViewReencode`
    /// (`G/entity_view.cpp:23-45`), and stops it (`G/input_main.cpp:1786-1811`); nothing is
    /// relayed.
    #[test]
    fn a_refused_move_too_far_reencodes_and_stops() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut mover = enter(&mut world, 7, (3200, 3200), 820);
        let mut viewer = enter(&mut world, 8, (3300, 3200), 820);
        assert!(world.goto(7, 4800, 3200));
        clock.set(2_667);
        world.step_motion(1);
        let _ = (drained(&mut mover), drained(&mut viewer));
        let turned = world.bodies[&Vid::new(7)].rotation;
        // The judge measures in hundreds of units: 76,000 east is 760 against the 750 limit,
        // turned the other way.
        let refused = world.run_move(
            Vid::new(7),
            &CgMove::new(FUNC_MOVE, 0, 54, 80_000, 3200, 77),
        );
        assert!(
            matches!(
                refused,
                Some(MoveOutcome::Refuse(MoveRefusal::TooFar { .. }))
            ),
            "{refused:?} from {:?}",
            world.spot_of(7)
        );
        let removal = super::super::view_encode::remove_record(7);
        let heard = drained(&mut viewer);
        assert_eq!(heard.len(), 3, "the removal, then the pair again");
        assert_eq!(heard[0], removal);
        let own = drained(&mut mover);
        assert_eq!(own[0], removal, "its own removal first");
        assert!(
            !own.contains(&super::super::view_encode::remove_record(8)),
            "no removal of what it sees"
        );
        let motion = world.bodies[&Vid::new(7)].motion;
        assert_eq!((motion.start, motion.dest), ((4000, 3200), (4000, 3200)));
        assert!(!motion.moving);
        assert!(!world.movers.contains(&7));
        assert_eq!(
            world.bodies[&Vid::new(7)].rotation.to_bits(),
            turned.to_bits(),
            "the rotation stays"
        );
    }

    /// A move to `i32::MIN` wraps the 32-bit distance as legacy's `long` does
    /// (`G/input_main.cpp:1786`) and is refused for its distance; the world does not panic.
    #[test]
    fn a_move_past_the_long_range_wraps_and_is_refused() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _mover = enter(&mut world, 7, (3200, 3200), 820);
        for (x, y) in [(i32::MIN, 3200), (3200, i32::MIN)] {
            let refused = world.run_move(Vid::new(7), &CgMove::new(FUNC_MOVE, 0, 0, x, y, 1));
            assert!(
                matches!(
                    refused,
                    Some(MoveOutcome::Refuse(MoveRefusal::TooFar { distance }))
                        if distance > 21_474_000.0
                ),
                "{refused:?}"
            );
            assert_eq!(
                world.spot_of(7).map(|spot| (spot.x, spot.y)),
                Some((3200, 3200))
            );
        }
    }

    /// A move with a function byte past the table and no skill bit is refused with no record
    /// and no change (`G/input_main.cpp:1770-1774`).
    #[test]
    fn an_invalid_function_sends_nothing_and_changes_nothing() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut mover = enter(&mut world, 7, (3200, 3200), 820);
        let mut viewer = enter(&mut world, 8, (3300, 3200), 820);
        let _ = (drained(&mut mover), drained(&mut viewer));
        for function in [FUNC_MAX_NUM, 0x7f] {
            let outcome = world.run_move(Vid::new(7), &a_move(function, 3250, 3200));
            assert!(
                matches!(
                    outcome,
                    Some(MoveOutcome::Refuse(MoveRefusal::InvalidFunction { .. }))
                ),
                "{function}: {outcome:?}"
            );
            assert!(drained(&mut viewer).is_empty(), "{function}");
            assert!(drained(&mut mover).is_empty(), "{function}");
            let spot = world.spot_of(7).unwrap();
            assert_eq!((spot.x, spot.y), (3200, 3200), "{function}");
            assert!(world.movers.is_empty(), "{function}");
            assert_eq!(
                world.bodies[&Vid::new(7)].rotation.to_bits(),
                0.0_f32.to_bits(),
                "{function}"
            );
        }
        let wait = world.run_move(Vid::new(7), &a_move(FUNC_WAIT, 3250, 3200));
        assert!(
            matches!(wait, Some(MoveOutcome::Accept(_))),
            "a wait is a step"
        );
        assert_eq!(drained(&mut viewer).len(), 1);
    }

    /// A walk from a body with no movement speed is consumed: no goto and no relay. A step
    /// with no speed still moves and is relayed.
    #[test]
    fn a_func_move_with_no_speed_is_not_relayed() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut viewer = enter(&mut world, 8, (3300, 3200), 820);
        let (tx, _still) = tokio::sync::mpsc::unbounded_channel();
        world
            .enter_world_with_items(
                Vid::new(7),
                7,
                "Unloaded",
                &[],
                crate::client_registry::ClientOutbox::new(tx),
            )
            .unwrap();
        let (channel, map) = PLACE;
        let card = world.bodies[&Vid::new(8)].card.clone();
        let place = EnterPlace {
            channel,
            map,
            x: 3200,
            y: 3200,
            z: 0,
        };
        let _own = world.place_body(Vid::new(7), place, card);
        assert_eq!(world.move_speed_of(7), 0, "no points");
        let _ = drained(&mut viewer);
        let walk = world.run_move(Vid::new(7), &a_move(FUNC_MOVE, 3250, 3200));
        assert!(matches!(walk, Some(MoveOutcome::Ignore(_))), "{walk:?}");
        assert!(drained(&mut viewer).is_empty());
        assert!(world.movers.is_empty());
        let hit = world.run_move(Vid::new(7), &a_move(FUNC_ATTACK, 3250, 3200));
        assert!(matches!(hit, Some(MoveOutcome::Accept(_))), "{hit:?}");
        assert_eq!(world.spot_of(7).map(|spot| spot.x), Some(3250));
        assert_eq!(drained(&mut viewer).len(), 1, "the step is relayed");
    }

    /// An accepted move turns the body to the client's byte times five (`SetRotation`,
    /// `G/input_main.cpp:1832`, `:1871`). `Goto` does not turn it, so the byte stands until the
    /// first step's `Sync` turns the body toward its point; a step to the point it stands on
    /// keeps the byte.
    #[test]
    fn an_accepted_move_turns_the_body_to_the_clients_byte() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _mover = enter(&mut world, 7, (3200, 3200), 820);
        let rotation = |world: &GameState| world.bodies[&Vid::new(7)].rotation.to_bits();
        let walk = CgMove::new(FUNC_MOVE, 0, 30, 4800, 3200, 77);
        let walked = world.run_move(Vid::new(7), &walk);
        assert!(matches!(walked, Some(MoveOutcome::Accept(_))), "{walked:?}");
        assert_eq!(rotation(&world), 150.0_f32.to_bits());
        // The walk's step in the same millisecond finds the body where it stands, and `Move`
        // returns before `Sync` (`G/char.cpp:3613-3614`).
        world.step_motion(1);
        assert_eq!(rotation(&world), 150.0_f32.to_bits(), "no step yet");
        clock.set(1_000);
        world.step_motion(1);
        assert_eq!(
            rotation(&world),
            90.0_f32.to_bits(),
            "the step turns it east"
        );
        let spot = world.spot_of(7).unwrap();
        let hit = CgMove::new(FUNC_ATTACK, 0, 40, spot.x, spot.y, 77);
        let struck = world.run_move(Vid::new(7), &hit);
        assert!(matches!(struck, Some(MoveOutcome::Accept(_))), "{struck:?}");
        assert_eq!(
            rotation(&world),
            200.0_f32.to_bits(),
            "the same point keeps the byte"
        );
    }

    /// The relay carries the motion's duration read after the goto for a walk, the stale one
    /// for a walk to the same destination, and 0 for every other function.
    #[test]
    fn only_func_move_relays_a_duration() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut mover = enter(&mut world, 7, (3200, 3200), 820);
        let mut viewer = enter(&mut world, 8, (3300, 3200), 820);
        let _ = (drained(&mut mover), drained(&mut viewer));
        let walk = a_move(FUNC_MOVE, 4800, 3200);
        let _ = world.run_move(Vid::new(7), &walk);
        assert_eq!(
            drained(&mut viewer),
            vec![move_relay(&walk, 7, 5333).encode()]
        );
        assert!(drained(&mut mover).is_empty(), "not to the mover itself");
        clock.set(2_667);
        world.step_motion(1);
        let _ = world.run_move(Vid::new(7), &walk);
        assert_eq!(
            drained(&mut viewer),
            vec![move_relay(&walk, 7, 5333).encode()],
            "the same destination: the stale duration"
        );
        let hit = a_move(FUNC_ATTACK, 4100, 3200);
        let _ = world.run_move(Vid::new(7), &hit);
        let heard = drained(&mut viewer);
        assert_eq!(heard, vec![move_relay(&hit, 7, 0).encode()]);
        assert_eq!(heard[0][heard[0].len() - 4..], [0, 0, 0, 0]);
        // A step mid-walk ends the walk: `Move`, then `Stop` (`G/input_main.cpp:1874-1875`).
        assert!(!world.movers.contains(&7), "the walk ended");
        let motion = world.bodies[&Vid::new(7)].motion;
        assert!(!motion.moving);
        assert_eq!((motion.start, motion.dest), ((4100, 3200), (4100, 3200)));
    }

    /// What a save writes is where the body stands now, mid-walk included, and leaving hands
    /// back the same point.
    #[test]
    fn kept_carries_the_live_point() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, 7, (3200, 3200), 820);
        assert!(world.goto(7, 4800, 3200));
        clock.set(2_667);
        world.step_motion(1);
        let (reply, answer) = tokio::sync::oneshot::channel();
        world.apply(GameCommand::KeptOf {
            vid: Vid::new(7),
            reply,
        });
        let kept: Option<Kept> = answer.blocking_recv().unwrap();
        assert_eq!(
            kept.and_then(|kept| kept.place),
            Some((PLACE.1, 4000, 3200))
        );
        assert!(world.sync_body(7, 4100, 3300));
        let left = world.leave_world(Vid::new(7)).unwrap();
        assert_eq!(left.place, Some((PLACE.1, 4100, 3300)));
    }

    /// A relay lets its `settled` go only once its records are queued, so the flush that the
    /// descriptor runs on the release finds them: the sender's own copy is in its inbox the
    /// moment the wait ends, though the relay copies a large record to 200 bodies first.
    #[test]
    fn a_relay_settles_only_once_its_records_are_queued() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        // Outside every sectree a body has no view to fill, and a dropped inbox takes nothing.
        for vid in 1..=200 {
            drop(enter(&mut world, vid, (-5, 3200), 820));
        }
        let mut sender = enter(&mut world, 1000, (-5, 3200), 820);
        let _ = drained(&mut sender);
        let (settled, settling) = tokio::sync::oneshot::channel();
        let records = vec![(RelayScope::Map, vec![0x5a; 1 << 20])];
        let game = std::thread::spawn(move || {
            world.apply_motion(GameCommand::Relay {
                vid: Vid::new(1000),
                records,
                settled,
            });
        });
        let _released = settling.blocking_recv();
        let own = sender.try_recv().map(|record| record.len());
        game.join().expect("the world ran the relay");
        assert_eq!(own, Ok(1 << 20), "queued before the relay settled");
    }
}
