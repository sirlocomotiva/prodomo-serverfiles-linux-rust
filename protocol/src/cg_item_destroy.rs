//! Explicit codec for the fixed legacy `TPacketCGItemDestroy` record.
//!
//! `server/server/game/packet.h` sets `HEADER_CG_ITEM_DESTROY = 21` and
//! declares `command_item_destroy` at `packet.h:627-631` as a `BYTE header`
//! followed by one packed `TItemPos Cell`. `TItemPos` is itself packed, so it
//! is exactly three bytes, declared at
//! `server/server/common/length.h:956-1057` inside `#pragma pack(push, 1)`,
//! which makes this record exactly four bytes and its framed payload three. The
//! two facts are independent: `TItemPos` is three bytes *within* its own
//! `#pragma pack(push, 1)`, and the containing record is one byte wider than
//! that *because* `packet.h:274` opens `#pragma pack(1)` for the packet
//! declarations and does not close it until `:3540`.
//! The width is **not** profile-dependent: nothing in the declaration is
//! conditional.
//!
//! The position is reused as the already source-verified
//! [`CgItemPos`](crate::cg_item_move::CgItemPos). This is the seventh Rust
//! codec to share that one packed three-byte layout.
//!
//! # This is not a second `CgItemUse`
//!
//! `TPacketCGItemUse` at `packet.h:599-603` is byte-for-byte the same 4-byte
//! shape, and it is already implemented as
//! [`CgItemUse`](crate::cg_item_use::CgItemUse). This module is still a
//! **separate type**, for the same API-level reason Section 143 gave for the
//! safebox move: with two distinct types, passing a header-21 record to a
//! header-11 call site is a **compile error**, whereas a single type carrying a
//! `kind` field would let the same mistake compile and fail on the wire.
//! `crate::cg_item_move`, `crate::cg_item_use`, and
//! `crate::cg_item_use_to_item` all set the same precedent.
//!
//! The record is also *not* the same operation, which is the opposite of the
//! Section 143 finding and is the interesting part here. The safebox move
//! handler reads only the cell
//! halves and never looks at a window byte. Here the whole `TItemPos` is
//! passed to `CHARACTER::DestroyItem`, and the window byte is genuinely read
//! and range-checked.
//!
//! # Cross-direction collision, and a naming trap
//!
//! Header 21 is **not** free in the opposite direction. The server calls it
//! `HEADER_GC_ITEM_SET` at `packet.h:122`, but the **client calls the very
//! same inbound record `HEADER_GC_ITEM_SET2`** at
//! `client/Client/UserInterface/Packet.h:117`, because the client's own
//! `HEADER_GC_ITEM_SET` is 20 at `Packet.h:116`. The server tree has no
//! `ItemSet2` at all.
//!
//! That asymmetry is sharper than the usual size mismatch, because tooling
//! that pairs game-to-client records by name across the two trees will match
//! the **wrong** records rather than merely mis-size them. The two directions
//! must never share a header table.
//!
//! # Scope
//!
//! This is a transport-free record boundary. It preserves the raw window byte
//! and the raw little-endian cell word without interpreting either one. The
//! legacy server entry point `CHARACTER::DestroyItem` at
//! `char_item.cpp:7407-7442` performs the gameplay, inventory, protection, and
//! unequip policy, and none of that belongs here.
//!
//! # Legacy facts kept out of the codec
//!
//! * **The window byte is read and validated, but the validation is nearly a
//!   no-op for the two windows that matter.** `DestroyItem` calls
//!   `IsValidItemPosition`, which range-checks per window at
//!   `char_item.cpp:10015`, but `GetItem` returns
//!   `m_pointsInstant.pItems[wCell]` for both `INVENTORY` and `EQUIPMENT` from
//!   the **same** array at `char_item.cpp:262-269` with the **same** bound at
//!   `:10020-10022`, while `GetWear` indexes
//!   `pItems[INVENTORY_MAX_NUM + bCell]` at `:655`. The two window values
//!   therefore alias. `SAFEBOX` and `MALL` pass the position check but fall
//!   into `GetItem`'s `default:` arm at `:301-302`, which returns `NULL` at
//!   `:302`, so container items are
//!   simply not destroyable through this header. None of this is reproduced:
//!   the byte is preserved opaquely.
//! * **The server's guards are present and sound for what they check.**
//!   `item->IsExchanging()` at `char_item.cpp:7420`,
//!   `item->GetCount() <= 0` at `:7426`, and `IsSecured()` at `:7430` under
//!   `__ENABLE_INVENTORY_PROTECTED_SYSTEM__`, which **is** defined at
//!   `server/server/common/prodomodefines.h:169` and included at
//!   `char_item.cpp:48`. There is no `IsEquipped` check, but the path is
//!   handled correctly anyway, unequipping as a side effect at
//!   `item.cpp:369-377` with the wear slot cleared at `:1562`.
//! * **A use-after-free fires on every successful destroy, and is not
//!   reproduced here.** `char_item.cpp:7438` calls
//!   `ITEM_MANAGER::instance().RemoveItem(item)`, whose tail at
//!   `item_manager.cpp:594` is `M2_DESTROY_ITEM(item)`, reaching the plain
//!   `M2_DELETE(item)` at `item_manager.cpp:649`; the pooling and
//!   debug-allocator alternatives are commented out at
//!   `server/server/game/stdafx.h:6` and `:8`. The next line,
//!   `char_item.cpp:7439`, then passes `item->GetName()` into `ChatPacket`, and
//!   `item.h:66` dereferences `m_pProto` from the freed object. This is a
//!   legacy C++ robustness follow-up, not a codec concern.
//! * **The client can only ever transmit window byte 1.** The wrapper
//!   `netSendItemDestroyPacket` at
//!   `client/Client/UserInterface/PythonNetworkStreamModule.cpp:826-835`
//!   declares `TItemPos Cell;` at `:828` and assigns **only** `Cell.cell` at
//!   `:830`. The client `SItemPos()` constructor at
//!   `client/Client/UserInterface/GameType.h:426-430` defaults `window_type` to
//!   `INVENTORY`, so the window byte is fully initialised but permanently 1.
//!   It would satisfy any "every byte is defined" check while no other window
//!   is reachable, which is why the codec preserves all 256 values rather than
//!   the one the stock client produces. It is worth recording that this is
//!   the *safe* shape, because the client is not uniformly safe here.
//!   `PythonNetworkStreamModule.cpp` contains thirteen arity
//!   `switch (PyTuple_Size(...))` statements. Twelve have a `default:` arm,
//!   and ten of those twelve return `Py_BuildException()` from the `default`
//!   arm itself and are therefore safe. The destroy wrapper is not one of the
//!   thirteen at all: it has no arity switch, and it returns
//!   `Py_BuildException()` at `:831` on any failure, so it cannot transmit an
//!   uninitialised value.
//!
//!   The three that *are* unsafe, for contrast, are recorded in the ledger:
//!   `netSendGiveItemPacket`, whose `default: break;` at `:963-964` falls
//!   through to `:968` and sends uninitialised `iTargetVID` and `iItemCount`;
//!   `netSendSafeboxCheckoutPacket`, whose `default: break;` at `:1376-1377`
//!   falls through to `:1381` and sends an uninitialised `iSafeBoxPos`; and
//!   `netSendSafeboxCheckinPacket`, which is the worst of the three because its
//!   switch at `:1329-1347` has **no `default:` arm at all**, so any arity
//!   other than 2 or 3 falls through to `:1350` and sends an uninitialised
//!   `iSafeBoxPos`.
//! * **The two trees disagree on the field name.** The server calls it `Cell`
//!   and the client calls it `pos`; this codec uses `cell`, matching
//!   `crate::cg_item_use`.
//! * A `cell` value of `0xffff` is the client's own default from that
//!   constructor and must round-trip.
//! * The window byte is an enum whose numeric values depend on the server
//!   build's feature flags at `length.h:657-676`, so it can only be
//!   preserved, never interpreted.

use crate::cg_inventory::HEADER_CG_ITEM_DESTROY;
use crate::cg_wire::ClientFrame;
use std::fmt;

/// The exact wire size of one packed legacy `TItemPos`.
pub use crate::cg_item_move::CG_ITEM_POS_SIZE;

/// The exact payload size of a framed item-destroy request.
pub const CG_ITEM_DESTROY_PAYLOAD_SIZE: usize = 3;

/// The exact wire size of a complete item-destroy request.
pub const CG_ITEM_DESTROY_WIRE_SIZE: usize = 4;

/// One fixed legacy `TPacketCGItemDestroy` request.
///
/// `header` is not stored: the codec accepts only header 21 and always writes
/// it, so keeping a second copy in memory could only create a state that
/// cannot be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgItemDestroy {
    /// The one source position named by the record.
    pub cell: crate::cg_item_move::CgItemPos,
}

impl CgItemDestroy {
    /// Build a request for the given position.
    #[must_use]
    pub const fn new(cell: crate::cg_item_move::CgItemPos) -> Self {
        Self { cell }
    }

    /// The one-byte header this record encodes.
    #[must_use]
    pub const fn header() -> crate::cg_inventory::CgHeader {
        HEADER_CG_ITEM_DESTROY
    }

    /// Encode this request as the exact four wire bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_ITEM_DESTROY_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode this request into an existing buffer.
    ///
    /// The buffer must already have room for [`CG_ITEM_DESTROY_WIRE_SIZE`]
    /// bytes. The output is exactly `[0x15][window_type][cell LE]`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        self.cell.encode_into(out);
    }

    /// Project this request as a `ClientFrame` with an exact three-byte
    /// payload.
    #[must_use]
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_ITEM_DESTROY_PAYLOAD_SIZE);
        self.cell.encode_into(&mut payload);
        ClientFrame::new(Self::header().value(), payload)
    }

    /// Decode one exact four-byte record.
    ///
    /// # Errors
    ///
    /// Returns [`CgItemDestroyError::Truncated`] for a short input,
    /// [`CgItemDestroyError::LengthMismatch`] for a long input, and
    /// [`CgItemDestroyError::InvalidHeader`] when the header is not 21. The
    /// length is checked before the header, and the header is checked before
    /// any field is read.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgItemDestroyError> {
        check_exact(bytes.len())?;
        check_header(bytes[0])?;
        Ok(Self {
            cell: crate::cg_item_move::CgItemPos::decode_at(&bytes[1..4]),
        })
    }

    /// Decode one framed request.
    ///
    /// # Errors
    ///
    /// Returns [`CgItemDestroyError::Truncated`] for a payload shorter than
    /// three bytes, [`CgItemDestroyError::LengthMismatch`] for a longer
    /// payload, and [`CgItemDestroyError::InvalidHeader`] when the frame
    /// header is not 21. The payload length is checked before the header, and
    /// the header is checked before any payload byte is read.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgItemDestroyError> {
        let len = frame.payload.len();
        match len.cmp(&CG_ITEM_DESTROY_PAYLOAD_SIZE) {
            core::cmp::Ordering::Less => {
                return Err(CgItemDestroyError::Truncated {
                    needed: CG_ITEM_DESTROY_WIRE_SIZE,
                    available: len.saturating_add(1),
                })
            }
            core::cmp::Ordering::Greater => {
                return Err(CgItemDestroyError::LengthMismatch {
                    expected: CG_ITEM_DESTROY_WIRE_SIZE,
                    actual: len.saturating_add(1),
                });
            }
            core::cmp::Ordering::Equal => {}
        }
        check_header(frame.header)?;
        Ok(Self {
            cell: crate::cg_item_move::CgItemPos::decode_at(&frame.payload[0..3]),
        })
    }
}

impl fmt::Display for CgItemDestroy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CgItemDestroy(cell={{ window_type: {}, cell: {} }})",
            self.cell.window_type, self.cell.cell
        )
    }
}

/// Every way decoding a fixed item-destroy request can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CgItemDestroyError {
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
    /// The leading byte was not [`HEADER_CG_ITEM_DESTROY`].
    InvalidHeader {
        /// The header byte that was actually present.
        actual: u8,
    },
}

impl fmt::Display for CgItemDestroyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "item-destroy record needs {needed} bytes, got {available}"
                )
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "item-destroy record needs {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { actual: byte } => {
                write!(f, "header {byte:#04x} is not the item-destroy header")
            }
        }
    }
}

impl std::error::Error for CgItemDestroyError {}

fn check_exact(len: usize) -> Result<(), CgItemDestroyError> {
    match len.cmp(&CG_ITEM_DESTROY_WIRE_SIZE) {
        core::cmp::Ordering::Less => Err(CgItemDestroyError::Truncated {
            needed: CG_ITEM_DESTROY_WIRE_SIZE,
            available: len,
        }),
        core::cmp::Ordering::Greater => Err(CgItemDestroyError::LengthMismatch {
            expected: CG_ITEM_DESTROY_WIRE_SIZE,
            actual: len,
        }),
        core::cmp::Ordering::Equal => Ok(()),
    }
}

fn check_header(byte: u8) -> Result<(), CgItemDestroyError> {
    if byte != CgItemDestroy::header().value() {
        return Err(CgItemDestroyError::InvalidHeader { actual: byte });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_item_move::CgItemPos;
    use crate::cg_item_use::CgItemUse;
    use crate::cg_wire::{resolve_client_frame_size, ClientFrameDecoder, ClientFrameSize};
    use std::error::Error;

    fn rec(window: u8, cell: u16) -> CgItemDestroy {
        CgItemDestroy::new(CgItemPos::new(window, cell))
    }

    #[test]
    fn inventory_resolves_the_fixed_four_byte_size() {
        assert_eq!(HEADER_CG_ITEM_DESTROY.value(), 21);
        assert_eq!(CG_ITEM_POS_SIZE, 3);
        assert_eq!(CG_ITEM_DESTROY_WIRE_SIZE, 4);
        assert_eq!(CG_ITEM_DESTROY_PAYLOAD_SIZE, 3);
        assert_eq!(
            resolve_client_frame_size(21).unwrap(),
            ClientFrameSize::Fixed(4)
        );
    }

    #[test]
    fn golden_record_has_exact_source_offsets() {
        // header 21 at offset 0, then the packed three-byte position:
        // window_type at 1 and the little-endian cell word at 2..4.
        let packet = rec(0x01, 0x0203);
        assert_eq!(packet.encode(), vec![0x15, 0x01, 0x03, 0x02]);
        assert_eq!(packet.to_frame().payload, vec![0x01, 0x03, 0x02]);
        assert_eq!(packet.to_frame().header, 0x15);
        assert_eq!(
            packet.to_string(),
            "CgItemDestroy(cell={ window_type: 1, cell: 515 })"
        );
        assert_eq!(CgItemDestroy::decode(&packet.encode()).unwrap(), packet);
        assert_eq!(
            CgItemDestroy::decode_frame(&packet.to_frame()).unwrap(),
            packet
        );
    }

    /// The client can only ever send window byte 1, because the wrapper at
    /// `PythonNetworkStreamModule.cpp:828-830` default-constructs the
    /// position and then assigns only `cell`. This is the exact byte sequence
    /// a stock client therefore produces.
    #[test]
    fn stock_client_bytes_are_window_one_with_the_client_default_cell() {
        // The client ctor at GameType.h:426-430 sets window_type = INVENTORY
        // and cell = WORD_MAX, and the wrapper then overwrites only the cell.
        let packet = rec(1, 0xffff);
        assert_eq!(packet.encode(), vec![0x15, 0x01, 0xff, 0xff]);
        assert_eq!(CgItemDestroy::decode(&packet.encode()).unwrap(), packet);
    }

    /// Every one of the 256 window bytes must round-trip, because the server
    /// reads and range-checks the window and the client happens to send only
    /// one value. The codec must not narrow to the reachable case.
    #[test]
    fn every_window_byte_round_trips_in_both_paths() {
        for w in 0..=u8::MAX {
            let p = rec(w, 0xbeef);
            assert_eq!(p.encode()[1], w, "raw encode lost window {w}");
            assert_eq!(p.to_frame().payload[0], w, "framed encode lost window {w}");
            assert_eq!(CgItemDestroy::decode(&p.encode()).unwrap(), p);
            assert_eq!(CgItemDestroy::decode_frame(&p.to_frame()).unwrap(), p);
        }
    }

    #[test]
    fn cell_boundaries_round_trip() {
        for c in [0u16, 1, 0x00ff, 0x0100, 0xfffe, 0xffff, u16::MAX] {
            let p = rec(1, c);
            assert_eq!(CgItemDestroy::decode(&p.encode()).unwrap(), p, "cell {c}");
            assert_eq!(
                CgItemDestroy::decode_frame(&p.to_frame()).unwrap(),
                p,
                "cell {c}"
            );
        }
    }

    /// All 255 wrong headers must be rejected at the exact length. If the
    /// wrong-header cases were shorter than the record, a length-first
    /// decoder would reject them on length and never reach the header branch.
    #[test]
    fn every_wrong_header_is_rejected_at_the_exact_length() {
        for b in 0..=u8::MAX {
            if b == 21 {
                continue;
            }
            let mut raw = vec![b, 0x01, 0x00, 0x00];
            assert_eq!(
                CgItemDestroy::decode(&raw),
                Err(CgItemDestroyError::InvalidHeader { actual: b }),
                "raw accepted header {b}"
            );
            raw[0] = 21;
            assert!(CgItemDestroy::decode(&raw).is_ok());

            let frame = ClientFrame::new(b, vec![0x01, 0x00, 0x00]);
            assert_eq!(
                CgItemDestroy::decode_frame(&frame),
                Err(CgItemDestroyError::InvalidHeader { actual: b }),
                "framed accepted header {b}"
            );
            let good = ClientFrame::new(21, vec![0x01, 0x00, 0x00]);
            assert!(CgItemDestroy::decode_frame(&good).is_ok());
        }
    }

    /// Header 21 and header 11 are byte-identical apart from byte 0, so a
    /// record that crossed the two boundaries must be rejected by both.
    #[test]
    fn header_21_and_header_11_are_not_interchangeable() {
        let mut eleven = rec(1, 0x0203).encode();
        eleven[0] = 11;
        assert_eq!(
            CgItemDestroy::decode(&eleven),
            Err(CgItemDestroyError::InvalidHeader { actual: 11 })
        );
        let mut twenty_one = rec(1, 0x0203).encode();
        twenty_one[0] = 21;
        assert_eq!(
            CgItemUse::decode(&twenty_one),
            Err(crate::cg_item_use::CgItemUseError::InvalidHeader { actual: 21 })
        );
    }

    /// The two 4-byte records really do share a byte layout, which is why
    /// keeping them as separate types matters.
    #[test]
    fn the_two_four_byte_records_agree_on_layout_but_not_header() {
        let a = rec(1, 0x0203);
        assert_eq!(
            a.encode()[1..],
            CgItemUse::new(CgItemPos::new(1, 0x0203)).encode()[1..]
        );
        assert_eq!(
            a.encode().len(),
            CgItemUse::new(CgItemPos::new(1, 0)).encode().len()
        );
    }

    #[test]
    fn raw_length_is_checked_before_the_header() {
        // A short input with a wrong header must report the length, because
        // the length check runs first.
        assert_eq!(
            CgItemDestroy::decode(&[0x99, 0x01, 0x00]),
            Err(CgItemDestroyError::Truncated {
                needed: 4,
                available: 3
            })
        );
        assert_eq!(
            CgItemDestroy::decode(&[21, 0x01, 0x00, 0x00, 0x00]),
            Err(CgItemDestroyError::LengthMismatch {
                expected: 4,
                actual: 5
            })
        );
    }

    #[test]
    fn framed_length_is_checked_before_the_header() {
        for (payload, err) in [
            (
                vec![0x01, 0x00],
                CgItemDestroyError::Truncated {
                    needed: 4,
                    available: 3,
                },
            ),
            (
                vec![0x01, 0x00, 0x00, 0x00],
                CgItemDestroyError::LengthMismatch {
                    expected: 4,
                    actual: 5,
                },
            ),
        ] {
            let frame = ClientFrame::new(0x99, payload);
            assert_eq!(CgItemDestroy::decode_frame(&frame), Err(err));
        }
    }

    #[test]
    fn a_wrong_header_yields_a_header_error_even_when_the_position_is_valid() {
        // The source does check the header first, but that ordering is not
        // observable from here and no test can make it so: `check_exact` has
        // already proved four bytes are present, so a field read can neither
        // panic nor change the result. This test therefore pins the observable
        // contract only, namely that a wrong header reports `InvalidHeader` even
        // when the trailing bytes would decode as a perfectly good position.
        let err = CgItemDestroy::decode(&[0x00, 0xff, 0xff, 0xff]).unwrap_err();
        assert_eq!(err, CgItemDestroyError::InvalidHeader { actual: 0 });
        assert!(matches!(
            err,
            CgItemDestroyError::InvalidHeader { actual: 0 }
        ));
    }

    #[test]
    fn header_accessor_matches_every_encoder_path() {
        assert_eq!(CgItemDestroy::header().value(), 21);
        assert_eq!(rec(1, 0).encode()[0], CgItemDestroy::header().value());
        assert_eq!(rec(1, 0).to_frame().header, CgItemDestroy::header().value());
        let mut buf = vec![0xaa, 0xbb];
        rec(1, 0).encode_into(&mut buf);
        assert_eq!(buf[2], CgItemDestroy::header().value());
        assert_eq!(buf.len(), 6);
    }

    #[test]
    fn every_length_outside_the_exact_size_is_rejected() {
        for len in 0..=12usize {
            if len == 4 {
                // The only raw length that decodes.
                let ok = CgItemDestroy::decode(&[0x15; 4]).expect("exact raw length");
                assert_eq!(ok, rec(0x15, 0x1515));
                let fok = ClientFrame::new(0x15, vec![0x15; 3]);
                assert!(CgItemDestroy::decode_frame(&fok).is_ok());
                continue;
            }
            let raw = vec![0x15; len];
            let err = CgItemDestroy::decode(&raw).unwrap_err();
            if len < 4 {
                assert_eq!(
                    err,
                    CgItemDestroyError::Truncated {
                        needed: 4,
                        available: len
                    }
                );
            } else {
                assert_eq!(
                    err,
                    CgItemDestroyError::LengthMismatch {
                        expected: 4,
                        actual: len
                    }
                );
            }
            if len == 3 {
                // A three-byte payload is the exact framed size, so this
                // length is valid framed and was already covered above.
                continue;
            }
            let frame = ClientFrame::new(0x15, vec![0x15; len]);
            let err = CgItemDestroy::decode_frame(&frame).unwrap_err();
            if len < 4 {
                assert_eq!(
                    err,
                    CgItemDestroyError::Truncated {
                        needed: 4,
                        available: len + 1
                    }
                );
            } else {
                assert_eq!(
                    err,
                    CgItemDestroyError::LengthMismatch {
                        expected: 4,
                        actual: len + 1
                    }
                );
            }
        }
    }

    #[test]
    fn fragmented_streaming_matches_bulk_decoding() {
        let records: Vec<Vec<u8>> = (0..4u16)
            .map(|i| rec(u8::try_from(1 + i).expect("small window"), 0x0100 + i).encode())
            .collect();
        let stream: Vec<u8> = records.concat();

        for split in 0..stream.len() {
            let mut d = ClientFrameDecoder::new();
            d.feed(&stream[..split]).expect("bulk prefix feed");
            d.feed(&stream[split..]).expect("bulk suffix feed");
            let mut out = Vec::new();
            while let Some(f) = d.try_decode().expect("bulk try_decode") {
                out.push(CgItemDestroy::decode_frame(&f).expect("bulk record"));
            }
            let expect: Vec<CgItemDestroy> = records
                .iter()
                .map(|r| CgItemDestroy::decode(r).expect("expect record"))
                .collect();
            assert_eq!(out, expect, "bulk split at {split}");
        }
    }

    #[test]
    fn byte_at_a_time_streaming_retains_partial_tails() {
        let records: Vec<Vec<u8>> = (0..3u16).map(|i| rec(1, 0x0f00 + i).encode()).collect();
        let stream: Vec<u8> = records.concat();
        let mut d = ClientFrameDecoder::new();
        let mut out = Vec::new();
        for (i, b) in stream.iter().enumerate() {
            d.feed(&[*b]).expect("single byte feed");
            if i < 3 {
                // A 4-byte record is never complete before its last byte.
                assert!(d.buffered_len() > 0, "tail dropped at byte {i}");
            }
            while let Some(f) = d.try_decode().expect("byte-wise try_decode") {
                out.push(CgItemDestroy::decode_frame(&f).expect("byte-wise record"));
            }
        }
        assert_eq!(out.len(), 3);
        assert!(d.is_empty());
    }

    #[test]
    fn two_records_stream_back_to_back() {
        let mut d = ClientFrameDecoder::new();
        d.feed(&rec(1, 0x1111).encode()).expect("first feed");
        d.feed(&rec(2, 0x2222).encode()).expect("second feed");
        let a = d
            .try_decode()
            .expect("first frame")
            .expect("first frame present");
        let b = d
            .try_decode()
            .expect("second frame")
            .expect("second frame present");
        assert!(d.try_decode().expect("no third frame").is_none());
        assert_eq!(
            CgItemDestroy::decode_frame(&a).expect("first record"),
            rec(1, 0x1111)
        );
        assert_eq!(
            CgItemDestroy::decode_frame(&b).expect("second record"),
            rec(2, 0x2222)
        );
    }

    /// The shared position size and both public size constants are pure
    /// documentation, so nothing else would notice if any of them drifted.
    #[test]
    fn declared_sizes_match_the_bytes_actually_produced() {
        assert_eq!(CG_ITEM_POS_SIZE, 3);
        assert_eq!(CG_ITEM_POS_SIZE, crate::cg_item_move::CG_ITEM_POS_SIZE);
        let r = rec(1, 0x0203);
        assert_eq!(r.encode().len(), CG_ITEM_DESTROY_WIRE_SIZE);
        assert_eq!(r.to_frame().payload.len(), CG_ITEM_DESTROY_PAYLOAD_SIZE);
        assert_eq!(r.encode().len() - 1, CG_ITEM_DESTROY_PAYLOAD_SIZE);
    }

    #[test]
    fn error_display_is_stable_through_a_trait_object() {
        let cases = [
            (
                CgItemDestroyError::Truncated {
                    needed: 4,
                    available: 3,
                },
                "item-destroy record needs 4 bytes, got 3",
            ),
            (
                CgItemDestroyError::LengthMismatch {
                    expected: 4,
                    actual: 5,
                },
                "item-destroy record needs 4 bytes, got 5",
            ),
            (
                CgItemDestroyError::InvalidHeader { actual: 0x53 },
                "header 0x53 is not the item-destroy header",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
            let boxed: Box<dyn Error> = Box::new(error);
            assert_eq!(boxed.to_string(), text);
        }
    }
}
