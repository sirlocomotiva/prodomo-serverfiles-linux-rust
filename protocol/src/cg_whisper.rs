//! Transport-free codec for the legacy `TPacketCGWhisper` record.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:587-592`:
//!
//! ```ignore
//! typedef struct command_whisper
//! {
//!     BYTE  bHeader;
//!     WORD  wSize;
//!     char  szNameTo[CHARACTER_NAME_MAX_LEN + 1];
//! } TPacketCGWhisper;
//! ```
//!
//! The fixed part is 1 + 2 + 25 = **28 packed bytes**, and the text follows it
//! raw. The legacy registration is the same width
//! (`Set(HEADER_CG_WHISPER, sizeof(TPacketCGWhisper), "Whisper")`,
//! `server/server/game/packet_info.cpp:112`), and `protocol/src/cg_inventory.rs`
//! pins that base size at 28.
//!
//! # Measured width, not a hand sum
//!
//! The 28-byte fixed part was measured by compiling the verbatim struct body
//! for i686 (`g++ -m32`) under `static_assert` next to the repository's
//! known-width controls: a packed two-`long` struct must be 8 bytes, which
//! fails on the native 64-bit compiler and passes on the 32-bit one, so the
//! probe is measuring the widths the legacy target uses. The struct holds a
//! `char[25]` and a `WORD` only, so no field's width depends on the target.
//! The i686 cross compiler (`i686-linux-gnu-g++-12`) is not installed on this
//! machine, so the probe used the native compiler's 32-bit mode.
//!
//! # `size` is the whole record and it is a floor, not an equality
//!
//! `CInputMain::Whisper` (`server/server/game/input_main.cpp:340-354`) does:
//!
//! ```ignore
//! if (uiBytes < pinfo->wSize)
//!     return -1;
//!
//! int iExtraLen = pinfo->wSize - sizeof(TPacketCGWhisper);
//!
//! if (iExtraLen < 0)
//! {
//!     sys_err("invalid packet length (len %d size %u buffer %u)", ...);
//!     ch->GetDesc()->SetPhase(PHASE_CLOSE);
//!     return -1;
//! }
//! ```
//!
//! Three consequences this codec preserves, because a stricter one would
//! reject bytes the legacy server accepts:
//!
//! 1. `uiBytes < size` is the only rejected relation, so **trailing bytes past
//!    `size` are legal and ignored**. A frame is not required to be exactly
//!    `size` long, and this decoder does not require it.
//! 2. `size` below 28 is a distinct refusal that **closes the descriptor**, not
//!    a decode error that drops a frame. It is reported as its own error so
//!    the caller can reproduce the close.
//! 3. The text is bounded by `size - 28`, not by the frame length.
//!
//! # The text stops at a NUL
//!
//! The copy is `strlcpy` into a `char buf[CHAT_MAX_LEN + 1]`
//! (`server/server/game/input_main.cpp:536-537`), and `input_main.cpp:538`
//! immediately takes `strlen(buf)` of the result, so a NUL ends the line. This
//! decoder truncates at the first NUL and bounds the result at
//! `CHAT_MAX_LEN` = 512 bytes.
//!
//! # The text is raw
//!
//! Legacy never transcodes it: `buf` is copied byte for byte and handed to
//! `ProcessTextTag`, then written into the record as it is. The Rewrite relays
//! the exact bytes and never decodes them as UTF-8 or requires a terminator.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::cg_inventory::HEADER_CG_WHISPER;
use crate::cg_wire::{ClientFrame, CLIENT_FRAME_HEADER_SIZE};

/// `sizeof(TPacketCGWhisper)`: the one-byte header, the length word, and the
/// name field.
pub const CG_WHISPER_WIRE_SIZE: usize = 28;

/// `CHARACTER_NAME_MAX_LEN + 1` (`server/server/common/length.h:15`): the name
/// field's width.
pub const CG_WHISPER_NAME_FIELD_SIZE: usize = 25;

/// `CHAT_MAX_LEN` (`server/server/common/length.h:49`): the longest text the
/// handler's buffer holds, less its terminator.
pub const CG_WHISPER_TEXT_MAX: usize = 512;

/// One client-to-game whisper.
///
/// The `to` field is the name of the character the line is for. Legacy reads it
/// as a C string (`CHARACTER_MANAGER::instance().FindPC(pinfo->szNameTo)`,
/// `server/server/game/input_main.cpp:370`), so the name is the bytes up to the
/// first NUL; the whole field is kept so a client that fills it without a
/// terminator is not silently read past.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgWhisper {
    /// The `size` the client declared, which counts the header.
    pub declared_size: u16,
    /// The 25 raw name bytes, the target's name.
    pub to: [u8; CG_WHISPER_NAME_FIELD_SIZE],
    /// The text bytes, already bounded by `size - 28` and by
    /// [`CG_WHISPER_TEXT_MAX`], and already stopped at the first NUL.
    pub text: Vec<u8>,
}

/// Why a whisper frame could not be read as a `TPacketCGWhisper`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgWhisperError {
    /// The frame carried fewer than 27 payload bytes, so the fixed part could
    /// not be read at all.
    TruncatedPrefix {
        /// Bytes of payload actually present.
        available: usize,
    },
    /// `size` was below `sizeof(TPacketCGWhisper)`.
    ///
    /// Legacy logs `invalid packet length` and calls `SetPhase(PHASE_CLOSE)`.
    /// The caller must close; dropping the frame is not parity.
    DeclaredSizeTooSmall {
        /// The declared total.
        declared_size: u16,
    },
}

impl fmt::Display for CgWhisperError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedPrefix { available } => {
                write!(
                    formatter,
                    "a whisper frame needs 27 payload bytes, got {available}"
                )
            }
            Self::DeclaredSizeTooSmall { declared_size } => {
                write!(
                    formatter,
                    "whisper size {declared_size} is under {CG_WHISPER_WIRE_SIZE}"
                )
            }
        }
    }
}

impl Error for CgWhisperError {}

impl CgWhisper {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_CG_WHISPER.value()
    }

    /// Read the record from a decoded frame.
    ///
    /// The frame payload **excludes** the header byte, so `size` is read from
    /// payload bytes 0 and 1, the name from payload bytes 2 to 27, and the
    /// text from payload byte 27.
    ///
    /// # Errors
    ///
    /// Returns [`CgWhisperError::TruncatedPrefix`] when the frame is too short
    /// to hold the prefix, and [`CgWhisperError::DeclaredSizeTooSmall`] when
    /// `size` is under [`CG_WHISPER_WIRE_SIZE`]. The latter is the close case,
    /// so it is kept distinct from a frame this codec simply could not read.
    pub fn decode(frame: &ClientFrame) -> Result<Self, CgWhisperError> {
        if frame.header != Self::header() {
            // The variable-frame resolver already routed by header, so reaching
            // here means the caller passed the wrong frame; treat it as a
            // truncated prefix rather than inventing a third error the legacy
            // handler has no arm for.
            return Err(CgWhisperError::TruncatedPrefix {
                available: frame.payload.len(),
            });
        }
        if frame.payload.len() < 2 {
            return Err(CgWhisperError::TruncatedPrefix {
                available: frame.payload.len(),
            });
        }

        // Legacy computes `iExtraLen` and refuses before it ever reads the
        // name, so a short frame that declares a short size still reaches the
        // close.
        let declared_size = u16::from_le_bytes([frame.payload[0], frame.payload[1]]);
        if usize::from(declared_size) < CG_WHISPER_WIRE_SIZE {
            return Err(CgWhisperError::DeclaredSizeTooSmall { declared_size });
        }

        if frame.payload.len() < CG_WHISPER_WIRE_SIZE - CLIENT_FRAME_HEADER_SIZE {
            return Err(CgWhisperError::TruncatedPrefix {
                available: frame.payload.len(),
            });
        }

        let mut to = [0_u8; CG_WHISPER_NAME_FIELD_SIZE];
        to.copy_from_slice(&frame.payload[2..2 + CG_WHISPER_NAME_FIELD_SIZE]);

        let text_len = usize::from(declared_size) - CG_WHISPER_WIRE_SIZE;
        // `size` bounds the text; the frame may be longer, and the extra bytes
        // are ignored exactly as `strlcpy` ignores them.
        let available = &frame.payload[2 + CG_WHISPER_NAME_FIELD_SIZE..];
        let capped = text_len.min(available.len()).min(CG_WHISPER_TEXT_MAX);
        let raw = &available[..capped];
        let text = match raw.iter().position(|&byte| byte == 0) {
            Some(nul) => raw[..nul].to_vec(),
            None => raw.to_vec(),
        };

        Ok(Self {
            declared_size,
            to,
            text,
        })
    }

    /// The target's name, up to the first NUL.
    ///
    /// Legacy reads `szNameTo` as a C string
    /// (`server/server/game/input_main.cpp:370`), so the name ends at the
    /// first NUL. A field with no NUL is read whole, which is the only reading
    /// that keeps the lookup inside the field.
    #[must_use]
    pub fn target_name(&self) -> &[u8] {
        let end = self
            .to
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(self.to.len());
        &self.to[..end]
    }

    /// Build the frame a legacy client would send for this whisper.
    #[must_use]
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_WHISPER_WIRE_SIZE - CLIENT_FRAME_HEADER_SIZE);
        let total = self.text.len() + CG_WHISPER_WIRE_SIZE;
        payload.extend_from_slice(&u16::try_from(total).unwrap_or(u16::MAX).to_le_bytes());
        payload.extend_from_slice(&self.to);
        payload.extend_from_slice(&self.text);
        ClientFrame::new(Self::header(), payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(payload: &[u8]) -> ClientFrame {
        ClientFrame::new(CgWhisper::header(), payload)
    }

    fn name(text: &[u8]) -> [u8; CG_WHISPER_NAME_FIELD_SIZE] {
        let mut field = [0_u8; CG_WHISPER_NAME_FIELD_SIZE];
        field[..text.len()].copy_from_slice(text);
        field
    }

    #[test]
    fn the_fixed_part_is_twenty_eight_packed_bytes() {
        assert_eq!(CG_WHISPER_WIRE_SIZE, 28);
        assert_eq!(CG_WHISPER_NAME_FIELD_SIZE, 25);
    }

    #[test]
    fn the_header_is_nineteen() {
        assert_eq!(CgWhisper::header(), 19);
    }

    #[test]
    fn decodes_a_whisper_from_golden_bytes() {
        // header 13 | size 1e 00 | "Yankee" | "hi"
        let mut bytes = vec![0x13, 0x1e, 0x00];
        bytes.extend_from_slice(&name(b"Yankee"));
        bytes.extend_from_slice(b"hi");
        let decoded = CgWhisper::decode(&frame(&bytes[1..])).expect("a valid whisper");
        assert_eq!(decoded.declared_size, 30);
        assert_eq!(decoded.target_name(), b"Yankee");
        assert_eq!(decoded.text, b"hi");
    }

    #[test]
    fn the_size_word_is_little_endian() {
        let mut payload = vec![0x02, 0x01];
        payload.extend_from_slice(&name(b"Bob"));
        payload.extend(std::iter::repeat(b'a').take(258));
        let decoded = CgWhisper::decode(&frame(&payload)).expect("a valid whisper");
        assert_eq!(decoded.declared_size, 0x0102);
        assert_eq!(usize::from(decoded.declared_size), 258);
        assert_eq!(decoded.text.len(), 258 - CG_WHISPER_WIRE_SIZE);
    }

    #[test]
    fn trailing_bytes_past_the_declared_size_are_ignored() {
        // size 30 means 2 text bytes; the frame carries 5.
        let mut payload = vec![30, 0];
        payload.extend_from_slice(&name(b"Bob"));
        payload.extend_from_slice(b"abcde");
        let decoded = CgWhisper::decode(&frame(&payload)).expect("a valid whisper");
        assert_eq!(decoded.text, b"ab");
    }

    #[test]
    fn a_short_declared_size_is_its_own_refusal() {
        let payload = [27, 0];
        let error = CgWhisper::decode(&frame(&payload)).expect_err("the size is under 28");
        assert_eq!(
            error,
            CgWhisperError::DeclaredSizeTooSmall { declared_size: 27 }
        );
    }

    #[test]
    fn a_truncated_prefix_is_reported() {
        // The size is 30, so the frame is read for the name and the text, and
        // two payload bytes are not enough.
        let error = CgWhisper::decode(&frame(&[30, 0])).expect_err("the prefix is short");
        assert_eq!(error, CgWhisperError::TruncatedPrefix { available: 2 });
    }

    #[test]
    fn a_short_size_is_refused_before_the_frame_is_read() {
        // Legacy computes `iExtraLen` and refuses before it reads the name, so
        // a frame that is too short and declares a short size still closes.
        let error = CgWhisper::decode(&frame(&[0, 0])).expect_err("the size is under 28");
        assert_eq!(
            error,
            CgWhisperError::DeclaredSizeTooSmall { declared_size: 0 }
        );
    }

    #[test]
    fn the_text_stops_at_a_nul() {
        let mut payload = vec![32, 0];
        payload.extend_from_slice(&name(b"Bob"));
        payload.extend_from_slice(b"ab\0cd");
        let decoded = CgWhisper::decode(&frame(&payload)).expect("a valid whisper");
        assert_eq!(decoded.text, b"ab");
    }

    #[test]
    fn the_name_stops_at_a_nul() {
        let mut field = [0_u8; CG_WHISPER_NAME_FIELD_SIZE];
        field[0] = b'B';
        field[1] = b'o';
        field[2] = b'b';
        let mut payload = vec![30, 0];
        payload.extend_from_slice(&field);
        payload.extend_from_slice(b"hi");
        let decoded = CgWhisper::decode(&frame(&payload)).expect("a valid whisper");
        assert_eq!(decoded.target_name(), b"Bob");
    }

    #[test]
    fn every_name_byte_survives() {
        for value in 0..=u8::MAX {
            let mut field = [0_u8; CG_WHISPER_NAME_FIELD_SIZE];
            field[0] = value;
            let mut payload = vec![30, 0];
            payload.extend_from_slice(&field);
            payload.extend_from_slice(b"hi");
            let decoded = CgWhisper::decode(&frame(&payload)).expect("a valid whisper");
            assert_eq!(decoded.to[0], value, "name byte {value} did not survive");
        }
    }

    #[test]
    fn the_frame_round_trips() {
        let record = CgWhisper {
            declared_size: 0,
            to: name(b"Yankee"),
            text: b"hello".to_vec(),
        };
        let frame = record.to_frame();
        let decoded = CgWhisper::decode(&frame).expect("a valid whisper");
        assert_eq!(decoded.target_name(), b"Yankee");
        assert_eq!(decoded.text, b"hello");
    }
}
