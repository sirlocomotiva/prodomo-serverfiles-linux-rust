//! Transport-free codecs for the five remaining legacy CG party records.
//!
//! `protocol::cg_party_skill` already covers `TPacketCGPartyUseSkill`. This
//! module completes the family with the other five records, all declared
//! together at `server/server/game/packet.h:2004-2083` and all registered
//! together at `server/server/game/packet_info.cpp:146-151`.
//!
//! # Wire layouts
//!
//! Every record is inside the `#pragma pack(1)` that `packet.h:274` opens and
//! does not close until `:3540`. The last conditional before the block is the
//! `#endif` at `packet.h:1997`, and the first declaration starts at `:2004`, so
//! all five sit outside every `#ifdef` and every width is fixed.
//!
//! ```text
//! 72  TPacketCGPartyInvite        [0x48][vid:        u32 LE]              5 bytes
//! 73  TPacketCGPartyInviteAnswer  [0x49][leader_vid: u32 LE][accept: u8]  6 bytes
//! 74  TPacketCGPartyRemove        [0x4a][pid:        u32 LE]              5 bytes
//! 75  TPacketCGPartySetState      [0x4b][pid: u32 LE][by_role: u8][flag: u8]  7 bytes
//! 78  TPacketCGPartyParameter     [0x4e][distribute_mode: u8]             2 bytes
//! ```
//!
//! The header values are `packet.h:56`, `:57`, `:58`, `:59`, and `:62`. The
//! derived widths match the `base_size` already recorded for these five rows in
//! `protocol/src/cg_inventory.rs` (5, 6, 5, 7, and 2), so no inventory change
//! is needed.
//!
//! # Field-name drift between the two trees
//!
//! The struct tags are identical in both trees, and the widths are identical,
//! but several member names differ. These are recorded, not normalised, because
//! a Rust field name should follow the server, which is the behavioural oracle.
//!
//! - `TPacketCGPartyInviteAnswer`: the server calls the second member
//!   `leader_vid` at `packet.h:2035`; the client calls it `leader_pid` at
//!   `client/Client/UserInterface/Packet.h:838`. The server resolves it with
//!   `CHARACTER_MANAGER::instance().Find(p->leader_vid)` at
//!   `input_main.cpp:2507`, which is a virtual-ID lookup, so the server's name
//!   is the accurate one and the client's is a misnomer.
//! - `TPacketCGPartySetState`: all four members differ. The server has
//!   `header`, `pid`, `byRole`, `flag` at `packet.h:2077`; the client has
//!   `byHeader`, `dwVID`, `byState`, `byFlag` at `Packet.h:851`. The client's
//!   `dwVID` is a **misnomer**: the server compares the same 4 bytes against
//!   `pParty->IsMember(p->pid)` at `input_main.cpp:2539` and against
//!   `ch->GetPlayerID()` at `:2620` and `:2642`, so the value is a player ID, not
//!   a virtual ID. An earlier draft of this note also cited `GetLeaderPID()` at
//!   `:2533` and `:2604`; both of those lines compare the *sender's* party
//!   leadership, `GetLeaderPID() != ch->GetPlayerID()`, and never read
//!   `p->pid`.
//! - `TPacketCGPartyUseSkill`, already implemented elsewhere, drifts the same
//!   way: server `header`/`vid` against client `byHeader`/`dwTargetVID`.
//!
//! # What these codecs deliberately do not do
//!
//! Every field stays opaque. The bounds and the gameplay policy all live above
//! the record boundary in `CInputMain`, and none of it is reproduced here.
//!
//! - `accept` is a truthiness byte. `CInputMain::PartyInviteAnswer` tests
//!   `!p->accept` at `input_main.cpp:2513`, so **any** non-zero byte accepts and
//!   only `0` denies. This codec keeps all 256 values and applies no such rule.
//! - `by_role` is range-checked above framing, by a `switch` over the
//!   `PARTY_ROLE_*` enumerators at `input_main.cpp:2548-2575` whose `default`
//!   arm only logs `sys_err`. This codec keeps all 256 values and never rejects.
//! - `flag` is a truthiness byte used directly in the `sys_log` format at
//!   `input_main.cpp:2546` and passed to `SetRole` at `:2559`.
//! - `pid` and `vid` are unchecked 4-byte identifiers. `PartyRemove` compares
//!   `p->pid` against `ch->GetPlayerID()` at `input_main.cpp:2620`, and
//!   `PartyInvite` feeds `p->vid` straight to `CHARACTER_MANAGER::Find` at
//!   `:2486`, so `0` and `u32::MAX` are both legal wire values.
//! - `distribute_mode` is passed unvalidated to `SetParameter` at
//!   `input_main.cpp:2771`.
//!
//! The codecs do not resolve parties, check leadership, check arena or dungeon
//! state, look up characters, build the `HEADER_GD_PARTY_STATE_CHANGE` DB
//! request, or send any reply. None of the five dispatch arms is wrapped in an
//! `IsObserverMode()` guard, and none of them touches `iExtraLen`, so all five
//! are fixed-width on the server path.
//!
//! The five client senders are all in
//! `client/Client/UserInterface/PythonNetworkStreamPhaseGame.cpp:3420-3518`.
//! Every one assigns all of its fields before sending and sends `sizeof(...)`.
//! None of them calls `__CanActMainInstance()`: that guard appears 16 times in
//! `PythonNetworkStreamPhaseGameItem.cpp` and 23 times elsewhere in
//! `PythonNetworkStreamPhaseGame.cpp`, including outside this range, so it is
//! applied per sender rather than omitted for whole files. The six party
//! senders are the exception, not a file-wide convention.

use crate::cg_inventory::{
    CgHeader, HEADER_CG_PARTY_INVITE, HEADER_CG_PARTY_INVITE_ANSWER, HEADER_CG_PARTY_PARAMETER,
    HEADER_CG_PARTY_REMOVE, HEADER_CG_PARTY_SET_STATE,
};
use crate::cg_wire::ClientFrame;

/// The full legacy `TPacketCGPartyInvite` record, header byte included.
pub const CG_PARTY_INVITE_WIRE_SIZE: usize = 5;
/// The framed payload of `TPacketCGPartyInvite`, everything after the header.
pub const CG_PARTY_INVITE_PAYLOAD_SIZE: usize = 4;
/// The full legacy `TPacketCGPartyInviteAnswer` record, header byte included.
pub const CG_PARTY_INVITE_ANSWER_WIRE_SIZE: usize = 6;
/// The framed payload of `TPacketCGPartyInviteAnswer`.
pub const CG_PARTY_INVITE_ANSWER_PAYLOAD_SIZE: usize = 5;
/// The full legacy `TPacketCGPartyRemove` record, header byte included.
pub const CG_PARTY_REMOVE_WIRE_SIZE: usize = 5;
/// The framed payload of `TPacketCGPartyRemove`.
pub const CG_PARTY_REMOVE_PAYLOAD_SIZE: usize = 4;
/// The full legacy `TPacketCGPartySetState` record, header byte included.
pub const CG_PARTY_SET_STATE_WIRE_SIZE: usize = 7;
/// The framed payload of `TPacketCGPartySetState`.
pub const CG_PARTY_SET_STATE_PAYLOAD_SIZE: usize = 6;
/// The full legacy `TPacketCGPartyParameter` record, header byte included.
pub const CG_PARTY_PARAMETER_WIRE_SIZE: usize = 2;
/// The framed payload of `TPacketCGPartyParameter`.
pub const CG_PARTY_PARAMETER_PAYLOAD_SIZE: usize = 1;

/// Every way one of these five decoders can refuse a byte slice.
///
/// The three failure modes are identical across the family, so the family
/// shares one error type rather than five near-identical ones. `InvalidHeader`
/// carries the expected value so the message still names the right record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgPartyError {
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

impl std::fmt::Display for CgPartyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(f, "party record needs {needed} bytes, got {available}")
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "party record must be exactly {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { expected, actual } => {
                write!(f, "party header {actual} is not {expected}")
            }
        }
    }
}

impl std::error::Error for CgPartyError {}

/// Read one little-endian `u32`. Explicit, never a packed struct.
fn read_u32_le(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Append one little-endian `u32`.
fn write_u32_le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Check a complete record width, then its header.
fn decode_parts(bytes: &[u8], wire_size: usize, expected: u8) -> Result<(), CgPartyError> {
    check_exact(bytes.len(), wire_size)?;
    check_header(bytes[0], expected)
}

/// Check a header-less frame payload width, then its header.
fn decode_frame_parts(
    frame: &ClientFrame,
    payload_size: usize,
    expected: u8,
) -> Result<(), CgPartyError> {
    check_exact(frame.payload.len(), payload_size)?;
    check_header(frame.header, expected)
}

fn check_exact(len: usize, size: usize) -> Result<(), CgPartyError> {
    if len < size {
        return Err(CgPartyError::Truncated {
            needed: size,
            available: len,
        });
    }
    if len > size {
        return Err(CgPartyError::LengthMismatch {
            expected: size,
            actual: len,
        });
    }
    Ok(())
}

fn check_header(actual: u8, expected: u8) -> Result<(), CgPartyError> {
    if actual != expected {
        return Err(CgPartyError::InvalidHeader { expected, actual });
    }
    Ok(())
}

/// The transport-free `TPacketCGPartyInvite` record, header 72.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgPartyInvite {
    /// The invitee's virtual ID. Opaque: `CInputMain::PartyInvite` feeds it
    /// straight to `CHARACTER_MANAGER::Find` and never range-checks it.
    pub vid: u32,
}

impl CgPartyInvite {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_PARTY_INVITE
    }

    /// Build a record from an invitee virtual ID. No value is rejected.
    pub const fn new(vid: u32) -> Self {
        Self { vid }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        write_u32_le(out, self.vid);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_PARTY_INVITE_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame::new(Self::header().value(), self.vid.to_le_bytes())
    }

    /// # Errors
    ///
    /// Returns [`CgPartyError::Truncated`] for a short input,
    /// [`CgPartyError::LengthMismatch`] for a long input, and
    /// [`CgPartyError::InvalidHeader`] when the header is not 72. The length is
    /// checked before the header, and the header before any payload byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgPartyError> {
        decode_parts(bytes, CG_PARTY_INVITE_WIRE_SIZE, Self::header().value())?;
        Ok(Self {
            vid: read_u32_le(bytes, 1),
        })
    }

    /// # Errors
    ///
    /// As [`CgPartyInvite::decode`], except that the frame payload excludes the
    /// header byte, so the **payload** width is checked before the frame header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgPartyError> {
        decode_frame_parts(frame, CG_PARTY_INVITE_PAYLOAD_SIZE, Self::header().value())?;
        Ok(Self {
            vid: read_u32_le(&frame.payload, 0),
        })
    }
}

/// The transport-free `TPacketCGPartyInviteAnswer` record, header 73.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgPartyInviteAnswer {
    /// The inviter's virtual ID, named `leader_vid` by the server and
    /// `leader_pid` by the client. Opaque and unchecked.
    pub leader_vid: u32,
    /// The answer byte. Opaque here: the server reads it as a truthiness value
    /// at `input_main.cpp:2513`, so every non-zero byte accepts.
    pub accept: u8,
}

impl CgPartyInviteAnswer {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_PARTY_INVITE_ANSWER
    }

    /// Build a record from a leader virtual ID and an answer byte.
    pub const fn new(leader_vid: u32, accept: u8) -> Self {
        Self { leader_vid, accept }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        write_u32_le(out, self.leader_vid);
        out.push(self.accept);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_PARTY_INVITE_ANSWER_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_PARTY_INVITE_ANSWER_PAYLOAD_SIZE);
        write_u32_le(&mut payload, self.leader_vid);
        payload.push(self.accept);
        ClientFrame::new(Self::header().value(), payload)
    }

    /// # Errors
    ///
    /// Returns [`CgPartyError::Truncated`] for a short input,
    /// [`CgPartyError::LengthMismatch`] for a long input, and
    /// [`CgPartyError::InvalidHeader`] when the header is not 73. The length is
    /// checked before the header, and the header before any payload byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgPartyError> {
        decode_parts(
            bytes,
            CG_PARTY_INVITE_ANSWER_WIRE_SIZE,
            Self::header().value(),
        )?;
        Ok(Self {
            leader_vid: read_u32_le(bytes, 1),
            accept: bytes[5],
        })
    }

    /// # Errors
    ///
    /// As [`CgPartyInviteAnswer::decode`], except that the **payload** width is
    /// checked before the frame header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgPartyError> {
        decode_frame_parts(
            frame,
            CG_PARTY_INVITE_ANSWER_PAYLOAD_SIZE,
            Self::header().value(),
        )?;
        Ok(Self {
            leader_vid: read_u32_le(&frame.payload, 0),
            accept: frame.payload[4],
        })
    }
}

/// The transport-free `TPacketCGPartyRemove` record, header 74.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgPartyRemove {
    /// The player ID being removed. Opaque: the server compares it against the
    /// sender's own ID at `input_main.cpp:2620` and never range-checks it.
    pub pid: u32,
}

impl CgPartyRemove {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_PARTY_REMOVE
    }

    /// Build a record from a player ID. No value is rejected.
    pub const fn new(pid: u32) -> Self {
        Self { pid }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        write_u32_le(out, self.pid);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_PARTY_REMOVE_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame::new(Self::header().value(), self.pid.to_le_bytes())
    }

    /// # Errors
    ///
    /// Returns [`CgPartyError::Truncated`] for a short input,
    /// [`CgPartyError::LengthMismatch`] for a long input, and
    /// [`CgPartyError::InvalidHeader`] when the header is not 74. The length is
    /// checked before the header, and the header before any payload byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgPartyError> {
        decode_parts(bytes, CG_PARTY_REMOVE_WIRE_SIZE, Self::header().value())?;
        Ok(Self {
            pid: read_u32_le(bytes, 1),
        })
    }

    /// # Errors
    ///
    /// As [`CgPartyRemove::decode`], except that the **payload** width is
    /// checked before the frame header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgPartyError> {
        decode_frame_parts(frame, CG_PARTY_REMOVE_PAYLOAD_SIZE, Self::header().value())?;
        Ok(Self {
            pid: read_u32_le(&frame.payload, 0),
        })
    }
}

/// The transport-free `TPacketCGPartySetState` record, header 75.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgPartySetState {
    /// The player ID whose role changes. The client calls this `dwVID`, but the
    /// server matches it with `IsMember` and against `ch->GetPlayerID()`, so it
    /// is a player ID. Opaque and unchecked.
    pub pid: u32,
    /// The role byte. Opaque here: the server switches over `PARTY_ROLE_*` at
    /// `input_main.cpp:2548` and its `default` arm only logs.
    pub by_role: u8,
    /// The flag byte, read as a truthiness value.
    pub flag: u8,
}

impl CgPartySetState {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_PARTY_SET_STATE
    }

    /// Build a record from a player ID, a role byte, and a flag byte.
    pub const fn new(pid: u32, by_role: u8, flag: u8) -> Self {
        Self { pid, by_role, flag }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        write_u32_le(out, self.pid);
        out.push(self.by_role);
        out.push(self.flag);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_PARTY_SET_STATE_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_PARTY_SET_STATE_PAYLOAD_SIZE);
        write_u32_le(&mut payload, self.pid);
        payload.push(self.by_role);
        payload.push(self.flag);
        ClientFrame::new(Self::header().value(), payload)
    }

    /// # Errors
    ///
    /// Returns [`CgPartyError::Truncated`] for a short input,
    /// [`CgPartyError::LengthMismatch`] for a long input, and
    /// [`CgPartyError::InvalidHeader`] when the header is not 75. The length is
    /// checked before the header, and the header before any payload byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgPartyError> {
        decode_parts(bytes, CG_PARTY_SET_STATE_WIRE_SIZE, Self::header().value())?;
        Ok(Self {
            pid: read_u32_le(bytes, 1),
            by_role: bytes[5],
            flag: bytes[6],
        })
    }

    /// # Errors
    ///
    /// As [`CgPartySetState::decode`], except that the **payload** width is
    /// checked before the frame header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgPartyError> {
        decode_frame_parts(
            frame,
            CG_PARTY_SET_STATE_PAYLOAD_SIZE,
            Self::header().value(),
        )?;
        Ok(Self {
            pid: read_u32_le(&frame.payload, 0),
            by_role: frame.payload[4],
            flag: frame.payload[5],
        })
    }
}

/// The transport-free `TPacketCGPartyParameter` record, header 78.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgPartyParameter {
    /// The item-distribution mode. Opaque: the server passes it straight to
    /// `SetParameter` at `input_main.cpp:2771` without validating it.
    pub distribute_mode: u8,
}

impl CgPartyParameter {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_PARTY_PARAMETER
    }

    /// Build a record from a distribution mode. No value is rejected.
    pub const fn new(distribute_mode: u8) -> Self {
        Self { distribute_mode }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.distribute_mode);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_PARTY_PARAMETER_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame::new(Self::header().value(), [self.distribute_mode])
    }

    /// # Errors
    ///
    /// Returns [`CgPartyError::Truncated`] for a short input,
    /// [`CgPartyError::LengthMismatch`] for a long input, and
    /// [`CgPartyError::InvalidHeader`] when the header is not 78. The length is
    /// checked before the header, and the header before any payload byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgPartyError> {
        decode_parts(bytes, CG_PARTY_PARAMETER_WIRE_SIZE, Self::header().value())?;
        Ok(Self {
            distribute_mode: bytes[1],
        })
    }

    /// # Errors
    ///
    /// As [`CgPartyParameter::decode`], except that the **payload** width is
    /// checked before the frame header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgPartyError> {
        decode_frame_parts(
            frame,
            CG_PARTY_PARAMETER_PAYLOAD_SIZE,
            Self::header().value(),
        )?;
        Ok(Self {
            distribute_mode: frame.payload[0],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H_INVITE: u8 = 72;
    const H_ANSWER: u8 = 73;
    const H_REMOVE: u8 = 74;
    const H_SET_STATE: u8 = 75;
    const H_PARAMETER: u8 = 78;

    // ---- golden bytes -----------------------------------------------------

    #[test]
    fn invite_golden_bytes_are_exact() {
        assert_eq!(
            CgPartyInvite::new(0x1122_3344).encode(),
            vec![H_INVITE, 0x44, 0x33, 0x22, 0x11]
        );
    }

    #[test]
    fn answer_golden_bytes_are_exact() {
        assert_eq!(
            CgPartyInviteAnswer::new(0x0a0b_0c0d, 1).encode(),
            vec![H_ANSWER, 0x0d, 0x0c, 0x0b, 0x0a, 1]
        );
    }

    #[test]
    fn remove_golden_bytes_are_exact() {
        assert_eq!(
            CgPartyRemove::new(0xdead_beef).encode(),
            vec![H_REMOVE, 0xef, 0xbe, 0xad, 0xde]
        );
    }

    #[test]
    fn set_state_golden_bytes_are_exact() {
        assert_eq!(
            CgPartySetState::new(0x0102_0304, 2, 1).encode(),
            vec![H_SET_STATE, 0x04, 0x03, 0x02, 0x01, 2, 1]
        );
    }

    #[test]
    fn parameter_golden_bytes_are_exact() {
        assert_eq!(CgPartyParameter::new(3).encode(), vec![H_PARAMETER, 3]);
    }

    // ---- declared widths --------------------------------------------------

    #[test]
    fn declared_widths_match_the_inventory_rows() {
        assert_eq!(CG_PARTY_INVITE_WIRE_SIZE, 5);
        assert_eq!(CG_PARTY_INVITE_PAYLOAD_SIZE, 4);
        assert_eq!(CG_PARTY_INVITE_ANSWER_WIRE_SIZE, 6);
        assert_eq!(CG_PARTY_INVITE_ANSWER_PAYLOAD_SIZE, 5);
        assert_eq!(CG_PARTY_REMOVE_WIRE_SIZE, 5);
        assert_eq!(CG_PARTY_REMOVE_PAYLOAD_SIZE, 4);
        assert_eq!(CG_PARTY_SET_STATE_WIRE_SIZE, 7);
        assert_eq!(CG_PARTY_SET_STATE_PAYLOAD_SIZE, 6);
        assert_eq!(CG_PARTY_PARAMETER_WIRE_SIZE, 2);
        assert_eq!(CG_PARTY_PARAMETER_PAYLOAD_SIZE, 1);
    }

    #[test]
    fn each_payload_size_is_the_wire_size_minus_one() {
        assert_eq!(CG_PARTY_INVITE_WIRE_SIZE - 1, CG_PARTY_INVITE_PAYLOAD_SIZE);
        assert_eq!(
            CG_PARTY_INVITE_ANSWER_WIRE_SIZE - 1,
            CG_PARTY_INVITE_ANSWER_PAYLOAD_SIZE
        );
        assert_eq!(CG_PARTY_REMOVE_WIRE_SIZE - 1, CG_PARTY_REMOVE_PAYLOAD_SIZE);
        assert_eq!(
            CG_PARTY_SET_STATE_WIRE_SIZE - 1,
            CG_PARTY_SET_STATE_PAYLOAD_SIZE
        );
        assert_eq!(
            CG_PARTY_PARAMETER_WIRE_SIZE - 1,
            CG_PARTY_PARAMETER_PAYLOAD_SIZE
        );
    }

    #[test]
    fn every_encoded_record_has_the_declared_width() {
        assert_eq!(
            CgPartyInvite::new(1).encode().len(),
            CG_PARTY_INVITE_WIRE_SIZE
        );
        assert_eq!(
            CgPartyInviteAnswer::new(1, 1).encode().len(),
            CG_PARTY_INVITE_ANSWER_WIRE_SIZE
        );
        assert_eq!(
            CgPartyRemove::new(1).encode().len(),
            CG_PARTY_REMOVE_WIRE_SIZE
        );
        assert_eq!(
            CgPartySetState::new(1, 1, 1).encode().len(),
            CG_PARTY_SET_STATE_WIRE_SIZE
        );
        assert_eq!(
            CgPartyParameter::new(1).encode().len(),
            CG_PARTY_PARAMETER_WIRE_SIZE
        );
    }

    // ---- header constants -------------------------------------------------

    #[test]
    fn header_values_are_the_legacy_ones() {
        assert_eq!(CgPartyInvite::header().value(), H_INVITE);
        assert_eq!(CgPartyInviteAnswer::header().value(), H_ANSWER);
        assert_eq!(CgPartyRemove::header().value(), H_REMOVE);
        assert_eq!(CgPartySetState::header().value(), H_SET_STATE);
        assert_eq!(CgPartyParameter::header().value(), H_PARAMETER);
    }

    // ---- round trips over the whole domain --------------------------------

    #[test]
    fn u32_fields_round_trip_every_boundary() {
        for v in [
            0u32,
            1,
            0x7f,
            0x80,
            0xff,
            0x100,
            0x7fff_ffff,
            0x8000_0000,
            u32::MAX,
        ] {
            assert_eq!(
                CgPartyInvite::decode(&CgPartyInvite::new(v).encode())
                    .unwrap()
                    .vid,
                v
            );
            assert_eq!(
                CgPartyRemove::decode(&CgPartyRemove::new(v).encode())
                    .unwrap()
                    .pid,
                v
            );
            assert_eq!(
                CgPartySetState::decode(&CgPartySetState::new(v, 0, 0).encode())
                    .unwrap()
                    .pid,
                v
            );
            assert_eq!(
                CgPartyInviteAnswer::decode(&CgPartyInviteAnswer::new(v, 0).encode())
                    .unwrap()
                    .leader_vid,
                v
            );
        }
    }

    #[test]
    fn u8_fields_round_trip_every_possible_value() {
        for v in 0u8..=u8::MAX {
            assert_eq!(
                CgPartyParameter::decode(&CgPartyParameter::new(v).encode())
                    .unwrap()
                    .distribute_mode,
                v
            );
            assert_eq!(
                CgPartyInviteAnswer::decode(&CgPartyInviteAnswer::new(7, v).encode())
                    .unwrap()
                    .accept,
                v
            );
            let r = CgPartySetState::new(7, v, v.wrapping_add(1)).encode();
            let back = CgPartySetState::decode(&r).unwrap();
            assert_eq!(back.by_role, v);
            assert_eq!(back.flag, v.wrapping_add(1));
        }
    }

    #[test]
    fn decode_frame_round_trips_through_to_frame() {
        let invite = CgPartyInvite::new(u32::MAX);
        assert_eq!(
            CgPartyInvite::decode_frame(&invite.to_frame()).unwrap(),
            invite
        );
        let answer = CgPartyInviteAnswer::new(42, 0xfe);
        assert_eq!(
            CgPartyInviteAnswer::decode_frame(&answer.to_frame()).unwrap(),
            answer
        );
        let remove = CgPartyRemove::new(0x8000_0000);
        assert_eq!(
            CgPartyRemove::decode_frame(&remove.to_frame()).unwrap(),
            remove
        );
        let set_state = CgPartySetState::new(1, 6, 1);
        assert_eq!(
            CgPartySetState::decode_frame(&set_state.to_frame()).unwrap(),
            set_state
        );
        let parameter = CgPartyParameter::new(0xab);
        assert_eq!(
            CgPartyParameter::decode_frame(&parameter.to_frame()).unwrap(),
            parameter
        );
    }

    #[test]
    fn to_frame_keeps_the_header_out_of_the_payload() {
        let f = CgPartySetState::new(0x1122_3344, 1, 0).to_frame();
        assert_eq!(f.header, H_SET_STATE);
        assert_eq!(f.payload.len(), CG_PARTY_SET_STATE_PAYLOAD_SIZE);
        assert_eq!(&f.payload, &[0x44, 0x33, 0x22, 0x11, 1, 0]);
    }

    // ---- exact-length behaviour -------------------------------------------

    #[test]
    fn short_input_is_truncated_with_both_lengths() {
        let err = CgPartySetState::decode(&[H_SET_STATE, 0, 0, 0]).unwrap_err();
        assert_eq!(
            err,
            CgPartyError::Truncated {
                needed: 7,
                available: 4
            }
        );
    }

    #[test]
    fn long_input_is_a_length_mismatch() {
        let err = CgPartyParameter::decode(&[H_PARAMETER, 0, 0]).unwrap_err();
        assert_eq!(
            err,
            CgPartyError::LengthMismatch {
                expected: 2,
                actual: 3
            }
        );
    }

    #[test]
    fn empty_input_is_truncated_not_a_panic() {
        assert!(matches!(
            CgPartyInvite::decode(&[]),
            Err(CgPartyError::Truncated { .. })
        ));
    }

    #[test]
    fn the_length_is_checked_before_the_header() {
        // Wrong header AND too long: the length error must win, even though the
        // header byte is also wrong.
        assert!(matches!(
            CgPartyRemove::decode(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
            Err(CgPartyError::LengthMismatch { .. })
        ));
        // Wrong header AND too short: still a length error, not a header error.
        assert!(matches!(
            CgPartyRemove::decode(&[0x00, 0x00]),
            Err(CgPartyError::Truncated { .. })
        ));
        // Wrong header but the right length: the header error must win.
        assert_eq!(
            CgPartyRemove::decode(&[0x00, 1, 2, 3, 4]).unwrap_err(),
            CgPartyError::InvalidHeader {
                expected: 74,
                actual: 0
            }
        );
    }

    #[test]
    fn the_frame_payload_length_is_checked_before_the_frame_header() {
        // Header is wrong, payload is the right width: header error.
        assert_eq!(
            CgPartyInvite::decode_frame(&ClientFrame::new(99, [0u8; 4])).unwrap_err(),
            CgPartyError::InvalidHeader {
                expected: 72,
                actual: 99
            }
        );
        // Header is right, payload is the wrong width: length error.
        assert_eq!(
            CgPartyInvite::decode_frame(&ClientFrame::new(H_INVITE, [0u8; 5])).unwrap_err(),
            CgPartyError::LengthMismatch {
                expected: 4,
                actual: 5
            }
        );
    }

    #[test]
    fn a_wrong_header_at_the_exact_length_is_rejected() {
        assert_eq!(
            CgPartyInviteAnswer::decode(&[0, 0, 0, 0, 0, 0]).unwrap_err(),
            CgPartyError::InvalidHeader {
                expected: 73,
                actual: 0
            }
        );
        assert_eq!(
            CgPartyParameter::decode(&[0, 0]).unwrap_err(),
            CgPartyError::InvalidHeader {
                expected: 78,
                actual: 0
            }
        );
    }

    #[test]
    fn a_one_byte_frame_payload_is_never_read_as_a_full_record() {
        // Regression guard for the decode_frame payload-width defect found in
        // Section 151: the payload check must use the payload size, never the
        // full record size.
        assert!(matches!(
            CgPartyParameter::decode_frame(&ClientFrame::new(H_PARAMETER, [])),
            Err(CgPartyError::Truncated {
                needed: 1,
                available: 0
            })
        ));
    }

    // ---- append semantics -------------------------------------------------

    #[test]
    fn encode_into_appends_and_does_not_clear() {
        let mut out = vec![0xAA, 0xBB];
        CgPartyParameter::new(9).encode_into(&mut out);
        assert_eq!(out, vec![0xAA, 0xBB, H_PARAMETER, 9]);
    }

    /// Mutation P14 survived the first sweep because the test above covers only
    /// `CgPartyParameter`, so clearing the buffer inside any *other* record's
    /// `encode_into` went undetected. This pins append semantics for all five.
    #[test]
    fn every_encode_into_appends_and_none_of_them_clears() {
        let sentinel = vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00, 0x11];
        let records: [(&[u8], &str); 5] = [
            (&CgPartyInvite::new(0x0102_0304).encode(), "invite"),
            (&CgPartyInviteAnswer::new(0x0102_0304, 5).encode(), "answer"),
            (&CgPartyRemove::new(0x0102_0304).encode(), "remove"),
            (
                &CgPartySetState::new(0x0102_0304, 5, 6).encode(),
                "set_state",
            ),
            (&CgPartyParameter::new(5).encode(), "parameter"),
        ];
        for (encoded, name) in records {
            let mut out = sentinel.clone();
            match name {
                "invite" => CgPartyInvite::new(0x0102_0304).encode_into(&mut out),
                "answer" => CgPartyInviteAnswer::new(0x0102_0304, 5).encode_into(&mut out),
                "remove" => CgPartyRemove::new(0x0102_0304).encode_into(&mut out),
                "set_state" => CgPartySetState::new(0x0102_0304, 5, 6).encode_into(&mut out),
                "parameter" => CgPartyParameter::new(5).encode_into(&mut out),
                _ => unreachable!(),
            }
            let mut want = sentinel.clone();
            want.extend_from_slice(encoded);
            assert_eq!(out, want, "{name} encode_into must append, not clear");
        }
    }

    #[test]
    fn encode_into_twice_appends_two_records() {
        let mut out = Vec::new();
        CgPartyRemove::new(1).encode_into(&mut out);
        CgPartyRemove::new(2).encode_into(&mut out);
        assert_eq!(out.len(), 2 * CG_PARTY_REMOVE_WIRE_SIZE);
    }

    // ---- no gameplay policy in the codec ----------------------------------

    #[test]
    fn the_role_byte_is_not_range_checked() {
        for v in 0u8..=u8::MAX {
            let ok = CgPartySetState::decode(&CgPartySetState::new(1, v, 0).encode());
            assert!(
                ok.is_ok(),
                "role {v} must stay legal at the record boundary"
            );
        }
    }

    #[test]
    fn a_nonzero_accept_byte_is_not_normalised_to_true() {
        let back = CgPartyInviteAnswer::decode(&CgPartyInviteAnswer::new(1, 0x42).encode())
            .unwrap()
            .accept;
        assert_eq!(back, 0x42);
    }

    #[test]
    fn the_distribution_mode_is_not_range_checked() {
        for v in 0u8..=u8::MAX {
            assert!(CgPartyParameter::new(v).encode().len() == CG_PARTY_PARAMETER_WIRE_SIZE);
        }
    }

    #[test]
    fn no_u32_value_is_rejected() {
        assert!(CgPartyInvite::new(0).encode().len() == CG_PARTY_INVITE_WIRE_SIZE);
        assert!(CgPartyRemove::new(u32::MAX).encode().len() == CG_PARTY_REMOVE_WIRE_SIZE);
    }

    // ---- error display ----------------------------------------------------

    #[test]
    fn error_messages_name_the_expected_header() {
        let e = CgPartySetState::decode(&[0, 0, 0, 0, 0, 0, 0]).unwrap_err();
        assert_eq!(e.to_string(), "party header 0 is not 75");
        let t = CgPartySetState::decode(&[0, 0]).unwrap_err();
        assert_eq!(t.to_string(), "party record needs 7 bytes, got 2");
        let l = CgPartySetState::decode(&[0; 9]).unwrap_err();
        assert_eq!(l.to_string(), "party record must be exactly 7 bytes, got 9");
    }

    #[test]
    fn the_five_headers_are_distinct() {
        let hs = [
            CgPartyInvite::header().value(),
            CgPartyInviteAnswer::header().value(),
            CgPartyRemove::header().value(),
            CgPartySetState::header().value(),
            CgPartyParameter::header().value(),
        ];
        for i in 0..hs.len() {
            for j in (i + 1)..hs.len() {
                assert_ne!(hs[i], hs[j], "headers {i} and {j} collide");
            }
        }
    }

    #[test]
    fn invite_and_remove_are_the_same_shape_but_not_interchangeable() {
        // Both are 5 bytes, which is exactly why the header check matters.
        assert_eq!(CG_PARTY_INVITE_WIRE_SIZE, CG_PARTY_REMOVE_WIRE_SIZE);
        let bytes = CgPartyInvite::new(0x0102_0304).encode();
        assert_eq!(CgPartyInvite::decode(&bytes).unwrap().vid, 0x0102_0304);
        assert_eq!(
            CgPartyRemove::decode(&bytes).unwrap_err(),
            CgPartyError::InvalidHeader {
                expected: 74,
                actual: 72
            }
        );
    }
}
