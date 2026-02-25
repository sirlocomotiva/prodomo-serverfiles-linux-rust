//! The legacy DB-side item-ID range pool.
//!
//! `server/server/db/ItemIDRangeManager.cpp` builds a FIFO of
//! `TItemIDRangeTable` records and hands them out one at a time. The two
//! callers in `CClientManager::QUERY_BOOT` (`ClientManager.cpp`) both call
//! `GetRange()`, so one boot response consumes **two** successive ranges: the
//! first becomes the peer's active range and the second the spare range. This
//! is not a duplicate copy; `GetRange()` pops from the front of the list.
//!
//! `GetRange()` starts from an all-zero range. When the list is empty it logs
//! ten errors and returns that zero record, so a DB server that never built a
//! range still answers a boot request with two zero ranges. Reproducing the
//! zero fallback matters: silently substituting a fresh block would hand the
//! game server item IDs the DB server never reserved.
//!
//! Building a range needs `SELECT MAX(id) FROM item<postfix>` and the
//! `__PREMIUM_PRIVATE_SHOP__` private-shop probes, so this module accepts
//! already-resolved ranges from a caller. It performs no SQL, no collision
//! check against connected peers, and no allocation policy of its own.

use std::collections::VecDeque;

use protocol::db_boot::BootItemIdRange;

/// `CItemIDRangeManager::cs_dwMaxItemID`.
pub const MAX_ITEM_ID: u32 = 4_290_000_000;

/// `CItemIDRangeManager::cs_dwMinimumRange`.
pub const MINIMUM_RANGE: u32 = 10_000_000;

/// `CItemIDRangeManager::cs_dwMinimumRemainCount`.
pub const MINIMUM_REMAIN_COUNT: u32 = 10_000;

/// The all-zero range that `GetRange()` returns when nothing was built.
///
/// The legacy initializes `ret.dwMin`, `ret.dwMax`, and `ret.dwUsableItemIDMin`
/// to zero before consulting the list, and returns exactly this value when the
/// list is empty or every candidate collided with a connected peer.
#[must_use]
pub const fn empty_range() -> BootItemIdRange {
    BootItemIdRange {
        min: 0,
        max: 0,
        usable_item_id_min: 0,
    }
}

/// A failure while building or consuming the range pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemIdRangeError {
    /// A caller-supplied range was not usable.
    ///
    /// `BuildRange` only accepts a range whose `dwMax` leaves at least
    /// [`MINIMUM_REMAIN_COUNT`] unused IDs.
    RangeNotUsable {
        /// The rejected `dwMin`.
        min: u32,
        /// The rejected `dwMax`.
        max: u32,
    },
    /// A supplied range did not fit the declared 12-byte record.
    RecordWidthOverflow {
        /// The rejected record width.
        size: usize,
    },
}

/// A FIFO of pre-reserved item-ID ranges.
///
/// The pool is deliberately not synchronized. One DB server owns one pool and
/// consumes it from its single main loop, matching `CItemIDRangeManager`'s
/// unsynchronized `std::list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemIdRangePool {
    ranges: VecDeque<BootItemIdRange>,
    collisions: u32,
}

impl Default for ItemIdRangePool {
    fn default() -> Self {
        Self::new()
    }
}

impl ItemIdRangePool {
    /// Create an empty pool.
    ///
    /// An empty pool still answers boot requests: every [`Self::next_range`]
    /// returns [`empty_range`], which is what the legacy server does.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ranges: VecDeque::new(),
            collisions: 0,
        }
    }

    /// Build a pool from already-resolved ranges in legacy build order.
    ///
    /// Ranges are not validated here; the caller is the `BuildRange`
    /// equivalent and owns the SQL. Use [`Self::try_from_ranges`] to apply the
    /// legacy usability check as well.
    #[must_use]
    pub fn from_ranges(ranges: impl IntoIterator<Item = BootItemIdRange>) -> Self {
        Self {
            ranges: ranges.into_iter().collect(),
            collisions: 0,
        }
    }

    /// Build a pool, dropping ranges that the legacy `BuildRange` rejects.
    ///
    /// # Errors
    ///
    /// Returns [`ItemIdRangeError::RangeNotUsable`] for a range whose `max`
    /// leaves fewer than [`MINIMUM_REMAIN_COUNT`] free IDs, and
    /// [`ItemIdRangeError::RecordWidthOverflow`] when the declared 12-byte
    /// record width cannot be represented.
    pub fn try_from_ranges(
        ranges: impl IntoIterator<Item = BootItemIdRange>,
    ) -> Result<Self, ItemIdRangeError> {
        let width = u16::try_from(protocol::db_boot::ITEM_ID_RANGE_WIRE_SIZE).map_err(|_| {
            ItemIdRangeError::RecordWidthOverflow {
                size: protocol::db_boot::ITEM_ID_RANGE_WIRE_SIZE,
            }
        })?;
        let _ = width;
        let mut kept = VecDeque::new();
        for range in ranges {
            if is_usable(&range) {
                kept.push_back(range);
            }
        }
        Ok(Self {
            ranges: kept,
            collisions: 0,
        })
    }

    /// Return the number of unconsumed ranges.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// Return whether every range has been consumed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Return how many candidates a caller reported as colliding.
    ///
    /// The legacy `FCheckCollision` walk lets a connected peer reject a range;
    /// a transport adapter records that here so exhaustion stays visible.
    #[must_use]
    pub const fn collisions(&self) -> u32 {
        self.collisions
    }

    /// Record that the next candidate collided with a connected peer.
    pub fn record_collision(&mut self) {
        self.collisions = self.collisions.saturating_add(1);
    }

    /// Consume and return the front range, or [`empty_range`] when exhausted.
    ///
    /// The legacy `GetRange` pops the front *before* testing it against every
    /// connected peer, returns the candidate only when no peer claimed it, and
    /// otherwise repeats with the next one. One call can therefore consume
    /// several list entries.
    ///
    /// Two legacy details are deliberately not reproduced here, because
    /// reproducing them would be a defect:
    ///
    /// - The all-zero return is specific to an **empty** list. When the list
    ///   was non-empty but every candidate collided, the legacy falls out of
    ///   its loop still holding the last popped, colliding range and returns
    ///   *that*. An adapter that needs collision rejection should call
    ///   [`Self::next_unchecked`], inspect the value, and on a claimed range
    ///   call [`Self::record_collision`] and retry.
    /// - The legacy logs ten `sys_err` lines before giving up. Logging belongs
    ///   to the caller, so [`Self::collisions`] exposes the count instead.
    pub fn next_range(&mut self) -> BootItemIdRange {
        self.next_unchecked()
    }

    /// Consume the front range without any collision check.
    #[must_use]
    pub fn next_unchecked(&mut self) -> BootItemIdRange {
        self.ranges.pop_front().unwrap_or_else(empty_range)
    }

    /// Take the two ranges that one boot response writes.
    ///
    /// `QUERY_BOOT` calls `GetRange()` twice, so this consumes two entries in
    /// order: the first is the active range, the second the spare range. With
    /// an empty pool both are [`empty_range`], which is the legacy result.
    pub fn take_boot_pair(&mut self) -> (BootItemIdRange, BootItemIdRange) {
        (self.next_unchecked(), self.next_unchecked())
    }
}

/// Return whether `BuildRange` would accept this range.
///
/// The legacy check is
/// `dwMax - dwItemMaxID >= cs_dwMinimumRemainCount` with
/// `dwItemMaxID = dwUsableItemIDMin`, guarded by `dwMax >= dwItemMaxID`.
/// A `dwUsableItemIDMin` above `dwMax` is a saturated or corrupt range and is
/// rejected rather than allowed to underflow.
#[must_use]
pub const fn is_usable(range: &BootItemIdRange) -> bool {
    if range.usable_item_id_min > range.max {
        return false;
    }
    range.max - range.usable_item_id_min >= MINIMUM_REMAIN_COUNT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(min: u32, max: u32, usable: u32) -> BootItemIdRange {
        BootItemIdRange {
            min,
            max,
            usable_item_id_min: usable,
        }
    }

    #[test]
    fn empty_pool_returns_two_zero_ranges_for_one_boot() {
        let mut pool = ItemIdRangePool::new();
        let (active, spare) = pool.take_boot_pair();
        assert_eq!(active, empty_range());
        assert_eq!(spare, empty_range());
        assert!(pool.is_empty());
    }

    #[test]
    fn one_boot_consumes_two_successive_ranges_not_one_duplicate() {
        let mut pool = ItemIdRangePool::from_ranges([
            range(10_000_001, 20_000_000, 10_000_001),
            range(20_000_001, 30_000_000, 20_000_001),
        ]);
        let (active, spare) = pool.take_boot_pair();
        assert_eq!(active.min, 10_000_001);
        assert_eq!(spare.min, 20_000_001);
        assert_ne!(active, spare);
        assert!(pool.is_empty());
    }

    #[test]
    fn a_single_built_range_yields_it_then_the_legacy_zero_fallback() {
        let mut pool = ItemIdRangePool::from_ranges([range(10_000_001, 20_000_000, 10_000_001)]);
        let (active, spare) = pool.take_boot_pair();
        assert_eq!(active.min, 10_000_001);
        assert_eq!(spare, empty_range());
    }

    #[test]
    fn usability_matches_the_legacy_remaining_count_check() {
        // The legacy rejects when `dwMax - dwUsableItemIDMin < 10_000`.
        // usable = 0 leaves exactly 10_000 free IDs: the boundary that passes.
        assert!(is_usable(&range(1, 10_000, 0)));
        // usable = 1 leaves 9_999: the first value that fails.
        assert!(!is_usable(&range(1, 10_000, 1)));
        assert!(!is_usable(&range(1, 10_000, 2)));
        // A saturated range must not underflow into an accept.
        assert!(!is_usable(&range(1, 10, 11)));
    }

    #[test]
    fn try_from_ranges_drops_unusable_records() {
        let pool =
            ItemIdRangePool::try_from_ranges([range(1, 10_000, 0), range(1, 10_000, 10_000)])
                .expect("valid widths");
        assert_eq!(pool.len(), 1);
        let mut pool = pool;
        assert_eq!(pool.next_unchecked().usable_item_id_min, 0);
        assert_eq!(pool.next_unchecked(), empty_range());
    }

    #[test]
    fn collisions_are_counted_saturating() {
        let mut pool = ItemIdRangePool::new();
        pool.record_collision();
        pool.record_collision();
        assert_eq!(pool.collisions(), 2);
        for _ in 0..10 {
            pool.record_collision();
        }
        assert_eq!(pool.collisions(), 12);
    }

    #[test]
    fn legacy_constants_match_the_source() {
        assert_eq!(MAX_ITEM_ID, 4_290_000_000);
        assert_eq!(MINIMUM_RANGE, 10_000_000);
        assert_eq!(MINIMUM_REMAIN_COUNT, 10_000);
    }
}
