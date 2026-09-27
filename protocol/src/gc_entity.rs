//! `HEADER_GC_ENTITY` (249): the characters the client is told to place.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:3364-3379`:
//!
//! ```ignore
//! enum EntityHeader { HEADER_GC_ENTITY = 249, };
//!
//! using TPacketGCEntity = struct SPacketGCEntity { BYTE bHeader; WORD wSize; };
//! using TPacketEntityInfo = struct SPacketEntityInfo
//! {
//!     DWORD dwVID;
//!     DWORD dwRaceVNum;
//!     WORD  wPart[CHR_EQUIPPART_NUM];
//!     LONG  xPos, yPos;
//! };
//! ```
//!
//! `wSize` counts the three header bytes **and** every following
//! [`GcEntityInfo`]. The producer is `SECTREE_MANAGER::SendEntity`
//! (`server/server/game/sectree_manager.cpp:1697-1751`), which writes the header
//! alone when the list is empty and splits the header and the body into a
//! `BufferedPacket` and a `Packet` otherwise.
//!
//! # Measured width, not a hand sum
//!
//! The legacy target is 32-bit x86, so `DWORD` is 4 bytes and `LONG` is 4 bytes,
//! and `CHR_EQUIPPART_NUM` is 6 because `__SASH_SYSTEM__` and `__AURA_SYSTEM__`
//! are both defined in `server/server/common/prodomodefines.h`. That gives 3 for
//! the header and 4 + 4 + 12 + 4 + 4 = 28 for one element. Both numbers were
//! measured by compiling the verbatim struct bodies with `g++ -m32` under
//! `static_assert`, alongside the repository's known-width controls. The probe
//! lives in `.scratch/ledger187/root-probe/`.
//!
//! # Scope
//!
//! A record codec only. It does not place a character in a world, and it does
//! not know which characters belong in a Channel's world.

use std::fmt;

use crate::gc_inventory::HEADER_GC_ENTITY;

/// `sizeof(TPacketGCEntity)`: the header byte and the total-size word.
pub const GC_ENTITY_WIRE_SIZE: usize = 3;

/// `sizeof(TPacketEntityInfo)`: a VID, a race vnum, six parts, and two positions.
pub const GC_ENTITY_INFO_WIRE_SIZE: usize = 28;

/// `CHR_EQUIPPART_NUM` on this deployment: armour, weapon, head, hair, sash, aura
/// (`server/server/game/packet.h:872-882`).
pub const ENTITY_PART_NUM: usize = 6;

/// Index of the armour part in [`GcEntityInfo::parts`].
pub const ENTITY_PART_ARMOR: usize = 0;
/// Index of the weapon part in [`GcEntityInfo::parts`].
pub const ENTITY_PART_WEAPON: usize = 1;
/// Index of the head part in [`GcEntityInfo::parts`].
pub const ENTITY_PART_HEAD: usize = 2;
/// Index of the hair part in [`GcEntityInfo::parts`].
pub const ENTITY_PART_HAIR: usize = 3;
/// Index of the sash part in [`GcEntityInfo::parts`].
pub const ENTITY_PART_SASH: usize = 4;
/// Index of the aura part in [`GcEntityInfo::parts`].
pub const ENTITY_PART_AURA: usize = 5;

/// One character inside a [`GcEntity`] list.
///
/// Legacy `TPacketEntityInfo`. The producer fills six of the parts and leaves
/// the rest at whatever `= {}` put there, which is zero
/// (`sectree_manager.cpp:1712-1727`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcEntityInfo {
    /// `dwVID`.
    pub vid: u32,
    /// `dwRaceVNum`.
    pub race_vnum: u32,
    /// `wPart`, in `ECharacterEquipmentPart` order.
    pub parts: [u16; ENTITY_PART_NUM],
    /// `xPos`.
    pub x: i32,
    /// `yPos`.
    pub y: i32,
}

impl GcEntityInfo {
    /// Number of bytes one element occupies on the wire.
    pub const WIRE_SIZE: usize = GC_ENTITY_INFO_WIRE_SIZE;

    /// Build an element with every field set explicitly.
    #[must_use]
    pub const fn new(
        vid: u32,
        race_vnum: u32,
        parts: [u16; ENTITY_PART_NUM],
        x: i32,
        y: i32,
    ) -> Self {
        Self {
            vid,
            race_vnum,
            parts,
            x,
            y,
        }
    }

    /// Appends the 28 packed bytes to `out`, without a header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.vid.to_le_bytes());
        out.extend_from_slice(&self.race_vnum.to_le_bytes());
        for part in self.parts {
            out.extend_from_slice(&part.to_le_bytes());
        }
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
    }

    /// Reads the 28 packed bytes from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcEntityError::Truncated`] unless `bytes` holds the full
    /// [`GcEntityInfo::WIRE_SIZE`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcEntityError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcEntityError::Truncated {
                context: "GcEntityInfo",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        let mut parts = [0u16; ENTITY_PART_NUM];
        for (index, part) in parts.iter_mut().enumerate() {
            let at = 8 + index * 2;
            *part = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        }
        Ok(Self {
            vid: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            race_vnum: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            parts,
            x: i32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
            y: i32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
        })
    }
}

/// `HEADER_GC_ENTITY` (249): the characters a client should place on its map.
///
/// The record is three fixed bytes and then zero or more [`GcEntityInfo`]
/// elements. `wSize` is the whole record, so a decoder never has to guess how
/// many elements follow: it reads the size and then exactly that many bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GcEntity {
    /// The characters, in the order the producer wrote them.
    pub entities: Vec<GcEntityInfo>,
}

impl GcEntity {
    /// Number of bytes the fixed part of this record occupies.
    pub const HEADER_SIZE: usize = GC_ENTITY_WIRE_SIZE;

    /// Builds the record for a list of characters.
    #[must_use]
    pub fn new(entities: Vec<GcEntityInfo>) -> Self {
        Self { entities }
    }

    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_ENTITY.value()
    }

    /// The whole record's length on the wire, which is what `wSize` holds.
    #[must_use]
    pub fn wire_size(&self) -> usize {
        Self::HEADER_SIZE + self.entities.len() * GcEntityInfo::WIRE_SIZE
    }

    /// Appends the whole packed record to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`GcEntityError::TooManyEntities`] when the list is long enough
    /// that `wSize` would not fit the `WORD` the legacy field is. The legacy
    /// producer has no such check, but it also never has that many players; the
    /// Rewrite refuses rather than wrapping.
    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), GcEntityError> {
        let size = self.wire_size();
        let size =
            u16::try_from(size).map_err(|_| GcEntityError::TooManyEntities { wanted: size })?;
        out.push(Self::header());
        out.extend_from_slice(&size.to_le_bytes());
        for entity in &self.entities {
            entity.encode_into(out);
        }
        Ok(())
    }

    /// Decodes the whole packed record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcEntityError::Truncated`] when the buffer is shorter than the
    /// record's own `wSize`, [`GcEntityError::Header`] when the leading byte is
    /// not [`HEADER_GC_ENTITY`], and [`GcEntityError::Size`] when `wSize` is
    /// smaller than the three header bytes or is not `3 + 28 * count`. A legacy
    /// defect that produced a mismatched size is not reproduced as a partial
    /// read; it is reported.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcEntityError> {
        if bytes.len() < Self::HEADER_SIZE {
            return Err(GcEntityError::Truncated {
                context: "GcEntity",
                needed: Self::HEADER_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header() {
            return Err(GcEntityError::Header {
                context: "GcEntity",
                expected: Self::header(),
                actual: bytes[0],
            });
        }
        let size = usize::from(u16::from_le_bytes([bytes[1], bytes[2]]));
        let body = size
            .checked_sub(Self::HEADER_SIZE)
            .ok_or(GcEntityError::Size { size })?;
        if body % GcEntityInfo::WIRE_SIZE != 0 {
            return Err(GcEntityError::Size { size });
        }
        if bytes.len() < size {
            return Err(GcEntityError::Truncated {
                context: "GcEntity",
                needed: size,
                actual: bytes.len(),
            });
        }
        let count = body / GcEntityInfo::WIRE_SIZE;
        let mut entities = Vec::with_capacity(count);
        for index in 0..count {
            let at = Self::HEADER_SIZE + index * GcEntityInfo::WIRE_SIZE;
            entities.push(GcEntityInfo::decode(&bytes[at..])?);
        }
        Ok(Self { entities })
    }
}

/// What a [`GcEntity`] decode or encode can report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcEntityError {
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
        /// Which record was being decoded.
        context: &'static str,
        /// The header byte this record requires.
        expected: u8,
        /// The header byte that was present.
        actual: u8,
    },
    /// `wSize` is not `3 + 28 * count`.
    Size {
        /// The `wSize` the record carried.
        size: usize,
    },
    /// The list is too long for the `WORD` size field.
    TooManyEntities {
        /// The length the record would have needed.
        wanted: usize,
    },
}

impl fmt::Display for GcEntityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                context,
                needed,
                actual,
            } => write!(f, "{context}: need {needed} bytes, buffer held {actual}"),
            Self::Header {
                context,
                expected,
                actual,
            } => write!(
                f,
                "{context}: header {actual:#04x}, expected {expected:#04x}"
            ),
            Self::Size { size } => write!(
                f,
                "GcEntity: wSize {size} is not {GC_ENTITY_WIRE_SIZE} plus a whole number of \
                 {GC_ENTITY_INFO_WIRE_SIZE}-byte elements"
            ),
            Self::TooManyEntities { wanted } => {
                write!(
                    f,
                    "GcEntity: {wanted} bytes do not fit the 16-bit size field"
                )
            }
        }
    }
}

impl std::error::Error for GcEntityError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The widths the i386 probe measured, restated so a refactor cannot change
    /// them silently. See the module docs for the probe.
    const MEASURED_HEADER: usize = 3;
    const MEASURED_INFO: usize = 28;

    /// A character list with the fields set to values whose byte halves differ, so
    /// an endianness slip cannot cancel out.
    fn sample() -> GcEntity {
        GcEntity::new(vec![
            GcEntityInfo::new(
                0x0102_0304,
                0x0a0b_0c0d,
                [0x1111, 0x2222, 0x3333, 0x4444, 0x5555, 0x6666],
                0x0102_0305,
                -0x0102_0306,
            ),
            GcEntityInfo::new(
                0x0a0b_0c0d,
                0x0506_0708,
                [0x7171, 0x7272, 0x7373, 0x7474, 0x7575, 0x7676],
                -0x0102_0307,
                0x0f0f_0f0f,
            ),
        ])
    }

    /// The golden bytes, taken from the source field order and not from the encoder.
    fn golden() -> Vec<u8> {
        let mut want = vec![249, 59, 0];
        want.extend_from_slice(&0x0102_0304u32.to_le_bytes());
        want.extend_from_slice(&0x0a0b_0c0du32.to_le_bytes());
        for part in [0x1111u16, 0x2222, 0x3333, 0x4444, 0x5555, 0x6666] {
            want.extend_from_slice(&part.to_le_bytes());
        }
        want.extend_from_slice(&0x0102_0305i32.to_le_bytes());
        want.extend_from_slice(&(-0x0102_0306i32).to_le_bytes());
        want.extend_from_slice(&0x0a0b_0c0du32.to_le_bytes());
        want.extend_from_slice(&0x0506_0708u32.to_le_bytes());
        for part in [0x7171u16, 0x7272, 0x7373, 0x7474, 0x7575, 0x7676] {
            want.extend_from_slice(&part.to_le_bytes());
        }
        want.extend_from_slice(&(-0x0102_0307i32).to_le_bytes());
        want.extend_from_slice(&0x0f0f_0f0fu32.to_le_bytes());
        want
    }

    #[test]
    fn the_widths_are_the_measured_i386_ones() {
        assert_eq!(GcEntityInfo::WIRE_SIZE, MEASURED_INFO);
        assert_eq!(GcEntity::HEADER_SIZE, MEASURED_HEADER);
        assert_eq!(MEASURED_HEADER + 2 * MEASURED_INFO, 59);
    }

    #[test]
    fn the_part_indices_are_the_legacy_enumerators() {
        // `ECharacterEquipmentPart`, `server/server/game/packet.h:872-882`.
        assert_eq!(ENTITY_PART_ARMOR, 0);
        assert_eq!(ENTITY_PART_WEAPON, 1);
        assert_eq!(ENTITY_PART_HEAD, 2);
        assert_eq!(ENTITY_PART_HAIR, 3);
        assert_eq!(ENTITY_PART_SASH, 4);
        assert_eq!(ENTITY_PART_AURA, 5);
        assert_eq!(ENTITY_PART_NUM, 6);
    }

    #[test]
    fn encodes_the_golden_bytes() {
        let mut got = Vec::new();
        sample().encode_into(&mut got).expect("fits");
        assert_eq!(got, golden());
    }

    #[test]
    fn size_counts_the_header_and_every_element() {
        assert_eq!(GcEntity::new(Vec::new()).wire_size(), 3);
        assert_eq!(GcEntity::new(vec![sample().entities[0]]).wire_size(), 31);
        assert_eq!(sample().wire_size(), 59);
        let mut empty = Vec::new();
        GcEntity::new(Vec::new())
            .encode_into(&mut empty)
            .expect("fits");
        // Legacy writes the header alone when the list is empty.
        assert_eq!(empty, vec![249, 3, 0]);
    }

    #[test]
    fn round_trips_and_leaves_trailing_bytes_alone() {
        let mut bytes = golden();
        bytes.extend_from_slice(&[0xaa; 16]);
        assert_eq!(GcEntity::decode(&bytes).expect("reads"), sample());
    }

    #[test]
    fn rejects_a_foreign_header() {
        let mut bytes = golden();
        bytes[0] = 0xf8;
        assert!(matches!(
            GcEntity::decode(&bytes),
            Err(GcEntityError::Header { actual: 0xf8, .. })
        ));
    }

    #[test]
    fn rejects_a_buffer_shorter_than_the_record_claims() {
        let bytes = golden();
        for cut in 0..bytes.len() {
            assert!(
                matches!(
                    GcEntity::decode(&bytes[..cut]),
                    Err(GcEntityError::Truncated { .. })
                ),
                "a {cut}-byte prefix must not decode"
            );
        }
    }

    #[test]
    fn accepts_every_valid_size() {
        for count in 0..4usize {
            let chat = GcEntity::new(vec![sample().entities[0]; count]);
            let mut bytes = Vec::new();
            chat.encode_into(&mut bytes).expect("fits");
            assert_eq!(
                usize::from(u16::from_le_bytes([bytes[1], bytes[2]])),
                chat.wire_size()
            );
            assert_eq!(GcEntity::decode(&bytes).expect("reads"), chat);
        }
    }

    #[test]
    fn rejects_a_size_that_is_not_a_whole_number_of_elements() {
        for size in [0u16, 1, 2, 4, 30, 32, 58, 60, 88, 116] {
            let mut bytes = golden();
            bytes[1..3].copy_from_slice(&size.to_le_bytes());
            assert!(
                matches!(GcEntity::decode(&bytes), Err(GcEntityError::Size { size: got }) if got == usize::from(size)),
                "wSize {size} must be refused"
            );
        }
    }

    #[test]
    fn refuses_a_list_too_long_for_the_size_field() {
        let many =
            vec![GcEntityInfo::default(); usize::from(u16::MAX) / GcEntityInfo::WIRE_SIZE + 1];
        assert!(matches!(
            GcEntity::new(many).encode_into(&mut Vec::new()),
            Err(GcEntityError::TooManyEntities { .. })
        ));
    }

    #[test]
    fn an_element_decodes_on_its_own() {
        let bytes = &golden()[GcEntity::HEADER_SIZE..];
        assert_eq!(
            GcEntityInfo::decode(bytes).expect("reads"),
            sample().entities[0]
        );
        assert!(matches!(
            GcEntityInfo::decode(&bytes[..27]),
            Err(GcEntityError::Truncated {
                context: "GcEntityInfo",
                needed: 28,
                actual: 27
            })
        ));
    }
}
