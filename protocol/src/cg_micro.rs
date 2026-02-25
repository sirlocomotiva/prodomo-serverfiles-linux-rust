//! Transport-free codecs for four tiny fixed-width CG game-phase records.
//!
//! These four share nothing domain-wise. They are grouped only because they are
//! the same *shape*: a one-byte header and at most one opaque `u8`, all
//! registered unconditionally, all dispatched by `CInputMain` in
//! `input_main.cpp`, all free of sub-headers, `iExtraLen`, and observer
//! guards. Grouping by width and phase rather than by domain is a deliberate
//! departure from the domain-named modules around them, and the module doc says
//! so rather than implying a family that does not exist.
//!
//! | record | header | wire | payload | declaration |
//! |---|---|---|---|---|
//! | `TPacketCGPosition` | 28 `0x1c` | 2 | 1 | `packet.h:738-741` |
//! | `TPacketCGScriptAnswer` | 29 `0x1d` | 2 | 1 | `packet.h:744-749` |
//! | `TPacketCGWarp` | 65 `0x41` | 1 | 0 | `packet.h:1864-1866` |
//! | `TPacketCGFishing` | 82 `0x52` | 2 | 1 | `packet.h:2297-2300` |
//!
//! All four lie inside the single `#pragma pack(1)` that opens at
//! `packet.h:274` and closes at `:3540`, and outside every `#ifdef`. Their
//! registrations are `packet_info.cpp:131`, `:132`, `:144`, and `:160`, and
//! their dispatch arms are `input_main.cpp:3686`, `:3786`, `:3801`, and
//! `:3841`.
//!
//! # What the legacy handlers do, and why none of it is framing
//!
//! - `CInputMain::Position` at `input_main.cpp:1530` reads the byte through the
//!   **struct tag** `struct command_position *`, not the typedef, and switches
//!   on it with only three arms: `POSITION_GENERAL` to `Standup()`,
//!   `POSITION_SITTING_CHAIR` to `Sitdown(0)`, and `POSITION_SITTING_GROUND` to
//!   `Sitdown(1)`. `ECharacterPosition` at `length.h:435-444` also numbers
//!   `POSITION_BATTLE`, `POSITION_DYING`, `POSITION_INTRO`, and the sentinel
//!   `POSITION_MAX_NUM`, and the switch has **no `default` arm**, so those and
//!   every out-of-range value fall through and do nothing at all.
//! - `CInputMain::ScriptAnswer` at `:2200` does **not** treat the byte as a
//!   threshold with a no-op branch. It is a genuine two-way dispatch:
//!   `answer > 250` calls `quest::CQuestManager::Resume` at `:2207`, and the
//!   `else` at `:2209-2211` calls `quest::CQuestManager::Select(pid, answer)`.
//!   So values 0 through 250 select and 251 through 255 resume.
//! - `CInputMain::Warp` at `:2271` **ignores its payload entirely**; the whole
//!   body is `ch->WarpEnd();`. The one-byte record is still a real fixed record
//!   that the frame layer must validate, and `CgWarp` has an empty payload.
//! - `CInputMain::Fishing` at `:3166` calls `ch->SetRotation(p->dir * 5)` and
//!   then `ch->fishing()`. The client mirrors the factor of five exactly, and
//!   in the opposite direction, but **without a struct**: there is no
//!   `TPacketCGFishing` anywhere in `client/Client`, and
//!   `CPythonNetworkStream::SendFishingPacket(int iRotation)` at
//!   `PythonNetworkStreamPhaseGame.cpp:4366-4376` emits the header byte and
//!   `iRotation / 5` as two separate `Send` calls, the second narrowing a signed
//!   `int` into a `BYTE` with no range check.
//!
//! Every one of those behaviours is policy or gameplay. None of it belongs in
//! a codec, so [`CgPosition::position`], [`CgScriptAnswer::answer`], and
//! [`CgFishing::dir`] stay opaque across all 256 values, and the dead commented
//! fields in the `ScriptAnswer` declaration stay dead.

use crate::cg_inventory::{
    CgHeader, HEADER_CG_CHARACTER_POSITION, HEADER_CG_FISHING, HEADER_CG_SCRIPT_ANSWER,
    HEADER_CG_WARP,
};
use crate::cg_wire::ClientFrame;

/// The full legacy `TPacketCGPosition` record, header byte included.
pub const CG_POSITION_WIRE_SIZE: usize = 2;
/// The framed payload of `TPacketCGPosition`, everything after the header.
pub const CG_POSITION_PAYLOAD_SIZE: usize = 1;
/// The full legacy `TPacketCGScriptAnswer` record, header byte included.
pub const CG_SCRIPT_ANSWER_WIRE_SIZE: usize = 2;
/// The framed payload of `TPacketCGScriptAnswer`.
pub const CG_SCRIPT_ANSWER_PAYLOAD_SIZE: usize = 1;
/// The full legacy `TPacketCGWarp` record. It is a header and nothing else.
pub const CG_WARP_WIRE_SIZE: usize = 1;
/// The framed payload of `TPacketCGWarp`, which is empty by construction.
pub const CG_WARP_PAYLOAD_SIZE: usize = 0;
/// The full legacy `TPacketCGFishing` record, header byte included.
pub const CG_FISHING_WIRE_SIZE: usize = 2;
/// The framed payload of `TPacketCGFishing`.
pub const CG_FISHING_PAYLOAD_SIZE: usize = 1;

/// Every way one of these four decoders can refuse a byte slice.
///
/// The three failure modes are identical across the four, so they share one
/// error type rather than four near-identical ones. `InvalidHeader` carries the
/// expected value so the message still names the right record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgMicroError {
    /// Fewer bytes than the fixed record needs.
    Truncated {
        /// The fixed width the decoder requires.
        needed: usize,
        /// How many bytes were actually offered.
        available: usize,
    },
    /// A complete-length slice that is not the fixed width.
    LengthMismatch {
        /// The one width this record has.
        expected: usize,
        /// The width that was offered.
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

impl std::fmt::Display for CgMicroError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(f, "CG record needs {needed} bytes, got {available}")
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "CG record must be exactly {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { expected, actual } => {
                write!(f, "CG header {actual} is not {expected}")
            }
        }
    }
}

impl std::error::Error for CgMicroError {}

/// Check a complete record width, then its header.
fn decode_parts(bytes: &[u8], wire_size: usize, expected: u8) -> Result<(), CgMicroError> {
    check_exact(bytes.len(), wire_size)?;
    check_header(bytes[0], expected)
}

/// Check a header-less frame payload width, then its header.
///
/// A zero-width payload is a real case here, not a degenerate one:
/// `CG_WARP_PAYLOAD_SIZE` is 0, so this must accept an empty payload and still
/// reject a one-byte one.
fn decode_frame_parts(
    frame: &ClientFrame,
    payload_size: usize,
    expected: u8,
) -> Result<(), CgMicroError> {
    check_exact(frame.payload.len(), payload_size)?;
    check_header(frame.header, expected)
}

fn check_exact(len: usize, size: usize) -> Result<(), CgMicroError> {
    if len < size {
        return Err(CgMicroError::Truncated {
            needed: size,
            available: len,
        });
    }
    if len > size {
        return Err(CgMicroError::LengthMismatch {
            expected: size,
            actual: len,
        });
    }
    Ok(())
}

fn check_header(actual: u8, expected: u8) -> Result<(), CgMicroError> {
    if actual != expected {
        return Err(CgMicroError::InvalidHeader { expected, actual });
    }
    Ok(())
}

/// The transport-free `TPacketCGPosition` record, header 28.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgPosition {
    /// The requested character position. Opaque: the server switches over
    /// `ECharacterPosition` at `input_main.cpp:1534` with no `default` arm, so
    /// unhandled and out-of-range values are silently ignored rather than
    /// rejected. All 256 values are therefore legal here.
    pub position: u8,
}

impl CgPosition {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_CHARACTER_POSITION
    }

    /// Build a record from a position byte. No value is rejected.
    pub const fn new(position: u8) -> Self {
        Self { position }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.position);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_POSITION_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame::new(Self::header().value(), [self.position])
    }

    /// # Errors
    ///
    /// Returns [`CgMicroError::Truncated`] for a short input,
    /// [`CgMicroError::LengthMismatch`] for a long input, and
    /// [`CgMicroError::InvalidHeader`] when the header is not 28. The length is
    /// checked before the header, and the header before any payload byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgMicroError> {
        decode_parts(bytes, CG_POSITION_WIRE_SIZE, Self::header().value())?;
        Ok(Self { position: bytes[1] })
    }

    /// # Errors
    ///
    /// As [`CgPosition::decode`], except that the frame payload excludes the
    /// header byte, so the **payload** width is checked before the frame header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgMicroError> {
        decode_frame_parts(frame, CG_POSITION_PAYLOAD_SIZE, Self::header().value())?;
        Ok(Self {
            position: frame.payload[0],
        })
    }
}

/// The transport-free `TPacketCGScriptAnswer` record, header 29.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgScriptAnswer {
    /// The script answer byte. Opaque: the server compares it against 250 at
    /// `input_main.cpp:2205` and then hands it to `CQuestManager::Select`, so
    /// every value has a distinct effect and none of them is a framing error.
    pub answer: u8,
}

impl CgScriptAnswer {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_SCRIPT_ANSWER
    }

    /// Build a record from an answer byte. No value is rejected.
    pub const fn new(answer: u8) -> Self {
        Self { answer }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.answer);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_SCRIPT_ANSWER_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame::new(Self::header().value(), [self.answer])
    }

    /// # Errors
    ///
    /// Returns [`CgMicroError::Truncated`] for a short input,
    /// [`CgMicroError::LengthMismatch`] for a long input, and
    /// [`CgMicroError::InvalidHeader`] when the header is not 29. The length is
    /// checked before the header, and the header before any payload byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgMicroError> {
        decode_parts(bytes, CG_SCRIPT_ANSWER_WIRE_SIZE, Self::header().value())?;
        Ok(Self { answer: bytes[1] })
    }

    /// # Errors
    ///
    /// As [`CgScriptAnswer::decode`], except that the **payload** width is
    /// checked before the frame header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgMicroError> {
        decode_frame_parts(frame, CG_SCRIPT_ANSWER_PAYLOAD_SIZE, Self::header().value())?;
        Ok(Self {
            answer: frame.payload[0],
        })
    }
}

/// The transport-free `TPacketCGWarp` record, header 65.
///
/// The record has no payload field. `CInputMain::Warp` at
/// `input_main.cpp:2271` reads nothing and calls `ch->WarpEnd()`; the byte on
/// the wire is the header and the codec has to validate it, which is the whole
/// of this record's job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgWarp;

impl CgWarp {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_WARP
    }

    /// Build the record. It carries no field.
    pub const fn new() -> Self {
        Self
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_WARP_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, which has an empty payload.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame::new(Self::header().value(), [0_u8; 0])
    }

    /// # Errors
    ///
    /// Returns [`CgMicroError::Truncated`] for an empty input,
    /// [`CgMicroError::LengthMismatch`] for an input longer than one byte, and
    /// [`CgMicroError::InvalidHeader`] when the header is not 65.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgMicroError> {
        decode_parts(bytes, CG_WARP_WIRE_SIZE, Self::header().value())?;
        Ok(Self)
    }

    /// # Errors
    ///
    /// As [`CgWarp::decode`], except that the frame payload must be **empty**.
    /// A one-byte payload is a [`CgMicroError::LengthMismatch`], not an
    /// accepted record, because `CInputMain::Warp` never reads it and the
    /// legacy sender never sends it.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgMicroError> {
        decode_frame_parts(frame, CG_WARP_PAYLOAD_SIZE, Self::header().value())?;
        Ok(Self)
    }
}

/// The transport-free `TPacketCGFishing` record, header 82.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgFishing {
    /// The fishing direction. Opaque: the server multiplies it by five at
    /// `input_main.cpp:3169` and passes the result straight to
    /// `SetRotation`, with no range check, so every byte is legal.
    pub dir: u8,
}

impl CgFishing {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_FISHING
    }

    /// Build a record from a direction byte. No value is rejected.
    pub const fn new(dir: u8) -> Self {
        Self { dir }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.dir);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_FISHING_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame::new(Self::header().value(), [self.dir])
    }

    /// # Errors
    ///
    /// Returns [`CgMicroError::Truncated`] for a short input,
    /// [`CgMicroError::LengthMismatch`] for a long input, and
    /// [`CgMicroError::InvalidHeader`] when the header is not 82. The length is
    /// checked before the header, and the header before any payload byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgMicroError> {
        decode_parts(bytes, CG_FISHING_WIRE_SIZE, Self::header().value())?;
        Ok(Self { dir: bytes[1] })
    }

    /// # Errors
    ///
    /// As [`CgFishing::decode`], except that the **payload** width is checked
    /// before the frame header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgMicroError> {
        decode_frame_parts(frame, CG_FISHING_PAYLOAD_SIZE, Self::header().value())?;
        Ok(Self {
            dir: frame.payload[0],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every byte value a field can hold, so opacity is tested over the whole
    /// `u8` domain rather than at a few sampled points.
    fn all_bytes() -> Vec<u8> {
        (0..=u8::MAX).collect()
    }

    /// One record's identity and encodings, so the shared assertions can walk
    /// all four without repeating the dispatch four times.
    struct Sample {
        name: &'static str,
        header: CgHeader,
        /// The encoding of the field set to zero. Warp has no field, so this is
        /// the whole one-byte record.
        low: Vec<u8>,
        /// The encoding of the field set to `u8::MAX`, or the same bytes again
        /// for the fieldless record.
        high: Vec<u8>,
        wire_size: usize,
        payload_size: usize,
    }

    impl Sample {
        /// Decode `bytes` with whichever decoder matches this record's name.
        fn decode(&self, bytes: &[u8]) -> Result<Self2, CgMicroError> {
            match self.name {
                "CgPosition" => CgPosition::decode(bytes).map(Self2::Position),
                "CgScriptAnswer" => CgScriptAnswer::decode(bytes).map(Self2::ScriptAnswer),
                "CgWarp" => CgWarp::decode(bytes).map(|_| Self2::Warp),
                _ => CgFishing::decode(bytes).map(Self2::Fishing),
            }
        }
    }

    /// The decoded value of any one of the four, so `decode` above can return a
    /// single error type without losing which record was checked.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Self2 {
        Position(CgPosition),
        ScriptAnswer(CgScriptAnswer),
        Warp,
        Fishing(CgFishing),
    }

    fn frames() -> Vec<Sample> {
        vec![
            Sample {
                name: "CgPosition",
                header: HEADER_CG_CHARACTER_POSITION,
                low: CgPosition::new(0).encode(),
                high: CgPosition::new(u8::MAX).encode(),
                wire_size: CG_POSITION_WIRE_SIZE,
                payload_size: CG_POSITION_PAYLOAD_SIZE,
            },
            Sample {
                name: "CgScriptAnswer",
                header: HEADER_CG_SCRIPT_ANSWER,
                low: CgScriptAnswer::new(0).encode(),
                high: CgScriptAnswer::new(u8::MAX).encode(),
                wire_size: CG_SCRIPT_ANSWER_WIRE_SIZE,
                payload_size: CG_SCRIPT_ANSWER_PAYLOAD_SIZE,
            },
            Sample {
                name: "CgWarp",
                header: HEADER_CG_WARP,
                low: CgWarp::new().encode(),
                high: CgWarp::new().encode(),
                wire_size: CG_WARP_WIRE_SIZE,
                payload_size: CG_WARP_PAYLOAD_SIZE,
            },
            Sample {
                name: "CgFishing",
                header: HEADER_CG_FISHING,
                low: CgFishing::new(0).encode(),
                high: CgFishing::new(u8::MAX).encode(),
                wire_size: CG_FISHING_WIRE_SIZE,
                payload_size: CG_FISHING_PAYLOAD_SIZE,
            },
        ]
    }

    #[test]
    fn header_constants_match_the_legacy_values() {
        assert_eq!(CgPosition::header().value(), 28);
        assert_eq!(CgScriptAnswer::header().value(), 29);
        assert_eq!(CgWarp::header().value(), 65);
        assert_eq!(CgFishing::header().value(), 82);
        // 0x1c, 0x1d, 0x41, 0x52
        assert_eq!(CgPosition::header().value(), 0x1c);
        assert_eq!(CgScriptAnswer::header().value(), 0x1d);
        assert_eq!(CgWarp::header().value(), 0x41);
        assert_eq!(CgFishing::header().value(), 0x52);
    }

    #[test]
    fn golden_bytes_are_exact() {
        assert_eq!(CgPosition::new(3).encode(), vec![0x1c, 0x03]);
        assert_eq!(CgScriptAnswer::new(251).encode(), vec![0x1d, 0xfb]);
        assert_eq!(CgWarp::new().encode(), vec![0x41]);
        assert_eq!(CgFishing::new(5).encode(), vec![0x52, 0x05]);
    }

    #[test]
    fn warp_is_exactly_one_byte_with_no_payload() {
        let encoded = CgWarp::new().encode();
        assert_eq!(encoded.len(), CG_WARP_WIRE_SIZE);
        assert_eq!(CG_WARP_PAYLOAD_SIZE, 0);
        assert_eq!(CG_WARP_WIRE_SIZE, CG_WARP_PAYLOAD_SIZE + 1);
    }

    #[test]
    fn every_size_constant_agrees_with_the_real_encoding() {
        for s in frames() {
            assert_eq!(s.wire_size, s.low.len(), "{} wire constant", s.name);
            assert_eq!(s.wire_size, s.high.len(), "{} wire constant", s.name);
            assert_eq!(
                s.wire_size,
                s.payload_size + 1,
                "{} wire must be payload plus the header",
                s.name
            );
        }
    }

    #[test]
    fn position_round_trips() {
        for value in all_bytes() {
            let bytes = CgPosition::new(value).encode();
            assert_eq!(CgPosition::decode(&bytes), Ok(CgPosition::new(value)));
        }
    }

    #[test]
    fn script_answer_round_trips() {
        for value in all_bytes() {
            let bytes = CgScriptAnswer::new(value).encode();
            assert_eq!(
                CgScriptAnswer::decode(&bytes),
                Ok(CgScriptAnswer::new(value))
            );
        }
    }

    #[test]
    fn fishing_round_trips() {
        for value in all_bytes() {
            let bytes = CgFishing::new(value).encode();
            assert_eq!(CgFishing::decode(&bytes), Ok(CgFishing::new(value)));
        }
    }

    #[test]
    fn warp_round_trips() {
        assert_eq!(CgWarp::decode(&CgWarp::new().encode()), Ok(CgWarp));
    }

    #[test]
    fn every_field_accepts_the_whole_u8_domain() {
        // The legacy handlers switch, multiply, or compare the byte; none of
        // them rejects one. Opacity means all 256 values decode.
        for value in 0..=u8::MAX {
            assert!(CgPosition::decode(&CgPosition::new(value).encode()).is_ok());
            assert!(CgScriptAnswer::decode(&CgScriptAnswer::new(value).encode()).is_ok());
            assert!(CgFishing::decode(&CgFishing::new(value).encode()).is_ok());
        }
    }

    #[test]
    fn boundary_field_values_are_preserved_exactly() {
        // 0 and 255 are the two values a bound or a cast would break.
        for value in [0_u8, 1, 127, 128, 254, u8::MAX] {
            assert_eq!(
                CgPosition::decode(&CgPosition::new(value).encode())
                    .unwrap()
                    .position,
                value
            );
            assert_eq!(
                CgScriptAnswer::decode(&CgScriptAnswer::new(value).encode())
                    .unwrap()
                    .answer,
                value
            );
            assert_eq!(
                CgFishing::decode(&CgFishing::new(value).encode())
                    .unwrap()
                    .dir,
                value
            );
        }
    }

    #[test]
    fn a_short_input_is_truncated() {
        assert_eq!(
            CgPosition::decode(&[]),
            Err(CgMicroError::Truncated {
                needed: 2,
                available: 0
            })
        );
        assert_eq!(
            CgPosition::decode(&[0x1c]),
            Err(CgMicroError::Truncated {
                needed: 2,
                available: 1
            })
        );
        assert_eq!(
            CgScriptAnswer::decode(&[0x1d]),
            Err(CgMicroError::Truncated {
                needed: 2,
                available: 1
            })
        );
        assert_eq!(
            CgFishing::decode(&[0x52]),
            Err(CgMicroError::Truncated {
                needed: 2,
                available: 1
            })
        );
    }

    #[test]
    fn an_empty_input_is_truncated_for_warp() {
        assert_eq!(
            CgWarp::decode(&[]),
            Err(CgMicroError::Truncated {
                needed: 1,
                available: 0
            })
        );
    }

    #[test]
    fn a_long_input_is_a_length_mismatch() {
        assert_eq!(
            CgPosition::decode(&[0x1c, 0x01, 0x02]),
            Err(CgMicroError::LengthMismatch {
                expected: 2,
                actual: 3
            })
        );
        assert_eq!(
            CgScriptAnswer::decode(&[0x1d, 0x01, 0x02, 0x03]),
            Err(CgMicroError::LengthMismatch {
                expected: 2,
                actual: 4
            })
        );
        assert_eq!(
            CgFishing::decode(&[0x52, 0x01, 0x02]),
            Err(CgMicroError::LengthMismatch {
                expected: 2,
                actual: 3
            })
        );
    }

    #[test]
    fn a_trailing_byte_on_warp_is_a_length_mismatch() {
        // The handler ignores the payload, so this is the one record where
        // "the server does not read it" must not become "the codec accepts it".
        assert_eq!(
            CgWarp::decode(&[0x41, 0x00]),
            Err(CgMicroError::LengthMismatch {
                expected: 1,
                actual: 2
            })
        );
    }

    #[test]
    fn a_wrong_header_of_the_right_length_is_rejected() {
        assert_eq!(
            CgPosition::decode(&[0x1d, 0x01]),
            Err(CgMicroError::InvalidHeader {
                expected: 0x1c,
                actual: 0x1d
            })
        );
        assert_eq!(
            CgWarp::decode(&[0x42]),
            Err(CgMicroError::InvalidHeader {
                expected: 0x41,
                actual: 0x42
            })
        );
        assert_eq!(
            CgFishing::decode(&[0x1c, 0x01]),
            Err(CgMicroError::InvalidHeader {
                expected: 0x52,
                actual: 0x1c
            })
        );
    }

    #[test]
    fn length_is_checked_before_the_header() {
        // A short slice whose first byte is also the wrong header must report
        // the length problem, not the header problem. Otherwise a truncated
        // stream is diagnosed as a protocol violation.
        assert!(matches!(
            CgPosition::decode(&[0xff]),
            Err(CgMicroError::Truncated { .. })
        ));
        assert!(matches!(
            CgFishing::decode(&[0x00]),
            Err(CgMicroError::Truncated { .. })
        ));
    }

    #[test]
    fn every_record_rejects_every_other_header_at_its_own_width() {
        for s in frames() {
            for candidate in 0..=u8::MAX {
                if candidate == s.header.value() {
                    continue;
                }
                let mut bytes = s.low.clone();
                bytes[0] = candidate;
                assert_eq!(
                    s.decode(&bytes).err(),
                    Some(CgMicroError::InvalidHeader {
                        expected: s.header.value(),
                        actual: candidate
                    }),
                    "{} accepted header {candidate}",
                    s.name
                );
            }
        }
    }

    #[test]
    fn decode_frame_checks_the_payload_width_and_not_the_record_width() {
        // The Section 151 defect: a frame payload excludes the header, so the
        // payload width is 1 for these records, not 2. Validating the record
        // width here would reject every correct frame.
        for value in all_bytes() {
            let frame = CgPosition::new(value).to_frame();
            assert_eq!(frame.payload.len(), CG_POSITION_PAYLOAD_SIZE);
            assert_eq!(frame.payload.len() + 1, CG_POSITION_WIRE_SIZE);
            assert_eq!(CgPosition::decode_frame(&frame), Ok(CgPosition::new(value)));
        }
    }

    #[test]
    fn decode_frame_agrees_with_decode_for_all_three_byte_records() {
        for value in all_bytes() {
            let position = CgPosition::new(value);
            assert_eq!(
                CgPosition::decode_frame(&position.to_frame()),
                CgPosition::decode(&position.encode())
            );
            let answer = CgScriptAnswer::new(value);
            assert_eq!(
                CgScriptAnswer::decode_frame(&answer.to_frame()),
                CgScriptAnswer::decode(&answer.encode())
            );
            let fishing = CgFishing::new(value);
            assert_eq!(
                CgFishing::decode_frame(&fishing.to_frame()),
                CgFishing::decode(&fishing.encode())
            );
        }
    }

    #[test]
    fn warp_frame_carries_an_empty_payload() {
        let frame = CgWarp::new().to_frame();
        assert!(frame.payload.is_empty());
        assert_eq!(frame.header, 0x41);
        assert_eq!(frame.encoded_len(), Ok(CG_WARP_WIRE_SIZE));
        assert_eq!(CgWarp::decode_frame(&frame), Ok(CgWarp));
    }

    #[test]
    fn warp_frame_with_a_payload_is_rejected() {
        // A one-byte payload must not slip through just because the handler
        // ignores it. CG_WARP_PAYLOAD_SIZE is 0, so 1 is a mismatch.
        let frame = ClientFrame::new(CgWarp::header().value(), [0x00]);
        assert_eq!(
            CgWarp::decode_frame(&frame),
            Err(CgMicroError::LengthMismatch {
                expected: 0,
                actual: 1
            })
        );
    }

    #[test]
    fn decode_frame_rejects_a_wrong_header_at_the_right_payload_width() {
        assert_eq!(
            CgPosition::decode_frame(&ClientFrame::new(0x1d, [0x01])),
            Err(CgMicroError::InvalidHeader {
                expected: 0x1c,
                actual: 0x1d
            })
        );
        assert_eq!(
            CgWarp::decode_frame(&ClientFrame::new(0x40, [0_u8; 0])),
            Err(CgMicroError::InvalidHeader {
                expected: 0x41,
                actual: 0x40
            })
        );
    }

    #[test]
    fn decode_frame_checks_the_payload_width_before_the_header() {
        assert!(matches!(
            CgPosition::decode_frame(&ClientFrame::new(0xff, [0x01, 0x02])),
            Err(CgMicroError::LengthMismatch { .. })
        ));
        assert!(matches!(
            CgScriptAnswer::decode_frame(&ClientFrame::new(0x00, [])),
            Err(CgMicroError::Truncated { .. })
        ));
    }

    #[test]
    fn to_frame_matches_encode_for_every_record() {
        for value in all_bytes() {
            for (frame, encoded) in [
                (
                    CgPosition::new(value).to_frame(),
                    CgPosition::new(value).encode(),
                ),
                (
                    CgScriptAnswer::new(value).to_frame(),
                    CgScriptAnswer::new(value).encode(),
                ),
                (
                    CgFishing::new(value).to_frame(),
                    CgFishing::new(value).encode(),
                ),
            ] {
                let mut rebuilt = vec![frame.header];
                rebuilt.extend_from_slice(&frame.payload);
                assert_eq!(rebuilt, encoded);
                assert_eq!(rebuilt.len(), frame.encoded_len().expect("fixed frame"));
            }
        }
    }

    #[test]
    fn every_encode_into_appends_and_none_of_them_clears() {
        // The Section 152 P14 lesson: one representative append test let a
        // `clear()` in a sibling survive. All four are checked here.
        let seed = [0xaa_u8, 0xbb, 0xcc];

        let mut out = seed.to_vec();
        CgPosition::new(1).encode_into(&mut out);
        assert_eq!(out, [0xaa, 0xbb, 0xcc, 0x1c, 0x01], "position must append");

        let mut out = seed.to_vec();
        CgScriptAnswer::new(2).encode_into(&mut out);
        assert_eq!(
            out,
            [0xaa, 0xbb, 0xcc, 0x1d, 0x02],
            "script answer must append"
        );

        let mut out = seed.to_vec();
        CgWarp::new().encode_into(&mut out);
        assert_eq!(out, [0xaa, 0xbb, 0xcc, 0x41], "warp must append");

        let mut out = seed.to_vec();
        CgFishing::new(3).encode_into(&mut out);
        assert_eq!(out, [0xaa, 0xbb, 0xcc, 0x52, 0x03], "fishing must append");
    }

    #[test]
    fn encode_into_accumulates_in_order() {
        let mut out = Vec::new();
        CgPosition::new(1).encode_into(&mut out);
        CgWarp::new().encode_into(&mut out);
        CgFishing::new(2).encode_into(&mut out);
        CgScriptAnswer::new(3).encode_into(&mut out);
        assert_eq!(out, vec![0x1c, 0x01, 0x41, 0x52, 0x02, 0x1d, 0x03]);
    }

    #[test]
    fn the_four_headers_are_distinct() {
        let headers: BTreeSet<u8> = frames().iter().map(|s| s.header.value()).collect();
        let unique = headers;
        assert_eq!(unique.len(), 4, "two records share a header byte");
    }

    #[test]
    fn no_record_uses_another_records_header() {
        // Cross-feeding a well-formed record of one type into another decoder
        // must fail, which is what the distinct-header check means in practice.
        let position = CgPosition::new(7).encode();
        let answer = CgScriptAnswer::new(7).encode();
        let fishing = CgFishing::new(7).encode();
        assert!(CgScriptAnswer::decode(&position).is_err());
        assert!(CgPosition::decode(&answer).is_err());
        assert!(CgFishing::decode(&answer).is_err());
        assert!(CgPosition::decode(&fishing).is_err());
        // Warp is a different width, so it fails on length before the header.
        assert!(CgWarp::decode(&fishing).is_err());
    }

    #[test]
    fn the_error_messages_name_the_width_and_the_header() {
        assert_eq!(
            CgPosition::decode(&[0x1c]).unwrap_err().to_string(),
            "CG record needs 2 bytes, got 1"
        );
        assert_eq!(
            CgPosition::decode(&[0x1c, 0x00, 0x00])
                .unwrap_err()
                .to_string(),
            "CG record must be exactly 2 bytes, got 3"
        );
        assert_eq!(
            CgPosition::decode(&[0x1d, 0x00]).unwrap_err().to_string(),
            "CG header 29 is not 28"
        );
        assert_eq!(
            CgWarp::decode(&[0x41, 0x00]).unwrap_err().to_string(),
            "CG record must be exactly 1 bytes, got 2"
        );
    }

    #[test]
    fn the_error_type_is_a_standard_error() {
        let boxed: Box<dyn std::error::Error> = Box::new(CgMicroError::Truncated {
            needed: 2,
            available: 0,
        });
        assert!(boxed.to_string().contains("2 bytes"));
    }
}
