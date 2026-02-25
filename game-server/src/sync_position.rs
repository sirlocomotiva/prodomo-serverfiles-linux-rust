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
    /// acceptance result.  Any ownership packets or damage-block effects belong
    /// to the implementation of this port.
    fn set_sync_owner(&mut self, actor: Vid, victim: &SyncPositionVictim) -> bool;

    /// Returns the victim's current last-sync timestamp, if one exists.
    fn last_sync_time(&mut self, victim: Vid) -> Option<Duration>;

    /// Stores the victim's accepted-sync timestamp.
    fn set_last_sync_time(&mut self, victim: Vid, now: Duration);

    /// Applies the legacy victim synchronization side effect.
    fn sync(&mut self, victim: Vid, x: i32, y: i32);

    /// Closes the actor descriptor for a policy rejection.
    fn close(&mut self, actor: Vid, reason: SyncPositionCloseReason);

    /// Broadcasts a complete GC frame around the actor.  Although the legacy
    /// call passes the actor as the `except` argument, `PacketView` always
    /// sends the frame to the entity itself after skipping that entity in the
    /// view loop; this port therefore preserves the source self-send quirk.
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
        let too_soon = ports.last_sync_time(victim.vid).is_some_and(|last| {
            now.checked_sub(last).unwrap_or(Duration::ZERO) < SYNC_VALID_INTERVAL
        });
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
        encode_gc_sync_position, process, SyncPositionActor, SyncPositionCloseReason,
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
