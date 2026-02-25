//! Explicit codec for the fixed legacy `TPacketCGItemPickup` record.
//!
//! `server/server/game/packet.h` sets `HEADER_CG_ITEM_PICKUP = 15` at `:24` and
//! declares `command_item_pickup` at `packet.h:658-662` as a `BYTE header`
//! followed by a single `DWORD vid`. The record sits under the
//! `#pragma pack(1)` that `packet.h:274` opens and does not close until
//! `:3540`; `grep -n '#pragma pack' packet.h` returns exactly those two lines,
//! so nothing relaxes the packing in between and the declaration at `:658-662`
//! is strictly inside it. The record is therefore exactly **five bytes** and
//! its framed payload **four**. Nothing in the declaration is conditional:
//! the neighbouring `TPacketCGEnvanter` at `:642-647` and `TPacketCGItemSell`
//! at `:649-656` each carry their own `#ifdef` guard, and the pickup struct
//! carries none, so the width is not profile-dependent.
//!
//! `DWORD` is four bytes on the server without depending on a build flag:
//! `server/server/libthecore/typedef.h:15` defines it as `unsigned int` inside
//! the `#ifndef __WIN32__` block opened at `:9`, and the Windows branch is the
//! SDK's own 32-bit type. Either way it is four bytes.
//!
//! The record is registered exactly once, at
//! `server/server/game/packet_info.cpp:124`, as
//! `Set(HEADER_CG_ITEM_PICKUP, sizeof(TPacketCGItemPickup), "ItemPickup")`
//! inside `CPacketInfoCG`'s constructor. The four-byte size there agrees with
//! the hand-derived width, and there is no second `Set` for this header and no
//! literal `Set(15` anywhere in either tree.
//!
//! # `vid` is a virtual item ID, not a position
//!
//! Unlike its siblings `TPacketCGItemDrop`, `TPacketCGItemDrop2`, and
//! `TPacketCGItemDestroy`, this record carries **no** `TItemPos`. Its single
//! field identifies a ground item by the virtual ID that
//! `ITEM_MANAGER::FindByVID` keys on, not by a window and a cell. The dispatch
//! at `server/server/game/input_main.cpp:3715-3718` routes it to
//! `CInputMain::ItemPickup`, declared at `input.h:92` and defined at
//! `input_main.cpp:1076-1081`.
//!
//! `CHARACTER::PickupItem` is declared `bool PickupItem(DWORD vid)` at
//! `server/server/game/char.h:1210` and defined at `char_item.cpp:7972-8161`.
//! Its parameter `dwVID` occurs **exactly once** in that whole 190-line
//! function, at `:7974`, in
//! `LPITEM item = ITEM_MANAGER::instance().FindByVID(dwVID);`. That callee is
//! `item_manager.cpp:664-672`, a `std::map::find` declared at
//! `item_manager.h:445` that returns null when the key is absent. There is no
//! `-1`, no `0`, no `INVALID_VID`, no range test, no signed comparison, and no
//! `(int)` cast anywhere on that path.
//!
//! The field is therefore modelled here as an opaque `u32`. It is **not**
//! reinterpreted as signed, and it is deliberately **not** validated: `0` and
//! `0xFFFF_FFFF` are ordinary wire values, this boundary does not decide
//! whether a virtual ID is live, nearby, or owned, and it does not reproduce
//! the server's `false` return for an unknown key.
//!
//! # The handler returns `void` and discards the result
//!
//! `CInputMain::ItemPickup`'s entire body is `if (ch) ch->PickupItem(...)`.
//! `PickupItem` returns `bool` and that value is **thrown away**, so a failed
//! pickup produces no error record and no reply of any kind. This codec
//! accordingly models no success or failure result. The `if (ch)` is also
//! redundant: `Analyze` at `input_main.cpp:3647-3652` already closes the phase
//! when `d->GetCharacter()` is null.
//!
//! The handler performs no length check of its own and is safe only because the
//! framer has already established the registered five bytes:
//! `server/server/game/input.cpp:83` reads `iPacketLen` from the CG map and
//! `:92-93` returns early while `m_iBufferLeft < iPacketLen`, which is the only
//! length gate. That is a property of the framer and the single registration,
//! not of this record, which is why [`CgItemPickup::decode`] checks the exact
//! length itself.
//!
//! The dispatch sits inside `CInputMain::Analyze` (`input_main.cpp:3643-4086`)
//! and leaves `iExtraLen` at zero, so no count-derived extension exists. Header
//! 15 is not a variable header and is not in the Rust variable set.
//!
//! # Header 15 is collision-free inside the CG direction
//!
//! Parsing the `server/server/game/packet.h` enumerator list programmatically,
//! with C auto-increment simulated rather than read by eye, gives exactly three
//! enumerators with the value 15: `HEADER_CG_ITEM_PICKUP` at `:24`,
//! `HEADER_GC_MAIN_CHARACTER_OLD` at `:115`, and
//! `HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX` at `:244`. The client enum at
//! `client/Client/UserInterface/Packet.h:8-241` has exactly two:
//! `HEADER_CG_ITEM_PICKUP` at `:24` and `HEADER_GC_MAIN_CHARACTER` at `:111`.
//!
//! Under the project's four-bucket taxonomy, buckets (a) live two-sizes,
//! (b) colliding but not two-sizes, and (c) width-equal semantic collision are
//! all **empty**: both trees spell the CG 15 record with the same struct tag, the
//! same members, and the same five-byte width, so no second CG record carries
//! 15 anywhere. Only bucket (d), nominally colliding, applies, and it does not
//! constrain this codec because every remaining 15 is a different direction
//! family with a different decoder:
//!
//! - GC 15 travels the other way and is **dead**. `HEADER_GC_MAIN_CHARACTER_OLD`
//!   has no struct, no send site, and no client consumer; a whole-tree search
//!   for that name returns only the enumerator itself. The client's own GC 15 is
//!   a different and far wider record, `TPacketGCMainCharacter` at
//!   `client/Client/UserInterface/Packet.h:1429-1437`, which is 45 bytes with
//!   `CHARACTER_NAME_MAX_LEN = 24`. The two trees disagree on the *name* of a
//!   value the server never sends.
//! - GG 15 is `SPacketGGGuildWarMapIndex`, a 13-byte record at
//!   `packet.h:421-427` with its own decoder, already covered by the Rust `gg`
//!   module.
//! - GD 15 is `HEADER_GD_EMPIRE_SELECT` at `common/tables.h:32`, in the
//!   `1-byte header + 4-byte handle + 4-byte length` DB-peer framing, which is
//!   not the CG framing at all.
//!
//! The standing rule that no header table may be shared between directions still
//! applies, but nothing here could mislead a decoder.
//!
//! # The two trees spell the typedef differently
//!
//! The server declares `TPacketCGItemPickup` at `packet.h:662` and the client
//! declares `TPacketCGItemPickUp`, with a capital `U`, at
//! `client/Client/UserInterface/Packet.h:547`. The struct tag
//! `command_item_pickup` is identical on both sides, and so is the member list,
//! `BYTE header; DWORD vid;`. The client is packed by `#pragma pack(push)` and
//! `#pragma pack(1)` opened at `client/Client/UserInterface/Packet.h:300-301`
//! and closed by `#pragma pack(pop)` at `:3906`, with the declaration at
//! `:543-547` strictly between.
//!
//! The server handler does not use the typedef at all: it casts to the tag, at
//! `input_main.cpp:1078`, `(struct command_item_pickup*) data`. The two names
//! must not be unified across the trees, but both may be cited. This is recorded
//! so a later name-based coverage census does not report the client declaration
//! as missing.
//!
//! # Three client defects are recorded here and are not reproduced
//!
//! The client send path can put values on this wire that the server has no
//! meaning for. None of them is a property of the record, and none of them may
//! be mirrored or compensated for in Rust.
//!
//! 1. **Signed-to-unsigned widening at a script entry point.** The Python wrapper
//!    at `client/Client/UserInterface/PythonNetworkStreamModule.cpp:927-936`
//!    declares `int vid` at `:929` and passes it to a `DWORD` parameter at
//!    `:934`, so a negative or above-`0x7FFF_FFFF` Python argument silently
//!    becomes a large unsigned wire value. The second entry point at
//!    `client/Client/UserInterface/PythonPlayerModule.cpp:841-849` has the same
//!    `int ivid` widening at `:843` into `SendClickItemPacket` at `:847`. Both
//!    discard the `bool` return.
//! 2. **Unchecked `PyLong_AsLong`.** `PyTuple_GetInteger` at
//!    `client/Client/ScriptLib/PythonUtils.cpp:127-139` stores the return value
//!    of `PyLong_AsLong` at `:137` and never checks it. A non-integer argument
//!    leaves `-1` in the out-parameter, still returns `true`, leaks the live
//!    Python exception, and the wrapper then transmits `0xFFFFFFFF`.
//! 3. **Neither wrapper has an arity switch.** Both accept any tuple of length
//!    at least one and never type-check the argument, which is strictly less
//!    safe than the ten wrappers in the same file that do have a
//!    `default: return Py_BuildException();` arm. Unlike the safebox wrappers
//!    recorded earlier, these two have no fall-through defect of that class,
//!    because they contain no `switch (PyTuple_Size(...))` at all.
//!
//! That `0xFFFFFFFF` is reachable from a script is a **client** defect, not a
//! server sentinel. The server's only reaction is a failed `std::map::find` and
//! a discarded `false`. It is the specific reason this codec must not grow a
//! "non-zero vid" or "vid must be a known ground item" rule: such a rule would
//! silently normalize a legal wire value in order to compensate for a bug that
//! lives on the other side of the socket.
//!
//! Whether any shipped script actually calls these two entry points is **not
//! determined**: no `.py` file in this tree calls either wrapper. Nor is anything
//! determined about an unmodded retail client, because the checked-in `client/`
//! tree is a modified derivative that adds enumerators the server enum lacks.
//! It remains a valid oracle for the shared subset, which includes header 15.

use crate::cg_inventory::{CgHeader, HEADER_CG_ITEM_PICKUP};
use crate::cg_wire::ClientFrame;

/// The complete on-wire size of a `TPacketCGItemPickup`, including the header.
pub const CG_ITEM_PICKUP_WIRE_SIZE: usize = 5;

/// The number of bytes after the one-byte header.
pub const CG_ITEM_PICKUP_PAYLOAD_SIZE: usize = 4;

/// Errors returned by the `TPacketCGItemPickup` codec.
///
/// The length is always checked before the header byte, and the header byte is
/// always checked before any payload byte is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CgItemPickupError {
    /// The input ended before the complete record was available.
    Truncated {
        /// The exact wire size that was required.
        needed: usize,
        /// The number of bytes that were actually available.
        available: usize,
    },
    /// The input carried more bytes than the fixed record can hold.
    LengthMismatch {
        /// The exact wire size the record requires.
        expected: usize,
        /// The number of bytes that were actually supplied.
        actual: usize,
    },
    /// The leading byte was not [`HEADER_CG_ITEM_PICKUP`].
    InvalidHeader {
        /// The header byte that was actually present.
        actual: u8,
    },
}

impl core::fmt::Display for CgItemPickupError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "truncated TPacketCGItemPickup: need {needed} bytes, got {available}"
                )
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "TPacketCGItemPickup must be exactly {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { actual } => {
                write!(f, "expected header {HEADER_CG_ITEM_PICKUP:?}, got {actual}")
            }
        }
    }
}

impl std::error::Error for CgItemPickupError {}

/// A request to pick up one ground item by its virtual ID.
///
/// The header is fixed and is never stored as a field: it is always emitted as
/// byte 15 and is accepted only as 15.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgItemPickup {
    /// The virtual item ID, opaque and unchecked.
    pub vid: u32,
}

impl CgItemPickup {
    /// Build a request for the given virtual item ID.
    #[must_use]
    pub const fn new(vid: u32) -> Self {
        Self { vid }
    }

    /// The one-byte header this record encodes.
    #[must_use]
    pub const fn header() -> CgHeader {
        HEADER_CG_ITEM_PICKUP
    }

    /// Encode this request as the exact five wire bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_ITEM_PICKUP_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode this request into an existing buffer.
    ///
    /// The output is exactly `[0x0f][vid LE]`, four little-endian bytes after
    /// the header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.extend_from_slice(&self.vid.to_le_bytes());
    }

    /// Project this request as a `ClientFrame` with an exact four-byte
    /// payload.
    #[must_use]
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_ITEM_PICKUP_PAYLOAD_SIZE);
        payload.extend_from_slice(&self.vid.to_le_bytes());
        ClientFrame::new(Self::header().value(), payload)
    }

    /// Decode one exact five-byte record.
    ///
    /// # Errors
    ///
    /// Returns [`CgItemPickupError::Truncated`] for a short input,
    /// [`CgItemPickupError::LengthMismatch`] for a long input, and
    /// [`CgItemPickupError::InvalidHeader`] when the header is not 15. The
    /// length is checked before the header, and the header is checked before
    /// the virtual ID is read.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgItemPickupError> {
        check_exact(bytes.len())?;
        check_header(bytes[0])?;
        Ok(Self {
            vid: u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]),
        })
    }

    /// Decode one framed request.
    ///
    /// # Errors
    ///
    /// Returns [`CgItemPickupError::Truncated`] for a payload shorter than four
    /// bytes, [`CgItemPickupError::LengthMismatch`] for a longer payload, and
    /// [`CgItemPickupError::InvalidHeader`] when the frame header is not 15.
    /// The payload length is checked before the header, and the header is
    /// checked before any payload byte is read.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgItemPickupError> {
        let len = frame.payload.len();
        match len.cmp(&CG_ITEM_PICKUP_PAYLOAD_SIZE) {
            core::cmp::Ordering::Less => {
                return Err(CgItemPickupError::Truncated {
                    needed: CG_ITEM_PICKUP_PAYLOAD_SIZE,
                    available: len,
                })
            }
            core::cmp::Ordering::Greater => {
                return Err(CgItemPickupError::LengthMismatch {
                    expected: CG_ITEM_PICKUP_PAYLOAD_SIZE,
                    actual: len,
                })
            }
            core::cmp::Ordering::Equal => {}
        }
        check_header(frame.header)?;
        Ok(Self {
            vid: u32::from_le_bytes([
                frame.payload[0],
                frame.payload[1],
                frame.payload[2],
                frame.payload[3],
            ]),
        })
    }
}

fn check_exact(len: usize) -> Result<(), CgItemPickupError> {
    if len < CG_ITEM_PICKUP_WIRE_SIZE {
        return Err(CgItemPickupError::Truncated {
            needed: CG_ITEM_PICKUP_WIRE_SIZE,
            available: len,
        });
    }
    if len > CG_ITEM_PICKUP_WIRE_SIZE {
        return Err(CgItemPickupError::LengthMismatch {
            expected: CG_ITEM_PICKUP_WIRE_SIZE,
            actual: len,
        });
    }
    Ok(())
}

fn check_header(actual: u8) -> Result<(), CgItemPickupError> {
    if actual != HEADER_CG_ITEM_PICKUP.value() {
        return Err(CgItemPickupError::InvalidHeader { actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CgItemPickup, CgItemPickupError, CG_ITEM_PICKUP_PAYLOAD_SIZE, CG_ITEM_PICKUP_WIRE_SIZE,
    };
    use crate::cg_inventory::HEADER_CG_ITEM_PICKUP;
    use crate::cg_wire::ClientFrame;

    /// The five wire bytes the legacy server would receive for `vid`.
    fn wire(vid: u32) -> Vec<u8> {
        let mut v = vec![15u8];
        v.extend_from_slice(&vid.to_le_bytes());
        v
    }

    #[test]
    fn constants_match_the_source() {
        assert_eq!(HEADER_CG_ITEM_PICKUP.value(), 15);
        assert_eq!(CgItemPickup::header().value(), 15);
        // packet.h:658-662 -> BYTE header + DWORD vid.
        assert_eq!(CG_ITEM_PICKUP_WIRE_SIZE, 5);
        assert_eq!(CG_ITEM_PICKUP_PAYLOAD_SIZE, 4);
        assert_eq!(CG_ITEM_PICKUP_WIRE_SIZE, 1 + CG_ITEM_PICKUP_PAYLOAD_SIZE);
    }

    #[test]
    fn golden_bytes_for_a_known_virtual_id() {
        assert_eq!(wire(0x0102_0304), vec![15, 0x04, 0x03, 0x02, 0x01]);
        assert_eq!(CgItemPickup::new(0x0102_0304).encode(), wire(0x0102_0304));
        assert_eq!(CgItemPickup::new(1).encode(), vec![15, 1, 0, 0, 0]);
    }

    /// The virtual ID is a `DWORD` on both sides, so every `u32` value must
    /// round-trip and no value is reserved.
    #[test]
    fn every_boundary_virtual_id_round_trips() {
        let cases = [
            0u32,
            1,
            2,
            0x7fff_ffff,
            0x8000_0000,
            0xffff_fffe,
            0xffff_ffff,
        ];
        for vid in cases {
            let rec = CgItemPickup::new(vid);
            let bytes = rec.encode();
            assert_eq!(bytes.len(), CG_ITEM_PICKUP_WIRE_SIZE, "vid {vid:#x}");
            assert_eq!(bytes[0], 15, "vid {vid:#x}");
            assert_eq!(
                CgItemPickup::decode(&bytes).expect("decodes"),
                rec,
                "vid {vid:#x}"
            );
            let frame = rec.to_frame();
            assert_eq!(frame.payload.len(), CG_ITEM_PICKUP_PAYLOAD_SIZE);
            assert_eq!(
                CgItemPickup::decode_frame(&frame).expect("frame decodes"),
                rec,
                "vid {vid:#x}"
            );
        }
    }

    #[test]
    fn a_dense_sweep_of_virtual_ids_round_trips() {
        for vid in 0u32..4096 {
            let rec = CgItemPickup::new(vid.wrapping_mul(0x0100_0101));
            assert_eq!(CgItemPickup::decode(&rec.encode()).expect("decodes"), rec);
        }
    }

    /// The field is little-endian. A big-endian read would make these two
    /// inputs identical, so pinning both directions pins the byte order.
    #[test]
    fn the_virtual_id_is_read_little_endian() {
        let lo = CgItemPickup::new(0x0000_00ff);
        let hi = CgItemPickup::new(0xff00_0000);
        assert_eq!(lo.encode(), vec![15, 0xff, 0x00, 0x00, 0x00]);
        assert_eq!(hi.encode(), vec![15, 0x00, 0x00, 0x00, 0xff]);
        assert_eq!(CgItemPickup::decode(&lo.encode()).expect("lo").vid, 0xff);
        assert_eq!(
            CgItemPickup::decode(&hi.encode()).expect("hi").vid,
            0xff00_0000
        );
    }

    /// The exact length is checked before the header byte, so a short input
    /// whose first byte is a valid header is still a truncation, and a short
    /// input whose first byte is not a valid header is *also* a truncation.
    #[test]
    fn length_is_checked_before_the_header() {
        for first in [0x00u8, 0x0f, 0xff] {
            for len in 0..CG_ITEM_PICKUP_WIRE_SIZE {
                let mut buf = vec![first];
                buf.resize(len, 0xaa);
                assert_eq!(
                    CgItemPickup::decode(&buf),
                    Err(CgItemPickupError::Truncated {
                        needed: CG_ITEM_PICKUP_WIRE_SIZE,
                        available: len,
                    }),
                    "first {first:#x} len {len}"
                );
            }
        }
    }

    #[test]
    fn a_long_input_is_a_length_mismatch() {
        for len in (CG_ITEM_PICKUP_WIRE_SIZE + 1)..(CG_ITEM_PICKUP_WIRE_SIZE + 8) {
            let mut buf = wire(7);
            buf.resize(len, 0xbb);
            assert_eq!(
                CgItemPickup::decode(&buf),
                Err(CgItemPickupError::LengthMismatch {
                    expected: CG_ITEM_PICKUP_WIRE_SIZE,
                    actual: len,
                }),
                "len {len}"
            );
        }
    }

    #[test]
    fn a_wrong_header_is_rejected_only_after_an_exact_length() {
        for bad in [0x00u8, 0x0e, 0x10, 0x14, 0xff] {
            let mut buf = wire(0);
            buf[0] = bad;
            assert_eq!(
                CgItemPickup::decode(&buf),
                Err(CgItemPickupError::InvalidHeader { actual: bad }),
                "header {bad:#x}"
            );
        }
    }

    /// Every possible leading byte is classified: exactly one is accepted.
    #[test]
    fn exactly_one_leading_byte_is_accepted() {
        let mut accepted = 0;
        for first in 0u8..=255 {
            let mut buf = vec![first];
            buf.extend_from_slice(&0x1122_3344u32.to_le_bytes());
            match CgItemPickup::decode(&buf) {
                Ok(rec) => {
                    assert_eq!(first, 15);
                    assert_eq!(rec.vid, 0x1122_3344);
                    accepted += 1;
                }
                Err(CgItemPickupError::InvalidHeader { actual }) => assert_eq!(actual, first),
                Err(other) => panic!("header {first:#x} gave {other:?}"),
            }
        }
        assert_eq!(accepted, 1);
    }

    #[test]
    fn the_framed_payload_length_is_checked_before_the_frame_header() {
        // Wrong payload length AND wrong header: the length must win.
        let frame = ClientFrame::new(0xff, vec![1, 2, 3]);
        assert_eq!(
            CgItemPickup::decode_frame(&frame),
            Err(CgItemPickupError::Truncated {
                needed: CG_ITEM_PICKUP_PAYLOAD_SIZE,
                available: 3,
            })
        );
        let frame = ClientFrame::new(0xff, vec![1, 2, 3, 4, 5]);
        assert_eq!(
            CgItemPickup::decode_frame(&frame),
            Err(CgItemPickupError::LengthMismatch {
                expected: CG_ITEM_PICKUP_PAYLOAD_SIZE,
                actual: 5,
            })
        );
        // Exact payload length, wrong header: now the header is reported.
        let frame = ClientFrame::new(0xff, vec![1, 2, 3, 4]);
        assert_eq!(
            CgItemPickup::decode_frame(&frame),
            Err(CgItemPickupError::InvalidHeader { actual: 0xff })
        );
        // Empty payload is a truncation, not a header error.
        let frame = ClientFrame::new(0xff, Vec::new());
        assert_eq!(
            CgItemPickup::decode_frame(&frame),
            Err(CgItemPickupError::Truncated {
                needed: CG_ITEM_PICKUP_PAYLOAD_SIZE,
                available: 0,
            })
        );
    }

    /// The header is never stored as a field, so every instance emits 15 and
    /// no instance can be built that would emit anything else.
    #[test]
    fn the_header_is_fixed_and_never_stored() {
        for vid in [0u32, 1, 0xdead_beef, u32::MAX] {
            assert_eq!(CgItemPickup::new(vid).encode()[0], 15);
            assert_eq!(CgItemPickup::new(vid).to_frame().header, 15);
        }
        // A record decoded from a valid frame re-encodes to the same bytes.
        let rec = CgItemPickup::new(0x0102_0304);
        let frame = rec.to_frame();
        assert_eq!(
            CgItemPickup::decode_frame(&frame).expect("d").encode(),
            rec.encode()
        );
    }

    #[test]
    fn encode_into_appends_rather_than_replaces() {
        let mut out = vec![0xde, 0xad];
        CgItemPickup::new(0x0102_0304).encode_into(&mut out);
        assert_eq!(out, vec![0xde, 0xad, 15, 0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn errors_display_without_panicking() {
        let cases = [
            CgItemPickupError::Truncated {
                needed: 5,
                available: 2,
            },
            CgItemPickupError::LengthMismatch {
                expected: 5,
                actual: 9,
            },
            CgItemPickupError::InvalidHeader { actual: 0x14 },
        ];
        for e in cases {
            let s = e.to_string();
            assert!(!s.is_empty());
            assert!(s.contains("TPacketCGItemPickup") || s.contains("header"));
            let dyn_err: &dyn std::error::Error = &e;
            assert!(!dyn_err.to_string().is_empty());
        }
    }

    /// The record is registered at five bytes, so it must decode through the
    /// ordinary fixed-frame decoder without any special case.
    #[test]
    fn it_decodes_through_the_fixed_frame_decoder() {
        use crate::cg_wire::ClientFrameDecoder;
        let mut stream = Vec::new();
        for vid in [0u32, 1, 0x1122_3344, u32::MAX] {
            stream.extend_from_slice(&wire(vid));
        }
        let mut dec = ClientFrameDecoder::new();
        dec.feed(&stream).expect("feed");
        let mut got = Vec::new();
        while let Some(frame) = dec.try_decode().expect("decode") {
            assert_eq!(frame.header, 15);
            got.push(CgItemPickup::decode_frame(&frame).expect("codec").vid);
        }
        assert_eq!(got, vec![0, 1, 0x1122_3344, u32::MAX]);
        assert!(dec.is_empty());
    }

    /// The record carries no `TItemPos`, so it must not expose one.
    #[test]
    fn the_record_exposes_only_the_virtual_id() {
        let rec = CgItemPickup::new(9);
        let frame = rec.to_frame();
        assert_eq!(frame.payload.len(), 4, "no window or cell bytes");
        assert_eq!(rec.encode().len(), 5, "no window or cell bytes");
    }

    /// The audit found that `dwVID` reaches exactly one place on the server: a
    /// `std::map::find` in `item_manager.cpp:664-672`. No sentinel, no range
    /// test, no signed comparison. Both edges of the domain must therefore
    /// round-trip, and neither may be special-cased away.
    #[test]
    fn no_virtual_id_value_is_reserved() {
        // 0 is the "absent ground item" a caller might naively guess about.
        assert_eq!(
            CgItemPickup::decode(&CgItemPickup::new(0).encode())
                .expect("d")
                .vid,
            0
        );
        // 0xFFFF_FFFF is exactly what the client's unchecked signed Python
        // wrapper transmits. It is a legal wire value here, and the server just
        // fails its map lookup.
        let all_ones = CgItemPickup::new(u32::MAX);
        assert_eq!(all_ones.encode(), vec![15, 0xff, 0xff, 0xff, 0xff]);
        assert_eq!(
            CgItemPickup::decode(&all_ones.encode()).expect("d").vid,
            u32::MAX
        );
        // Exhaustive over the low byte, so a 256-wide sweep at 0 is cheap.
        for lo in 0u32..=255 {
            assert_eq!(
                CgItemPickup::decode(&CgItemPickup::new(lo).encode())
                    .expect("d")
                    .vid,
                lo
            );
        }
    }

    /// The record must not grow a header field or a validation rule. The struct
    /// has exactly one field, and it is the opaque identifier.
    #[test]
    fn the_record_has_exactly_one_field() {
        let rec = CgItemPickup::new(0x0102_0304);
        // Destructure exhaustively: a second field would fail to compile here.
        let CgItemPickup { vid } = rec;
        assert_eq!(vid, 0x0102_0304);
        // `Default` yields the zero identifier rather than panicking, because
        // there is no field whose value would be invalid to default.
        assert_eq!(CgItemPickup::default(), CgItemPickup::new(0));
    }

    /// `PickupItem` returns `bool` and the legacy handler discards it, so this
    /// boundary models no success or failure. A failed pickup produces no reply,
    /// which is what an absent reply here records.
    #[test]
    fn a_pickup_attempt_has_no_reply_record() {
        for vid in [0u32, 1, u32::MAX] {
            let rec = CgItemPickup::new(vid);
            let frame = rec.to_frame();
            // The only thing a decode can ever produce is the record itself.
            assert_eq!(CgItemPickup::decode_frame(&frame).expect("d"), rec);
        }
    }
}
