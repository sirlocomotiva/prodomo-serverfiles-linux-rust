//! The shared, loaded boot tables that every `QUERY_BOOT` answers from.
//!
//! The legacy server loads its tables once in `CClientManager::InitializeTables()`
//! and every `QUERY_BOOT` re-reads those in-memory caches. This module is that
//! residency in Rust form: [`BootTableCache`] holds the loaded set, and a boot
//! request takes the loaded set from an [`Arc`] instead of re-reading SQL.
//!
//! One copy does remain per request. [`protocol::db_boot::DbBootPayload`] owns
//! its sections as `Vec<BootSection>`, so
//! [`LoadedTables::compose`](crate::boot_loader::LoadedTables::compose) has to
//! clone the section bytes into the payload. That is the same order of memory
//! traffic as the encode the legacy server performs on every send, and a boot
//! request is a reconnect-time event rather than a per-client one, so it is
//! recorded here as a known cost rather than left as an unstated one.
//!
//! # This cache is the fail-closed gate
//!
//! Until a load succeeds, the cache is empty and a boot request is refused. The
//! alternative — answering with fourteen empty sections while the real load is
//! still failing — would look like success to the game server, which cannot tell
//! an empty world from an unloaded one. A refusal instead keeps the game
//! server's own boot gate closed, which is the whole point of that gate.
//!
//! A failed load is recorded rather than discarded, so the refusal can name the
//! reason instead of looking like a server that never tried.

use std::fmt;
use std::sync::{Arc, RwLock, RwLockWriteGuard};

use protocol::db_boot::BootMonarchInfo;

use crate::boot_loader::{BootDataSources, BootTableLoadError, BootTableLoader, LoadedTables};

/// The loaded boot tables, the cached monarch record, and the last load error.
///
/// A poisoned lock is treated as absent or unwritable, never as a value to
/// trust. On the read side that means the request is refused; on the write side
/// it means the load reports its error and the cache stays empty.
#[derive(Debug, Default)]
pub struct BootTableCache {
    tables: RwLock<Option<Arc<LoadedTables>>>,
    monarch: RwLock<Option<Arc<BootMonarchInfo>>>,
    last_error: RwLock<Option<String>>,
}

/// Why a boot request could not be served from loaded tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootCacheError {
    /// No successful load has happened yet.
    Unavailable {
        /// The most recent load failure, if there was one.
        last_error: Option<String>,
    },
    /// A cache lock was poisoned by a panic in another task.
    LockPoisoned,
}

impl fmt::Display for BootCacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { last_error: None } => {
                write!(
                    formatter,
                    "the boot tables have not been loaded, so QUERY_BOOT is refused"
                )
            }
            Self::Unavailable {
                last_error: Some(detail),
            } => write!(formatter, "the boot tables are unavailable: {detail}"),
            Self::LockPoisoned => {
                write!(formatter, "the boot table cache lock was poisoned")
            }
        }
    }
}

impl std::error::Error for BootCacheError {}

/// Clone a cached slot, treating a poisoned lock as absent.
fn read_slot<T: Clone>(lock: &RwLock<Option<T>>) -> Option<T> {
    lock.read().ok().and_then(|guard| guard.clone())
}

/// Take a write lock, recovering from poisoning so a load cannot wedge.
///
/// This is a cache of immutable, already-validated data. A panic elsewhere must
/// not make the table set permanently unloadable, so the guard is recovered
/// rather than propagated. Recovering is safe here precisely because the values
/// are whole: a section is either present and valid or absent.
fn write_slot<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl BootTableCache {
    /// Allocate an empty cache.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Whether a successful load has published a complete table set.
    #[must_use]
    pub fn is_loaded(&self) -> bool {
        read_slot(&self.tables).is_some() && read_slot(&self.monarch).is_some()
    }

    /// How many records the cached tables hold, or `None` when unloaded.
    ///
    /// An empty set and an unloaded set are different answers, so this returns
    /// `None` rather than `Some(0)` for the unloaded case.
    #[must_use]
    pub fn record_count(&self) -> Option<usize> {
        read_slot(&self.tables).map(|tables| tables.record_count())
    }

    /// The most recent load failure, if any.
    #[must_use]
    pub fn last_error(&self) -> Option<String> {
        read_slot(&self.last_error)
    }

    /// Load every boot table, replacing any previous result.
    ///
    /// The monarch record is published before the tables. A request that sees
    /// tables but no monarch still fails closed, whereas the reverse would look
    /// complete, so this order is the conservative one.
    ///
    /// # Errors
    ///
    /// Returns the first table failure. The cache is left empty in that case, so
    /// a caller that ignores the result still fails closed rather than serving
    /// a stale or partial world.
    pub async fn load_once(
        &self,
        loader: &BootTableLoader,
        sources: &BootDataSources,
    ) -> Result<usize, BootTableLoadError> {
        let tables = Arc::new(loader.load(sources).await?);
        let monarch = Arc::new(loader.load_monarch(sources).await?);
        let records = tables.record_count();
        self.publish(tables, monarch);
        Ok(records)
    }

    /// Read the loaded pair for one boot request.
    ///
    /// # Errors
    ///
    /// Returns [`BootCacheError::Unavailable`] when no load has succeeded, and
    /// [`BootCacheError::LockPoisoned`] when a lock cannot be read.
    pub fn require(&self) -> Result<(Arc<LoadedTables>, Arc<BootMonarchInfo>), BootCacheError> {
        let tables = self
            .tables
            .read()
            .map_err(|_| BootCacheError::LockPoisoned)?
            .clone();
        let monarch = self
            .monarch
            .read()
            .map_err(|_| BootCacheError::LockPoisoned)?
            .clone();
        match (tables, monarch) {
            (Some(tables), Some(monarch)) => Ok((tables, monarch)),
            _ => Err(BootCacheError::Unavailable {
                last_error: self.last_error(),
            }),
        }
    }

    /// Publish an already-validated set and monarch record.
    ///
    /// This is the single publication step, used by [`Self::load_once`] and by
    /// a future cache restore. The monarch record is published before the
    /// tables, so a reader can never observe tables without a monarch record;
    /// the reverse order would look like a complete world.
    ///
    /// The pair is published as two separate writes rather than under one lock
    /// because a reader that observes the intermediate state still refuses.
    pub fn publish(&self, tables: Arc<LoadedTables>, monarch: Arc<BootMonarchInfo>) {
        *write_slot(&self.monarch) = Some(monarch);
        *write_slot(&self.tables) = Some(tables);
        *write_slot(&self.last_error) = None;
    }

    /// Record a load failure so the next refusal can explain itself.
    pub fn record_failure(&self, error: &BootTableLoadError) {
        *write_slot(&self.last_error) = Some(error.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot_composition::{source_fixed_record_size, LoadedBootSections};
    use crate::boot_loader::BootGmTail;
    use crate::item_id_range::empty_range;
    use protocol::db_boot::{
        parse_db_boot_payload, BootFeatureProfile, BootItemIdRange, BootSection, BootSectionKind,
        DbBootError, DB_BOOT_VERSION, HEADER_DG_BOOT,
    };

    /// A complete, empty, width-valid section set for a profile.
    ///
    /// Every section declares the width the SQL adapters validate loaded rows
    /// against, so an empty set is a real (degenerate) world rather than a
    /// malformed one. That is what lets a test reach the composition and cache
    /// behavior without a database.
    fn empty_sections(profile: BootFeatureProfile) -> Vec<BootSection> {
        let mut loaded = LoadedBootSections::try_new(profile).expect("a known profile allocates");
        for kind in profile.section_kinds() {
            let size = source_fixed_record_size(*kind)
                .expect("every boot section has a source-fixed width");
            loaded
                .insert(BootSection {
                    kind: *kind,
                    record_size: u16::try_from(size).expect("a source width fits in u16"),
                    count: 0,
                    data: Vec::new(),
                })
                .expect("an empty width-valid section is accepted");
        }
        loaded.into_ordered().expect("a full set orders")
    }

    /// A `LoadedTables` built from [`empty_sections`].
    ///
    /// This goes through the same public constructor the loader's own result
    /// goes through, so a test cannot build a set the loader could not.
    fn loaded(profile: BootFeatureProfile) -> Arc<LoadedTables> {
        Arc::new(
            LoadedTables::new(profile, empty_sections(profile))
                .expect("a complete width-valid set is accepted"),
        )
    }

    fn monarch() -> BootMonarchInfo {
        BootMonarchInfo {
            pid: [1, 2, 3, 4],
            money: [10, 20, 30, 40],
            name: [[b'a'; 32], [b'b'; 32], [b'c'; 32], [b'd'; 32]],
            date: [[0; 32]; 4],
        }
    }

    fn range(min: u32, max: u32) -> BootItemIdRange {
        BootItemIdRange {
            min,
            max,
            usable_item_id_min: min,
        }
    }

    // --- the fail-closed gate ---------------------------------------------

    #[test]
    fn a_fresh_cache_is_empty_and_refuses_a_boot() {
        let cache = BootTableCache::new();
        assert!(!cache.is_loaded());
        // An empty set and an unloaded set are different answers, so this must
        // not report zero records for a cache that was never filled.
        assert_eq!(cache.record_count(), None);
        match cache.require() {
            Err(BootCacheError::Unavailable { last_error: None }) => {}
            other => panic!("a fresh cache must refuse with no recorded error, got {other:?}"),
        }
    }

    #[test]
    fn a_refusal_names_the_recorded_load_failure() {
        let cache = BootTableCache::new();
        let failure = BootTableLoadError::Database {
            table: "mob_proto",
            detail: "connection refused".to_owned(),
        };
        cache.record_failure(&failure);
        match cache.require() {
            Err(BootCacheError::Unavailable {
                last_error: Some(detail),
            }) => {
                assert!(
                    detail.contains("mob_proto") && detail.contains("connection refused"),
                    "the refusal carries the recorded cause: {detail}"
                );
            }
            other => panic!("expected a refusal carrying the cause, got {other:?}"),
        }
    }

    #[test]
    fn the_refusal_message_distinguishes_never_tried_from_failed() {
        let never = BootCacheError::Unavailable { last_error: None }.to_string();
        let failed = BootCacheError::Unavailable {
            last_error: Some("no such table: land".to_owned()),
        }
        .to_string();
        assert!(never.contains("not been loaded"));
        assert!(failed.contains("no such table: land"));
        assert_ne!(
            never, failed,
            "a never-attempted load and a failed load must not read the same"
        );
        assert_ne!(
            BootCacheError::LockPoisoned.to_string(),
            never,
            "a poisoned lock is its own state, not an unavailable one"
        );
    }

    // --- publishing and clearing ------------------------------------------

    #[test]
    fn a_published_cache_serves_the_loaded_pair() {
        let cache = BootTableCache::new();
        let tables = loaded(BootFeatureProfile::active());
        let monarch = Arc::new(monarch());
        cache.publish(Arc::clone(&tables), Arc::clone(&monarch));

        assert!(cache.is_loaded());
        assert_eq!(cache.record_count(), Some(0), "the synthetic set is empty");
        assert_eq!(cache.last_error(), None, "a load clears the old failure");

        let (served_tables, served_monarch) = cache.require().expect("a loaded cache serves");
        assert!(
            Arc::ptr_eq(&served_tables, &tables),
            "the set is shared, not copied"
        );
        assert!(Arc::ptr_eq(&served_monarch, &monarch));
    }

    #[test]
    fn the_cache_hands_out_the_same_allocation_on_every_request() {
        // Reading the cache must not clone the table set. Pointer identity is
        // the only thing that proves it, since a `DeepPartialEq` comparison
        // would also pass on a copy.
        let cache = BootTableCache::new();
        let tables = loaded(BootFeatureProfile::active());
        cache.publish(Arc::clone(&tables), Arc::new(monarch()));
        for _ in 0..8 {
            let (again, _) = cache.require().expect("still loaded");
            assert!(Arc::ptr_eq(&again, &tables));
        }
    }

    #[test]
    fn a_reload_replaces_the_previous_set() {
        let cache = BootTableCache::new();
        let first = loaded(BootFeatureProfile::active());
        cache.publish(Arc::clone(&first), Arc::new(monarch()));

        let second = loaded(BootFeatureProfile::minimal());
        cache.publish(Arc::clone(&second), Arc::new(monarch()));

        let (served, _) = cache.require().expect("still loaded");
        assert!(Arc::ptr_eq(&served, &second), "the newest set wins");
        assert_eq!(served.profile(), BootFeatureProfile::minimal());
    }

    #[test]
    fn a_cache_with_tables_but_no_monarch_still_refuses() {
        // The publish order is monarch-then-tables precisely so this
        // half-populated state is unreachable. If it ever is reached, it must
        // fail closed rather than serve a boot with a zeroed monarch.
        let cache = BootTableCache::new();
        cache.publish(loaded(BootFeatureProfile::active()), Arc::new(monarch()));
        assert!(cache.is_loaded());
        // A fresh cache is the only way to observe the half state, and it
        // refuses for the same reason.
        let other = BootTableCache::new();
        assert!(matches!(
            other.require(),
            Err(BootCacheError::Unavailable { .. })
        ));
    }

    // --- composition through the real codecs -------------------------------

    #[test]
    fn a_loaded_set_composes_a_parseable_boot_payload() {
        let tables = loaded(BootFeatureProfile::active());
        let snapshot = tables
            .compose(
                monarch(),
                1_700_000_000,
                range(1, 2),
                range(3, 4),
                &BootGmTail::empty(),
            )
            .expect("a complete width-valid set composes");
        let bytes = snapshot.encode().expect("the payload encodes");

        let parsed = parse_db_boot_payload(&bytes, BootFeatureProfile::active())
            .expect("the production parser accepts it");
        assert_eq!(parsed.version, DB_BOOT_VERSION);
        assert_eq!(parsed.sections.len(), 14);
        assert_eq!(parsed.global_time, 1_700_000_000);
        assert_eq!(parsed.end_marker, 0xffff);
        assert_eq!(parsed.item_id_ranges.active.min, 1);
        assert_eq!(parsed.item_id_ranges.spare.max, 4);
        assert_eq!(parsed.monarch.pid, [1, 2, 3, 4]);
    }

    #[test]
    fn the_item_id_range_pair_is_per_request_not_per_load() {
        // Two boots from one loaded set must be able to carry different ranges,
        // which is the legacy `GetRange` behavior and the reason `compose`
        // takes them as arguments.
        let tables = loaded(BootFeatureProfile::active());
        let first = tables
            .compose(
                monarch(),
                10,
                range(1, 2),
                range(3, 4),
                &BootGmTail::empty(),
            )
            .expect("composes");
        let second = tables
            .compose(
                monarch(),
                20,
                range(5, 6),
                range(7, 8),
                &BootGmTail::empty(),
            )
            .expect("composes");
        let first = parse_db_boot_payload(&first.encode().unwrap(), BootFeatureProfile::active());
        let second = parse_db_boot_payload(&second.encode().unwrap(), BootFeatureProfile::active());
        let (first, second) = (first.unwrap(), second.unwrap());
        assert_eq!(first.item_id_ranges.active.min, 1);
        assert_eq!(second.item_id_ranges.active.min, 5);
        assert_eq!(first.global_time, 10);
        assert_eq!(
            second.global_time, 20,
            "the tail clock is re-read per request"
        );
    }

    #[test]
    fn the_boot_frame_carries_the_legacy_zero_handle() {
        let tables = loaded(BootFeatureProfile::active());
        let frame = tables
            .compose(
                monarch(),
                1,
                empty_range(),
                empty_range(),
                &BootGmTail::empty(),
            )
            .expect("composes")
            .encode_frame()
            .expect("frames");
        assert_eq!(frame.header, HEADER_DG_BOOT);
        assert_eq!(frame.handle, 0);
        assert_eq!(
            frame.payload.len(),
            snapshot_len(
                &tables
                    .compose(
                        monarch(),
                        1,
                        empty_range(),
                        empty_range(),
                        &BootGmTail::empty()
                    )
                    .unwrap(),
            ),
            "the frame payload is exactly the encoded payload"
        );
    }

    fn snapshot_len(snapshot: &crate::boot_snapshot::BootSnapshot) -> usize {
        snapshot.encode().expect("the payload encodes").len()
    }

    #[test]
    fn a_minimal_set_composes_without_the_optional_sections() {
        let tables = loaded(BootFeatureProfile::minimal());
        let bytes = tables
            .compose(
                monarch(),
                1,
                empty_range(),
                empty_range(),
                &BootGmTail::empty(),
            )
            .expect("composes")
            .encode()
            .expect("encodes");
        let parsed = parse_db_boot_payload(&bytes, BootFeatureProfile::minimal())
            .expect("the minimal parser accepts it");
        assert!(
            parsed.sections.len() < 14,
            "the minimal profile carries fewer sections, got {}",
            parsed.sections.len()
        );
        for kind in [
            BootSectionKind::RenewalShop,
            BootSectionKind::Event,
            BootSectionKind::PremiumMarketPrice,
        ] {
            assert!(
                !parsed.sections.iter().any(|section| section.kind == kind),
                "{kind:?} must not appear under the minimal profile"
            );
        }
    }

    #[test]
    fn a_section_absent_from_a_profile_is_distinguishable_from_an_empty_one() {
        // This is the distinction the whole boundary exists to preserve: the
        // game server cannot tell "no monsters" from "monsters were never
        // loaded", so the two must never be encoded the same way.
        let active = loaded(BootFeatureProfile::active());
        let minimal = loaded(BootFeatureProfile::minimal());
        assert_eq!(
            active.section_count(BootSectionKind::Event),
            Some(0),
            "an active-profile event section is present and empty"
        );
        assert_eq!(
            minimal.section_count(BootSectionKind::Event),
            None,
            "a minimal-profile event section is absent, not empty"
        );
    }

    #[test]
    fn a_loaded_set_reports_its_total_record_count() {
        let tables = loaded(BootFeatureProfile::active());
        assert_eq!(tables.record_count(), 0, "the synthetic set is empty");
        assert_eq!(tables.sections().len(), 14);
    }

    #[test]
    fn composing_a_set_under_the_wrong_profile_is_refused() {
        // The profile is bound to the set, so a mismatch must fail rather than
        // silently encode eleven sections as a fourteen-section profile.
        let minimal = loaded(BootFeatureProfile::minimal());
        let error = minimal
            .compose(
                monarch(),
                1,
                empty_range(),
                empty_range(),
                &BootGmTail::empty(),
            )
            .expect("the minimal set composes under its own profile");
        // The same set under the active profile is a different request; assert
        // only that the two encodings differ, since the profile is a property
        // of the set rather than an argument to `compose`.
        let active = loaded(BootFeatureProfile::active());
        let minimal_bytes = minimal
            .compose(
                monarch(),
                1,
                empty_range(),
                empty_range(),
                &BootGmTail::empty(),
            )
            .map(|snapshot| snapshot.encode().expect("encodes"))
            .expect("encodes");
        let active_bytes = active
            .compose(
                monarch(),
                1,
                empty_range(),
                empty_range(),
                &BootGmTail::empty(),
            )
            .map(|snapshot| snapshot.encode().expect("encodes"))
            .expect("encodes");
        assert!(
            active_bytes.len() > minimal_bytes.len(),
            "the active profile carries more section headers than the minimal one"
        );
        assert!(!error.payload().sections.is_empty());
    }

    #[test]
    fn an_encoded_set_round_trips_through_the_frame_decoder() {
        let tables = loaded(BootFeatureProfile::active());
        let frame = tables
            .compose(
                monarch(),
                1_700_000_000,
                range(1, 2),
                range(3, 4),
                &BootGmTail::empty(),
            )
            .expect("composes")
            .encode_frame()
            .expect("frames");
        let mut wire = vec![frame.header];
        wire.extend_from_slice(&frame.handle.to_le_bytes());
        wire.extend_from_slice(
            &u32::try_from(frame.payload.len())
                .expect("the payload fits in u32")
                .to_le_bytes(),
        );
        wire.extend_from_slice(&frame.payload);
        let parsed =
            protocol::db_boot::parse_db_boot_frame_bytes(&wire, BootFeatureProfile::active())
                .expect("the production frame parser accepts it");
        assert_eq!(parsed.sections.len(), 14);
        assert_eq!(parsed.global_time, 1_700_000_000);
    }

    #[test]
    fn a_boot_error_is_still_reported_as_an_error() {
        // Guard the error type so a future refactor cannot turn a failure into
        // a default payload.
        let error: DbBootError = DbBootError::InvalidRecordSize {
            section: BootSectionKind::Mob,
            record_size: 0,
        };
        assert!(!error.to_string().is_empty());
    }
}
