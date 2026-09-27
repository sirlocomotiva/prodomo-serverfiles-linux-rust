//! `HEADER_GC_CHAT` (4): a line of text the client shows in a chat box.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:979-990`:
//!
//! ```ignore
//! typedef struct packet_chat
//! {
//!     BYTE   header;
//!     WORD   size;
//!     BYTE   type;
//!     DWORD  id;
//!     BYTE   bEmpire;
//! #if defined(LOCALE_STRING_RENEWAL)
//!     bool   bCanFormat;
//!     packet_chat() : bCanFormat(true) {}
//! #endif
//! } TPacketGCChat;
//! ```
//!
//! `LOCALE_STRING_RENEWAL` is defined in
//! `server/server/common/prodomodefines.h:32`, so `bCanFormat` is on the wire
//! and the fixed part is 1 + 2 + 1 + 4 + 1 + 1 = 10 bytes. The producer is
//! `CHARACTER::ChatPacket` (`server/server/game/char.cpp:5140-5187`), which sets
//! `size = sizeof(struct packet_chat) + len`, leaves `id` at 0, takes `bEmpire`
//! from the descriptor, and writes exactly `len` bytes of text with no
//! terminator.
//!
//! # Measured width, not a hand sum
//!
//! The 10-byte fixed part was measured by compiling the verbatim struct body
//! with `g++ -m32` under `static_assert` next to the repository's known-width
//! controls; the probe is in `.scratch/ledger187/root-probe/`. A 64-bit reading
//! would be 10 here only by luck, so the control that pins a packed two-`long`
//! struct at 8 bytes is what makes the number trustworthy.
//!
//! # Text bytes
//!
//! The text is raw. It is not UTF-8, not NUL-terminated, and not rounded up to
//! a field; it is the bytes `vsnprintf` produced, with the locale substitution
//! already applied. The client renders them as they arrive, so the Rewrite
//! relays the exact bytes and never transcodes.
//!
//! # A legacy defect, not reproduced
//!
//! `ChatPacket` ignores the fact that `vsnprintf` returns the length the text
//! *would* have had. A message longer than `CHAT_MAX_LEN` (512) therefore makes
//! `buf.write(chatbuf, len)` read past the end of the stack buffer
//! (`server/server/game/char.cpp:5169-5186`). The Rewrite refuses a text that
//! does not fit instead of over-reading.

use std::fmt;

use crate::gc_inventory::HEADER_GC_CHAT;

/// `sizeof(struct packet_chat)` with `LOCALE_STRING_RENEWAL` defined.
pub const GC_CHAT_WIRE_SIZE: usize = 10;

/// `CHAT_MAX_LEN` (`server/server/common/length.h:49`): the longest text
/// `CHARACTER::ChatPacket` can build.
pub const CHAT_MAX_LEN: usize = 512;

/// `CHAT_TYPE_TALKING` (`EChatType`, `server/server/common/length.h:404`).
pub const CHAT_TYPE_TALKING: u8 = 0;
/// `CHAT_TYPE_INFO`.
pub const CHAT_TYPE_INFO: u8 = 1;
/// `CHAT_TYPE_NOTICE`: a line the server says rather than a player.
pub const CHAT_TYPE_NOTICE: u8 = 2;
/// `CHAT_TYPE_PARTY`.
pub const CHAT_TYPE_PARTY: u8 = 3;
/// `CHAT_TYPE_GUILD`.
pub const CHAT_TYPE_GUILD: u8 = 4;
/// `CHAT_TYPE_COMMAND`: a line the client runs as a command.
pub const CHAT_TYPE_COMMAND: u8 = 5;
/// `CHAT_TYPE_SHOUT`.
pub const CHAT_TYPE_SHOUT: u8 = 6;
/// `CHAT_TYPE_WHISPER`.
pub const CHAT_TYPE_WHISPER: u8 = 7;
/// `CHAT_TYPE_BIG_NOTICE`.
pub const CHAT_TYPE_BIG_NOTICE: u8 = 8;
/// `CHAT_TYPE_MONARCH_NOTICE`.
pub const CHAT_TYPE_MONARCH_NOTICE: u8 = 9;
/// `CHAT_TYPE_MAX_NUM` with `ENABLE_DICE_SYSTEM` undefined, which is this tree.
pub const CHAT_TYPE_MAX_NUM: u8 = 10;

/// `HEADER_GC_CHAT` (4): a text line for the client to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcChat {
    /// The `EChatType` value.
    pub chat_type: u8,
    /// `id`: the speaker's VID, or 0 for a server line.
    pub id: u32,
    /// `bEmpire`: the speaker's empire, from the descriptor.
    pub empire: u8,
    /// `bCanFormat`: whether the client should run the text through its markup.
    pub can_format: bool,
    /// The text bytes, with no terminator and no padding.
    pub text: Vec<u8>,
}

impl Default for GcChat {
    /// The legacy `packet_chat` constructor: a line the client may format.
    fn default() -> Self {
        Self {
            chat_type: CHAT_TYPE_TALKING,
            id: 0,
            empire: 0,
            can_format: true,
            text: Vec::new(),
        }
    }
}

impl GcChat {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_CHAT.value()
    }

    /// A server line of the given type, as `CHARACTER::ChatPacket` builds one.
    ///
    /// `id` stays 0 and `bCanFormat` stays true, as the legacy constructor and
    /// producer leave them.
    ///
    /// # Errors
    ///
    /// Returns [`GcChatError::TextTooLong`] when the text is longer than
    /// [`CHAT_MAX_LEN`], which the legacy producer would over-read.
    pub fn notice(chat_type: u8, empire: u8, text: &[u8]) -> Result<Self, GcChatError> {
        if text.len() > CHAT_MAX_LEN {
            return Err(GcChatError::TextTooLong { len: text.len() });
        }
        Ok(Self {
            chat_type,
            id: 0,
            empire,
            can_format: true,
            text: text.to_vec(),
        })
    }

    /// The whole record's length on the wire, which is what `size` holds.
    #[must_use]
    pub fn wire_size(&self) -> usize {
        GC_CHAT_WIRE_SIZE + self.text.len()
    }

    /// Appends the whole packed record to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`GcChatError::TextTooLong`] when the record would not fit the
    /// `WORD` size field, which for this header means a text longer than 65,525
    /// bytes. That is far past [`CHAT_MAX_LEN`], so in practice the check is
    /// unreachable; it is here so no input can wrap the size word.
    pub fn encode(&self) -> Result<Vec<u8>, GcChatError> {
        let mut out = Vec::with_capacity(self.wire_size());
        self.encode_into(&mut out)?;
        Ok(out)
    }

    /// Appends the whole packed record to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`GcChatError::TextTooLong`] when the record would not fit the
    /// `WORD` size field, which for this header means a text longer than 65,525
    /// bytes. That is far past [`CHAT_MAX_LEN`], so in practice the check is
    /// unreachable; it is here so no input can wrap the size word.
    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), GcChatError> {
        let size = u16::try_from(self.wire_size()).map_err(|_| GcChatError::TextTooLong {
            len: self.text.len(),
        })?;
        out.push(Self::header());
        out.extend_from_slice(&size.to_le_bytes());
        out.push(self.chat_type);
        out.extend_from_slice(&self.id.to_le_bytes());
        out.push(self.empire);
        out.push(u8::from(self.can_format));
        out.extend_from_slice(&self.text);
        Ok(())
    }

    /// Decodes the whole packed record from the start of `bytes`.
    ///
    /// Trailing bytes after `size` are left alone, as they are for every other
    /// record in this crate.
    ///
    /// # Errors
    ///
    /// Returns [`GcChatError::Truncated`] when the buffer is shorter than the
    /// record's own `size`, [`GcChatError::Header`] when the leading byte is not
    /// [`HEADER_GC_CHAT`], and [`GcChatError::Size`] when `size` is smaller than
    /// the fixed part.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcChatError> {
        if bytes.len() < GC_CHAT_WIRE_SIZE {
            return Err(GcChatError::Truncated {
                context: "GcChat",
                needed: GC_CHAT_WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header() {
            return Err(GcChatError::Header {
                context: "GcChat",
                expected: Self::header(),
                actual: bytes[0],
            });
        }
        let size = usize::from(u16::from_le_bytes([bytes[1], bytes[2]]));
        if size < GC_CHAT_WIRE_SIZE {
            return Err(GcChatError::Size { size });
        }
        if bytes.len() < size {
            return Err(GcChatError::Truncated {
                context: "GcChat",
                needed: size,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            chat_type: bytes[3],
            id: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            empire: bytes[8],
            can_format: bytes[9] != 0,
            text: bytes[GC_CHAT_WIRE_SIZE..size].to_vec(),
        })
    }
}

/// What a [`GcChat`] decode or encode can report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcChatError {
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
    /// `size` is smaller than the fixed part.
    Size {
        /// The `size` the record carried.
        size: usize,
    },
    /// The text is longer than the record can carry.
    TextTooLong {
        /// The text length that was refused.
        len: usize,
    },
}

impl fmt::Display for GcChatError {
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
            Self::Size { size } => {
                write!(
                    f,
                    "GcChat: size {size} is under the {GC_CHAT_WIRE_SIZE}-byte fixed part"
                )
            }
            Self::TextTooLong { len } => write!(
                f,
                "GcChat: {len} bytes of text exceed the {CHAT_MAX_LEN}-byte limit or the \
                 16-bit size field"
            ),
        }
    }
}

impl std::error::Error for GcChatError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The width the i386 probe measured. See the module docs.
    const MEASURED_WIRE_SIZE: usize = 10;

    /// The chat types, in `EChatType` order with `ENABLE_DICE_SYSTEM` undefined.
    const TYPES: [(u8, &str); 10] = [
        (CHAT_TYPE_TALKING, "TALKING"),
        (CHAT_TYPE_INFO, "INFO"),
        (CHAT_TYPE_NOTICE, "NOTICE"),
        (CHAT_TYPE_PARTY, "PARTY"),
        (CHAT_TYPE_GUILD, "GUILD"),
        (CHAT_TYPE_COMMAND, "COMMAND"),
        (CHAT_TYPE_SHOUT, "SHOUT"),
        (CHAT_TYPE_WHISPER, "WHISPER"),
        (CHAT_TYPE_BIG_NOTICE, "BIG_NOTICE"),
        (CHAT_TYPE_MONARCH_NOTICE, "MONARCH_NOTICE"),
    ];

    /// The golden bytes, taken from the source field order and not from the encoder.
    fn golden(text: &[u8], chat_type: u8, id: u32, empire: u8, can_format: bool) -> Vec<u8> {
        let mut want = vec![4];
        let total = u16::try_from(MEASURED_WIRE_SIZE + text.len()).expect("a test line is short");
        want.extend_from_slice(&total.to_le_bytes());
        want.push(chat_type);
        want.extend_from_slice(&id.to_le_bytes());
        want.push(empire);
        want.push(u8::from(can_format));
        want.extend_from_slice(text);
        want
    }

    #[test]
    fn the_fixed_width_is_the_measured_i386_one() {
        assert_eq!(GC_CHAT_WIRE_SIZE, MEASURED_WIRE_SIZE);
        // 1 header + 2 size + 1 type + 4 id + 1 empire + 1 can_format.
        assert_eq!(1 + 2 + 1 + 4 + 1 + 1, MEASURED_WIRE_SIZE);
    }

    /// The enumerator numbers read out of `EChatType` (`server/server/common/length.h:404`).
    /// The list is written out rather than derived, so a renumbered constant fails here
    /// instead of quietly agreeing with itself.
    const ENUMERATORS: [(u8, u8); 10] = [
        (0, CHAT_TYPE_TALKING),
        (1, CHAT_TYPE_INFO),
        (2, CHAT_TYPE_NOTICE),
        (3, CHAT_TYPE_PARTY),
        (4, CHAT_TYPE_GUILD),
        (5, CHAT_TYPE_COMMAND),
        (6, CHAT_TYPE_SHOUT),
        (7, CHAT_TYPE_WHISPER),
        (8, CHAT_TYPE_BIG_NOTICE),
        (9, CHAT_TYPE_MONARCH_NOTICE),
    ];

    #[test]
    fn the_chat_types_are_the_legacy_enumerators() {
        for (want, value) in ENUMERATORS {
            assert_eq!(value, want);
        }
        assert_eq!(
            TYPES.len(),
            usize::from(CHAT_TYPE_MAX_NUM),
            "one name per type"
        );
        assert_eq!(CHAT_TYPE_NOTICE, 2);
        assert_eq!(CHAT_TYPE_COMMAND, 5);
        assert_eq!(CHAT_TYPE_MAX_NUM, 10);
    }

    #[test]
    fn encodes_the_golden_bytes() {
        // Distinct byte halves, so an endianness slip cannot cancel out.
        let chat = GcChat {
            chat_type: CHAT_TYPE_NOTICE,
            id: 0x0102_0304,
            empire: 2,
            can_format: true,
            text: b"Hello".to_vec(),
        };
        let mut got = Vec::new();
        chat.encode_into(&mut got).expect("fits");
        assert_eq!(got, golden(b"Hello", 2, 0x0102_0304, 2, true));
        assert_eq!(got.len(), 15);
    }

    #[test]
    fn a_server_line_leaves_id_zero_and_formatting_on() {
        let chat = GcChat::notice(CHAT_TYPE_COMMAND, 3, b"ConsoleEnable").expect("fits");
        assert_eq!(chat.id, 0);
        assert!(chat.can_format);
        let mut got = Vec::new();
        chat.encode_into(&mut got).expect("fits");
        assert_eq!(got, golden(b"ConsoleEnable", 5, 0, 3, true));
        assert_eq!(got.len(), 23);
    }

    #[test]
    fn an_empty_text_is_a_ten_byte_record() {
        let mut got = Vec::new();
        GcChat::default().encode_into(&mut got).expect("fits");
        assert_eq!(got, golden(b"", CHAT_TYPE_TALKING, 0, 0, true));
    }

    #[test]
    fn round_trips_and_leaves_trailing_bytes_alone() {
        let chat = GcChat {
            chat_type: CHAT_TYPE_WHISPER,
            id: 0x0a0b_0c0d,
            empire: 1,
            can_format: false,
            text: b"psst".to_vec(),
        };
        let mut bytes = Vec::new();
        chat.encode_into(&mut bytes).expect("fits");
        bytes.extend_from_slice(&[0xbb; 8]);
        assert_eq!(GcChat::decode(&bytes).expect("reads"), chat);
    }

    /// The field is a C++ `bool`, so the encoder normalises it to 0 or 1 and the
    /// decoder treats every other byte as true. Legacy only ever writes 1
    /// (`packet_chat`'s constructor), so no byte is ever lost by the normalise.
    #[test]
    fn the_can_format_byte_normalises_and_reads_every_value() {
        for raw in 0u8..=255 {
            let mut bytes = Vec::new();
            GcChat::notice(CHAT_TYPE_INFO, 0, b"x")
                .expect("fits")
                .encode_into(&mut bytes)
                .expect("fits");
            bytes[9] = raw;
            assert_eq!(GcChat::decode(&bytes).expect("reads").can_format, raw != 0);
        }
        for (value, written) in [(false, 0u8), (true, 1u8)] {
            let mut bytes = Vec::new();
            GcChat {
                chat_type: CHAT_TYPE_INFO,
                id: 0,
                empire: 0,
                can_format: value,
                text: b"x".to_vec(),
            }
            .encode_into(&mut bytes)
            .expect("fits");
            assert_eq!(bytes[9], written);
        }
    }

    #[test]
    fn every_chat_type_byte_round_trips() {
        for raw in 0u8..=255 {
            let chat = GcChat {
                chat_type: raw,
                id: 0,
                empire: 0,
                can_format: true,
                text: Vec::new(),
            };
            let mut bytes = Vec::new();
            chat.encode_into(&mut bytes).expect("fits");
            assert_eq!(GcChat::decode(&bytes).expect("reads"), chat);
        }
    }

    #[test]
    fn the_text_is_raw_bytes() {
        // Legacy writes what vsnprintf produced, so a non-UTF-8 line is legal and
        // must not be refused or replaced.
        let raw: &[u8] = &[0x00, 0xff, 0x80, b' ', 0xc3, 0x28, 0x0a];
        let chat = GcChat::notice(CHAT_TYPE_NOTICE, 0, raw).expect("fits");
        let mut bytes = Vec::new();
        chat.encode_into(&mut bytes).expect("fits");
        assert_eq!(&bytes[GC_CHAT_WIRE_SIZE..], raw);
        assert_eq!(GcChat::decode(&bytes).expect("reads").text, raw);
    }

    #[test]
    fn rejects_a_foreign_header() {
        let mut bytes = golden(b"x", 2, 0, 0, true);
        bytes[0] = 0x03;
        assert!(matches!(
            GcChat::decode(&bytes),
            Err(GcChatError::Header { actual: 0x03, .. })
        ));
    }

    #[test]
    fn rejects_a_buffer_shorter_than_the_record_claims() {
        let bytes = golden(b"Hello", 2, 0, 0, true);
        for cut in 0..bytes.len() {
            assert!(
                matches!(
                    GcChat::decode(&bytes[..cut]),
                    Err(GcChatError::Truncated { .. })
                ),
                "a {cut}-byte prefix must not decode"
            );
        }
    }

    #[test]
    fn rejects_a_size_under_the_fixed_part() {
        for size in 0..u16::try_from(MEASURED_WIRE_SIZE).expect("the fixed part fits a u16") {
            let mut bytes = golden(b"x", 2, 0, 0, true);
            bytes[1..3].copy_from_slice(&size.to_le_bytes());
            assert!(
                matches!(GcChat::decode(&bytes), Err(GcChatError::Size { size: got }) if got == usize::from(size)),
                "size {size} must be refused"
            );
        }
    }

    #[test]
    fn refuses_text_the_legacy_producer_would_over_read() {
        let long = vec![b'x'; CHAT_MAX_LEN + 1];
        assert!(matches!(
            GcChat::notice(CHAT_TYPE_NOTICE, 0, &long),
            Err(GcChatError::TextTooLong { len }) if len == CHAT_MAX_LEN + 1
        ));
        // The last text the legacy producer can build is still fine.
        assert!(GcChat::notice(CHAT_TYPE_NOTICE, 0, &vec![b'x'; CHAT_MAX_LEN]).is_ok());
    }
}
