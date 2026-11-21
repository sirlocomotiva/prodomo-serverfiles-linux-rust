//! The `CG_MOVE` (7) and `CG_CHARACTER_POSITION` (28) policy of `CInputMain::Move`
//! and `CInputMain::Position`.
//!
//! # Provenance
//!
//! `CInputMain::Move` is `server/server/game/input_main.cpp:1757-1915` and
//! `CInputMain::Position` is `:1530-1548`. This module decides what the Rewrite does
//! with those two records; the world moves the body and relays the record
//! (`prodomo/src/game_state/motion.rs`).
//!
//! # The order of the checks is the behaviour
//!
//! Legacy checks, in this order:
//!
//! 1. `CanMove()` - not stunned, no open private shop (`char.cpp:3367-3381`). Its
//!    0.2-second rate limit is commented out, so there is no rate limit here either.
//! 2. `bFunc >= FUNC_MAX_NUM && !(bFunc & 0x80)` - a refused function byte.
//! 3. The distance test, `((!riding && fDist > 750) || fDist > 999)` and
//!    `OXEVENT_MAP_INDEX != map`.
//! 4. `IsPC() && IsDead()`, under `ENABLE_CHECK_GHOSTMODE`, which
//!    `input_main.cpp:55` defines.
//! 5. `POINT_MOV_SPEED == 0` on the `FUNC_MOVE` branch, which returns with no packet.
//!
//! Steps 3 and 4 call `ch->Show(current)` and `ch->Stop()` before returning, so a
//! refusal pulls the client back to where the server thinks the character is. They do
//! **not** send a `GC_MOVE`, so the client learns about the refusal from the view
//! record, not from a move.
//!
//! # The distance unit is a legacy defect
//!
//! Legacy computes `DISTANCE_SQRT((x - lX) / 100, (y - lY) / 100)`
//! (`input_main.cpp:1782`), and both arguments are `long`, so the division is
//! integer division by 100 before the square root. The limits that follow are 750 and
//! 999 in those units, which are metres, not the centimetres a reader would expect
//! from a "distance 750" check. The arithmetic is reproduced exactly, because a
//! client that moves against these limits must be accepted exactly as legacy accepts
//! it. It is recorded as a Defect in `docs/PROTOCOL_NOTES.md`; the Rewrite does not
//! "fix" it, since fixing it would change which moves a real client may make.
//!
//! # A move never echoes to its sender
//!
//! `CInputMain::Move` finishes with `ch->PacketAround(&pack, sizeof(TPacketGCMove), ch)`
//! (`:1891`). `CEntity::PacketView` (`entity.cpp:93-104`) ends with an unconditional
//! self-send, `f(std::make_pair(this, 0))`, but `FuncPacketAround::operator()` returns
//! early for `m_except`, and `m_except` is the moving character. So the self-send is
//! suppressed and the mover sees nothing.
//!
//! A sit or stand is the opposite: `CHARACTER::Standup` and `CHARACTER::Sitdown`
//! (`char.cpp:3318-3350`) call `PacketAround` with **no** `except`, so `m_except` is
//! `NULL` and the trailing self-send does fire. The character that sat down sees its
//! own pose record.
//!
//! # A sit-down defect, not reproduced
//!
//! `CHARACTER::Sitdown(int is_ground)` ignores its argument and always writes
//! `POSITION_SITTING_GROUND` (`char.cpp:3348`), so a chair request arrives as a ground
//! sit. The Rewrite honours the request.

#![warn(missing_docs)]

use protocol::cg_move::CgMove;
use protocol::gc_actors::GcCharacterMove;
use protocol::gc_position::GcCharacterPosition;

/// `FUNC_WAIT`, from the commented `EMoveFuncType` at `input_main.cpp:1769-1778`.
pub const FUNC_WAIT: u8 = 0;
/// `FUNC_MOVE`: the plain walk. This is the branch that calls `Goto`.
pub const FUNC_MOVE: u8 = 1;
/// `FUNC_ATTACK`: a melee swing while moving.
pub const FUNC_ATTACK: u8 = 2;
/// `FUNC_COMBO`: the second hit of a combo.
pub const FUNC_COMBO: u8 = 3;
/// `FUNC_MOB_SKILL`: a skill aimed at a mob.
pub const FUNC_MOB_SKILL: u8 = 4;
/// `_FUNC_SKILL`, the unused value 5. Legacy lets it through the range test and then
/// treats it as a plain move, because the skill test is `bFunc & 0x80`, not `== 5`.
pub const FUNC_SKILL_UNUSED: u8 = 5;
/// `FUNC_MAX_NUM`: the exclusive upper bound of the range test.
pub const FUNC_MAX_NUM: u8 = 6;
/// `FUNC_SKILL`: the bit that marks a skill motion.
pub const FUNC_SKILL: u8 = 0x80;
/// `MASK_SKILL_MOTION` (`input_main.cpp:1843`).
pub const MASK_SKILL_MOTION: u8 = 0x7f;

/// `OXEVENT_MAP_INDEX` (`server/server/game/OXEvent.h:2`), the one map where the
/// distance test is skipped.
pub const OXEVENT_MAP_INDEX: i32 = 113;

/// The divisor legacy applies to each axis before the square root.
pub const MOVE_DISTANCE_DIVISOR: i32 = 100;

/// The 750 limit of `input_main.cpp:1784`, in units of [`MOVE_DISTANCE_DIVISOR`].
pub const MOVE_DISTANCE_LIMIT: f64 = 750.0;

/// The 999 limit of `input_main.cpp:1784`, in the same units, and unconditional.
pub const MOVE_DISTANCE_HARD_LIMIT: f64 = 999.0;

/// The `DISTANCE_SQRT` of `server/server/game/utils.h:12-15`, with the integer
/// division legacy performs at the call site folded in.
#[must_use]
pub fn move_distance(from_x: i32, from_y: i32, to_x: i32, to_y: i32) -> f64 {
    // `DISTANCE_SQRT((ch->GetX() - pinfo->lX) / 100, (ch->GetY() - pinfo->lY) / 100)` takes
    // two `long` arguments (`server/server/game/utils.h:12`), so the division happens in
    // 32-bit integer arithmetic that truncates toward zero and the squares are computed in
    // `float`. No narrowing cast belongs here: the legacy axis stays a `long` all the way
    // into the conversion.
    // The squares and the root are taken in `f64`, not the legacy `float`. Every `i32` is
    // exact in `f64`, so no axis is rounded, and the comparison against the limit therefore
    // differs from legacy only in the last bit of a value that is either far below or far
    // above the limit. Taking it in `f32` instead would round an axis above 16,777,216,
    // which is a delta of more than 1.6 million world units.
    // Legacy is built `-m32`, so `long` is 32 bits and a client's far x or y wraps the
    // subtraction (`G/input_main.cpp:1786`); the wrap is kept, and a debug build does not panic.
    let dx = f64::from(from_x.wrapping_sub(to_x) / MOVE_DISTANCE_DIVISOR);
    let dy = f64::from(from_y.wrapping_sub(to_y) / MOVE_DISTANCE_DIVISOR);
    (dx * dx + dy * dy).sqrt()
}

/// What the sender must be known by before a move can be judged.
#[derive(Debug, Clone, PartialEq)]
pub struct MoveContext {
    /// `ch->GetMapIndex()`, which selects the OX-event exemption. Legacy's
    /// `GetMapIndex()` returns `int`, and the atlas lookup that produced it is signed.
    pub map: i32,
    /// `ch->GetX()`, the authoritative x.
    pub x: i32,
    /// `ch->GetY()`, the authoritative y.
    pub y: i32,
    /// `ch->IsRiding()`, which lifts the 750 limit.
    pub riding: bool,
    /// `ch->IsDead()`, tested under `ENABLE_CHECK_GHOSTMODE`.
    pub dead: bool,
    /// `CanMove()`: false when stunned or when a private shop is open.
    pub can_move: bool,
    /// `ch->GetLimitPoint(POINT_MOV_SPEED)`, which gates the `FUNC_MOVE` branch.
    /// The legacy signature takes a `BYTE` index but returns an `int`
    /// (`server/server/game/char.h:829`), so the value is not a byte.
    pub move_speed: i32,
}

/// Why a move was consumed without moving.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MoveRefusal {
    /// `if (!ch->CanMove()) return;` (`input_main.cpp:1759`).
    CannotMove,
    /// The `bFunc` range test (`:1765`).
    InvalidFunction {
        /// The refused `bFunc` byte.
        function: u8,
    },
    /// The distance test (`:1784`).
    TooFar {
        /// The measured distance, in legacy units.
        distance: f64,
    },
    /// `if (ch->IsPC() && ch->IsDead())` (`:1806`).
    Dead,
}

/// What an accepted move asks the world to do.
///
/// Legacy's two branches differ in more than speed: `FUNC_MOVE` calls `Goto`, which
/// records a destination and puts the character in the Move state, whose Pulses then walk
/// the body there. Every other branch calls `Move` and then `Stop`, and `Move` calls
/// `Sync`, which puts the body at the point at once. A client that sends `FUNC_MOVE` and
/// then a farther move is measured from where the walk has brought the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDisposition {
    /// `Goto(x, y)`: record the destination, which the Pulses walk the body to.
    Goto {
        /// The requested x.
        x: i32,
        /// The requested y.
        y: i32,
    },
    /// `Move(x, y)` then `Stop()`: set the position and the destination together.
    Step {
        /// The requested x.
        x: i32,
        /// The requested y.
        y: i32,
    },
}

/// The full answer for one accepted move.
#[derive(Debug, Clone, PartialEq)]
pub struct MoveAccepted {
    /// `ch->SetRotation(pinfo->bRot * 5)` - the client's byte times five. The legacy
    /// setter takes a `float` (`server/server/game/char.h:884`), so nothing wraps.
    pub rotation: f32,
    /// What the world should do with the position.
    pub disposition: MoveDisposition,
}

/// The result of judging one `CG_MOVE`.
#[derive(Debug, Clone, PartialEq)]
pub enum MoveOutcome {
    /// The record is consumed and nothing at all is sent.
    Ignore(&'static str),
    /// The record is consumed, the client is pulled back, and nothing is broadcast.
    Refuse(MoveRefusal),
    /// The move happened.
    Accept(MoveAccepted),
}

/// `ch->SetRotation(pinfo->bRot * 5)`. `CHARACTER::SetRotation` takes a `float`
/// (`server/server/game/char.h:884`), so the product is a plain float with no wrap:
/// the largest input, 255, gives 1275.0, which is a legal rotation.
#[must_use]
pub fn rotation_from_client(byte: u8) -> f32 {
    f32::from(byte) * 5.0
}

/// Judge one `CG_MOVE` against the legacy order of checks.
///
/// # Errors
///
/// This function cannot fail; the refusals are values in [`MoveOutcome`], so the
/// descriptor can log each one and keep running.
#[must_use]
pub fn judge_move(record: &CgMove, context: &MoveContext) -> MoveOutcome {
    if !context.can_move {
        return MoveOutcome::Refuse(MoveRefusal::CannotMove);
    }
    let is_skill = record.function & FUNC_SKILL != 0;
    if record.function >= FUNC_MAX_NUM && !is_skill {
        return MoveOutcome::Refuse(MoveRefusal::InvalidFunction {
            function: record.function,
        });
    }
    let distance = move_distance(context.x, context.y, record.x, record.y);
    let too_far =
        (distance > MOVE_DISTANCE_LIMIT && !context.riding) || distance > MOVE_DISTANCE_HARD_LIMIT;
    if too_far && context.map != OXEVENT_MAP_INDEX {
        return MoveOutcome::Refuse(MoveRefusal::TooFar { distance });
    }
    if context.dead {
        return MoveOutcome::Refuse(MoveRefusal::Dead);
    }
    if record.function == FUNC_MOVE {
        // `char.cpp:1826-1827`: a character with no movement speed returns without
        // even building a packet, so nothing is broadcast either.
        if context.move_speed == 0 {
            return MoveOutcome::Ignore("no movement speed");
        }
    }
    let disposition = if record.function == FUNC_MOVE {
        MoveDisposition::Goto {
            x: record.x,
            y: record.y,
        }
    } else {
        MoveDisposition::Step {
            x: record.x,
            y: record.y,
        }
    };
    MoveOutcome::Accept(MoveAccepted {
        rotation: rotation_from_client(record.rotation),
        disposition,
    })
}

/// The `GC_MOVE` `CInputMain::Move` relays around the mover, without the mover
/// (`input_main.cpp:1879-1891`): the client's own bytes, the mover's VID, and
/// `GetCurrentMoveDuration()` read after the `Goto` for `FUNC_MOVE`, 0 for every other
/// function.
#[must_use]
pub fn move_relay(record: &CgMove, vid: u32, move_duration: u32) -> GcCharacterMove {
    let duration = if record.function == FUNC_MOVE {
        move_duration
    } else {
        0
    };
    GcCharacterMove::new(
        record.function,
        record.argument,
        record.rotation,
        vid,
        record.x,
        record.y,
        record.time,
        duration,
    )
}

/// What `CInputMain::Position` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoseOutcome {
    /// `ch->Standup()`.
    Stand,
    /// `ch->Sitdown(0)` for a chair and `ch->Sitdown(1)` for the ground. Legacy
    /// collapses both to the ground; the Rewrite keeps them apart.
    Sit {
        /// The `is_ground` argument legacy drops.
        ground: bool,
    },
    /// No arm matched, so nothing happens.
    Ignore,
}

/// Translate one `CG_CHARACTER_POSITION` byte into the legacy call it selects.
///
/// `CInputMain::Position` (`input_main.cpp:1534-1547`) is a bare `switch` with three
/// arms and no default arm, so every other byte is consumed and ignored. The byte is
/// not range-checked anywhere, which is why all 256 values reach this function.
#[must_use]
pub fn judge_pose(position: u8) -> PoseOutcome {
    match position {
        protocol::cg_position::POSITION_GENERAL => PoseOutcome::Stand,
        protocol::cg_position::POSITION_SITTING_CHAIR => PoseOutcome::Sit { ground: false },
        protocol::cg_position::POSITION_SITTING_GROUND => PoseOutcome::Sit { ground: true },
        _ => PoseOutcome::Ignore,
    }
}

/// The `GC_CHARACTER_POSITION` a pose change broadcasts.
///
/// Legacy passes no `except` to `PacketAround`, so the record reaches the character
/// that changed pose as well as everyone around it. A caller that has already
/// excluded the sender is therefore wrong, and this function's name says so.
#[must_use]
pub fn pose_record(vid: u32, position: u8) -> GcCharacterPosition {
    GcCharacterPosition::new(vid, position)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An exact float comparison the lint accepts: two equal doubles carry equal bits, and
    /// `+0.0` is the only value that can compare equal to itself with different bits, which
    /// no assertion here relies on.
    fn same_f32(actual: f32, expected: f32) -> bool {
        actual.to_bits() == expected.to_bits()
    }

    /// The same comparison for a distance, which is an `f64`.
    fn same_f64(actual: f64, expected: f64) -> bool {
        actual.to_bits() == expected.to_bits()
    }

    /// A tolerance comparison for a computed square root.
    ///
    /// The tolerance is relative and never below a nanometre of a unit, so a distance of
    /// zero still has to be exactly zero while a distance of 2000 may differ in the last
    /// bits.
    fn close_to(actual: f64, expected: f64) -> bool {
        let slack = 1e-12_f64 * expected.abs().max(1.0);
        (actual - expected).abs() <= slack
    }

    /// Distinct byte halves, so an endianness slip cannot pass by symmetry.
    const VID: u32 = 0xdf9b_5713;
    const X: i32 = 900;
    const Y: i32 = 1200;
    const TIME: u32 = 0x1357_9bdf;

    fn context() -> MoveContext {
        MoveContext {
            map: 1,
            x: 0,
            y: 0,
            riding: false,
            dead: false,
            can_move: true,
            move_speed: 600,
        }
    }

    fn request(function: u8, x: i32, y: i32) -> CgMove {
        CgMove {
            function,
            argument: 0,
            rotation: 72,
            x,
            y,
            time: TIME,
        }
    }

    fn accepted(outcome: MoveOutcome) -> MoveAccepted {
        match outcome {
            MoveOutcome::Accept(accepted) => accepted,
            other => panic!("expected an accepted move, got {other:?}"),
        }
    }

    #[test]
    fn a_plain_walk_is_a_goto_and_carries_the_current_duration() {
        let record = request(FUNC_MOVE, X, Y);
        let result = accepted(judge_move(&record, &context()));
        assert_eq!(result.disposition, MoveDisposition::Goto { x: X, y: Y });
        let relay = move_relay(&record, VID, 1234);
        assert_eq!(relay.dw_duration, 1234, "FUNC_MOVE keeps the duration");
        assert_eq!(relay.dw_vid, VID);
        assert_eq!(relay.dw_time, TIME);
        assert!(
            same_f32(result.rotation, 360.0),
            "the rotation byte is multiplied by five"
        );
    }

    #[test]
    fn every_other_function_steps_and_carries_no_duration() {
        for function in [
            FUNC_WAIT,
            FUNC_ATTACK,
            FUNC_COMBO,
            FUNC_MOB_SKILL,
            FUNC_SKILL_UNUSED,
        ] {
            let record = request(function, X, Y);
            let result = accepted(judge_move(&record, &context()));
            assert_eq!(
                result.disposition,
                MoveDisposition::Step { x: X, y: Y },
                "bFunc {function} is not the walk"
            );
            assert_eq!(
                move_relay(&record, VID, 1234).dw_duration,
                0,
                "only FUNC_MOVE keeps a duration"
            );
        }
    }

    #[test]
    fn a_skill_function_is_a_step_because_the_test_is_the_high_bit() {
        let result = accepted(judge_move(&request(FUNC_SKILL, X, Y), &context()));
        assert_eq!(result.disposition, MoveDisposition::Step { x: X, y: Y });
    }

    #[test]
    fn the_unused_five_passes_the_range_test_like_legacy() {
        // `FUNC_MAX_NUM` is 6, so 5 is under it and accepted. The skill test is
        // `bFunc & 0x80`, which 5 fails, so the branch is a plain step.
        assert!(matches!(
            judge_move(&request(FUNC_SKILL_UNUSED, X, Y), &context()),
            MoveOutcome::Accept(_)
        ));
    }

    #[test]
    fn the_first_refused_function_is_six() {
        for function in [FUNC_MAX_NUM, 7, 0x7f] {
            let outcome = judge_move(&request(function, X, Y), &context());
            assert_eq!(
                outcome,
                MoveOutcome::Refuse(MoveRefusal::InvalidFunction { function }),
                "bFunc {function} is out of range",
            );
        }
    }

    #[test]
    fn every_high_bit_function_passes_the_range_test() {
        for function in [FUNC_SKILL, 0x81, 0xff] {
            assert!(
                matches!(
                    judge_move(&request(function, X, Y), &context()),
                    MoveOutcome::Accept(_)
                ),
                "bFunc {function} carries the skill bit"
            );
        }
    }

    #[test]
    fn a_stunned_character_is_refused_before_the_function_byte_is_read() {
        let mut ctx = context();
        ctx.can_move = false;
        assert_eq!(
            judge_move(&request(200, X, Y), &ctx),
            MoveOutcome::Refuse(MoveRefusal::CannotMove),
            "CanMove runs first, so a bad bFunc is never even looked at",
        );
    }

    #[test]
    fn a_far_move_is_refused_and_the_distance_is_reported() {
        // 100 units of the divisor is 1; 200000 cm is 2000 units, over the 999 limit.
        let outcome = judge_move(&request(FUNC_MOVE, 200_000, 0), &context());
        match outcome {
            MoveOutcome::Refuse(MoveRefusal::TooFar { distance }) => {
                assert!(close_to(distance, 2000.0), "got {distance}");
            }
            other => panic!("expected a distance refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_riding_character_lifts_the_soft_limit_but_not_the_hard_one() {
        let mut ctx = context();
        ctx.riding = true;
        // 800 units is over 750 but under 999: a rider is allowed.
        assert!(matches!(
            judge_move(&request(FUNC_MOVE, 80_000, 0), &ctx),
            MoveOutcome::Accept(_)
        ));
        // 1200 units is over 999: a rider is still refused.
        assert!(matches!(
            judge_move(&request(FUNC_MOVE, 120_000, 0), &ctx),
            MoveOutcome::Refuse(MoveRefusal::TooFar { .. })
        ));
    }

    #[test]
    fn a_walker_over_the_soft_limit_is_refused() {
        // 800 units is over 750.
        assert!(matches!(
            judge_move(&request(FUNC_MOVE, 80_000, 0), &context()),
            MoveOutcome::Refuse(MoveRefusal::TooFar { .. })
        ));
    }

    /// Both limits are strict: legacy refuses `fDist > 750` and `fDist > 999`
    /// (`G/input_main.cpp:1786-1788`), so a move of exactly the limit is allowed. The axis is
    /// divided by 100 first, so 75,099 is still 750.
    #[test]
    fn a_move_of_exactly_either_limit_is_allowed() {
        for x in [75_000, 75_099, -75_000] {
            assert!(
                matches!(
                    judge_move(&request(FUNC_MOVE, x, 0), &context()),
                    MoveOutcome::Accept(_)
                ),
                "{x}"
            );
        }
        assert!(matches!(
            judge_move(&request(FUNC_MOVE, 75_100, 0), &context()),
            MoveOutcome::Refuse(MoveRefusal::TooFar { .. })
        ));
        let mut ctx = context();
        ctx.riding = true;
        for x in [99_900, 99_999] {
            assert!(
                matches!(
                    judge_move(&request(FUNC_MOVE, x, 0), &ctx),
                    MoveOutcome::Accept(_)
                ),
                "{x}"
            );
        }
        assert!(matches!(
            judge_move(&request(FUNC_MOVE, 100_000, 0), &ctx),
            MoveOutcome::Refuse(MoveRefusal::TooFar { .. })
        ));
    }

    #[test]
    fn the_ox_event_map_is_exempt_from_every_distance_limit() {
        let mut ctx = context();
        ctx.map = OXEVENT_MAP_INDEX;
        assert!(
            matches!(
                judge_move(&request(FUNC_MOVE, 5_000_000, 5_000_000), &ctx),
                MoveOutcome::Accept(_)
            ),
            "the OX-event map skips the test entirely",
        );
    }

    #[test]
    fn a_dead_character_is_refused_after_the_distance_test() {
        let mut ctx = context();
        ctx.dead = true;
        assert_eq!(
            judge_move(&request(FUNC_MOVE, X, Y), &ctx),
            MoveOutcome::Refuse(MoveRefusal::Dead),
        );
    }

    #[test]
    fn a_dead_character_who_is_also_too_far_is_refused_for_the_distance() {
        let mut ctx = context();
        ctx.dead = true;
        assert!(matches!(
            judge_move(&request(FUNC_MOVE, 200_000, 0), &ctx),
            MoveOutcome::Refuse(MoveRefusal::TooFar { .. })
        ));
    }

    #[test]
    fn a_walk_with_no_speed_is_consumed_silently() {
        let mut ctx = context();
        ctx.move_speed = 0;
        assert_eq!(
            judge_move(&request(FUNC_MOVE, X, Y), &ctx),
            MoveOutcome::Ignore("no movement speed"),
            "legacy returns before it builds a packet",
        );
    }

    #[test]
    fn a_step_with_no_speed_still_moves() {
        let mut ctx = context();
        ctx.move_speed = 0;
        assert!(
            matches!(
                judge_move(&request(FUNC_ATTACK, X, Y), &ctx),
                MoveOutcome::Accept(_)
            ),
            "the speed gate is only on the FUNC_MOVE branch",
        );
    }

    #[test]
    fn the_distance_matches_the_legacy_integer_division() {
        // 150 cm over 100 is 1 by integer division, not 2 by rounding.
        assert!(close_to(move_distance(0, 0, 150, 0), 1.0));
        assert!(close_to(move_distance(0, 0, 199, 0), 1.0));
        assert!(close_to(move_distance(0, 0, 250, 0), 2.0));
        // The divisor truncates towards zero, so a negative axis truncates upwards.
        assert!(close_to(move_distance(0, 0, -150, 0), 1.0));
    }

    /// A client's x or y past the `long` range from the character wraps the subtraction, as
    /// legacy's 32-bit `long` does: 1 - `i32::MIN` is -2147483647, and the division by 100
    /// truncates toward zero.
    #[test]
    fn a_far_axis_wraps_as_a_32_bit_long() {
        assert!(close_to(move_distance(1, 0, i32::MIN, 0), 21_474_836.0));
        assert!(close_to(move_distance(0, 1, 0, i32::MIN), 21_474_836.0));
        assert!(close_to(move_distance(-2, 0, i32::MAX, 0), 21_474_836.0));
        assert!(close_to(move_distance(0, 0, i32::MIN, 0), 21_474_836.0));
    }

    #[test]
    fn the_distance_is_euclidean_over_both_axes() {
        assert!(close_to(move_distance(0, 0, 300, 400), 5.0));
    }

    #[test]
    fn the_rotation_is_the_client_byte_times_five_with_no_wrap() {
        assert!(same_f32(rotation_from_client(0), 0.0));
        assert!(same_f32(rotation_from_client(72), 360.0));
        assert!(
            same_f32(rotation_from_client(255), 1275.0),
            "the setter is a float, not a WORD"
        );
    }

    #[test]
    fn the_broadcast_carries_the_clients_own_bytes_unchanged() {
        let record = CgMove {
            function: FUNC_COMBO,
            argument: 42,
            rotation: 9,
            x: -1234,
            y: 5678,
            time: TIME,
        };
        let _accepted = accepted(judge_move(&record, &context()));
        let mut out = Vec::with_capacity(protocol::gc_actors::GC_CHARACTER_MOVE_WIRE_SIZE);
        move_relay(&record, VID, 1234).encode_into(&mut out);
        assert_eq!(out.len(), 24, "GC_MOVE is 24 bytes on the wire");
        assert_eq!(out[0], 0x03, "HEADER_GC_MOVE is byte 3");
        assert_eq!(out[1], FUNC_COMBO);
        assert_eq!(out[2], 42, "bArg is relayed");
        assert_eq!(out[3], 9, "bRot is relayed, not multiplied, on the wire");
        assert_eq!(
            &out[4..8],
            &VID.to_le_bytes(),
            "dwVID follows the three bytes, in the struct order at packet.h:1701-1712",
        );
        assert_eq!(&out[8..12], &(-1234i32).to_le_bytes(), "lX");
        assert_eq!(&out[12..16], &5678i32.to_le_bytes(), "lY");
        assert_eq!(&out[16..20], &TIME.to_le_bytes(), "dwTime");
        assert_eq!(
            &out[20..24],
            &0u32.to_le_bytes(),
            "dwDuration is zero off FUNC_MOVE"
        );
    }

    #[test]
    fn the_pose_bytes_select_the_legacy_arms() {
        assert_eq!(
            judge_pose(protocol::cg_position::POSITION_GENERAL),
            PoseOutcome::Stand
        );
        assert_eq!(
            judge_pose(protocol::cg_position::POSITION_SITTING_CHAIR),
            PoseOutcome::Sit { ground: false }
        );
        assert_eq!(
            judge_pose(protocol::cg_position::POSITION_SITTING_GROUND),
            PoseOutcome::Sit { ground: true }
        );
    }

    #[test]
    fn an_unknown_pose_byte_is_consumed_and_ignored() {
        for byte in 3..=u8::MAX {
            assert_eq!(judge_pose(byte), PoseOutcome::Ignore, "pose byte {byte}");
        }
    }

    #[test]
    fn a_chair_sit_stays_a_chair_which_legacy_does_not_do() {
        // Legacy writes POSITION_SITTING_GROUND for both; the Rewrite keeps them apart.
        let chair = judge_pose(protocol::cg_position::POSITION_SITTING_CHAIR);
        let ground = judge_pose(protocol::cg_position::POSITION_SITTING_GROUND);
        assert_ne!(chair, ground);
    }

    #[test]
    fn the_pose_record_is_six_bytes_with_the_client_pose() {
        assert_eq!(
            pose_record(VID, protocol::cg_position::POSITION_SITTING_GROUND).encode(),
            vec![0x2b, 0x13, 0x57, 0x9b, 0xdf, 0x02],
        );
    }

    #[test]
    fn the_function_constants_are_the_commented_enum_values() {
        assert_eq!(
            [
                FUNC_WAIT,
                FUNC_MOVE,
                FUNC_ATTACK,
                FUNC_COMBO,
                FUNC_MOB_SKILL,
                FUNC_SKILL_UNUSED,
                FUNC_MAX_NUM
            ],
            [0, 1, 2, 3, 4, 5, 6],
        );
        assert_eq!(FUNC_SKILL, 0x80);
        assert_eq!(MASK_SKILL_MOTION, 0x7f);
    }

    #[test]
    fn the_distance_limits_are_the_legacy_numbers() {
        assert!(same_f64(MOVE_DISTANCE_LIMIT, 750.0));
        assert!(same_f64(MOVE_DISTANCE_HARD_LIMIT, 999.0));
        assert_eq!(OXEVENT_MAP_INDEX, 113);
        assert_eq!(MOVE_DISTANCE_DIVISOR, 100);
    }

    #[test]
    fn a_mutation_that_moves_the_walk_branch_onto_every_function_is_killed() {
        // If the `FUNC_MOVE` test were `!=` instead of `==`, every other function
        // would become a goto and every walk a step. The two dispositions are
        // therefore not interchangeable.
        let walk = accepted(judge_move(&request(FUNC_MOVE, X, Y), &context()));
        let attack = accepted(judge_move(&request(FUNC_ATTACK, X, Y), &context()));
        assert_eq!(walk.disposition, MoveDisposition::Goto { x: X, y: Y });
        assert_eq!(attack.disposition, MoveDisposition::Step { x: X, y: Y });
        assert_ne!(walk.disposition, attack.disposition);
    }

    #[test]
    fn a_mutation_that_puts_the_speed_gate_on_every_branch_is_killed() {
        let mut ctx = context();
        ctx.move_speed = 0;
        assert!(matches!(
            judge_move(&request(FUNC_WAIT, X, Y), &ctx),
            MoveOutcome::Accept(_)
        ));
    }

    #[test]
    fn a_mutation_that_drops_the_ox_event_exemption_is_killed() {
        let mut ctx = context();
        ctx.map = OXEVENT_MAP_INDEX;
        assert!(
            judge_move(&request(FUNC_MOVE, 5_000_000, 0), &ctx).eq(&MoveOutcome::Accept(accepted(
                judge_move(&request(FUNC_MOVE, 5_000_000, 0), &ctx)
            )))
        );
    }
}
