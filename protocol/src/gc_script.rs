//! `HEADER_GC_SCRIPT` (45): a quest dialog, as the text the client's script window reads.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:1677-1683`, inside the file's `#pragma pack(1)`:
//!
//! ```ignore
//! struct packet_script
//! {
//!     BYTE    header;
//!     WORD    size;
//!     BYTE    skin;
//!     WORD    src_size;
//! };
//! ```
//!
//! The producer is `CQuestManager::SendScript`
//! (`server/server/game/questmanager.cpp:1113-1151`). It writes the six header bytes and then the
//! script's bytes, with no terminating NUL, so `size = src_size + 6`. The script is the text the
//! quest built with `say`, `select` and the rest (`[ENTER]`, `[QUESTION ...]`, `[NEXT]`,
//! `[DONE]`); the codec keeps it raw.
//!
//! # A script too long for the `WORD` fields
//!
//! Legacy assigns `m_strScript.size()` to both `WORD` fields, so a script longer than 65529 bytes
//! is sent with sizes that have wrapped around and the client misreads the stream. That is a
//! Defect; the encoder refuses such a script instead.

use std::fmt;

use crate::gc_inventory::HEADER_GC_SCRIPT;

/// `sizeof(struct packet_script)`: header, total size, skin and script size.
pub const GC_SCRIPT_WIRE_SIZE: usize = 6;

/// The longest script whose record `size` still fits its `WORD`: 65535 less the header.
pub const GC_SCRIPT_MAX_LEN: usize = 65_529;

/// `HEADER_GC_SCRIPT` (45): the script window's text and its skin.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GcScript {
    /// `skin`: the window style (`QUEST_SKIN_NOWINDOW` 0, `QUEST_SKIN_NORMAL` 1, and so on).
    pub skin: u8,
    /// The script's bytes.
    pub script: Vec<u8>,
}

impl GcScript {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_SCRIPT.value()
    }

    /// Build the record for a skin and a script.
    #[must_use]
    pub fn new(skin: u8, script: Vec<u8>) -> Self {
        Self { skin, script }
    }

    /// The whole record's length on the wire, which is what `size` holds.
    #[must_use]
    pub fn wire_size(&self) -> usize {
        GC_SCRIPT_WIRE_SIZE + self.script.len()
    }

    /// Appends the whole packed record to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`GcScriptError::TooLong`] when the script is longer than
    /// [`GC_SCRIPT_MAX_LEN`], so `size` would not fit its `WORD` field.
    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), GcScriptError> {
        let too_long = GcScriptError::TooLong {
            len: self.script.len(),
        };
        let size = u16::try_from(self.wire_size()).map_err(|_| too_long)?;
        let src_size = u16::try_from(self.script.len()).map_err(|_| too_long)?;
        out.push(Self::header());
        out.extend_from_slice(&size.to_le_bytes());
        out.push(self.skin);
        out.extend_from_slice(&src_size.to_le_bytes());
        out.extend_from_slice(&self.script);
        Ok(())
    }

    /// Decodes the whole packed record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcScriptError::Truncated`] when the buffer is shorter than the record's own
    /// `size`, [`GcScriptError::Header`] when the leading byte is not [`HEADER_GC_SCRIPT`], and
    /// [`GcScriptError::Size`] when `size` is not `6 + src_size`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcScriptError> {
        if bytes.len() < GC_SCRIPT_WIRE_SIZE {
            return Err(GcScriptError::Truncated {
                needed: GC_SCRIPT_WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header() {
            return Err(GcScriptError::Header {
                expected: Self::header(),
                actual: bytes[0],
            });
        }
        let size = usize::from(u16::from_le_bytes([bytes[1], bytes[2]]));
        let src_size = usize::from(u16::from_le_bytes([bytes[4], bytes[5]]));
        if size != GC_SCRIPT_WIRE_SIZE + src_size {
            return Err(GcScriptError::Size { size, src_size });
        }
        if bytes.len() < size {
            return Err(GcScriptError::Truncated {
                needed: size,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            skin: bytes[3],
            script: bytes[GC_SCRIPT_WIRE_SIZE..size].to_vec(),
        })
    }
}

/// What a [`GcScript`] decode or encode can report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcScriptError {
    /// The buffer ended before the record did.
    Truncated {
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
    /// `size` does not agree with `src_size`.
    Size {
        /// The `size` the record carried.
        size: usize,
        /// The `src_size` it carried.
        src_size: usize,
    },
    /// The script is too long for the `WORD` size fields.
    TooLong {
        /// The script length that was refused.
        len: usize,
    },
}

impl fmt::Display for GcScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, actual } => {
                write!(f, "GcScript: need {needed} bytes, buffer held {actual}")
            }
            Self::Header { expected, actual } => write!(
                f,
                "GcScript: header {actual:#04x}, expected {expected:#04x}"
            ),
            Self::Size { size, src_size } => write!(
                f,
                "GcScript: size {size} is not {GC_SCRIPT_WIRE_SIZE} plus a {src_size}-byte script"
            ),
            Self::TooLong { len } => write!(
                f,
                "GcScript: a {len}-byte script does not fit the 16-bit sizes \
                 (at most {GC_SCRIPT_MAX_LEN})"
            ),
        }
    }
}

impl std::error::Error for GcScriptError {}

#[cfg(test)]
mod tests {
    use super::*;

    const MEASURED_HEADER: usize = 1 + 2 + 1 + 2;

    /// The golden bytes, taken from the source field order and not from the encoder.
    fn golden(skin: u8, script: &[u8]) -> Vec<u8> {
        let mut want = vec![45];
        let size = u16::try_from(MEASURED_HEADER + script.len()).expect("a test script is short");
        want.extend_from_slice(&size.to_le_bytes());
        want.push(skin);
        let src_size = u16::try_from(script.len()).expect("a test script is short");
        want.extend_from_slice(&src_size.to_le_bytes());
        want.extend_from_slice(script);
        want
    }

    const QUESTION: &[u8] = b"[QUESTION 1;OX Contest |2;Inchide]";

    #[test]
    fn the_header_is_the_packed_six_bytes() {
        assert_eq!(GC_SCRIPT_WIRE_SIZE, MEASURED_HEADER);
        assert_eq!(GcScript::header(), 0x2d);
        assert_eq!(
            GC_SCRIPT_MAX_LEN,
            usize::from(u16::MAX) - GC_SCRIPT_WIRE_SIZE
        );
        assert_eq!(GcScript::new(1, QUESTION.to_vec()).wire_size(), 6 + 34);
    }

    #[test]
    fn encodes_the_golden_bytes() {
        let mut got = Vec::new();
        GcScript::new(1, QUESTION.to_vec())
            .encode_into(&mut got)
            .expect("fits");
        assert_eq!(got, golden(1, QUESTION));
        assert_eq!(&got[..6], &[0x2d, 40, 0, 1, 34, 0]);
    }

    #[test]
    fn an_empty_script_is_a_six_byte_record() {
        let mut got = Vec::new();
        GcScript::default().encode_into(&mut got).expect("fits");
        assert_eq!(got, golden(0, b""));
        assert_eq!(GcScript::decode(&got).expect("reads"), GcScript::default());
    }

    #[test]
    fn round_trips_and_leaves_trailing_bytes_alone() {
        let record = GcScript::new(0, b"[DONE]".to_vec());
        let mut bytes = golden(0, b"[DONE]");
        bytes.extend_from_slice(&[0xcc; 8]);
        assert_eq!(GcScript::decode(&bytes).expect("reads"), record);
    }

    #[test]
    fn a_script_keeps_its_nuls_and_high_bytes() {
        let raw = vec![0, 0xff, b'[', 0x80, 0];
        let record = GcScript::new(5, raw.clone());
        let mut bytes = Vec::new();
        record.encode_into(&mut bytes).expect("fits");
        assert_eq!(&bytes[6..], &raw[..]);
        assert_eq!(GcScript::decode(&bytes).expect("reads"), record);
    }

    #[test]
    fn rejects_a_foreign_header() {
        let mut bytes = golden(1, QUESTION);
        bytes[0] = 0x2e;
        assert!(matches!(
            GcScript::decode(&bytes),
            Err(GcScriptError::Header { actual: 0x2e, .. })
        ));
    }

    #[test]
    fn rejects_a_buffer_shorter_than_the_record_claims() {
        let bytes = golden(1, QUESTION);
        for cut in 0..bytes.len() {
            assert!(
                matches!(
                    GcScript::decode(&bytes[..cut]),
                    Err(GcScriptError::Truncated { .. })
                ),
                "a {cut}-byte prefix must not decode"
            );
        }
    }

    #[test]
    fn rejects_a_size_that_does_not_match_the_script_size() {
        for (size, src_size) in [(5u16, 0u16), (7, 0), (6, 1), (40, 33), (40, 35)] {
            let mut bytes = golden(1, QUESTION);
            bytes[1..3].copy_from_slice(&size.to_le_bytes());
            bytes[4..6].copy_from_slice(&src_size.to_le_bytes());
            assert!(
                matches!(
                    GcScript::decode(&bytes),
                    Err(GcScriptError::Size { size: got_size, src_size: got_src })
                        if got_size == usize::from(size) && got_src == usize::from(src_size)
                ),
                "size {size} with src_size {src_size} must be refused"
            );
        }
    }

    #[test]
    fn refuses_a_script_too_long_for_the_size_fields() {
        let longest = GcScript::new(1, vec![b'a'; GC_SCRIPT_MAX_LEN]);
        let mut bytes = Vec::new();
        longest.encode_into(&mut bytes).expect("fits");
        assert_eq!(bytes.len(), usize::from(u16::MAX));
        assert_eq!(GcScript::decode(&bytes).expect("reads"), longest);
        let too_long = GcScript::new(1, vec![b'a'; GC_SCRIPT_MAX_LEN + 1]);
        let mut out = Vec::new();
        assert_eq!(
            too_long.encode_into(&mut out),
            Err(GcScriptError::TooLong {
                len: GC_SCRIPT_MAX_LEN + 1
            })
        );
        assert!(out.is_empty());
    }
}
