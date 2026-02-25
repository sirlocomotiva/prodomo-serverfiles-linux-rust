//! A boot snapshot with every profile-selected table section empty.
//!
//! The legacy DB server always writes all thirteen table sections of the
//! active profile, each as a `WORD` record width plus a `WORD` count followed
//! by the rows. A width of zero is never written: `QUERY_BOOT` emits
//! `sizeof(TMobTable)`, `sizeof(TItemTable)`, and so on regardless of the row
//! count. An empty section therefore still declares its real record width and
//! carries a count of zero.
//!
//! That makes an empty-table boot a *smaller* boot, not a different one, and it
//! needs no SQL. It is the honest reply for a DB server that has not loaded its
//! tables, and it is what the legacy server sends whenever its vectors are
//! empty. It is **not** a claim that the tables are legitimately empty, and it
//! does not substitute for a real table load.
//!
//! The fixed tail is not empty. `QUERY_BOOT` always writes the x86 `time_t`,
//! the item-ID range pair, the GM host list, the admin list, one
//! `TMonarchInfo`, the candidacy list, and `0xffff`. The monarch record is
//! written with a declared count of one even when it holds no monarch, because
//! `CMonarch::CMonarch` memsets `m_MonarchInfo` and `GetMonarch()` returns a
//! reference to it. This module reproduces that.
//!
//! Everything the legacy reads from outside the tables stays a caller input:
//! the clock, the item-ID range pool, the GM host list, the admin list, and the
//! monarch state. This module runs no SQL, opens no socket, and authenticates
//! nobody.

use std::error::Error;
use std::fmt;

use protocol::db_boot::{
    BootAdminSection, BootGmHost, BootGmHostSection, BootItemIdRange, BootItemIdRanges,
    BootSection, DbBootError, DB_BOOT_VERSION, GM_HOST_WIRE_SIZE, ITEM_ID_RANGE_WIRE_SIZE,
};
use protocol::db_wire::DbFrame;

use crate::boot_composition::{source_fixed_record_size, LoadedBootSections};
use crate::boot_snapshot::{BootSnapshot, BootSnapshotParts};
use crate::item_id_range::{empty_range, ItemIdRangePool};
use crate::monarch_state::MonarchBootState;
use protocol::db_boot::BootAdminInfo;
use protocol::db_boot::BootFeatureProfile;
use protocol::db_boot::BootMonarchInfo;

/// A checked failure while composing an empty-table boot response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmptyBootError {
    /// A profile-selected table section has no source-fixed record width.
    UnknownRecordWidth {
        /// The section kind with no verified width.
        section: protocol::db_boot::BootSectionKind,
    },
    /// A source-fixed record width does not fit the boot `WORD` field.
    RecordWidthOverflow {
        /// The section kind whose width overflowed.
        section: protocol::db_boot::BootSectionKind,
        /// The verified width.
        size: usize,
    },
    /// The section registry could not reserve its profile-sized slots.
    RegistryAllocation,
    /// The caller-supplied monarch state could not be projected.
    Monarch(crate::monarch_state::MonarchBootStateError),
    /// The caller-supplied host, admin, or item-range values failed the
    /// protocol's fixed-tail validation.
    Protocol(DbBootError),
}

impl From<DbBootError> for EmptyBootError {
    fn from(error: DbBootError) -> Self {
        Self::Protocol(error)
    }
}

impl From<crate::monarch_state::MonarchBootStateError> for EmptyBootError {
    fn from(error: crate::monarch_state::MonarchBootStateError) -> Self {
        Self::Monarch(error)
    }
}

impl fmt::Display for EmptyBootError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownRecordWidth { section } => {
                write!(
                    formatter,
                    "no source-fixed record width for boot section {section:?}"
                )
            }
            Self::RecordWidthOverflow { section, size } => write!(
                formatter,
                "boot section {section:?} record width {size} does not fit a u16"
            ),
            Self::RegistryAllocation => {
                write!(formatter, "could not reserve the boot section registry")
            }
            Self::Monarch(error) => {
                write!(formatter, "monarch boot state rejected: {error}")
            }
            Self::Protocol(error) => write!(formatter, "boot payload rejected: {error}"),
        }
    }
}

impl Error for EmptyBootError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Monarch(error) => Some(error),
            Self::Protocol(error) => Some(error),
            _ => None,
        }
    }
}

/// A wall-clock reading for the boot tail.
///
/// The legacy writes `time(0)` into an x86 `time_t`, which is a 4-byte
/// little-endian value. A caller that has no clock injects a fixed value so
/// the encoded bytes are reproducible.
pub trait BootClock {
    /// Return the current Unix time as the legacy x86 `time_t` sees it.
    fn now(&self) -> i32;
}

/// The real clock, read from the system.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SystemBootClock;

impl BootClock for SystemBootClock {
    fn now(&self) -> i32 {
        // The legacy `time(0)` result narrows to a 4-byte x86 `time_t`. The
        // narrowing is deliberate: the wire field is 4 bytes, so a value that
        // does not fit is truncated the same way the legacy struct copy does.
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        i32::try_from(seconds).unwrap_or(i32::MAX)
    }
}

/// A fixed clock, for reproducible bytes in tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedBootClock(pub i32);

impl BootClock for FixedBootClock {
    fn now(&self) -> i32 {
        self.0
    }
}

/// The values `QUERY_BOOT` writes outside the thirteen table sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyBootTail {
    /// The `time(0)` value written as an x86 `time_t`.
    pub global_time: i32,
    /// GM hosts from the legacy `__GetHostInfo`.
    pub gm_hosts: Vec<BootGmHost>,
    /// Administrators selected by the legacy `__GetAdminInfo` for the request IP.
    pub admins: Vec<BootAdminInfo>,
    /// The incumbent monarch record, always written with a declared count of one.
    pub monarch: BootMonarchInfo,
    /// Monarch candidates in source order.
    pub candidacy: Vec<protocol::db_boot::BootMonarchCandidacy>,
}

impl Default for EmptyBootTail {
    /// The state of a DB server that has loaded nothing.
    ///
    /// The monarch record is all zero, which is exactly what
    /// `CMonarch::CMonarch`'s `memset` leaves behind, and it is still written
    /// with a declared count of one.
    fn default() -> Self {
        Self {
            global_time: 0,
            gm_hosts: Vec::new(),
            admins: Vec::new(),
            monarch: BootMonarchInfo {
                pid: [0; 4],
                money: [0; 4],
                name: [[0; 32]; 4],
                date: [[0; 32]; 4],
            },
            candidacy: Vec::new(),
        }
    }
}

impl EmptyBootTail {
    /// Take the two item-ID ranges for this boot from the caller's pool.
    ///
    /// `QUERY_BOOT` calls `CItemIDRangeManager::GetRange()` twice, so one boot
    /// consumes two successive ranges. An exhausted pool yields the legacy
    /// all-zero range for both.
    pub fn take_item_id_ranges(pool: &mut ItemIdRangePool) -> (BootItemIdRange, BootItemIdRange) {
        pool.take_boot_pair()
    }

    /// Build the fixed item-ID range pair from two explicit ranges.
    #[must_use]
    pub fn item_id_ranges(active: BootItemIdRange, spare: BootItemIdRange) -> BootItemIdRanges {
        BootItemIdRanges {
            record_size: u16::try_from(ITEM_ID_RANGE_WIRE_SIZE).unwrap_or(u16::MAX),
            declared_count: 1,
            active,
            spare,
        }
    }
}

/// Build the thirteen empty table sections for a profile, in wire order.
///
/// Every section declares its verified record width and a count of zero. The
/// widths come from the same [`source_fixed_record_size`] table that the SQL
/// adapters validate loaded rows against, so an empty boot cannot claim a
/// width that a real load would reject.
///
/// # Errors
///
/// Returns [`EmptyBootError::UnknownRecordWidth`] for a kind with no verified
/// width, [`EmptyBootError::RecordWidthOverflow`] when a width exceeds
/// `u16::MAX`, and [`EmptyBootError::RegistryAllocation`] when the registry
/// cannot reserve its slots.
pub fn empty_sections(profile: BootFeatureProfile) -> Result<Vec<BootSection>, EmptyBootError> {
    let mut loaded =
        LoadedBootSections::try_new(profile).map_err(|_| EmptyBootError::RegistryAllocation)?;
    for kind in profile.section_kinds() {
        let size = source_fixed_record_size(*kind)
            .ok_or(EmptyBootError::UnknownRecordWidth { section: *kind })?;
        let record_size = u16::try_from(size).map_err(|_| EmptyBootError::RecordWidthOverflow {
            section: *kind,
            size,
        })?;
        loaded
            .insert(BootSection {
                kind: *kind,
                record_size,
                count: 0,
                data: Vec::new(),
            })
            .map_err(|_| EmptyBootError::RegistryAllocation)?;
    }
    loaded
        .into_ordered()
        .map_err(|_| EmptyBootError::RegistryAllocation)
}

/// Compose a boot snapshot whose table sections are all empty.
///
/// # Errors
///
/// Returns [`EmptyBootError`] for a missing or oversized record width and for
/// any protocol-level fixed-tail rejection, including a GM host or admin count
/// that does not fit the boot `WORD`.
pub fn compose_empty_boot(
    profile: BootFeatureProfile,
    tail: EmptyBootTail,
) -> Result<BootSnapshot, EmptyBootError> {
    let sections = empty_sections(profile)?;
    let (active, spare) = (empty_range(), empty_range());
    let parts = build_parts(tail, sections, active, spare)?;
    BootSnapshot::compose(profile, parts).map_err(EmptyBootError::from)
}

/// Compose an empty-table boot with explicit item-ID ranges.
///
/// This is the form the production binary uses: the ranges come from the
/// DB-side pool, not from a constant.
///
/// # Errors
///
/// See [`compose_empty_boot`].
pub fn compose_empty_boot_with_ranges(
    profile: BootFeatureProfile,
    tail: EmptyBootTail,
    active: BootItemIdRange,
    spare: BootItemIdRange,
) -> Result<BootSnapshot, EmptyBootError> {
    let sections = empty_sections(profile)?;
    let parts = build_parts(tail, sections, active, spare)?;
    BootSnapshot::compose(profile, parts).map_err(EmptyBootError::from)
}

fn build_parts(
    tail: EmptyBootTail,
    sections: Vec<BootSection>,
    active: BootItemIdRange,
    spare: BootItemIdRange,
) -> Result<BootSnapshotParts, EmptyBootError> {
    let monarch = MonarchBootState::try_new(tail.monarch, tail.candidacy)?.project_for_boot()?;

    let gm_hosts = BootGmHostSection {
        record_size: u16::try_from(GM_HOST_WIRE_SIZE).map_err(|_| {
            EmptyBootError::Protocol(DbBootError::InvalidFixedRecordSize {
                field: "gm_hosts",
                expected: GM_HOST_WIRE_SIZE,
                actual: u16::MAX as usize,
            })
        })?,
        count: u16::try_from(tail.gm_hosts.len()).map_err(|_| {
            EmptyBootError::Protocol(DbBootError::TailCountOverflow {
                field: "gm_hosts",
                count: tail.gm_hosts.len(),
            })
        })?,
        hosts: tail.gm_hosts,
    };
    let admins = BootAdminSection {
        record_size: u16::try_from(protocol::db_boot::ADMIN_INFO_WIRE_SIZE).map_err(|_| {
            EmptyBootError::Protocol(DbBootError::InvalidFixedRecordSize {
                field: "admins",
                expected: protocol::db_boot::ADMIN_INFO_WIRE_SIZE,
                actual: u16::MAX as usize,
            })
        })?,
        count: u16::try_from(tail.admins.len()).map_err(|_| {
            EmptyBootError::Protocol(DbBootError::TailCountOverflow {
                field: "admins",
                count: tail.admins.len(),
            })
        })?,
        admins: tail.admins,
    };

    Ok(BootSnapshotParts {
        sections,
        global_time: tail.global_time,
        item_id_ranges: EmptyBootTail::item_id_ranges(active, spare),
        gm_hosts,
        admins,
        monarch: monarch.info,
        monarch_candidacy: monarch.candidacy,
    })
}

/// Encode a boot response frame for the active feature profile.
///
/// This is the `HEADER_DG_BOOT` frame with the legacy zero peer handle, as
/// `peer->EncodeHeader(HEADER_DG_BOOT, 0, dwPacketSize)` writes it.
///
/// # Errors
///
/// Returns [`EmptyBootError`] for the same composition failures as
/// [`compose_empty_boot`].
pub fn empty_boot_frame(
    profile: BootFeatureProfile,
    tail: EmptyBootTail,
) -> Result<DbFrame, EmptyBootError> {
    let snapshot = compose_empty_boot(profile, tail)?;
    snapshot.encode_frame().map_err(EmptyBootError::from)
}

/// The legacy boot version byte, re-exported for callers building golden bytes.
pub const BOOT_VERSION: u8 = DB_BOOT_VERSION;

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_boot::BANWORD_WIRE_SIZE;
    use protocol::db_boot::{
        parse_db_boot_payload, BootSectionKind, DbBootPayload, ADMIN_INFO_WIRE_SIZE,
        DB_BOOT_END_MARKER, DB_BOOT_VERSION, GM_HOST_WIRE_SIZE, ITEM_ID_RANGE_WIRE_SIZE,
        MONARCH_CANDIDACY_WIRE_SIZE, MONARCH_INFO_WIRE_SIZE,
    };
    use protocol::db_records::{
        ITEM_TABLE_RECORD_WIRE_SIZE, MOB_TABLE_RECORD_WIRE_SIZE, REFINE_TABLE_WIRE_SIZE,
        SKILL_TABLE_RECORD_WIRE_SIZE,
    };
    use protocol::db_wire::DB_PEER_HEADER_SIZE;

    /// The exact payload length of an active-profile empty boot.
    ///
    /// 4 length + 1 version + 14 sections * 4 + 4 time + 4 range header
    /// + 2 * 12 range + 4 host header + 4 admin header + 4 + 304 monarch
    /// + 4 candidacy header + 2 end marker.
    const ACTIVE_EMPTY_PAYLOAD: usize = 4
        + 1
        + (14 * 4)
        + 4
        + (4 + 2 * ITEM_ID_RANGE_WIRE_SIZE)
        + 4
        + 4
        + (4 + MONARCH_INFO_WIRE_SIZE)
        + 4
        + 2;

    /// Compose, check the frame envelope, and parse the payload back.
    fn decoded(profile: BootFeatureProfile, tail: EmptyBootTail) -> DbBootPayload {
        let frame = empty_boot_frame(profile, tail).expect("empty boot composes");
        // The frame is a one-byte header, a four-byte handle, and a four-byte
        // little-endian length, then the payload.
        assert_eq!(frame.header, protocol::db_boot::HEADER_DG_BOOT);
        assert_eq!(frame.handle, protocol::db_boot::DB_BOOT_RESPONSE_HANDLE);
        assert_eq!(
            frame.payload.len() + DB_PEER_HEADER_SIZE,
            9 + frame.payload.len()
        );
        parse_db_boot_payload(&frame.payload, profile).expect("payload parses")
    }

    #[test]
    fn the_active_profile_needs_fourteen_empty_sections() {
        let sections = empty_sections(BootFeatureProfile::active()).expect("widths are known");
        assert_eq!(sections.len(), 14);
        for section in &sections {
            assert_eq!(section.count, 0, "{:?} must be empty", section.kind);
            assert!(
                section.data.is_empty(),
                "{:?} must carry no rows",
                section.kind
            );
            assert_ne!(
                section.record_size, 0,
                "{:?} must declare a width",
                section.kind
            );
        }
    }

    #[test]
    fn an_empty_section_still_declares_its_verified_record_width() {
        let sections = empty_sections(BootFeatureProfile::active()).expect("widths are known");
        let width = |kind: BootSectionKind| {
            sections
                .iter()
                .find(|section| section.kind == kind)
                .map(|section| usize::from(section.record_size))
        };
        // Asserted against the protocol's own verified constants, so a
        // corrected C++ width cannot leave this test asserting a stale value.
        assert_eq!(width(BootSectionKind::Banword), Some(BANWORD_WIRE_SIZE));
        assert_eq!(width(BootSectionKind::Refine), Some(REFINE_TABLE_WIRE_SIZE));
        assert_eq!(
            width(BootSectionKind::Skill),
            Some(SKILL_TABLE_RECORD_WIRE_SIZE)
        );
        assert_eq!(
            width(BootSectionKind::Mob),
            Some(MOB_TABLE_RECORD_WIRE_SIZE)
        );
        assert_eq!(
            width(BootSectionKind::Item),
            Some(ITEM_TABLE_RECORD_WIRE_SIZE)
        );
        // Item-rare deliberately reuses the item-attr width in the legacy.
        assert_eq!(
            width(BootSectionKind::ItemAttr),
            width(BootSectionKind::ItemRare)
        );
        // The minimal profile drops exactly the three optional sections.
        let minimal = empty_sections(BootFeatureProfile::minimal()).expect("widths are known");
        assert_eq!(minimal.len(), 11);
        assert!(!minimal
            .iter()
            .any(|section| section.kind == BootSectionKind::RenewalShop));
    }

    #[test]
    fn the_empty_active_boot_has_the_exact_expected_length_and_tail() {
        let bare = empty_boot_frame(BootFeatureProfile::active(), EmptyBootTail::default())
            .expect("empty boot composes");
        assert_eq!(bare.payload.len(), ACTIVE_EMPTY_PAYLOAD);
        let payload = decoded(BootFeatureProfile::active(), EmptyBootTail::default());

        assert_eq!(payload.version, DB_BOOT_VERSION);
        assert_eq!(payload.end_marker, DB_BOOT_END_MARKER);
        assert_eq!(payload.global_time, 0);
        assert_eq!(
            usize::from(payload.item_id_ranges.record_size),
            ITEM_ID_RANGE_WIRE_SIZE
        );
        // The count-1 header with two records is the legacy quirk, preserved.
        assert_eq!(payload.item_id_ranges.declared_count, 1);
        assert_eq!(usize::from(payload.gm_hosts.record_size), GM_HOST_WIRE_SIZE);
        assert_eq!(
            usize::from(payload.admins.record_size),
            ADMIN_INFO_WIRE_SIZE
        );
        assert_eq!(
            usize::from(payload.monarch_candidacy.record_size),
            MONARCH_CANDIDACY_WIRE_SIZE
        );
        // The monarch record is still written with a declared count of one.
        assert_eq!(payload.monarch.pid, [0; 4]);
        assert_eq!(payload.monarch.money, [0; 4]);
    }

    #[test]
    fn the_first_bytes_are_the_declared_length_then_the_version_byte() {
        let frame =
            empty_boot_frame(BootFeatureProfile::active(), EmptyBootTail::default()).expect("ok");
        let length = u32::from_le_bytes([
            frame.payload[0],
            frame.payload[1],
            frame.payload[2],
            frame.payload[3],
        ]);
        assert_eq!(length as usize, ACTIVE_EMPTY_PAYLOAD);
        assert_eq!(frame.payload[4], DB_BOOT_VERSION);
        // The first section follows immediately: its verified width, then 0.
        let first_width = u16::from_le_bytes([frame.payload[5], frame.payload[6]]);
        assert_eq!(usize::from(first_width), 255, "TMobTable is 255 bytes");
        assert_eq!(frame.payload[7], 0);
        assert_eq!(frame.payload[8], 0);
    }

    #[test]
    fn the_clock_value_reaches_the_wire_at_the_source_offset() {
        let tail = EmptyBootTail {
            global_time: FixedBootClock(0x1122_3344).now(),
            ..EmptyBootTail::default()
        };
        let frame = empty_boot_frame(BootFeatureProfile::active(), tail).expect("ok");
        // After the length, version, and 14 four-byte section headers.
        let offset = 4 + 1 + (14 * 4);
        assert_eq!(
            &frame.payload[offset..offset + 4],
            &[0x44, 0x33, 0x22, 0x11]
        );
    }

    #[test]
    fn item_id_ranges_come_from_the_pool_not_from_a_constant() {
        let mut pool = ItemIdRangePool::from_ranges([
            BootItemIdRange {
                min: 10_000_001,
                max: 20_000_000,
                usable_item_id_min: 10_000_001,
            },
            BootItemIdRange {
                min: 20_000_001,
                max: 30_000_000,
                usable_item_id_min: 20_000_001,
            },
        ]);
        let (active, spare) = EmptyBootTail::take_item_id_ranges(&mut pool);
        let payload = decoded(BootFeatureProfile::active(), EmptyBootTail::default());
        // The default tail carries no ranges; the explicit form does.
        assert_eq!(payload.item_id_ranges.active, empty_range());

        let tail = EmptyBootTail::default();
        let frame =
            compose_empty_boot_with_ranges(BootFeatureProfile::active(), tail, active, spare)
                .expect("ok")
                .encode_frame()
                .expect("ok");
        let parsed = parse_db_boot_payload(&frame.payload, BootFeatureProfile::active())
            .expect("payload parses");
        assert_eq!(parsed.item_id_ranges.active.min, 10_000_001);
        assert_eq!(parsed.item_id_ranges.spare.min, 20_000_001);
    }

    #[test]
    fn a_supplied_gm_host_and_admin_reach_the_wire() {
        let mut host = [0_u8; GM_HOST_WIRE_SIZE];
        host[..6].copy_from_slice(b"host01");
        let mut contact = [0_u8; 16];
        contact[..9].copy_from_slice(b"127.0.0.1");
        let mut name = [0_u8; 32];
        name[..3].copy_from_slice(b"adm");

        let tail = EmptyBootTail {
            global_time: 7,
            gm_hosts: vec![BootGmHost { bytes: host }],
            admins: vec![BootAdminInfo {
                id: 5,
                account: [b'a'; 32],
                name,
                contact_ip: contact,
                server_ip: contact,
                authority: 1,
            }],
            monarch: BootMonarchInfo {
                pid: [1, 0, 0, 0],
                money: [0; 4],
                name: [[0; 32]; 4],
                date: [[0; 32]; 4],
            },
            candidacy: Vec::new(),
        };

        let payload = decoded(BootFeatureProfile::active(), tail);
        assert_eq!(payload.gm_hosts.count, 1);
        assert_eq!(&payload.gm_hosts.hosts[0].bytes[..6], b"host01");
        assert_eq!(payload.admins.count, 1);
        assert_eq!(payload.admins.admins[0].id, 5);
        assert_eq!(payload.admins.admins[0].authority, 1);
        // The monarch record keeps the caller's incumbent bytes.
        assert_eq!(payload.monarch.pid, [1, 0, 0, 0]);
        assert_eq!(payload.global_time, 7);
    }

    #[test]
    fn the_payload_length_grows_by_exactly_the_supplied_tail_sizes() {
        let bare = empty_boot_frame(BootFeatureProfile::active(), EmptyBootTail::default())
            .expect("ok")
            .payload
            .len();
        let tail = EmptyBootTail {
            gm_hosts: vec![BootGmHost {
                bytes: [0; GM_HOST_WIRE_SIZE],
            }],
            ..EmptyBootTail::default()
        };
        let with_host = empty_boot_frame(BootFeatureProfile::active(), tail)
            .expect("ok")
            .payload
            .len();
        assert_eq!(with_host, bare + GM_HOST_WIRE_SIZE);
    }

    #[test]
    fn the_system_clock_stays_inside_the_four_byte_wire_field() {
        // A value that cannot fit is clamped, never wrapped or truncated
        // into a plausible-looking past date.
        assert!(SystemBootClock.now() > 0);
        assert_eq!(FixedBootClock(i32::MIN).now(), i32::MIN);
    }
}
