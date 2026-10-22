//! `HEADER_GC_WHISPER` (0x22): a private line, from one player to another.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:992-1002`:
//!
//! ```ignore
//! typedef struct packet_whisper
//! {
//!     BYTE   bHeader;
//!     WORD   wSize;
//!     BYTE   bType;
//!     char   szNameFrom[CHARACTER_NAME_MAX_LEN + 1];
//! #if defined(LOCALE_STRING_RENEWAL)
//!     bool   bCanFormat;
//!     packet_whisper() : bCanFormat(true) {}
//! #endif
//! } TPacketGCWhisper;
//! ```
//!
//! `LOCALE_STRING_RENEWAL` is defined in
//! `server/server/common/prodomodefines.h:32`, so `bCanFormat` is on the wire
//! and the fixed part is 1 + 2 + 1 + 25 + 1 = 30 bytes. The text follows the
//! fixed part, and `wSize` counts the whole record.
//!
//! # Measured width, not a hand sum
//!
//! The 30-byte fixed part was measured by compiling the verbatim struct body
//! for i686 (`g++ -m32`) under `static_assert` next to the repository's
//! known-width controls: a packed two-`long` struct must be 8 bytes, which
//! fails on the native 64-bit compiler and passes on the 32-bit one, so the
//! probe is measuring the widths the legacy target uses. The struct holds a
//! `char[25]` and a `WORD` only, so no field's width depends on the target.
//! The i686 cross compiler (`i686-linux-gnu-g++-12`) is not installed on this
//! machine, so the probe used the native compiler's 32-bit mode.
//!
//! # `wSize` counts the whole record
//!
//! Every producer sets it the same way:
//! `pack.wSize = sizeof(TPacketGCWhisper) + len`
//! (`G/char.cpp:12514`, `G/input_main.cpp:458`, `G/input_db.cpp:2468`).
//! The fixed part alone is the size of the records that carry no text
//! (`G/input_main.cpp:394`, `:486`).
//!
//! # The text is raw
//!
//! The producers write the bytes as they are: a relayed line is `buf` with
//! `buflen` bytes and no terminator (`G/input_main.cpp:623-641`), while the
//! system lines carry their terminator, because their length is
//! `strlen(msg) + 1` (`G/input_main.cpp:484`, `G/input_db.cpp:2466`). The
//! Rewrite relays the exact bytes and never transcodes them.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::gc_inventory::HEADER_GC_WHISPER;

/// `sizeof(struct packet_whisper)` with `LOCALE_STRING_RENEWAL` defined.
pub const GC_WHISPER_WIRE_SIZE: usize = 30;

/// `CHARACTER_NAME_MAX_LEN + 1` (`server/server/common/length.h:15`): the
/// name field's width.
pub const GC_WHISPER_NAME_FIELD_SIZE: usize = 25;

/// `WHISPER_TYPE_NORMAL` (`EWhisperType`, `server/server/common/length.h:423`).
pub const WHISPER_TYPE_NORMAL: u8 = 0;
/// `WHISPER_TYPE_NOT_EXIST`.
pub const WHISPER_TYPE_NOT_EXIST: u8 = 1;
/// `WHISPER_TYPE_TARGET_BLOCKED`: the target does not take whispers.
pub const WHISPER_TYPE_TARGET_BLOCKED: u8 = 2;
/// `WHISPER_TYPE_SENDER_BLOCKED`: the sender does not send whispers.
pub const WHISPER_TYPE_SENDER_BLOCKED: u8 = 3;
/// `WHISPER_TYPE_ERROR`: the server refused the line.
pub const WHISPER_TYPE_ERROR: u8 = 4;
/// `WHISPER_TYPE_GM`: the sender is a game master.
pub const WHISPER_TYPE_GM: u8 = 5;
/// `WHISPER_TYPE_OFFLINE`: a line left for the character while it was away.
pub const WHISPER_TYPE_OFFLINE: u8 = 6;
/// `WHISPER_TYPE_SYSTEM`: a line the server says rather than a player.
pub const WHISPER_TYPE_SYSTEM: u8 = 0xff;

/// One game-to-client whisper.
///
/// `name_from` names who the line is from: the sender for a delivered line,
/// and the *target* for the lines the server sends back to the sender
/// (`G/input_main.cpp:396`, `:505`, `:511`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcWhisper {
    /// The `EWhisperType` value.
    pub whisper_type: u8,
    /// The 25 raw name bytes. No NUL is required and none is implied.
    pub name_from: [u8; GC_WHISPER_NAME_FIELD_SIZE],
    /// `bCanFormat`: whether the client should run the text through its markup.
    ///
    /// The legacy constructor leaves it true, and the only producer that sets
    /// it false is behind `__BL_CLIENT_LOCALE_STRING__`, which this tree does
    /// not define.
    pub can_format: bool,
    /// The text bytes, with no terminator of their own.
    pub text: Vec<u8>,
}

impl Default for GcWhisper {
    /// The legacy `packet_whisper` constructor: a line the client may format.
    fn default() -> Self {
        Self {
            whisper_type: WHISPER_TYPE_NORMAL,
            name_from: [0_u8; GC_WHISPER_NAME_FIELD_SIZE],
            can_format: true,
            text: Vec::new(),
        }
    }
}

/// Why a whisper record could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcWhisperError {
    /// The buffer is shorter than the fixed part.
    Truncated {
        /// Bytes actually present.
        actual: usize,
    },
    /// The leading byte is not [`HEADER_GC_WHISPER`].
    Header {
        /// The byte that was found.
        actual: u8,
    },
    /// `wSize` is smaller than the fixed part.
    Size {
        /// The declared total.
        size: u16,
    },
}

impl fmt::Display for GcWhisperError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { actual } => write!(
                formatter,
                "a whisper record needs {GC_WHISPER_WIRE_SIZE} bytes, got {actual}"
            ),
            Self::Header { actual } => {
                write!(formatter, "expected a whisper record, got byte {actual}")
            }
            Self::Size { size } => {
                write!(
                    formatter,
                    "whisper size {size} is under {GC_WHISPER_WIRE_SIZE}"
                )
            }
        }
    }
}

impl Error for GcWhisperError {}

impl GcWhisper {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_WHISPER.value()
    }

    /// The whole record's length on the wire, which is what `wSize` holds.
    #[must_use]
    pub fn wire_size(&self) -> usize {
        GC_WHISPER_WIRE_SIZE + self.text.len()
    }

    /// Appends the whole packed record to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`GcWhisperError::Size`] when the record would not fit the
    /// `WORD` size field, which for this header means a text longer than
    /// 65,525 bytes. That is far past `CHAT_MAX_LEN`, so in practice the check
    /// is unreachable; it is here so no input can wrap the size word.
    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), GcWhisperError> {
        let size =
            u16::try_from(self.wire_size()).map_err(|_| GcWhisperError::Size { size: u16::MAX })?;
        out.push(Self::header());
        out.extend_from_slice(&size.to_le_bytes());
        out.push(self.whisper_type);
        out.extend_from_slice(&self.name_from);
        out.push(u8::from(self.can_format));
        out.extend_from_slice(&self.text);
        Ok(())
    }

    /// Encodes to a fresh buffer.
    ///
    /// # Errors
    ///
    /// As [`GcWhisper::encode_into`].
    pub fn encode(&self) -> Result<Vec<u8>, GcWhisperError> {
        let mut out = Vec::with_capacity(self.wire_size());
        self.encode_into(&mut out)?;
        Ok(out)
    }

    /// Decodes the whole packed record from the start of `bytes`.
    ///
    /// Trailing bytes after `size` are left alone, as they are for every other
    /// record in this crate.
    ///
    /// # Errors
    ///
    /// Returns [`GcWhisperError::Truncated`] when the buffer is shorter than
    /// the record's own `size`, [`GcWhisperError::Header`] when the leading
    /// byte is not [`HEADER_GC_WHISPER`], and [`GcWhisperError::Size`] when
    /// `size` is smaller than the fixed part.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcWhisperError> {
        if bytes.len() < GC_WHISPER_WIRE_SIZE {
            return Err(GcWhisperError::Truncated {
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header() {
            return Err(GcWhisperError::Header { actual: bytes[0] });
        }
        let size = u16::from_le_bytes([bytes[1], bytes[2]]);
        if usize::from(size) < GC_WHISPER_WIRE_SIZE {
            return Err(GcWhisperError::Size { size });
        }
        let size = usize::from(size);
        if bytes.len() < size {
            return Err(GcWhisperError::Truncated {
                actual: bytes.len(),
            });
        }

        let mut name_from = [0_u8; GC_WHISPER_NAME_FIELD_SIZE];
        name_from.copy_from_slice(&bytes[4..4 + GC_WHISPER_NAME_FIELD_SIZE]);

        Ok(Self {
            whisper_type: bytes[3],
            name_from,
            can_format: bytes[29] != 0,
            text: bytes[GC_WHISPER_WIRE_SIZE..size].to_vec(),
        })
    }

    /// The name, up to the first NUL.
    ///
    /// The field is a C string in every producer (`strlcpy` into it), so the
    /// name ends at the first NUL.
    #[must_use]
    pub fn sender_name(&self) -> &[u8] {
        let end = self
            .name_from
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(self.name_from.len());
        &self.name_from[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &[u8]) -> [u8; GC_WHISPER_NAME_FIELD_SIZE] {
        let mut field = [0_u8; GC_WHISPER_NAME_FIELD_SIZE];
        field[..text.len()].copy_from_slice(text);
        field
    }

    #[test]
    fn the_fixed_part_is_thirty_packed_bytes() {
        assert_eq!(GC_WHISPER_WIRE_SIZE, 30);
        assert_eq!(GC_WHISPER_NAME_FIELD_SIZE, 25);
    }

    #[test]
    fn the_header_is_thirty_four() {
        assert_eq!(GcWhisper::header(), 0x22);
    }

    #[test]
    fn a_line_without_text_is_thirty_bytes() {
        let record = GcWhisper {
            whisper_type: WHISPER_TYPE_NOT_EXIST,
            name_from: name(b"Yankee"),
            ..GcWhisper::default()
        };
        let bytes = record.encode().expect("the record fits");
        assert_eq!(bytes.len(), 30);
        assert_eq!(bytes[0], 0x22);
        assert_eq!(u16::from_le_bytes([bytes[1], bytes[2]]), 30);
        assert_eq!(bytes[3], WHISPER_TYPE_NOT_EXIST);
    }

    #[test]
    fn decodes_a_line_from_golden_bytes() {
        // header 22 | size 20 00 | type 00 | "Yankee" | can format 01 | "hi"
        let mut bytes = vec![0x22, 0x20, 0x00, WHISPER_TYPE_NORMAL];
        bytes.extend_from_slice(&name(b"Yankee"));
        bytes.push(1);
        bytes.extend_from_slice(b"hi");
        let decoded = GcWhisper::decode(&bytes).expect("a valid record");
        assert_eq!(decoded.whisper_type, WHISPER_TYPE_NORMAL);
        assert_eq!(decoded.sender_name(), b"Yankee");
        assert!(decoded.can_format);
        assert_eq!(decoded.text, b"hi");
    }

    #[test]
    fn the_size_word_is_little_endian() {
        let mut bytes = vec![0x22, 0x02, 0x01, WHISPER_TYPE_NORMAL];
        bytes.extend_from_slice(&name(b"Bob"));
        bytes.push(1);
        bytes.extend(std::iter::repeat(b'a').take(258));
        let decoded = GcWhisper::decode(&bytes).expect("a valid record");
        assert_eq!(decoded.text.len(), 258 - GC_WHISPER_WIRE_SIZE);
    }

    #[test]
    fn every_type_byte_survives() {
        for value in 0..=u8::MAX {
            let record = GcWhisper {
                whisper_type: value,
                ..GcWhisper::default()
            };
            let bytes = record.encode().expect("the record fits");
            let decoded = GcWhisper::decode(&bytes).expect("a valid record");
            assert_eq!(decoded.whisper_type, value, "type {value} did not survive");
        }
    }

    #[test]
    fn the_can_format_byte_is_the_legacy_bool() {
        for value in [0_u8, 1] {
            let record = GcWhisper {
                can_format: value != 0,
                ..GcWhisper::default()
            };
            let bytes = record.encode().expect("the record fits");
            assert_eq!(bytes[29], value);
        }
    }

    #[test]
    fn a_short_size_is_refused() {
        let mut bytes = vec![0x22, 0x1d, 0x00];
        bytes.extend(std::iter::repeat(0_u8).take(27));
        let error = GcWhisper::decode(&bytes).expect_err("the size is under 30");
        assert_eq!(error, GcWhisperError::Size { size: 29 });
    }

    #[test]
    fn a_foreign_header_is_refused() {
        let mut bytes = vec![0x04, 0x1e, 0x00];
        bytes.extend(std::iter::repeat(0_u8).take(27));
        let error = GcWhisper::decode(&bytes).expect_err("the header is not a whisper");
        assert_eq!(error, GcWhisperError::Header { actual: 0x04 });
    }

    #[test]
    fn a_truncated_record_is_refused() {
        let bytes = vec![0x22, 0x1e, 0x00];
        let error = GcWhisper::decode(&bytes).expect_err("the record is short");
        assert_eq!(error, GcWhisperError::Truncated { actual: 3 });
    }

    #[test]
    fn the_name_stops_at_a_nul() {
        let mut field = [0_u8; GC_WHISPER_NAME_FIELD_SIZE];
        field[0] = b'B';
        field[1] = b'o';
        field[2] = b'b';
        let record = GcWhisper {
            name_from: field,
            ..GcWhisper::default()
        };
        let bytes = record.encode().expect("the record fits");
        let decoded = GcWhisper::decode(&bytes).expect("a valid record");
        assert_eq!(decoded.sender_name(), b"Bob");
    }

    #[test]
    fn the_record_round_trips() {
        let record = GcWhisper {
            whisper_type: WHISPER_TYPE_GM,
            name_from: name(b"Yankee"),
            can_format: true,
            text: b"hello".to_vec(),
        };
        let bytes = record.encode().expect("the record fits");
        let decoded = GcWhisper::decode(&bytes).expect("a valid record");
        assert_eq!(decoded, record);
    }
}
