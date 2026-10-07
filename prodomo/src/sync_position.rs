//! Pure policy for the legacy `HEADER_CG_SYNC_POSITION` gameplay action.
//!
//! The protocol crate owns the variable-frame decoder.  This module starts
//! after decoding and applies only the source-backed admission and replay
//! rules.  Character lookup, sync ownership, movement, descriptor closure,
//! and view broadcast are injected ports; this module does not claim to be a
//! live descriptor or a world/sectree implementation.

use std::error::Error;
use std::fmt;
use std::time::Duration;

use common::vid::Vid;
use protocol::cg_variable::{SyncPositionElement, SyncPositionPacket};

/// Wire size of `TPacketGCOwnership`, which an accepted claim sends.
pub const OWNERSHIP_RECORD_SIZE: usize = 1 + 4 + 4;

/// Wire byte of `HEADER_GC_OWNERSHIP`.
pub const HEADER_GC_OWNERSHIP: u8 = 62;

/// `SetSyncOwner`'s `DISTANCE_APPROX(...) > 250` limit, compared against the *approximated*
/// distance rather than the raw coordinates, so the effective raw threshold is 260 on one
/// axis. Both distances are raw map units, which are 100th of a tile.
pub const SYNC_OWNER_APPROX_DISTANCE: i32 = 250;

/// `ENABLE_FLY_FIX` claim lifetime in `IsSyncOwner`, which is
/// `get_dword_time() - m_fSyncTime >= 100`. `get_dword_time()` is milliseconds, so the
/// Rewrite counts the same 100 units.
pub const SYNC_CLAIM_LIFETIME: Duration = Duration::from_millis(100);

/// `DISTANCE_APPROX` from `server/server/game/utils.h:17-40`: the octagonal
/// approximation `(123/128) * max + (51/128) * min`, evaluated in the shifted
/// integer form the source uses so no rounding difference appears.
#[must_use]
pub fn distance_approx(dx: i32, dy: i32) -> i32 {
    let (dx, dy) = (i64::from(dx).abs(), i64::from(dy).abs());
    let (min, max) = if dx < dy { (dx, dy) } else { (dy, dx) };
    let value = (max << 8) + (max << 3) - (max << 4) - (max << 1) + (min << 7) - (min << 5)
        + (min << 3)
        - (min << 1);
    // The source computes this in a 32-bit `int` and shifts it back down. A `u32`
    // coordinate space keeps the shifts exact, so only the final narrowing needs a
    // checked cast.
    i32::try_from(value >> 8).unwrap_or(i32::MAX)
}

/// What `CHARACTER::IsSyncOwner` reads off a victim.
///
/// `None` is a character that was never claimed. The source does not store that case: its
/// constructor seeds `m_fSyncTime` with `get_float_time() - 3`, which is already past the
/// window, so a fresh victim is always claimable and `None` is the faithful answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncOwnershipState {
    /// `m_pkChrSyncOwner`, the character currently allowed to move the victim.
    pub owner: Option<Vid>,
    /// `m_fSyncTime`, refreshed on every accepted claim, in the same units as `now`.
    pub claimed_at: Duration,
}

/// The result of judging one `SetSyncOwner(actor)` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOwnershipOutcome {
    /// The claim was accepted. `owner_changed` tells the caller to reset the victim's
    /// last-sync stamp, and `record` is the `TPacketGCOwnership` to send around the victim.
    Accepted {
        /// True when `m_pkChrSyncOwner` was not already the actor, which is the arm that
        /// resets the last-sync stamp and rewrites the owned list.
        owner_changed: bool,
        /// `TPacketGCOwnership`, which the source sends after every accepted claim.
        record: Vec<u8>,
    },
    /// The actor already holds the claim and the victim is past the distance limit: the claim
    /// stands and the element is applied, but the claim's time is not refreshed and no record
    /// is sent (`if (m_pkChrSyncOwner == ch) return true;`, `G/char.cpp:5510-5518`).
    Kept,
    /// The claim was refused, and the element is skipped.
    Refused,
}

/// Judge one `CHARACTER::SetSyncOwner(actor)` call against a victim.
///
/// `server/server/game/char.cpp:5469-5551`, in the source's own order. Two arms are not
/// ported: `AIFLAG_NOMOVE`, which only a monster carries, and `!battle_is_attackable`, which
/// refuses with a `DAMAGE_BLOCK` record (`:5476-5481`). That one is the PK rule
/// (`G/battle.cpp:99-151`, `CPVPManager::CanAttack`) and can refuse a pair of player
/// characters, for one in the protect PK mode or two of one empire in the peace mode; it
/// waits for the battle rules, and STATUS lists it among the movement gaps.
#[must_use]
pub fn judge_sync_ownership(
    actor: Vid,
    victim: &SyncPositionVictim,
    state: SyncOwnershipState,
    actor_x: i32,
    actor_y: i32,
    now: Duration,
) -> SyncOwnershipOutcome {
    // `if (ch == this) { sys_err("SetSyncOwner owner == this"); return false; }`. A client
    // cannot move itself by naming its own VID.
    if victim.vid == actor {
        return SyncOwnershipOutcome::Refused;
    }
    // `if (!IsSyncOwner(ch)) return false;`. `IsSyncOwner` accepts the current owner, or any
    // character once the claim stamp is at least `SYNC_CLAIM_LIFETIME` old.
    let holds = state.owner == Some(actor);
    let age = state.owner.and(now.checked_sub(state.claimed_at));
    let claim_is_fresh = age.is_some_and(|age| age < SYNC_CLAIM_LIFETIME);
    if !holds && claim_is_fresh {
        return SyncOwnershipOutcome::Refused;
    }
    // `DISTANCE_APPROX(GetX() - ch->GetX(), GetY() - ch->GetY()) > 250`, on the raw
    // coordinates with no `/ 100`. A character that already holds the claim keeps it past
    // the limit, with no new stamp and no record; a new owner does not get one.
    let over = distance_approx(victim.x - actor_x, victim.y - actor_y) > SYNC_OWNER_APPROX_DISTANCE;
    if over {
        return if holds {
            SyncOwnershipOutcome::Kept
        } else {
            SyncOwnershipOutcome::Refused
        };
    }
    SyncOwnershipOutcome::Accepted {
        owner_changed: !holds,
        record: ownership_record(actor, victim.vid),
    }
}

/// The `TPacketGCOwnership` bytes an accepted claim sends.
#[must_use]
pub fn ownership_record(owner: Vid, victim: Vid) -> Vec<u8> {
    let mut record = Vec::with_capacity(OWNERSHIP_RECORD_SIZE);
    record.push(HEADER_GC_OWNERSHIP);
    record.extend_from_slice(&owner.raw().to_le_bytes());
    record.extend_from_slice(&victim.raw().to_le_bytes());
    debug_assert_eq!(record.len(), OWNERSHIP_RECORD_SIZE);
    record
}

/// Wire size of the fixed `TPacketCGSyncPosition` prefix.
pub const SYNC_POSITION_PREFIX_SIZE: usize = 3;

/// Wire size of one `TPacketCGSyncPositionElement`.
pub const SYNC_POSITION_ELEMENT_SIZE: usize = 12;

/// Maximum number of elements processed by the legacy gameplay action.
///
/// The variable-frame decoder deliberately retains all declared elements.  The
/// cap belongs here because it is a gameplay policy, not a wire-format rule.
pub const SYNC_POSITION_ELEMENT_LIMIT: usize = 16;

/// Default `g_iSyncHackLimitCount` from the active game configuration.
pub const SYNC_HACK_LIMIT_COUNT: u32 = 10;

/// Owner-distance limit after the legacy `/ 100` coordinate conversion.
pub const SYNC_OWNER_DISTANCE_LIMIT: f32 = 3_500.0;

/// Reported-position displacement limit after the legacy `/ 100` conversion.
pub const SYNC_DISPLACEMENT_LIMIT: f32 = 25.0;

/// Minimum time between accepted syncs for one victim.
pub const SYNC_VALID_INTERVAL: Duration = Duration::from_millis(100);

/// Header value for `TPacketGCSyncPosition`.
pub const HEADER_GC_SYNC_POSITION: u8 = 0x05;

/// The legacy character kinds explicitly skipped by `SyncPosition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPositionVictimKind {
    /// A player character.
    Player,
    /// A monster character.
    Monster,
    /// An NPC, which cannot be claimed as a sync victim.
    Npc,
    /// A warp entity, which cannot be claimed as a sync victim.
    Warp,
    /// A goto entity, which cannot be claimed as a sync victim.
    Goto,
    /// Any other active legacy entity kind.
    Other,
}

impl SyncPositionVictimKind {
    /// Returns whether the legacy switch skips this kind.
    #[must_use]
    pub const fn is_skipped(self) -> bool {
        matches!(self, Self::Npc | Self::Warp | Self::Goto)
    }
}

/// Snapshot of a resolved sync victim.
///
/// `last_sync_time` is intentionally not stored in this snapshot.  The owner
/// port can reset it as part of `SetSyncOwner`, so the policy reads the
/// current value through [`SyncPositionPorts::last_sync_time`] after the owner
/// call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncPositionVictim {
    /// Virtual identifier requested by the client element.
    pub vid: Vid,
    /// Legacy character/entity kind.
    pub kind: SyncPositionVictimKind,
    /// Current X coordinate in the source coordinate unit.
    pub x: i32,
    /// Current Y coordinate in the source coordinate unit.
    pub y: i32,
}

/// Mutable actor state used by the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncPositionActor {
    /// Actor virtual identifier.
    pub vid: Vid,
    /// Actor current X coordinate in the source coordinate unit.
    pub x: i32,
    /// Actor current Y coordinate in the source coordinate unit.
    pub y: i32,
    /// Legacy actor sync-hack counter.
    pub sync_hack_count: u32,
}

/// Why the policy asks its descriptor port to close the actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPositionCloseReason {
    /// The declared frame is shorter than the three-byte source prefix.
    InvalidDeclaredSize,
    /// The owner-distance violation counter reached its limit.
    OwnerDistanceHackLimit,
    /// The too-frequent-sync counter reached its limit.
    SyncIntervalHackLimit,
    /// A reported position moved more than the displacement limit.
    SyncDisplacement,
}

/// Whether the handler consumed the frame or asked the descriptor to close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPositionDisposition {
    /// The declared frame was consumed without closing the descriptor.
    Consumed,
    /// The descriptor should close before normal frame consumption completes.
    Closed,
}

/// Result of applying the pure sync-position policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPositionResult {
    /// Normal consumption or terminal descriptor closure.
    pub disposition: SyncPositionDisposition,
    /// Bytes after the three-byte prefix that the legacy input loop would
    /// consume on a normal return.  `None` corresponds to the legacy `-1`
    /// return used for a close.
    pub consumed_extra: Option<usize>,
    /// Number of elements examined after applying the 16-element gameplay cap.
    pub processed_elements: usize,
    /// Accepted elements in their original input order.
    pub accepted_elements: Vec<SyncPositionElement>,
    /// Exact one-frame GC sync-position bytes, when at least one element was
    /// accepted and the handler completed normally.
    pub gc_packet: Option<Vec<u8>>,
    /// Close reason, present exactly when `disposition` is `Closed`.
    pub close_reason: Option<SyncPositionCloseReason>,
}

impl SyncPositionResult {
    fn consumed(
        consumed_extra: usize,
        processed_elements: usize,
        accepted_elements: Vec<SyncPositionElement>,
    ) -> Self {
        Self {
            disposition: SyncPositionDisposition::Consumed,
            consumed_extra: Some(consumed_extra),
            processed_elements,
            accepted_elements,
            gc_packet: None,
            close_reason: None,
        }
    }

    fn closed(
        reason: SyncPositionCloseReason,
        processed_elements: usize,
        accepted_elements: Vec<SyncPositionElement>,
    ) -> Self {
        Self {
            disposition: SyncPositionDisposition::Closed,
            consumed_extra: None,
            processed_elements,
            accepted_elements,
            gc_packet: None,
            close_reason: Some(reason),
        }
    }

    fn with_gc_packet(mut self) -> Self {
        if !self.accepted_elements.is_empty() {
            debug_assert!(self.accepted_elements.len() <= SYNC_POSITION_ELEMENT_LIMIT);
            self.gc_packet = Some(
                encode_gc_sync_position(&self.accepted_elements)
                    .expect("accepted sync-position elements are capped before encoding"),
            );
        }
        self
    }
}

/// Error returned by the standalone GC packet encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPositionEncodeError {
    /// The legacy gameplay action cannot emit more than 16 accepted elements.
    TooManyElements {
        /// Number of elements supplied to the encoder.
        count: usize,
        /// Legacy gameplay limit.
        limit: usize,
    },
    /// The explicit output size cannot fit the source `WORD` field.
    SizeOverflow {
        /// Computed complete packet size.
        size: usize,
    },
}

impl fmt::Display for SyncPositionEncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyElements { count, limit } => {
                write!(
                    formatter,
                    "sync-position output has {count} elements; limit is {limit}"
                )
            }
            Self::SizeOverflow { size } => {
                write!(
                    formatter,
                    "sync-position output size {size} does not fit a WORD"
                )
            }
        }
    }
}

impl Error for SyncPositionEncodeError {}

/// Injected ports for the source-side sync-position effects.
///
/// `set_sync_owner` is deliberately a real port rather than a boolean-only
/// predicate.  The legacy implementation also performs attackability,
/// ownership, timestamp, GC ownership, and damage-block effects.  `sync` is
/// likewise side-effecting: the legacy handler ignores its `bool` result, so
/// this port has no error return that could suppress replay.
pub trait SyncPositionPorts {
    /// Resolves a requested VID, or returns `None` when it is absent.
    fn resolve(&mut self, vid: Vid) -> Option<SyncPositionVictim>;

    /// Applies the legacy `SetSyncOwner(actor)` operation and returns its
    /// acceptance result.
    ///
    /// The whole of `CHARACTER::SetSyncOwner`
    /// (`server/server/game/char.cpp:5469-5551`) is the port, because every arm
    /// of it is entity state the pure policy does not own:
    ///
    /// * `AIFLAG_NOMOVE` refuses a monster victim.
    /// * `!battle_is_attackable(actor, victim)` sends a blocked-damage record
    ///   and refuses, which is the PK rule and belongs to `sys.char.pk`.
    /// * `ch == this` refuses, so a character cannot claim itself.
    /// * `!IsSyncOwner(actor)` refuses while another character holds a claim
    ///   that `ENABLE_FLY_FIX`'s 100-unit `m_fSyncTime` has not expired.
    /// * `DISTANCE_APPROX(...) > 250` on the raw coordinates refuses a new
    ///   claim and lets the current owner keep it.
    /// * An accepted claim refreshes `m_fSyncTime` and, when the owner
    ///   changes, resets the last-sync stamp.
    /// * Every accepted claim sends `TPacketGCOwnership` with
    ///   `PacketAround` and no exception, so the victim sees its own
    ///   ownership; that record belongs to this port too.
    fn set_sync_owner(&mut self, actor: Vid, victim: &SyncPositionVictim) -> bool;

    /// Returns the victim's current last-sync timestamp, if one exists.
    fn last_sync_time(&mut self, victim: Vid) -> Option<Duration>;

    /// Stores the victim's accepted-sync timestamp.
    fn set_last_sync_time(&mut self, victim: Vid, now: Duration);

    /// Applies the legacy victim synchronization side effect.
    fn sync(&mut self, victim: Vid, x: i32, y: i32);

    /// Closes the actor descriptor for a policy rejection.
    fn close(&mut self, actor: Vid, reason: SyncPositionCloseReason);

    /// Broadcasts one record to the victim's view, the victim included, which is
    /// `PacketAround` with no exception argument.
    fn broadcast_around_victim(&mut self, victim: Vid, packet: &[u8]);

    /// Broadcasts a complete GC frame around the actor, without the actor.
    ///
    /// `CEntity::PacketView` (`server/server/game/entity.cpp:93-104`) always ends
    /// with `f(std::make_pair(this, 0))`, a self-send, but `FuncPacketAround::operator()`
    /// returns early for `m_except`. `CInputMain::SyncPosition` passes `ch` as that
    /// argument (`input_main.cpp:2161`), so the self-send is suppressed and the
    /// requester never sees the accepted positions. This port therefore delivers to
    /// the view minus the actor.
    fn broadcast(&mut self, actor: Vid, packet: &[u8]);
}

/// Encodes the source-fixed `GC_SYNC_POSITION` frame.
///
/// The accepted-element count is bounded by the legacy gameplay cap, and the
/// `WORD` size is encoded little-endian.  Each element preserves the decoded
/// `u32` VID and signed `i32` coordinates in explicit little-endian order.
///
/// # Errors
///
/// Returns [`SyncPositionEncodeError::TooManyElements`] when more than the
/// legacy 16-element gameplay limit is supplied, or
/// [`SyncPositionEncodeError::SizeOverflow`] if the explicit size cannot fit
/// the source `WORD` field.
pub fn encode_gc_sync_position(
    elements: &[SyncPositionElement],
) -> Result<Vec<u8>, SyncPositionEncodeError> {
    if elements.len() > SYNC_POSITION_ELEMENT_LIMIT {
        return Err(SyncPositionEncodeError::TooManyElements {
            count: elements.len(),
            limit: SYNC_POSITION_ELEMENT_LIMIT,
        });
    }

    let size = SYNC_POSITION_PREFIX_SIZE
        .checked_add(
            elements
                .len()
                .checked_mul(SYNC_POSITION_ELEMENT_SIZE)
                .ok_or(SyncPositionEncodeError::SizeOverflow { size: usize::MAX })?,
        )
        .ok_or(SyncPositionEncodeError::SizeOverflow { size: usize::MAX })?;
    let size_field =
        u16::try_from(size).map_err(|_| SyncPositionEncodeError::SizeOverflow { size })?;

    let mut packet = Vec::with_capacity(size);
    packet.push(HEADER_GC_SYNC_POSITION);
    packet.extend_from_slice(&size_field.to_le_bytes());
    for element in elements {
        packet.extend_from_slice(&element.vid.to_le_bytes());
        packet.extend_from_slice(&element.x.to_le_bytes());
        packet.extend_from_slice(&element.y.to_le_bytes());
    }
    Ok(packet)
}

/// Applies the legacy sync-position policy to an already decoded packet.
///
/// The packet must come from `protocol::cg_variable::decode_sync_position`;
/// that decoder retains all declared elements and has already validated the
/// complete variable frame.  The service nevertheless retains the legacy
/// count cap and consume-on-error behavior here.
pub fn process<P: SyncPositionPorts + ?Sized>(
    ports: &mut P,
    packet: &SyncPositionPacket,
    actor: &mut SyncPositionActor,
    now: Duration,
) -> SyncPositionResult {
    let Some(consumed_extra) = packet.declared_size.checked_sub(SYNC_POSITION_PREFIX_SIZE) else {
        ports.close(actor.vid, SyncPositionCloseReason::InvalidDeclaredSize);
        return SyncPositionResult::closed(
            SyncPositionCloseReason::InvalidDeclaredSize,
            0,
            Vec::new(),
        );
    };

    if consumed_extra % SYNC_POSITION_ELEMENT_SIZE != 0 {
        return SyncPositionResult::consumed(consumed_extra, 0, Vec::new());
    }

    let declared_count = consumed_extra / SYNC_POSITION_ELEMENT_SIZE;
    let count = declared_count.min(SYNC_POSITION_ELEMENT_LIMIT);
    let mut processed_elements = 0;
    let mut accepted_elements = Vec::with_capacity(count);

    for element in packet.elements.iter().take(count) {
        processed_elements += 1;
        let Some(victim) = ports.resolve(Vid::new(element.vid)) else {
            continue;
        };

        if victim.kind.is_skipped() {
            continue;
        }

        if !ports.set_sync_owner(actor.vid, &victim) {
            continue;
        }

        let owner_delta_x = scaled_delta(actor.x, victim.x);
        let owner_delta_y = scaled_delta(actor.y, victim.y);
        if distance_exceeds(owner_delta_x, owner_delta_y, SYNC_OWNER_DISTANCE_LIMIT) {
            if actor.sync_hack_count < SYNC_HACK_LIMIT_COUNT {
                actor.sync_hack_count += 1;
                continue;
            }
            ports.close(actor.vid, SyncPositionCloseReason::OwnerDistanceHackLimit);
            return SyncPositionResult::closed(
                SyncPositionCloseReason::OwnerDistanceHackLimit,
                processed_elements,
                accepted_elements,
            );
        }

        let position_delta_x = scaled_delta(victim.x, element.x);
        let position_delta_y = scaled_delta(victim.y, element.y);
        // `static const long g_lValidSyncInterval = 100 * 1000;` is microseconds, and the
        // test is `tvDiff->tv_sec == 0 && tvDiff->tv_usec < g_lValidSyncInterval`. A stamp in
        // the future gives a negative difference, so it is not "too soon", and
        // `checked_sub` returning `None` is the same answer as legacy's `tv_sec != 0` arm.
        let too_soon = ports
            .last_sync_time(victim.vid)
            .and_then(|last| now.checked_sub(last))
            .is_some_and(|age| age < SYNC_VALID_INTERVAL);
        if too_soon {
            if actor.sync_hack_count < SYNC_HACK_LIMIT_COUNT {
                actor.sync_hack_count += 1;
                continue;
            }
            ports.close(actor.vid, SyncPositionCloseReason::SyncIntervalHackLimit);
            return SyncPositionResult::closed(
                SyncPositionCloseReason::SyncIntervalHackLimit,
                processed_elements,
                accepted_elements,
            );
        }

        if distance_exceeds(position_delta_x, position_delta_y, SYNC_DISPLACEMENT_LIMIT) {
            ports.close(actor.vid, SyncPositionCloseReason::SyncDisplacement);
            return SyncPositionResult::closed(
                SyncPositionCloseReason::SyncDisplacement,
                processed_elements,
                accepted_elements,
            );
        }

        ports.set_last_sync_time(victim.vid, now);
        ports.sync(victim.vid, element.x, element.y);
        accepted_elements.push(*element);
    }

    let result =
        SyncPositionResult::consumed(consumed_extra, processed_elements, accepted_elements)
            .with_gc_packet();
    if let Some(packet) = result.gc_packet.as_deref() {
        ports.broadcast(actor.vid, packet);
    }
    result
}

fn scaled_delta(from: i32, to: i32) -> i64 {
    (i64::from(to) - i64::from(from)) / 100
}

#[allow(clippy::cast_precision_loss)]
fn distance_exceeds(dx: i64, dy: i64, limit: f32) -> bool {
    let dx = dx as f32;
    let dy = dy as f32;
    (dx * dx + dy * dy).sqrt() > limit
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use common::vid::Vid;
    use protocol::cg_variable::{SyncPositionElement, SyncPositionPacket};

    use super::{
        distance_approx, encode_gc_sync_position, judge_sync_ownership, ownership_record, process,
        SyncOwnershipOutcome, SyncOwnershipState, SyncPositionActor, SyncPositionCloseReason,
        SyncPositionDisposition, SyncPositionEncodeError, SyncPositionPorts, SyncPositionResult,
        SyncPositionVictim, SyncPositionVictimKind, HEADER_GC_SYNC_POSITION, SYNC_HACK_LIMIT_COUNT,
        SYNC_POSITION_ELEMENT_LIMIT, SYNC_POSITION_PREFIX_SIZE, SYNC_VALID_INTERVAL,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Resolve(u32),
        Owner(u32, u32),
        LastSync(u32),
        SetLastSync(u32, Duration),
        Sync(u32, i32, i32),
        Close(u32, SyncPositionCloseReason),
        Broadcast(u32, Vec<u8>),
        Around(u32, Vec<u8>),
    }

    /// A player victim on the same map, at the origin.
    fn at_origin(vid: u32) -> SyncPositionVictim {
        SyncPositionVictim {
            vid: Vid::new(vid),
            kind: SyncPositionVictimKind::Player,
            x: 0,
            y: 0,
        }
    }

    /// Never claimed, which is the state `CHARACTER::CHARACTER`'s `get_float_time() - 3`
    /// seed leaves behind.
    fn unowned() -> SyncOwnershipState {
        SyncOwnershipState {
            owner: None,
            claimed_at: Duration::ZERO,
        }
    }

    #[test]
    fn ownership_record_is_the_source_field_order() {
        // `packet.h:1716-1720`: header, dwOwnerVID, dwVictimVID, little-endian.
        assert_eq!(
            ownership_record(Vid::new(0x0102_0304), Vid::new(0x0a0b_0c0d)),
            [62, 4, 3, 2, 1, 0x0d, 0x0c, 0x0b, 0x0a]
        );
    }

    #[test]
    fn distance_approx_matches_the_shifted_source_form() {
        // `(123/128) * max + (51/128) * min`, from `utils.h:17-40`.
        assert_eq!(distance_approx(0, 0), 0);
        assert_eq!(distance_approx(128, 0), 123);
        assert_eq!(distance_approx(0, 128), 123);
        assert_eq!(distance_approx(128, 128), 174);
        // The sign of either argument is taken away before the coefficients apply.
        assert_eq!(distance_approx(-200, 300), distance_approx(200, 300));
        // A zero on one axis stays an exact multiple of the other.
        assert_eq!(distance_approx(1000, 0), 960);
    }

    #[test]
    fn a_claim_on_oneself_is_refused() {
        // `if (ch == this) { sys_err("SetSyncOwner owner == this"); return false; }`
        let vid = Vid::new(7);
        assert_eq!(
            judge_sync_ownership(vid, &at_origin(7), unowned(), 0, 0, Duration::ZERO),
            SyncOwnershipOutcome::Refused
        );
    }

    #[test]
    fn an_unclaimed_victim_is_accepted_and_its_stamp_is_reset() {
        let actor = Vid::new(1);
        assert_eq!(
            judge_sync_ownership(actor, &at_origin(2), unowned(), 0, 0, Duration::ZERO),
            SyncOwnershipOutcome::Accepted {
                owner_changed: true,
                record: ownership_record(actor, Vid::new(2)),
            }
        );
    }

    #[test]
    fn a_second_character_is_refused_while_the_claim_is_fresh() {
        let actor = Vid::new(3);
        let state = SyncOwnershipState {
            owner: Some(Vid::new(1)),
            claimed_at: Duration::from_millis(20),
        };
        assert_eq!(
            judge_sync_ownership(actor, &at_origin(2), state, 0, 0, Duration::from_millis(50)),
            SyncOwnershipOutcome::Refused
        );
    }

    #[test]
    fn a_claim_takes_over_once_the_stamp_is_old_enough() {
        // `ENABLE_FLY_FIX` selects the 100-unit form of `IsSyncOwner`.
        let actor = Vid::new(3);
        let state = SyncOwnershipState {
            owner: Some(Vid::new(1)),
            claimed_at: Duration::from_millis(20),
        };
        assert_eq!(
            judge_sync_ownership(
                actor,
                &at_origin(2),
                state,
                0,
                0,
                Duration::from_millis(120)
            ),
            SyncOwnershipOutcome::Accepted {
                owner_changed: true,
                record: ownership_record(actor, Vid::new(2)),
            }
        );
    }

    #[test]
    fn the_current_owner_may_claim_again_without_a_reset() {
        let actor = Vid::new(1);
        let state = SyncOwnershipState {
            owner: Some(actor),
            claimed_at: Duration::ZERO,
        };
        assert_eq!(
            judge_sync_ownership(actor, &at_origin(2), state, 0, 0, Duration::from_millis(1)),
            SyncOwnershipOutcome::Accepted {
                owner_changed: false,
                record: ownership_record(actor, Vid::new(2)),
            }
        );
    }

    #[test]
    fn a_new_owner_is_refused_past_the_approximate_distance() {
        // `DISTANCE_APPROX(GetX() - ch->GetX(), GetY() - ch->GetY()) > 250`. The limit is on
        // the *approximated* distance, not the raw one, so with only x set the boundary is
        // 123/128 of the raw value: 260 approximates to 249 and 261 to 250, and only a
        // value above 250 refuses. 260 raw units is 2.6 map units.
        let actor = Vid::new(1);
        assert_eq!(distance_approx(260, 0), 249, "just inside the limit");
        assert_eq!(distance_approx(261, 0), 250, "on the limit");
        let mut far = at_origin(2);
        far.x = 262;
        assert_eq!(
            judge_sync_ownership(actor, &far, unowned(), 0, 0, Duration::ZERO),
            SyncOwnershipOutcome::Refused
        );
        let mut near = at_origin(2);
        near.x = 261;
        assert!(matches!(
            judge_sync_ownership(actor, &near, unowned(), 0, 0, Duration::ZERO),
            SyncOwnershipOutcome::Accepted { .. }
        ));
    }

    #[test]
    fn the_current_owner_keeps_a_claim_past_the_approximate_distance() {
        // `if (m_pkChrSyncOwner == ch) return true;`, before the stamp and the record.
        let actor = Vid::new(1);
        let mut far = at_origin(2);
        far.x = 4000;
        let state = SyncOwnershipState {
            owner: Some(actor),
            claimed_at: Duration::ZERO,
        };
        assert_eq!(
            judge_sync_ownership(actor, &far, state, 0, 0, Duration::from_millis(1)),
            SyncOwnershipOutcome::Kept
        );
        // Just inside the limit the same owner is accepted again, with a record.
        far.x = 261;
        assert_eq!(
            judge_sync_ownership(actor, &far, state, 0, 0, Duration::from_millis(1)),
            SyncOwnershipOutcome::Accepted {
                owner_changed: false,
                record: ownership_record(actor, Vid::new(2)),
            }
        );
    }

    #[test]
    fn the_distance_limit_is_checked_before_the_stamp_arms_the_takeover() {
        // A far-away new owner is refused on distance even when the old claim has expired,
        // because the source tests `IsSyncOwner` first and distance second, and both
        // refusals land on the same answer.
        let actor = Vid::new(1);
        let mut far = at_origin(2);
        far.y = 1000;
        let state = SyncOwnershipState {
            owner: Some(Vid::new(9)),
            claimed_at: Duration::ZERO,
        };
        assert_eq!(
            judge_sync_ownership(actor, &far, state, 0, 0, Duration::from_secs(60)),
            SyncOwnershipOutcome::Refused
        );
    }

    #[derive(Debug, Default)]
    struct MockPorts {
        victims: HashMap<u32, SyncPositionVictim>,
        last_sync: HashMap<u32, Duration>,
        owner_accepted: bool,
        owner_calls: Vec<(u32, u32)>,
        calls: Vec<Call>,
        broadcasts: Vec<(u32, Vec<u8>)>,
    }

    impl MockPorts {
        fn with_victims(victims: impl IntoIterator<Item = SyncPositionVictim>) -> Self {
            Self {
                victims: victims
                    .into_iter()
                    .map(|victim| (victim.vid.raw(), victim))
                    .collect(),
                owner_accepted: true,
                ..Self::default()
            }
        }
    }

    impl SyncPositionPorts for MockPorts {
        fn broadcast_around_victim(&mut self, victim: Vid, packet: &[u8]) {
            self.calls.push(Call::Around(victim.raw(), packet.to_vec()));
        }

        fn resolve(&mut self, vid: Vid) -> Option<SyncPositionVictim> {
            self.calls.push(Call::Resolve(vid.raw()));
            self.victims.get(&vid.raw()).copied()
        }

        fn set_sync_owner(&mut self, actor: Vid, victim: &SyncPositionVictim) -> bool {
            self.owner_calls.push((actor.raw(), victim.vid.raw()));
            self.calls.push(Call::Owner(actor.raw(), victim.vid.raw()));
            self.owner_accepted
        }

        fn last_sync_time(&mut self, victim: Vid) -> Option<Duration> {
            self.calls.push(Call::LastSync(victim.raw()));
            self.last_sync.get(&victim.raw()).copied()
        }

        fn set_last_sync_time(&mut self, victim: Vid, now: Duration) {
            self.calls.push(Call::SetLastSync(victim.raw(), now));
            self.last_sync.insert(victim.raw(), now);
        }

        fn sync(&mut self, victim: Vid, x: i32, y: i32) {
            self.calls.push(Call::Sync(victim.raw(), x, y));
        }

        fn close(&mut self, actor: Vid, reason: SyncPositionCloseReason) {
            self.calls.push(Call::Close(actor.raw(), reason));
        }

        fn broadcast(&mut self, actor: Vid, packet: &[u8]) {
            self.calls
                .push(Call::Broadcast(actor.raw(), packet.to_vec()));
            self.broadcasts.push((actor.raw(), packet.to_vec()));
        }
    }

    fn element(vid: u32, x: i32, y: i32) -> SyncPositionElement {
        SyncPositionElement { vid, x, y }
    }

    fn packet(elements: &[SyncPositionElement]) -> SyncPositionPacket {
        SyncPositionPacket {
            declared_size: SYNC_POSITION_PREFIX_SIZE + elements.len() * 12,
            elements: elements.to_vec(),
        }
    }

    fn victim(vid: u32, kind: SyncPositionVictimKind, x: i32, y: i32) -> SyncPositionVictim {
        SyncPositionVictim {
            vid: Vid::new(vid),
            kind,
            x,
            y,
        }
    }

    fn actor() -> SyncPositionActor {
        SyncPositionActor {
            vid: Vid::new(1),
            x: 0,
            y: 0,
            sync_hack_count: 0,
        }
    }

    fn no_effect_result() -> SyncPositionResult {
        SyncPositionResult {
            disposition: SyncPositionDisposition::Consumed,
            consumed_extra: Some(0),
            processed_elements: 0,
            accepted_elements: Vec::new(),
            gc_packet: None,
            close_reason: None,
        }
    }

    #[test]
    fn caps_processing_at_sixteen_but_consumes_all_declared_elements() {
        let elements = (1..=17)
            .map(|vid| element(u32::try_from(vid).unwrap(), 0, 0))
            .collect::<Vec<_>>();
        let victims = (1..=17).map(|vid| victim(vid, SyncPositionVictimKind::Player, 0, 0));
        let mut ports = MockPorts::with_victims(victims);
        let mut actor = actor();
        let result = process(&mut ports, &packet(&elements), &mut actor, Duration::ZERO);

        assert_eq!(result.disposition, SyncPositionDisposition::Consumed);
        assert_eq!(result.consumed_extra, Some(17 * 12));
        assert_eq!(result.processed_elements, 16);
        assert_eq!(result.accepted_elements, elements[..16]);
        assert_eq!(ports.sync_calls().len(), 16);
        assert_eq!(ports.broadcasts.len(), 1);
        assert_eq!(ports.broadcasts[0].1[0], HEADER_GC_SYNC_POSITION);
        assert_eq!(ports.broadcasts[0].1.len(), 3 + 16 * 12);
    }

    #[test]
    fn skips_missing_ignored_kinds_and_owner_rejection() {
        let valid = element(4, 10, 20);
        let packet = packet(&[
            element(99, 0, 0),
            element(1, 0, 0),
            element(2, 0, 0),
            element(3, 0, 0),
            valid,
        ]);
        let mut ports = MockPorts::with_victims([
            victim(1, SyncPositionVictimKind::Npc, 0, 0),
            victim(2, SyncPositionVictimKind::Warp, 0, 0),
            victim(3, SyncPositionVictimKind::Goto, 0, 0),
            victim(4, SyncPositionVictimKind::Player, 0, 0),
        ]);
        ports.owner_accepted = false;
        let mut actor = actor();
        let result = process(&mut ports, &packet, &mut actor, Duration::ZERO);

        assert_eq!(result.accepted_elements, Vec::<SyncPositionElement>::new());
        assert!(ports.broadcasts.is_empty());
        assert_eq!(ports.owner_calls, vec![(1, 4)]);
    }

    #[test]
    fn owner_distance_uses_scaled_integer_boundaries() {
        let exact = packet(&[element(1, 350_000, 0)]);
        let mut ports =
            MockPorts::with_victims([victim(1, SyncPositionVictimKind::Player, 350_000, 0)]);
        let mut actor = actor();
        let result = process(&mut ports, &exact, &mut actor, Duration::from_secs(1));
        assert_eq!(result.accepted_elements.len(), 1);
        assert_eq!(actor.sync_hack_count, 0);

        let over = packet(&[element(1, 350_100, 0)]);
        ports
            .victims
            .insert(1, victim(1, SyncPositionVictimKind::Player, 350_100, 0));
        let result = process(&mut ports, &over, &mut actor, Duration::from_secs(1));
        assert!(result.accepted_elements.is_empty());
        assert_eq!(actor.sync_hack_count, 1);
        assert!(result.gc_packet.is_none());
    }

    #[test]
    fn displacement_limit_is_inclusive_and_closes_over_limit() {
        let mut ports = MockPorts::with_victims([victim(1, SyncPositionVictimKind::Player, 0, 0)]);
        let mut actor = actor();
        let now = Duration::from_secs(1);

        let exact = packet(&[element(1, 2_500, 0)]);
        let result = process(&mut ports, &exact, &mut actor, now);
        assert_eq!(result.accepted_elements.len(), 1);
        assert_eq!(result.disposition, SyncPositionDisposition::Consumed);

        let over = packet(&[element(1, 2_600, 0)]);
        let result = process(&mut ports, &over, &mut actor, now + SYNC_VALID_INTERVAL);
        assert_eq!(result.disposition, SyncPositionDisposition::Closed);
        assert_eq!(
            result.close_reason,
            Some(SyncPositionCloseReason::SyncDisplacement)
        );
        assert!(ports.broadcasts.len() == 1);
    }

    #[test]
    fn interval_is_less_than_one_hundred_milliseconds_and_count_closes_at_limit() {
        let mut ports = MockPorts::with_victims([victim(1, SyncPositionVictimKind::Player, 0, 0)]);
        ports.last_sync.insert(1, Duration::from_millis(1));
        let mut actor = actor();
        let packet = packet(&[element(1, 0, 0)]);

        let too_soon = process(&mut ports, &packet, &mut actor, Duration::from_millis(100));
        assert!(too_soon.accepted_elements.is_empty());
        assert_eq!(actor.sync_hack_count, 1);
        assert_eq!(too_soon.disposition, SyncPositionDisposition::Consumed);

        actor.sync_hack_count = SYNC_HACK_LIMIT_COUNT - 1;
        let still_under_limit =
            process(&mut ports, &packet, &mut actor, Duration::from_millis(100));
        assert_eq!(
            still_under_limit.disposition,
            SyncPositionDisposition::Consumed
        );
        assert_eq!(actor.sync_hack_count, SYNC_HACK_LIMIT_COUNT);

        let close = process(&mut ports, &packet, &mut actor, Duration::from_millis(100));
        assert_eq!(close.disposition, SyncPositionDisposition::Closed);
        assert_eq!(
            close.close_reason,
            Some(SyncPositionCloseReason::SyncIntervalHackLimit)
        );

        ports.last_sync.insert(1, Duration::from_millis(100));
        actor.sync_hack_count = 0;
        let exact = process(&mut ports, &packet, &mut actor, Duration::from_millis(200));
        assert_eq!(exact.disposition, SyncPositionDisposition::Consumed);
    }

    #[test]
    fn accepted_elements_keep_order_and_encode_exact_gc_bytes() {
        let first = element(7, -1, 2);
        let second = element(8, 300, -400);
        let mut ports = MockPorts::with_victims([
            victim(7, SyncPositionVictimKind::Monster, 0, 0),
            victim(8, SyncPositionVictimKind::Player, 0, 0),
        ]);
        let mut actor = actor();
        let result = process(
            &mut ports,
            &packet(&[first, second]),
            &mut actor,
            Duration::from_secs(2),
        );

        let expected = vec![
            0x05, 27, 0, 7, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 2, 0, 0, 0, 8, 0, 0, 0, 44, 1, 0, 0,
            0x70, 0xfe, 0xff, 0xff,
        ];
        assert_eq!(result.gc_packet.as_deref(), Some(expected.as_slice()));
        assert_eq!(result.accepted_elements, vec![first, second]);
        assert_eq!(ports.sync_calls(), vec![(7, -1, 2), (8, 300, -400)]);
        assert_eq!(ports.broadcasts, vec![(1, expected)]);
    }

    #[test]
    fn nonaligned_and_zero_element_frames_are_consumed_without_effects() {
        let mut ports = MockPorts::default();
        let mut actor = actor();
        let nonaligned = SyncPositionPacket {
            declared_size: 4,
            elements: Vec::new(),
        };
        let result = process(&mut ports, &nonaligned, &mut actor, Duration::ZERO);
        assert_eq!(result.consumed_extra, Some(1));
        assert_eq!(result.processed_elements, 0);
        assert!(result.accepted_elements.is_empty());
        assert!(ports.calls.is_empty());

        let zero = packet(&[]);
        let result = process(&mut ports, &zero, &mut actor, Duration::ZERO);
        assert_eq!(result, no_effect_result());
        assert!(ports.calls.is_empty());
    }

    #[test]
    fn invalid_declared_size_closes_without_normal_consumption() {
        let mut ports = MockPorts::default();
        let mut actor = actor();
        let packet = SyncPositionPacket {
            declared_size: 2,
            elements: Vec::new(),
        };
        let result = process(&mut ports, &packet, &mut actor, Duration::ZERO);
        assert_eq!(result.disposition, SyncPositionDisposition::Closed);
        assert_eq!(result.consumed_extra, None);
        assert_eq!(
            result.close_reason,
            Some(SyncPositionCloseReason::InvalidDeclaredSize)
        );
        assert_eq!(
            ports.calls,
            vec![Call::Close(1, SyncPositionCloseReason::InvalidDeclaredSize)]
        );
    }

    #[test]
    fn displacement_rejection_does_not_discard_prior_side_effects_or_replay() {
        let first = element(1, 0, 0);
        let second = element(2, 0, 0);
        let mut ports = MockPorts::with_victims([
            victim(1, SyncPositionVictimKind::Player, 0, 0),
            victim(2, SyncPositionVictimKind::Player, 0, 0),
        ]);
        let mut actor = actor();
        let result = process(
            &mut ports,
            &packet(&[first, second]),
            &mut actor,
            Duration::from_secs(1),
        );
        assert_eq!(result.disposition, SyncPositionDisposition::Consumed);
        assert_eq!(result.accepted_elements, vec![first, second]);
        assert_eq!(ports.sync_calls(), vec![(1, 0, 0), (2, 0, 0)]);
    }

    #[test]
    fn encoder_rejects_more_than_the_legacy_limit() {
        let elements = (0..=SYNC_POSITION_ELEMENT_LIMIT)
            .map(|vid| element(u32::try_from(vid).unwrap(), 0, 0))
            .collect::<Vec<_>>();
        assert_eq!(
            encode_gc_sync_position(&elements),
            Err(SyncPositionEncodeError::TooManyElements {
                count: 17,
                limit: 16,
            })
        );
    }

    impl MockPorts {
        fn sync_calls(&self) -> Vec<(u32, i32, i32)> {
            self.calls
                .iter()
                .filter_map(|call| match call {
                    Call::Sync(vid, x, y) => Some((*vid, *x, *y)),
                    _ => None,
                })
                .collect()
        }
    }
}
