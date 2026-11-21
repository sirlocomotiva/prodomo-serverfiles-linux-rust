//! Transport-free codec for the CG direction of `TPacketCGAura`.
//!
//! | record | header | wire | payload | declaration | registration |
//! |---|---|---|---|---|---|
//! | `SPacketCGAura` | 172 `0xac` | 4 | 3 | `packet.h:3056-3063` | `packet_info.cpp:184` |
//!
//! ```c
//! typedef struct SPacketCGAura
//! {
//!     SPacketCGAura() : bHeader(HEADER_CG_AURA) {}
//!     BYTE bHeader;
//!     WORD wSize;
//!     BYTE bSubHeader;
//! } TPacketCGAura;
//! ```
//!
//! # Width
//!
//! `1 + 2 + 1 = 4`, which closes against
//! `Set(HEADER_CG_AURA, sizeof(TPacketCGAura), "Aura")` at `packet_info.cpp:184`.
//! The declaration is under `__AURA_SYSTEM__` in both trees, which is `#define`d
//! at `server/server/common/prodomodefines.h:14`, so the record is in the active
//! build of both. Dispatch is live at `input_main.cpp:3865-3868`.
//!
//! # The `wSize` word is dead
//!
//! The handler reads only the sub-header (`input_main.cpp:5304`) and sizes the
//! sub-record from it (`GetAuraSubPacketLength`, `:5277-5292`). Nothing reads
//! `wSize`, so the codec preserves its bytes and assigns them no meaning.
//!
//! # The sub-header is an enum with no `default:`
//!
//! `AURA_SUBHEADER_CG_REFINE_CHECKIN` (0), `REFINE_CHECKOUT` (1),
//! `REFINE_ACCEPT` (2), `REFINE_CANCEL` (3) at `packet.h:3026-3032`. The
//! handler's `switch` has no `default:` arm (`input_main.cpp:5312-5337`), so an
//! unknown value falls through and returns 0, consuming the prefix alone. The
//! field stays an opaque `u8`.

use crate::cg_inventory::{CgHeader, HEADER_CG_AURA};
use crate::cg_wire::ClientFrame;
use crate::item_pos::ItemPos;

/// The full legacy `TPacketCGAura` record, header byte included.
pub const CG_AURA_WIRE_SIZE: usize = 1 + 2 + 1;
/// The framed payload of `TPacketCGAura`.
pub const CG_AURA_PAYLOAD_SIZE: usize = CG_AURA_WIRE_SIZE - 1;

/// `AURA_SUBHEADER_CG_REFINE_CHECKIN`.
pub const AURA_SUBHEADER_CG_REFINE_CHECKIN: u8 = 0;
/// `AURA_SUBHEADER_CG_REFINE_CHECKOUT`.
pub const AURA_SUBHEADER_CG_REFINE_CHECKOUT: u8 = 1;
/// `AURA_SUBHEADER_CG_REFINE_ACCEPT`.
pub const AURA_SUBHEADER_CG_REFINE_ACCEPT: u8 = 2;
/// `AURA_SUBHEADER_CG_REFINE_CANCEL`.
pub const AURA_SUBHEADER_CG_REFINE_CANCEL: u8 = 3;

/// `sizeof(TSubPacketCGAuraRefineCheckIn)`: two `TItemPos` and one window byte.
pub const AURA_CHECKIN_EXTRA_WIRE_SIZE: usize = ItemPos::WIRE_SIZE + ItemPos::WIRE_SIZE + 1;
/// `sizeof(TSubPacketCGAuraRefineCheckOut)`: one `TItemPos` and one window byte.
pub const AURA_CHECKOUT_EXTRA_WIRE_SIZE: usize = ItemPos::WIRE_SIZE + 1;
/// `sizeof(TSubPacketCGAuraRefineAccept)`: one window byte.
pub const AURA_ACCEPT_EXTRA_WIRE_SIZE: usize = 1;

/// Every way the aura decoder can refuse a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgAuraError {
    /// Fewer bytes than the record needs.
    Truncated {
        /// The width the decoder requires.
        needed: usize,
        /// How many bytes were actually offered.
        available: usize,
    },
    /// A complete slice that is not the width the sub-header needs.
    LengthMismatch {
        /// The width the decoder requires.
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

impl core::fmt::Display for CgAuraError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(f, "truncated aura: need {needed} bytes, got {available}")
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "aura length mismatch: expected {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { expected, actual } => {
                write!(f, "invalid aura header: expected {expected}, got {actual}")
            }
        }
    }
}

impl std::error::Error for CgAuraError {}

/// The `TPacketCGAura` prefix: the dead size word and the sub-header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgAura {
    /// The legacy `WORD wSize`: preserved, never read.
    pub w_size: u16,
    /// The legacy `BYTE bSubHeader`, an opaque `u8`.
    pub subheader: u8,
}

impl CgAura {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_AURA
    }
    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_AURA_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_AURA_PAYLOAD_SIZE;

    /// Build the prefix.
    pub const fn new(w_size: u16, subheader: u8) -> Self {
        Self { w_size, subheader }
    }

    /// Append the prefix to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.extend_from_slice(&self.w_size.to_le_bytes());
        out.push(self.subheader);
    }

    /// Encode to a fresh 4-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// # Errors
    ///
    /// [`CgAuraError::Truncated`] below 4 bytes,
    /// [`CgAuraError::LengthMismatch`] above,
    /// [`CgAuraError::InvalidHeader`] for a full-length slice not starting
    /// with 172.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgAuraError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(CgAuraError::Truncated {
                needed: Self::WIRE_SIZE,
                available: bytes.len(),
            });
        }
        if bytes.len() > Self::WIRE_SIZE {
            return Err(CgAuraError::LengthMismatch {
                expected: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header().value() {
            return Err(CgAuraError::InvalidHeader {
                expected: Self::header().value(),
                actual: bytes[0],
            });
        }
        Ok(Self {
            w_size: u16::from_le_bytes([bytes[1], bytes[2]]),
            subheader: bytes[3],
        })
    }

    /// # Errors
    ///
    /// As [`CgAura::decode`], except that the payload must be exactly 3
    /// bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgAuraError> {
        if frame.payload.len() < Self::PAYLOAD_SIZE {
            return Err(CgAuraError::Truncated {
                needed: Self::PAYLOAD_SIZE,
                available: frame.payload.len(),
            });
        }
        if frame.payload.len() > Self::PAYLOAD_SIZE {
            return Err(CgAuraError::LengthMismatch {
                expected: Self::PAYLOAD_SIZE,
                actual: frame.payload.len(),
            });
        }
        Ok(Self {
            w_size: u16::from_le_bytes([frame.payload[0], frame.payload[1]]),
            subheader: frame.payload[2],
        })
    }
}

/// `TSubPacketCGAuraRefineCheckIn` (`packet.h:3034-3039`): the checked-in item
/// position, the aura slot, and the window type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgAuraCheckIn {
    /// The legacy `TItemPos ItemCell`.
    pub item_cell: ItemPos,
    /// The legacy `TItemPos AuraCell`.
    pub aura_cell: ItemPos,
    /// The legacy `BYTE byAuraRefineWindowType`.
    pub window: u8,
}

impl CgAuraCheckIn {
    /// The sub-record width: two positions and one window byte.
    pub const WIRE_SIZE: usize = AURA_CHECKIN_EXTRA_WIRE_SIZE;

    /// # Errors
    ///
    /// [`CgAuraError::Truncated`] below 7 bytes,
    /// [`CgAuraError::LengthMismatch`] above.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgAuraError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(CgAuraError::Truncated {
                needed: Self::WIRE_SIZE,
                available: bytes.len(),
            });
        }
        if bytes.len() > Self::WIRE_SIZE {
            return Err(CgAuraError::LengthMismatch {
                expected: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            item_cell: ItemPos::decode_at(bytes, 0),
            aura_cell: ItemPos::decode_at(bytes, 3),
            window: bytes[6],
        })
    }
}

/// `TSubPacketCGAuraRefineCheckOut` (`packet.h:3041-3045`): the aura slot and
/// the window type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgAuraCheckOut {
    /// The legacy `TItemPos AuraCell`.
    pub aura_cell: ItemPos,
    /// The legacy `BYTE byAuraRefineWindowType`.
    pub window: u8,
}

impl CgAuraCheckOut {
    /// The sub-record width: one position and one window byte.
    pub const WIRE_SIZE: usize = AURA_CHECKOUT_EXTRA_WIRE_SIZE;

    /// # Errors
    ///
    /// [`CgAuraError::Truncated`] below 4 bytes,
    /// [`CgAuraError::LengthMismatch`] above.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgAuraError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(CgAuraError::Truncated {
                needed: Self::WIRE_SIZE,
                available: bytes.len(),
            });
        }
        if bytes.len() > Self::WIRE_SIZE {
            return Err(CgAuraError::LengthMismatch {
                expected: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            aura_cell: ItemPos::decode_at(bytes, 0),
            window: bytes[3],
        })
    }
}

/// `TSubPacketCGAuraRefineAccept` (`packet.h:3047-3050`): the window type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgAuraAccept {
    /// The legacy `BYTE byAuraRefineWindowType`.
    pub window: u8,
}

impl CgAuraAccept {
    /// The sub-record width: one window byte.
    pub const WIRE_SIZE: usize = AURA_ACCEPT_EXTRA_WIRE_SIZE;

    /// # Errors
    ///
    /// [`CgAuraError::Truncated`] below 1 byte,
    /// [`CgAuraError::LengthMismatch`] above.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgAuraError> {
        if bytes.is_empty() {
            return Err(CgAuraError::Truncated {
                needed: Self::WIRE_SIZE,
                available: 0,
            });
        }
        if bytes.len() > Self::WIRE_SIZE {
            return Err(CgAuraError::LengthMismatch {
                expected: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        Ok(Self { window: bytes[0] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden bytes from the source field order, with distinct byte halves.
    #[test]
    fn the_prefix_counts_four_and_keeps_the_dead_word() {
        let record = CgAura::new(0x1234, AURA_SUBHEADER_CG_REFINE_CHECKIN);
        assert_eq!(record.encode(), vec![172, 0x34, 0x12, 0]);
        assert_eq!(CgAura::decode(&[172, 0x34, 0x12, 0]), Ok(record));
        let frame = ClientFrame {
            header: 172,
            payload: vec![0x34, 0x12, 3],
        };
        assert_eq!(
            CgAura::decode_frame(&frame),
            Ok(CgAura::new(0x1234, AURA_SUBHEADER_CG_REFINE_CANCEL))
        );
    }

    /// The check-in reads the item cell, then the aura slot, then the window.
    #[test]
    fn the_checkin_reads_two_positions_and_a_window() {
        let bytes = [1, 0x22, 0x11, 7, 1, 0, 0];
        let decoded = CgAuraCheckIn::decode(&bytes).expect("seven bytes");
        assert_eq!(decoded.item_cell, ItemPos::new(1, 0x1122));
        assert_eq!(decoded.aura_cell, ItemPos::new(7, 1));
        assert_eq!(decoded.window, 0);
        assert!(matches!(
            CgAuraCheckIn::decode(&bytes[..6]),
            Err(CgAuraError::Truncated { .. })
        ));
    }

    /// The check-out and the accept read their trailing window byte.
    #[test]
    fn the_checkout_and_the_accept_read_their_window() {
        let out = CgAuraCheckOut::decode(&[7, 0, 0, 2]).expect("four bytes");
        assert_eq!(out.aura_cell, ItemPos::new(7, 0));
        assert_eq!(out.window, 2);
        let accept = CgAuraAccept::decode(&[1]).expect("one byte");
        assert_eq!(accept.window, 1);
        assert!(matches!(
            CgAuraAccept::decode(&[]),
            Err(CgAuraError::Truncated { .. })
        ));
    }
}
