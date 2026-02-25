//! The two item-refinement request records.
//!
//! | record | header | wire | declaration | handler |
//! |---|---|---|---|---|
//! | `Refine` | 96 `0x60` | 3 | `packet.h:2396` | `input_main.cpp:3315` |
//! | `Attr67Add` | 169 `0xa9` | 8 | `packet.h:3099` | `input_main.cpp:3489` |
//!
//! They share a module because they are the two records that ask the server to
//! consume player items and turn them into something else, and because both use
//! the same "header, optional sub-header, then data" idiom.
//!
//! # `Refine` is 3 bytes because the alternative is commented out
//!
//! ```cpp
//! typedef struct SPacketCGRefine
//! {
//!     BYTE header;
//! // #ifdef ENABLE_CUSTOM_INVENTORY
//! // WORD  pos;
//! // #else
//!     BYTE  pos;
//! // #endif
//!     BYTE type;
//! } TPacketCGRefine;
//! ```
//!
//! The `ENABLE_CUSTOM_INVENTORY` branch is **commented out**, not
//! `#ifdef`-ed: there is no matching `#ifdef` or `#endif`, just `//`. The active
//! record is therefore `1 + 1 + 1 = 3` with a **`BYTE` position**. The client at
//! `client/Client/UserInterface/Packet.h:963` declares the same `BYTE pos;` with
//! no trace of the comment, so both trees agree at 3.
//!
//! A 4-byte reading is the historical custom-inventory shape and is **not**
//! active. This is a different situation from `Exchange`, where the wide
//! variant is behind a real `#ifdef` that is genuinely enabled: here the wide
//! variant is dead text, so there is no profile to choose.
//!
//! `pos` is a `BYTE` and stays a `BYTE`. It is not widened to a `u16` for
//! convenience, and not narrowed: a 4-byte `pos` would move `type` and break the
//! record.
//!
//! # `Attr67Add` nests a 6-byte struct from another header
//!
//! ```cpp
//! typedef struct SPacketCGAttr67Add
//! {
//!     BYTE byHeader;
//!     BYTE bySubHeader;
//!     TAttr67AddData Attr67AddData;
//! } TPacketCGAttr67Add;
//! ```
//!
//! `TAttr67AddData` is not in `packet.h`. It is in
//! `server/server/common/tables.h:1740-1747`, and identically in
//! `client/Client/UserInterface/GameType.h:575-582`:
//!
//! ```cpp
//! typedef struct SAttr67AddData
//! {
//!     WORD wRegistItemPos;
//!     BYTE byMaterialCount;
//!     WORD wSupportItemPos;
//!     BYTE bySupportItemCount;
//! } TAttr67AddData;
//! ```
//!
//! `2 + 1 + 2 + 1 = 6`, so the record is `1 + 1 + 6 = 8`, which closes against
//! `Set(HEADER_CG_ATTR67_ADD, sizeof(TPacketCGAttr67Add), "Attr67Add")`.
//!
//! **The nested struct needs no packing to be 6 bytes.** Its natural alignment
//! is already 2 and its fields sum to 6, so a packed and an unpacked reading
//! agree. That is worth stating explicitly rather than assuming: for the 3-byte
//! `TItemPos` the pack *was* load-bearing, and here it is not. Both nested
//! `WORD` fields are little-endian.
//!
//! The two `WORD` fields are item **positions**, not window/cell pairs. They are
//! flat indices handed to `CHARACTER::Attr67Add(const TAttr67AddData)` at
//! `char_item.cpp:10481`, which is a function parameter, not a wire read. They
//! stay opaque `u16`s; whether an index names a real item is inventory policy.
//!
//! The `bySubHeader` is `switch`ed with a `default:` arm at
//! `input_main.cpp:3489-3495`, so unknown values are absorbed rather than
//! fatal. It stays an opaque `u8`.

use crate::cg_inventory::{CgHeader, HEADER_CG_ATTR67_ADD, HEADER_CG_REFINE};
use crate::cg_wire::ClientFrame;

/// The full legacy `TPacketCGRefine` record, header byte included.
pub const CG_REFINE_WIRE_SIZE: usize = 1 + 1 + 1;
/// The framed payload of `TPacketCGRefine`.
pub const CG_REFINE_PAYLOAD_SIZE: usize = 2;

/// The packed width of `TAttr67AddData`: `WORD`, `BYTE`, `WORD`, `BYTE`.
pub const ATTR67_ADD_DATA_WIRE_SIZE: usize = 2 + 1 + 2 + 1;

/// The full legacy `TPacketCGAttr67Add` record, header byte included.
pub const CG_ATTR67_ADD_WIRE_SIZE: usize = 1 + 1 + ATTR67_ADD_DATA_WIRE_SIZE;
/// The framed payload of `TPacketCGAttr67Add`.
pub const CG_ATTR67_ADD_PAYLOAD_SIZE: usize = CG_ATTR67_ADD_WIRE_SIZE - 1;

/// Every way the two refinement decoders can refuse a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgRefineError {
    /// Fewer bytes than the fixed record needs.
    Truncated {
        /// The fixed width the decoder requires.
        needed: usize,
        /// How many bytes were actually offered.
        available: usize,
    },
    /// A complete-length slice that is not the fixed width.
    LengthMismatch {
        /// The fixed width the decoder requires.
        expected: usize,
        /// How many bytes were actually offered.
        actual: usize,
    },
    /// The right number of bytes, but the header byte is not this record's.
    InvalidHeader {
        /// The header byte this record requires.
        expected: u8,
        /// The header byte that was actually present.
        actual: u8,
    },
}

/// The nested `TAttr67AddData` payload, flattened into the record.
///
/// The four fields are in declaration order and each is read at its own fixed
/// offset. This is a view of the 6 wire bytes, not a nested Rust struct, because
/// the legacy nesting is flattened by the packet anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Attr67AddData {
    /// `WORD wRegistItemPos`, little-endian. A flat item position index.
    pub w_regist_item_pos: u16,
    /// `BYTE byMaterialCount`.
    pub by_material_count: u8,
    /// `WORD wSupportItemPos`, little-endian. A flat item position index.
    pub w_support_item_pos: u16,
    /// `BYTE bySupportItemCount`.
    pub by_support_item_count: u8,
}

impl Attr67AddData {
    /// The packed width of this payload.
    pub const WIRE_SIZE: usize = ATTR67_ADD_DATA_WIRE_SIZE;

    /// Build the payload from its four fields.
    pub const fn new(
        w_regist_item_pos: u16,
        by_material_count: u8,
        w_support_item_pos: u16,
        by_support_item_count: u8,
    ) -> Self {
        Self {
            w_regist_item_pos,
            by_material_count,
            w_support_item_pos,
            by_support_item_count,
        }
    }

    /// Append the 6 payload bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.w_regist_item_pos.to_le_bytes());
        out.push(self.by_material_count);
        out.extend_from_slice(&self.w_support_item_pos.to_le_bytes());
        out.push(self.by_support_item_count);
    }

    /// Encode to a fresh 6-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Read the payload from the start of `bytes`, which must be 6 bytes long.
    ///
    /// # Panics
    ///
    /// If `bytes` is shorter than [`Attr67AddData::WIRE_SIZE`]. This is the
    /// unchecked offset reader for a payload whose width the enclosing record
    /// has already validated; use [`Attr67AddData::decode`] at a trust boundary.
    #[must_use]
    pub fn read_at(bytes: &[u8]) -> Self {
        assert!(
            bytes.len() >= Self::WIRE_SIZE,
            "Attr67AddData needs {} bytes",
            Self::WIRE_SIZE
        );
        Self {
            w_regist_item_pos: u16::from_le_bytes([bytes[0], bytes[1]]),
            by_material_count: bytes[2],
            w_support_item_pos: u16::from_le_bytes([bytes[3], bytes[4]]),
            by_support_item_count: bytes[5],
        }
    }

    /// # Errors
    ///
    /// [`CgRefineError::LengthMismatch`] unless `bytes` is exactly
    /// [`Attr67AddData::WIRE_SIZE`] bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgRefineError> {
        if bytes.len() != Self::WIRE_SIZE {
            return Err(CgRefineError::LengthMismatch {
                expected: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        Ok(Self::read_at(bytes))
    }
}

/// The 3-byte `Refine` record: a header, a `BYTE` position, and a `BYTE` type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgRefine {
    /// The legacy `BYTE pos`. One byte, because the wide branch is commented
    /// out in the source.
    pub pos: u8,
    /// The legacy `BYTE type`. Opaque: its meaning is decided by the handler.
    pub ty: u8,
}

/// The 8-byte `Attr67Add` record: a header, an opaque sub-header, and the
/// 6-byte nested payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgAttr67Add {
    /// The legacy `BYTE bySubHeader`. `switch`ed with a `default:` arm.
    pub by_sub_header: u8,
    /// The nested `TAttr67AddData`.
    pub attr67_add_data: Attr67AddData,
}

/// Reject anything that is not exactly `expected` bytes wide.
fn check_exact(actual: usize, expected: usize) -> Result<(), CgRefineError> {
    if actual < expected {
        return Err(CgRefineError::Truncated {
            needed: expected,
            available: actual,
        });
    }
    if actual > expected {
        return Err(CgRefineError::LengthMismatch { expected, actual });
    }
    Ok(())
}

/// Reject a header byte that is not `expected`.
fn check_header(actual: u8, expected: u8) -> Result<(), CgRefineError> {
    if actual != expected {
        return Err(CgRefineError::InvalidHeader { expected, actual });
    }
    Ok(())
}

impl CgRefine {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_REFINE
    }
    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_REFINE_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_REFINE_PAYLOAD_SIZE;

    /// Build the record.
    pub const fn new(pos: u8, ty: u8) -> Self {
        Self { pos, ty }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.pos);
        out.push(self.ty);
    }

    /// Encode to a fresh 3-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode the header-less payload to a fresh 2-byte buffer.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame {
            header: Self::header().value(),
            payload: vec![self.pos, self.ty],
        }
    }

    /// # Errors
    ///
    /// [`CgRefineError::Truncated`] below 3 bytes,
    /// [`CgRefineError::LengthMismatch`] above,
    /// [`CgRefineError::InvalidHeader`] for a full-length slice not starting
    /// with 96.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgRefineError> {
        check_exact(bytes.len(), Self::WIRE_SIZE)?;
        check_header(bytes[0], Self::header().value())?;
        Ok(Self {
            pos: bytes[1],
            ty: bytes[2],
        })
    }

    /// # Errors
    ///
    /// As [`CgRefine::decode`], except that the payload must be exactly 2
    /// bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgRefineError> {
        check_exact(frame.payload.len(), Self::PAYLOAD_SIZE)?;
        check_header(frame.header, Self::header().value())?;
        Ok(Self {
            pos: frame.payload[0],
            ty: frame.payload[1],
        })
    }
}

impl CgAttr67Add {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_ATTR67_ADD
    }
    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_ATTR67_ADD_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_ATTR67_ADD_PAYLOAD_SIZE;

    /// Build the record.
    pub const fn new(by_sub_header: u8, attr67_add_data: Attr67AddData) -> Self {
        Self {
            by_sub_header,
            attr67_add_data,
        }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.by_sub_header);
        self.attr67_add_data.encode_into(out);
    }

    /// Encode to a fresh 8-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode the header-less payload to a fresh 7-byte buffer.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(Self::PAYLOAD_SIZE);
        payload.push(self.by_sub_header);
        self.attr67_add_data.encode_into(&mut payload);
        ClientFrame {
            header: Self::header().value(),
            payload,
        }
    }

    /// # Errors
    ///
    /// [`CgRefineError::Truncated`] below 8 bytes,
    /// [`CgRefineError::LengthMismatch`] above,
    /// [`CgRefineError::InvalidHeader`] for a full-length slice not starting
    /// with 169.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgRefineError> {
        check_exact(bytes.len(), Self::WIRE_SIZE)?;
        check_header(bytes[0], Self::header().value())?;
        Ok(Self {
            by_sub_header: bytes[1],
            attr67_add_data: Attr67AddData::read_at(&bytes[2..]),
        })
    }

    /// # Errors
    ///
    /// As [`CgAttr67Add::decode`], except that the payload must be exactly 7
    /// bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgRefineError> {
        check_exact(frame.payload.len(), Self::PAYLOAD_SIZE)?;
        check_header(frame.header, Self::header().value())?;
        Ok(Self {
            by_sub_header: frame.payload[0],
            attr67_add_data: Attr67AddData::read_at(&frame.payload[1..]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFINE: u8 = 96;
    const ATTR67: u8 = 169;

    #[test]
    fn the_headers_are_96_and_169() {
        assert_eq!(CgRefine::header().value(), REFINE);
        assert_eq!(CgAttr67Add::header().value(), ATTR67);
    }

    #[test]
    fn refine_is_three_bytes_with_a_byte_position() {
        assert_eq!(CG_REFINE_WIRE_SIZE, 3);
        assert_eq!(CgRefine::WIRE_SIZE, 3);
        assert_eq!(CgRefine::PAYLOAD_SIZE, 2);
        // The commented-out wide branch would make this 4. It is dead text.
        assert_ne!(CG_REFINE_WIRE_SIZE, 1 + 2 + 1);
    }

    #[test]
    fn the_nested_payload_is_six_bytes() {
        assert_eq!(ATTR67_ADD_DATA_WIRE_SIZE, 6);
        assert_eq!(Attr67AddData::WIRE_SIZE, 6);
        // Natural alignment is 2 and the fields sum to 6, so packed and unpacked
        // agree. Stated rather than assumed.
        assert_eq!(2 + 1 + 2 + 1, 6);
    }

    #[test]
    fn attr67_add_is_eight_bytes() {
        assert_eq!(CG_ATTR67_ADD_WIRE_SIZE, 8);
        assert_eq!(CgAttr67Add::WIRE_SIZE, 8);
        assert_eq!(CgAttr67Add::PAYLOAD_SIZE, 7);
        assert_eq!(1 + 1 + Attr67AddData::WIRE_SIZE, 8);
    }

    #[test]
    fn refine_encodes_to_the_exact_legacy_bytes() {
        assert_eq!(CgRefine::new(0, 0).encode(), vec![REFINE, 0, 0]);
        assert_eq!(CgRefine::new(0xAB, 0xCD).encode(), vec![REFINE, 0xAB, 0xCD]);
    }

    #[test]
    fn attr67_add_encodes_to_the_exact_legacy_bytes() {
        let data = Attr67AddData::new(0x0102, 0x03, 0x0405, 0x06);
        let rec = CgAttr67Add::new(0x07, data);
        assert_eq!(
            rec.encode(),
            vec![ATTR67, 0x07, 0x02, 0x01, 0x03, 0x05, 0x04, 0x06]
        );
    }

    #[test]
    fn the_nested_words_are_little_endian() {
        let data = Attr67AddData::new(0x0102, 0, 0x0304, 0);
        let bytes = data.encode();
        assert_eq!(&bytes[0..2], &[0x02, 0x01]);
        assert_eq!(&bytes[3..5], &[0x04, 0x03]);
    }

    #[test]
    fn the_nested_payload_decodes_standalone() {
        let data = Attr67AddData::new(1, 2, 3, 4);
        assert_eq!(Attr67AddData::decode(&data.encode()).unwrap(), data);
    }

    #[test]
    fn the_nested_payload_rejects_wrong_lengths() {
        for len in 0..6 {
            let b = vec![0u8; len];
            assert_eq!(
                Attr67AddData::decode(&b).unwrap_err(),
                CgRefineError::LengthMismatch {
                    expected: 6,
                    actual: len
                },
                "len {len}"
            );
        }
        assert!(Attr67AddData::decode(&[0; 7]).is_err());
    }

    #[test]
    fn both_round_trip() {
        for (pos, ty) in [(0, 0), (0xFF, 0xFF), (1, 2), (0x80, 0x7F)] {
            let r = CgRefine::new(pos, ty);
            assert_eq!(CgRefine::decode(&r.encode()).unwrap(), r);
        }
        let a = CgAttr67Add::new(0, Attr67AddData::new(0, 0, 0, 0));
        assert_eq!(CgAttr67Add::decode(&a.encode()).unwrap(), a);
        let b = CgAttr67Add::new(0xFF, Attr67AddData::new(u16::MAX, 0xFF, u16::MAX, 0xFF));
        assert_eq!(CgAttr67Add::decode(&b.encode()).unwrap(), b);
    }

    #[test]
    fn both_round_trip_through_a_frame() {
        let r = CgRefine::new(5, 6);
        let fr = r.to_frame();
        assert_eq!(fr.header, REFINE);
        assert_eq!(fr.payload.len(), 2);
        assert_eq!(CgRefine::decode_frame(&fr).unwrap(), r);
        assert_eq!(&fr.payload[..], &r.encode()[1..]);

        let a = CgAttr67Add::new(8, Attr67AddData::new(9, 10, 11, 12));
        let fa = a.to_frame();
        assert_eq!(fa.header, ATTR67);
        assert_eq!(fa.payload.len(), 7);
        assert_eq!(CgAttr67Add::decode_frame(&fa).unwrap(), a);
        assert_eq!(&fa.payload[..], &a.encode()[1..]);
    }

    #[test]
    fn the_two_records_are_not_interchangeable() {
        assert!(CgAttr67Add::decode(&CgRefine::new(1, 2).encode()).is_err());
        assert!(CgRefine::decode(&CgAttr67Add::new(1, Attr67AddData::default()).encode()).is_err());
    }

    #[test]
    fn refine_rejects_every_short_length() {
        for len in 0..3 {
            let mut b = CgRefine::new(1, 2).encode();
            b.truncate(len);
            assert_eq!(
                CgRefine::decode(&b).unwrap_err(),
                CgRefineError::Truncated {
                    needed: 3,
                    available: len
                },
                "len {len}"
            );
        }
    }

    #[test]
    fn refine_rejects_every_long_length() {
        for extra in 1..=3 {
            let mut b = CgRefine::new(1, 2).encode();
            b.extend(std::iter::repeat_n(0u8, extra));
            assert_eq!(
                CgRefine::decode(&b).unwrap_err(),
                CgRefineError::LengthMismatch {
                    expected: 3,
                    actual: 3 + extra
                }
            );
        }
    }

    #[test]
    fn attr67_add_rejects_every_short_length() {
        for len in 0..8 {
            let mut b = CgAttr67Add::new(1, Attr67AddData::default()).encode();
            b.truncate(len);
            assert_eq!(
                CgAttr67Add::decode(&b).unwrap_err(),
                CgRefineError::Truncated {
                    needed: 8,
                    available: len
                },
                "len {len}"
            );
        }
    }

    #[test]
    fn attr67_add_rejects_every_long_length() {
        for extra in 1..=3 {
            let mut b = CgAttr67Add::new(1, Attr67AddData::default()).encode();
            b.extend(std::iter::repeat_n(0u8, extra));
            assert_eq!(
                CgAttr67Add::decode(&b).unwrap_err(),
                CgRefineError::LengthMismatch {
                    expected: 8,
                    actual: 8 + extra
                }
            );
        }
    }

    #[test]
    fn every_wrong_header_is_rejected_by_both() {
        for v in 0..=255_u8 {
            if v != REFINE {
                let mut b = CgRefine::new(0, 0).encode();
                b[0] = v;
                assert_eq!(
                    CgRefine::decode(&b).unwrap_err(),
                    CgRefineError::InvalidHeader {
                        expected: 96,
                        actual: v
                    },
                    "refine header {v}"
                );
            }
            if v != ATTR67 {
                let mut b = CgAttr67Add::new(0, Attr67AddData::default()).encode();
                b[0] = v;
                assert_eq!(
                    CgAttr67Add::decode(&b).unwrap_err(),
                    CgRefineError::InvalidHeader {
                        expected: 169,
                        actual: v
                    },
                    "attr67 header {v}"
                );
            }
        }
    }

    #[test]
    fn rejects_every_wrong_frame_length() {
        for len in 0..2 {
            let f = ClientFrame {
                header: REFINE,
                payload: vec![0; len],
            };
            assert!(CgRefine::decode_frame(&f).is_err(), "len {len}");
        }
        for len in 0..7 {
            let f = ClientFrame {
                header: ATTR67,
                payload: vec![0; len],
            };
            assert!(CgAttr67Add::decode_frame(&f).is_err(), "len {len}");
        }
    }

    #[test]
    fn every_refine_byte_is_read() {
        let base = CgRefine::new(0, 0);
        let mut changed = 0;
        for i in 1..3 {
            let mut b = base.encode();
            b[i] = 0xFF;
            if CgRefine::decode(&b).unwrap() != base {
                changed += 1;
            }
        }
        assert_eq!(changed, 2);
    }

    #[test]
    fn every_attr67_byte_is_read() {
        let base = CgAttr67Add::new(0, Attr67AddData::default());
        let mut changed = 0;
        for i in 1..8 {
            let mut b = base.encode();
            b[i] = 0xFF;
            if CgAttr67Add::decode(&b).unwrap() != base {
                changed += 1;
            }
        }
        assert_eq!(changed, 7);
    }

    #[test]
    fn every_nested_payload_byte_is_read() {
        let base = Attr67AddData::default();
        let mut changed = 0;
        for i in 0..6 {
            let mut b = base.encode();
            b[i] = 0xFF;
            if Attr67AddData::decode(&b).unwrap() != base {
                changed += 1;
            }
        }
        assert_eq!(changed, 6);
    }

    #[test]
    fn the_position_and_type_are_independent() {
        let a = CgRefine::new(0x11, 0x22);
        let b = CgRefine::new(0x22, 0x11);
        assert_eq!(CgRefine::decode(&a.encode()).unwrap(), a);
        assert_eq!(CgRefine::decode(&b.encode()).unwrap(), b);
        assert_ne!(a, b);
    }

    #[test]
    fn the_two_nested_words_are_independent() {
        let a = Attr67AddData::new(1, 9, 2, 9);
        let b = Attr67AddData::new(2, 9, 1, 9);
        assert_eq!(Attr67AddData::decode(&a.encode()).unwrap(), a);
        assert_eq!(Attr67AddData::decode(&b.encode()).unwrap(), b);
        assert_ne!(a, b);
    }

    #[test]
    fn encode_into_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE];
        CgAttr67Add::new(1, Attr67AddData::new(2, 3, 4, 5)).encode_into(&mut out);
        assert_eq!(out.len(), 1 + 8);
        assert_eq!(out[1], ATTR67);
    }
}
