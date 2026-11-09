//! Transport-free codec for the CG direction of `TPacketCGSwitchbot`.
//!
//! | record | header | wire | payload | declaration | registration |
//! |---|---|---|---|---|---|
//! | `SPacketCGSwitchbot` | 171 `0xab` | 7 | 6 | `packet.h:3400-3406` | `packet_info.cpp:172` |
//!
//! ```c
//! struct TPacketCGSwitchbot
//! {
//!     BYTE header;
//!     int size;
//!     BYTE subheader;
//!     BYTE slot;
//! };
//! ```
//!
//! # Width
//!
//! `1 + 4 + 1 + 1 = 7`, which closes against
//! `Set(HEADER_CG_SWITCHBOT, sizeof(TPacketGCSwitchbot), "Switchbot")` at
//! `packet_info.cpp:172` (the registration names the GC struct, whose head has the
//! same width). The declaration is under `ENABLE_SWITCHBOT`, which is `#define`d at
//! `server/server/common/prodomodefines.h:191`. Dispatch is live at
//! `input_main.cpp:3923-3927`.
//!
//! The START arm then reads `SWITCHBOT_ALTERNATIVE_COUNT` (2)
//! `TSwitchbotAttributeAlternativeTable` bodies (`input_main.cpp:5425-5444`): five
//! `TPlayerItemAttribute` each, 15 bytes, 30 in all. The STOP arm reads nothing
//! (`:5447-5451`). The handler's `switch` has no `default:` arm, so an unknown
//! subheader falls through and returns 0, consuming the prefix alone. The field
//! stays an opaque `u8`.
//!
//! # The `size` int is dead
//!
//! The handler reads only the subheader and the slot and sizes the bodies from the
//! subheader. Nothing reads `size`, so the codec preserves its bytes and assigns
//! them no meaning.

use crate::cg_inventory::{CgHeader, HEADER_CG_SWITCHBOT};
use crate::cg_wire::ClientFrame;
use crate::gc_item_window::ItemAttribute;

/// The full legacy `TPacketCGSwitchbot` record, header byte included.
pub const CG_SWITCHBOT_WIRE_SIZE: usize = 1 + 4 + 1 + 1;
/// The framed payload of `TPacketCGSwitchbot`.
pub const CG_SWITCHBOT_PAYLOAD_SIZE: usize = CG_SWITCHBOT_WIRE_SIZE - 1;

/// `SUBHEADER_CG_SWITCHBOT_START`.
pub const SUBHEADER_CG_SWITCHBOT_START: u8 = 0;
/// `SUBHEADER_CG_SWITCHBOT_STOP`.
pub const SUBHEADER_CG_SWITCHBOT_STOP: u8 = 1;

/// One alternative's width: five `TPlayerItemAttribute`, 15 bytes.
pub const SWITCHBOT_ALTERNATIVE_WIRE_SIZE: usize = 15;
/// The START bodies' width: two alternatives, 30 bytes.
pub const SWITCHBOT_START_EXTRA_WIRE_SIZE: usize = 2 * SWITCHBOT_ALTERNATIVE_WIRE_SIZE;

/// The prefix: the dead size, the subheader, the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgSwitchbot {
    /// Legacy `size`, preserved, meaningless.
    pub size: i32,
    /// Legacy `subheader`: START, STOP, or an opaque byte the handler drops.
    pub subheader: u8,
    /// Legacy `slot`.
    pub slot: u8,
}

impl CgSwitchbot {
    /// The legacy header byte.
    #[must_use]
    pub const fn header() -> CgHeader {
        HEADER_CG_SWITCHBOT
    }

    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_SWITCHBOT_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_SWITCHBOT_PAYLOAD_SIZE;

    /// Decode the prefix from a frame whose payload holds at least it. The START
    /// bodies are decoded separately from the bytes past the prefix.
    ///
    /// # Errors
    ///
    /// [`CgSwitchbotError::Truncated`] when the payload is short,
    /// [`CgSwitchbotError::Header`] on a wrong header byte.
    pub fn decode_prefix(frame: &ClientFrame) -> Result<Self, CgSwitchbotError> {
        if frame.payload.len() < Self::PAYLOAD_SIZE {
            return Err(CgSwitchbotError::Truncated {
                needed: Self::PAYLOAD_SIZE,
                available: frame.payload.len(),
            });
        }
        if frame.header != Self::header().value() {
            return Err(CgSwitchbotError::Header {
                expected: Self::header().value(),
                actual: frame.header,
            });
        }
        Ok(Self {
            size: i32::from_le_bytes([
                frame.payload[0],
                frame.payload[1],
                frame.payload[2],
                frame.payload[3],
            ]),
            subheader: frame.payload[4],
            slot: frame.payload[5],
        })
    }

    /// Encode the prefix to a fresh buffer.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        out.push(Self::header().value());
        out.extend_from_slice(&self.size.to_le_bytes());
        out.push(self.subheader);
        out.push(self.slot);
        out
    }
}

/// One START alternative: the five wanted attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgSwitchbotAlternative {
    /// Legacy `attributes`.
    pub attributes: [ItemAttribute; 5],
}

impl CgSwitchbotAlternative {
    /// Decode one alternative from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// [`CgSwitchbotError::Truncated`] when fewer than 15 bytes remain.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgSwitchbotError> {
        if bytes.len() < SWITCHBOT_ALTERNATIVE_WIRE_SIZE {
            return Err(CgSwitchbotError::Truncated {
                needed: SWITCHBOT_ALTERNATIVE_WIRE_SIZE,
                available: bytes.len(),
            });
        }
        let mut attributes = [ItemAttribute::new(0, 0); 5];
        for (index, attribute) in attributes.iter_mut().enumerate() {
            let at = index * 3;
            *attribute = ItemAttribute::new(
                bytes[at],
                i16::from_le_bytes([bytes[at + 1], bytes[at + 2]]),
            );
        }
        Ok(Self { attributes })
    }
}

/// Every way the switchbot decoder can refuse a byte slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgSwitchbotError {
    /// Fewer bytes than the prefix or the alternative needs.
    Truncated {
        /// The bytes needed.
        needed: usize,
        /// The bytes present.
        available: usize,
    },
    /// A header byte that is not `HEADER_CG_SWITCHBOT`.
    Header {
        /// The expected byte.
        expected: u8,
        /// The byte met.
        actual: u8,
    },
}

impl std::fmt::Display for CgSwitchbotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "switchbot record truncated: need {needed}, have {available}"
                )
            }
            Self::Header { expected, actual } => {
                write!(f, "switchbot header {actual:#x}, want {expected:#x}")
            }
        }
    }
}

impl std::error::Error for CgSwitchbotError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden bytes in legacy field order: the header, the size's distinct halves,
    /// the subheader, the slot.
    #[test]
    fn the_prefix_round_trips_in_legacy_field_order() {
        let prefix = CgSwitchbot {
            size: 0x0102_0304,
            subheader: SUBHEADER_CG_SWITCHBOT_START,
            slot: 3,
        };
        let wire = prefix.encode();
        assert_eq!(wire, vec![0xab, 0x04, 0x03, 0x02, 0x01, 0, 3]);
        let frame = ClientFrame {
            header: 0xab,
            payload: wire[1..].to_vec(),
        };
        assert_eq!(CgSwitchbot::decode_prefix(&frame), Ok(prefix));
    }

    /// A short payload and a wrong header refuse with their own errors.
    #[test]
    fn the_prefix_refuses_a_short_payload_and_a_wrong_header() {
        let short = ClientFrame {
            header: 0xab,
            payload: vec![0; CG_SWITCHBOT_PAYLOAD_SIZE - 1],
        };
        assert_eq!(
            CgSwitchbot::decode_prefix(&short),
            Err(CgSwitchbotError::Truncated {
                needed: CG_SWITCHBOT_PAYLOAD_SIZE,
                available: CG_SWITCHBOT_PAYLOAD_SIZE - 1,
            })
        );
        let wrong = ClientFrame {
            header: 0xaa,
            payload: vec![0; CG_SWITCHBOT_PAYLOAD_SIZE],
        };
        assert_eq!(
            CgSwitchbot::decode_prefix(&wrong),
            Err(CgSwitchbotError::Header {
                expected: 0xab,
                actual: 0xaa,
            })
        );
    }

    /// Golden bytes: the five attributes' distinct halves, then the truncation.
    #[test]
    fn the_alternative_decodes_five_attributes() {
        let mut wire = Vec::new();
        for (b_type, s_value) in [
            (0x11, 0x2233),
            (0, 0),
            (0x44, 0x5566),
            (0, 0),
            (0x77, 0x1122),
        ] {
            wire.push(b_type);
            wire.extend_from_slice(&i16::to_le_bytes(s_value));
        }
        let alternative = CgSwitchbotAlternative::decode(&wire).expect("fifteen bytes");
        assert_eq!(alternative.attributes[0], ItemAttribute::new(0x11, 0x2233));
        assert_eq!(alternative.attributes[2], ItemAttribute::new(0x44, 0x5566));
        assert_eq!(alternative.attributes[4], ItemAttribute::new(0x77, 0x1122));
        assert_eq!(
            CgSwitchbotAlternative::decode(&wire[..14]),
            Err(CgSwitchbotError::Truncated {
                needed: SWITCHBOT_ALTERNATIVE_WIRE_SIZE,
                available: 14,
            })
        );
    }
}
