//! The three position records a character movement broadcast needs: a sit or stand,
//! a batch of synced positions, and a walk-mode change.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:1665-1670`:
//!
//! ```ignore
//! struct packet_position
//! {
//!     BYTE  header;
//!     DWORD vid;
//!     BYTE  position;
//! };
//! ```
//!
//! `server/server/game/packet.h:1722-1735`:
//!
//! ```ignore
//! typedef struct packet_sync_position_element
//! {
//!     DWORD dwVID;
//!     long  lX;
//!     long  lY;
//! } TPacketGCSyncPositionElement;
//!
//! typedef struct packet_sync_position
//! {
//!     BYTE bHeader;
//!     WORD wSize;
//! } TPacketGCSyncPosition;
//! ```
//!
//! `server/server/game/packet.h:2383-2388`:
//!
//! ```ignore
//! typedef struct SPacketGCWalkMode
//! {
//!     BYTE   header;
//!     DWORD  vid;
//!     BYTE   mode;
//! } TPacketGCWalkMode;
//! ```
//!
//! # Widths
//!
//! `packet_position` and `TPacketGCWalkMode` are both 1 + 4 + 1 = 6 bytes, and
//! `TPacketGCSyncPositionElement` is 4 + 4 + 4 = 12 bytes, leaving a 3-byte
//! `TPacketGCSyncPosition` head. Those are hand sums. The legacy target is 32-bit
//! x86 (`server/server/premake5.lua:12`), where `DWORD` is 4 and `long` is 4, and
//! every one of these structs sits under `#pragma pack(1)` (which the enclosing
//! `packet.h` applies at the top of the file), so the sums hold. This machine has
//! no `i686-linux-gnu-g++-12`, so the widths were not re-measured by a compiled
//! probe; they are pinned instead by the golden-byte tests in this module, which
//! use distinct byte halves so an endianness slip cannot pass.

#![warn(missing_docs)]

use std::fmt;

use crate::gc_inventory::{
    HEADER_GC_CHARACTER_POSITION, HEADER_GC_SYNC_POSITION, HEADER_GC_WALK_MODE,
};

/// `sizeof(struct packet_position)`.
pub const GC_CHARACTER_POSITION_WIRE_SIZE: usize = 6;

/// `sizeof(TPacketGCSyncPosition)`: the header and the size word, with no elements.
pub const GC_SYNC_POSITION_HEAD_WIRE_SIZE: usize = 3;

/// `sizeof(TPacketGCSyncPositionElement)` on the 32-bit legacy target, where
/// `long` is 4 bytes.
pub const GC_SYNC_POSITION_ELEMENT_WIRE_SIZE: usize = 12;

/// `sizeof(SPacketGCWalkMode)`.
pub const GC_WALK_MODE_WIRE_SIZE: usize = 6;

/// The most elements one `TPacketGCSyncPosition` can carry.
///
/// `wSize` is a `WORD`, so a record can describe at most `(65535 - 3) / 12` = 5461
/// elements. The Rewrite caps far lower, at the legacy handler's own limit: more
/// than 16 elements logs and truncates, because the close path in
/// `CInputMain::SyncPosition` (`server/server/game/input_main.cpp:2114-2121`) is
/// commented out.
pub const GC_SYNC_POSITION_MAX_ELEMENTS: usize = 16;

/// The bytes one of these records shares with the other two: a header, a VID, and a
/// trailing byte. They are not interchangeable on the wire, so there is no shared
/// codec; only this layout is common.
fn push_vid_and_byte(out: &mut Vec<u8>, header: u8, vid: u32, last: u8) {
    out.push(header);
    out.extend_from_slice(&vid.to_le_bytes());
    out.push(last);
}

/// `HEADER_GC_CHARACTER_POSITION` (43): a character sat down or stood up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcCharacterPosition {
    /// The character whose pose changed.
    pub vid: u32,
    /// One of the `POSITION_*` values.
    pub position: u8,
}

impl GcCharacterPosition {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_CHARACTER_POSITION.value()
    }

    /// A pose change for one character.
    #[must_use]
    pub const fn new(vid: u32, position: u8) -> Self {
        Self { vid, position }
    }

    /// The exact packed record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_CHARACTER_POSITION_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Appends the record to `out`. It cannot fail, so it returns nothing.
    pub fn encode_into(self, out: &mut Vec<u8>) {
        push_vid_and_byte(out, Self::header(), self.vid, self.position);
    }

    /// Read the record back.
    ///
    /// # Errors
    ///
    /// Returns [`GcPositionError::Truncated`] when the buffer is shorter than
    /// [`GC_CHARACTER_POSITION_WIRE_SIZE`], and [`GcPositionError::Header`] when the
    /// leading byte is not 43.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcPositionError> {
        let raw = fixed(
            bytes,
            GC_CHARACTER_POSITION_WIRE_SIZE,
            "GcCharacterPosition",
        )?;
        if raw[0] != Self::header() {
            return Err(GcPositionError::Header {
                context: "GcCharacterPosition",
                expected: Self::header(),
                actual: raw[0],
            });
        }
        Ok(Self {
            vid: vid_at(raw, 1),
            position: raw[5],
        })
    }
}

/// `HEADER_GC_WALK_MODE` (111): a character changed between walking and running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcWalkMode {
    /// The character whose mode changed.
    pub vid: u32,
    /// The legacy walk-mode byte, relayed unchanged.
    pub mode: u8,
}

impl GcWalkMode {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_WALK_MODE.value()
    }

    /// A mode change for one character.
    #[must_use]
    pub const fn new(vid: u32, mode: u8) -> Self {
        Self { vid, mode }
    }

    /// The exact packed record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_WALK_MODE_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Appends the record to `out`. It cannot fail, so it returns nothing.
    pub fn encode_into(self, out: &mut Vec<u8>) {
        push_vid_and_byte(out, Self::header(), self.vid, self.mode);
    }

    /// Read the record back.
    ///
    /// # Errors
    ///
    /// Returns [`GcPositionError::Truncated`] when the buffer is shorter than
    /// [`GC_WALK_MODE_WIRE_SIZE`], and [`GcPositionError::Header`] when the leading
    /// byte is not 111.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcPositionError> {
        let raw = fixed(bytes, GC_WALK_MODE_WIRE_SIZE, "GcWalkMode")?;
        if raw[0] != Self::header() {
            return Err(GcPositionError::Header {
                context: "GcWalkMode",
                expected: Self::header(),
                actual: raw[0],
            });
        }
        Ok(Self {
            vid: vid_at(raw, 1),
            mode: raw[5],
        })
    }
}

/// One `TPacketGCSyncPositionElement`: a VID and where that character is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcSyncPositionElement {
    /// The character being placed.
    pub vid: u32,
    /// The map x coordinate, as a legacy `long`.
    pub x: i32,
    /// The map y coordinate, as a legacy `long`.
    pub y: i32,
}

impl GcSyncPositionElement {
    /// A position for one character.
    #[must_use]
    pub const fn new(vid: u32, x: i32, y: i32) -> Self {
        Self { vid, x, y }
    }

    /// The element's 12 packed bytes.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_SYNC_POSITION_ELEMENT_WIRE_SIZE);
        out.extend_from_slice(&self.vid.to_le_bytes());
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
        out
    }

    /// Read one element back.
    ///
    /// # Errors
    ///
    /// Returns [`GcPositionError::Truncated`] when fewer than
    /// [`GC_SYNC_POSITION_ELEMENT_WIRE_SIZE`] bytes are available.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcPositionError> {
        let raw = fixed(
            bytes,
            GC_SYNC_POSITION_ELEMENT_WIRE_SIZE,
            "GcSyncPositionElement",
        )?;
        Ok(Self {
            vid: vid_at(raw, 0),
            x: i32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]),
            y: i32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]),
        })
    }
}

/// `HEADER_GC_SYNC_POSITION` (5): a batch of freshly placed characters.
///
/// `wSize` is the whole record length, header and size word included, which is what
/// `CInputMain::SyncPosition` writes (`input_main.cpp:2159`). An empty batch is
/// never sent: the legacy producer compares the built size against
/// `sizeof(TPacketGCSyncPosition)` and only sends when it grew.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcSyncPosition {
    /// The elements, in the order they will be placed.
    pub elements: Vec<GcSyncPositionElement>,
}

impl GcSyncPosition {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_SYNC_POSITION.value()
    }

    /// A batch of positions.
    ///
    /// # Errors
    ///
    /// Returns [`GcPositionError::TooManyElements`] above
    /// [`GC_SYNC_POSITION_MAX_ELEMENTS`], the legacy handler's own truncation point.
    pub fn new(elements: Vec<GcSyncPositionElement>) -> Result<Self, GcPositionError> {
        if elements.len() > GC_SYNC_POSITION_MAX_ELEMENTS {
            return Err(GcPositionError::TooManyElements {
                count: elements.len(),
                maximum: GC_SYNC_POSITION_MAX_ELEMENTS,
            });
        }
        Ok(Self { elements })
    }

    /// The whole record's length, which is what `wSize` carries.
    #[must_use]
    pub fn wire_size(&self) -> usize {
        GC_SYNC_POSITION_HEAD_WIRE_SIZE + self.elements.len() * GC_SYNC_POSITION_ELEMENT_WIRE_SIZE
    }

    /// The exact packed record, or `None` for an empty batch.
    ///
    /// An empty batch has nothing to place, and legacy never sends one, so returning
    /// `None` here keeps the caller from inventing a record the client would render
    /// as a bare header.
    #[must_use]
    pub fn encode(&self) -> Option<Vec<u8>> {
        if self.elements.is_empty() {
            return None;
        }
        let mut out = Vec::with_capacity(self.wire_size());
        out.push(Self::header());
        let size = u16::try_from(self.wire_size()).ok()?;
        out.extend_from_slice(&size.to_le_bytes());
        for element in &self.elements {
            out.extend_from_slice(&element.encode());
        }
        Some(out)
    }

    /// Read a batch back.
    ///
    /// # Errors
    ///
    /// Returns [`GcPositionError::Truncated`] when the buffer is shorter than the
    /// declared `wSize`, [`GcPositionError::Size`] when `wSize` is below the head, and
    /// [`GcPositionError::Misaligned`] when the extra length is not a whole number of
    /// elements. That last one is the case the legacy handler logs and consumes
    /// without closing, so it is a typed error here rather than a silent guess.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcPositionError> {
        if bytes.len() < GC_SYNC_POSITION_HEAD_WIRE_SIZE {
            return Err(GcPositionError::Truncated {
                context: "GcSyncPosition",
                needed: GC_SYNC_POSITION_HEAD_WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header() {
            return Err(GcPositionError::Header {
                context: "GcSyncPosition",
                expected: Self::header(),
                actual: bytes[0],
            });
        }
        let size = usize::from(u16::from_le_bytes([bytes[1], bytes[2]]));
        if size < GC_SYNC_POSITION_HEAD_WIRE_SIZE {
            return Err(GcPositionError::Size { size });
        }
        if bytes.len() < size {
            return Err(GcPositionError::Truncated {
                context: "GcSyncPosition",
                needed: size,
                actual: bytes.len(),
            });
        }
        let extra = size - GC_SYNC_POSITION_HEAD_WIRE_SIZE;
        if extra % GC_SYNC_POSITION_ELEMENT_WIRE_SIZE != 0 {
            return Err(GcPositionError::Misaligned { extra });
        }
        let count = extra / GC_SYNC_POSITION_ELEMENT_WIRE_SIZE;
        let mut elements = Vec::with_capacity(count);
        for index in 0..count {
            let start =
                GC_SYNC_POSITION_HEAD_WIRE_SIZE + index * GC_SYNC_POSITION_ELEMENT_WIRE_SIZE;
            elements.push(GcSyncPositionElement::decode(&bytes[start..start + 12])?);
        }
        Ok(Self { elements })
    }
}

fn fixed<'a>(
    bytes: &'a [u8],
    needed: usize,
    context: &'static str,
) -> Result<&'a [u8], GcPositionError> {
    if bytes.len() < needed {
        return Err(GcPositionError::Truncated {
            context,
            needed,
            actual: bytes.len(),
        });
    }
    Ok(&bytes[..needed])
}

fn vid_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// What a position-record decode or encode can report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcPositionError {
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
    /// `wSize` is smaller than the fixed head.
    Size {
        /// The `wSize` the record carried.
        size: usize,
    },
    /// The extra length is not a whole number of 12-byte elements.
    Misaligned {
        /// The bytes found after the head.
        extra: usize,
    },
    /// More elements than the legacy handler will carry.
    TooManyElements {
        /// How many elements were offered.
        count: usize,
        /// The most the legacy handler carries.
        maximum: usize,
    },
}

impl fmt::Display for GcPositionError {
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
                "{context}: header {expected} expected, buffer held {actual}"
            ),
            Self::Size { size } => write!(f, "GcSyncPosition: wSize {size} is under the head"),
            Self::Misaligned { extra } => write!(
                f,
                "GcSyncPosition: {extra} bytes after the head is not a whole number of elements"
            ),
            Self::TooManyElements { count, maximum } => {
                write!(
                    f,
                    "GcSyncPosition: {count} elements offered, the most is {maximum}"
                )
            }
        }
    }
}

impl std::error::Error for GcPositionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_position::{POSITION_GENERAL, POSITION_SITTING_CHAIR, POSITION_SITTING_GROUND};
    use crate::gc_inventory::{
        HEADER_GC_CHARACTER_POSITION, HEADER_GC_SYNC_POSITION, HEADER_GC_WALK_MODE,
    };

    /// Distinct byte halves on every multi-byte field, so a big-endian read cannot
    /// pass by byte symmetry.
    const ELEMENT_BYTES: [u8; 12] = [
        0x13, 0x57, 0x9b, 0xdf, 0xa2, 0xb4, 0xd6, 0xf8, 0x0c, 0x1e, 0x35, 0x7b,
    ];
    const VID: u32 = 0xdf9b_5713;
    const X: i32 = -0x07_29_4b_5e;
    const Y: i32 = 0x7b_35_1e_0c;

    #[test]
    fn the_headers_are_the_legacy_bytes() {
        assert_eq!(HEADER_GC_CHARACTER_POSITION.value(), 0x2b);
        assert_eq!(HEADER_GC_SYNC_POSITION.value(), 0x05);
        assert_eq!(HEADER_GC_WALK_MODE.value(), 0x6f);
        assert_eq!(GcCharacterPosition::header(), 0x2b);
        assert_eq!(GcWalkMode::header(), 0x6f);
        assert_eq!(GcSyncPosition::header(), 0x05);
    }

    #[test]
    fn the_widths_are_the_legacy_sums() {
        assert_eq!(GC_CHARACTER_POSITION_WIRE_SIZE, 6);
        assert_eq!(GC_WALK_MODE_WIRE_SIZE, 6);
        assert_eq!(GC_SYNC_POSITION_HEAD_WIRE_SIZE, 3);
        assert_eq!(GC_SYNC_POSITION_ELEMENT_WIRE_SIZE, 12);
        assert_eq!(GcCharacterPosition::new(1, 2).encode().len(), 6);
        assert_eq!(GcWalkMode::new(1, 2).encode().len(), 6);
    }

    #[test]
    fn the_pose_golden_bytes_are_the_legacy_field_order() {
        assert_eq!(
            GcCharacterPosition::new(VID, POSITION_SITTING_GROUND).encode(),
            vec![0x2b, 0x13, 0x57, 0x9b, 0xdf, 0x02],
            "header, then the VID low byte first, then the pose",
        );
    }

    #[test]
    fn the_walk_mode_golden_bytes_are_the_legacy_field_order() {
        assert_eq!(
            GcWalkMode::new(VID, 0xa7).encode(),
            vec![0x6f, 0x13, 0x57, 0x9b, 0xdf, 0xa7],
        );
    }

    #[test]
    fn the_sync_element_golden_bytes_are_the_legacy_field_order() {
        assert_eq!(
            GcSyncPositionElement::new(VID, X, Y).encode(),
            ELEMENT_BYTES.to_vec(),
            "VID, then x, then y, each little-endian",
        );
    }

    #[test]
    fn a_one_element_batch_golden_bytes_carry_the_whole_length() {
        let batch = GcSyncPosition::new(vec![GcSyncPositionElement::new(VID, X, Y)])
            .expect("one element is fine");
        let mut want = vec![0x05, 0x0f, 0x00];
        want.extend_from_slice(&ELEMENT_BYTES);
        assert_eq!(batch.encode().expect("a non-empty batch encodes"), want);
        assert_eq!(batch.wire_size(), 15);
    }

    #[test]
    fn an_empty_batch_is_never_sent() {
        let batch = GcSyncPosition::new(Vec::new()).expect("an empty batch is legal to build");
        assert_eq!(
            batch.encode(),
            None,
            "legacy sends nothing for zero elements"
        );
        assert_eq!(batch.wire_size(), 3);
    }

    #[test]
    fn a_pose_record_round_trips() {
        for position in 0..=u8::MAX {
            let original = GcCharacterPosition::new(VID, position);
            assert_eq!(
                GcCharacterPosition::decode(&original.encode()).expect("round trip"),
                original
            );
        }
    }

    #[test]
    fn a_walk_mode_record_round_trips() {
        for mode in 0..=u8::MAX {
            let original = GcWalkMode::new(VID, mode);
            assert_eq!(
                GcWalkMode::decode(&original.encode()).expect("round trip"),
                original
            );
        }
    }

    #[test]
    fn a_batch_round_trips_in_order() {
        let elements = vec![
            GcSyncPositionElement::new(0x1111_2222, -1, 1),
            GcSyncPositionElement::new(0x3333_4444, i32::MIN, i32::MAX),
            GcSyncPositionElement::new(0x5555_6666, 0, 0),
        ];
        let batch = GcSyncPosition::new(elements.clone()).expect("three elements are fine");
        let bytes = batch.encode().expect("a non-empty batch encodes");
        assert_eq!(GcSyncPosition::decode(&bytes).expect("round trip"), batch);
        assert_eq!(
            GcSyncPosition::decode(&bytes).expect("round trip").elements,
            elements
        );
    }

    #[test]
    fn a_batch_beyond_the_legacy_limit_is_refused() {
        let elements = (0..=GC_SYNC_POSITION_MAX_ELEMENTS)
            .map(|index| {
                GcSyncPositionElement::new(u32::try_from(index).expect("16 fits a u32"), 0, 0)
            })
            .collect();
        let error = GcSyncPosition::new(elements).expect_err("17 elements is over the limit");
        assert!(
            matches!(
                error,
                GcPositionError::TooManyElements {
                    count: 17,
                    maximum: 16
                }
            ),
            "{error}"
        );
    }

    #[test]
    fn exactly_sixteen_elements_are_accepted() {
        let elements = (0..GC_SYNC_POSITION_MAX_ELEMENTS)
            .map(|index| {
                GcSyncPositionElement::new(u32::try_from(index).expect("16 fits a u32"), 0, 0)
            })
            .collect();
        let batch = GcSyncPosition::new(elements).expect("16 is the limit, not over it");
        assert_eq!(batch.wire_size(), 3 + 16 * 12);
        assert_eq!(batch.encode().expect("encodes").len(), 195);
    }

    #[test]
    fn a_misaligned_extra_length_is_a_typed_error() {
        let mut bytes = vec![0x05, 0x0c, 0x00];
        bytes.extend_from_slice(&ELEMENT_BYTES[..9]);
        let error = GcSyncPosition::decode(&bytes).expect_err("9 bytes is not a whole element");
        assert!(
            matches!(error, GcPositionError::Misaligned { extra: 9 }),
            "{error}"
        );
    }

    #[test]
    fn a_size_under_the_head_is_refused() {
        let error = GcSyncPosition::decode(&[0x05, 0x02, 0x00]).expect_err("2 is under the head");
        assert!(
            matches!(error, GcPositionError::Size { size: 2 }),
            "{error}"
        );
    }

    #[test]
    fn a_batch_shorter_than_its_declared_size_is_refused() {
        let error = GcSyncPosition::decode(&[0x05, 0x0f, 0x00]).expect_err("no elements arrived");
        assert!(
            matches!(error, GcPositionError::Truncated { actual: 3, .. }),
            "{error}"
        );
    }

    #[test]
    fn a_head_shorter_than_three_bytes_is_refused() {
        let error = GcSyncPosition::decode(&[0x05, 0x0f]).expect_err("two bytes is short");
        assert!(
            matches!(error, GcPositionError::Truncated { actual: 2, .. }),
            "{error}"
        );
    }

    #[test]
    fn a_wrong_header_is_refused_on_every_record() {
        let mut pose = GcCharacterPosition::new(1, 2).encode();
        pose[0] = 0x2c;
        let mut walk = GcWalkMode::new(1, 2).encode();
        walk[0] = 0x70;
        let mut batch = vec![0x06, 0x0f, 0x00];
        batch.extend_from_slice(&ELEMENT_BYTES);

        for error in [
            GcCharacterPosition::decode(&pose).expect_err("a wrong pose header"),
            GcWalkMode::decode(&walk).expect_err("a wrong walk-mode header"),
            GcSyncPosition::decode(&batch).expect_err("a wrong sync header"),
        ] {
            assert!(matches!(error, GcPositionError::Header { .. }), "{error}");
        }
    }

    #[test]
    fn a_short_pose_buffer_is_refused_rather_than_read_past() {
        for length in 0..GC_CHARACTER_POSITION_WIRE_SIZE {
            let bytes = vec![0x2b; length];
            let error = GcCharacterPosition::decode(&bytes).expect_err("short");
            assert!(
                matches!(error, GcPositionError::Truncated { actual, .. } if actual == length),
                "{length} bytes: {error}"
            );
        }
    }

    #[test]
    fn a_long_pose_buffer_keeps_only_the_legacy_width() {
        let mut bytes = GcCharacterPosition::new(VID, 7).encode();
        bytes.push(0xaa);
        let decoded = GcCharacterPosition::decode(&bytes).expect("the record is fixed size");
        assert_eq!(
            decoded.position, 7,
            "the trailing byte is not part of the record"
        );
    }

    #[test]
    fn the_pose_values_are_the_legacy_ones() {
        assert_eq!(POSITION_GENERAL, 0);
        assert_eq!(POSITION_SITTING_CHAIR, 1);
        assert_eq!(POSITION_SITTING_GROUND, 2);
    }

    #[test]
    fn the_error_text_names_the_record_and_the_reason() {
        let error = GcCharacterPosition::decode(&[]).expect_err("empty");
        let text = error.to_string();
        assert!(text.contains("GcCharacterPosition"), "{text}");
        let error = GcSyncPosition::decode(&[0x05, 0x0c, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0])
            .expect_err("a nine-byte tail");
        assert!(error.to_string().contains("whole number"), "{error}");
    }

    #[test]
    fn a_mutation_that_swaps_the_vid_half_order_is_killed() {
        // Reading the VID big-endian would give 0x13579bdf; the pinned bytes rule that out.
        let decoded = GcCharacterPosition::decode(&GcCharacterPosition::new(VID, 0).encode())
            .expect("decodes");
        assert_eq!(decoded.vid, 0xdf9b_5713);
        assert_ne!(decoded.vid, 0x1357_9bdf);
    }

    #[test]
    fn a_mutation_that_swaps_x_and_y_is_killed() {
        let element = GcSyncPositionElement::new(VID, X, Y);
        let bytes = element.encode();
        let decoded = GcSyncPositionElement::decode(&bytes).expect("decodes");
        assert_eq!((decoded.x, decoded.y), (X, Y));
        let swapped = GcSyncPositionElement::new(VID, Y, X);
        assert_ne!(bytes, swapped.encode(), "x and y are not interchangeable");
    }

    #[test]
    fn a_negative_coordinate_keeps_its_two_complement_bytes() {
        let element = GcSyncPositionElement::new(VID, -1, i32::MIN);
        let bytes = element.encode();
        assert_eq!(&bytes[4..8], &0xffff_ffff_u32.to_le_bytes());
        assert_eq!(&bytes[8..12], &0x8000_0000_u32.to_le_bytes());
        let decoded = GcSyncPositionElement::decode(&bytes).expect("decodes");
        assert_eq!((decoded.x, decoded.y), (-1, i32::MIN));
    }
}
