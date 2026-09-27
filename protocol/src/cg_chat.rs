//! Transport-free codec for the legacy `TPacketCGChat` record.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:579-584`:
//!
//! ```ignore
//! typedef struct command_chat
//! {
//!     BYTE   header;
//!     WORD   size;
//!     BYTE   type;
//! } TPacketCGChat;
//! ```
//!
//! The fixed part is 1 + 2 + 1 = **4 packed bytes**, and the text follows it
//! raw. `packet_info.cpp` registers the record as a 4-byte base with a
//! self-declared length, which is why the base size in
//! `protocol/src/cg_inventory.rs` is 4 and why
//! `protocol::cg_variable::three_byte_prefix_declared_size` reads the total
//! from the wire rather than from a table.
//!
//! # Measured width, not a hand sum
//!
//! The 4-byte fixed part was measured by compiling the verbatim struct body for
//! i686 under `static_assert` next to the repository's known-width controls. The
//! native `g++` the probe used agrees, and the control that pins a packed
//! two-`long` struct at 8 bytes is what makes the native reading trustworthy: a
//! 64-bit `long` would make this struct 8 bytes and every offset after `header`
//! would move.
//!
//! # `size` is the whole record and it is a floor, not an equality
//!
//! `CInputMain::Chat` (`server/server/game/input_main.cpp:785-799`) does:
//!
//! ```ignore
//! if (uiBytes < pinfo->size)
//!     return -1;
//! const int iExtraLen = pinfo->size - sizeof(TPacketCGChat);
//! if (iExtraLen < 0)
//! { ...; ch->GetDesc()->SetPhase(PHASE_CLOSE); return -1; }
//! strlcpy(buf, data + sizeof(TPacketCGChat), MIN(iExtraLen + 1, sizeof(buf)));
//! ```
//!
//! Three consequences this codec preserves, because a stricter one would reject
//! bytes the legacy server accepts:
//!
//! 1. `uiBytes < size` is the only rejected relation, so **trailing bytes past
//!    `size` are legal and ignored**. A frame is not required to be exactly
//!    `size` long, and this decoder does not require it.
//! 2. `size` below 4 is a distinct refusal that **closes the descriptor**, not
//!    a decode error that drops a frame. It is reported as its own error so the
//!    caller can reproduce the close.
//! 3. The text is bounded by `size - 4`, not by the frame length.
//!
//! # The text stops at a NUL
//!
//! The copy is `strlcpy`, and the only `strlcpy` in the tree is the Windows
//! macro at `server/server/libthecore/stdafx.h:50`, which maps to
//! `strncpy_s(dst, size, src, _TRUNCATE)`. **The Linux definition is not in the
//! checked-in tree**, so this codec does not claim to have read it. Both
//! implementations that plausibly stand behind the macro stop at the first NUL
//! and terminate inside the bound, and `input_main.cpp:800` immediately takes
//! `strlen(buf)` of the result, so a NUL ends the line in either case. This
//! decoder therefore truncates at the first NUL, which is the behaviour both
//! candidates share, and bounds the result at
//! `CHAT_MAX_LEN - (CHARACTER_NAME_MAX_LEN + 3)` = 485 bytes.
//!
//! # The text is raw
//!
//! Legacy never transcodes it: `buf` is copied byte for byte and handed to
//! `ProcessTextTag`, then to `snprintf` as a `%s`. The Rewrite relays the exact
//! bytes and never decodes them as UTF-8 or requires a terminator.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::cg_inventory::HEADER_CG_CHAT;
use crate::cg_wire::{ClientFrame, CLIENT_FRAME_HEADER_SIZE};

/// `sizeof(TPacketCGChat)`: the one-byte header, the length word, and the type.
pub const CG_CHAT_WIRE_SIZE: usize = 4;

/// `CHAT_MAX_LEN` (`server/server/common/length.h:49`).
pub const CHAT_MAX_LEN: usize = 512;

/// `CHARACTER_NAME_MAX_LEN` (`server/server/common/length.h:15`).
pub const CHARACTER_NAME_MAX_LEN: usize = 24;

/// `sizeof(buf)` in `CInputMain::Chat`
/// (`input_main.cpp:798`): `CHAT_MAX_LEN - (CHARACTER_NAME_MAX_LEN + 3) + 1`.
///
/// The handler passes `MIN(iExtraLen + 1, sizeof(buf))` as the `strlcpy` size,
/// so the longest line it can hold is one byte shorter.
pub const CHAT_TEXT_BUFFER_SIZE: usize = CHAT_MAX_LEN - (CHARACTER_NAME_MAX_LEN + 3) + 1;

/// The longest text a line can carry: `sizeof(buf) - 1`.
pub const CHAT_TEXT_MAX: usize = CHAT_TEXT_BUFFER_SIZE - 1;

/// One client-to-game chat line.
///
/// The `type` field is an `EChatType` value. The codec does not range-check it:
/// `CInputMain::Chat` only ever compares it against a constant and otherwise
/// falls into a `sys_err` arm, so a value outside the enumeration is a
/// gameplay outcome, not a malformed frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgChat {
    /// The `size` the client declared, which counts the header.
    pub declared_size: u16,
    /// The `type` field, an `EChatType` value.
    pub chat_type: u8,
    /// The text bytes, already bounded by `size - 4` and by [`CHAT_TEXT_MAX`],
    /// and already stopped at the first NUL.
    pub text: Vec<u8>,
}

/// Why a chat frame could not be read as a `TPacketCGChat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgChatError {
    /// The frame carried fewer than three payload bytes, so the length word
    /// could not be read at all.
    TruncatedPrefix {
        /// Bytes of payload actually present.
        available: usize,
    },
    /// `size` was below `sizeof(TPacketCGChat)`.
    ///
    /// Legacy logs `invalid packet length` and calls `SetPhase(PHASE_CLOSE)`.
    /// The caller must close; dropping the frame is not parity.
    DeclaredSizeTooSmall {
        /// The declared total.
        declared_size: u16,
    },
}

impl fmt::Display for CgChatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedPrefix { available } => {
                write!(
                    formatter,
                    "a chat frame needs 3 payload bytes, got {available}"
                )
            }
            Self::DeclaredSizeTooSmall { declared_size } => {
                write!(
                    formatter,
                    "chat size {declared_size} is under {CG_CHAT_WIRE_SIZE}"
                )
            }
        }
    }
}

impl Error for CgChatError {}

impl CgChat {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_CG_CHAT.value()
    }

    /// Read the record from a decoded frame.
    ///
    /// The frame payload **excludes** the header byte, so `size` is read from
    /// payload bytes 0 and 1 and the text starts at payload byte 3.
    ///
    /// # Errors
    ///
    /// Returns [`CgChatError::TruncatedPrefix`] when the frame is too short to
    /// hold the prefix, and [`CgChatError::DeclaredSizeTooSmall`] when `size` is
    /// under [`CG_CHAT_WIRE_SIZE`]. The latter is the close case, so it is kept
    /// distinct from a frame this codec simply could not read.
    pub fn decode(frame: &ClientFrame) -> Result<Self, CgChatError> {
        if frame.header != Self::header() {
            // The variable-frame resolver already routed by header, so reaching
            // here means the caller passed the wrong frame; treat it as a
            // truncated prefix rather than inventing a third error the legacy
            // handler has no arm for.
            return Err(CgChatError::TruncatedPrefix {
                available: frame.payload.len(),
            });
        }
        if frame.payload.len() < 2 {
            return Err(CgChatError::TruncatedPrefix {
                available: frame.payload.len(),
            });
        }

        // Legacy computes `iExtraLen` and refuses before it ever reads `type`, so
        // a four-byte frame that declares a short size still reaches the close.
        let declared_size = u16::from_le_bytes([frame.payload[0], frame.payload[1]]);
        if usize::from(declared_size) < CG_CHAT_WIRE_SIZE {
            return Err(CgChatError::DeclaredSizeTooSmall { declared_size });
        }

        if frame.payload.len() < 3 {
            return Err(CgChatError::TruncatedPrefix {
                available: frame.payload.len(),
            });
        }

        let chat_type = frame.payload[2];
        let text_len = usize::from(declared_size) - CG_CHAT_WIRE_SIZE;
        // `size` bounds the text; the frame may be longer, and the extra bytes
        // are ignored exactly as `strlcpy` ignores them.
        let available = &frame.payload[3..];
        let capped = text_len.min(available.len()).min(CHAT_TEXT_MAX);
        let raw = &available[..capped];
        let text = match raw.iter().position(|&byte| byte == 0) {
            Some(nul) => raw[..nul].to_vec(),
            None => raw.to_vec(),
        };

        Ok(Self {
            declared_size,
            chat_type,
            text,
        })
    }

    /// Build the frame a legacy client would send for this line.
    #[must_use]
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_CHAT_WIRE_SIZE - CLIENT_FRAME_HEADER_SIZE);
        let total = self.text.len() + CG_CHAT_WIRE_SIZE;
        payload.extend_from_slice(&u16::try_from(total).unwrap_or(u16::MAX).to_le_bytes());
        payload.push(self.chat_type);
        payload.extend_from_slice(&self.text);
        ClientFrame::new(Self::header(), payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(payload: &[u8]) -> ClientFrame {
        ClientFrame::new(CgChat::header(), payload)
    }

    #[test]
    fn the_fixed_part_is_four_packed_bytes() {
        assert_eq!(CG_CHAT_WIRE_SIZE, 4);
    }

    #[test]
    fn the_header_is_three() {
        assert_eq!(CgChat::header(), 3);
    }

    #[test]
    fn decodes_a_talking_line_from_golden_bytes() {
        // header 03 | size 09 00 | type 00 | "hi"
        let bytes = [0x03, 0x09, 0x00, 0x00, b'h', b'i'];
        let decoded = CgChat::decode(&frame(&bytes[1..])).expect("a valid line");
        assert_eq!(decoded.declared_size, 9);
        assert_eq!(decoded.chat_type, 0);
        assert_eq!(decoded.text, b"hi");
    }

    #[test]
    fn the_size_word_is_little_endian() {
        // 0x0102 must read as 258, not 513. A 258-byte total leaves 254 text
        // bytes, so both halves are checked: the word and the text arithmetic.
        let mut payload = vec![0x02, 0x01, 0x00];
        payload.extend(std::iter::repeat(b'a').take(258));
        let decoded = CgChat::decode(&frame(&payload)).expect("a valid line");
        assert_eq!(decoded.declared_size, 0x0102);
        assert_eq!(usize::from(decoded.declared_size), 258);
        assert_eq!(decoded.text.len(), 258 - CG_CHAT_WIRE_SIZE);
    }

    #[test]
    fn the_type_byte_is_opaque_and_all_256_values_survive() {
        for value in 0..=u8::MAX {
            let decoded = CgChat::decode(&frame(&[0x06, 0x00, value, b'x'])).expect("a valid line");
            assert_eq!(decoded.chat_type, value, "type {value} did not survive");
        }
    }

    #[test]
    fn trailing_bytes_past_the_declared_size_are_ignored() {
        // size 6 means 2 text bytes; the frame carries 5.
        let payload = [0x06, 0x00, 0x00, b'a', b'b', b'c', b'd', b'e'];
        let decoded = CgChat::decode(&frame(&payload)).expect("a valid line");
        assert_eq!(decoded.text, b"ab");
    }

    #[test]
    fn a_size_below_four_is_the_close_case() {
        for declared in 0..4_u16 {
            let payload = u16::to_le_bytes(declared).to_vec();
            assert_eq!(
                CgChat::decode(&frame(&payload)),
                Err(CgChatError::DeclaredSizeTooSmall {
                    declared_size: declared
                }),
                "size {declared} must be the close case",
            );
        }
    }

    #[test]
    fn a_payload_too_short_for_the_size_word_is_refused() {
        for available in 0..2_usize {
            let payload = vec![0u8; available];
            assert_eq!(
                CgChat::decode(&frame(&payload)),
                Err(CgChatError::TruncatedPrefix { available }),
                "a {available}-byte payload must be refused",
            );
        }
    }

    #[test]
    fn the_size_is_refused_before_the_type_byte_is_read() {
        // Legacy reaches the close with a four-byte frame, because it computes
        // `iExtraLen` from the length word alone. A size of zero must therefore
        // report the close, not a short prefix, even though `type` is absent.
        assert_eq!(
            CgChat::decode(&frame(&[0x00, 0x00])),
            Err(CgChatError::DeclaredSizeTooSmall { declared_size: 0 }),
        );
        // A long-enough size with the type byte still missing is a truncated
        // prefix, because the close has already been passed.
        assert_eq!(
            CgChat::decode(&frame(&[0x06, 0x00])),
            Err(CgChatError::TruncatedPrefix { available: 2 }),
        );
    }

    #[test]
    fn the_text_stops_at_a_nul() {
        let payload = [0x07, 0x00, 0x00, b'a', 0x00, b'b'];
        let decoded = CgChat::decode(&frame(&payload)).expect("a valid line");
        assert_eq!(decoded.text, b"a");
    }

    #[test]
    fn the_text_is_capped_at_the_handler_buffer_bound() {
        assert_eq!(CHAT_TEXT_BUFFER_SIZE, 486);
        assert_eq!(CHAT_TEXT_MAX, 485);
        let long = 600_u16;
        let mut payload = (long + 4).to_le_bytes().to_vec();
        payload.push(0x00);
        payload.extend(std::iter::repeat(b'z').take(long as usize));
        let decoded = CgChat::decode(&frame(&payload)).expect("a valid line");
        assert_eq!(decoded.text.len(), CHAT_TEXT_MAX);
    }

    #[test]
    fn a_text_exactly_at_the_bound_is_kept_whole() {
        let size = u16::try_from(CHAT_TEXT_MAX + 4).expect("the text bound fits the size word");
        let mut payload = size.to_le_bytes().to_vec();
        payload.push(0x00);
        payload.extend(std::iter::repeat(b'y').take(CHAT_TEXT_MAX));
        let decoded = CgChat::decode(&frame(&payload)).expect("a valid line");
        assert_eq!(decoded.text.len(), CHAT_TEXT_MAX);
    }

    #[test]
    fn to_frame_then_decode_is_stable() {
        let original = CgChat {
            declared_size: 0,
            chat_type: 7,
            text: b"hello world".to_vec(),
        };
        let encoded = original.to_frame();
        assert_eq!(encoded.header, CgChat::header());
        let decoded = CgChat::decode(&encoded).expect("a valid line");
        assert_eq!(decoded.declared_size, 4 + 11);
        assert_eq!(decoded.chat_type, 7);
        assert_eq!(decoded.text, b"hello world");
    }

    #[test]
    fn the_error_messages_name_the_failure() {
        let prefix = CgChatError::TruncatedPrefix { available: 1 };
        assert!(prefix.to_string().contains("3 payload bytes"));
        let small = CgChatError::DeclaredSizeTooSmall { declared_size: 2 };
        assert!(small.to_string().contains("chat size 2"));
    }
}
