//! Explicit codecs for the four fixed-width game-phase records that carry a
//! single client choice and nothing else.
//!
//! Legacy evidence, all inside the single `#pragma pack(1)` at
//! `server/server/game/packet.h:274`-`:3540`:
//!
//! - `TPacketCGQuestConfirm` at `packet.h:764-769`, header 31 (`0x1f`).
//! - `TPacketCGTarget` at `packet.h:1795-1799`, header 61 (`0x3d`).
//! - `TPacketCGScriptButton` at `packet.h:752-756`, header 66 (`0x42`).
//! - `TPacketCGScriptSelectItem` at `packet.h:2582-2586`, header 114 (`0x72`).
//!
//! All four are registered unconditionally in `packet_info.cpp` and dispatched
//! from `CInputMain::Analyze`.
//!
//! ## Why these four are grouped
//!
//! They share one wire shape: a header byte and a single client-declared
//! choice, with no acknowledgement, no sequence number, and no reply width that
//! the client must know in advance. Grouping them is about the wire, not about
//! gameplay — three are quest or script records and one, `Target`, is a world
//! lookup — and the module name describes the shared shape rather than any
//! legacy type name.
//!
//! ## A conditional-scan trap, recorded because it is easy to repeat
//!
//! All four declarations sit at preprocessor `#if` depth 1, which a naive
//! conditional-context scan reports as "gated". It is not gated. The single
//! open `#if` is the `#ifndef __INC_PACKET_H__` include guard at
//! `packet.h:1`-`:2`, which **never closes** — the file is 3,542 lines and ends
//! inside the guard. Any conditional map that does not special-case a
//! file-spanning include guard marks every declaration in `packet.h` as
//! conditional, which makes the map worse than having no map at all.
//!
//! ## The field-order surprise in `TPacketCGQuestConfirm`
//!
//! `packet.h:766-768` declares, in this order:
//!
//! ```c
//! BYTE  header;
//! BYTE  answer;
//! DWORD requestPID;
//! ```
//!
//! so the single byte comes **before** the word, and a "narrow field first, wide
//! field last" assumption produces the right total width and the wrong six
//! bytes. [`CgQuestConfirm`] reads `answer` at offset 1 and `request_pid` at
//! offset 2, and a golden test pins the exact byte order for that reason.
//!
//! ## Four different reasons the fields stay opaque
//!
//! Opacity is not a single decision here, and the four records are opaque for
//! four different reasons. All four fields are preserved across their full
//! domain regardless.
//!
//! - [`CgQuestConfirm::answer`] is a **truthiness** byte.
//!   `CInputMain::QuestConfirm` at `input_main.cpp:2236` does
//!   `if (p->answer) p->answer = quest::CONFIRM_YES;`, so all 255 non-zero values
//!   collapse server-side. That collapse is the server's policy and not framing's,
//!   so this codec still round-trips all 256 values.
//! - [`CgQuestConfirm::request_pid`] is **not the sender**. The handler calls
//!   `FindByPID(p->requestPID)` and then `Confirm(ch_wait->GetPlayerID(), ...)`,
//!   where `ch_wait` is a *different* character resolved from the wire. The
//!   record therefore lets one client speak for another. That is an
//!   admission-policy question and is recorded, not enforced here.
//! - [`CgScriptButton::idx`] is **behaviourally inert**. `CInputMain::ScriptButton`
//!   at `input_main.cpp:2179` logs `p->idx` at `:2182` and then calls
//!   `Confirm(ch->GetPlayerID(), quest::CONFIRM_TIMEOUT)`, a call that takes no
//!   index. The value reaches a log line and nothing else, so validating it
//!   against a button count would invent a constraint the server does not have.
//! - [`CgScriptSelectItem::selection`] and [`CgTarget::vid`] go straight to
//!   `quest::CQuestManager::SelectItem` and `FindObjectByVID` with no validation
//!   at all.
//!
//! ## A one-sided profile note
//!
//! The `HEADER_CG_TARGET` **request** is 5 bytes in every profile, but its reply
//! is not: `#if defined(__SHIP_DEFENSE__)` at `input_main.cpp:2260-2264` adds
//! `bAlliance`, `iAllianceMinHP`, and `iAllianceMaxHP` to `TPacketGCTarget`. A CG
//! codec for this record must be sized from the CG declaration, never from the
//! GC side. This one is, and [`CG_TARGET_WIRE_SIZE`] is 5 unconditionally.
//!
//! Header 66 is the only one of these four values with **no** collision in any
//! bucket in either tree, across CG, GC, and GG. The other three each share
//! their value with a GC header and, for 31, a GG header. That is a fact worth
//! keeping in mind before building any future cross-direction table.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::cg_inventory::{
    CgHeader, HEADER_CG_QUEST_CONFIRM, HEADER_CG_SCRIPT_BUTTON, HEADER_CG_SCRIPT_SELECT_ITEM,
    HEADER_CG_TARGET,
};
use crate::cg_wire::ClientFrame;

/// The full legacy `TPacketCGQuestConfirm` record, header byte included.
pub const CG_QUEST_CONFIRM_WIRE_SIZE: usize = 6;
/// The framed payload of `TPacketCGQuestConfirm`, everything after the header.
pub const CG_QUEST_CONFIRM_PAYLOAD_SIZE: usize = 5;
/// The full legacy `TPacketCGTarget` record, header byte included.
pub const CG_TARGET_WIRE_SIZE: usize = 5;
/// The framed payload of `TPacketCGTarget`.
pub const CG_TARGET_PAYLOAD_SIZE: usize = 4;
/// The full legacy `TPacketCGScriptButton` record, header byte included.
pub const CG_SCRIPT_BUTTON_WIRE_SIZE: usize = 5;
/// The framed payload of `TPacketCGScriptButton`.
pub const CG_SCRIPT_BUTTON_PAYLOAD_SIZE: usize = 4;
/// The full legacy `TPacketCGScriptSelectItem` record, header byte included.
pub const CG_SCRIPT_SELECT_ITEM_WIRE_SIZE: usize = 5;
/// The framed payload of `TPacketCGScriptSelectItem`.
pub const CG_SCRIPT_SELECT_ITEM_PAYLOAD_SIZE: usize = 4;

/// Every way one of these four decoders can refuse a byte slice.
///
/// The three failure modes are identical across the four records, so they share
/// one error type rather than four near-identical ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgPickError {
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

impl fmt::Display for CgPickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(f, "CG record needs {needed} bytes, got {available}")
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "CG record must be exactly {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { expected, actual } => {
                write!(f, "CG header {actual} is not {expected}")
            }
        }
    }
}

impl Error for CgPickError {}

fn read_u32_le(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn write_u32_le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Check a complete record width, then its header.
fn decode_parts(bytes: &[u8], wire_size: usize, expected: u8) -> Result<(), CgPickError> {
    check_exact(bytes.len(), wire_size)?;
    check_header(bytes[0], expected)
}

/// Check a header-less frame payload width, then its header.
fn decode_frame_parts(
    frame: &ClientFrame,
    payload_size: usize,
    expected: u8,
) -> Result<(), CgPickError> {
    check_exact(frame.payload.len(), payload_size)?;
    check_header(frame.header, expected)
}

fn check_exact(len: usize, size: usize) -> Result<(), CgPickError> {
    if len < size {
        return Err(CgPickError::Truncated {
            needed: size,
            available: len,
        });
    }
    if len > size {
        return Err(CgPickError::LengthMismatch {
            expected: size,
            actual: len,
        });
    }
    Ok(())
}

fn check_header(actual: u8, expected: u8) -> Result<(), CgPickError> {
    if actual != expected {
        return Err(CgPickError::InvalidHeader { expected, actual });
    }
    Ok(())
}

/// The transport-free `TPacketCGQuestConfirm` record, header 31.
///
/// The only one of the four with a second field, and the only one whose byte
/// order is not the obvious one: `packet.h:766-768` puts the `u8` before the
/// `u32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgQuestConfirm {
    /// The answer byte. Opaque, and a **truthiness** byte rather than a small
    /// enumeration: `CInputMain::QuestConfirm` at `input_main.cpp:2236` rewrites
    /// every non-zero value to `quest::CONFIRM_YES` before the cast to
    /// `EQuestConfirmType`, so all 255 non-zero values behave identically on the
    /// server. The codec still preserves all 256.
    pub answer: u8,
    /// The character the confirmation is about. Opaque, and deliberately **not
    /// the sender**: the handler resolves it with `FindByPID` and then acts on
    /// `ch_wait->GetPlayerID()`, a different character. See the module
    /// documentation.
    pub request_pid: u32,
}

impl CgQuestConfirm {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_QUEST_CONFIRM
    }

    /// Build a record from its two opaque fields, in declaration order.
    pub const fn new(answer: u8, request_pid: u32) -> Self {
        Self {
            answer,
            request_pid,
        }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.answer);
        write_u32_le(out, self.request_pid);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_QUEST_CONFIRM_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_QUEST_CONFIRM_PAYLOAD_SIZE);
        payload.push(self.answer);
        write_u32_le(&mut payload, self.request_pid);
        ClientFrame::new(Self::header().value(), payload)
    }

    /// # Errors
    ///
    /// Returns [`CgPickError::Truncated`] for a short input,
    /// [`CgPickError::LengthMismatch`] for a long input, and
    /// [`CgPickError::InvalidHeader`] when the header is not 31. The length is
    /// checked before the header, and the header before any payload byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgPickError> {
        decode_parts(bytes, CG_QUEST_CONFIRM_WIRE_SIZE, Self::header().value())?;
        Ok(Self {
            answer: bytes[1],
            request_pid: read_u32_le(bytes, 2),
        })
    }

    /// # Errors
    ///
    /// As [`CgQuestConfirm::decode`], except that the frame payload excludes the
    /// header byte, so the **payload** width is checked before the frame header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgPickError> {
        decode_frame_parts(frame, CG_QUEST_CONFIRM_PAYLOAD_SIZE, Self::header().value())?;
        Ok(Self {
            answer: frame.payload[0],
            request_pid: read_u32_le(&frame.payload, 1),
        })
    }
}

/// The transport-free `TPacketCGTarget` record, header 61.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgTarget {
    /// The targeted virtual ID. Opaque: `CInputMain::Target` at
    /// `input_main.cpp:2249` passes it to `FindObjectByVID` and does nothing at
    /// all when that fails, so every value is legal and none is a framing error.
    pub vid: u32,
}

impl CgTarget {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_TARGET
    }

    /// Build a record from its one opaque word. No value is rejected.
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
        let mut out = Vec::with_capacity(CG_TARGET_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_TARGET_PAYLOAD_SIZE);
        write_u32_le(&mut payload, self.vid);
        ClientFrame::new(Self::header().value(), payload)
    }

    /// # Errors
    ///
    /// Returns [`CgPickError::Truncated`] for a short input,
    /// [`CgPickError::LengthMismatch`] for a long input, and
    /// [`CgPickError::InvalidHeader`] when the header is not 61. The length is
    /// checked before the header, and the header before any payload byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgPickError> {
        decode_parts(bytes, CG_TARGET_WIRE_SIZE, Self::header().value())?;
        Ok(Self {
            vid: read_u32_le(bytes, 1),
        })
    }

    /// # Errors
    ///
    /// As [`CgTarget::decode`], except that the **payload** width is checked
    /// before the frame header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgPickError> {
        decode_frame_parts(frame, CG_TARGET_PAYLOAD_SIZE, Self::header().value())?;
        Ok(Self {
            vid: read_u32_le(&frame.payload, 0),
        })
    }
}

/// The transport-free `TPacketCGScriptButton` record, header 66.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgScriptButton {
    /// The pressed button index. Opaque, and **behaviourally inert**:
    /// `CInputMain::ScriptButton` at `input_main.cpp:2179` logs it at `:2182`
    /// and then calls `Confirm(ch->GetPlayerID(), quest::CONFIRM_TIMEOUT)`,
    /// which takes no index. Validating this against a button count would invent
    /// a constraint the legacy server does not have.
    pub idx: u32,
}

impl CgScriptButton {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_SCRIPT_BUTTON
    }

    /// Build a record from its one opaque word. No value is rejected.
    pub const fn new(idx: u32) -> Self {
        Self { idx }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        write_u32_le(out, self.idx);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_SCRIPT_BUTTON_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_SCRIPT_BUTTON_PAYLOAD_SIZE);
        write_u32_le(&mut payload, self.idx);
        ClientFrame::new(Self::header().value(), payload)
    }

    /// # Errors
    ///
    /// Returns [`CgPickError::Truncated`] for a short input,
    /// [`CgPickError::LengthMismatch`] for a long input, and
    /// [`CgPickError::InvalidHeader`] when the header is not 66. The length is
    /// checked before the header, and the header before any payload byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgPickError> {
        decode_parts(bytes, CG_SCRIPT_BUTTON_WIRE_SIZE, Self::header().value())?;
        Ok(Self {
            idx: read_u32_le(bytes, 1),
        })
    }

    /// # Errors
    ///
    /// As [`CgScriptButton::decode`], except that the **payload** width is
    /// checked before the frame header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgPickError> {
        decode_frame_parts(frame, CG_SCRIPT_BUTTON_PAYLOAD_SIZE, Self::header().value())?;
        Ok(Self {
            idx: read_u32_le(&frame.payload, 0),
        })
    }
}

/// The transport-free `TPacketCGScriptSelectItem` record, header 114.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgScriptSelectItem {
    /// The selected quest item. Opaque: `CInputMain::ScriptSelectItem` at
    /// `input_main.cpp:2217` hands it to
    /// `quest::CQuestManager::Instance().SelectItem` with no validation at all.
    ///
    /// The client sender is `SendSelectItemPacket(DWORD dwItemPos)`, so the
    /// parameter is *named* for an item position while the field is a quest
    /// selection. The client name is the misleading one; the wire has no
    /// position in it.
    pub selection: u32,
}

impl CgScriptSelectItem {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_SCRIPT_SELECT_ITEM
    }

    /// Build a record from its one opaque word. No value is rejected.
    pub const fn new(selection: u32) -> Self {
        Self { selection }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        write_u32_le(out, self.selection);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_SCRIPT_SELECT_ITEM_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_SCRIPT_SELECT_ITEM_PAYLOAD_SIZE);
        write_u32_le(&mut payload, self.selection);
        ClientFrame::new(Self::header().value(), payload)
    }

    /// # Errors
    ///
    /// Returns [`CgPickError::Truncated`] for a short input,
    /// [`CgPickError::LengthMismatch`] for a long input, and
    /// [`CgPickError::InvalidHeader`] when the header is not 114. The length is
    /// checked before the header, and the header before any payload byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgPickError> {
        decode_parts(
            bytes,
            CG_SCRIPT_SELECT_ITEM_WIRE_SIZE,
            Self::header().value(),
        )?;
        Ok(Self {
            selection: read_u32_le(bytes, 1),
        })
    }

    /// # Errors
    ///
    /// As [`CgScriptSelectItem::decode`], except that the **payload** width is
    /// checked before the frame header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgPickError> {
        decode_frame_parts(
            frame,
            CG_SCRIPT_SELECT_ITEM_PAYLOAD_SIZE,
            Self::header().value(),
        )?;
        Ok(Self {
            selection: read_u32_le(&frame.payload, 0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CgPickError, CgQuestConfirm, CgScriptButton, CgScriptSelectItem, CgTarget,
        CG_QUEST_CONFIRM_PAYLOAD_SIZE, CG_QUEST_CONFIRM_WIRE_SIZE, CG_SCRIPT_BUTTON_PAYLOAD_SIZE,
        CG_SCRIPT_BUTTON_WIRE_SIZE, CG_SCRIPT_SELECT_ITEM_PAYLOAD_SIZE,
        CG_SCRIPT_SELECT_ITEM_WIRE_SIZE, CG_TARGET_PAYLOAD_SIZE, CG_TARGET_WIRE_SIZE,
    };
    use crate::cg_wire::ClientFrame;
    use std::collections::BTreeSet;

    fn sample_qc() -> CgQuestConfirm {
        CgQuestConfirm::new(0x5a, 0x0102_0304)
    }
    fn golden_qc() -> Vec<u8> {
        vec![0x1f, 0x5a, 0x04, 0x03, 0x02, 0x01]
    }
    fn sample_tg() -> CgTarget {
        CgTarget::new(0x1122_3344)
    }
    fn golden_tg() -> Vec<u8> {
        vec![0x3d, 0x44, 0x33, 0x22, 0x11]
    }
    fn sample_sb() -> CgScriptButton {
        CgScriptButton::new(0xa1b2_c3d4)
    }
    fn golden_sb() -> Vec<u8> {
        vec![0x42, 0xd4, 0xc3, 0xb2, 0xa1]
    }
    fn sample_si() -> CgScriptSelectItem {
        CgScriptSelectItem::new(0xdead_beef)
    }
    fn golden_si() -> Vec<u8> {
        vec![0x72, 0xef, 0xbe, 0xad, 0xde]
    }

    /// The four golden records, as `(header, bytes)`.
    fn all_samples() -> Vec<(u8, Vec<u8>)> {
        vec![
            (0x1f, golden_qc()),
            (0x3d, golden_tg()),
            (0x42, golden_sb()),
            (0x72, golden_si()),
        ]
    }

    // ---- widths ----------------------------------------------------------

    #[test]
    fn the_four_wire_widths_match_the_packed_declarations() {
        assert_eq!(CG_QUEST_CONFIRM_WIRE_SIZE, 6, "packet.h:764-769");
        assert_eq!(CG_TARGET_WIRE_SIZE, 5, "packet.h:1795-1799");
        assert_eq!(CG_SCRIPT_BUTTON_WIRE_SIZE, 5, "packet.h:752-756");
        assert_eq!(CG_SCRIPT_SELECT_ITEM_WIRE_SIZE, 5, "packet.h:2582-2586");
    }

    #[test]
    fn the_four_payload_widths_are_each_one_byte_smaller() {
        assert_eq!(CG_QUEST_CONFIRM_PAYLOAD_SIZE, 5);
        assert_eq!(CG_TARGET_PAYLOAD_SIZE, 4);
        assert_eq!(CG_SCRIPT_BUTTON_PAYLOAD_SIZE, 4);
        assert_eq!(CG_SCRIPT_SELECT_ITEM_PAYLOAD_SIZE, 4);
        for (w, p) in [
            (CG_QUEST_CONFIRM_WIRE_SIZE, CG_QUEST_CONFIRM_PAYLOAD_SIZE),
            (CG_TARGET_WIRE_SIZE, CG_TARGET_PAYLOAD_SIZE),
            (CG_SCRIPT_BUTTON_WIRE_SIZE, CG_SCRIPT_BUTTON_PAYLOAD_SIZE),
            (
                CG_SCRIPT_SELECT_ITEM_WIRE_SIZE,
                CG_SCRIPT_SELECT_ITEM_PAYLOAD_SIZE,
            ),
        ] {
            assert_eq!(w, p + 1, "the frame drops exactly the header byte");
        }
    }

    #[test]
    fn every_encode_is_exactly_its_declared_wire_width() {
        assert_eq!(sample_qc().encode().len(), CG_QUEST_CONFIRM_WIRE_SIZE);
        assert_eq!(sample_tg().encode().len(), CG_TARGET_WIRE_SIZE);
        assert_eq!(sample_sb().encode().len(), CG_SCRIPT_BUTTON_WIRE_SIZE);
        assert_eq!(sample_si().encode().len(), CG_SCRIPT_SELECT_ITEM_WIRE_SIZE);
    }

    #[test]
    fn the_three_single_word_records_share_one_width_and_quest_confirm_does_not() {
        let five = BTreeSet::from([
            CG_TARGET_WIRE_SIZE,
            CG_SCRIPT_BUTTON_WIRE_SIZE,
            CG_SCRIPT_SELECT_ITEM_WIRE_SIZE,
        ]);
        assert_eq!(
            five.len(),
            1,
            "the three single-word records are all 5 bytes"
        );
        assert_ne!(
            CG_QUEST_CONFIRM_WIRE_SIZE, 5,
            "QuestConfirm carries an extra byte"
        );
    }

    // ---- golden bytes ----------------------------------------------------

    #[test]
    fn quest_confirm_places_the_byte_before_the_word() {
        let packet = sample_qc();
        assert_eq!(packet.encode(), golden_qc());
        let swapped = vec![0x1f, 0x04, 0x03, 0x02, 0x01, 0x5a];
        assert_ne!(
            packet.encode(),
            swapped,
            "packet.h:766-768 declares answer then requestPID; the reverse is a real risk"
        );
    }

    #[test]
    fn quest_confirm_decodes_answer_and_pid_at_the_declared_offsets() {
        let got = CgQuestConfirm::decode(&golden_qc()).unwrap();
        assert_eq!(got.answer, 0x5a);
        assert_eq!(got.request_pid, 0x0102_0304);
    }

    #[test]
    fn target_is_a_five_byte_record_with_one_little_endian_word() {
        assert_eq!(sample_tg().encode(), golden_tg());
        let got = CgTarget::decode(&golden_tg()).unwrap();
        assert_eq!(got.vid, 0x1122_3344);
    }

    #[test]
    fn script_button_is_a_five_byte_record_with_one_little_endian_word() {
        assert_eq!(sample_sb().encode(), golden_sb());
        assert_eq!(
            CgScriptButton::decode(&golden_sb()).unwrap().idx,
            0xa1b2_c3d4
        );
    }

    #[test]
    fn script_select_item_is_a_five_byte_record_with_one_little_endian_word() {
        assert_eq!(sample_si().encode(), golden_si());
        assert_eq!(
            CgScriptSelectItem::decode(&golden_si()).unwrap().selection,
            0xdead_beef
        );
    }

    #[test]
    fn every_word_is_little_endian_so_the_low_byte_comes_first() {
        // The word does not start at the same offset in every record, so its
        // least significant byte does not land at the same index either. Naming
        // the offset explicitly is the only way this assertion stays honest.
        for (golden, word_at, value) in [
            (&golden_qc()[..], 2usize, 0x0102_0304_u32),
            (&golden_tg()[..], 1, 0x1122_3344),
            (&golden_sb()[..], 1, 0xa1b2_c3d4),
            (&golden_si()[..], 1, 0xdead_beef),
        ] {
            assert_eq!(
                &golden[word_at..word_at + 4],
                value.to_le_bytes(),
                "the word at offset {word_at} is little-endian, least significant byte first"
            );
        }
    }

    /// The word offset differs between the two layouts, and getting it wrong
    /// would still produce six or five bytes.
    #[test]
    fn the_word_offset_is_two_for_quest_confirm_and_one_for_the_others() {
        assert_eq!(
            CG_QUEST_CONFIRM_WIRE_SIZE - 4,
            2,
            "answer occupies offset 1"
        );
        assert_eq!(CG_TARGET_WIRE_SIZE - 4, 1);
        assert_eq!(CG_SCRIPT_BUTTON_WIRE_SIZE - 4, 1);
        assert_eq!(CG_SCRIPT_SELECT_ITEM_WIRE_SIZE - 4, 1);
    }

    // ---- headers ---------------------------------------------------------

    #[test]
    fn the_four_header_bytes_match_the_legacy_enumerators() {
        assert_eq!(CgQuestConfirm::header().value(), 0x1f);
        assert_eq!(CgTarget::header().value(), 0x3d);
        assert_eq!(CgScriptButton::header().value(), 0x42);
        assert_eq!(CgScriptSelectItem::header().value(), 0x72);
    }

    #[test]
    fn the_four_header_bytes_are_all_distinct() {
        let set = BTreeSet::from([
            CgQuestConfirm::header().value(),
            CgTarget::header().value(),
            CgScriptButton::header().value(),
            CgScriptSelectItem::header().value(),
        ]);
        assert_eq!(set.len(), 4, "31, 61, 66 and 114 are distinct");
    }

    #[test]
    fn every_other_header_byte_is_rejected_at_the_right_width() {
        for (header, wire) in [
            (0x1f_u8, CG_QUEST_CONFIRM_WIRE_SIZE),
            (0x3d, CG_TARGET_WIRE_SIZE),
            (0x42, CG_SCRIPT_BUTTON_WIRE_SIZE),
            (0x72, CG_SCRIPT_SELECT_ITEM_WIRE_SIZE),
        ] {
            for other in u8::MIN..=u8::MAX {
                if other == header {
                    continue;
                }
                let mut bytes = vec![other; wire];
                bytes[0] = other;
                let err = decode_for(header, &bytes).unwrap_err();
                assert_eq!(
                    err,
                    CgPickError::InvalidHeader {
                        expected: header,
                        actual: other
                    },
                    "header {other} must not be accepted where {header} is required"
                );
            }
        }
    }

    fn decode_for(header: u8, bytes: &[u8]) -> Result<u8, CgPickError> {
        match header {
            0x1f => CgQuestConfirm::decode(bytes).map(|_| 0x1f),
            0x3d => CgTarget::decode(bytes).map(|_| 0x3d),
            0x42 => CgScriptButton::decode(bytes).map(|_| 0x42),
            0x72 => CgScriptSelectItem::decode(bytes).map(|_| 0x72),
            other => panic!("no decoder for {other}"),
        }
    }

    // ---- lengths and error precedence ------------------------------------

    #[test]
    fn a_short_input_is_truncated_at_every_length() {
        for (wire, header) in [
            (CG_QUEST_CONFIRM_WIRE_SIZE, 0x1f_u8),
            (CG_TARGET_WIRE_SIZE, 0x3d),
            (CG_SCRIPT_BUTTON_WIRE_SIZE, 0x42),
            (CG_SCRIPT_SELECT_ITEM_WIRE_SIZE, 0x72),
        ] {
            for len in 0..wire {
                let bytes = vec![header; len];
                assert_eq!(
                    decode_for(header, &bytes).unwrap_err(),
                    CgPickError::Truncated {
                        needed: wire,
                        available: len
                    }
                );
            }
        }
    }

    #[test]
    fn a_long_input_is_a_length_mismatch() {
        for (wire, header) in [
            (CG_QUEST_CONFIRM_WIRE_SIZE, 0x1f_u8),
            (CG_TARGET_WIRE_SIZE, 0x3d),
            (CG_SCRIPT_BUTTON_WIRE_SIZE, 0x42),
            (CG_SCRIPT_SELECT_ITEM_WIRE_SIZE, 0x72),
        ] {
            for len in (wire + 1)..=(wire + 3) {
                let bytes = vec![header; len];
                assert_eq!(
                    decode_for(header, &bytes).unwrap_err(),
                    CgPickError::LengthMismatch {
                        expected: wire,
                        actual: len
                    }
                );
            }
        }
    }

    #[test]
    fn length_is_checked_before_the_header() {
        // A two-byte slice of zeroes is both short and wrongly headed, and the
        // length error is what surfaces.
        assert_eq!(
            CgTarget::decode(&[0x00, 0x00]).unwrap_err(),
            CgPickError::Truncated {
                needed: 5,
                available: 2
            }
        );
        // A six-byte slice of zeroes is long and wrongly headed, and the length
        // error is still what surfaces.
        assert_eq!(
            CgScriptButton::decode(&[0x00; 6]).unwrap_err(),
            CgPickError::LengthMismatch {
                expected: 5,
                actual: 6
            }
        );
    }

    #[test]
    fn the_same_slice_takes_different_variants_from_records_of_different_widths() {
        // This is the same fact cg_mark had to work around, and here it points
        // the other way: a 5-byte slice is complete for the three single-word
        // records and merely short for QuestConfirm.
        let slice = [0x1f, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(
            CgTarget::decode(&slice).unwrap_err(),
            CgPickError::InvalidHeader {
                expected: 0x3d,
                actual: 0x1f
            },
            "5 bytes is the full width of Target, so the header decides"
        );
        assert_eq!(
            CgQuestConfirm::decode(&slice).unwrap_err(),
            CgPickError::Truncated {
                needed: 6,
                available: 5
            },
            "5 bytes is short of QuestConfirm's 6, so the length decides"
        );
    }

    // ---- cross-decoding ---------------------------------------------------

    #[test]
    fn the_three_single_word_records_are_header_discriminated_not_width_discriminated() {
        // All three are 5 bytes, so a real record from one of them reaches the
        // others' decoders at the correct width and is rejected on the header
        // alone. This is what the cg_mark family cannot do, because its three
        // widths are all distinct.
        for (owner, foreign) in [
            (0x3d_u8, golden_tg()),
            (0x42, golden_sb()),
            (0x72, golden_si()),
        ] {
            for (header, wire) in [
                (0x3d_u8, CG_TARGET_WIRE_SIZE),
                (0x42, CG_SCRIPT_BUTTON_WIRE_SIZE),
                (0x72, CG_SCRIPT_SELECT_ITEM_WIRE_SIZE),
            ] {
                if header == owner {
                    continue;
                }
                assert_eq!(foreign.len(), wire);
                let err = decode_for(header, &foreign).unwrap_err();
                assert!(
                    matches!(err, CgPickError::InvalidHeader { .. }),
                    "a 5-byte foreign record must be rejected on its header, got {err:?}"
                );
            }
        }
    }

    #[test]
    fn quest_confirm_never_reaches_a_header_error_against_the_five_byte_records() {
        // Widths 6 and 5 differ, so length always decides first and
        // InvalidHeader can never appear in either direction.
        for (bytes, header) in [
            (golden_qc(), 0x1f_u8),
            (golden_tg(), 0x3d),
            (golden_sb(), 0x42),
            (golden_si(), 0x72),
        ] {
            if header == 0x1f {
                assert_eq!(
                    decode_for(0x3d, &bytes).unwrap_err(),
                    CgPickError::LengthMismatch {
                        expected: 5,
                        actual: 6
                    }
                );
            } else {
                assert_eq!(
                    decode_for(0x1f, &bytes).unwrap_err(),
                    CgPickError::Truncated {
                        needed: 6,
                        available: 5
                    }
                );
            }
        }
    }

    #[test]
    fn no_cross_decoding_ever_succeeds() {
        let table = all_samples();
        for (src_header, src_bytes) in &table {
            for (dst_header, _) in &table {
                if src_header == dst_header {
                    continue;
                }
                assert!(
                    decode_for(*dst_header, src_bytes).is_err(),
                    "a {src_header} record must not decode as {dst_header}"
                );
            }
        }
    }

    // ---- opacity ---------------------------------------------------------

    #[test]
    fn the_answer_byte_round_trips_all_256_values() {
        // The server collapses every non-zero answer to CONFIRM_YES, but that
        // is server policy; framing preserves all 256.
        for value in u8::MIN..=u8::MAX {
            let packet = CgQuestConfirm::new(value, 0);
            let got = CgQuestConfirm::decode(&packet.encode()).unwrap();
            assert_eq!(got.answer, value);
        }
    }

    #[test]
    fn every_word_round_trips_across_the_full_u32_domain_edges() {
        for value in [
            0x0000_0000,
            0x0000_0001,
            0x7fff_ffff,
            0x8000_0000,
            0xffff_fffe,
            0xffff_ffff,
        ] {
            assert_eq!(CgTarget::new(value).encode()[1..], value.to_le_bytes());
            assert_eq!(
                CgTarget::decode(&CgTarget::new(value).encode())
                    .unwrap()
                    .vid,
                value
            );
            assert_eq!(
                CgScriptButton::decode(&CgScriptButton::new(value).encode())
                    .unwrap()
                    .idx,
                value
            );
            assert_eq!(
                CgScriptSelectItem::decode(&CgScriptSelectItem::new(value).encode())
                    .unwrap()
                    .selection,
                value
            );
            assert_eq!(
                CgQuestConfirm::decode(&CgQuestConfirm::new(0, value).encode())
                    .unwrap()
                    .request_pid,
                value
            );
        }
    }

    #[test]
    fn script_button_applies_no_validation_to_its_index() {
        // input_main.cpp:2182 logs idx and :2185 confirms with no argument, so
        // there is no button count to check against. Every value must survive.
        for value in [0x0000_0000, 0x0000_0001, 0xffff_ffff, 0xdead_beef] {
            let packet = CgScriptButton::new(value);
            assert_eq!(packet.encode().len(), CG_SCRIPT_BUTTON_WIRE_SIZE);
            assert_eq!(CgScriptButton::decode(&packet.encode()).unwrap().idx, value);
        }
    }

    #[test]
    fn a_request_pid_that_names_another_character_still_decodes() {
        // The handler resolves request_pid with FindByPID and acts on a
        // different character. The codec must not "help" by rejecting a PID
        // that is not the sender; it has no way to know the sender.
        let packet = CgQuestConfirm::new(1, 42);
        assert_eq!(
            CgQuestConfirm::decode(&packet.encode())
                .unwrap()
                .request_pid,
            42
        );
    }

    #[test]
    fn the_default_value_of_each_record_round_trips() {
        for (bytes, ok) in [
            (CgQuestConfirm::default().encode(), true),
            (CgTarget::default().encode(), true),
            (CgScriptButton::default().encode(), true),
            (CgScriptSelectItem::default().encode(), true),
        ] {
            assert!(ok);
            assert!(decode_for(bytes[0], &bytes).is_ok());
        }
    }

    // ---- encode_into -----------------------------------------------------

    #[test]
    fn encode_into_appends_to_a_non_empty_buffer() {
        let mut out = vec![0xaa, 0xbb];
        sample_qc().encode_into(&mut out);
        assert_eq!(&out[..2], &[0xaa, 0xbb], "the prefix survives");
        assert_eq!(&out[2..], golden_qc());
    }

    #[test]
    fn encode_into_into_an_empty_buffer_equals_encode() {
        let mut qc = Vec::new();
        CgQuestConfirm::decode(&sample_qc().encode())
            .unwrap()
            .encode_into(&mut qc);
        assert_eq!(qc, golden_qc());

        let mut tg = Vec::new();
        CgTarget::decode(&sample_tg().encode())
            .unwrap()
            .encode_into(&mut tg);
        assert_eq!(tg, golden_tg());

        let mut sb = Vec::new();
        CgScriptButton::decode(&sample_sb().encode())
            .unwrap()
            .encode_into(&mut sb);
        assert_eq!(sb, golden_sb());

        let mut si = Vec::new();
        CgScriptSelectItem::decode(&sample_si().encode())
            .unwrap()
            .encode_into(&mut si);
        assert_eq!(si, golden_si());
    }

    #[test]
    fn encoding_three_records_in_a_row_concatenates_cleanly() {
        let mut out = Vec::new();
        sample_tg().encode_into(&mut out);
        sample_sb().encode_into(&mut out);
        sample_si().encode_into(&mut out);
        assert_eq!(out.len(), 15);
        assert_eq!(&out[..5], &golden_tg()[..]);
        assert_eq!(&out[5..10], &golden_sb()[..]);
        assert_eq!(&out[10..], &golden_si()[..]);
    }

    // ---- frames ----------------------------------------------------------

    #[test]
    fn each_to_frame_drops_exactly_the_header_byte() {
        for (packet, header, payload_size) in [
            (
                sample_qc().to_frame(),
                0x1f_u8,
                CG_QUEST_CONFIRM_PAYLOAD_SIZE,
            ),
            (sample_tg().to_frame(), 0x3d, CG_TARGET_PAYLOAD_SIZE),
            (sample_sb().to_frame(), 0x42, CG_SCRIPT_BUTTON_PAYLOAD_SIZE),
            (
                sample_si().to_frame(),
                0x72,
                CG_SCRIPT_SELECT_ITEM_PAYLOAD_SIZE,
            ),
        ] {
            assert_eq!(packet.header, header);
            assert_eq!(packet.payload.len(), payload_size);
        }
    }

    #[test]
    fn a_frame_and_a_record_agree_byte_for_byte() {
        for (record, frame, header) in [
            (golden_qc(), sample_qc().to_frame(), 0x1f_u8),
            (golden_tg(), sample_tg().to_frame(), 0x3d),
            (golden_sb(), sample_sb().to_frame(), 0x42),
            (golden_si(), sample_si().to_frame(), 0x72),
        ] {
            assert_eq!(frame.header, header);
            assert_eq!(frame.payload, record[1..]);
        }
    }

    #[test]
    fn every_frame_round_trips_through_its_own_decoder() {
        assert_eq!(
            CgQuestConfirm::decode_frame(&sample_qc().to_frame()).unwrap(),
            sample_qc()
        );
        assert_eq!(
            CgTarget::decode_frame(&sample_tg().to_frame()).unwrap(),
            sample_tg()
        );
        assert_eq!(
            CgScriptButton::decode_frame(&sample_sb().to_frame()).unwrap(),
            sample_sb()
        );
        assert_eq!(
            CgScriptSelectItem::decode_frame(&sample_si().to_frame()).unwrap(),
            sample_si()
        );
    }

    #[test]
    fn a_frame_of_the_wrong_header_is_rejected() {
        for (header, wire_payload) in [
            (0x1f_u8, CG_QUEST_CONFIRM_PAYLOAD_SIZE),
            (0x3d, CG_TARGET_PAYLOAD_SIZE),
            (0x42, CG_SCRIPT_BUTTON_PAYLOAD_SIZE),
            (0x72, CG_SCRIPT_SELECT_ITEM_PAYLOAD_SIZE),
        ] {
            let frame = ClientFrame::new(0xff, vec![0u8; wire_payload]);
            let err = frame_decode_for(header, &frame).unwrap_err();
            assert_eq!(
                err,
                CgPickError::InvalidHeader {
                    expected: header,
                    actual: 0xff
                }
            );
        }
    }

    fn frame_decode_for(header: u8, frame: &ClientFrame) -> Result<u8, CgPickError> {
        match header {
            0x1f => CgQuestConfirm::decode_frame(frame).map(|_| 0x1f),
            0x3d => CgTarget::decode_frame(frame).map(|_| 0x3d),
            0x42 => CgScriptButton::decode_frame(frame).map(|_| 0x42),
            0x72 => CgScriptSelectItem::decode_frame(frame).map(|_| 0x72),
            other => panic!("no decoder for {other}"),
        }
    }

    #[test]
    fn the_frame_payload_width_is_checked_before_the_frame_header() {
        // An empty payload is short for every one of the four.
        let frame = ClientFrame::new(0xff, Vec::new());
        assert_eq!(
            CgTarget::decode_frame(&frame).unwrap_err(),
            CgPickError::Truncated {
                needed: 4,
                available: 0
            }
        );
        // A five-byte payload is long for the 4-byte records.
        let frame = ClientFrame::new(0x3d, vec![0u8; 5]);
        assert_eq!(
            CgTarget::decode_frame(&frame).unwrap_err(),
            CgPickError::LengthMismatch {
                expected: 4,
                actual: 5
            }
        );
    }

    #[test]
    fn a_record_width_payload_is_rejected_by_the_frame_decoder() {
        // Handing the frame decoder the whole 5-byte record instead of its
        // 4-byte payload is the single most likely framing mistake here.
        let frame = ClientFrame::new(0x3d, golden_tg());
        assert_eq!(
            CgTarget::decode_frame(&frame).unwrap_err(),
            CgPickError::LengthMismatch {
                expected: 4,
                actual: 5
            }
        );
    }

    #[test]
    fn quest_confirms_frame_payload_keeps_the_byte_before_the_word() {
        let frame = sample_qc().to_frame();
        assert_eq!(frame.payload[0], 0x5a);
        assert_eq!(&frame.payload[1..], &0x0102_0304u32.to_le_bytes());
    }

    #[test]
    fn frames_of_the_three_five_byte_records_are_header_discriminated() {
        for (header, frame) in [
            (0x3d_u8, sample_tg().to_frame()),
            (0x42, sample_sb().to_frame()),
            (0x72, sample_si().to_frame()),
        ] {
            for other in [0x3d_u8, 0x42, 0x72] {
                if other == header {
                    continue;
                }
                let err = frame_decode_for(other, &frame).unwrap_err();
                assert!(
                    matches!(err, CgPickError::InvalidHeader { .. }),
                    "payloads are all 4 bytes, so only the header can separate them"
                );
            }
        }
    }

    // ---- error surface ---------------------------------------------------

    #[test]
    fn all_three_error_variants_are_distinguishable() {
        let truncated = CgTarget::decode(&[0x3d]).unwrap_err();
        let mismatch = CgTarget::decode(&[0x3d; 6]).unwrap_err();
        let header = CgTarget::decode(&[0x00; 5]).unwrap_err();
        assert_ne!(truncated, mismatch);
        assert_ne!(mismatch, header);
        assert_ne!(truncated, header);
    }

    #[test]
    fn the_error_display_names_the_actual_and_expected_values() {
        assert_eq!(
            CgPickError::Truncated {
                needed: 5,
                available: 2
            }
            .to_string(),
            "CG record needs 5 bytes, got 2"
        );
        assert_eq!(
            CgPickError::LengthMismatch {
                expected: 5,
                actual: 7
            }
            .to_string(),
            "CG record must be exactly 5 bytes, got 7"
        );
        assert_eq!(
            CgPickError::InvalidHeader {
                expected: 61,
                actual: 31
            }
            .to_string(),
            "CG header 31 is not 61"
        );
    }

    #[test]
    fn the_error_type_is_a_std_error() {
        let boxed: Box<dyn std::error::Error> = Box::new(CgTarget::decode(&[]).unwrap_err());
        assert!(boxed.to_string().contains("CG record"));
    }

    #[test]
    fn the_error_is_copy_and_eq() {
        let err = CgTarget::decode(&[]).unwrap_err();
        let copied = err;
        assert_eq!(err, copied);
    }

    // ---- exhaustive ------------------------------------------------------

    #[test]
    fn every_header_and_every_length_is_covered() {
        for (header, wire, payload) in [
            (
                0x1f_u8,
                CG_QUEST_CONFIRM_WIRE_SIZE,
                CG_QUEST_CONFIRM_PAYLOAD_SIZE,
            ),
            (0x3d, CG_TARGET_WIRE_SIZE, CG_TARGET_PAYLOAD_SIZE),
            (
                0x42,
                CG_SCRIPT_BUTTON_WIRE_SIZE,
                CG_SCRIPT_BUTTON_PAYLOAD_SIZE,
            ),
            (
                0x72,
                CG_SCRIPT_SELECT_ITEM_WIRE_SIZE,
                CG_SCRIPT_SELECT_ITEM_PAYLOAD_SIZE,
            ),
        ] {
            for len in 0..=wire + 2 {
                let bytes = vec![header; len];
                match decode_for(header, &bytes) {
                    Ok(_) => assert_eq!(len, wire, "{header} accepted {len} bytes"),
                    Err(CgPickError::Truncated { needed, available }) => {
                        assert_eq!((needed, available), (wire, len));
                    }
                    Err(CgPickError::LengthMismatch { expected, actual }) => {
                        assert_eq!((expected, actual), (wire, len));
                    }
                    Err(other) => panic!("unexpected {other:?} for {header} at {len}"),
                }
            }
            for len in 0..=payload + 2 {
                let frame = ClientFrame::new(header, vec![header; len]);
                match frame_decode_for(header, &frame) {
                    Ok(_) => assert_eq!(len, payload),
                    Err(CgPickError::Truncated { needed, available }) => {
                        assert_eq!((needed, available), (payload, len));
                    }
                    Err(CgPickError::LengthMismatch { expected, actual }) => {
                        assert_eq!((expected, actual), (payload, len));
                    }
                    Err(other) => panic!("unexpected {other:?}"),
                }
            }
        }
    }

    #[test]
    fn a_decode_is_a_pure_function_of_its_input() {
        for (golden, header) in [
            (golden_qc(), 0x1f_u8),
            (golden_tg(), 0x3d),
            (golden_sb(), 0x42),
            (golden_si(), 0x72),
        ] {
            let first = decode_for(header, &golden).unwrap();
            let second = decode_for(header, &golden).unwrap();
            assert_eq!(first, second);
        }
    }
}
