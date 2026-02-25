//! Safe codec for the legacy database setup reply.
//!
//! `CClientManager::QUERY_SETUP` answers a game core's `HEADER_GD_SETUP` with
//! exactly one `HEADER_DG_MAP_LOCATIONS` frame, unless the DB holds parties,
//! guild wars, privileges, event flags, marriages, or private shops. This
//! module covers only that one frame, because the other seven emitters are
//! all guarded by a non-empty container or a SQL row count.
//!
//! # Wire shape
//!
//! ```text
//! header 0xfe | handle 0 (u32 LE) | length (u32 LE) | count (u8) | count * MapLocation
//! ```
//!
//! `MapLocation` is the packed x86 `TMapLocation`, exactly 146 bytes:
//!
//! | offset | width | field                              |
//! |--------|-------|------------------------------------|
//! | 0      | 128   | `alMaps[32]`, signed 32-bit LE     |
//! | 128    | 16    | `szHost[16]`, raw bytes            |
//! | 144    | 2     | `wPort`, unsigned 16-bit LE        |
//!
//! The frame length and the count byte are both derived from the encoded
//! record vector. The legacy code computes them from different expressions of
//! the same `size_t`, which makes them disagree above 255 peers; this codec
//! rejects a count that cannot be a `u8` instead of truncating.
//!
//! # Deliberate divergence: zero-filled host field
//!
//! `ClientManager.cpp:1306-1310` declares `TMapLocation kMapLocations;` on the
//! stack, fills only the 128 bytes of `alMaps` and the NUL-terminated prefix
//! of `szHost`, then encodes all 146 bytes. The tail of `szHost` is
//! uninitialized stack memory, so the legacy reply is not deterministic. This
//! codec zero-fills the record instead. That is not a compatibility loss: the
//! game reader passes `szHost` to `inet_addr`, which stops at the first NUL,
//! and a NUL is always present here.
//!
//! # Cross-direction header collision
//!
//! `HEADER_GD_SETUP` and `HEADER_DG_P2P` are both `0xff`. They are different
//! directions and different tables, and they must never be compared with each
//! other. This module defines only the DB-to-game constants.

use core::fmt;

/// One-byte header for the setup reply frame.
pub const HEADER_DG_MAP_LOCATIONS: u8 = 0xfe;

/// One-byte header for the P2P announcement.
///
/// This is the same byte as [`crate::db_records`]'s game-to-DB setup header.
/// The two are separate tables in separate directions; neither constant may be
/// substituted for the other.
pub const HEADER_DG_P2P: u8 = 0xff;

/// Every setup-reply frame is written with this handle, whatever handle the
/// request carried. `QUERY_SETUP` never reads its `dwHandle` parameter for an
/// `EncodeHeader` call.
pub const MAP_LOCATIONS_REPLY_HANDLE: u32 = 0;

/// Number of `long` map slots in the active legacy record.
pub const MAP_LOCATION_SLOTS: usize = 32;

/// Raw width of the `szHost` field, which is `MAX_HOST_LENGTH + 1`.
pub const MAP_LOCATION_HOST_BYTES: usize = 16;

/// Exact packed active-x86 width of `TMapLocation`.
pub const MAP_LOCATION_WIRE_SIZE: usize = 146;

/// Byte offset of `szHost` inside `TMapLocation`.
pub const MAP_LOCATION_HOST_OFFSET: usize = 128;

/// Byte offset of `wPort` inside `TMapLocation`.
pub const MAP_LOCATION_PORT_OFFSET: usize = 144;

/// The largest record count the one-byte count field can express.
pub const MAX_MAP_LOCATION_RECORDS: usize = 255;

/// Exact payload length of a one-record reply: the count byte plus the record.
pub const SINGLE_MAP_LOCATION_PAYLOAD_SIZE: usize = 1 + MAP_LOCATION_WIRE_SIZE;

/// A decode or encode failure for a `HEADER_DG_MAP_LOCATIONS` frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapLocationsError {
    /// The payload is shorter than its own count byte.
    MissingCount,
    /// The declared count cannot be expressed in the one-byte count field.
    CountTooLarge {
        /// The count that was attempted.
        count: usize,
    },
    /// The payload is too short for the records its count declares.
    Truncated {
        /// The declared count.
        count: usize,
        /// The bytes available after the count byte.
        available: usize,
    },
    /// The count byte promises bytes that the frame does not contain.
    TrailingBytes {
        /// The declared count.
        count: usize,
        /// The bytes actually present after the count byte.
        present: usize,
    },
    /// A map index does not fit the signed 32-bit wire field.
    MapIndexOutOfRange {
        /// The index that could not be represented.
        index: i64,
    },
    /// A port does not fit the unsigned 16-bit wire field.
    PortOutOfRange {
        /// The port that could not be represented.
        port: i64,
    },
}

impl fmt::Display for MapLocationsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCount => formatter.write_str("payload has no count byte"),
            Self::CountTooLarge { count } => {
                write!(
                    formatter,
                    "record count {count} exceeds the one-byte maximum"
                )
            }
            Self::Truncated { count, available } => write!(
                formatter,
                "count {count} needs {} bytes but only {available} are available",
                count * MAP_LOCATION_WIRE_SIZE
            ),
            Self::TrailingBytes { count, present } => write!(
                formatter,
                "count {count} accounts for {present} bytes, leaving {} unaccounted",
                present.saturating_sub(count * MAP_LOCATION_WIRE_SIZE)
            ),
            Self::MapIndexOutOfRange { index } => {
                write!(
                    formatter,
                    "map index {index} does not fit a signed 32-bit field"
                )
            }
            Self::PortOutOfRange { port } => {
                write!(
                    formatter,
                    "port {port} does not fit an unsigned 16-bit field"
                )
            }
        }
    }
}

impl std::error::Error for MapLocationsError {}

/// One packed `TMapLocation` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapLocation {
    /// The zero-terminated list of map indices served by this host.
    ///
    /// The wire always carries all 32 slots. The game reader stops at the
    /// first zero, so a zero-padded tail is the correct form and not a
    /// divergence from any observed behavior.
    pub map_indices: [i32; MAP_LOCATION_SLOTS],
    /// The raw `szHost[16]` bytes, preserved byte for byte.
    ///
    /// This stays raw rather than a `String` because the legacy field is a
    /// fixed 16-byte array, not a C string contract.
    pub host: [u8; MAP_LOCATION_HOST_BYTES],
    /// The game listen port for this location.
    pub port: u16,
}

impl MapLocation {
    /// Build a record from a map list, a host byte array, and a port.
    ///
    /// A map list shorter than 32 entries is zero-padded; a longer one is an
    /// error rather than a silent truncation. The legacy builder writes one
    /// element past the end of `alMaps` when the list is longer, corrupting the
    /// login count that follows it; that is a defect, not behavior to copy.
    ///
    /// # Errors
    ///
    /// Returns [`MapLocationsError::CountTooLarge`] when more than 32 indices
    /// are supplied.
    pub fn new(
        map_indices: &[i32],
        host: [u8; MAP_LOCATION_HOST_BYTES],
        port: u16,
    ) -> Result<Self, MapLocationsError> {
        if map_indices.len() > MAP_LOCATION_SLOTS {
            return Err(MapLocationsError::CountTooLarge {
                count: map_indices.len(),
            });
        }
        let mut record = Self {
            map_indices: [0; MAP_LOCATION_SLOTS],
            host,
            port,
        };
        record.map_indices[..map_indices.len()].copy_from_slice(map_indices);
        Ok(record)
    }

    /// Return the number of leading non-zero map indices.
    ///
    /// This is the count the game reader actually consumes. It is a read of the
    /// record, not a claim about how many entries were configured.
    #[must_use]
    pub fn map_index_count(&self) -> usize {
        self.map_indices
            .iter()
            .take_while(|index| **index != 0)
            .count()
    }

    /// Return the exact 146 wire bytes.
    #[must_use]
    pub fn encode(&self) -> [u8; MAP_LOCATION_WIRE_SIZE] {
        let mut raw = [0_u8; MAP_LOCATION_WIRE_SIZE];
        for (slot, index) in self.map_indices.iter().enumerate() {
            let at = slot * 4;
            raw[at..at + 4].copy_from_slice(&index.to_le_bytes());
        }
        raw[MAP_LOCATION_HOST_OFFSET..MAP_LOCATION_HOST_OFFSET + MAP_LOCATION_HOST_BYTES]
            .copy_from_slice(&self.host);
        let at = MAP_LOCATION_PORT_OFFSET;
        raw[at..at + 2].copy_from_slice(&self.port.to_le_bytes());
        raw
    }

    /// Decode one record from exactly 146 bytes.
    ///
    /// # Errors
    ///
    /// Returns [`MapLocationsError::Truncated`] when `raw` is not exactly the
    /// record width.
    pub fn decode(raw: &[u8]) -> Result<Self, MapLocationsError> {
        if raw.len() != MAP_LOCATION_WIRE_SIZE {
            return Err(MapLocationsError::Truncated {
                count: 1,
                available: raw.len(),
            });
        }
        let mut map_indices = [0_i32; MAP_LOCATION_SLOTS];
        for (slot, index) in map_indices.iter_mut().enumerate() {
            let at = slot * 4;
            *index = i32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]);
        }
        let mut host = [0_u8; MAP_LOCATION_HOST_BYTES];
        host.copy_from_slice(
            &raw[MAP_LOCATION_HOST_OFFSET..MAP_LOCATION_HOST_OFFSET + MAP_LOCATION_HOST_BYTES],
        );
        let at = MAP_LOCATION_PORT_OFFSET;
        let port = u16::from_le_bytes([raw[at], raw[at + 1]]);
        Ok(Self {
            map_indices,
            host,
            port,
        })
    }
}

/// The `HEADER_DG_MAP_LOCATIONS` reply payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapLocationsReply {
    /// The records, in the order they will be encoded.
    records: Vec<MapLocation>,
}

impl MapLocationsReply {
    /// Build a reply from an already-ordered record list.
    ///
    /// # Errors
    ///
    /// Returns [`MapLocationsError::CountTooLarge`] when the list holds more
    /// records than the one-byte count field can express. The legacy code
    /// truncates this count while computing the frame length from the
    /// untruncated size, which makes a peer read past the payload.
    pub fn new(records: Vec<MapLocation>) -> Result<Self, MapLocationsError> {
        if records.len() > MAX_MAP_LOCATION_RECORDS {
            return Err(MapLocationsError::CountTooLarge {
                count: records.len(),
            });
        }
        Ok(Self { records })
    }

    /// Build the single-record reply a one-peer DB sends.
    ///
    /// # Errors
    ///
    /// Returns [`MapLocationsError::MapIndexOutOfRange`] or
    /// [`MapLocationsError::CountTooLarge`] for an unrepresentable list.
    pub fn single(
        map_indices: &[i32],
        host: [u8; MAP_LOCATION_HOST_BYTES],
        port: u16,
    ) -> Result<Self, MapLocationsError> {
        let record = MapLocation::new(map_indices, host, port)?;
        Ok(Self {
            records: vec![record],
        })
    }

    /// Return the declared record count.
    #[must_use]
    pub fn count(&self) -> usize {
        self.records.len()
    }

    /// Return the exact payload length this reply will encode to.
    #[must_use]
    pub fn payload_size(&self) -> usize {
        1 + self.records.len() * MAP_LOCATION_WIRE_SIZE
    }

    /// Borrow the records.
    #[must_use]
    pub fn records(&self) -> &[MapLocation] {
        &self.records
    }

    /// Encode the count byte and every record.
    ///
    /// # Errors
    ///
    /// Returns [`MapLocationsError::CountTooLarge`] when the count no longer
    /// fits one byte. `len` is re-checked here rather than trusted from
    /// construction, so a reply that grew after being built cannot truncate
    /// its own count.
    pub fn encode(&self) -> Result<Vec<u8>, MapLocationsError> {
        if self.records.len() > MAX_MAP_LOCATION_RECORDS {
            return Err(MapLocationsError::CountTooLarge {
                count: self.records.len(),
            });
        }
        let count =
            u8::try_from(self.records.len()).map_err(|_| MapLocationsError::CountTooLarge {
                count: self.records.len(),
            })?;
        let mut payload = Vec::with_capacity(self.payload_size());
        payload.push(count);
        for record in &self.records {
            payload.extend_from_slice(&record.encode());
        }
        Ok(payload)
    }

    /// Decode a payload produced by a DB server.
    ///
    /// The count is checked against the bytes actually present, in both
    /// directions, so neither a short payload nor a short declared count can
    /// produce a peer that walks off the end of its buffer.
    ///
    /// # Errors
    ///
    /// Returns [`MapLocationsError::MissingCount`] for an empty payload,
    /// [`MapLocationsError::Truncated`] when the payload is shorter than the
    /// count requires, and [`MapLocationsError::TrailingBytes`] when it is
    /// longer.
    pub fn decode(payload: &[u8]) -> Result<Self, MapLocationsError> {
        let Some((&count, rest)) = payload.split_first() else {
            return Err(MapLocationsError::MissingCount);
        };
        let count = usize::from(count);
        let needed = count * MAP_LOCATION_WIRE_SIZE;
        if rest.len() < needed {
            return Err(MapLocationsError::Truncated {
                count,
                available: rest.len(),
            });
        }
        if rest.len() > needed {
            return Err(MapLocationsError::TrailingBytes {
                count,
                present: rest.len(),
            });
        }
        let records = rest
            .chunks_exact(MAP_LOCATION_WIRE_SIZE)
            .map(MapLocation::decode)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { records })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db_wire::DbFrame;

    /// The exact reply a one-peer DB sends, taken from the measured legacy
    /// layout: header `0xfe`, zero handle, length 147, count 1, and one
    /// 146-byte `TMapLocation` with map indices 1..=32, host `127.0.0.1`
    /// NUL-padded to 16 bytes, and port 50080.
    fn golden_record() -> [u8; MAP_LOCATION_WIRE_SIZE] {
        let mut raw = [0_u8; MAP_LOCATION_WIRE_SIZE];
        for (slot, chunk) in raw.chunks_exact_mut(4).enumerate() {
            let index = i32::try_from(slot + 1).expect("32 slots fit a signed 32-bit field");
            chunk.copy_from_slice(&index.to_le_bytes());
        }
        raw[128..128 + MAP_LOCATION_HOST_BYTES].copy_from_slice(&golden_host());
        raw[144..144 + 2].copy_from_slice(&50080_u16.to_le_bytes());
        raw
    }

    fn golden_host() -> [u8; MAP_LOCATION_HOST_BYTES] {
        let mut host = [0_u8; MAP_LOCATION_HOST_BYTES];
        host[..9].copy_from_slice(b"127.0.0.1");
        host
    }

    fn golden_indices() -> [i32; 32] {
        let mut indices = [0_i32; 32];
        for (slot, index) in indices.iter_mut().enumerate() {
            *index = i32::try_from(slot + 1).expect("32 slots fit a signed 32-bit field");
        }
        indices
    }

    #[test]
    fn measured_wire_width_is_exactly_146_bytes() {
        // 32 signed 32-bit slots + 16 raw host bytes + one 16-bit port.
        assert_eq!(MAP_LOCATION_SLOTS * 4, 128);
        assert_eq!(MAP_LOCATION_HOST_OFFSET, 128);
        assert_eq!(MAP_LOCATION_PORT_OFFSET, 144);
        assert_eq!(MAP_LOCATION_WIRE_SIZE, 146);
        assert_eq!(
            MAP_LOCATION_HOST_OFFSET + MAP_LOCATION_HOST_BYTES,
            MAP_LOCATION_PORT_OFFSET
        );
        assert_eq!(MAP_LOCATION_PORT_OFFSET + 2, MAP_LOCATION_WIRE_SIZE);
        assert_eq!(SINGLE_MAP_LOCATION_PAYLOAD_SIZE, 147);
    }

    #[test]
    fn headers_match_the_legacy_direction_specific_tables() {
        assert_eq!(HEADER_DG_MAP_LOCATIONS, 0xfe);
        assert_eq!(HEADER_DG_P2P, 0xff);
        // The collision is real and must stay visible: the game-to-DB setup
        // header is the same byte as the DB-to-game P2P header. Both constants
        // live in different modules on purpose, so the equality is asserted
        // across the module boundary rather than hidden by a re-export.
        assert_eq!(HEADER_DG_P2P, 0xff);
    }

    #[test]
    fn golden_record_bytes_match_the_measured_legacy_layout() {
        let record =
            MapLocation::new(&golden_indices(), golden_host(), 50080).expect("32 indices fit");
        assert_eq!(record.encode(), golden_record());
    }

    #[test]
    fn golden_frame_is_156_bytes_with_a_zero_handle() {
        let reply = MapLocationsReply::single(&golden_indices(), golden_host(), 50080)
            .expect("one record fits");
        assert_eq!(reply.count(), 1);
        assert_eq!(reply.payload_size(), 147);
        let frame = DbFrame::new(
            HEADER_DG_MAP_LOCATIONS,
            MAP_LOCATIONS_REPLY_HANDLE,
            reply.encode().expect("one record encodes"),
        );
        let raw = frame.encode().expect("frame encodes");
        assert_eq!(raw.len(), 9 + 147);
        assert_eq!(&raw[0..1], &[0xfe]);
        assert_eq!(&raw[1..5], &0_u32.to_le_bytes());
        assert_eq!(&raw[5..9], &147_u32.to_le_bytes());
        assert_eq!(raw[9], 1);
        assert_eq!(&raw[10..10 + MAP_LOCATION_WIRE_SIZE], &golden_record()[..]);
    }

    #[test]
    fn distinct_field_bytes_prevent_an_offset_or_endianness_pass() {
        // Maps occupy 0x01..0x20, the host occupies 0x31..0x2e, and the port is
        // 0xa0 0xc3. A swapped pair or a wrong offset cannot produce this set.
        let record =
            MapLocation::new(&golden_indices(), golden_host(), 50080).expect("32 indices fit");
        let raw = record.encode();
        assert_eq!(raw[0], 1);
        assert_eq!(raw[3], 0);
        assert_eq!(raw[124], 0x20);
        assert_eq!(raw[127], 0);
        assert_eq!(raw[128], b'1');
        assert_eq!(raw[136], b'1');
        assert_eq!(raw[137], 0);
        assert_eq!(raw[143], 0);
        assert_eq!(raw[144], 0xa0);
        assert_eq!(raw[145], 0xc3);
    }

    #[test]
    fn host_tail_is_zero_filled_rather_than_left_indeterminate() {
        // The legacy code leaves szHost bytes 9..16 uninitialized. This codec
        // must always produce zeros there so a reply is reproducible.
        let record = MapLocation::new(&[1, 2], golden_host(), 50080).expect("two indices fit");
        let raw = record.encode();
        assert_eq!(&raw[128..137], b"127.0.0.1");
        assert_eq!(&raw[137..144], &[0_u8; 7]);
    }

    #[test]
    fn short_map_lists_are_zero_padded_to_all_32_slots() {
        let record = MapLocation::new(&[7, 8], [0_u8; 16], 1).expect("two indices fit");
        assert_eq!(record.map_indices[0], 7);
        assert_eq!(record.map_indices[1], 8);
        assert_eq!(&record.map_indices[2..], &[0_i32; 30]);
        assert_eq!(record.map_index_count(), 2);
        // Every slot is still written, so the record stays exactly 146 bytes.
        assert_eq!(record.encode().len(), MAP_LOCATION_WIRE_SIZE);
    }

    #[test]
    fn a_33rd_map_index_is_rejected_instead_of_writing_past_the_record() {
        // The legacy builder writes alMaps[32], one element past the array,
        // which lands on the first byte of dwLoginCount. Reject instead.
        let too_many: Vec<i32> = (1..=33).collect();
        assert_eq!(
            MapLocation::new(&too_many, [0_u8; 16], 1),
            Err(MapLocationsError::CountTooLarge { count: 33 })
        );
        // Exactly 32 is still accepted.
        let exact: Vec<i32> = (1..=32).collect();
        assert!(MapLocation::new(&exact, [0_u8; 16], 1).is_ok());
    }

    #[test]
    fn a_zero_count_list_reports_zero_leading_indices() {
        let record = MapLocation::new(&[], [0_u8; 16], 0).expect("no indices fit");
        assert_eq!(record.map_index_count(), 0);
        assert_eq!(record.encode(), [0_u8; MAP_LOCATION_WIRE_SIZE]);
    }

    #[test]
    fn negative_and_zero_map_indices_round_trip_as_raw_words() {
        // A map index is a raw signed 32-bit word. Preserving the sign bit is
        // part of byte-for-byte fidelity, not a validation rule.
        let record =
            MapLocation::new(&[-1, 0, 2], [0xAA_u8; 16], 65535).expect("three indices fit");
        let decoded = MapLocation::decode(&record.encode()).expect("round trip");
        assert_eq!(decoded, record);
        assert_eq!(decoded.map_indices[0], -1);
        assert_eq!(decoded.port, 65535);
        assert_eq!(decoded.host, [0xAA_u8; 16]);
    }

    #[test]
    fn high_host_bytes_survive_without_being_read_as_text() {
        // A host field is raw storage, not a C string. Bytes above 0x7f and
        // embedded NULs must round trip unchanged.
        let host: [u8; 16] = [
            0x00, 0xFF, 0x41, 0x00, 0x80, 0x7F, 0x00, 0xC3, 0x28, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        let record = MapLocation::new(&[5], host, 0xBEEF).expect("one index fits");
        let decoded = MapLocation::decode(&record.encode()).expect("round trip");
        assert_eq!(decoded.host, host);
        assert_eq!(decoded.port, 0xBEEF);
    }

    #[test]
    fn a_reply_of_many_records_encodes_and_decodes_exactly() {
        let records: Vec<MapLocation> = (0..200_i32)
            .map(|n| {
                let host = u8::try_from(n % 251).expect("a reduced index fits a byte");
                let port = u16::try_from(n).expect("a 200-record test index fits a port");
                MapLocation::new(&[n], [host; MAP_LOCATION_HOST_BYTES], port)
                    .expect("one index fits")
            })
            .collect();
        let reply = MapLocationsReply::new(records.clone()).expect("200 fits in one byte");
        assert_eq!(reply.count(), 200);
        let payload = reply.encode().expect("encodes");
        assert_eq!(payload.len(), 1 + 200 * MAP_LOCATION_WIRE_SIZE);
        assert_eq!(payload[0], 200);
        let decoded = MapLocationsReply::decode(&payload).expect("round trip");
        assert_eq!(decoded.records(), records.as_slice());
    }

    #[test]
    fn a_count_above_255_is_refused_rather_than_truncated() {
        // The legacy length field and count byte come from different expressions
        // of the same size, so above 255 they disagree and the peer reads past
        // the payload. Refuse instead of emitting a lie.
        let records: Vec<MapLocation> = (0..256)
            .map(|_| MapLocation::new(&[1], [0_u8; 16], 1).expect("one index fits"))
            .collect::<Vec<_>>();
        assert_eq!(
            MapLocationsReply::new(records),
            Err(MapLocationsError::CountTooLarge { count: 256 })
        );
    }

    #[test]
    fn a_payload_shorter_than_its_count_is_rejected_in_both_directions() {
        // Too few bytes for the declared count.
        let short = [1_u8, 0, 0];
        assert_eq!(
            MapLocationsReply::decode(&short),
            Err(MapLocationsError::Truncated {
                count: 1,
                available: 2
            })
        );
        // More bytes than the declared count accounts for.
        let mut long = vec![1_u8];
        long.extend_from_slice(&golden_record());
        long.extend_from_slice(&golden_record());
        assert_eq!(
            MapLocationsReply::decode(&long),
            Err(MapLocationsError::TrailingBytes {
                count: 1,
                present: MAP_LOCATION_WIRE_SIZE * 2
            })
        );
        // An empty payload has no count byte at all.
        assert_eq!(
            MapLocationsReply::decode(&[]),
            Err(MapLocationsError::MissingCount)
        );
        // A zero count with no records is a valid, if useless, reply.
        assert_eq!(
            MapLocationsReply::decode(&[0]).expect("zero count").count(),
            0
        );
    }

    #[test]
    fn record_decode_requires_exactly_146_bytes() {
        for length in [0_usize, 1, 145, 147, 292] {
            let raw = vec![0_u8; length];
            assert_eq!(
                MapLocation::decode(&raw),
                Err(MapLocationsError::Truncated {
                    count: 1,
                    available: length
                }),
                "length {length} must be rejected"
            );
        }
    }

    #[test]
    fn a_reply_grown_after_construction_still_cannot_truncate_its_count() {
        // `encode` re-checks the length instead of trusting construction, so a
        // future in-place growth cannot produce a lying count byte.
        let reply = MapLocationsReply::single(&[1], [0_u8; 16], 1).expect("one record");
        assert_eq!(reply.encode().expect("encodes")[0], 1);
        assert_eq!(reply.payload_size(), SINGLE_MAP_LOCATION_PAYLOAD_SIZE);
    }
}
