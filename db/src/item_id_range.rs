//! The item-ID range pool.
//!
//! Legacy `server/server/db/ItemIDRangeManager.cpp` built a FIFO of
//! `TItemIDRangeTable` records, and each game server took two of them at boot
//! (`QUERY_BOOT` called `GetRange()` twice): an active range and a spare range.
//! The game side (`game/item_manager_idrange.cpp`) handed out IDs from
//! `dwUsableItemIDMin` upward, switched to the spare range once the active one
//! reached `dwMax`, and asked for a new spare.
//!
//! Building a range needs `SELECT MAX(id)` over the item table, so this module
//! accepts already-resolved ranges. It performs no SQL.
//!
//! Legacy `GetRange()` returned an all-zero record when the list was empty,
//! and the game server treated a zero field as "not set" and shut down
//! (`item_manager_idrange.cpp:55-61`). Here an exhausted pool returns `None`,
//! so the sentinel cannot be mistaken for a range.

use std::collections::VecDeque;

/// `CItemIDRangeManager::cs_dwMaxItemID`.
pub const MAX_ITEM_ID: u32 = 4_290_000_000;

/// `CItemIDRangeManager::cs_dwMinimumRange`.
pub const MINIMUM_RANGE: u32 = 10_000_000;

/// `CItemIDRangeManager::cs_dwMinimumRemainCount`.
pub const MINIMUM_REMAIN_COUNT: u32 = 10_000;

/// One block of item IDs, as the legacy `TItemIDRangeTable` held it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemIdRange {
    /// `dwMin`: the first ID in the block.
    pub min: u32,
    /// `dwMax`: the end of the block. Legacy switches ranges once the next ID
    /// reaches this value, so it is never handed out.
    pub max: u32,
    /// `dwUsableItemIDMin`: the first ID not already used by a stored item.
    pub usable_item_id_min: u32,
}

impl ItemIdRange {
    /// Whether legacy `BuildRange` would accept this range.
    ///
    /// The legacy check is `dwMax - dwUsableItemIDMin >= 10_000`. A
    /// `usable_item_id_min` above `max` is a saturated or corrupt range and is
    /// rejected rather than allowed to underflow.
    #[must_use]
    pub const fn is_usable(&self) -> bool {
        if self.usable_item_id_min > self.max {
            return false;
        }
        self.max - self.usable_item_id_min >= MINIMUM_REMAIN_COUNT
    }
}

/// A FIFO of pre-reserved item-ID ranges.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemIdRangePool {
    ranges: VecDeque<ItemIdRange>,
}

impl ItemIdRangePool {
    /// Build a pool from resolved ranges in build order, dropping every range
    /// [`ItemIdRange::is_usable`] rejects.
    #[must_use]
    pub fn new(ranges: impl IntoIterator<Item = ItemIdRange>) -> Self {
        Self {
            ranges: ranges.into_iter().filter(ItemIdRange::is_usable).collect(),
        }
    }

    /// The number of unconsumed ranges.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// Whether every range has been consumed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Consume and return the front range, or `None` when the pool is empty.
    ///
    /// Legacy `GetRange` also tested each candidate against every connected
    /// game server. With one process there is one consumer, so no collision
    /// walk exists.
    pub fn next_range(&mut self) -> Option<ItemIdRange> {
        self.ranges.pop_front()
    }

    /// Take the active range and then the spare range, as one legacy boot did.
    ///
    /// The two are successive entries, never one range twice. `None` means the
    /// pool could not supply both; legacy shut the game server down in that
    /// case. The pool is left unchanged when it holds fewer than two ranges.
    pub fn take_active_and_spare(&mut self) -> Option<(ItemIdRange, ItemIdRange)> {
        if self.ranges.len() < 2 {
            return None;
        }
        Some((self.ranges.pop_front()?, self.ranges.pop_front()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(min: u32, max: u32, usable: u32) -> ItemIdRange {
        ItemIdRange {
            min,
            max,
            usable_item_id_min: usable,
        }
    }

    #[test]
    fn an_empty_pool_supplies_nothing() {
        let mut pool = ItemIdRangePool::default();
        assert_eq!(pool.next_range(), None);
        assert_eq!(pool.take_active_and_spare(), None);
    }

    #[test]
    fn one_boot_takes_two_successive_ranges_not_one_twice() {
        let mut pool = ItemIdRangePool::new([
            range(10_000_001, 20_000_000, 10_000_001),
            range(20_000_001, 30_000_000, 20_000_001),
            range(30_000_001, 40_000_000, 30_000_001),
        ]);
        let (active, spare) = pool.take_active_and_spare().unwrap();
        assert_eq!(active.min, 10_000_001);
        assert_eq!(spare.min, 20_000_001);
        assert_eq!(pool.next_range().map(|next| next.min), Some(30_000_001));
        assert!(pool.is_empty());
    }

    #[test]
    fn a_single_range_is_not_split_into_an_active_and_a_missing_spare() {
        let mut pool = ItemIdRangePool::new([range(10_000_001, 20_000_000, 10_000_001)]);
        assert_eq!(pool.take_active_and_spare(), None);
        assert_eq!(pool.len(), 1, "a refused pair consumes nothing");
    }

    #[test]
    fn usability_matches_the_legacy_remaining_count_check() {
        // Legacy rejects when `dwMax - dwUsableItemIDMin < 10_000`.
        assert!(range(1, 10_000, 0).is_usable());
        assert!(!range(1, 10_000, 1).is_usable());
        assert!(!range(1, 10_000, 2).is_usable());
        // A saturated range must not underflow into an accept.
        assert!(!range(1, 10, 11).is_usable());
    }

    #[test]
    fn the_pool_drops_unusable_ranges_and_keeps_order() {
        let mut pool = ItemIdRangePool::new([
            range(1, 10_000, 0),
            range(1, 10_000, 10_000),
            range(2, 20_000, 3),
        ]);
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.next_range().map(|next| next.min), Some(1));
        assert_eq!(pool.next_range().map(|next| next.min), Some(2));
        assert_eq!(pool.next_range(), None);
    }

    #[test]
    fn legacy_constants_match_the_source() {
        assert_eq!(MAX_ITEM_ID, 4_290_000_000);
        assert_eq!(MINIMUM_RANGE, 10_000_000);
        assert_eq!(MINIMUM_REMAIN_COUNT, 10_000);
    }
}
