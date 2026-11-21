//! Transport-free codecs for the two 66-byte client quest-text requests.
//!
//! | record | header | wire | payload | declaration | registration |
//! |---|---|---|---|---|---|
//! | `TPacketCGQuestInputString` | 30 `0x1e` | 66 | 65 | `packet.h:758-762` | `packet_info.cpp` |
//! | `SPacketCGRequestEventQuest` | 32 `0x20` | 66 | 65 | `packet.h:3488-3492` | `packet_info.cpp` |
//!
//! Both are **a header plus a 65-byte `char` buffer**. They are grouped in one
//! module because they share a shape, not because the source shares a
//! declaration -- it does not.
//!
//! # The two buffers are equal by coincidence, and nothing ties them together
//!
//! ```cpp
//! typedef struct command_quest_input_string
//! {
//!     BYTE header;
//!     char msg[64+1];
//! } TPacketCGQuestInputString;
//!
//! typedef struct command_request_event_quest
//! {
//!     BYTE    bHeader;
//!     char    szName[QUEST_NAME_MAX_NUM + 1];
//! } SPacketCGRequestEventQuest;
//! ```
//!
//! `64+1` is a **literal**; `QUEST_NAME_MAX_NUM + 1` is a named constant that
//! happens to be 64 at `common/length.h:43`. So the two widths are equal
//! *today*, and **change `QUEST_NAME_MAX_NUM` and they diverge with no compile
//! error anywhere.** That is the reason this module defines one shared constant
//! and tests the arithmetic for both records: the test is the thing that will
//! fail when a constant moves, which is the whole point.
//!
//! # The same shape, two different safety properties
//!
//! Both handlers read their buffer as a C string, and they do not agree on how.
//!
//! `CInputMain::QuestInputString` at `input_main.cpp:2226-2234` is **bounded**:
//!
//! ```cpp
//! char msg[65];
//! strlcpy(msg, p->msg, sizeof(msg));
//! ```
//!
//! `strlcpy` copies at most 64 characters and always NUL-terminates, so an
//! all-65-nonzero buffer is safely truncated rather than overread.
//!
//! `CInputMain::RequestEventQuest` at `input_main.cpp:5459-5464` is **not**:
//!
//! ```cpp
//! quest::CQuestManager::instance().RequestEventQuest(p->szName, ch->GetPlayerID());
//! ```
//!
//! The raw pointer is handed straight to the quest manager as a `const char *`
//! with no length bound. A 65-byte buffer with no NUL is an overread, the same
//! hazard `check_name` presents in the change-name handler.
//!
//! **Neither of those belongs in this codec.** The rule from the other fixed
//! string records applies unchanged: a legacy `char[N]` field is raw fixed
//! storage, so the codec exposes `[u8; 65]` with no `&str`, no `CStr`, and no
//! NUL validation. Adding one would reject records the legacy server accepts, and
//! `RequestEventQuest` in particular is the path where being stricter would
//! change who can start a quest. The unsafe read is a **legacy C++ finding to
//! record, not a rewrite requirement to enforce.**

use crate::cg_inventory::{CgHeader, HEADER_CG_QUEST_INPUT_STRING, HEADER_CG_REQUEST_EVENT_QUEST};
use crate::cg_wire::ClientFrame;

/// The shared fixed text field width: `64 + 1`, and `QUEST_NAME_MAX_NUM + 1`.
///
/// Both records are `1 + CG_QUEST_TEXT_FIELD_SIZE`. See the module
/// documentation for why the equality is a property of the active constants and
/// not a shared declaration.
pub const CG_QUEST_TEXT_FIELD_SIZE: usize = 64 + 1;

/// The full legacy `TPacketCGQuestInputString` record, header byte included.
pub const CG_QUEST_INPUT_STRING_WIRE_SIZE: usize = 1 + CG_QUEST_TEXT_FIELD_SIZE;
/// The framed payload of `TPacketCGQuestInputString`.
pub const CG_QUEST_INPUT_STRING_PAYLOAD_SIZE: usize = CG_QUEST_INPUT_STRING_WIRE_SIZE - 1;

/// The full legacy `SPacketCGRequestEventQuest` record, header byte included.
pub const CG_REQUEST_EVENT_QUEST_WIRE_SIZE: usize = 1 + CG_QUEST_TEXT_FIELD_SIZE;
/// The framed payload of `SPacketCGRequestEventQuest`.
pub const CG_REQUEST_EVENT_QUEST_PAYLOAD_SIZE: usize = CG_REQUEST_EVENT_QUEST_WIRE_SIZE - 1;

/// Every way the two quest-text decoders can refuse a byte slice.
///
/// Both records are the same shape, so they share one error type the same way the
/// three fixed-buffer records in `cg_name` share `CgNameError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgQuestTextError {
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

/// The client-to-server quest input string.
///
/// Field order is the header byte at offset 0, then a raw 65-byte
/// [`CG_QUEST_TEXT_FIELD_SIZE`] buffer at offset 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgQuestInputString {
    /// The legacy `msg[65]` buffer as raw fixed storage. No NUL is required,
    /// none is searched for, and no text accessor is offered.
    pub msg: [u8; CG_QUEST_TEXT_FIELD_SIZE],
}

/// The client-to-server event quest request by name.
///
/// Field order is the header byte at offset 0, then a raw 65-byte
/// [`CG_QUEST_TEXT_FIELD_SIZE`] buffer at offset 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgRequestEventQuest {
    /// The legacy `szName[65]` buffer as raw fixed storage. The legacy handler
    /// passes this straight to the quest manager as a `const char *` with no
    /// length bound, so a non-NUL-terminated 65-byte buffer is representable
    /// here exactly as it is representable there.
    pub sz_name: [u8; CG_QUEST_TEXT_FIELD_SIZE],
}

/// Build a [`CgQuestInputString`] from a 65-byte buffer.
pub const fn quest_input_string_from(msg: [u8; CG_QUEST_TEXT_FIELD_SIZE]) -> CgQuestInputString {
    CgQuestInputString { msg }
}

/// Build a [`CgRequestEventQuest`] from a 65-byte buffer.
pub const fn request_event_quest_from(
    sz_name: [u8; CG_QUEST_TEXT_FIELD_SIZE],
) -> CgRequestEventQuest {
    CgRequestEventQuest { sz_name }
}

impl CgQuestInputString {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_QUEST_INPUT_STRING
    }

    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_QUEST_INPUT_STRING_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_QUEST_INPUT_STRING_PAYLOAD_SIZE;

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.extend_from_slice(&self.msg);
    }

    /// Encode to a fresh 66-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode the header-less payload to a fresh 65-byte buffer.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame {
            header: Self::header().value(),
            payload: self.msg.to_vec(),
        }
    }

    /// # Errors
    ///
    /// [`CgQuestTextError::Truncated`] below 66 bytes,
    /// [`CgQuestTextError::LengthMismatch`] above,
    /// [`CgQuestTextError::InvalidHeader`] for a full-length slice not starting
    /// with 30.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgQuestTextError> {
        check_exact(bytes.len(), Self::WIRE_SIZE)?;
        check_header(bytes[0], Self::header().value())?;
        let mut msg = [0_u8; CG_QUEST_TEXT_FIELD_SIZE];
        msg.copy_from_slice(&bytes[1..]);
        Ok(Self { msg })
    }

    /// # Errors
    ///
    /// As [`CgQuestInputString::decode`], except that the payload must be
    /// exactly 65 bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgQuestTextError> {
        check_exact(frame.payload.len(), Self::PAYLOAD_SIZE)?;
        check_header(frame.header, Self::header().value())?;
        let mut msg = [0_u8; CG_QUEST_TEXT_FIELD_SIZE];
        msg.copy_from_slice(&frame.payload);
        Ok(Self { msg })
    }
}

impl CgRequestEventQuest {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_REQUEST_EVENT_QUEST
    }

    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_REQUEST_EVENT_QUEST_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_REQUEST_EVENT_QUEST_PAYLOAD_SIZE;

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.extend_from_slice(&self.sz_name);
    }

    /// Encode to a fresh 66-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode the header-less payload to a fresh 65-byte buffer.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame {
            header: Self::header().value(),
            payload: self.sz_name.to_vec(),
        }
    }

    /// # Errors
    ///
    /// [`CgQuestTextError::Truncated`] below 66 bytes,
    /// [`CgQuestTextError::LengthMismatch`] above,
    /// [`CgQuestTextError::InvalidHeader`] for a full-length slice not starting
    /// with 32.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgQuestTextError> {
        check_exact(bytes.len(), Self::WIRE_SIZE)?;
        check_header(bytes[0], Self::header().value())?;
        let mut sz_name = [0_u8; CG_QUEST_TEXT_FIELD_SIZE];
        sz_name.copy_from_slice(&bytes[1..]);
        Ok(Self { sz_name })
    }

    /// # Errors
    ///
    /// As [`CgRequestEventQuest::decode`], except that the payload must be
    /// exactly 65 bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgQuestTextError> {
        check_exact(frame.payload.len(), Self::PAYLOAD_SIZE)?;
        check_header(frame.header, Self::header().value())?;
        let mut sz_name = [0_u8; CG_QUEST_TEXT_FIELD_SIZE];
        sz_name.copy_from_slice(&frame.payload);
        Ok(Self { sz_name })
    }
}

/// Reject anything that is not exactly `expected` bytes wide.
fn check_exact(actual: usize, expected: usize) -> Result<(), CgQuestTextError> {
    if actual < expected {
        return Err(CgQuestTextError::Truncated {
            needed: expected,
            available: actual,
        });
    }
    if actual > expected {
        return Err(CgQuestTextError::LengthMismatch { expected, actual });
    }
    Ok(())
}

/// Reject a header byte that is not `expected`.
fn check_header(actual: u8, expected: u8) -> Result<(), CgQuestTextError> {
    if actual != expected {
        return Err(CgQuestTextError::InvalidHeader { expected, actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const QIS: u8 = 30;
    const REQ: u8 = 32;

    fn field(fill: u8) -> [u8; CG_QUEST_TEXT_FIELD_SIZE] {
        [fill; CG_QUEST_TEXT_FIELD_SIZE]
    }

    #[test]
    fn the_shared_field_is_sixty_five_bytes() {
        assert_eq!(CG_QUEST_TEXT_FIELD_SIZE, 65);
        assert_eq!(64 + 1, CG_QUEST_TEXT_FIELD_SIZE);
    }

    #[test]
    fn both_records_are_sixty_six_bytes() {
        assert_eq!(CgQuestInputString::WIRE_SIZE, 66);
        assert_eq!(CgRequestEventQuest::WIRE_SIZE, 66);
        assert_eq!(CgQuestInputString::PAYLOAD_SIZE, 65);
        assert_eq!(CgRequestEventQuest::PAYLOAD_SIZE, 65);
    }

    #[test]
    fn both_records_share_the_field_width() {
        // The assertion that will fail if either legacy constant moves.
        assert_eq!(1 + CG_QUEST_TEXT_FIELD_SIZE, CgQuestInputString::WIRE_SIZE);
        assert_eq!(1 + CG_QUEST_TEXT_FIELD_SIZE, CgRequestEventQuest::WIRE_SIZE);
    }

    #[test]
    fn the_headers_are_30_and_32() {
        assert_eq!(CgQuestInputString::header().value(), QIS);
        assert_eq!(CgRequestEventQuest::header().value(), REQ);
    }

    #[test]
    fn the_two_headers_are_distinct() {
        assert_ne!(
            CgQuestInputString::header().value(),
            CgRequestEventQuest::header().value()
        );
    }

    #[test]
    fn quest_input_string_encodes_to_the_exact_legacy_bytes() {
        let rec = quest_input_string_from(field(0x41));
        let bytes = rec.encode();
        assert_eq!(bytes.len(), 66);
        assert_eq!(bytes[0], QIS);
        assert_eq!(&bytes[1..], &[0x41; 65]);
    }

    #[test]
    fn request_event_quest_encodes_to_the_exact_legacy_bytes() {
        let rec = request_event_quest_from(field(0x42));
        let bytes = rec.encode();
        assert_eq!(bytes.len(), 66);
        assert_eq!(bytes[0], REQ);
        assert_eq!(&bytes[1..], &[0x42; 65]);
    }

    #[test]
    fn both_round_trip() {
        let a = quest_input_string_from(field(0x11));
        assert_eq!(CgQuestInputString::decode(&a.encode()).unwrap(), a);
        let b = request_event_quest_from(field(0x22));
        assert_eq!(CgRequestEventQuest::decode(&b.encode()).unwrap(), b);
    }

    #[test]
    fn both_round_trip_through_a_frame() {
        let a = quest_input_string_from(field(0x11));
        let fa = a.to_frame();
        assert_eq!(fa.header, QIS);
        assert_eq!(fa.payload.len(), 65);
        assert_eq!(CgQuestInputString::decode_frame(&fa).unwrap(), a);

        let b = request_event_quest_from(field(0x22));
        let fb = b.to_frame();
        assert_eq!(fb.header, REQ);
        assert_eq!(fb.payload.len(), 65);
        assert_eq!(CgRequestEventQuest::decode_frame(&fb).unwrap(), b);
    }

    #[test]
    fn the_frame_payload_is_the_record_minus_the_header() {
        let a = quest_input_string_from(field(0x33));
        assert_eq!(&a.to_frame().payload[..], &a.encode()[1..]);
        let b = request_event_quest_from(field(0x44));
        assert_eq!(&b.to_frame().payload[..], &b.encode()[1..]);
    }

    #[test]
    fn a_buffer_with_no_nul_is_representable() {
        // The RequestEventQuest handler passes this pointer to the quest
        // manager as a `const char *` with no length bound, so a 65-byte
        // non-NUL-terminated field is a real wire value, not an error.
        let rec = request_event_quest_from(field(0xFF));
        let back = CgRequestEventQuest::decode(&rec.encode()).unwrap();
        assert_eq!(back.sz_name, field(0xFF));
        assert!(!back.sz_name.contains(&0));
    }

    #[test]
    fn a_nul_terminated_field_is_representable() {
        let mut f = field(0x00);
        f[0] = b'a';
        f[1] = b'b';
        f[2] = 0;
        let rec = request_event_quest_from(f);
        assert_eq!(
            CgRequestEventQuest::decode(&rec.encode()).unwrap().sz_name,
            f
        );
    }

    #[test]
    fn a_nul_in_the_first_byte_is_representable() {
        // An empty string is a legitimate value; it must not be confused with
        // truncation.
        let rec = quest_input_string_from(field(0));
        assert_eq!(rec.msg, [0_u8; 65]);
        assert_eq!(CgQuestInputString::decode(&rec.encode()).unwrap(), rec);
    }

    #[test]
    fn the_two_records_are_not_interchangeable() {
        let a = quest_input_string_from(field(0x41));
        assert!(CgRequestEventQuest::decode(&a.encode()).is_err());
        let b = request_event_quest_from(field(0x42));
        assert!(CgQuestInputString::decode(&b.encode()).is_err());
    }

    #[test]
    fn the_field_is_read_from_offset_one() {
        let mut bytes = quest_input_string_from(field(0)).encode();
        bytes[1] = 0xEE;
        assert_eq!(CgQuestInputString::decode(&bytes).unwrap().msg[0], 0xEE);
        let mut bytes = request_event_quest_from(field(0)).encode();
        bytes[1] = 0xEE;
        assert_eq!(
            CgRequestEventQuest::decode(&bytes).unwrap().sz_name[0],
            0xEE
        );
    }

    #[test]
    fn the_last_field_byte_is_read() {
        let mut f = field(0);
        f[64] = 0x7E;
        let rec = request_event_quest_from(f);
        assert_eq!(
            CgRequestEventQuest::decode(&rec.encode()).unwrap().sz_name[64],
            0x7E
        );
    }

    #[test]
    fn rejects_every_short_length_for_both() {
        let a = quest_input_string_from(field(0x01));
        let b = request_event_quest_from(field(0x02));
        for len in 0..66 {
            let mut ba = a.encode();
            ba.truncate(len);
            assert_eq!(
                CgQuestInputString::decode(&ba).unwrap_err(),
                CgQuestTextError::Truncated {
                    needed: 66,
                    available: len
                },
                "input len {len}"
            );
            let mut bb = b.encode();
            bb.truncate(len);
            assert_eq!(
                CgRequestEventQuest::decode(&bb).unwrap_err(),
                CgQuestTextError::Truncated {
                    needed: 66,
                    available: len
                },
                "event len {len}"
            );
        }
    }

    #[test]
    fn rejects_every_long_length_for_both() {
        let a = quest_input_string_from(field(0x01));
        let b = request_event_quest_from(field(0x02));
        for extra in 1..=3 {
            let mut ba = a.encode();
            ba.extend(std::iter::repeat_n(0u8, extra));
            assert_eq!(
                CgQuestInputString::decode(&ba).unwrap_err(),
                CgQuestTextError::LengthMismatch {
                    expected: 66,
                    actual: 66 + extra
                }
            );
            let mut bb = b.encode();
            bb.extend(std::iter::repeat_n(0u8, extra));
            assert_eq!(
                CgRequestEventQuest::decode(&bb).unwrap_err(),
                CgQuestTextError::LengthMismatch {
                    expected: 66,
                    actual: 66 + extra
                }
            );
        }
    }

    #[test]
    fn rejects_the_other_records_header() {
        let a = quest_input_string_from(field(0x01));
        let b = request_event_quest_from(field(0x02));
        assert!(CgRequestEventQuest::decode(&a.encode()).is_err());
        assert!(CgQuestInputString::decode(&b.encode()).is_err());
    }

    #[test]
    fn rejects_every_wrong_frame_length() {
        for len in 0..65 {
            let f = ClientFrame {
                header: QIS,
                payload: vec![0; len],
            };
            assert!(CgQuestInputString::decode_frame(&f).is_err(), "len {len}");
            let f = ClientFrame {
                header: REQ,
                payload: vec![0; len],
            };
            assert!(CgRequestEventQuest::decode_frame(&f).is_err(), "len {len}");
        }
    }

    #[test]
    fn rejects_a_wrong_frame_header() {
        let f = ClientFrame {
            header: 99,
            payload: field(0x01).to_vec(),
        };
        assert_eq!(
            CgQuestInputString::decode_frame(&f).unwrap_err(),
            CgQuestTextError::InvalidHeader {
                expected: 30,
                actual: 99
            }
        );
    }

    #[test]
    fn an_empty_input_reports_each_records_own_width() {
        let empty: [u8; 0] = [];
        assert_eq!(
            CgQuestInputString::decode(&empty).unwrap_err(),
            CgQuestTextError::Truncated {
                needed: 66,
                available: 0
            }
        );
        assert_eq!(
            CgRequestEventQuest::decode(&empty).unwrap_err(),
            CgQuestTextError::Truncated {
                needed: 66,
                available: 0
            }
        );
    }

    #[test]
    fn encode_into_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE, 0xAD];
        quest_input_string_from(field(0x01)).encode_into(&mut out);
        assert_eq!(out.len(), 2 + 66);
        assert_eq!(
            &out[2..],
            &quest_input_string_from(field(0x01)).encode()[..]
        );

        let mut out = vec![0xBE, 0xEF];
        request_event_quest_from(field(0x02)).encode_into(&mut out);
        assert_eq!(out.len(), 2 + 66);
    }

    #[test]
    fn a_distinct_byte_sweep_finds_exactly_65_field_bytes() {
        // Each field byte is read independently, so mutating any one of the 65
        // positions must change the decoded record.
        let base = quest_input_string_from(field(0x00));
        let mut changed = 0;
        for i in 0..CG_QUEST_TEXT_FIELD_SIZE {
            let mut f = field(0x00);
            f[i] = 0x01;
            let rec = quest_input_string_from(f);
            if rec != base {
                changed += 1;
            }
        }
        assert_eq!(changed, 65);
    }
}
