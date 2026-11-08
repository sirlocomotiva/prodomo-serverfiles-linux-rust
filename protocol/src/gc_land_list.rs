//! `HEADER_GC_LAND_LIST` (130): the guild lands of the map a client enters, and the
//! land an update changes.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:2509-2521`:
//!
//! ```ignore
//! typedef struct
//! {
//!     DWORD   dwID;
//!     long    x, y;
//!     long    width, height;
//!     DWORD   dwGuildID;
//! } TLandPacketElement;
//!
//! typedef struct packet_land_list
//! {
//!     BYTE    header;
//!     WORD    size;
//! } TPacketGCLandList;
//! ```
//!
//! The producers are `CManager::SendLandList`
//! (`server/server/game/building.cpp:938-989`), which the enter-game walk calls with the
//! entering character's map (`server/server/game/input_login.cpp:676`), and
//! `CManager::UpdateLand` (`server/server/game/building.cpp:709-752`), which fans one
//! element to every client on the changed land's map. `SendLandList` sends nothing when the
//! map holds no land; otherwise it writes the three header bytes through `BufferedPacket`
//! and the elements through `Packet`, so `size = 3 + count * 20`. Before each element's
//! guild the producer sends that guild's NAME (`SendGuildName`), which is a separate
//! `GC_GUILD` record the codec does not carry.
//!
//! # Width
//!
//! One element is `dwID`, `x`, `y`, `width`, `height` and `dwGuildID`:
//! 4 + 4 * 4 + 4 = 24 bytes on i386, where `long` is 4 bytes. The three-byte
//! header is 1 + 2, both measured by compiling the verbatim struct bodies for i386
//! (`g++ -m32`, next to the 8-byte packed two-`long` control; the probe is in
//! `.scratch/probe475/`). The element count is implicit: `(size - 3) / 24`, and a
//! `size` that is not `3 + 24 * count` is malformed.

use std::fmt;

use crate::gc_inventory::HEADER_GC_LAND_LIST;

/// `sizeof(TPacketGCLandList)`: the header and the total size.
pub const GC_LAND_LIST_WIRE_SIZE: usize = 3;

/// `sizeof(TLandPacketElement)`: the id, the rect, and the owning guild.
pub const GC_LAND_LIST_ELEMENT_SIZE: usize = 24;

/// One guild land the client draws on its map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcLandListEntry {
    /// `dwID`: the land id.
    pub id: u32,
    /// `x`, in map-local units.
    pub x: i32,
    /// `y`, in map-local units.
    pub y: i32,
    /// `width`, in map-local units.
    pub width: i32,
    /// `height`, in map-local units.
    pub height: i32,
    /// `dwGuildID`: the owning guild, 0 for an unclaimed land.
    pub guild_id: u32,
}

impl GcLandListEntry {
    /// Number of bytes one element occupies on the wire.
    pub const WIRE_SIZE: usize = GC_LAND_LIST_ELEMENT_SIZE;

    /// Build an element with every field set explicitly.
    #[must_use]
    pub const fn new(id: u32, x: i32, y: i32, width: i32, height: i32, guild_id: u32) -> Self {
        Self {
            id,
            x,
            y,
            width,
            height,
            guild_id,
        }
    }

    /// Appends the 24 packed bytes to `out`, without a header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&self.guild_id.to_le_bytes());
    }

    /// Reads the 24 packed bytes from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcLandListError::Truncated`] unless `bytes` holds the full
    /// [`GcLandListEntry::WIRE_SIZE`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcLandListError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcLandListError::Truncated {
                context: "GcLandListEntry",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            id: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            x: i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            y: i32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            width: i32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
            height: i32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]),
            guild_id: u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
        })
    }
}

/// `HEADER_GC_LAND_LIST` (130): the map's guild lands.
///
/// The producers never send an empty list: `SendLandList` returns without
/// sending when the map holds no land, and `UpdateLand` always carries the one
/// changed land. The codec still encodes and decodes the empty record, so a
/// `size` of 3 is a legal record with no elements.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GcLandList {
    /// The lands, in the order the world recorded them.
    pub entries: Vec<GcLandListEntry>,
}

impl GcLandList {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_LAND_LIST.value()
    }

    /// Build the record for a list of lands.
    #[must_use]
    pub fn new(entries: Vec<GcLandListEntry>) -> Self {
        Self { entries }
    }

    /// The whole record's length on the wire, which is what `size` holds.
    #[must_use]
    pub fn wire_size(&self) -> usize {
        GC_LAND_LIST_WIRE_SIZE + self.entries.len() * GcLandListEntry::WIRE_SIZE
    }

    /// Appends the whole packed record to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`GcLandListError::TooManyEntries`] when the list is long
    /// enough that `size` would not fit its `WORD` field.
    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), GcLandListError> {
        let size =
            u16::try_from(self.wire_size()).map_err(|_| GcLandListError::TooManyEntries {
                count: self.entries.len(),
            })?;
        out.push(Self::header());
        out.extend_from_slice(&size.to_le_bytes());
        for entry in &self.entries {
            entry.encode_into(out);
        }
        Ok(())
    }

    /// Decodes the whole packed record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcLandListError::Truncated`] when the buffer is shorter than
    /// the record's own `size`, [`GcLandListError::Header`] when the leading
    /// byte is not [`HEADER_GC_LAND_LIST`], and [`GcLandListError::Size`] when
    /// `size` is not `3 + 24 * count`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcLandListError> {
        if bytes.len() < GC_LAND_LIST_WIRE_SIZE {
            return Err(GcLandListError::Truncated {
                context: "GcLandList",
                needed: GC_LAND_LIST_WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header() {
            return Err(GcLandListError::Header {
                expected: Self::header(),
                actual: bytes[0],
            });
        }
        let size = usize::from(u16::from_le_bytes([bytes[1], bytes[2]]));
        let Some(rest) = size.checked_sub(GC_LAND_LIST_WIRE_SIZE) else {
            return Err(GcLandListError::Size { size });
        };
        if rest % GcLandListEntry::WIRE_SIZE != 0 {
            return Err(GcLandListError::Size { size });
        }
        if bytes.len() < size {
            return Err(GcLandListError::Truncated {
                context: "GcLandList",
                needed: size,
                actual: bytes.len(),
            });
        }
        let count = rest / GcLandListEntry::WIRE_SIZE;
        let mut entries = Vec::with_capacity(count);
        for index in 0..count {
            let at = GC_LAND_LIST_WIRE_SIZE + index * GcLandListEntry::WIRE_SIZE;
            entries.push(GcLandListEntry::decode(&bytes[at..])?);
        }
        Ok(Self { entries })
    }
}

/// What a [`GcLandList`] decode or encode can report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcLandListError {
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
    /// `size` is not `3 + 24 * count`.
    Size {
        /// The `size` the record carried.
        size: usize,
    },
    /// The list is too long for the `WORD` size field.
    TooManyEntries {
        /// The number of entries that was refused.
        count: usize,
    },
}

impl fmt::Display for GcLandListError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                context,
                needed,
                actual,
            } => write!(f, "{context}: need {needed} bytes, buffer held {actual}"),
            Self::Header { expected, actual } => write!(
                f,
                "GcLandList: header {actual:#04x}, expected {expected:#04x}"
            ),
            Self::Size { size } => write!(
                f,
                "GcLandList: size {size} is not {GC_LAND_LIST_WIRE_SIZE} plus a run of \
                 {GC_LAND_LIST_ELEMENT_SIZE}-byte elements"
            ),
            Self::TooManyEntries { count } => {
                write!(f, "GcLandList: {count} entries do not fit the 16-bit size")
            }
        }
    }
}

impl std::error::Error for GcLandListError {}

#[cfg(test)]
mod tests {
    use super::*;

    const MEASURED_HEADER: usize = 3;
    const MEASURED_ELEMENT: usize = 24;

    /// The golden bytes, taken from the source field order and not from the encoder.
    fn golden(entries: &[GcLandListEntry]) -> Vec<u8> {
        let mut want = vec![130];
        let total = u16::try_from(MEASURED_HEADER + entries.len() * MEASURED_ELEMENT)
            .expect("a test view is short");
        want.extend_from_slice(&total.to_le_bytes());
        for entry in entries {
            want.extend_from_slice(&entry.id.to_le_bytes());
            want.extend_from_slice(&entry.x.to_le_bytes());
            want.extend_from_slice(&entry.y.to_le_bytes());
            want.extend_from_slice(&entry.width.to_le_bytes());
            want.extend_from_slice(&entry.height.to_le_bytes());
            want.extend_from_slice(&entry.guild_id.to_le_bytes());
        }
        want
    }

    fn sample() -> GcLandList {
        GcLandList::new(vec![
            GcLandListEntry::new(0x0102_0304, 0x0506_0708, -0x0102_0304, 3000, 3000, 7),
            GcLandListEntry::new(0x0a0b_0c0d, 0x1112_1314, 0x1516_1718, 100, 200, 0),
        ])
    }

    #[test]
    fn the_widths_are_the_measured_i386_ones() {
        assert_eq!(GC_LAND_LIST_WIRE_SIZE, MEASURED_HEADER);
        assert_eq!(GcLandListEntry::WIRE_SIZE, MEASURED_ELEMENT);
        assert_eq!(MEASURED_ELEMENT, 4 + 4 * 4 + 4);
        assert_eq!(GcLandList::new(Vec::new()).wire_size(), 3);
        assert_eq!(sample().wire_size(), 51);
    }

    #[test]
    fn encodes_the_golden_bytes() {
        let mut got = Vec::new();
        sample().encode_into(&mut got).expect("fits");
        assert_eq!(got, golden(&sample().entries));
    }

    #[test]
    fn decodes_the_golden_bytes() {
        let bytes = golden(&sample().entries);
        assert_eq!(GcLandList::decode(&bytes).expect("decodes"), sample());
    }

    #[test]
    fn the_empty_list_is_three_bytes() {
        let mut got = Vec::new();
        GcLandList::new(Vec::new())
            .encode_into(&mut got)
            .expect("fits");
        assert_eq!(got, vec![130, 3, 0]);
        assert_eq!(
            GcLandList::decode(&got).expect("decodes"),
            GcLandList::new(Vec::new())
        );
    }

    #[test]
    fn a_ragged_size_is_malformed() {
        assert_eq!(
            GcLandList::decode(&[130, 4, 0, 0]),
            Err(GcLandListError::Size { size: 4 })
        );
        assert_eq!(
            GcLandList::decode(&[130, 2, 0]),
            Err(GcLandListError::Size { size: 2 })
        );
    }

    #[test]
    fn a_short_buffer_is_truncated() {
        assert_eq!(
            GcLandList::decode(&[130, 27, 0][..2]),
            Err(GcLandListError::Truncated {
                context: "GcLandList",
                needed: 3,
                actual: 2,
            })
        );
        assert_eq!(
            GcLandList::decode(&[130, 27, 0]),
            Err(GcLandListError::Truncated {
                context: "GcLandList",
                needed: 27,
                actual: 3,
            })
        );
    }

    #[test]
    fn a_wrong_header_is_refused() {
        assert_eq!(
            GcLandList::decode(&[131, 3, 0]),
            Err(GcLandListError::Header {
                expected: 130,
                actual: 131,
            })
        );
    }

    #[test]
    fn an_overflowing_list_is_refused() {
        let list = GcLandList::new(vec![GcLandListEntry::new(1, 0, 0, 0, 0, 0); 3000]);
        assert_eq!(
            list.encode_into(&mut Vec::new()),
            Err(GcLandListError::TooManyEntries { count: 3000 })
        );
    }
}
