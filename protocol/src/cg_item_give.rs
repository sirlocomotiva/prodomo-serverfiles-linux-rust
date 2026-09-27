//! Explicit codec for the fixed legacy `TPacketCGGiveItem` record.
//!
//! `server/server/game/packet.h` defines `HEADER_CG_ITEM_GIVE = 83` and a
//! packed record holding a header byte, a 32-bit target character
//! identifier, one `TItemPos` position, and a one-byte count. `TItemPos` is
//! itself packed, so it is exactly three bytes, which makes this record
//! exactly nine bytes.
//!
//! The position is reused as the already source-verified
//! [`CgItemPos`](crate::cg_item_move::CgItemPos) from the item-move record.
//! This is the **fourth Rust codec** to reuse that shared type, after
//! item-move, item-use, and item-use-to-item. That is a statement about this
//! repository, not about the legacy source: `packet.h` in fact embeds a
//! packed `TItemPos` in **twelve** `TPacketCG*` records, and this one is the
//! twelfth of them in declaration order, not the fourth. The register is at
//! `packet_info.cpp:161`.
//!
//! # Wire layout
//!
//! ```text
//! offset 0  BYTE    header          (always 0x53)
//! offset 1  DWORD   dwTargetVID     little-endian
//! offset 5  TItemPos               packed, three bytes
//! offset 8  BYTE    byItemCount
//! ```
//!
//! # Scope
//!
//! This is a transport-free record boundary. It preserves the raw target
//! identifier, the raw window byte, the raw little-endian cell word, and
//! the raw count byte without interpreting any of them. Everything above
//! the record belongs to `CHARACTER_MANAGER::Find`, `CHARACTER::GiveItem`,
//! `CHARACTER::CanReceiveItem`, and `CHARACTER::ReceiveItem`, none of which
//! is reproduced here. Note in particular that the legacy path has **no
//! ownership check and no same-map check at all**: its only locality test
//! is a two-dimensional distance comparison, so a future adapter must not
//! assume either check exists.
//!
//! # Legacy facts kept out of the codec
//!
//! * The count byte is a **dead wire field**. `byItemCount` is declared at
//!   `packet.h:2325` and read nowhere on this path: the handler
//!   `CInputMain::ItemGive` at `input_main.cpp:3174-3183` casts the record
//!   and forwards only `p->ItemPos`, and the only other `byItemCount` in
//!   the server is the unrelated `SShopTable::byItemCount` field at
//!   `common/tables.h:825`. The transferred amount is therefore chosen
//!   entirely by the receiver, and the only possible outcomes are zero
//!   units, exactly one unit, or the whole stack. No code path can honor
//!   it. The byte is still physically present in every frame the client
//!   sends, so the codec preserves it, but it is not evidence of a
//!   transfer policy and must never be used to derive one.
//! * **This is not a player-to-player give.** `CanReceiveItem` opens with
//!   `if (IsPC()) return false;` at `char_item.cpp:9255-9256`, and
//!   `ReceiveItem` repeats the guard at `:9425-9426`, so the destination of
//!   a header-83 transfer can never be a player. A future adapter must not
//!   read this record as a trade or a player transfer primitive.
//! * **Documented legacy defects, not codec behaviour.** The client builder
//!   at `PythonNetworkStreamPhaseGame.cpp:4378-4390` has no
//!   `__CanActMainInstance()` gate, unlike every other item builder, so a
//!   script can emit this record while dead, loading, trading, shopping, or
//!   attacking. The Python wrapper at `PythonNetworkStreamModule.cpp:938-970`
//!   ends its arity switch with `default: break;` at `:963-964` instead of an
//!   early return, so any arity other than 3 or 4 falls through to `:968` and
//!   sends **uninitialized stack values** for the target identifier and the
//!   count; that is the strongest reason the count byte must stay opaque. On
//!   the server side `CInputMain::ItemGive` is a bare C-style cast with no
//!   length, header, or data check at `input_main.cpp:3176`, and the dispatch
//!   at `:3723-3726` gates only on `!ch->IsObserverMode()`.
//! * The eight-byte framed payload is guaranteed by the framing layer, not
//!   by the handler: `input.cpp:83` looks the size up in `CPacketInfoCG`,
//!   `input.cpp:92-93` refuses to dispatch until the buffer holds the whole
//!   record, and `packet_info.cpp:161` supplies `sizeof(TPacketCGGiveItem)`.
//!   `CInputMain::ItemGive` itself performs no length, header, or data
//!   check, which is a documented legacy defect and stays out of here.
//! * The client and the server spell the same constant differently:
//!   `HEADER_CG_ITEM_GIVE` in the server and `HEADER_CG_GIVE_ITEM` in the
//!   client. The two structs are byte-identical.
//! * Header 83 is **not unique across directions**. `HEADER_GC_PARTY_PARAMETER`
//!   is also 83, and that record is only two bytes, so the same number
//!   means a nine-byte inbound request and a two-byte outbound record. No
//!   header table may be shared across the two directions.
//! * The window byte is a `BYTE` field that carries an `EWindows` value
//!   whose numbering depends on the build's feature flags, so it can only
//!   be preserved, never interpreted.
//! * `byItemCount` is narrowed from a signed `int` on the client with no
//!   range check, so any byte value can legitimately appear.

use crate::cg_inventory::HEADER_CG_ITEM_GIVE;
use crate::cg_wire::ClientFrame;
use std::fmt;

/// The exact wire size of one packed legacy `TItemPos`.
pub use crate::cg_item_move::CG_ITEM_POS_SIZE;

/// The exact payload size of a framed item-give request.
pub const CG_ITEM_GIVE_PAYLOAD_SIZE: usize = 8;

/// The exact wire size of a complete item-give request.
pub const CG_ITEM_GIVE_WIRE_SIZE: usize = 9;

/// One fixed legacy `TPacketCGGiveItem` request.
///
/// `header` is not stored: the codec accepts only header 83 and always
/// writes it, so keeping a second copy in memory could only create a state
/// that cannot be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgItemGive {
    /// The little-endian 32-bit identifier of the receiving character.
    pub target_vid: u32,
    /// The source position the item is taken from.
    pub item_pos: crate::cg_item_move::CgItemPos,
    /// The raw count byte. Preserved verbatim and never interpreted.
    pub item_count: u8,
}

impl CgItemGive {
    /// Build a request for the given target, source position, and raw
    /// count byte.
    #[must_use]
    pub const fn new(
        target_vid: u32,
        item_pos: crate::cg_item_move::CgItemPos,
        item_count: u8,
    ) -> Self {
        Self {
            target_vid,
            item_pos,
            item_count,
        }
    }

    /// Encode this request as the exact nine wire bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_ITEM_GIVE_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode this request into an existing buffer.
    ///
    /// The buffer must already have room for [`CG_ITEM_GIVE_WIRE_SIZE`]
    /// bytes. The output is exactly
    /// `[0x53][dwTargetVID LE][window_type][cell LE][byItemCount]`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(HEADER_CG_ITEM_GIVE.value());
        out.extend_from_slice(&self.target_vid.to_le_bytes());
        self.item_pos.encode_into(out);
        out.push(self.item_count);
    }

    /// Project this request as a `ClientFrame` with an exact eight-byte
    /// payload.
    #[must_use]
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_ITEM_GIVE_PAYLOAD_SIZE);
        payload.extend_from_slice(&self.target_vid.to_le_bytes());
        self.item_pos.encode_into(&mut payload);
        payload.push(self.item_count);
        ClientFrame::new(HEADER_CG_ITEM_GIVE.value(), payload)
    }

    /// Decode one exact nine-byte record.
    ///
    /// # Errors
    ///
    /// Returns [`CgItemGiveError::Truncated`] for a short input,
    /// [`CgItemGiveError::LengthMismatch`] for a long input, and
    /// [`CgItemGiveError::InvalidHeader`] when the header is not 83. The
    /// length is checked before the header, and the header is checked
    /// before any field is read.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgItemGiveError> {
        check_exact(bytes.len())?;
        check_header(bytes[0])?;
        let mut vid = [0u8; 4];
        vid.copy_from_slice(&bytes[1..5]);
        Ok(Self {
            target_vid: u32::from_le_bytes(vid),
            item_pos: crate::cg_item_move::CgItemPos::decode_at(bytes, 5),
            item_count: bytes[8],
        })
    }

    /// Decode one framed request.
    ///
    /// # Errors
    ///
    /// Returns [`CgItemGiveError::Truncated`] for a payload shorter than
    /// eight bytes, [`CgItemGiveError::LengthMismatch`] for a longer
    /// payload, and [`CgItemGiveError::InvalidHeader`] when the frame
    /// header is not 83. The payload length is checked before the header,
    /// and the header is checked before any payload byte is read.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgItemGiveError> {
        let len = frame.payload.len();
        match len.cmp(&CG_ITEM_GIVE_PAYLOAD_SIZE) {
            core::cmp::Ordering::Less => {
                return Err(CgItemGiveError::Truncated {
                    needed: CG_ITEM_GIVE_WIRE_SIZE,
                    available: len.saturating_add(1),
                })
            }
            core::cmp::Ordering::Greater => {
                return Err(CgItemGiveError::LengthMismatch {
                    expected: CG_ITEM_GIVE_WIRE_SIZE,
                    actual: len.saturating_add(1),
                });
            }
            core::cmp::Ordering::Equal => {}
        }
        check_header(frame.header)?;
        let mut vid = [0u8; 4];
        vid.copy_from_slice(&frame.payload[0..4]);
        Ok(Self {
            target_vid: u32::from_le_bytes(vid),
            item_pos: crate::cg_item_move::CgItemPos::decode_at(&frame.payload, 4),
            item_count: frame.payload[7],
        })
    }
}

impl fmt::Display for CgItemGive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CgItemGive(target_vid: {}, item_pos={{ window_type: {}, cell: {} }}, item_count: {})",
            self.target_vid, self.item_pos.window_type, self.item_pos.cell, self.item_count
        )
    }
}

/// Every way decoding a fixed item-give request can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CgItemGiveError {
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
    /// The leading byte was not [`HEADER_CG_ITEM_GIVE`].
    InvalidHeader {
        /// The header byte that was actually present.
        actual: u8,
    },
}

impl fmt::Display for CgItemGiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "truncated item-give record: need {needed} bytes, have {available}"
                )
            }
            Self::LengthMismatch { expected, actual } => {
                write!(f, "item-give record must be {expected} bytes, got {actual}")
            }
            Self::InvalidHeader { actual } => {
                write!(f, "invalid item-give header {actual:#04x}, expected 0x53")
            }
        }
    }
}

impl std::error::Error for CgItemGiveError {}

fn check_exact(len: usize) -> Result<(), CgItemGiveError> {
    match len.cmp(&CG_ITEM_GIVE_WIRE_SIZE) {
        core::cmp::Ordering::Less => Err(CgItemGiveError::Truncated {
            needed: CG_ITEM_GIVE_WIRE_SIZE,
            available: len,
        }),
        core::cmp::Ordering::Greater => Err(CgItemGiveError::LengthMismatch {
            expected: CG_ITEM_GIVE_WIRE_SIZE,
            actual: len,
        }),
        core::cmp::Ordering::Equal => Ok(()),
    }
}

fn check_header(actual: u8) -> Result<(), CgItemGiveError> {
    if actual == HEADER_CG_ITEM_GIVE.value() {
        Ok(())
    } else {
        Err(CgItemGiveError::InvalidHeader { actual })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_item_move::{CgItemMove, CgItemPos};
    use crate::cg_item_use::CgItemUse;
    use crate::cg_item_use_to_item::CgItemUseToItem;
    use crate::cg_shoot::CgShoot;
    use crate::cg_wire::{resolve_client_frame_size, ClientFrameDecoder, ClientFrameSize};

    #[test]
    fn inventory_resolves_the_fixed_nine_byte_size() {
        assert_eq!(HEADER_CG_ITEM_GIVE.value(), 83);
        assert_eq!(CG_ITEM_POS_SIZE, 3);
        assert_eq!(CG_ITEM_GIVE_WIRE_SIZE, 9);
        assert_eq!(CG_ITEM_GIVE_PAYLOAD_SIZE, 8);
        assert_eq!(
            resolve_client_frame_size(83).unwrap(),
            ClientFrameSize::Fixed(9)
        );
    }

    #[test]
    fn golden_record_has_exact_source_offsets() {
        // header 83 at offset 0, the little-endian target identifier at
        // 1..5, the packed three-byte position at 5..8, and the raw count
        // byte at offset 8.
        let packet = CgItemGive::new(0x0a0b_0c0d, CgItemPos::new(0x01, 0x0203), 0x07);
        assert_eq!(
            packet.encode(),
            vec![0x53, 0x0d, 0x0c, 0x0b, 0x0a, 0x01, 0x03, 0x02, 0x07]
        );
        assert_eq!(
            packet.to_frame().payload,
            vec![0x0d, 0x0c, 0x0b, 0x0a, 0x01, 0x03, 0x02, 0x07]
        );
        assert_eq!(packet.to_frame().header, 0x53);
        assert_eq!(
            packet.to_string(),
            "CgItemGive(target_vid: 168496141, item_pos={ window_type: 1, cell: 515 }, item_count: 7)"
        );
        assert_eq!(CgItemGive::decode(&packet.encode()).unwrap(), packet);
        assert_eq!(
            CgItemGive::decode_frame(&packet.to_frame()).unwrap(),
            packet
        );
    }

    #[test]
    fn the_packed_position_offsets_match_every_other_cg_item_pos_consumer() {
        // The four Rust codecs that reuse CgItemPos must all place the same
        // packed three-byte position identically. Offsets below are WIRE
        // offsets, so offset 0 is the header byte and the position starts
        // at 1 for the single-position records. Item-give places it after
        // the four-byte target identifier, so it must start at 5.
        let pos = CgItemPos::new(0x01, 0x0203);
        let give = CgItemGive::new(0, pos, 0).encode();
        let give_payload = CgItemGive::new(0, pos, 0).to_frame().payload;

        assert_eq!(&give[5..8], &[0x01, 0x03, 0x02]);
        assert_eq!(&give_payload[4..7], &[0x01, 0x03, 0x02]);

        // The same three bytes, at the same relative offsets, in the
        // three already-receipted records.
        assert_eq!(&CgItemUse::new(pos).encode()[1..4], &[0x01, 0x03, 0x02]);
        assert_eq!(
            &CgItemMove::new(pos, CgItemPos::new(0, 0), 0).encode()[1..4],
            &[0x01, 0x03, 0x02]
        );
        let u2i = CgItemUseToItem::new(pos, pos).encode();
        assert_eq!(&u2i[1..4], &[0x01, 0x03, 0x02]);
        assert_eq!(&u2i[4..7], &[0x01, 0x03, 0x02]);
    }

    #[test]
    fn every_raw_value_round_trips_unchanged() {
        // The target identifier is opaque, so every boundary word must
        // survive: zero, one, the high bit, and u32::MAX.
        for vid in [0u32, 1, 0x8000_0000, 0xffff_ffff, 0x0a0b_0c0d] {
            // The count is a dead wire field narrowed from a signed int on
            // the client, so 0 and 0xff are both ordinary values.
            for count in [0u8, 1, 0x7f, 0x80, 0xff] {
                // The window byte carries an EWindows value whose numbering
                // is build-dependent, so unknown windows must round-trip.
                for window in [0u8, 1, 0x34, 0xff] {
                    // 0xffff is the client NPOS-equivalent cell and must
                    // survive. The source and target are the same field
                    // here, so no equality constraint exists.
                    for cell in [0u16, 1, 0x8000, 0xfffe, 0xffff] {
                        let packet = CgItemGive::new(vid, CgItemPos::new(window, cell), count);
                        let raw = packet.encode();
                        assert_eq!(raw.len(), CG_ITEM_GIVE_WIRE_SIZE);
                        assert_eq!(raw[0], 0x53);
                        assert_eq!(CgItemGive::decode(&raw).unwrap(), packet);
                        let frame = packet.to_frame();
                        assert_eq!(frame.payload.len(), CG_ITEM_GIVE_PAYLOAD_SIZE);
                        assert_eq!(CgItemGive::decode_frame(&frame).unwrap(), packet);
                    }
                }
            }
        }
    }

    #[test]
    fn every_raw_length_is_rejected_before_the_header() {
        // Every wrong length is paired with several wrong headers, so a
        // decoder that validated the header first would fail these cases.
        let wrong_headers = [0x00u8, 0x0b, 0x0d, 0x52, 0x54];
        for header in wrong_headers {
            for len in 0..CG_ITEM_GIVE_WIRE_SIZE {
                let mut bytes = vec![header; len];
                assert_eq!(
                    CgItemGive::decode(&bytes),
                    Err(CgItemGiveError::Truncated {
                        needed: CG_ITEM_GIVE_WIRE_SIZE,
                        available: len,
                    }),
                    "len {len} with header {header:#04x} must report Truncated"
                );
                bytes.clear();
            }
            for extra in 1..=4usize {
                let bytes = vec![header; CG_ITEM_GIVE_WIRE_SIZE + extra];
                assert_eq!(
                    CgItemGive::decode(&bytes),
                    Err(CgItemGiveError::LengthMismatch {
                        expected: CG_ITEM_GIVE_WIRE_SIZE,
                        actual: CG_ITEM_GIVE_WIRE_SIZE + extra,
                    }),
                    "len {} with header {header:#04x} must report LengthMismatch",
                    CG_ITEM_GIVE_WIRE_SIZE + extra
                );
            }
        }
        // A correct length with a wrong header is only a header problem.
        for header in 0u8..=255 {
            let bytes = vec![header; CG_ITEM_GIVE_WIRE_SIZE];
            let expected = if header == 0x53 {
                // Every byte is the header value, so every field is 0x53.
                Ok(CgItemGive::new(
                    0x5353_5353,
                    CgItemPos::new(0x53, 0x5353),
                    0x53,
                ))
            } else {
                Err(CgItemGiveError::InvalidHeader { actual: header })
            };
            assert_eq!(CgItemGive::decode(&bytes), expected);
        }
    }

    #[test]
    fn every_framed_payload_length_and_header_is_checked() {
        let wrong_headers = [0x00u8, 0x0b, 0x0d, 0x52, 0x54];
        for header in wrong_headers {
            for len in 0..CG_ITEM_GIVE_PAYLOAD_SIZE {
                let frame = ClientFrame::new(header, vec![0u8; len]);
                assert_eq!(
                    CgItemGive::decode_frame(&frame),
                    Err(CgItemGiveError::Truncated {
                        needed: CG_ITEM_GIVE_WIRE_SIZE,
                        available: len + 1,
                    }),
                    "payload {len} with header {header:#04x} must report Truncated"
                );
            }
            for extra in 1..=4usize {
                let frame = ClientFrame::new(header, vec![0u8; CG_ITEM_GIVE_PAYLOAD_SIZE + extra]);
                assert_eq!(
                    CgItemGive::decode_frame(&frame),
                    Err(CgItemGiveError::LengthMismatch {
                        expected: CG_ITEM_GIVE_WIRE_SIZE,
                        actual: CG_ITEM_GIVE_PAYLOAD_SIZE + extra + 1,
                    }),
                    "payload {} with header {header:#04x} must report LengthMismatch",
                    CG_ITEM_GIVE_PAYLOAD_SIZE + extra
                );
            }
        }

        // Every header value at the *correct* payload length, so the framed
        // header check is actually reached. Without this block the framed
        // path never reports InvalidHeader, because every other framed case
        // pairs a wrong header with a wrong length and the length branch
        // always wins.
        for header in 0u8..=u8::MAX {
            if header == 0x53 {
                continue;
            }
            let frame = ClientFrame::new(header, vec![0u8; CG_ITEM_GIVE_PAYLOAD_SIZE]);
            assert_eq!(
                CgItemGive::decode_frame(&frame),
                Err(CgItemGiveError::InvalidHeader { actual: header }),
                "framed header {header:#04x} at the correct payload length must report InvalidHeader"
            );
        }
        let ok = ClientFrame::new(0x53, vec![0u8; CG_ITEM_GIVE_PAYLOAD_SIZE]);
        assert!(CgItemGive::decode_frame(&ok).is_ok());
    }

    #[test]
    fn fragmented_and_coalesced_frames_stream_through_the_shared_decoder() {
        let give = CgItemGive::new(0x1122_3344, CgItemPos::new(0x22, 0x3344), 0x55);
        let shoot = CgShoot::new(0x01);
        let mut wire = Vec::new();
        wire.extend_from_slice(&give.encode());
        wire.extend_from_slice(&shoot.encode());
        wire.extend_from_slice(&give.encode());

        let mut decoder = ClientFrameDecoder::new();

        // Drive the decoder from the slice and assert the incomplete
        // state at every byte boundary. Without these intermediate
        // assertions a single bulk feed would pass this test unchanged, so
        // the test would not actually prove that the decoder retains a
        // partial tail.
        let mut fed = 0usize;
        while fed + 1 < CG_ITEM_GIVE_WIRE_SIZE {
            fed += 1;
            decoder.feed(&wire[fed - 1..fed]).unwrap();
            assert!(
                decoder.try_decode().unwrap().is_none(),
                "{fed} of {CG_ITEM_GIVE_WIRE_SIZE} bytes must not yield a frame"
            );
            assert_eq!(decoder.buffered_len(), fed);
            assert_eq!(decoder.peek_header(), Some(0x53));
        }
        decoder.feed(&wire[fed..]).unwrap();
        assert_eq!(decoder.buffered_len(), wire.len());

        let first = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgItemGive::decode_frame(&first).unwrap(), give);
        let second = decoder.try_decode().unwrap().unwrap();
        assert_eq!(second.header, shoot.to_frame().header);
        let third = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgItemGive::decode_frame(&third).unwrap(), give);
        assert!(decoder.is_empty());
        assert_eq!(decoder.buffered_len(), 0);
        assert!(decoder.try_decode().unwrap().is_none());

        // Feeding the same bytes in one call must yield identical results.
        let mut bulk = ClientFrameDecoder::new();
        let _ = bulk.feed(&wire);
        let first = bulk.try_decode().unwrap().unwrap();
        assert_eq!(CgItemGive::decode_frame(&first).unwrap(), give);
        let second = bulk.try_decode().unwrap().unwrap();
        assert_eq!(second.header, shoot.to_frame().header);
        let third = bulk.try_decode().unwrap().unwrap();
        assert_eq!(CgItemGive::decode_frame(&third).unwrap(), give);
        assert!(bulk.is_empty());
        assert_eq!(bulk.buffered_len(), 0);
    }

    #[test]
    fn error_display_names_the_expected_sizes_and_header() {
        assert_eq!(
            CgItemGiveError::Truncated {
                needed: 9,
                available: 4
            }
            .to_string(),
            "truncated item-give record: need 9 bytes, have 4"
        );
        assert_eq!(
            CgItemGiveError::LengthMismatch {
                expected: 9,
                actual: 11
            }
            .to_string(),
            "item-give record must be 9 bytes, got 11"
        );
        assert_eq!(
            CgItemGiveError::InvalidHeader { actual: 0x11 }.to_string(),
            "invalid item-give header 0x11, expected 0x53"
        );
    }
}
