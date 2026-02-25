//! SQL-free composition of a caller-resolved legacy DB boot snapshot.
//!
//! This module is deliberately a composition boundary, not a table loader.
//! The legacy sender in `server/server/db/ClientManager.cpp:436-642` obtains
//! table rows, GM hosts, administrators, item ranges, and monarch state before
//! it writes the response.  A caller of this module must supply those values
//! after its own source-specific loading and policy decisions.  No SQL query,
//! connection, cache mutation, item-ID allocation, or fallback row is created
//! here.  In particular, hosts and administrators are resolved for each boot
//! request's `szIP`; a production caller must build the fixed tail per
//! response rather than reuse a stale request-IP lookup.
//!
//! The source writes the response in this order:
//!
//! 1. the four-byte payload length and version byte;
//! 2. profile-selected table sections, each with a `u16` record width and
//!    `u16` count;
//! 3. the x86 `time_t`, the count-1/two-record item-range quirk, GM hosts,
//!    administrators, monarch data, candidacy data, and `0xffff`.
//!
//! The profile is explicit because the optional renewal-shop, event, and
//! market-price sections have no identifiers on the wire.  Ordinary table
//! records stay opaque: this module checks their declared width/count and
//! exact byte length, but never guesses a C++ record layout or decodes a row.
//! Source-fixed values that are not table rows (the version and end marker)
//! are set to the legacy constants.  Every other payload value is supplied by
//! the caller.  Typed table consumers (for example banword, refine, event, or
//! market-price decoders) remain responsible for applying their source-fixed
//! row widths; this boundary only preserves their already-loaded bytes.

use protocol::db_boot::{
    BootAdminSection, BootFeatureProfile, BootGmHostSection, BootItemIdRanges,
    BootMonarchCandidacySection, BootMonarchInfo, BootSection, DbBootError, DbBootPayload,
    ADMIN_INFO_WIRE_SIZE, DB_BOOT_END_MARKER, DB_BOOT_VERSION, DEFAULT_MAX_DB_BOOT_PAYLOAD_SIZE,
    GM_HOST_WIRE_SIZE, ITEM_ID_RANGE_WIRE_SIZE, MONARCH_CANDIDACY_WIRE_SIZE,
    MONARCH_INFO_WIRE_SIZE, X86_TIME_T_WIRE_SIZE,
};
use protocol::db_wire::DbFrame;

/// Caller-resolved values for one legacy version-6 boot payload.
///
/// The fields mirror the public [`DbBootPayload`] value.  `sections` must be
/// supplied in the exact order selected by the profile passed to
/// [`BootSnapshot::compose`].  The constructor does not create a section,
/// choose a profile, query a database, or fill a missing table with zero rows.
///
/// `version` and `end_marker` are intentionally not fields: the legacy
/// sender always writes [`DB_BOOT_VERSION`] and [`DB_BOOT_END_MARKER`].  The
/// remaining fixed-tail values, including monarch state and both item ranges,
/// remain explicit caller inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootSnapshotParts {
    /// Profile-ordered table sections with opaque record bytes.
    pub sections: Vec<BootSection>,
    /// The source `time(0)` value represented by the active x86 `time_t`.
    pub global_time: i32,
    /// The active and spare item-ID ranges written under the count-1 header.
    pub item_id_ranges: BootItemIdRanges,
    /// GM host records selected by the caller's host source.
    pub gm_hosts: BootGmHostSection,
    /// Administrator records selected by the caller's request-IP policy.
    pub admins: BootAdminSection,
    /// The one monarch record emitted by the legacy sender.
    pub monarch: BootMonarchInfo,
    /// Monarch candidacy records in source order.
    pub monarch_candidacy: BootMonarchCandidacySection,
}

impl BootSnapshotParts {
    /// Construct caller-resolved boot parts without changing any row bytes.
    #[must_use]
    pub fn new(
        sections: Vec<BootSection>,
        global_time: i32,
        item_id_ranges: BootItemIdRanges,
        gm_hosts: BootGmHostSection,
        admins: BootAdminSection,
        monarch: BootMonarchInfo,
        monarch_candidacy: BootMonarchCandidacySection,
    ) -> Self {
        Self {
            sections,
            global_time,
            item_id_ranges,
            gm_hosts,
            admins,
            monarch,
            monarch_candidacy,
        }
    }
}

/// An immutable, profile-bound, SQL-free boot snapshot.
///
/// Construction validates the profile-selected section order, opaque section
/// metadata, fixed-tail metadata, exact payload length, and the configured
/// allocation limit through the existing [`DbBootPayload`] encoder.  The
/// snapshot then reuses that same encoder for response framing.  It never
/// changes the profile after construction, so a caller cannot accidentally
/// encode the same rows with a different optional-section layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootSnapshot {
    profile: BootFeatureProfile,
    max_payload_size: usize,
    payload: DbBootPayload,
}

impl BootSnapshot {
    /// Compose a snapshot using the protocol's conservative default limit.
    ///
    /// The caller must provide every profile-selected section and every fixed
    /// tail value.  An absent source result is an input error; it is not
    /// converted into an empty table or a zero-filled runtime record.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError`] for a profile/section mismatch, malformed
    /// opaque section metadata, malformed fixed-tail metadata, checked-size
    /// overflow, a payload above the default limit, or an encoder allocation
    /// failure.
    pub fn compose(
        profile: BootFeatureProfile,
        parts: BootSnapshotParts,
    ) -> Result<Self, DbBootError> {
        Self::compose_with_limit(profile, parts, DEFAULT_MAX_DB_BOOT_PAYLOAD_SIZE)
    }

    /// Compose a snapshot with an explicit complete-payload allocation limit.
    ///
    /// The limit is checked after exact, checked size calculation and before
    /// the protocol encoder allocates its output.  All source values remain
    /// caller-owned; this function only validates and composes them.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError`] for any composition or encoding failure, or
    /// [`DbBootError::EncodePayloadTooLarge`] when the computed payload is
    /// above `max_payload_size`.
    pub fn compose_with_limit(
        profile: BootFeatureProfile,
        parts: BootSnapshotParts,
        max_payload_size: usize,
    ) -> Result<Self, DbBootError> {
        validate_profile_sections(profile, &parts.sections)?;

        let BootSnapshotParts {
            sections,
            global_time,
            item_id_ranges,
            gm_hosts,
            admins,
            monarch,
            monarch_candidacy,
        } = parts;
        let length = payload_length(
            &sections,
            gm_hosts.hosts.len(),
            admins.admins.len(),
            monarch_candidacy.candidates.len(),
        )?;
        if length > max_payload_size {
            return Err(DbBootError::EncodePayloadTooLarge {
                length,
                maximum: max_payload_size,
            });
        }
        let packet_size =
            u32::try_from(length).map_err(|_| DbBootError::PayloadSizeOverflow { length })?;

        let payload = DbBootPayload {
            packet_size,
            version: DB_BOOT_VERSION,
            sections,
            global_time,
            item_id_ranges,
            gm_hosts,
            admins,
            monarch,
            monarch_candidacy,
            end_marker: DB_BOOT_END_MARKER,
        };

        // The protocol encoder is the authoritative validation boundary for
        // fixed-tail widths/counts and the exact packet-size prefix.  Discard
        // its temporary output after validation; response encoding repeats
        // the same checked operation when requested.
        let _validated = payload.encode_with_limit(profile, max_payload_size)?;

        Ok(Self {
            profile,
            max_payload_size,
            payload,
        })
    }

    /// Return the explicit feature profile bound to this snapshot.
    #[must_use]
    pub const fn profile(&self) -> BootFeatureProfile {
        self.profile
    }

    /// Return the complete-payload allocation limit used at composition.
    #[must_use]
    pub const fn max_payload_size(&self) -> usize {
        self.max_payload_size
    }

    /// Borrow the validated payload without exposing mutable snapshot state.
    #[must_use]
    pub const fn payload(&self) -> &DbBootPayload {
        &self.payload
    }

    /// Consume the snapshot and return its validated payload.
    #[must_use]
    pub fn into_payload(self) -> DbBootPayload {
        self.payload
    }

    /// Encode the complete version-6 boot payload using the bound profile.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError`] if the payload no longer passes the protocol
    /// encoder's validation or exceeds the stored limit.
    pub fn encode(&self) -> Result<Vec<u8>, DbBootError> {
        self.payload
            .encode_with_limit(self.profile, self.max_payload_size)
    }

    /// Encode the source-fixed `HEADER_DG_BOOT` response frame.
    ///
    /// The outer peer handle is always zero, as in `ClientManager::QUERY_BOOT`.
    /// No `ENABLE_ITEMSHOP` frame is appended; that legacy feature is a
    /// separate DB frame and is outside this payload boundary.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError`] for the same validation, size, or allocation
    /// failures as [`Self::encode`].
    pub fn encode_frame(&self) -> Result<DbFrame, DbBootError> {
        self.payload
            .encode_frame_with_limit(self.profile, self.max_payload_size)
    }
}

fn validate_profile_sections(
    profile: BootFeatureProfile,
    sections: &[BootSection],
) -> Result<(), DbBootError> {
    let expected = profile.section_kinds();
    if sections.len() != expected.len() {
        return Err(DbBootError::SectionCountMismatch {
            expected: expected.len(),
            actual: sections.len(),
        });
    }

    for (index, (section, expected_kind)) in sections.iter().zip(expected).enumerate() {
        if section.kind != *expected_kind {
            return Err(DbBootError::UnexpectedSectionKind {
                index,
                expected: *expected_kind,
                actual: section.kind,
            });
        }
        if section.record_size == 0 {
            return Err(DbBootError::InvalidRecordSize {
                section: section.kind,
                record_size: section.record_size,
            });
        }
        let expected_len =
            checked_product(usize::from(section.record_size), usize::from(section.count))?;
        if section.data.len() != expected_len {
            return Err(DbBootError::SectionLengthMismatch {
                section: section.kind,
                expected: expected_len,
                actual: section.data.len(),
            });
        }
    }
    Ok(())
}

fn payload_length(
    sections: &[BootSection],
    gm_host_count: usize,
    admin_count: usize,
    candidacy_count: usize,
) -> Result<usize, DbBootError> {
    // u32 length prefix + version byte.
    let mut length = 5_usize;
    for section in sections {
        length = checked_add(length, 4)?;
        length = checked_add(length, section.data.len())?;
    }

    // x86 time_t.
    length = checked_add(length, X86_TIME_T_WIRE_SIZE)?;
    // Item-range header plus the source's two records under count=1.
    length = checked_add(length, 4)?;
    length = checked_add(length, checked_product(ITEM_ID_RANGE_WIRE_SIZE, 2)?)?;
    // GM-host header and records.
    length = checked_add(length, 4)?;
    length = checked_add(length, checked_product(GM_HOST_WIRE_SIZE, gm_host_count)?)?;
    // Administrator header and records.
    length = checked_add(length, 4)?;
    length = checked_add(length, checked_product(ADMIN_INFO_WIRE_SIZE, admin_count)?)?;
    // Monarch header and exactly one record.
    length = checked_add(length, 4)?;
    length = checked_add(length, MONARCH_INFO_WIRE_SIZE)?;
    // Candidacy header and records.
    length = checked_add(length, 4)?;
    length = checked_add(
        length,
        checked_product(MONARCH_CANDIDACY_WIRE_SIZE, candidacy_count)?,
    )?;
    // End marker.
    checked_add(length, 2)
}

fn checked_add(left: usize, right: usize) -> Result<usize, DbBootError> {
    left.checked_add(right).ok_or(DbBootError::SizeOverflow)
}

fn checked_product(left: usize, right: usize) -> Result<usize, DbBootError> {
    left.checked_mul(right).ok_or(DbBootError::SizeOverflow)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::boot_composition::source_fixed_record_size;
    use protocol::db_boot::{
        parse_db_boot_frame, BootAdminInfo, BootItemIdRange, BootMonarchCandidacy, BootSectionKind,
        DB_BOOT_RESPONSE_HANDLE, HEADER_DG_BOOT,
    };
    use protocol::db_records::{ITEM_TABLE_RECORD_WIRE_SIZE, MOB_TABLE_RECORD_WIRE_SIZE};

    fn section(kind: BootSectionKind, data: &[u8]) -> BootSection {
        let count = u16::from(u8::from(!data.is_empty()));
        let record_size = match kind {
            BootSectionKind::Mob => u16::try_from(MOB_TABLE_RECORD_WIRE_SIZE).unwrap(),
            BootSectionKind::Item => u16::try_from(ITEM_TABLE_RECORD_WIRE_SIZE).unwrap(),
            _ => u16::try_from(if data.is_empty() { 1 } else { data.len() }).unwrap(),
        };
        BootSection {
            kind,
            record_size,
            count,
            data: data.to_vec(),
        }
    }

    fn parts(profile: BootFeatureProfile) -> BootSnapshotParts {
        let sections = profile
            .section_kinds()
            .iter()
            .map(|kind| section(*kind, &[]))
            .collect();
        let range = BootItemIdRange {
            min: 10,
            max: 20,
            usable_item_id_min: 10,
        };
        BootSnapshotParts::new(
            sections,
            0x1122_3344,
            BootItemIdRanges {
                record_size: u16::try_from(ITEM_ID_RANGE_WIRE_SIZE).unwrap(),
                declared_count: 1,
                active: range,
                spare: range,
            },
            BootGmHostSection {
                record_size: u16::try_from(GM_HOST_WIRE_SIZE).unwrap(),
                count: 0,
                hosts: Vec::new(),
            },
            BootAdminSection {
                record_size: u16::try_from(ADMIN_INFO_WIRE_SIZE).unwrap(),
                count: 0,
                admins: Vec::new(),
            },
            BootMonarchInfo {
                pid: [0; 4],
                money: [0; 4],
                name: [[0; 32]; 4],
                date: [[0; 32]; 4],
            },
            BootMonarchCandidacySection {
                record_size: u16::try_from(MONARCH_CANDIDACY_WIRE_SIZE).unwrap(),
                count: 0,
                candidates: Vec::new(),
            },
        )
    }

    #[test]
    fn composes_every_explicit_profile_without_inventing_optional_sections() {
        for mask in 0_u8..8 {
            let profile = BootFeatureProfile::new(mask & 1 != 0, mask & 2 != 0, mask & 4 != 0);
            let snapshot = BootSnapshot::compose(profile, parts(profile)).unwrap();
            assert_eq!(snapshot.profile(), profile);
            assert_eq!(
                snapshot.payload().sections.len(),
                profile.section_kinds().len()
            );
            assert_eq!(snapshot.payload().version, DB_BOOT_VERSION);
            assert_eq!(snapshot.payload().end_marker, DB_BOOT_END_MARKER);
            // Empty sections still carry their four-byte width/count header.
            // These are the source-derived minimal (403) and active (415)
            // complete-payload lengths for this zero-row fixture.
            if profile == BootFeatureProfile::minimal() {
                assert_eq!(snapshot.payload().packet_size, 403);
            }
            if profile == BootFeatureProfile::active() {
                assert_eq!(snapshot.payload().packet_size, 415);
            }
            let frame = snapshot.encode_frame().unwrap();
            assert_eq!(frame.header, HEADER_DG_BOOT);
            assert_eq!(frame.handle, DB_BOOT_RESPONSE_HANDLE);
            let parsed = parse_db_boot_frame(&frame, profile).unwrap();
            assert_eq!(parsed, *snapshot.payload());
        }
    }

    #[test]
    fn preserves_opaque_section_bytes_and_never_reorders_them() {
        let profile = BootFeatureProfile::minimal();
        let mut source = parts(profile);
        let first_kind = profile.section_kinds()[0];
        let opaque = vec![0xde, 0xad, 0xbe, 0xef]
            .into_iter()
            .cycle()
            .take(source_fixed_record_size(first_kind).unwrap_or(4))
            .collect::<Vec<_>>();
        source.sections[0] = section(first_kind, &opaque);
        let snapshot = BootSnapshot::compose(profile, source).unwrap();
        assert_eq!(snapshot.payload().sections[0].data, opaque);
        let parsed = parse_db_boot_frame(&snapshot.encode_frame().unwrap(), profile).unwrap();
        assert_eq!(parsed.sections[0].data, opaque);
    }

    #[test]
    fn rejects_missing_or_reordered_sections_instead_of_filling_them() {
        let profile = BootFeatureProfile::minimal();
        let mut source = parts(profile);
        source.sections.pop();
        assert!(matches!(
            BootSnapshot::compose(profile, source),
            Err(DbBootError::SectionCountMismatch { .. })
        ));

        let mut source = parts(profile);
        source.sections.swap(0, 1);
        assert!(matches!(
            BootSnapshot::compose(profile, source),
            Err(DbBootError::UnexpectedSectionKind { index: 0, .. })
        ));
    }

    #[test]
    fn rejects_malformed_opaque_metadata_and_fixed_tail_metadata() {
        let profile = BootFeatureProfile::minimal();
        let mut source = parts(profile);
        source.sections[0].record_size = 0;
        assert!(matches!(
            BootSnapshot::compose(profile, source),
            Err(DbBootError::InvalidRecordSize { .. })
        ));

        let mut source = parts(profile);
        source.gm_hosts.record_size = 15;
        assert!(matches!(
            BootSnapshot::compose(profile, source),
            Err(DbBootError::InvalidFixedRecordSize {
                field: "gm_hosts",
                ..
            })
        ));
    }

    #[test]
    fn enforces_a_complete_payload_limit_before_encoding() {
        let profile = BootFeatureProfile::minimal();
        let error = BootSnapshot::compose_with_limit(profile, parts(profile), 1).unwrap_err();
        assert!(matches!(
            error,
            DbBootError::EncodePayloadTooLarge { maximum: 1, .. }
        ));
    }

    #[test]
    fn preserves_fixed_tail_values_and_the_two_item_range_quirk() {
        let profile = BootFeatureProfile::minimal();
        let mut source = parts(profile);
        source.global_time = -7;
        source.item_id_ranges.active = BootItemIdRange {
            min: 1,
            max: 2,
            usable_item_id_min: 3,
        };
        source.item_id_ranges.spare = BootItemIdRange {
            min: 4,
            max: 5,
            usable_item_id_min: 6,
        };
        source
            .monarch_candidacy
            .candidates
            .push(BootMonarchCandidacy {
                pid: 99,
                name: [b'x'; 32],
                date: [b'y'; 32],
            });
        source.monarch_candidacy.count = 1;
        let snapshot = BootSnapshot::compose(profile, source).unwrap();
        let parsed = parse_db_boot_frame(&snapshot.encode_frame().unwrap(), profile).unwrap();
        assert_eq!(parsed.global_time, -7);
        assert_eq!(parsed.item_id_ranges.active.usable_item_id_min, 3);
        assert_eq!(parsed.item_id_ranges.spare.usable_item_id_min, 6);
        assert_eq!(parsed.monarch_candidacy.candidates[0].pid, 99);
    }

    #[test]
    fn composition_error_is_a_protocol_error_without_sql_state() {
        let profile = BootFeatureProfile::minimal();
        let mut source = parts(profile);
        source.admins.admins.push(BootAdminInfo {
            id: 1,
            account: [0; 32],
            name: [0; 32],
            contact_ip: [0; 16],
            server_ip: [0; 16],
            authority: 0,
        });
        // The count must describe the supplied vector; the builder must not
        // silently repair a source mismatch.
        source.admins.count = 0;
        let error = BootSnapshot::compose(profile, source).unwrap_err();
        assert!(matches!(
            error,
            DbBootError::InvalidRecordCount {
                field: "admins",
                ..
            }
        ));
        let _: &dyn Error = &error;
    }
}
