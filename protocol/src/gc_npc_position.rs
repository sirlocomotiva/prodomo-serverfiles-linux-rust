//! `HEADER_GC_NPC_POSITION` (115): the static NPCs of the map a client enters.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:2426-2441`:
//!
//! ```ignore
//! struct TNPCPosition
//! {
//!     BYTE bType;
//!     char name[CHARACTER_NAME_MAX_LEN+1];
//!     long x;
//!     long y;
//! };
//!
//! typedef struct SPacketGCNPCPosition
//! {
//!     BYTE header;
//!     WORD size;
//!     WORD count;
//!     // array of TNPCPosition
//! } TPacketGCNPCPosition;
//! ```
//!
//! The producer is `SECTREE_MANAGER::SendNPCPosition`
//! (`server/server/game/sectree_manager.cpp:1089-1128`). It returns without
//! sending anything when the map has no recorded NPC, and otherwise writes the
//! five header bytes through `BufferedPacket` and the elements through
//! `Packet`, so `size = sizeof(TPacketGCNPCPosition) + count * sizeof(TNPCPosition)`
//! and the header is sent even when the element buffer is empty.
//!
//! # `CHARACTER_NAME_MAX_LEN + 1` is 25
//!
//! `CHARACTER_NAME_MAX_LEN` is 24 (`server/server/common/length.h:15`), so the
//! name field holds 25 bytes. It is a fixed `char` array, not a C string: the
//! producer uses `strlcpy`, which always terminates, but a NUL is not a
//! requirement of the wire and the codec keeps the field raw.
//!
//! # Width
//!
//! One element is 1 + 25 + 4 + 4 = 34 bytes on i386, where `long` is 4 bytes.
//! The five-byte header is 1 + 2 + 2. Both were measured by compiling the
//! verbatim struct bodies with `g++ -m32` under `static_assert`, next to the
//! repository's known-width controls; the probe is in
//! `.scratch/ledger187/root-probe/`.

use std::fmt;

use crate::gc_actors::NAME_LEN;
use crate::gc_inventory::HEADER_GC_NPC_POSITION;

/// `sizeof(TPacketGCNPCPosition)`: header, total size, and count.
pub const GC_NPC_POSITION_WIRE_SIZE: usize = 5;

/// `sizeof(TNPCPosition)`: a type byte, a 25-byte name, and two positions.
pub const GC_NPC_POSITION_ELEMENT_SIZE: usize = 1 + NAME_LEN + 4 + 4;

/// One static NPC the client draws without a world update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcNpcPositionEntry {
    /// `bType`: the shop or warp kind, opaque to the codec.
    pub npc_type: u8,
    /// The raw `name` field.
    pub name: [u8; NAME_LEN],
    /// `x`, in world units.
    pub x: i32,
    /// `y`, in world units.
    pub y: i32,
}

impl GcNpcPositionEntry {
    /// Number of bytes one element occupies on the wire.
    pub const WIRE_SIZE: usize = GC_NPC_POSITION_ELEMENT_SIZE;

    /// Build an element with every field set explicitly.
    #[must_use]
    pub const fn new(npc_type: u8, name: [u8; NAME_LEN], x: i32, y: i32) -> Self {
        Self {
            npc_type,
            name,
            x,
            y,
        }
    }

    /// Appends the 34 packed bytes to `out`, without a header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.npc_type);
        out.extend_from_slice(&self.name);
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
    }

    /// Reads the 34 packed bytes from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcNpcPositionError::Truncated`] unless `bytes` holds the full
    /// [`GcNpcPositionEntry::WIRE_SIZE`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNpcPositionError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcNpcPositionError::Truncated {
                context: "GcNpcPositionEntry",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        let mut name = [0u8; NAME_LEN];
        name.copy_from_slice(&bytes[1..=NAME_LEN]);
        Ok(Self {
            npc_type: bytes[0],
            name,
            x: i32::from_le_bytes([bytes[26], bytes[27], bytes[28], bytes[29]]),
            y: i32::from_le_bytes([bytes[30], bytes[31], bytes[32], bytes[33]]),
        })
    }
}

/// `HEADER_GC_NPC_POSITION` (115): the map's static NPCs.
///
/// A count of zero is a legal record: the producer sends the five header bytes
/// alone, and the client reads a count of 0 and draws no NPC.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GcNpcPosition {
    /// The NPCs, in the order the world recorded them.
    pub entries: Vec<GcNpcPositionEntry>,
}

impl GcNpcPosition {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_NPC_POSITION.value()
    }

    /// Build the record for a list of NPCs.
    #[must_use]
    pub fn new(entries: Vec<GcNpcPositionEntry>) -> Self {
        Self { entries }
    }

    /// The whole record's length on the wire, which is what `size` holds.
    #[must_use]
    pub fn wire_size(&self) -> usize {
        GC_NPC_POSITION_WIRE_SIZE + self.entries.len() * GcNpcPositionEntry::WIRE_SIZE
    }

    /// Appends the whole packed record to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`GcNpcPositionError::TooManyEntries`] when the list is long
    /// enough that `size` or `count` would not fit its `WORD` field.
    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), GcNpcPositionError> {
        let size =
            u16::try_from(self.wire_size()).map_err(|_| GcNpcPositionError::TooManyEntries {
                count: self.entries.len(),
            })?;
        let count =
            u16::try_from(self.entries.len()).map_err(|_| GcNpcPositionError::TooManyEntries {
                count: self.entries.len(),
            })?;
        out.push(Self::header());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        for entry in &self.entries {
            entry.encode_into(out);
        }
        Ok(())
    }

    /// Decodes the whole packed record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcNpcPositionError::Truncated`] when the buffer is shorter than
    /// the record's own `size`, [`GcNpcPositionError::Header`] when the leading
    /// byte is not [`HEADER_GC_NPC_POSITION`], and
    /// [`GcNpcPositionError::Size`] when `size` is not
    /// `5 + 34 * count`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNpcPositionError> {
        if bytes.len() < GC_NPC_POSITION_WIRE_SIZE {
            return Err(GcNpcPositionError::Truncated {
                context: "GcNpcPosition",
                needed: GC_NPC_POSITION_WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header() {
            return Err(GcNpcPositionError::Header {
                expected: Self::header(),
                actual: bytes[0],
            });
        }
        let size = usize::from(u16::from_le_bytes([bytes[1], bytes[2]]));
        let count = usize::from(u16::from_le_bytes([bytes[3], bytes[4]]));
        if size != GC_NPC_POSITION_WIRE_SIZE + count * GcNpcPositionEntry::WIRE_SIZE {
            return Err(GcNpcPositionError::Size { size, count });
        }
        if bytes.len() < size {
            return Err(GcNpcPositionError::Truncated {
                context: "GcNpcPosition",
                needed: size,
                actual: bytes.len(),
            });
        }
        let mut entries = Vec::with_capacity(count);
        for index in 0..count {
            let at = GC_NPC_POSITION_WIRE_SIZE + index * GcNpcPositionEntry::WIRE_SIZE;
            entries.push(GcNpcPositionEntry::decode(&bytes[at..])?);
        }
        Ok(Self { entries })
    }
}

/// What a [`GcNpcPosition`] decode or encode can report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcNpcPositionError {
    /// The buffer ended before the record did.
    Truncated {
        /// Which record was being decoded.
        context: &'static str,
        /// Bytes the record needs.
        needed: usize,
        /// Bytes the buffer actually had.
        actual: usize,
    },
    /// The leading byte is not this record's header.
    Header {
        /// The header byte this record requires.
        expected: u8,
        /// The header byte that was present.
        actual: u8,
    },
    /// `size` does not agree with `count`.
    Size {
        /// The `size` the record carried.
        size: usize,
        /// The `count` it carried.
        count: usize,
    },
    /// The list is too long for the `WORD` size and count fields.
    TooManyEntries {
        /// The number of entries that was refused.
        count: usize,
    },
}

impl fmt::Display for GcNpcPositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                context,
                needed,
                actual,
            } => write!(f, "{context}: need {needed} bytes, buffer held {actual}"),
            Self::Header { expected, actual } => write!(
                f,
                "GcNpcPosition: header {actual:#04x}, expected {expected:#04x}"
            ),
            Self::Size { size, count } => write!(
                f,
                "GcNpcPosition: size {size} is not {GC_NPC_POSITION_WIRE_SIZE} plus {count} \
                 {GC_NPC_POSITION_ELEMENT_SIZE}-byte elements"
            ),
            Self::TooManyEntries { count } => {
                write!(
                    f,
                    "GcNpcPosition: {count} entries do not fit the 16-bit size and count"
                )
            }
        }
    }
}

impl std::error::Error for GcNpcPositionError {}

#[cfg(test)]
mod tests {
    use super::*;

    const MEASURED_HEADER: usize = 5;
    const MEASURED_ELEMENT: usize = 34;

    fn name(text: &[u8]) -> [u8; NAME_LEN] {
        let mut field = [0u8; NAME_LEN];
        field[..text.len()].copy_from_slice(text);
        field
    }

    /// The golden bytes, taken from the source field order and not from the encoder.
    fn golden(count: usize, entries: &[GcNpcPositionEntry]) -> Vec<u8> {
        let mut want = vec![115];
        let total = u16::try_from(MEASURED_HEADER + count * MEASURED_ELEMENT)
            .expect("a test view is short");
        want.extend_from_slice(&total.to_le_bytes());
        want.extend_from_slice(
            &u16::try_from(count)
                .expect("a test view is short")
                .to_le_bytes(),
        );
        for entry in entries {
            want.push(entry.npc_type);
            want.extend_from_slice(&entry.name);
            want.extend_from_slice(&entry.x.to_le_bytes());
            want.extend_from_slice(&entry.y.to_le_bytes());
        }
        want
    }

    fn sample() -> GcNpcPosition {
        GcNpcPosition::new(vec![
            GcNpcPositionEntry::new(0x01, name(b"shop"), 0x0102_0304, -0x0102_0305),
            GcNpcPositionEntry::new(0x0a, name(b"warehouse"), 0x0a0b_0c0d, 0x0f0f_0f0f),
        ])
    }

    #[test]
    fn the_widths_are_the_measured_i386_ones() {
        assert_eq!(GC_NPC_POSITION_WIRE_SIZE, MEASURED_HEADER);
        assert_eq!(GcNpcPositionEntry::WIRE_SIZE, MEASURED_ELEMENT);
        assert_eq!(GcNpcPosition::new(Vec::new()).wire_size(), 5);
        assert_eq!(sample().wire_size(), 73);
    }

    #[test]
    fn the_name_field_is_twenty_five_bytes() {
        // `CHARACTER_NAME_MAX_LEN + 1` with `CHARACTER_NAME_MAX_LEN = 24`.
        assert_eq!(NAME_LEN, 25);
        assert_eq!(MEASURED_ELEMENT, 1 + 25 + 4 + 4);
    }

    #[test]
    fn encodes_the_golden_bytes() {
        let mut got = Vec::new();
        sample().encode_into(&mut got).expect("fits");
        assert_eq!(got, golden(2, &sample().entries));
    }

    #[test]
    fn an_empty_list_is_a_five_byte_record() {
        let mut got = Vec::new();
        GcNpcPosition::default()
            .encode_into(&mut got)
            .expect("fits");
        assert_eq!(got, golden(0, &[]));
        assert_eq!(
            GcNpcPosition::decode(&got).expect("reads"),
            GcNpcPosition::default()
        );
    }

    #[test]
    fn round_trips_and_leaves_trailing_bytes_alone() {
        let mut bytes = golden(2, &sample().entries);
        bytes.extend_from_slice(&[0xcc; 8]);
        assert_eq!(GcNpcPosition::decode(&bytes).expect("reads"), sample());
    }

    #[test]
    fn a_name_field_with_no_nul_round_trips() {
        // The field is raw, so 25 bytes of Name and no terminator is legal.
        let full = [b'X'; NAME_LEN];
        let record = GcNpcPosition::new(vec![GcNpcPositionEntry::new(7, full, 1, 2)]);
        let mut bytes = Vec::new();
        record.encode_into(&mut bytes).expect("fits");
        assert_eq!(&bytes[6..31], &full);
        assert_eq!(GcNpcPosition::decode(&bytes).expect("reads"), record);
    }

    #[test]
    fn rejects_a_foreign_header() {
        let mut bytes = golden(1, &sample().entries[..1]);
        bytes[0] = 0x4e;
        assert!(matches!(
            GcNpcPosition::decode(&bytes),
            Err(GcNpcPositionError::Header { actual: 0x4e, .. })
        ));
    }

    #[test]
    fn rejects_a_buffer_shorter_than_the_record_claims() {
        let bytes = golden(2, &sample().entries);
        for cut in 0..bytes.len() {
            assert!(
                matches!(
                    GcNpcPosition::decode(&bytes[..cut]),
                    Err(GcNpcPositionError::Truncated { .. })
                ),
                "a {cut}-byte prefix must not decode"
            );
        }
    }

    #[test]
    fn rejects_a_count_that_does_not_match_the_size() {
        // A size that is not `5 + 34 * count` is refused before any element is read.
        for (size, count) in [(6u16, 0u16), (4, 0), (5, 1), (39, 0), (40, 1), (73, 1)] {
            let mut bytes = golden(0, &[]);
            bytes[1..3].copy_from_slice(&size.to_le_bytes());
            bytes[3..5].copy_from_slice(&count.to_le_bytes());
            assert!(
                matches!(
                    GcNpcPosition::decode(&bytes),
                    Err(GcNpcPositionError::Size { size: got_size, count: got_count })
                        if got_size == usize::from(size) && got_count == usize::from(count)
                ),
                "size {size} with count {count} must be refused"
            );
        }
    }

    #[test]
    fn trusts_the_size_and_the_count_together_only() {
        // size and count agree on three elements, but the buffer holds two.
        let mut bytes = golden(2, &sample().entries);
        bytes[3..5].copy_from_slice(&3u16.to_le_bytes());
        bytes[1..3].copy_from_slice(&107u16.to_le_bytes());
        assert!(matches!(
            GcNpcPosition::decode(&bytes),
            Err(GcNpcPositionError::Truncated {
                context: "GcNpcPosition",
                needed: 107,
                actual: 73
            })
        ));
    }

    #[test]
    fn refuses_a_list_too_long_for_the_count_field() {
        let many = vec![GcNpcPositionEntry::default(); usize::from(u16::MAX) + 1];
        assert!(matches!(
            GcNpcPosition::new(many).encode_into(&mut Vec::new()),
            Err(GcNpcPositionError::TooManyEntries { .. })
        ));
    }
}
