//! Transport-free codec for the legacy safebox and mall item-transfer records.
//!
//! The server declares **two** packed record types and registers **three**
//! inbound client headers against them, so this module covers all three wire
//! identities at once. The client declares **three** types for the same three
//! headers, adding a separate `TPacketCGMallCheckout` whose container field is
//! named `bMallPos` at `client/Client/UserInterface/Packet.h:811-820`. All
//! three client types are 8 bytes packed, so the wire is unaffected:
//!
//! | Header | Value | Legacy server type | Legacy direction |
//! | --- | --- | --- | --- |
//! | [`HEADER_CG_MALL_CHECKOUT`] | `0x45` (69) | `TPacketCGSafeboxCheckout` | item leaves the mall for the inventory |
//! | [`HEADER_CG_SAFEBOX_CHECKIN`] | `0x46` (70) | `TPacketCGSafeboxCheckin` | item leaves the inventory for the safebox |
//! | [`HEADER_CG_SAFEBOX_CHECKOUT`] | `0x47` (71) | `TPacketCGSafeboxCheckout` | item leaves the safebox for the inventory |
//!
//! The three payloads are byte-identical. The only difference between the
//! mall and safebox checkout frames is byte 0, and only header 70 carries the
//! opposite transfer direction. Nothing in the payload says which container
//! or which direction is meant, so **this record stores a header byte**, as a
//! closed [`SafeBoxKind`] enum whose three values round-trip verbatim.
//!
//! That is not a new idea in this series. The standing rule is to omit a
//! header field when a record has exactly one encodable value, because a
//! second copy in memory could only create a state the encoder cannot emit.
//! Two records break that rule on purpose, and both do so because the legacy
//! record really does have more than one legal header: the inbound handshake,
//! which accepts `0xff` and `0xfc` and stores a [`crate::cg_handshake::CgHandshakeHeader`], and
//! this one, which is the first with **three** legal values.
//!
//! # Wire layout
//!
//! Active build profile, eight bytes, no padding:
//!
//! ```text
//! offset 0      header        0x45, 0x46 or 0x47
//! offset 1..5   container_pos explicit little-endian u32
//! offset 5      window_type   opaque byte
//! offset 6..8   cell          explicit little-endian u16
//! ```
//!
//! The position is the already source-verified [`crate::cg_item_move::CgItemPos`] from the
//! item-move record, reused so the item records that carry a packed
//! three-byte position cannot drift apart. The legacy source embeds a packed
//! `TItemPos` in exactly **twelve** `TPacketCG*` records, and these two
//! safebox types are the tenth and eleventh of them in declaration order at
//! `packet.h:1979` and `:1990`. This module is the fifth Rust codec to reuse
//! the shared `CgItemPos`, after item-move, item-use, item-use-to-item, and
//! item-give. It is also the first of the twelve whose packed width is
//! *profile-dependent* rather than fixed.
//!
//! # The feature profile is a compile-time constant, not a run-time one
//!
//! The legacy declaration is:
//!
//! ```text
//! typedef struct command_safebox_checkout
//! {
//!     BYTE bHeader;
//! #if defined(__EXTENDED_SAFEBOX__)
//!     DWORD bSafePos;
//! #else
//!     BYTE bSafePos;
//! #endif
//!     TItemPos ItemPos;
//! } TPacketCGSafeboxCheckout;
//! ```
//!
//! so the record is **8 bytes** with the flag and **5 bytes** without it. The
//! flag has exactly one definition site per tree, at
//! `server/server/common/prodomodefines.h:61` and
//! `client/Client/UserInterface/LOCALE_INC.H:35`, both commented
//! `__GF_Safebox__`, and neither tree contains an `#undef` or a build-system
//! override. The two trees therefore agree on 8 bytes today, and nothing
//! enforces that agreement. Unlike the DB boot feature profile, this one
//! cannot be selected at run time: producing the 5-byte shape would require
//! editing the `#define` itself, which is a source change. This codec
//! therefore implements the active 8-byte profile and does not model the
//! 5-byte shape, because the legacy source cannot produce it.
//!
//! # All three values collide across packet directions
//!
//! `HEADER_GC_ADD_FLY_TARGETING` is 69, `HEADER_GC_CREATE_FLY` is 70, and
//! `HEADER_GC_FLY_TARGETING` is 71, at `server/server/game/packet.h:157-159`
//! and client `Packet.h:153-155`. The client registers those three inbound as
//! fixed records of 10 and 17 bytes at
//! `client/Client/UserInterface/PythonNetworkStream.cpp:100-102` and dispatches
//! them at `PythonNetworkStreamPhaseGame.cpp:444`, `:448`, and `:452`. A
//! header value alone therefore never identifies a record, and no table of
//! header values may be shared between the two directions.
//!
//! # Legacy facts deliberately kept out of this codec
//!
//! * The container index is a **flat cell index** into a grid five columns
//!   wide, scaled by a page count that the server sends separately. It is not
//!   a page index and not a window index, and nothing on the wire encodes a
//!   page. `bSafePos` on the server is read at
//!   `server/server/game/input_main.cpp:2310`, `:2385`, and `:2427` as a cell.
//! * The same `window_type` byte is a **source** cell for header 70 and a
//!   **destination** cell for headers 69 and 71. The record cannot be typed as
//!   "move to" or "move from" without adding a role the wire does not carry.
//! * The legacy server never range-checks `window_type` against an allowlist.
//!   It is constrained only indirectly, because `CHARACTER::IsValidItemPosition`
//!   at `char_item.cpp:10010-10045` and `CHARACTER::IsEmptyItemGrid` at
//!   `char_item.cpp:737-1036` both fail closed on an unknown window. This codec
//!   therefore keeps `window_type` **opaque** and must not validate it. In
//!   particular the struct-level `SItemPos::IsValidItemPosition` returns
//!   `false` for `SAFEBOX` and `MALL` at `length.h:985-987`, so borrowing that
//!   predicate would reject every legitimate use of this record.
//! * `ItemPos.cell` is a `WORD` at `length.h:960` narrowed to the `BYTE
//!   bOldPos` parameter of `SyncQuickslot` at `char.h:1035` when it is passed
//!   at `input_main.cpp:2360`, so for a cell at or above 256 the quickslot that
//!   is deleted is not the one the client named. The container index
//!   `bSafePos` is a `DWORD` and never reaches that call. That is a
//!   caller-layer defect and is not reproduced here.
//! * Neither handler consults the protected-inventory state: a full-tree grep
//!   finds no `IsSecured()` anywhere in `input_main.cpp`, although the house
//!   pattern enforces it at `char_item.cpp:7430` and a dozen other sites.
//! * Neither handler rejects an equipped item. `SafeboxCheckin` reads its
//!   source with no window restriction at `input_main.cpp:2287`, and
//!   `CItem::RemoveFromCharacter` at `item.cpp:369-377` unequips as a side
//!   effect, so a header-70 frame with an equipment cell unequips and stores
//!   the worn item.
//! * `SafeboxCheckin` omits the `IsExchanging()`, `GetCount() <= 0`, and
//!   `IsSecured()` checks that `CHARACTER::DestroyItem` applies at
//!   `char_item.cpp:7420`, `:7426`, and `:7430`. The `IsExchanging` concept
//!   does exist elsewhere in the same class, in `CSafebox::MoveItem` at
//!   `safebox.cpp:194-195`, but that is a different method and is never
//!   reached from either wire handler; `CSafebox::Add` at
//!   `safebox.cpp:56-91` has no exchange check at all, and `safebox.cpp`
//!   contains no `IsSecured` check anywhere.
//! * Both handlers detach an item from its container **before** the operation
//!   that must re-home it, and discard that operation's `bool`: the checkin at
//!   `input_main.cpp:2358-2361` and the two checkout paths at `:2427-2428` and
//!   `:2445-2446`.
//! * The mall branch mutates a Dragon-Soul item **before** validating the
//!   transfer, because `DragonSoulItemInitialize` is called at
//!   `input_main.cpp:2406` ahead of the window check at `:2409-2413`, and its
//!   `bool` is discarded. The safebox branch never does this.
//! * All three client builders write every byte, so a wrong argument count
//!   leaks **no** indeterminate byte from them. The leak is in the Python
//!   wrappers: `netSendSafeboxCheckoutPacket` has `default: break;` at
//!   `client/Client/UserInterface/PythonNetworkStreamModule.cpp:1376-1377`
//!   and `netSendSafeboxCheckinPacket` has **no default label at all**, falling
//!   out of its switch at `:1347`, so both transmit an uninitialised `int` as
//!   the 4-byte container index. The mall wrapper is the exception, returning
//!   before its send at `:1425-1426`. That is the strongest reason the index
//!   must stay an unvalidated `u32` here.
//! * None of the three client builders calls `__CanActMainInstance()`, so a
//!   script can emit this record while dead, loading, trading, or shopping.
//! * The two safebox script wrappers take the same two arguments in opposite
//!   order: the checkin switch reads the cell at `:1333` and the slot at
//!   `:1335`, while the checkout switch reads the slot at `:1363` and the cell
//!   at `:1365`. A script calling both with one argument list transposes them,
//!   and this codec does not normalise that.

use crate::cg_inventory::{
    CgHeader, HEADER_CG_MALL_CHECKOUT, HEADER_CG_SAFEBOX_CHECKIN, HEADER_CG_SAFEBOX_CHECKOUT,
};
use crate::cg_wire::ClientFrame;
use std::fmt;

/// The exact wire size of one packed legacy `TItemPos`.
pub use crate::cg_item_move::CG_ITEM_POS_SIZE;

/// The exact payload size of a framed safebox or mall transfer request.
pub const CG_SAFEBOX_PAYLOAD_SIZE: usize = 7;

/// The exact wire size of a complete safebox or mall transfer request.
pub const CG_SAFEBOX_WIRE_SIZE: usize = 8;

/// The exact wire size of the record when `__EXTENDED_SAFEBOX__` is not
/// defined.
///
/// The active tree always defines the flag, so this shape is unreachable and
/// is recorded only to make the profile dependency explicit. No function in
/// this module produces or accepts it.
pub const CG_SAFEBOX_LEGACY_WIRE_SIZE: usize = 5;

/// Which of the three legacy wire identities a record carries.
///
/// The payloads are identical, so this is the only thing that distinguishes
/// them on the wire, and it is preserved exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SafeBoxKind {
    /// Header 69: take an item out of the mall into the inventory.
    MallCheckout,
    /// Header 70: put an item from the inventory into the safebox.
    SafeboxCheckin,
    /// Header 71: take an item out of the safebox into the inventory.
    SafeboxCheckout,
}

impl SafeBoxKind {
    /// The exact one-byte header this identity uses.
    #[must_use]
    pub const fn header(self) -> CgHeader {
        match self {
            Self::MallCheckout => HEADER_CG_MALL_CHECKOUT,
            Self::SafeboxCheckin => HEADER_CG_SAFEBOX_CHECKIN,
            Self::SafeboxCheckout => HEADER_CG_SAFEBOX_CHECKOUT,
        }
    }

    /// Every identity, in ascending header order.
    pub const ALL: [Self; 3] = [
        Self::MallCheckout,
        Self::SafeboxCheckin,
        Self::SafeboxCheckout,
    ];

    /// Map a header byte back to the identity that uses it.
    #[must_use]
    pub const fn from_header(byte: u8) -> Option<Self> {
        match byte {
            0x45 => Some(Self::MallCheckout),
            0x46 => Some(Self::SafeboxCheckin),
            0x47 => Some(Self::SafeboxCheckout),
            _ => None,
        }
    }

    /// Whether this identity moves an item **into** a container rather than
    /// out of one.
    ///
    /// This is a source fact about the legacy dispatch, not a policy: only
    /// [`Self::SafeboxCheckin`] reaches `CInputMain::SafeboxCheckin`, which
    /// calls `pkSafebox->Add` at `input_main.cpp:2361`. The other two reach
    /// `CInputMain::SafeboxCheckout`, which calls `pkSafebox->Remove` at
    /// `:2427` and `:2445`.
    #[must_use]
    pub const fn is_deposit(self) -> bool {
        matches!(self, Self::SafeboxCheckin)
    }
}

impl From<SafeBoxKind> for CgHeader {
    fn from(kind: SafeBoxKind) -> Self {
        kind.header()
    }
}

impl fmt::Display for SafeBoxKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::MallCheckout => "mall checkout",
            Self::SafeboxCheckin => "safebox check-in",
            Self::SafeboxCheckout => "safebox check-out",
        };
        write!(f, "{name} (header {})", self.header().value())
    }
}

/// One fixed legacy safebox or mall item-transfer request.
///
/// The three legacy record types collapse to one layout here, but the header
/// is kept because the three wire identities differ in byte 0 alone and
/// carry different transfer directions. This follows the same rule as
/// [`CgInboundHandshake`](crate::cg_handshake::CgInboundHandshake), whose
/// stored [`crate::cg_handshake::CgHandshakeHeader`] exists for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgSafeBoxItem {
    /// Which of the three wire identities this record is.
    pub kind: SafeBoxKind,
    /// The raw container cell index, an explicit little-endian `u32`.
    ///
    /// Preserved verbatim. It is a flat cell index into a five-column grid,
    /// not a page or a window, and the legacy client can put an
    /// uninitialised `int` here on a wrong argument count.
    pub container_pos: u32,
    /// The item position: a source cell for the check-in identity and a
    /// destination cell for the two checkout identities.
    pub item_pos: crate::cg_item_move::CgItemPos,
}

impl CgSafeBoxItem {
    /// Build a record without interpreting any field.
    #[must_use]
    pub const fn new(
        kind: SafeBoxKind,
        container_pos: u32,
        item_pos: crate::cg_item_move::CgItemPos,
    ) -> Self {
        Self {
            kind,
            container_pos,
            item_pos,
        }
    }

    /// The one-byte header this record encodes.
    #[must_use]
    pub const fn header(&self) -> CgHeader {
        self.kind.header()
    }

    /// Encode this record as the exact eight wire bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_SAFEBOX_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode this record into an existing buffer.
    ///
    /// The buffer must already have room for [`CG_SAFEBOX_WIRE_SIZE`] bytes.
    /// The output is
    /// `[header][container_pos little-endian u32][window_type][cell little-endian u16]`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.header().value());
        out.extend_from_slice(&self.container_pos.to_le_bytes());
        self.item_pos.encode_into(out);
    }

    /// Project this record as a `ClientFrame` with an exact seven-byte
    /// payload.
    #[must_use]
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_SAFEBOX_PAYLOAD_SIZE);
        payload.extend_from_slice(&self.container_pos.to_le_bytes());
        self.item_pos.encode_into(&mut payload);
        ClientFrame::new(self.header().value(), payload)
    }

    /// Decode one exact eight-byte record.
    ///
    /// # Errors
    ///
    /// Returns [`CgSafeBoxError::Truncated`] for a short input,
    /// [`CgSafeBoxError::LengthMismatch`] for a long input, and
    /// [`CgSafeBoxError::InvalidHeader`] when the header is not 69, 70, or 71.
    /// The length is checked before the header, and the header is checked
    /// before any field is read.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgSafeBoxError> {
        check_exact(bytes.len())?;
        let kind =
            SafeBoxKind::from_header(bytes[0]).ok_or(CgSafeBoxError::InvalidHeader(bytes[0]))?;
        let mut pos = [0u8; 4];
        pos.copy_from_slice(&bytes[1..5]);
        Ok(Self {
            kind,
            container_pos: u32::from_le_bytes(pos),
            item_pos: crate::cg_item_move::CgItemPos::decode_at(bytes, 5),
        })
    }

    /// Decode a `ClientFrame` with an exact seven-byte payload.
    ///
    /// # Errors
    ///
    /// The payload length is checked before the header, and the header before
    /// any field, exactly as in [`Self::decode`].
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgSafeBoxError> {
        check_exact(frame.payload.len().saturating_add(1))?;
        let kind = SafeBoxKind::from_header(frame.header)
            .ok_or(CgSafeBoxError::InvalidHeader(frame.header))?;
        let mut pos = [0u8; 4];
        pos.copy_from_slice(&frame.payload[0..4]);
        Ok(Self {
            kind,
            container_pos: u32::from_le_bytes(pos),
            item_pos: crate::cg_item_move::CgItemPos::decode_at(&frame.payload, 4),
        })
    }
}

fn check_exact(len: usize) -> Result<(), CgSafeBoxError> {
    match len.cmp(&CG_SAFEBOX_WIRE_SIZE) {
        core::cmp::Ordering::Less => Err(CgSafeBoxError::Truncated {
            needed: CG_SAFEBOX_WIRE_SIZE,
            available: len,
        }),
        core::cmp::Ordering::Greater => Err(CgSafeBoxError::LengthMismatch {
            expected: CG_SAFEBOX_WIRE_SIZE,
            actual: len,
        }),
        core::cmp::Ordering::Equal => Ok(()),
    }
}

/// Why a safebox or mall transfer record could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgSafeBoxError {
    /// The input was shorter than the fixed eight bytes.
    Truncated {
        /// The fixed wire size that was required.
        needed: usize,
        /// How many bytes were actually available.
        available: usize,
    },
    /// The input was longer than the fixed eight bytes.
    LengthMismatch {
        /// The fixed wire size that was required.
        expected: usize,
        /// How many bytes were actually supplied.
        actual: usize,
    },
    /// The header byte was not 69, 70, or 71.
    InvalidHeader(u8),
}

impl fmt::Display for CgSafeBoxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(f, "safebox record needs {needed} bytes, got {available}")
            }
            Self::LengthMismatch { expected, actual } => {
                write!(f, "safebox record needs {expected} bytes, got {actual}")
            }
            Self::InvalidHeader(byte) => write!(
                f,
                "header {byte:#04x} is not a safebox or mall transfer header"
            ),
        }
    }
}

impl std::error::Error for CgSafeBoxError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_item_move::CgItemPos;
    use crate::cg_wire::{resolve_client_frame_size, ClientFrameDecoder, ClientFrameSize};

    const P: CgItemPos = CgItemPos {
        window_type: 1,
        cell: 0xffff,
    };

    fn rec(kind: SafeBoxKind, pos: u32) -> CgSafeBoxItem {
        CgSafeBoxItem::new(kind, pos, P)
    }

    /// The three identities are the only headers this record accepts, and
    /// the active inventory must agree that all three are fixed 8-byte
    /// records.
    #[test]
    fn active_inventory_agrees_on_all_three_headers() {
        const EXPECTED: [(SafeBoxKind, u8); 3] = [
            (SafeBoxKind::MallCheckout, 0x45),
            (SafeBoxKind::SafeboxCheckin, 0x46),
            (SafeBoxKind::SafeboxCheckout, 0x47),
        ];
        assert_eq!(SafeBoxKind::ALL.len(), EXPECTED.len());
        for (index, (kind, value)) in EXPECTED.into_iter().enumerate() {
            assert_eq!(kind, SafeBoxKind::ALL[index]);
            let header = kind.header();
            assert_eq!(header.value(), value);
            let size = resolve_client_frame_size(u8::from(header)).expect("registered header");
            assert_eq!(size, ClientFrameSize::Fixed(CG_SAFEBOX_WIRE_SIZE));
            assert!(size.is_fixed());
        }
    }

    /// The registry must not have been changed by this module, and the
    /// alternative 5-byte profile must never be what the active tree
    /// produces.
    #[test]
    fn active_profile_is_eight_bytes_and_the_five_byte_shape_is_unreachable() {
        assert_eq!(CG_SAFEBOX_WIRE_SIZE, 8);
        assert_eq!(CG_SAFEBOX_PAYLOAD_SIZE, 7);
        assert_eq!(CG_SAFEBOX_LEGACY_WIRE_SIZE, 5);
        assert_eq!(
            CG_SAFEBOX_WIRE_SIZE,
            CG_SAFEBOX_PAYLOAD_SIZE + 1,
            "payload is the wire size minus the header byte"
        );
        // 1 header + 4 container index + 3 packed position.
        assert_eq!(CG_SAFEBOX_WIRE_SIZE, 1 + 4 + CG_ITEM_POS_SIZE);
    }

    /// Golden bytes for each identity. The three differ in byte 0 only.
    #[test]
    fn golden_bytes_differ_only_in_the_header() {
        for (kind, header) in [
            (SafeBoxKind::MallCheckout, 0x45u8),
            (SafeBoxKind::SafeboxCheckin, 0x46),
            (SafeBoxKind::SafeboxCheckout, 0x47),
        ] {
            assert_eq!(
                rec(kind, 0x0403_0201).encode(),
                vec![header, 1, 2, 3, 4, 1, 0xff, 0xff]
            );
        }
        assert_eq!(
            rec(SafeBoxKind::MallCheckout, 0).encode()[1..],
            rec(SafeBoxKind::SafeboxCheckout, 0).encode()[1..]
        );
    }

    /// The shared position sits at offset 5, after the four-byte index.
    /// Offsets here are WIRE offsets, so 0 is the header byte.
    #[test]
    fn shared_position_starts_after_the_four_byte_index() {
        let wire = rec(SafeBoxKind::SafeboxCheckout, 0xdead_beef).encode();
        assert_eq!(&wire[1..5], &0xdead_beef_u32.to_le_bytes());
        assert_eq!(wire[5], P.window_type);
        assert_eq!(&wire[6..8], &P.cell.to_le_bytes());
        // The same three bytes are what the item-move codec puts at its
        // own position, so the packed TItemPos cannot have drifted.
        let mut packed = Vec::new();
        P.encode_into(&mut packed);
        assert_eq!(wire[5..8], packed[..]);
    }

    /// Every header value, every index value, and every window byte must
    /// round-trip unchanged, including the extremes.
    #[test]
    fn boundary_field_values_round_trip() {
        for kind in SafeBoxKind::ALL {
            for index in [0u32, 1, 255, 256, 0x7fff_ffff, 0x8000_0000, u32::MAX] {
                for (window, cell) in [(0u8, 0u16), (1, 0xffff), (0xff, 0x7fff), (0xfe, 0x8000)] {
                    let original = CgSafeBoxItem::new(
                        kind,
                        index,
                        CgItemPos {
                            window_type: window,
                            cell,
                        },
                    );
                    let wire = original.encode();
                    assert_eq!(CgSafeBoxItem::decode(&wire), Ok(original));
                    let frame = original.to_frame();
                    assert_eq!(frame.header, kind.header().value());
                    assert_eq!(frame.payload.len(), CG_SAFEBOX_PAYLOAD_SIZE);
                    assert_eq!(CgSafeBoxItem::decode_frame(&frame), Ok(original));
                }
            }
        }
    }

    /// The public `header()` accessor must agree with the stored kind, or a
    /// caller reading the identity of a record would be told a lie. A mutation
    /// that made it return a fixed value previously passed every other test.
    #[test]
    fn public_header_accessor_tracks_the_stored_kind() {
        for kind in SafeBoxKind::ALL {
            for index in [0u32, 42, u32::MAX] {
                let record = rec(kind, index);
                assert_eq!(record.header(), kind.header());
                assert_eq!(u8::from(record.header()), record.encode()[0]);
                assert_eq!(record.to_frame().header, record.encode()[0]);
            }
        }
    }

    /// `window_type` must stay opaque: no window byte may be rejected, and
    /// every one must round-trip. The legacy server never range-checks it.
    #[test]
    fn window_type_stays_opaque_and_is_never_rejected() {
        for byte in 0u8..=255 {
            let record = CgSafeBoxItem::new(
                SafeBoxKind::SafeboxCheckin,
                3,
                CgItemPos {
                    window_type: byte,
                    cell: 5,
                },
            );
            let wire = record.encode();
            assert_eq!(wire[5], byte);
            assert_eq!(CgSafeBoxItem::decode(&wire), Ok(record));
            assert_eq!(CgSafeBoxItem::decode_frame(&record.to_frame()), Ok(record));
        }
        // Including the two windows this record operates on, which the
        // struct-level legacy predicate rejects.
        for byte in [0u8, 1, 0xff] {
            let record = CgSafeBoxItem::new(
                SafeBoxKind::SafeboxCheckout,
                0,
                CgItemPos {
                    window_type: byte,
                    cell: 0xffff,
                },
            );
            assert_eq!(CgSafeBoxItem::decode(&record.encode()), Ok(record));
        }
    }

    /// The transfer direction is a wire fact, not a policy: only header 70
    /// moves an item into a container.
    #[test]
    fn only_the_checkin_identity_is_a_deposit() {
        assert!(SafeBoxKind::SafeboxCheckin.is_deposit());
        assert!(!SafeBoxKind::MallCheckout.is_deposit());
        assert!(!SafeBoxKind::SafeboxCheckout.is_deposit());
        for kind in SafeBoxKind::ALL {
            assert_eq!(SafeBoxKind::from_header(kind.header().value()), Some(kind));
        }
        assert_eq!(SafeBoxKind::from_header(0x44), None);
        assert_eq!(SafeBoxKind::from_header(0x48), None);
        assert_eq!(SafeBoxKind::from_header(0), None);
        assert_eq!(SafeBoxKind::from_header(255), None);
    }

    /// Raw decoding: length is checked before the header, so a short or long
    /// input reports a length error even when the header is also wrong.
    #[test]
    fn raw_length_errors_take_precedence_over_the_header() {
        let good = rec(SafeBoxKind::SafeboxCheckout, 7).encode();
        for len in 0..CG_SAFEBOX_WIRE_SIZE {
            let short = &good[..len];
            assert!(
                matches!(
                    CgSafeBoxItem::decode(short),
                    Err(CgSafeBoxError::Truncated { .. })
                ),
                "{len} bytes must be Truncated"
            );
            // Even with a header that is not one of ours.
            let mut bad = short.to_vec();
            if let Some(first) = bad.first_mut() {
                *first = 0xAA;
            }
            assert!(matches!(
                CgSafeBoxItem::decode(&bad),
                Err(CgSafeBoxError::Truncated { .. })
            ));
        }
        let mut long = good.clone();
        long.push(0);
        assert!(matches!(
            CgSafeBoxItem::decode(&long),
            Err(CgSafeBoxError::LengthMismatch { .. })
        ));
        // A wrong header at the exact length is the only way to reach
        // InvalidHeader.
        for byte in 0u8..=255 {
            let mut wire = good.clone();
            wire[0] = byte;
            let result = CgSafeBoxItem::decode(&wire);
            if SafeBoxKind::from_header(byte).is_some() {
                assert!(result.is_ok(), "{byte:#04x} is a valid header");
            } else {
                assert_eq!(
                    result,
                    Err(CgSafeBoxError::InvalidHeader(byte)),
                    "{byte:#04x}"
                );
            }
        }
    }

    /// The framed path must get the same precedence, and must be tested at
    /// the CORRECT payload length so the header branch is actually reached.
    /// A wrong header paired with a wrong length would only ever prove the
    /// length branch.
    #[test]
    fn framed_header_is_validated_at_the_exact_payload_length() {
        for byte in 0u8..=255 {
            let frame = ClientFrame::new(
                byte,
                rec(SafeBoxKind::SafeboxCheckout, 9).to_frame().payload,
            );
            let result = CgSafeBoxItem::decode_frame(&frame);
            if SafeBoxKind::from_header(byte).is_some() {
                assert!(
                    result.is_ok(),
                    "{byte:#04x} at the correct payload length must decode"
                );
            } else {
                assert_eq!(
                    result,
                    Err(CgSafeBoxError::InvalidHeader(byte)),
                    "{byte:#04x}"
                );
            }
        }
    }

    /// Framed length errors take precedence over the header, and the
    /// available length is reported against the full wire size.
    #[test]
    fn framed_length_errors_take_precedence_over_the_header() {
        let payload = rec(SafeBoxKind::MallCheckout, 3).to_frame().payload;
        for len in 0..CG_SAFEBOX_PAYLOAD_SIZE {
            for header in [0x00u8, 0x46, 0xAA] {
                let frame = ClientFrame::new(header, &payload[..len]);
                assert!(
                    matches!(
                        CgSafeBoxItem::decode_frame(&frame),
                        Err(CgSafeBoxError::Truncated { .. })
                    ),
                    "payload {len} with header {header:#04x} must be Truncated"
                );
            }
        }
        let mut big = payload.clone();
        big.push(0);
        assert!(matches!(
            CgSafeBoxItem::decode_frame(&ClientFrame::new(0xAA, big)),
            Err(CgSafeBoxError::LengthMismatch { .. })
        ));
    }

    /// A byte-at-a-time feed must retain the partial tail and yield
    /// nothing until the record is complete. A single bulk feed would not
    /// prove any of this.
    #[test]
    fn fragmented_stream_retains_a_partial_tail() {
        let a = rec(SafeBoxKind::SafeboxCheckin, 0x1122_3344).encode();
        let b = rec(SafeBoxKind::SafeboxCheckout, 0x5566_7788).encode();
        let mut stream = Vec::new();
        stream.extend_from_slice(&a);
        stream.extend_from_slice(&b);

        let mut decoder = ClientFrameDecoder::new();
        let mut got: Vec<CgSafeBoxItem> = Vec::new();
        let mut fed = 0usize;
        while fed < stream.len() {
            fed += 1;
            // Byte at a time, never a bulk feed.
            decoder.feed(&stream[fed - 1..fed]).unwrap();
            while let Some(frame) = decoder.try_decode().expect("framed record must decode") {
                got.push(CgSafeBoxItem::decode_frame(&frame).unwrap());
            }
            if got.is_empty() {
                // Nothing yet: the partial tail must be retained intact.
                assert_eq!(
                    decoder.buffered_len(),
                    fed,
                    "{fed} of 8 bytes must be retained"
                );
                assert_eq!(decoder.peek_header(), Some(0x46));
            }
        }
        assert_eq!(fed, 2 * CG_SAFEBOX_WIRE_SIZE);
        assert_eq!(got.len(), 2, "both coalesced records must decode");
        assert_eq!(got[0], rec(SafeBoxKind::SafeboxCheckin, 0x1122_3344));
        assert_eq!(got[0].container_pos, 0x1122_3344);
        assert!(got[0].kind.is_deposit());
        assert_eq!(got[1], rec(SafeBoxKind::SafeboxCheckout, 0x5566_7788));
        assert!(!got[1].kind.is_deposit());
        assert!(decoder.is_empty());
    }

    /// Feeding a whole buffer at once must agree with byte-at-a-time
    /// feeding, so the two paths cannot disagree about framing.
    #[test]
    fn bulk_and_fragmented_feeds_agree() {
        let mut stream = Vec::new();
        for kind in SafeBoxKind::ALL {
            stream.extend_from_slice(&rec(kind, 0xabcd_ef01).encode());
        }
        let mut bulk = ClientFrameDecoder::new();
        bulk.feed(&stream).unwrap();
        let mut fragmented = ClientFrameDecoder::new();
        for chunk in stream.chunks(3) {
            fragmented.feed(chunk).unwrap();
        }
        let drain = |d: &mut ClientFrameDecoder| {
            let mut out = Vec::new();
            while let Some(frame) = d.try_decode().unwrap() {
                out.push(CgSafeBoxItem::decode_frame(&frame).unwrap());
            }
            out
        };
        let from_bulk = drain(&mut bulk);
        assert_eq!(from_bulk, drain(&mut fragmented));
        assert_eq!(from_bulk.len(), 3);
        assert!(bulk.is_empty() && fragmented.is_empty());
    }

    /// Error display must name the size and the offending header.
    #[test]
    fn error_display_is_specific() {
        assert!(CgSafeBoxError::Truncated {
            needed: 8,
            available: 3
        }
        .to_string()
        .contains('8'));
        assert!(CgSafeBoxError::Truncated {
            needed: 8,
            available: 3
        }
        .to_string()
        .contains('3'));
        assert!(CgSafeBoxError::LengthMismatch {
            expected: 8,
            actual: 9
        }
        .to_string()
        .contains('9'));
        assert!(CgSafeBoxError::InvalidHeader(0xAA)
            .to_string()
            .contains("0xaa"));
        // The three valid headers must NOT be reported as invalid.
        for kind in SafeBoxKind::ALL {
            let s = CgSafeBoxError::InvalidHeader(kind.header().value()).to_string();
            assert!(s.contains("0x"), "{s}");
        }
        assert!(SafeBoxKind::SafeboxCheckin.to_string().contains("70"));
        assert!(SafeBoxKind::MallCheckout.to_string().contains('6'));
    }
}
