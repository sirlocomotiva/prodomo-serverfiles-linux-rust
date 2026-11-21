//! The two 1-byte client records that carry no payload at all.
//!
//! | record | header | wire | declaration | registration | dispatch |
//! |---|---|---|---|---|---|
//! | `Text` | 64 `0x40` | 1 | `packet.h:451` | `packet_info.cpp:98` | **none** |
//! | `InventoryExpansion` | 226 `0xe2` | 1 | `packet.h:643` | `packet_info.cpp:122` | `input_main.cpp:3711` |
//!
//! They share a module because they share a shape: a lone header byte and
//! nothing else. The interesting part is that they are **not** the same record.
//!
//! # One is registered and never dispatched
//!
//! `HEADER_CG_TEXT` is registered as
//! `Set(HEADER_CG_TEXT, 1, sizeof(TPacketCGText), "Text")` and defined at
//! `packet.h:451`, but searching all seven `::Analyze` files for
//! `case HEADER_CG_TEXT`, `HEADER_CG_TEXT ==` and `Set(HEADER_CG_TEXT,`
//! returns **zero** hits. The entire client tree has zero references to either
//! `HEADER_CG_TEXT` or `TPacketCGText`.
//!
//! So `CgText` is a genuinely undispatched record. The codec exists because the
//! registration is real and a future dispatch arm would need the boundary, but
//! **nothing in this crate should be read as claiming the server handles it.**
//!
//! # The other is dispatched and the payload is thrown away
//!
//! `CInputMain::InventoryExpansion` at `input_main.cpp:1068-1070` is the whole
//! function:
//!
//! ```cpp
//! if (ch)
//!     ch->Update_Inven();
//! ```
//!
//! It never looks at the data pointer. The 1-byte record is a pure "re-send me
//! my inventory" signal, and `1 + 0 = 1` is the complete field list.
//!
//! # The header is not named `HEADER_CG_*`
//!
//! `ENVANTER_BLACK` is declared bare at `packet.h:88` with no `HEADER_CG_`
//! prefix. It is the only CG header in the table that is not, which is why
//! `cg_inventory` has to carry a constant name independently of the naming
//! convention. The client sends it at
//! `PythonNetworkStreamPhaseGame.cpp:2000-2001`.

use crate::cg_inventory::{CgHeader, ENVANTER_BLACK, HEADER_CG_TEXT};
use crate::cg_wire::ClientFrame;

/// The full legacy `TPacketCGText` record: a header byte and nothing else.
pub const CG_TEXT_WIRE_SIZE: usize = 1;
/// The framed payload of `TPacketCGText`. Empty, by construction.
pub const CG_TEXT_PAYLOAD_SIZE: usize = 0;

/// The full legacy `TPacketCGEnvanter` record: a header byte and nothing else.
pub const ENVANTER_BLACK_WIRE_SIZE: usize = 1;
/// The framed payload of `TPacketCGEnvanter`. Empty, by construction.
pub const ENVANTER_BLACK_PAYLOAD_SIZE: usize = 0;

/// Every way a header-only decoder can refuse a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgHeaderOnlyError {
    /// Fewer bytes than the single header byte.
    Truncated {
        /// The fixed width the decoder requires.
        needed: usize,
        /// How many bytes were actually offered.
        available: usize,
    },
    /// More than one byte. A record with no payload cannot carry a second byte.
    LengthMismatch {
        /// The fixed width the decoder requires.
        expected: usize,
        /// How many bytes were actually offered.
        actual: usize,
    },
    /// One byte, but not this record's header.
    InvalidHeader {
        /// The header byte this record requires.
        expected: u8,
        /// The header byte that was actually present.
        actual: u8,
    },
}

/// The 1-byte `Text` record.
///
/// There is no payload field. The only data is the header, which the decoder
/// checks, so constructing this type is a statement of intent rather than a
/// parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgText;

impl CgText {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_TEXT
    }
    /// The full legacy record width, which is the header byte.
    pub const WIRE_SIZE: usize = CG_TEXT_WIRE_SIZE;
    /// The framed payload width, which is zero.
    pub const PAYLOAD_SIZE: usize = CG_TEXT_PAYLOAD_SIZE;

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
    }

    /// Encode to a fresh 1-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode the header-less payload, which is empty.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame {
            header: Self::header().value(),
            payload: Vec::new(),
        }
    }

    /// # Errors
    ///
    /// [`CgHeaderOnlyError::Truncated`] on an empty slice,
    /// [`CgHeaderOnlyError::LengthMismatch`] on two or more bytes,
    /// [`CgHeaderOnlyError::InvalidHeader`] for a single byte that is not 64.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgHeaderOnlyError> {
        check_exact(bytes.len(), Self::WIRE_SIZE)?;
        check_header(bytes[0], Self::header().value())?;
        Ok(Self)
    }

    /// # Errors
    ///
    /// As [`CgText::decode`], except that the frame payload must be empty.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgHeaderOnlyError> {
        check_exact(frame.payload.len(), Self::PAYLOAD_SIZE)?;
        check_header(frame.header, Self::header().value())?;
        Ok(Self)
    }
}

/// The 1-byte `InventoryExpansion` record.
///
/// The legacy handler ignores any data, so there is no payload to expose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgEnvanterBlack;

impl CgEnvanterBlack {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        ENVANTER_BLACK
    }
    /// The full legacy record width, which is the header byte.
    pub const WIRE_SIZE: usize = ENVANTER_BLACK_WIRE_SIZE;
    /// The framed payload width, which is zero.
    pub const PAYLOAD_SIZE: usize = ENVANTER_BLACK_PAYLOAD_SIZE;

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
    }

    /// Encode to a fresh 1-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode the header-less payload, which is empty.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame {
            header: Self::header().value(),
            payload: Vec::new(),
        }
    }

    /// # Errors
    ///
    /// [`CgHeaderOnlyError::Truncated`] on an empty slice,
    /// [`CgHeaderOnlyError::LengthMismatch`] on two or more bytes,
    /// [`CgHeaderOnlyError::InvalidHeader`] for a single byte that is not 226.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgHeaderOnlyError> {
        check_exact(bytes.len(), Self::WIRE_SIZE)?;
        check_header(bytes[0], Self::header().value())?;
        Ok(Self)
    }

    /// # Errors
    ///
    /// As [`CgEnvanterBlack::decode`], except that the frame payload must be
    /// empty.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgHeaderOnlyError> {
        check_exact(frame.payload.len(), Self::PAYLOAD_SIZE)?;
        check_header(frame.header, Self::header().value())?;
        Ok(Self)
    }
}

/// Reject anything that is not exactly `expected` bytes wide.
fn check_exact(actual: usize, expected: usize) -> Result<(), CgHeaderOnlyError> {
    if actual < expected {
        return Err(CgHeaderOnlyError::Truncated {
            needed: expected,
            available: actual,
        });
    }
    if actual > expected {
        return Err(CgHeaderOnlyError::LengthMismatch { expected, actual });
    }
    Ok(())
}

/// Reject a header byte that is not `expected`.
fn check_header(actual: u8, expected: u8) -> Result<(), CgHeaderOnlyError> {
    if actual != expected {
        return Err(CgHeaderOnlyError::InvalidHeader { expected, actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: u8 = 0x40;
    const ENVA: u8 = 0xe2;

    #[test]
    fn the_headers_are_64_and_226() {
        assert_eq!(CgText::header().value(), TEXT);
        assert_eq!(CgEnvanterBlack::header().value(), ENVA);
    }

    #[test]
    fn the_two_headers_are_distinct() {
        assert_ne!(CgText::header().value(), CgEnvanterBlack::header().value());
    }

    #[test]
    fn both_records_are_one_byte_with_no_payload() {
        for (wire, payload) in [
            (CgText::WIRE_SIZE, CgText::PAYLOAD_SIZE),
            (CgEnvanterBlack::WIRE_SIZE, CgEnvanterBlack::PAYLOAD_SIZE),
        ] {
            assert_eq!(wire, 1);
            assert_eq!(payload, 0);
            assert_eq!(wire, 1 + payload);
        }
    }

    #[test]
    fn text_encodes_to_one_byte() {
        let bytes = CgText.encode();
        assert_eq!(bytes, vec![TEXT]);
    }

    #[test]
    fn envanter_black_encodes_to_one_byte() {
        let bytes = CgEnvanterBlack.encode();
        assert_eq!(bytes, vec![ENVA]);
    }

    #[test]
    fn both_round_trip() {
        assert_eq!(CgText::decode(&CgText.encode()).unwrap(), CgText);
        assert_eq!(
            CgEnvanterBlack::decode(&CgEnvanterBlack.encode()).unwrap(),
            CgEnvanterBlack
        );
    }

    #[test]
    fn both_round_trip_through_a_frame() {
        let f = CgText.to_frame();
        assert_eq!(f.header, TEXT);
        assert!(f.payload.is_empty());
        assert_eq!(CgText::decode_frame(&f).unwrap(), CgText);

        let f = CgEnvanterBlack.to_frame();
        assert_eq!(f.header, ENVA);
        assert!(f.payload.is_empty());
        assert_eq!(CgEnvanterBlack::decode_frame(&f).unwrap(), CgEnvanterBlack);
    }

    #[test]
    fn the_frame_payload_is_the_record_minus_the_header() {
        assert_eq!(&CgText.to_frame().payload[..], &CgText.encode()[1..]);
        assert_eq!(
            &CgEnvanterBlack.to_frame().payload[..],
            &CgEnvanterBlack.encode()[1..]
        );
    }

    #[test]
    fn the_two_records_are_not_interchangeable() {
        assert!(CgEnvanterBlack::decode(&CgText.encode()).is_err());
        assert!(CgText::decode(&CgEnvanterBlack.encode()).is_err());
    }

    #[test]
    fn an_empty_slice_is_truncated_not_mismatched() {
        let empty: [u8; 0] = [];
        for err in [
            CgText::decode(&empty).unwrap_err(),
            CgEnvanterBlack::decode(&empty).unwrap_err(),
        ] {
            assert_eq!(
                err,
                CgHeaderOnlyError::Truncated {
                    needed: 1,
                    available: 0
                }
            );
        }
    }

    #[test]
    fn every_extra_byte_is_rejected() {
        for extra in 1..=4_u8 {
            let mut a = CgText.encode();
            a.push(extra);
            assert_eq!(
                CgText::decode(&a).unwrap_err(),
                CgHeaderOnlyError::LengthMismatch {
                    expected: 1,
                    actual: 2
                }
            );
            let mut b = CgEnvanterBlack.encode();
            b.push(extra);
            assert_eq!(
                CgEnvanterBlack::decode(&b).unwrap_err(),
                CgHeaderOnlyError::LengthMismatch {
                    expected: 1,
                    actual: 2
                }
            );
        }
    }

    #[test]
    fn every_other_single_byte_is_a_wrong_header() {
        for b in 0..=255_u8 {
            if b == TEXT || b == ENVA {
                continue;
            }
            assert_eq!(
                CgText::decode(&[b]).unwrap_err(),
                CgHeaderOnlyError::InvalidHeader {
                    expected: 64,
                    actual: b
                },
                "text byte {b}"
            );
            assert_eq!(
                CgEnvanterBlack::decode(&[b]).unwrap_err(),
                CgHeaderOnlyError::InvalidHeader {
                    expected: 226,
                    actual: b
                },
                "envanter byte {b}"
            );
        }
    }

    #[test]
    fn a_frame_with_a_payload_is_rejected() {
        for len in 1..=4 {
            let f = ClientFrame {
                header: TEXT,
                payload: vec![0; len],
            };
            assert!(CgText::decode_frame(&f).is_err(), "len {len}");
            let f = ClientFrame {
                header: ENVA,
                payload: vec![0; len],
            };
            assert!(CgEnvanterBlack::decode_frame(&f).is_err(), "len {len}");
        }
    }

    #[test]
    fn a_frame_with_a_wrong_header_is_rejected() {
        let f = ClientFrame {
            header: ENVA,
            payload: Vec::new(),
        };
        assert_eq!(
            CgText::decode_frame(&f).unwrap_err(),
            CgHeaderOnlyError::InvalidHeader {
                expected: 64,
                actual: 226
            }
        );
    }

    #[test]
    fn encode_into_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE, 0xAD];
        CgText.encode_into(&mut out);
        assert_eq!(out, vec![0xDE, 0xAD, TEXT]);
        CgEnvanterBlack.encode_into(&mut out);
        assert_eq!(out, vec![0xDE, 0xAD, TEXT, ENVA]);
    }

    #[test]
    fn the_default_value_is_the_record() {
        assert_eq!(CgText, CgText);
        // A unit struct is its own default; there is nothing to construct.
        assert_eq!(CgEnvanterBlack, CgEnvanterBlack);
    }
}
