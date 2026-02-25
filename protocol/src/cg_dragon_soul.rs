//! Transport-free codec for the client-to-server dragon-soul refine request.
//!
//! | record | header | wire | payload | declaration | registration |
//! |---|---|---|---|---|---|
//! | `SPacketCGDragonSoulRefine` | 205 `0xcd` | 47 | 46 | `packet.h:2771-2777` | `packet_info.cpp:171` |
//!
//! # The width is the third independent proof that `TItemPos` is 3 bytes
//!
//! ```cpp
//! typedef struct SPacketCGDragonSoulRefine
//! {
//!     SPacketCGDragonSoulRefine() : header (HEADER_CG_DRAGON_SOUL_REFINE) {}
//!     BYTE header;
//!     BYTE bSubType;
//!     TItemPos ItemGrid[DRAGON_SOUL_REFINE_GRID_SIZE];
//! } SPacketCGDragonSoulRefine;
//! ```
//!
//! `DRAGON_SOUL_REFINE_GRID_SIZE` is 15 (`common/length.h:86`), so the record is
//! `1 + 1 + 15 * 3 = 47`, which is exactly what `packet_info.cpp:171` registers.
//! A 4-byte `TItemPos` would give 62. This record shares no field, no header and
//! no dispatch arm with `TPacketSash`, so its agreement with that record's 23-byte
//! registration is a genuine cross-check on the shared struct width rather than a
//! restatement of one constant.
//!
//! # The sub-header enum is shared with the GC direction, and the two trees
//! disagree on three of its names
//!
//! `EPacketCGDragonSoulSubHeaderType` at `packet.h:2755-2770` has thirteen
//! values, 0..=12. The client's identically named enum
//! (`client/Client/UserInterface/Packet.h:2817-2831`) assigns the **same numbers**
//! but names values 2, 3 and 4 differently:
//!
//! | value | server name | client name |
//! |---|---|---|
//! | 0 | `DS_SUB_HEADER_OPEN` | `DS_SUB_HEADER_OPEN` |
//! | 1 | `DS_SUB_HEADER_CLOSE` | `DS_SUB_HEADER_CLOSE` |
//! | 2 | `DS_SUB_HEADER_DO_REFINE_GRADE` | `DS_SUB_HEADER_DO_UPGRADE` |
//! | 3 | `DS_SUB_HEADER_DO_REFINE_STEP` | `DS_SUB_HEADER_DO_IMPROVEMENT` |
//! | 4 | `DS_SUB_HEADER_DO_REFINE_STRENGTH` | `DS_SUB_HEADER_DO_REFINE` |
//! | 5..=11 | `REFINE_FAIL` .. `REFINE_SUCCEED` | same names |
//! | 12 | `DS_SUB_HEADER_REFINE_ALL` | `DS_SUB_HEADER_REFINE_ALL` |
//!
//! **This is a naming divergence, not a value divergence**, and the difference
//! matters. Every byte on the wire is the same number in both trees, so unlike
//! `CG_CHANGE_LANGUAGE` (server 238, client 245) there is nothing to reproduce or
//! correct: a client that sends 2 means `DO_UPGRADE` to the client and
//! `DO_REFINE_GRADE` to the server, and both are the same action. The server
//! names are the oracle and are recorded here; no `DS_SUB_HEADER_*` constant is
//! re-exported, because a Rust name that silently picks one tree's vocabulary
//! would hide exactly this divergence.
//!
//! # Only six of the thirteen sub-headers are handled on input
//!
//! `CInputMain::Analyze` at `input_main.cpp:3885-3919` switches on `p->bSubType`
//! with cases `OPEN`, `CLOSE`, `DO_REFINE_GRADE`, `DO_REFINE_STEP`,
//! `DO_REFINE_STRENGTH` and `REFINE_ALL` -- and **no `default:` arm**. Values 5
//! through 11 are the `REFINE_FAIL` / `REFINE_SUCCEED` results, which are
//! server-to-client outcomes that happen to share the enum. They are silently
//! discarded on input.
//!
//! So this is a shared bidirectional enum in the same shape as the sash one, and
//! the same rule applies: the codec preserves all 256 values and names none of
//! them, because the handler's `switch` is policy above this boundary.
//!
//! # What the grid holds
//!
//! `ItemGrid` is fifteen [`ItemPos`] values. `DS_SUB_HEADER_REFINE_ALL` reads
//! only the first two, as raw `cell` words
//! (`DSManager::instance().DoAllRefineGrade(ch, p->ItemGrid[0].cell, p->ItemGrid[1].cell)`),
//! and the other five sub-commands pass the whole array to the manager. None of
//! that is framing. This codec does not range-check `cell`, does not validate
//! `window_type` against `SItemPos::IsValidItemPosition()`, and does not reject
//! the reserved 15-cell shape.

use crate::cg_inventory::{CgHeader, HEADER_CG_DRAGON_SOUL_REFINE};
use crate::cg_wire::ClientFrame;
use crate::item_pos::ItemPos;

/// `DRAGON_SOUL_REFINE_GRID_SIZE` from `common/length.h:86`.
pub const CG_DRAGON_SOUL_GRID_SIZE: usize = 15;

/// The full legacy `SPacketCGDragonSoulRefine` record, header byte included.
pub const CG_DRAGON_SOUL_REFINE_WIRE_SIZE: usize =
    1 + 1 + CG_DRAGON_SOUL_GRID_SIZE * ItemPos::WIRE_SIZE;
/// The framed payload of `SPacketCGDragonSoulRefine`, everything after the header.
pub const CG_DRAGON_SOUL_REFINE_PAYLOAD_SIZE: usize = CG_DRAGON_SOUL_REFINE_WIRE_SIZE - 1;

/// Every way the dragon-soul decoder can refuse a byte slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgDragonSoulError {
    /// Fewer bytes than the fixed record needs.
    Truncated {
        /// The fixed width the decoder requires.
        needed: usize,
        /// How many bytes were actually offered.
        available: usize,
    },
    /// A complete-length slice that is not the fixed width.
    LengthMismatch {
        /// The fixed width the decoder requires.
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

impl From<crate::item_pos::ItemPosError> for CgDragonSoulError {
    fn from(e: crate::item_pos::ItemPosError) -> Self {
        match e {
            crate::item_pos::ItemPosError::Truncated { needed, available } => {
                CgDragonSoulError::Truncated { needed, available }
            }
            crate::item_pos::ItemPosError::LengthMismatch { expected, actual } => {
                CgDragonSoulError::LengthMismatch { expected, actual }
            }
        }
    }
}

/// The client-to-server dragon-soul refine request.
///
/// Field order is `by_sub_header: u8` at offset 1, then fifteen [`ItemPos`]
/// values from offset 2 to offset 46.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgDragonSoulRefine {
    /// The sub-command. Only `0`, `1`, `2`, `3`, `4` and `12` are handled on
    /// input, but every `u8` is preserved because the legacy inner `switch` has
    /// no `default:` arm and discards the rest silently.
    pub by_sub_header: u8,
    /// The fifteen-slot material grid.
    pub grid: [ItemPos; CG_DRAGON_SOUL_GRID_SIZE],
}

impl CgDragonSoulRefine {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_DRAGON_SOUL_REFINE
    }

    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_DRAGON_SOUL_REFINE_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_DRAGON_SOUL_REFINE_PAYLOAD_SIZE;

    /// Build a request.
    pub fn new(by_sub_header: u8, grid: [ItemPos; CG_DRAGON_SOUL_GRID_SIZE]) -> Self {
        Self {
            by_sub_header,
            grid,
        }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.by_sub_header);
        for p in &self.grid {
            p.encode_into(out);
        }
    }

    /// Encode to a fresh 47-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode the header-less payload to a fresh 46-byte buffer.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame {
            header: Self::header().value(),
            payload: {
                let mut p = Vec::with_capacity(Self::PAYLOAD_SIZE);
                p.push(self.by_sub_header);
                for pos in &self.grid {
                    pos.encode_into(&mut p);
                }
                p
            },
        }
    }

    /// # Errors
    ///
    /// Returns [`CgDragonSoulError::Truncated`] for fewer than 47 bytes,
    /// [`CgDragonSoulError::LengthMismatch`] for more, and
    /// [`CgDragonSoulError::InvalidHeader`] for a full-length slice whose first
    /// byte is not 205.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgDragonSoulError> {
        check_exact(bytes.len(), Self::WIRE_SIZE)?;
        check_header(bytes[0], Self::header().value())?;
        let mut grid = [ItemPos::default(); CG_DRAGON_SOUL_GRID_SIZE];
        for (i, slot) in grid.iter_mut().enumerate() {
            let at = 2 + i * ItemPos::WIRE_SIZE;
            *slot = ItemPos::decode(&bytes[at..at + ItemPos::WIRE_SIZE])?;
        }
        Ok(Self {
            by_sub_header: bytes[1],
            grid,
        })
    }

    /// # Errors
    ///
    /// As [`CgDragonSoulRefine::decode`], except that the frame payload must be
    /// exactly 46 bytes and every offset shifts down by one.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgDragonSoulError> {
        check_exact(frame.payload.len(), Self::PAYLOAD_SIZE)?;
        check_header(frame.header, Self::header().value())?;
        let mut grid = [ItemPos::default(); CG_DRAGON_SOUL_GRID_SIZE];
        for (i, slot) in grid.iter_mut().enumerate() {
            let at = 1 + i * ItemPos::WIRE_SIZE;
            *slot = ItemPos::decode(&frame.payload[at..at + ItemPos::WIRE_SIZE])?;
        }
        Ok(Self {
            by_sub_header: frame.payload[0],
            grid,
        })
    }
}

/// Reject anything that is not exactly `expected` bytes wide.
fn check_exact(actual: usize, expected: usize) -> Result<(), CgDragonSoulError> {
    if actual < expected {
        return Err(CgDragonSoulError::Truncated {
            needed: expected,
            available: actual,
        });
    }
    if actual > expected {
        return Err(CgDragonSoulError::LengthMismatch { expected, actual });
    }
    Ok(())
}

/// Reject a header byte that is not `expected`.
fn check_header(actual: u8, expected: u8) -> Result<(), CgDragonSoulError> {
    if actual != expected {
        return Err(CgDragonSoulError::InvalidHeader { expected, actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DS: u8 = 205;

    fn sample_grid() -> [ItemPos; CG_DRAGON_SOUL_GRID_SIZE] {
        let mut g = [ItemPos::default(); CG_DRAGON_SOUL_GRID_SIZE];
        for (i, slot) in g.iter_mut().enumerate() {
            let w = u8::try_from(i).unwrap_or(0);
            *slot = ItemPos::new(w, u16::try_from(i * 7).unwrap_or(0));
        }
        g
    }

    fn sample() -> CgDragonSoulRefine {
        CgDragonSoulRefine::new(2, sample_grid())
    }

    #[test]
    fn the_width_matches_the_legacy_registration() {
        assert_eq!(CG_DRAGON_SOUL_GRID_SIZE, 15);
        assert_eq!(ItemPos::WIRE_SIZE, 3);
        assert_eq!(CgDragonSoulRefine::WIRE_SIZE, 47);
        assert_eq!(CgDragonSoulRefine::PAYLOAD_SIZE, 46);
        assert_eq!(1 + 1 + 15 * 3, 47);
    }

    #[test]
    fn the_header_is_205() {
        assert_eq!(CgDragonSoulRefine::header().value(), 205);
    }

    #[test]
    fn encodes_to_the_exact_legacy_bytes() {
        let rec = CgDragonSoulRefine::new(2, [ItemPos::new(1, 0x0304); 15]);
        let bytes = rec.encode();
        assert_eq!(bytes.len(), 47);
        assert_eq!(bytes[0], DS);
        assert_eq!(bytes[1], 2);
        // first slot: window 1, cell 0x0304 little-endian
        assert_eq!(&bytes[2..5], &[1, 0x04, 0x03]);
        // all fifteen slots are identical here
        for i in 0..15 {
            assert_eq!(&bytes[2 + i * 3..5 + i * 3], &[1, 0x04, 0x03]);
        }
    }

    #[test]
    fn decodes_the_exact_legacy_bytes() {
        let rec = sample();
        assert_eq!(CgDragonSoulRefine::decode(&rec.encode()).unwrap(), rec);
    }

    #[test]
    fn the_grid_slots_are_independent() {
        let mut g = sample_grid();
        g[7] = ItemPos::new(0xFE, 0xFFFF);
        let rec = CgDragonSoulRefine::new(0, g);
        let back = CgDragonSoulRefine::decode(&rec.encode()).unwrap();
        assert_eq!(back.grid[7], ItemPos::new(0xFE, 0xFFFF));
        assert_ne!(back.grid[6], back.grid[7]);
    }

    #[test]
    fn the_last_slot_ends_exactly_at_the_record_end() {
        let mut g = [ItemPos::default(); CG_DRAGON_SOUL_GRID_SIZE];
        g[14] = ItemPos::new(0xAB, 0xCDEF);
        let rec = CgDragonSoulRefine::new(0, g);
        let bytes = rec.encode();
        assert_eq!(&bytes[44..47], &[0xAB, 0xEF, 0xCD]);
        assert_eq!(bytes.len(), 47);
    }

    #[test]
    fn round_trips_through_a_frame() {
        let rec = sample();
        let frame = rec.to_frame();
        assert_eq!(frame.header, 205);
        assert_eq!(frame.payload.len(), 46);
        assert_eq!(frame.payload[0], rec.by_sub_header);
        assert_eq!(CgDragonSoulRefine::decode_frame(&frame).unwrap(), rec);
    }

    #[test]
    fn the_frame_payload_is_the_record_minus_the_header() {
        let rec = sample();
        let frame = rec.to_frame();
        assert_eq!(&frame.payload[..], &rec.encode()[1..]);
    }

    #[test]
    fn frame_and_slice_agree() {
        let rec = sample();
        assert_eq!(
            CgDragonSoulRefine::decode(&rec.encode()).unwrap(),
            CgDragonSoulRefine::decode_frame(&rec.to_frame()).unwrap()
        );
    }

    #[test]
    fn every_sub_header_value_round_trips() {
        for v in 0..=u8::MAX {
            let rec = CgDragonSoulRefine::new(v, [ItemPos::new(1, 1); 15]);
            assert_eq!(
                CgDragonSoulRefine::decode(&rec.encode())
                    .unwrap()
                    .by_sub_header,
                v
            );
            let f = rec.to_frame();
            assert_eq!(
                CgDragonSoulRefine::decode_frame(&f).unwrap().by_sub_header,
                v
            );
        }
    }

    #[test]
    fn the_six_handled_sub_headers_are_representable() {
        // OPEN, CLOSE, DO_REFINE_GRADE, DO_REFINE_STEP, DO_REFINE_STRENGTH,
        // REFINE_ALL. The seven GC-direction values 5..=11 are also
        // representable; the codec does not police either set.
        for v in [0_u8, 1, 2, 3, 4, 5, 11, 12] {
            let rec = CgDragonSoulRefine::new(v, [ItemPos::default(); 15]);
            assert_eq!(
                CgDragonSoulRefine::decode(&rec.encode())
                    .unwrap()
                    .by_sub_header,
                v
            );
        }
    }

    #[test]
    fn every_window_type_value_survives_in_every_slot() {
        for w in 0..=u8::MAX {
            let rec = CgDragonSoulRefine::new(0, [ItemPos::new(w, 0); 15]);
            let back = CgDragonSoulRefine::decode(&rec.encode()).unwrap();
            for slot in &back.grid {
                assert_eq!(slot.window_type, w);
            }
        }
    }

    #[test]
    fn every_cell_value_survives() {
        for c in [0_u16, 1, 0x00FF, 0x0100, 0x7FFF, 0x8000, 0xFFFF] {
            let rec = CgDragonSoulRefine::new(0, [ItemPos::new(0, c); 15]);
            assert_eq!(
                CgDragonSoulRefine::decode(&rec.encode()).unwrap().grid[14].cell,
                c
            );
        }
    }

    #[test]
    fn rejects_every_short_length() {
        for len in 0..CgDragonSoulRefine::WIRE_SIZE {
            let mut bytes = sample().encode();
            bytes.truncate(len);
            assert_eq!(
                CgDragonSoulRefine::decode(&bytes).unwrap_err(),
                CgDragonSoulError::Truncated {
                    needed: 47,
                    available: len
                },
                "length {len}"
            );
        }
    }

    #[test]
    fn rejects_a_foreign_header() {
        let mut bytes = sample().encode();
        bytes[0] = 204;
        assert_eq!(
            CgDragonSoulRefine::decode(&bytes).unwrap_err(),
            CgDragonSoulError::InvalidHeader {
                expected: 205,
                actual: 204
            }
        );
    }

    #[test]
    fn rejects_a_long_slice() {
        let mut bytes = sample().encode();
        bytes.push(0);
        assert_eq!(
            CgDragonSoulRefine::decode(&bytes).unwrap_err(),
            CgDragonSoulError::LengthMismatch {
                expected: 47,
                actual: 48
            }
        );
    }

    #[test]
    fn rejects_a_wrong_frame_header() {
        let mut f = sample().to_frame();
        f.header = 0;
        assert_eq!(
            CgDragonSoulRefine::decode_frame(&f).unwrap_err(),
            CgDragonSoulError::InvalidHeader {
                expected: 205,
                actual: 0
            }
        );
    }

    #[test]
    fn rejects_every_wrong_frame_length() {
        for len in 0..CgDragonSoulRefine::PAYLOAD_SIZE {
            let f = ClientFrame {
                header: DS,
                payload: vec![0; len],
            };
            assert!(CgDragonSoulRefine::decode_frame(&f).is_err(), "len {len}");
        }
    }

    #[test]
    fn encode_into_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE, 0xAD];
        sample().encode_into(&mut out);
        assert_eq!(out.len(), 2 + 47);
        assert_eq!(&out[2..], &sample().encode()[..]);
    }

    #[test]
    fn a_grid_of_zeros_is_not_the_same_as_a_missing_grid() {
        let zeros = CgDragonSoulRefine::new(0, [ItemPos::new(0, 0); 15]);
        let bytes = zeros.encode();
        assert_eq!(bytes.len(), 47);
        assert_eq!(&bytes[2..], &[0u8; 45]);
    }
}
