//! Explicit codec for the fixed legacy `TPacketCGItemDrop` record.
//!
//! `server/server/game/packet.h` sets `HEADER_CG_ITEM_DROP = 12` and declares
//! `command_item_drop` at `packet.h:612-617` as a `BYTE header`, a packed
//! `TItemPos Cell`, and a `DWORD gold`. `TItemPos` is itself packed, so it is
//! exactly three bytes, declared at
//! `server/server/common/length.h:956-1057` inside `#pragma pack(push, 1)`,
//! which makes this record exactly **eight bytes** and its framed payload
//! seven. Nothing in the declaration is conditional, so the width is not
//! profile-dependent.
//!
//! The position is reused as the already source-verified
//! [`CgItemPos`](crate::cg_item_move::CgItemPos). This is the eighth Rust
//! codec to share that one packed three-byte layout.
//!
//! # This is not `ItemDrop2`, and the two are different operations
//!
//! `TPacketCGItemDrop2` at `packet.h:619-625` carries an extra `WORD count`
//! and is ten bytes, so the two records are not two spellings of one request.
//! Their handlers differ in exactly the way that matters:
//!
//! * `CInputMain::ItemDrop` at `input_main.cpp:1024-1040` handles header 12,
//!   with the signature at `:1024` and the `command_item_drop` cast at `:1026`,
//!   and calls `ch->DropItem(pinfo->Cell)` at `:1039`, with **one** argument.
//! * `CInputMain::ItemDrop2` at `input_main.cpp:1042-1051` handles header 20
//!   and calls `ch->DropItem(pinfo->Cell, pinfo->count)` at `:1050`, with
//!   **two**.
//!
//! Header 12 therefore has no `count` field and reads none, and this codec
//! does not invent one.
//!
//! # The `gold` field selects the operation and must not be validated here
//!
//! `input_main.cpp:1036` is `if (pinfo->gold > 0)`, which is the **sole**
//! discriminator between dropping currency and dropping an item: a client that
//! sets `gold` never reaches the item path at all, and the window byte in the
//! same record is simply ignored on that path.
//!
//! The only bound the server applies to the amount is a comparison against
//! the character's own balance inside `CHARACTER::DropGold` at
//! `char_item.cpp:7556`. That is character state, so this codec preserves
//! `gold` as an opaque 32-bit word and deliberately applies no bound of its
//! own. In particular it does **not** reproduce the narrowing described below.
//!
//! # Legacy facts kept out of the codec
//!
//! * **Header 12 can never drop a partial stack.** The declaration at
//!   `char.h:1190` is `bool DropItem(TItemPos Cell, WORD bCount=0);`, so the
//!   one-argument call passes zero, and `char_item.cpp:7498-7499` is
//!   `if (bCount == 0 || bCount > item->GetCount()) bCount =
//!   item->GetCount();`. Zero therefore means "the whole stack", not "one".
//!   The partial-drop machinery at `:7510-7530` is real and does split a
//!   stack, but only header 20 can reach it.
//! * **A dead branch, the third of its shape in this audit series.**
//!   Because `:7498-7499` has already replaced any zero, the
//!   `if (bCount == 0)` guard at `char_item.cpp:7512-7517` can never be true.
//!   The same pattern appears in the safebox move rollback at
//!   `safebox.cpp:231-235` and in that method's merge guard at `:216`.
//! * **`abs()` at `char_item.cpp:7446` is dead.** The line is
//!   `bCount = abs(bCount);` on a `WORD`. A `WORD` promotes to `int`, so the
//!   value is always in `0..=65535` and `abs` is the identity function.
//! * **A signedness narrowing changes behaviour on the gold path.**
//!   `CHARACTER::DropGold` is declared `bool DropGold(int gold)` at
//!   `char_item.cpp:7554`, taking a **signed** `int`, while the wire field is
//!   an unsigned `DWORD` passed straight through at `input_main.cpp:1037`.
//!   Any value at or above `0x80000000` arrives negative and is rejected by
//!   the first test at `:7556`, so the upper half of the wire range is
//!   unreachable for currency. The codec keeps all `u32` values and does not
//!   narrow. The declaration is reached through `char.h:1265`, which sits
//!   inside `#ifdef ENABLE_REMOVE_LIMIT_GOLD` at `char.h:1262`; that macro
//!   **is** defined, at `prodomodefines.h:157`, so the signed form is the
//!   active one. `GetGold()` in the same guard returns `unsigned long long`
//!   at `char.h:1263`, so the second half of that same test,
//!   `gold > GetGold()`, is evaluated in unsigned 64-bit arithmetic. Only
//!   the first half, `gold <= 0`, is signed.
//! * **The gold path is guarded less than the item path.** `DropGold` checks
//!   its amount at `:7556`, `CanHandleItem()` at `:7559`, and a time limit at
//!   `:7562-7569`. It has **no `IsDead()` test**, although `DropItem` has one
//!   at `:7467`, so a dead character can drop currency but not items. It also
//!   has **no `IsSecured()` test**, although `DropItem` has one at
//!   `:7489-7495` under `__ENABLE_INVENTORY_PROTECTED_SYSTEM__`, which is
//!   itself defined at `prodomodefines.h:169`, so a protected inventory still
//!   permits currency to be dropped. Both are character-level checks, which
//!   makes them genuine asymmetries. The item-level checks absent here,
//!   `IsExchanging`, `isLocked`, the quest-running test, and the
//!   `ITEM_ANTIFLAG_DROP` test, do **not** apply, because this path creates a
//!   synthetic ground item rather than removing a container item.
//! * **Neither path is rate limited as shipped.** `g_ItemDropTimeLimitValue`
//!   and `g_GoldDropTimeLimitValue` are both initialised to `0` at
//!   `questmanager.cpp:35` and `:33`, and both guards are written
//!   `if (0 != g_...)`, so a default server drops with no delay at all. The
//!   limits are only set from configuration, at `:1605` and `:1600`.
//! * **`DropGold` can report success on a failed placement.** `Save()` at
//!   `:7595` and `return true` at `:7596` sit *outside* the `AddToGround`
//!   block, while the actual currency deduction `PointChange` at `:7582` sits
//!   inside it. So a drop whose ground placement fails still returns true and
//!   still persists the character. The severity is nil only because the
//!   handler discards the return value at `input_main.cpp:1037`; a caller that
//!   checked it would see a false success.
//! * **The item path's checks, for completeness.** `DropItem` tests
//!   `CanHandleItem()` at `:7449`, a drop time limit at `:7455-7463` under
//!   `ENABLE_NEWSTUFF`, `IsDead()` at `:7467`, `IsValidItemPosition` and
//!   `GetItem` at `:7470`, `IsExchanging()` at `:7473`, `isLocked()` at
//!   `:7476`, a quest-running test at `:7479`, the
//!   `ITEM_ANTIFLAG_DROP | ITEM_ANTIFLAG_GIVE` test at `:7482-7486`, and
//!   `IsSecured()` at `:7489-7495`. Both return values are discarded by the
//!   handler at `input_main.cpp:1037` and `:1039`.
//! * **The two trees may disagree on the field names.** The server calls the
//!   position `Cell`; check `client/Client/UserInterface/Packet.h` for the
//!   client's spelling before relying on the name used here.
//! * **The client's own drop wrapper is one of the safe ones, and it never
//!   sends a currency word.** `netSendItemDropPacket` at
//!   `PythonNetworkStreamModule.cpp:802-824` switches on the arity at `:805`,
//!   with `case 1` at `:807` reading only `Cell.cell` and `case 2` at `:811`
//!   reading `Cell.window_type` and `Cell.cell`, and its `default:` arm at
//!   `:817-818` **returns** `Py_BuildException()` rather than breaking, so no
//!   arity falls through to a send. It then calls
//!   `SendItemDropPacket(Cell, 0)` at `:822`, hardcoding the currency word to
//!   zero, so the stock client can never reach the gold path through this
//!   wrapper. In `case 1` the window byte is never assigned, but it is also
//!   **not** indeterminate: `TItemPos Cell;` at `:804` is default-constructed
//!   through `SItemPos()` at `GameType.h:426-430` to `(INVENTORY, WORD_MAX)`,
//!   so the transmitted window byte is deterministically `1`. Calling that
//!   uninitialised would be wrong, and this codec is careful to distinguish
//!   the two cases.
//! * A `cell` value of `0xffff` is a normal opaque client value and must
//!   round-trip, because `SItemPos()` produces it.

use crate::cg_inventory::HEADER_CG_ITEM_DROP;
use crate::cg_wire::ClientFrame;
use std::fmt;

/// The exact wire size of one packed legacy `TItemPos`.
pub use crate::cg_item_move::CG_ITEM_POS_SIZE;

/// The exact payload size of a framed item-drop request.
pub const CG_ITEM_DROP_PAYLOAD_SIZE: usize = 7;

/// The exact wire size of a complete item-drop request.
pub const CG_ITEM_DROP_WIRE_SIZE: usize = 8;

/// One fixed legacy `TPacketCGItemDrop` request.
///
/// `header` is not stored: the codec accepts only header 12 and always writes
/// it, so keeping a second copy in memory could only create a state that
/// cannot be encoded.
///
/// `gold` is an opaque 32-bit word, not a validated amount. See the module
/// documentation for why it must not be bounded here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgItemDrop {
    /// The one source position named by the record.
    pub cell: crate::cg_item_move::CgItemPos,
    /// The opaque 32-bit currency word.
    pub gold: u32,
}

impl CgItemDrop {
    /// Build a request for the given position and currency word.
    #[must_use]
    pub const fn new(cell: crate::cg_item_move::CgItemPos, gold: u32) -> Self {
        Self { cell, gold }
    }

    /// The one-byte header this record encodes.
    #[must_use]
    pub const fn header() -> crate::cg_inventory::CgHeader {
        HEADER_CG_ITEM_DROP
    }

    /// Encode this request as the exact eight wire bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_ITEM_DROP_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode this request into an existing buffer.
    ///
    /// The buffer must already have room for [`CG_ITEM_DROP_WIRE_SIZE`] bytes.
    /// The output is exactly `[0x0c][window_type][cell LE][gold LE]`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        self.cell.encode_into(out);
        out.extend_from_slice(&self.gold.to_le_bytes());
    }

    /// Project this request as a `ClientFrame` with an exact seven-byte
    /// payload.
    #[must_use]
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_ITEM_DROP_PAYLOAD_SIZE);
        self.cell.encode_into(&mut payload);
        payload.extend_from_slice(&self.gold.to_le_bytes());
        ClientFrame::new(Self::header().value(), payload)
    }

    /// Decode one exact eight-byte record.
    ///
    /// # Errors
    ///
    /// Returns [`CgItemDropError::Truncated`] for a short input,
    /// [`CgItemDropError::LengthMismatch`] for a long input, and
    /// [`CgItemDropError::InvalidHeader`] when the header is not 12. The
    /// length is checked before the header, and the header is checked before
    /// any field is read.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgItemDropError> {
        check_exact(bytes.len())?;
        check_header(bytes[0])?;
        Ok(Self {
            cell: crate::cg_item_move::CgItemPos::decode_at(bytes, 1),
            gold: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        })
    }

    /// Decode one framed request.
    ///
    /// # Errors
    ///
    /// Returns [`CgItemDropError::Truncated`] for a payload shorter than
    /// seven bytes, [`CgItemDropError::LengthMismatch`] for a longer
    /// payload, and [`CgItemDropError::InvalidHeader`] when the frame header
    /// is not 12. The payload length is checked before the header, and the
    /// header is checked before any payload byte is read.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgItemDropError> {
        let len = frame.payload.len();
        match len.cmp(&CG_ITEM_DROP_PAYLOAD_SIZE) {
            core::cmp::Ordering::Less => {
                return Err(CgItemDropError::Truncated {
                    needed: CG_ITEM_DROP_WIRE_SIZE,
                    available: len.saturating_add(1),
                })
            }
            core::cmp::Ordering::Greater => {
                return Err(CgItemDropError::LengthMismatch {
                    expected: CG_ITEM_DROP_WIRE_SIZE,
                    actual: len.saturating_add(1),
                });
            }
            core::cmp::Ordering::Equal => {}
        }
        check_header(frame.header)?;
        Ok(Self {
            cell: crate::cg_item_move::CgItemPos::decode_at(&frame.payload, 0),
            gold: u32::from_le_bytes([
                frame.payload[3],
                frame.payload[4],
                frame.payload[5],
                frame.payload[6],
            ]),
        })
    }
}

impl fmt::Display for CgItemDrop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CgItemDrop(cell={{ window_type: {}, cell: {} }}, gold: {})",
            self.cell.window_type, self.cell.cell, self.gold
        )
    }
}

/// Every way decoding a fixed item-drop request can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CgItemDropError {
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
    /// The leading byte was not [`HEADER_CG_ITEM_DROP`].
    InvalidHeader {
        /// The header byte that was actually present.
        actual: u8,
    },
}

impl fmt::Display for CgItemDropError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(f, "item-drop record needs {needed} bytes, got {available}")
            }
            Self::LengthMismatch { expected, actual } => {
                write!(f, "item-drop record needs {expected} bytes, got {actual}")
            }
            Self::InvalidHeader { actual: byte } => {
                write!(f, "header {byte:#04x} is not the item-drop header")
            }
        }
    }
}

impl std::error::Error for CgItemDropError {}

fn check_exact(len: usize) -> Result<(), CgItemDropError> {
    match len.cmp(&CG_ITEM_DROP_WIRE_SIZE) {
        core::cmp::Ordering::Less => Err(CgItemDropError::Truncated {
            needed: CG_ITEM_DROP_WIRE_SIZE,
            available: len,
        }),
        core::cmp::Ordering::Greater => Err(CgItemDropError::LengthMismatch {
            expected: CG_ITEM_DROP_WIRE_SIZE,
            actual: len,
        }),
        core::cmp::Ordering::Equal => Ok(()),
    }
}

fn check_header(byte: u8) -> Result<(), CgItemDropError> {
    if byte != CgItemDrop::header().value() {
        return Err(CgItemDropError::InvalidHeader { actual: byte });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_wire::{resolve_client_frame_size, ClientFrameDecoder, ClientFrameSize};
    use std::error::Error;

    fn rec(window: u8, cell: u16, gold: u32) -> CgItemDrop {
        CgItemDrop::new(crate::cg_item_move::CgItemPos::new(window, cell), gold)
    }

    #[test]
    fn inventory_resolves_the_fixed_eight_byte_size() {
        assert_eq!(HEADER_CG_ITEM_DROP.value(), 12);
        assert_eq!(CG_ITEM_POS_SIZE, 3);
        assert_eq!(CG_ITEM_DROP_WIRE_SIZE, 8);
        assert_eq!(CG_ITEM_DROP_PAYLOAD_SIZE, 7);
        assert_eq!(
            resolve_client_frame_size(12).unwrap(),
            ClientFrameSize::Fixed(8)
        );
    }

    #[test]
    fn golden_record_has_exact_source_offsets() {
        // header 12 at 0, window_type at 1, cell LE at 2..4, gold LE at 4..8.
        let p = rec(0x01, 0x0203, 0x0405_0607);
        assert_eq!(
            p.encode(),
            vec![0x0c, 0x01, 0x03, 0x02, 0x07, 0x06, 0x05, 0x04]
        );
        assert_eq!(
            p.to_frame().payload,
            vec![0x01, 0x03, 0x02, 0x07, 0x06, 0x05, 0x04]
        );
        assert_eq!(p.to_frame().header, 0x0c);
        assert_eq!(
            p.to_string(),
            "CgItemDrop(cell={ window_type: 1, cell: 515 }, gold: 67438087)"
        );
        assert_eq!(CgItemDrop::decode(&p.encode()).unwrap(), p);
        assert_eq!(CgItemDrop::decode_frame(&p.to_frame()).unwrap(), p);
    }

    /// The gold path is the default when a client sets a currency word, so a
    /// zero gold is the ordinary item-drop case. Both must round-trip.
    #[test]
    fn zero_gold_and_the_gold_path_both_round_trip() {
        for gold in [0u32, 1, 1000, 0x7fff_ffff, 0x8000_0000, u32::MAX] {
            let p = rec(1, 0x0010, gold);
            assert_eq!(CgItemDrop::decode(&p.encode()).unwrap(), p, "gold {gold}");
            assert_eq!(
                CgItemDrop::decode_frame(&p.to_frame()).unwrap(),
                p,
                "gold {gold}"
            );
        }
    }

    /// The upper half of the `u32` range is unreachable for currency because
    /// the server narrows to a signed `int` before comparing. The codec must
    /// not reproduce that narrowing, so every value must still round-trip.
    #[test]
    fn the_whole_u32_range_round_trips_unnarrowed() {
        for gold in [0x8000_0000u32, 0xffff_ffff, 0x7fff_ffff, 0x8000_0001] {
            let p = rec(1, 0, gold);
            let back = CgItemDrop::decode(&p.encode()).unwrap();
            assert_eq!(back.gold, gold, "gold {gold} was narrowed");
        }
    }

    #[test]
    fn every_window_byte_round_trips_in_both_paths() {
        for w in 0..=u8::MAX {
            let p = rec(w, 0xbeef, 0);
            assert_eq!(p.encode()[1], w, "raw encode lost window {w}");
            assert_eq!(p.to_frame().payload[0], w, "framed encode lost window {w}");
            assert_eq!(CgItemDrop::decode(&p.encode()).unwrap(), p);
            assert_eq!(CgItemDrop::decode_frame(&p.to_frame()).unwrap(), p);
        }
    }

    #[test]
    fn cell_boundaries_round_trip() {
        for c in [0u16, 1, 0x00ff, 0x0100, 0xfffe, 0xffff, u16::MAX] {
            let p = rec(1, c, 7);
            assert_eq!(CgItemDrop::decode(&p.encode()).unwrap(), p, "cell {c}");
            assert_eq!(
                CgItemDrop::decode_frame(&p.to_frame()).unwrap(),
                p,
                "cell {c}"
            );
        }
    }

    #[test]
    fn every_wrong_header_is_rejected_at_the_exact_length() {
        for b in 0..=u8::MAX {
            if b == 12 {
                continue;
            }
            let raw = vec![b, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
            assert_eq!(
                CgItemDrop::decode(&raw),
                Err(CgItemDropError::InvalidHeader { actual: b }),
                "raw accepted header {b}"
            );
            let frame = ClientFrame::new(b, vec![0x01, 0x00, 0x00, 0, 0, 0, 0]);
            assert_eq!(
                CgItemDrop::decode_frame(&frame),
                Err(CgItemDropError::InvalidHeader { actual: b }),
                "framed accepted header {b}"
            );
        }
        assert!(CgItemDrop::decode(&[12, 0, 0, 0, 0, 0, 0, 0]).is_ok());
        assert!(CgItemDrop::decode_frame(&ClientFrame::new(12, vec![0; 7])).is_ok());
    }

    /// Header 12 and header 20 are different operations, not two spellings of
    /// one request, so their records must not be interchangeable.
    #[test]
    fn header_12_and_header_20_are_not_interchangeable() {
        let mut twenty = rec(1, 0x0203, 0).encode();
        twenty[0] = 20;
        assert_eq!(
            CgItemDrop::decode(&twenty),
            Err(CgItemDropError::InvalidHeader { actual: 20 })
        );
    }

    #[test]
    fn raw_length_is_checked_before_the_header() {
        assert_eq!(
            CgItemDrop::decode(&[0x99, 0x01, 0x00]),
            Err(CgItemDropError::Truncated {
                needed: 8,
                available: 3
            })
        );
        let mut long = vec![12u8; 9];
        long[0] = 0x99;
        assert_eq!(
            CgItemDrop::decode(&long),
            Err(CgItemDropError::LengthMismatch {
                expected: 8,
                actual: 9
            })
        );
    }

    #[test]
    fn framed_length_is_checked_before_the_header() {
        for (payload, err) in [
            (
                vec![0x01, 0x00],
                CgItemDropError::Truncated {
                    needed: 8,
                    available: 3,
                },
            ),
            (
                vec![0u8; 8],
                CgItemDropError::LengthMismatch {
                    expected: 8,
                    actual: 9,
                },
            ),
        ] {
            let frame = ClientFrame::new(0x99, payload);
            assert_eq!(CgItemDrop::decode_frame(&frame), Err(err));
        }
    }

    #[test]
    fn raw_header_is_checked_before_any_field_is_read() {
        let err =
            CgItemDrop::decode(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]).unwrap_err();
        assert_eq!(err, CgItemDropError::InvalidHeader { actual: 0 });
    }

    #[test]
    fn header_accessor_matches_every_encoder_path() {
        assert_eq!(CgItemDrop::header().value(), 12);
        assert_eq!(rec(1, 0, 0).encode()[0], CgItemDrop::header().value());
        assert_eq!(rec(1, 0, 0).to_frame().header, CgItemDrop::header().value());
        let mut buf = vec![0xaa, 0xbb];
        rec(1, 0, 0).encode_into(&mut buf);
        assert_eq!(buf[2], CgItemDrop::header().value());
        assert_eq!(buf.len(), 10);
    }

    #[test]
    fn every_length_outside_the_exact_size_is_rejected() {
        for len in 0..=14usize {
            if len == 8 {
                let ok = CgItemDrop::decode(&[0x0c; 8]).expect("exact raw length");
                assert_eq!(ok, rec(0x0c, 0x0c0c, 0x0c0c_0c0c));
                assert!(CgItemDrop::decode_frame(&ClientFrame::new(0x0c, vec![0x0c; 7])).is_ok());
                continue;
            }
            let raw = vec![0x0c; len];
            let err = CgItemDrop::decode(&raw).unwrap_err();
            if len < 8 {
                assert_eq!(
                    err,
                    CgItemDropError::Truncated {
                        needed: 8,
                        available: len
                    }
                );
            } else {
                assert_eq!(
                    err,
                    CgItemDropError::LengthMismatch {
                        expected: 8,
                        actual: len
                    }
                );
            }
            if len == 7 {
                // A seven-byte payload is the exact framed size.
                continue;
            }
            let frame = ClientFrame::new(0x0c, vec![0x0c; len]);
            let err = CgItemDrop::decode_frame(&frame).unwrap_err();
            if len < 8 {
                assert_eq!(
                    err,
                    CgItemDropError::Truncated {
                        needed: 8,
                        available: len + 1
                    }
                );
            } else {
                assert_eq!(
                    err,
                    CgItemDropError::LengthMismatch {
                        expected: 8,
                        actual: len + 1
                    }
                );
            }
        }
    }

    #[test]
    fn fragmented_streaming_matches_bulk_decoding() {
        let records: Vec<Vec<u8>> = (0..4u16)
            .map(|i| {
                rec(
                    u8::try_from(1 + i).expect("small window"),
                    0x0100 + i,
                    0x1000 * u32::from(i),
                )
                .encode()
            })
            .collect();
        let stream: Vec<u8> = records.concat();
        for split in 0..stream.len() {
            let mut d = ClientFrameDecoder::new();
            d.feed(&stream[..split]).expect("bulk prefix feed");
            d.feed(&stream[split..]).expect("bulk suffix feed");
            let mut out = Vec::new();
            while let Some(f) = d.try_decode().expect("bulk try_decode") {
                out.push(CgItemDrop::decode_frame(&f).expect("bulk record"));
            }
            let expect: Vec<CgItemDrop> = records
                .iter()
                .map(|r| CgItemDrop::decode(r).expect("expect record"))
                .collect();
            assert_eq!(out, expect, "bulk split at {split}");
        }
    }

    #[test]
    fn byte_at_a_time_streaming_retains_partial_tails() {
        let records: Vec<Vec<u8>> = (0..3u16)
            .map(|i| rec(1, 0x0f00 + i, u32::from(i)).encode())
            .collect();
        let stream: Vec<u8> = records.concat();
        let mut d = ClientFrameDecoder::new();
        let mut out = Vec::new();
        for (i, b) in stream.iter().enumerate() {
            d.feed(&[*b]).expect("single byte feed");
            if i < 7 {
                assert!(d.buffered_len() > 0, "tail dropped at byte {i}");
            }
            while let Some(f) = d.try_decode().expect("byte-wise try_decode") {
                out.push(CgItemDrop::decode_frame(&f).expect("byte-wise record"));
            }
        }
        assert_eq!(out.len(), 3);
        assert!(d.is_empty());
    }

    #[test]
    fn two_records_stream_back_to_back() {
        let mut d = ClientFrameDecoder::new();
        d.feed(&rec(1, 0x1111, 1).encode()).expect("first feed");
        d.feed(&rec(2, 0x2222, 2).encode()).expect("second feed");
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
            CgItemDrop::decode_frame(&a).expect("first record"),
            rec(1, 0x1111, 1)
        );
        assert_eq!(
            CgItemDrop::decode_frame(&b).expect("second record"),
            rec(2, 0x2222, 2)
        );
    }

    #[test]
    fn declared_sizes_match_the_bytes_actually_produced() {
        assert_eq!(CG_ITEM_POS_SIZE, 3);
        assert_eq!(CG_ITEM_POS_SIZE, crate::cg_item_move::CG_ITEM_POS_SIZE);
        let r = rec(1, 0x0203, 0x0405_0607);
        assert_eq!(r.encode().len(), CG_ITEM_DROP_WIRE_SIZE);
        assert_eq!(r.to_frame().payload.len(), CG_ITEM_DROP_PAYLOAD_SIZE);
        assert_eq!(r.encode().len() - 1, CG_ITEM_DROP_PAYLOAD_SIZE);
    }

    #[test]
    fn error_display_is_stable_through_a_trait_object() {
        let cases = [
            (
                CgItemDropError::Truncated {
                    needed: 8,
                    available: 3,
                },
                "item-drop record needs 8 bytes, got 3",
            ),
            (
                CgItemDropError::LengthMismatch {
                    expected: 8,
                    actual: 9,
                },
                "item-drop record needs 8 bytes, got 9",
            ),
            (
                CgItemDropError::InvalidHeader { actual: 0x53 },
                "header 0x53 is not the item-drop header",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
            let boxed: Box<dyn Error> = Box::new(error);
            assert_eq!(boxed.to_string(), text);
        }
    }
}
