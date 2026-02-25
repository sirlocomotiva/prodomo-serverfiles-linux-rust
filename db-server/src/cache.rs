//! Transport-free keyed cache core.
//!
//! The legacy DB server owns several `BaseCache<T>` specializations in
//! `unordered_map` instances. The active source also uses `Get()` as a mutable
//! escape, clears dirty state after an enqueue, and uses zero-valued records
//! as deletion markers. Those choices are not a safe persistence contract.
//! This module provides the smaller shared lifecycle instead: a single-owner
//! keyed map, explicit logical time, typed source results, and a caller-owned
//! persistence seam.
//!
//! This module does not open SQL connections, construct SQL, own the legacy
//! manager maps, schedule timers, open sockets, or mutate player/login state.
//! Packed C++ records must be encoded field by field by a later adapter.
//!
//! `CacheInstant` is a caller-injected logical clock, not a wall-clock or
//! `SystemTime` wrapper. The legacy `BaseCache<T>` uses one signed x86
//! `time_t` expiry for idle and flush checks; this core deliberately exposes
//! independent non-negative intervals and checked `u64` age arithmetic. It is
//! a safety boundary, not a claim of legacy timer parity. The persistence
//! trait is synchronous; a future SQL adapter must arrange executor/thread
//! ownership and must document whether success means enqueue acceptance or
//! durable completion.

use std::collections::HashMap;
use std::hash::Hash;

/// A logical monotonic cache timestamp in seconds.
///
/// Values are supplied by the caller. Age calculations use checked subtraction
/// so clock regression and `u64` overflow never make an entry due by wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct CacheInstant(u64);

impl CacheInstant {
    /// Construct a logical timestamp.
    #[must_use]
    pub const fn new(seconds: u64) -> Self {
        Self(seconds)
    }

    /// Return the logical seconds represented by this timestamp.
    #[must_use]
    pub const fn seconds(self) -> u64 {
        self.0
    }
}

/// Injected logical-time source used after persistence completes.
///
/// The core never reads a wall clock. A real adapter must provide a monotonic
/// source appropriate for its executor; a test may provide a fixed or
/// deterministic sequence instead.
pub trait CacheClock {
    /// Sample the current logical instant.
    fn sample(&mut self) -> CacheInstant;
}

/// Explicit idle and flush intervals, in logical seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachePolicy {
    idle_seconds: u64,
    flush_seconds: u64,
}

impl CachePolicy {
    /// Construct a policy from non-negative logical-second intervals.
    #[must_use]
    pub const fn new(idle_seconds: u64, flush_seconds: u64) -> Self {
        Self {
            idle_seconds,
            flush_seconds,
        }
    }

    /// Return the idle interval.
    #[must_use]
    pub const fn idle_seconds(self) -> u64 {
        self.idle_seconds
    }

    /// Return the flush interval.
    #[must_use]
    pub const fn flush_seconds(self) -> u64 {
        self.flush_seconds
    }
}

/// Whether a `put` leaves a value pending persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutMode {
    /// Mark the value dirty and eligible for a later flush. This mode cannot
    /// replace a pending delete tombstone; use [`PutMode::ResumeLive`] for an
    /// explicit decision to resume a live write.
    Dirty,
    /// Store a value that is already known to be persisted.
    ///
    /// This mode is intended for a new entry or an already-clean entry. A
    /// clean replacement of a dirty entry is rejected; it must not erase
    /// pending work. This differs from the legacy `skipQuery` argument.
    Clean,
    /// Replace any existing state with a value asserted by the caller to be
    /// authoritative and already persisted.
    ///
    /// This is an explicit caller assertion, not a durability check performed
    /// by the cache core.
    VerifiedClean,
    /// Explicitly replace a pending delete tombstone with a dirty live value.
    ///
    /// This preserves the need for a subsequent store operation. Ordinary
    /// dirty puts cannot silently cancel a delete.
    ResumeLive,
}

/// The result of a successful cache insertion or replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePutOutcome {
    /// The key was absent and the value was inserted.
    Inserted,
    /// A live value was replaced.
    Replaced,
    /// An existing clean value was replaced and its persistence baseline was
    /// refreshed.
    ReplacedClean,
    /// A pending tombstone was explicitly replaced by a live operation.
    ReplacedPendingDelete,
}

/// The reason a cache insertion was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePutError {
    /// A clean put would have discarded an existing dirty live value.
    DirtyEntry,
    /// An ordinary put would have discarded a pending delete tombstone.
    PendingDelete,
}

/// A source result for one cache key.
///
/// A source failure is returned as `Err` by [`CachePersistence::load`] and is
/// never represented as `Missing`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRow<V> {
    /// The source returned one value.
    Found(V),
    /// The source completed successfully and found no row.
    Missing,
}

/// The result of resolving a key through the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolve<V> {
    /// A live value was already cached; no source call was made.
    Cached(V),
    /// A source value was loaded and inserted as clean.
    Loaded(V),
    /// The source completed successfully with no row.
    Missing,
    /// A pending delete tombstone blocked a source load. This is distinct from
    /// a source-verified [`SourceRow::Missing`] result.
    PendingDelete,
}

/// The result of one explicit cache flush.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheFlushOutcome {
    /// The key was not present.
    NotPresent,
    /// The live entry was clean, so no persistence call was made.
    NotDirty,
    /// A live value was accepted by the adapter and is now clean at the
    /// adapter-defined boundary.
    StoreAccepted,
    /// A typed delete tombstone was accepted by the adapter and removed.
    DeleteAccepted,
}

/// The result of marking a key for deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    /// A new tombstone was inserted.
    Inserted,
    /// A live value was replaced by a tombstone.
    Replaced,
    /// The key was already a dirty tombstone and was refreshed.
    AlreadyTombstone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EntryState<V> {
    Live(V),
    Tombstone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheEntry<V> {
    state: EntryState<V>,
    dirty: bool,
    last_update: CacheInstant,
    last_flush: CacheInstant,
}

/// A caller-owned source and persistence boundary for cache entries.
///
/// `load`, `store`, and `delete` are deliberately separate. Implementations
/// must return a source error for an unavailable database or malformed row;
/// they must not turn an error into `SourceRow::Missing` or a fabricated value.
/// The core does not know whether a successful `store` means enqueueing or
/// durable completion. A production adapter should choose and document that
/// durability boundary explicitly. It must also validate that a source row's
/// key and value identity agree before returning [`SourceRow::Found`].
pub trait CachePersistence<K, V> {
    /// The source or persistence error type.
    type Error;

    /// Load one source row for `key`.
    ///
    /// # Errors
    ///
    /// Returns the backend error when the source cannot be queried or its
    /// result cannot be decoded. An unavailable source must not be encoded as
    /// [`SourceRow::Missing`].
    fn load(&mut self, key: &K) -> Result<SourceRow<V>, Self::Error>;

    /// Store one live value.
    ///
    /// # Errors
    ///
    /// Returns the backend error when the value was not accepted at the
    /// adapter's documented persistence boundary.
    fn store(&mut self, key: &K, value: &V) -> Result<(), Self::Error>;

    /// Delete one typed tombstone.
    ///
    /// # Errors
    ///
    /// Returns the backend error when the delete was not accepted at the
    /// adapter's documented persistence boundary.
    fn delete(&mut self, key: &K) -> Result<(), Self::Error>;
}

/// A single-owner keyed cache with explicit timing and persistence effects.
///
/// `get` is a non-touching read. Use [`touch`](Self::touch) to refresh idle
/// time and [`update`](Self::update) to mutate a value under the core's dirty
/// tracking. The map owns values, so callers cannot bypass dirty state through
/// a mutable reference.
pub struct CacheCore<K, V>
where
    K: Eq + Hash + Copy,
{
    policy: CachePolicy,
    entries: HashMap<K, CacheEntry<V>>,
}

impl<K, V> CacheCore<K, V>
where
    K: Eq + Hash + Copy,
{
    /// Create an empty cache with an explicit policy.
    #[must_use]
    pub fn new(policy: CachePolicy) -> Self {
        Self {
            policy,
            entries: HashMap::new(),
        }
    }

    /// Return the active timing policy.
    #[must_use]
    pub const fn policy(&self) -> CachePolicy {
        self.policy
    }

    /// Return the number of live and tombstone entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return whether the map has no live or tombstone entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return whether a key has a live value.
    #[must_use]
    pub fn contains_live(&self, key: &K) -> bool {
        matches!(
            self.entries.get(key),
            Some(CacheEntry {
                state: EntryState::Live(_),
                ..
            })
        )
    }

    /// Return whether a key has a pending delete tombstone.
    #[must_use]
    pub fn is_tombstone(&self, key: &K) -> bool {
        matches!(
            self.entries.get(key),
            Some(CacheEntry {
                state: EntryState::Tombstone,
                ..
            })
        )
    }

    /// Return a live value without touching its timestamp.
    #[must_use]
    pub fn get(&self, key: &K) -> Option<&V> {
        match self.entries.get(key) {
            Some(CacheEntry {
                state: EntryState::Live(value),
                ..
            }) => Some(value),
            Some(CacheEntry {
                state: EntryState::Tombstone,
                ..
            })
            | None => None,
        }
    }

    /// Return whether a key has a pending persistence operation.
    #[must_use]
    pub fn is_dirty(&self, key: &K) -> bool {
        self.entries.get(key).is_some_and(|entry| entry.dirty)
    }

    /// Remove a clean entry without issuing a persistence operation.
    ///
    /// Dirty live values and delete tombstones are retained. This narrow
    /// operation is available for later manager lifecycle policies; it never
    /// drops an unflushed mutation.
    pub fn evict_clean(&mut self, key: &K) -> bool {
        let removable = self.entries.get(key).is_some_and(|entry| !entry.dirty);
        if removable {
            self.entries.remove(key);
        }
        removable
    }

    /// Refresh a live entry's idle timestamp without changing its value.
    ///
    /// Returns `false` for a missing key or a pending delete tombstone.
    pub fn touch(&mut self, key: &K, now: CacheInstant) -> bool {
        let Some(entry) = self.entries.get_mut(key) else {
            return false;
        };
        if !matches!(entry.state, EntryState::Live(_)) {
            return false;
        }
        entry.last_update = now;
        true
    }

    /// Insert or replace a value with explicit dirty/clean semantics.
    ///
    /// # Errors
    ///
    /// Returns [`CachePutError::DirtyEntry`] when a normal [`PutMode::Clean`]
    /// operation would replace an existing dirty live entry, or
    /// [`CachePutError::PendingDelete`] when an ordinary put would discard a
    /// tombstone. Use [`PutMode::VerifiedClean`] or [`PutMode::ResumeLive`]
    /// only when the caller has an explicit policy decision.
    pub fn put(
        &mut self,
        key: K,
        value: V,
        now: CacheInstant,
        mode: PutMode,
    ) -> Result<CachePutOutcome, CachePutError> {
        let existing = self.entries.get(&key);
        let existing_is_tombstone =
            existing.is_some_and(|entry| matches!(entry.state, EntryState::Tombstone));
        if matches!(mode, PutMode::Dirty) && existing_is_tombstone {
            return Err(CachePutError::PendingDelete);
        }
        if matches!(mode, PutMode::Clean) && existing.is_some_and(|entry| entry.dirty) {
            return Err(if existing_is_tombstone {
                CachePutError::PendingDelete
            } else {
                CachePutError::DirtyEntry
            });
        }

        let (dirty, last_flush, outcome) = match (mode, existing) {
            (PutMode::Dirty, existing) => (
                true,
                existing.map_or(now, |entry| entry.last_flush),
                if existing.is_some() {
                    CachePutOutcome::Replaced
                } else {
                    CachePutOutcome::Inserted
                },
            ),
            (PutMode::ResumeLive, existing) => (
                true,
                existing.map_or(now, |entry| entry.last_flush),
                if existing_is_tombstone {
                    CachePutOutcome::ReplacedPendingDelete
                } else if existing.is_some() {
                    CachePutOutcome::Replaced
                } else {
                    CachePutOutcome::Inserted
                },
            ),
            (PutMode::Clean | PutMode::VerifiedClean, existing) => (
                false,
                now,
                if existing_is_tombstone {
                    CachePutOutcome::ReplacedPendingDelete
                } else if existing.is_some() {
                    CachePutOutcome::ReplacedClean
                } else {
                    CachePutOutcome::Inserted
                },
            ),
        };
        self.entries.insert(
            key,
            CacheEntry {
                state: EntryState::Live(value),
                dirty,
                last_update: now,
                last_flush,
            },
        );
        Ok(outcome)
    }

    fn insert_clean(&mut self, key: K, value: V, now: CacheInstant) {
        self.entries.insert(
            key,
            CacheEntry {
                state: EntryState::Live(value),
                dirty: false,
                last_update: now,
                last_flush: now,
            },
        );
    }

    /// Mutate a live value while keeping the cache's dirty transition explicit.
    ///
    /// Returns `false` for a missing key or a pending delete tombstone. The
    /// closure cannot obtain a mutable reference to the map itself.
    pub fn update<F>(&mut self, key: &K, now: CacheInstant, update: F) -> bool
    where
        F: FnOnce(&mut V),
    {
        let Some(entry) = self.entries.get_mut(key) else {
            return false;
        };
        let EntryState::Live(value) = &mut entry.state else {
            return false;
        };
        entry.dirty = true;
        entry.last_update = now;
        update(value);
        true
    }

    /// Mark a key with a typed delete tombstone.
    ///
    /// A missing key is inserted as a tombstone and is not fabricated as a
    /// zero-valued live row. Repeated deletion is idempotent at the state
    /// level while retaining the dirty delete intent.
    pub fn mark_deleted(&mut self, key: K, now: CacheInstant) -> DeleteOutcome {
        let outcome = match self.entries.get(&key) {
            None => DeleteOutcome::Inserted,
            Some(CacheEntry {
                state: EntryState::Live(_),
                ..
            }) => DeleteOutcome::Replaced,
            Some(CacheEntry {
                state: EntryState::Tombstone,
                ..
            }) => DeleteOutcome::AlreadyTombstone,
        };
        let last_flush = self.entries.get(&key).map_or(now, |entry| entry.last_flush);
        self.entries.insert(
            key,
            CacheEntry {
                state: EntryState::Tombstone,
                dirty: true,
                last_update: now,
                last_flush,
            },
        );
        outcome
    }

    /// Return whether a live entry is strictly past its idle interval.
    #[must_use]
    pub fn is_idle_due(&self, key: &K, now: CacheInstant) -> bool {
        let Some(entry) = self.entries.get(key) else {
            return false;
        };
        if !matches!(entry.state, EntryState::Live(_)) {
            return false;
        }
        age_past(now, entry.last_update, self.policy.idle_seconds)
    }

    /// Return whether a dirty live or tombstone entry is strictly past its
    /// flush interval.
    #[must_use]
    pub fn is_flush_due(&self, key: &K, now: CacheInstant) -> bool {
        let Some(entry) = self.entries.get(key) else {
            return false;
        };
        entry.dirty && age_past(now, entry.last_flush, self.policy.flush_seconds)
    }

    /// Resolve a key, loading from the source only on a live-cache miss.
    ///
    /// A pending tombstone is never exposed as a live value and does not
    /// trigger a source load. A source error leaves the map unchanged. A
    /// successful `Found` row is inserted clean; `Missing` is never cached.
    ///
    /// # Errors
    ///
    /// Returns the backend error when a cache miss cannot be loaded. No cache
    /// entry is inserted or changed when loading fails.
    pub fn resolve<P>(
        &mut self,
        key: &K,
        now: CacheInstant,
        persistence: &mut P,
    ) -> Result<Resolve<V>, P::Error>
    where
        V: Clone,
        P: CachePersistence<K, V> + ?Sized,
    {
        if let Some(value) = self.get(key) {
            return Ok(Resolve::Cached(value.clone()));
        }
        if self.is_tombstone(key) {
            return Ok(Resolve::PendingDelete);
        }

        match persistence.load(key)? {
            SourceRow::Found(value) => {
                self.insert_clean(*key, value.clone(), now);
                Ok(Resolve::Loaded(value))
            }
            SourceRow::Missing => Ok(Resolve::Missing),
        }
    }

    /// Flush one key through the injected persistence boundary.
    ///
    /// A store accepted at the adapter-defined boundary clears dirty state
    /// and advances only the flush timestamp sampled from `clock` after the
    /// store returns. An accepted delete removes the tombstone. Any source
    /// error leaves the entry, dirty bit, and timestamps unchanged.
    ///
    /// # Errors
    ///
    /// Returns the backend error when a dirty live value or tombstone cannot
    /// be accepted at the adapter-defined boundary. The cache remains dirty and
    /// retryable in that case.
    pub fn flush<P, C>(
        &mut self,
        key: &K,
        persistence: &mut P,
        clock: &mut C,
    ) -> Result<CacheFlushOutcome, P::Error>
    where
        P: CachePersistence<K, V> + ?Sized,
        C: CacheClock + ?Sized,
    {
        let Some(entry) = self.entries.get(key) else {
            return Ok(CacheFlushOutcome::NotPresent);
        };
        if !entry.dirty {
            return Ok(CacheFlushOutcome::NotDirty);
        }

        if matches!(entry.state, EntryState::Tombstone) {
            persistence.delete(key)?;
            self.entries.remove(key);
            return Ok(CacheFlushOutcome::DeleteAccepted);
        }

        let value = match self.entries.get(key) {
            Some(CacheEntry {
                state: EntryState::Live(value),
                ..
            }) => value,
            Some(CacheEntry {
                state: EntryState::Tombstone,
                ..
            })
            | None => return Ok(CacheFlushOutcome::NotPresent),
        };
        persistence.store(key, value)?;

        let Some(entry) = self.entries.get_mut(key) else {
            return Ok(CacheFlushOutcome::NotPresent);
        };
        if !entry.dirty {
            return Ok(CacheFlushOutcome::NotDirty);
        }
        entry.dirty = false;
        entry.last_flush = clock.sample();
        Ok(CacheFlushOutcome::StoreAccepted)
    }

    /// Flush at most `max_flush` due keys in unspecified hash-map order.
    ///
    /// The candidate list is capped by both `max_flush` and the current map
    /// size, so this operation performs no unbounded allocation. The due scan
    /// samples `clock` once; each accepted entry samples it again after its
    /// persistence call. Persistence errors stop the drain; entries accepted
    /// before the error remain clean.
    ///
    /// # Errors
    ///
    /// Returns [`CacheDrainError`] with the partial [`CacheDrainReport`] and
    /// the zero-based candidate index when persistence rejects an entry.
    pub fn drain_due<P, C>(
        &mut self,
        max_flush: usize,
        persistence: &mut P,
        clock: &mut C,
    ) -> Result<CacheDrainReport, CacheDrainError<P::Error>>
    where
        P: CachePersistence<K, V> + ?Sized,
        C: CacheClock + ?Sized,
    {
        let now = clock.sample();
        let mut report = CacheDrainReport::default();
        if max_flush == 0 {
            report.limit_reached = !self.entries.is_empty();
            return Ok(report);
        }
        let capacity = max_flush.min(self.entries.len());
        let mut candidates = Vec::with_capacity(capacity);

        for (key, entry) in &self.entries {
            report.examined += 1;
            if !entry.dirty || !age_past(now, entry.last_flush, self.policy.flush_seconds) {
                report.skipped += 1;
                continue;
            }
            if candidates.len() == max_flush {
                report.limit_reached = true;
                break;
            }
            candidates.push(*key);
        }

        for (index, key) in candidates.into_iter().enumerate() {
            match self.flush(&key, persistence, clock) {
                Ok(CacheFlushOutcome::StoreAccepted | CacheFlushOutcome::DeleteAccepted) => {
                    report.accepted += 1;
                }
                Ok(CacheFlushOutcome::NotPresent | CacheFlushOutcome::NotDirty) => {
                    report.skipped += 1;
                }
                Err(source) => {
                    return Err(CacheDrainError {
                        report,
                        failed_index: index,
                        source,
                    });
                }
            }
        }
        Ok(report)
    }
}

/// Summary of a bounded [`CacheCore::drain_due`] call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheDrainReport {
    /// Number of map entries examined before the limit stopped the scan.
    pub examined: usize,
    /// Number of values or tombstones accepted by the adapter-defined
    /// persistence boundary.
    pub accepted: usize,
    /// Number of clean or not-yet-due entries encountered.
    pub skipped: usize,
    /// Whether the explicit budget stopped the scan before all entries were
    /// examined. With a zero budget, any non-empty map is considered limited
    /// even when no entry is due.
    pub limit_reached: bool,
}

/// A bounded drain failure with the progress made before the failing entry.
/// The source error is an adapter-boundary error, not a fabricated cache result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheDrainError<E> {
    /// Progress made before the persistence failure.
    pub report: CacheDrainReport,
    /// Zero-based index in the selected candidate list that failed.
    pub failed_index: usize,
    /// The underlying persistence error.
    pub source: E,
}

fn age_past(now: CacheInstant, then: CacheInstant, interval: u64) -> bool {
    now.seconds()
        .checked_sub(then.seconds())
        .is_some_and(|age| age > interval)
}

#[cfg(test)]
mod tests {
    use super::{
        CacheClock, CacheCore, CacheDrainError, CacheFlushOutcome, CacheInstant, CachePersistence,
        CachePolicy, CachePutError, CachePutOutcome, DeleteOutcome, PutMode, Resolve, SourceRow,
    };
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::error::Error;
    use std::fmt;

    struct FixedClock(CacheInstant);

    impl CacheClock for FixedClock {
        fn sample(&mut self) -> CacheInstant {
            self.0
        }
    }

    struct SequenceClock {
        samples: Vec<CacheInstant>,
        index: usize,
    }

    impl CacheClock for SequenceClock {
        fn sample(&mut self) -> CacheInstant {
            let sample = self.samples.get(self.index).copied().unwrap_or_else(|| {
                self.samples
                    .last()
                    .copied()
                    .expect("sequence clock has a sample")
            });
            self.index += 1;
            sample
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestError(&'static str);

    impl fmt::Display for TestError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.0)
        }
    }

    impl Error for TestError {}

    #[derive(Debug, Default)]
    struct FakePersistence {
        loads: Cell<usize>,
        stores: Vec<(u32, u32)>,
        deletes: Vec<u32>,
        rows: HashMap<u32, u32>,
        load_error: Option<TestError>,
        store_error: Option<TestError>,
        fail_store_at: Option<usize>,
        delete_error: Option<TestError>,
    }

    impl CachePersistence<u32, u32> for FakePersistence {
        type Error = TestError;

        fn load(&mut self, key: &u32) -> Result<SourceRow<u32>, Self::Error> {
            self.loads.set(self.loads.get() + 1);
            if let Some(error) = &self.load_error {
                return Err(error.clone());
            }
            Ok(self
                .rows
                .get(key)
                .copied()
                .map_or(SourceRow::Missing, SourceRow::Found))
        }

        fn store(&mut self, key: &u32, value: &u32) -> Result<(), Self::Error> {
            if let Some(error) = &self.store_error {
                return Err(error.clone());
            }
            if self.fail_store_at == Some(self.stores.len()) {
                return Err(TestError("selected store failed"));
            }
            self.stores.push((*key, *value));
            Ok(())
        }

        fn delete(&mut self, key: &u32) -> Result<(), Self::Error> {
            if let Some(error) = &self.delete_error {
                return Err(error.clone());
            }
            self.deletes.push(*key);
            Ok(())
        }
    }

    fn core() -> CacheCore<u32, u32> {
        CacheCore::new(CachePolicy::new(10, 5))
    }

    fn at(seconds: u64) -> CacheInstant {
        CacheInstant::new(seconds)
    }

    fn put_value(
        cache: &mut CacheCore<u32, u32>,
        key: u32,
        value: u32,
        now: CacheInstant,
        mode: PutMode,
    ) {
        assert!(cache.put(key, value, now, mode).is_ok());
    }

    fn flush_value(
        cache: &mut CacheCore<u32, u32>,
        key: u32,
        now: CacheInstant,
        persistence: &mut FakePersistence,
    ) -> Result<CacheFlushOutcome, TestError> {
        let mut clock = FixedClock(now);
        cache.flush(&key, persistence, &mut clock)
    }

    fn drain_values(
        cache: &mut CacheCore<u32, u32>,
        now: CacheInstant,
        max_flush: usize,
        persistence: &mut FakePersistence,
    ) -> Result<super::CacheDrainReport, super::CacheDrainError<TestError>> {
        let mut clock = FixedClock(now);
        cache.drain_due(max_flush, persistence, &mut clock)
    }

    #[test]
    fn get_is_non_touching_and_tombstones_are_never_values() {
        let mut cache = core();
        put_value(&mut cache, 7, 42, at(100), PutMode::Clean);
        assert_eq!(cache.get(&7), Some(&42));
        assert!(cache.contains_live(&7));
        assert!(cache.is_idle_due(&7, at(1_000)));
        assert!(cache.touch(&7, at(1_000)));
        assert!(!cache.is_idle_due(&7, at(1_000)));
        assert_eq!(cache.mark_deleted(7, at(101)), DeleteOutcome::Replaced);
        assert_eq!(cache.get(&7), None);
        assert!(cache.is_tombstone(&7));
        assert_eq!(
            cache.mark_deleted(7, at(102)),
            DeleteOutcome::AlreadyTombstone
        );
    }

    #[test]
    fn explicit_put_modes_and_update_control_dirty_state() {
        let mut cache = core();
        put_value(&mut cache, 1, 10, at(0), PutMode::Clean);
        assert!(!cache.is_dirty(&1));
        assert!(cache.update(&1, at(1), |value| *value += 1));
        assert_eq!(cache.get(&1), Some(&11));
        assert!(cache.is_dirty(&1));
        assert_eq!(
            cache.put(1, 12, at(2), PutMode::Clean),
            Err(CachePutError::DirtyEntry)
        );
        assert_eq!(cache.get(&1), Some(&11));
        assert!(cache.is_dirty(&1));
        assert_eq!(
            cache.put(1, 12, at(2), PutMode::VerifiedClean),
            Ok(CachePutOutcome::ReplacedClean)
        );
        assert!(!cache.is_dirty(&1));
        assert!(!cache.update(&99, at(3), |_| unreachable!()));
    }

    #[test]
    fn ordinary_put_cannot_cancel_a_pending_delete() {
        let mut cache = core();
        assert_eq!(cache.mark_deleted(1, at(0)), DeleteOutcome::Inserted);
        assert_eq!(
            cache.put(1, 10, at(1), PutMode::Dirty),
            Err(CachePutError::PendingDelete)
        );
        assert!(cache.is_tombstone(&1));
        assert_eq!(
            cache.put(1, 10, at(1), PutMode::ResumeLive),
            Ok(CachePutOutcome::ReplacedPendingDelete)
        );
        assert!(cache.contains_live(&1));
        assert!(cache.is_dirty(&1));
    }

    #[test]
    fn flush_samples_the_completion_time_after_store() {
        let mut cache = core();
        put_value(&mut cache, 1, 10, at(0), PutMode::Dirty);
        let mut persistence = FakePersistence::default();
        let mut clock = SequenceClock {
            samples: vec![at(6)],
            index: 0,
        };
        assert_eq!(
            cache.flush(&1, &mut persistence, &mut clock),
            Ok(CacheFlushOutcome::StoreAccepted)
        );
        assert!(!cache.is_flush_due(&1, at(6)));
        assert!(cache.update(&1, at(6), |_| {}));
        assert!(cache.is_flush_due(&1, at(12)));
    }

    #[test]
    fn touch_changes_only_idle_time() {
        let mut cache = core();
        put_value(&mut cache, 1, 10, at(0), PutMode::Clean);
        assert!(cache.touch(&1, at(100)));
        assert!(!cache.is_idle_due(&1, at(110)));
        assert!(cache.is_idle_due(&1, at(111)));
        assert!(!cache.touch(&99, at(101)));
    }

    #[test]
    fn idle_and_flush_deadlines_are_independent_and_strict() {
        let mut cache = core();
        put_value(&mut cache, 1, 10, at(0), PutMode::Dirty);
        assert!(!cache.is_flush_due(&1, at(5)));
        assert!(cache.is_flush_due(&1, at(6)));
        assert!(!cache.is_idle_due(&1, at(10)));
        assert!(cache.is_idle_due(&1, at(11)));
        cache.touch(&1, at(20));
        assert!(cache.is_flush_due(&1, at(26)));
        assert!(!cache.is_idle_due(&1, at(30)));
    }

    #[test]
    fn clock_regression_and_underflow_never_make_an_entry_due() {
        let mut cache = core();
        put_value(&mut cache, 1, 10, at(u64::MAX - 1), PutMode::Dirty);
        assert!(!cache.is_flush_due(&1, at(u64::MAX)));
        assert!(!cache.is_idle_due(&1, at(0)));
    }

    #[test]
    fn resolve_found_inserts_clean_and_cached_does_not_reload() {
        let mut cache = core();
        let mut persistence = FakePersistence {
            rows: HashMap::from([(1, 10)]),
            ..FakePersistence::default()
        };

        assert_eq!(
            cache
                .resolve(&1, at(1), &mut persistence)
                .expect("load succeeds"),
            Resolve::Loaded(10)
        );
        assert!(!cache.is_dirty(&1));
        assert_eq!(persistence.loads.get(), 1);
        assert_eq!(
            cache
                .resolve(&1, at(2), &mut persistence)
                .expect("cache hit"),
            Resolve::Cached(10)
        );
        assert_eq!(persistence.loads.get(), 1);
    }

    #[test]
    fn missing_and_source_errors_remain_distinct_and_retryable() {
        let mut cache = core();
        let mut missing = FakePersistence::default();
        assert_eq!(
            cache
                .resolve(&1, at(1), &mut missing)
                .expect("empty source"),
            Resolve::Missing
        );
        assert!(cache.is_empty());
        missing.rows.insert(2, 20);
        assert_eq!(
            cache.resolve(&2, at(2), &mut missing).expect("new row"),
            Resolve::Loaded(20)
        );

        let mut failed = FakePersistence {
            load_error: Some(TestError("source unavailable")),
            ..FakePersistence::default()
        };
        assert_eq!(
            cache.resolve(&3, at(3), &mut failed),
            Err(TestError("source unavailable"))
        );
        assert!(!cache.contains_live(&3));
        failed.load_error = None;
        failed.rows.insert(3, 30);
        assert_eq!(
            cache
                .resolve(&3, at(4), &mut failed)
                .expect("retry succeeds"),
            Resolve::Loaded(30)
        );
    }

    #[test]
    fn delete_uses_a_tombstone_and_success_removes_it() {
        let mut cache = core();
        put_value(&mut cache, 1, 10, at(0), PutMode::Dirty);
        let mut persistence = FakePersistence::default();
        assert_eq!(
            flush_value(&mut cache, 1, at(1), &mut persistence).expect("store succeeds"),
            CacheFlushOutcome::StoreAccepted
        );
        assert_eq!(cache.mark_deleted(1, at(2)), DeleteOutcome::Replaced);
        assert!(cache.get(&1).is_none());
        assert!(matches!(
            cache.resolve(&1, at(2), &mut persistence),
            Ok(Resolve::PendingDelete)
        ));
        assert_eq!(persistence.loads.get(), 0);
        assert_eq!(
            flush_value(&mut cache, 1, at(3), &mut persistence).expect("delete succeeds"),
            CacheFlushOutcome::DeleteAccepted
        );
        assert!(cache.is_empty());
        assert_eq!(persistence.deletes, vec![1]);
    }

    #[test]
    fn failed_store_keeps_value_dirty_and_old_timestamps() {
        let mut cache = core();
        put_value(&mut cache, 1, 10, at(0), PutMode::Dirty);
        let mut persistence = FakePersistence {
            store_error: Some(TestError("write failed")),
            ..FakePersistence::default()
        };
        assert_eq!(
            flush_value(&mut cache, 1, at(20), &mut persistence),
            Err(TestError("write failed"))
        );
        assert!(cache.is_dirty(&1));
        assert!(cache.is_flush_due(&1, at(20)));
        assert_eq!(persistence.stores, Vec::<(u32, u32)>::new());
    }

    #[test]
    fn failed_delete_keeps_tombstone_for_retry() {
        let mut cache = core();
        assert_eq!(cache.mark_deleted(1, at(0)), DeleteOutcome::Inserted);
        let mut persistence = FakePersistence {
            delete_error: Some(TestError("delete failed")),
            ..FakePersistence::default()
        };
        assert_eq!(
            flush_value(&mut cache, 1, at(20), &mut persistence),
            Err(TestError("delete failed"))
        );
        assert!(cache.is_tombstone(&1));
        assert!(cache.is_dirty(&1));
    }

    #[test]
    fn clean_eviction_never_drops_dirty_data() {
        let mut cache = core();
        put_value(&mut cache, 1, 10, at(0), PutMode::Clean);
        assert!(cache.evict_clean(&1));
        assert!(cache.is_empty());
        put_value(&mut cache, 2, 20, at(0), PutMode::Dirty);
        assert!(!cache.evict_clean(&2));
        assert!(cache.is_dirty(&2));
    }

    #[test]
    fn exact_budget_flushes_all_due_entries_without_a_limit() {
        let mut cache = core();
        put_value(&mut cache, 1, 10, at(0), PutMode::Dirty);
        put_value(&mut cache, 2, 20, at(0), PutMode::Dirty);
        let mut persistence = FakePersistence::default();
        let report =
            drain_values(&mut cache, at(6), 2, &mut persistence).expect("exact budget succeeds");
        assert_eq!(report.accepted, 2);
        assert!(!report.limit_reached);
        assert_eq!(persistence.stores.len(), 2);
    }

    #[test]
    fn drain_respects_due_state_and_explicit_limit_without_order_claims() {
        let mut cache = core();
        put_value(&mut cache, 1, 10, at(0), PutMode::Dirty);
        put_value(&mut cache, 2, 20, at(0), PutMode::Dirty);
        put_value(&mut cache, 3, 30, at(0), PutMode::Clean);
        let mut persistence = FakePersistence::default();
        let report = drain_values(&mut cache, at(6), 1, &mut persistence).expect("drain succeeds");
        assert_eq!(report.accepted, 1);
        assert!(report.limit_reached);
        assert_eq!(persistence.stores.len(), 1);
        assert!(cache.is_dirty(&1) || cache.is_dirty(&2));
    }

    #[test]
    fn drain_failure_reports_partial_progress_and_candidate_index() {
        let mut cache = core();
        put_value(&mut cache, 1, 10, at(0), PutMode::Dirty);
        put_value(&mut cache, 2, 20, at(0), PutMode::Dirty);
        let mut persistence = FakePersistence {
            fail_store_at: Some(1),
            ..FakePersistence::default()
        };
        let error = drain_values(&mut cache, at(6), 2, &mut persistence)
            .expect_err("second selected store fails");
        assert!(matches!(
            error,
            CacheDrainError {
                report,
                failed_index: 1,
                source: TestError("selected store failed"),
            } if report.accepted == 1
        ));
        assert_eq!(persistence.stores.len(), 1);
    }

    #[test]
    fn zero_limit_does_not_persist_and_empty_map_is_not_limited() {
        let mut cache = core();
        put_value(&mut cache, 1, 10, at(0), PutMode::Dirty);
        let mut persistence = FakePersistence::default();
        let report =
            drain_values(&mut cache, at(6), 0, &mut persistence).expect("zero limit succeeds");
        assert!(report.limit_reached);
        assert_eq!(report.accepted, 0);
        assert!(persistence.stores.is_empty());

        let mut empty = core();
        let report =
            drain_values(&mut empty, at(6), 0, &mut persistence).expect("empty drain succeeds");
        assert!(!report.limit_reached);
    }
}
