//! The `HEADER_CG_MESSENGER` (67) records: the friend adds, the remove and the dead
//! invite-answer subheader.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:1944-1977`:
//!
//! ```ignore
//! enum
//! {
//!     MESSENGER_SUBHEADER_CG_ADD_BY_VID,   // 0
//!     MESSENGER_SUBHEADER_CG_ADD_BY_NAME,  // 1
//!     MESSENGER_SUBHEADER_CG_REMOVE,       // 2
//!     MESSENGER_SUBHEADER_CG_INVITE_ANSWER, // 3
//! };
//!
//! typedef struct command_messenger
//! {
//!     BYTE header;
//!     BYTE subheader;
//! } TPacketCGMessenger;
//! ```
//!
//! The base is **2 packed bytes** (`protocol/src/cg_variable.rs`'s
//! `MESSENGER_BASE_WIRE_SIZE`, measured the same way). The framing behind the extensions
//! lives there: `ADD_BY_VID` adds the **packed 4 bytes** of `TPacketCGMessengerAddByVID`'s
//! `DWORD vid`, `ADD_BY_NAME` and `REMOVE` add a **raw 24-byte name field**
//! (`CHARACTER_NAME_MAX_LEN`, `common/length.h:15`), and `INVITE_ANSWER` or an unknown
//! subheader consumes only the base — the analyzer has no arm for them.
//!
//! # The handler's own refuses
//!
//! `CInputMain::Messenger` (`G/input_main.cpp:1136-1233`) checks
//! `uiBytes < sizeof(TPacketCGMessenger)` first, then every arm checks its own suffix:
//! `if (uiBytes < sizeof(TPacketCGMessengerAddByVID)) return -1;` (:1141) and
//! `if (uiBytes < CHARACTER_NAME_MAX_LEN) return -1;` (:1181, `:1198`):
//! the **24** bytes, not the 31 the REMOVE record's own struct carries
//! (`command_messenger_remove`, `packet.h:1966-1971`), which legacy never reads.
//! `protocol/src/cg_variable.rs` already enforces this frame width before the record is
//! decoded, so this codec treats a shorter frame as its own error and the transport closes
//! the descriptor on it, as it does for every malformed frame (the ledger 160.5 rule).
//!
//! # The name is a C string
//!
//! Both name arms `strlcpy` the field into `CHARACTER_NAME_MAX_LEN + 1` bytes
//! (`G/input_main.cpp:1184`, `:1241`), so the name is the bytes up to the first NUL. The
//! codec keeps the whole 24-byte field and answers the C string's reading.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::cg_inventory::HEADER_CG_MESSENGER;
use crate::cg_wire::ClientFrame;

/// `sizeof(TPacketCGMessenger)`: the one-byte header and the subheader byte.
pub const CG_MESSENGER_WIRE_SIZE: usize = 2;

/// The base's bytes inside the frame payload, which excludes the header byte: the
/// subheader alone.
pub const CG_MESSENGER_PAYLOAD_SIZE: usize = CG_MESSENGER_WIRE_SIZE - 1;

/// `CHARACTER_NAME_MAX_LEN` (`server/server/common/length.h:15`): the 24 raw name bytes
/// both name arm's add on the base.
pub const CG_MESSENGER_NAME_EXTRA_WIRE_SIZE: usize = 24;

/// `sizeof(TPacketCGMessengerAddByVID)` (`packet.h:1954-1957`): the packed 4 vid bytes.
pub const CG_MESSENGER_ADD_BY_VID_EXTRA_WIRE_SIZE: usize = 4;

/// `MESSENGER_SUBHEADER_CG_ADD_BY_VID`: 0.
pub const MESSENGER_SUBHEADER_CG_ADD_BY_VID: u8 = 0;
/// `MESSENGER_SUBHEADER_CG_ADD_BY_NAME`: 1.
pub const MESSENGER_SUBHEADER_CG_ADD_BY_NAME: u8 = 1;
/// `MESSENGER_SUBHEADER_CG_REMOVE`: 2.
pub const MESSENGER_SUBHEADER_CG_REMOVE: u8 = 2;
/// `MESSENGER_SUBHEADER_CG_INVITE_ANSWER`: 3, which no handler has an arm for.
pub const MESSENGER_SUBHEADER_CG_INVITE_ANSWER: u8 = 3;

/// One client-to-game messenger record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgMessenger {
    /// `MESSENGER_SUBHEADER_CG_ADD_BY_VID`: add the character whose VID this is.
    AddByVid {
        /// The companion's VID, as the client sent it.
        vid: u32,
    },
    /// `MESSENGER_SUBHEADER_CG_ADD_BY_NAME`: add the character this name names.
    AddByName {
        /// The 24 raw name bytes, the companion's name.
        name: [u8; CG_MESSENGER_NAME_EXTRA_WIRE_SIZE],
    },
    /// `MESSENGER_SUBHEADER_CG_REMOVE`: remove the character this name names.
    Remove {
        /// The 24 raw name bytes, the companion's name.
        name: [u8; CG_MESSENGER_NAME_EXTRA_WIRE_SIZE],
    },
    /// `MESSENGER_SUBHEADER_CG_INVITE_ANSWER` and every unknown subheader: the analyzer
    /// logs and consumes the base alone (`G/input_main.cpp:1225-1228`).
    Consumed {
        /// The subheader the client sent.
        subheader: u8,
    },
}

/// Why a messenger frame could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgMessengerError {
    /// The frame is shorter than the base, or shorter than the record's own extension.
    Truncated {
        /// Bytes of payload actually present.
        available: usize,
    },
}

impl fmt::Display for CgMessengerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::Truncated { available } = self;
        write!(
            formatter,
            "a messenger frame needs the base {CG_MESSENGER_WIRE_SIZE} bytes with the extension its subheader asks, got {available}"
        )
    }
}

impl Error for CgMessengerError {}

impl CgMessenger {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_CG_MESSENGER.value()
    }

    /// Read the record from a decoded frame.
    ///
    /// The frame's payload **excludes** the header byte, so it starts at the subheader
    /// byte. The extensions this codec expects are the same ones
    /// `cg_variable`'s messenger size rule framed it by.
    ///
    /// # Errors
    ///
    /// Returns [`CgMessengerError::Truncated`] when the frame is too short for the
    /// subheader's own extension.
    ///
    /// # Panics
    ///
    /// Never: the extension's length is checked before any slice of it is taken.
    pub fn decode(frame: &ClientFrame) -> Result<Self, CgMessengerError> {
        let payload = &frame.payload;
        if payload.len() < CG_MESSENGER_PAYLOAD_SIZE {
            // `if (uiBytes < sizeof(TPacketCGMessenger)) return -1;`
            // (`G/input_main.cpp:1139-1140`).
            return Err(CgMessengerError::Truncated {
                available: payload.len(),
            });
        }
        let subheader = payload[0];
        let suffix = &payload[CG_MESSENGER_PAYLOAD_SIZE..];
        match subheader {
            MESSENGER_SUBHEADER_CG_ADD_BY_VID => {
                if payload.len()
                    < CG_MESSENGER_PAYLOAD_SIZE + CG_MESSENGER_ADD_BY_VID_EXTRA_WIRE_SIZE
                {
                    // `if (uiBytes < sizeof(TPacketCGMessengerAddByVID)) return -1;`
                    return Err(CgMessengerError::Truncated {
                        available: payload.len(),
                    });
                }
                Ok(Self::AddByVid {
                    vid: u32::from_le_bytes(
                        suffix[..CG_MESSENGER_ADD_BY_VID_EXTRA_WIRE_SIZE]
                            .try_into()
                            .expect("four bytes"),
                    ),
                })
            }
            MESSENGER_SUBHEADER_CG_ADD_BY_NAME | MESSENGER_SUBHEADER_CG_REMOVE => {
                if payload.len() < CG_MESSENGER_PAYLOAD_SIZE + CG_MESSENGER_NAME_EXTRA_WIRE_SIZE {
                    // `if (uiBytes < CHARACTER_NAME_MAX_LEN) return -1;`
                    return Err(CgMessengerError::Truncated {
                        available: payload.len(),
                    });
                }
                let mut name = [0_u8; CG_MESSENGER_NAME_EXTRA_WIRE_SIZE];
                name.copy_from_slice(&suffix[..CG_MESSENGER_NAME_EXTRA_WIRE_SIZE]);
                if subheader == MESSENGER_SUBHEADER_CG_ADD_BY_NAME {
                    Ok(Self::AddByName { name })
                } else {
                    Ok(Self::Remove { name })
                }
            }
            // The default arm: `sys_err(...); break;` — the base alone is consumed
            // (`G/input_main.cpp:1225-1228`).
            other => Ok(Self::Consumed { subheader: other }),
        }
    }

    /// The name, up to the first NUL, of the record's own 24-byte field.
    ///
    /// # Panics
    ///
    /// Only on a record the caller decoded to a nameless kind.
    #[must_use]
    pub fn companion_name(&self) -> &[u8] {
        let latin = match self {
            Self::AddByName { name } | Self::Remove { name } => name,
            other => unreachable!("no companion name: {other:?}"),
        };
        let end = latin
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(latin.len());
        &latin[..end]
    }

    /// Build the frame a legacy client would send for this record.
    ///
    /// The frame payload excludes the header byte, so it starts at the subheader byte.
    #[must_use]
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::new();
        match self {
            Self::AddByVid { vid } => {
                payload.push(MESSENGER_SUBHEADER_CG_ADD_BY_VID);
                payload.extend_from_slice(&vid.to_le_bytes());
            }
            Self::AddByName { name } => {
                payload.push(MESSENGER_SUBHEADER_CG_ADD_BY_NAME);
                payload.extend_from_slice(name);
            }
            Self::Remove { name } => {
                payload.push(MESSENGER_SUBHEADER_CG_REMOVE);
                payload.extend_from_slice(name);
            }
            Self::Consumed { subheader } => {
                payload.push(*subheader);
            }
        }
        ClientFrame::new(Self::header(), payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_of(subheader: u8, suffix: &[u8]) -> ClientFrame {
        let mut payload = Vec::with_capacity(1 + suffix.len());
        payload.push(subheader);
        payload.extend_from_slice(suffix);
        ClientFrame::new(HEADER_CG_MESSENGER.value(), payload)
    }

    #[test]
    fn the_base_is_two_packed_bytes() {
        // Measured, not summed: `sizeof(TPacketCGMessenger)` under `#pragma pack(1)` on the
        // 32-bit target.
        let frame = frame_of(
            MESSENGER_SUBHEADER_CG_ADD_BY_VID,
            &4_u32.to_le_bytes() as &[u8],
        );
        assert!(matches!(
            CgMessenger::decode(&frame),
            Ok(CgMessenger::AddByVid { vid: 4 })
        ));
    }

    #[test]
    fn the_add_by_vid_record_reads_the_vid() {
        let frame = frame_of(MESSENGER_SUBHEADER_CG_ADD_BY_VID, &[0x01, 0x02, 0x03, 0x04]);
        let record = CgMessenger::decode(&frame).expect("the record decodes");
        assert_eq!(record, CgMessenger::AddByVid { vid: 0x0403_0201 });
    }

    #[test]
    fn the_name_arms_read_the_24_raw_bytes() {
        let mut name = [0_u8; CG_MESSENGER_NAME_EXTRA_WIRE_SIZE];
        name[..3].copy_from_slice(b"Bob");
        let record = CgMessenger::decode(&frame_of(MESSENGER_SUBHEADER_CG_ADD_BY_NAME, &name))
            .expect("the record decodes");
        assert_eq!(record.companion_name(), b"Bob");
        let record = CgMessenger::decode(&frame_of(MESSENGER_SUBHEADER_CG_REMOVE, &name))
            .expect("the record decodes");
        assert_eq!(record.companion_name(), b"Bob");
    }

    #[test]
    fn a_name_field_with_no_terminator_reads_whole() {
        let latin = [0x61; CG_MESSENGER_NAME_EXTRA_WIRE_SIZE];
        let record = CgMessenger::decode(&frame_of(MESSENGER_SUBHEADER_CG_ADD_BY_NAME, &latin))
            .expect("the record decodes");
        assert_eq!(record.companion_name(), latin);
    }

    #[test]
    fn the_invite_answer_and_an_unknown_subheader_consume_the_base() {
        for subheader in [MESSENGER_SUBHEADER_CG_INVITE_ANSWER, 99] {
            let frame = frame_of(subheader, b"");
            let record = CgMessenger::decode(&frame).expect("the base decodes");
            assert_eq!(record, CgMessenger::Consumed { subheader });
        }
    }

    #[test]
    fn a_short_frame_is_its_own_error() {
        assert!(matches!(
            CgMessenger::decode(&ClientFrame::new(HEADER_CG_MESSENGER.value(), Vec::new())),
            Err(CgMessengerError::Truncated { available: 0 })
        ));
        assert!(matches!(
            CgMessenger::decode(&frame_of(MESSENGER_SUBHEADER_CG_ADD_BY_VID, &[1, 2, 3])),
            Err(CgMessengerError::Truncated { available: 4 })
        ));
        assert!(matches!(
            CgMessenger::decode(&frame_of(MESSENGER_SUBHEADER_CG_REMOVE, &[0x61; 23])),
            Err(CgMessengerError::Truncated { available: 24 })
        ));
    }
}
