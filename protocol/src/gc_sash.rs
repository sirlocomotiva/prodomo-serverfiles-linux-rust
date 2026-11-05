//! `HEADER_GC_SASH` (231): the sash window's answers.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:2819-2831` declares the record every sash send
//! writes:
//!
//! ```ignore
//! typedef struct SPacketSash
//! {
//!     BYTE    header;
//!     BYTE    subheader;
//!     bool    bWindow;
//!     DWORD    dwPrice;
//!     BYTE    bPos;
//!     TItemPos    tPos;
//!     DWORD    dwItemVnum;
//!     DWORD    dwMinAbs;
//!     DWORD    dwMaxAbs;
//! } TPacketSash;
//! ```
//!
//! The subheaders are the enum at `packet.h:2804-2818`, whose GC half runs 0..=4
//! (`SASH_SUBHEADER_GC_OPEN`, `GC_CLOSE`, `GC_ADDED`, `GC_REMOVED`, and the
//! misleadingly named `SASH_SUBHEADER_CG_REFINED`, which continues the GC
//! numbering and is only ever sent server-to-client; see
//! [`crate::cg_sash`]). The struct is bidirectional: the client sends the same
//! 23 bytes back with `HEADER_CG_SASH` (230) and the CG subheaders 0..=3.
//!
//! `CHARACTER::OpenSash` (`server/server/game/char.cpp:9888-9934`) sends the
//! open: `subheader = SASH_SUBHEADER_GC_OPEN`, `bWindow` the combination flag,
//! `dwPrice`, `bPos`, `dwItemVnum`, `dwMinAbs` and `dwMaxAbs` all 0, and
//! `tPos = { INVENTORY, 0 }`. The guards chat instead of sending: opening the
//! already-open window chats `SASH_OPEN`, and opening one window while the other
//! stands chats `SASH_CLOSE` (for combination) or `COMBINE_CLOSE` (for
//! absorption). `LC_TEXT` of an unknown key is the key itself
//! (`G/locale.cpp:71-82`, `LOCALE_YMIR` arm), so the client hears the raw keys.
//!
//! # Width
//!
//! `TItemPos` is 3 bytes (`BYTE` then `WORD` under `#pragma pack(1)`), so the
//! record is 1 + 1 + 1 + 4 + 1 + 3 + 4 + 4 + 4 = 23 bytes, matching the
//! `sizeof(TPacketSash)` the CG codec pins. The GC subheader domain is 0..=4,
//! but all 256 values encode: the client decodes what the server sends.

use std::fmt;

use crate::cg_sash::ItemPos;
use crate::gc_inventory::HEADER_GC_SASH;

/// `sizeof(TPacketSash)`: the whole record, header byte included.
pub const GC_SASH_WIRE_SIZE: usize = 23;

/// `INVENTORY` in `EWindows` (`C/length.h:657-678`): `RESERVED_WINDOW` is 0,
/// so the inventory the open record's `tPos` names is 1.
pub const INVENTORY_WINDOW: u8 = 1;

/// `SASH_SUBHEADER_GC_OPEN`: the window opens.
pub const SASH_SUBHEADER_GC_OPEN: u8 = 0;
/// `SASH_SUBHEADER_GC_CLOSE`: the window closes.
pub const SASH_SUBHEADER_GC_CLOSE: u8 = 1;
/// `SASH_SUBHEADER_GC_ADDED`: a material lands.
pub const SASH_SUBHEADER_GC_ADDED: u8 = 2;
/// `SASH_SUBHEADER_GC_REMOVED`: a material leaves.
pub const SASH_SUBHEADER_GC_REMOVED: u8 = 3;
/// `SASH_SUBHEADER_CG_REFINED`: the refine answer (a GC value with a `CG_` name).
pub const SASH_SUBHEADER_GC_REFINED: u8 = 4;

/// Every way the sash encoder can refuse its inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcSashError {
    /// The output buffer is shorter than the fixed record.
    Truncated {
        /// The fixed width the encoder requires.
        needed: usize,
        /// How many bytes were actually offered.
        available: usize,
    },
}

impl fmt::Display for GcSashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(f, "GC record needs {needed} bytes, got {available}")
            }
        }
    }
}

impl std::error::Error for GcSashError {}

fn write_u32_le(out: &mut [u8], at: usize, value: u32) -> Result<(), GcSashError> {
    let end = at + 4;
    if out.len() < end {
        return Err(GcSashError::Truncated {
            needed: end,
            available: out.len(),
        });
    }
    out[at..end].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

/// The server-to-client sash record.
///
/// Field order is `subheader: u8` at offset 1, `window: bool` at offset 2,
/// `price: u32` at offset 3, `pos: u8` at offset 7, `item_pos: ItemPos` at
/// offset 8, `item_vnum: u32` at offset 11, `min_absorb: u32` at offset 15, and
/// `max_absorb: u32` at offset 19.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GcSash {
    /// The sub-command. The GC domain is 0..=4; every value encodes.
    pub subheader: u8,
    /// The top-level sash-window flag: combination (`true`) or absorption.
    pub window: bool,
    /// The sash price.
    pub price: u32,
    /// The material slot.
    pub pos: u8,
    /// The material position.
    pub item_pos: ItemPos,
    /// The item vnum.
    pub item_vnum: u32,
    /// The minimum absorb percentage.
    pub min_absorb: u32,
    /// The maximum absorb percentage.
    pub max_absorb: u32,
}

impl GcSash {
    /// Build the record.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        subheader: u8,
        window: bool,
        price: u32,
        pos: u8,
        item_pos: ItemPos,
        item_vnum: u32,
        min_absorb: u32,
        max_absorb: u32,
    ) -> Self {
        Self {
            subheader,
            window,
            price,
            pos,
            item_pos,
            item_vnum,
            min_absorb,
            max_absorb,
        }
    }

    /// `CHARACTER::OpenSash`'s open record: subheader `OPEN`, the combination
    /// flag, every number 0, and `tPos = { INVENTORY, 0 }`.
    #[must_use]
    pub fn open(combination: bool) -> Self {
        Self::new(
            SASH_SUBHEADER_GC_OPEN,
            combination,
            0,
            0,
            ItemPos::new(INVENTORY_WINDOW, 0),
            0,
            0,
            0,
        )
    }

    /// Encode to a fresh buffer.
    ///
    /// # Panics
    ///
    /// Panics only if the fresh buffer is somehow short, which cannot happen.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![0; GC_SASH_WIRE_SIZE];
        self.encode_into(&mut out)
            .expect("a fresh 23-byte buffer holds the record");
        out
    }

    /// Encode into `out`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSashError::Truncated`] when `out` is shorter than
    /// [`GC_SASH_WIRE_SIZE`].
    pub fn encode_into(&self, out: &mut [u8]) -> Result<(), GcSashError> {
        if out.len() < GC_SASH_WIRE_SIZE {
            return Err(GcSashError::Truncated {
                needed: GC_SASH_WIRE_SIZE,
                available: out.len(),
            });
        }
        out[0] = HEADER_GC_SASH.value();
        out[1] = self.subheader;
        out[2] = u8::from(self.window);
        write_u32_le(out, 3, self.price)?;
        out[7] = self.pos;
        out[8] = self.item_pos.window_type;
        out[9..11].copy_from_slice(&self.item_pos.cell.to_le_bytes());
        write_u32_le(out, 11, self.item_vnum)?;
        write_u32_le(out, 15, self.min_absorb)?;
        write_u32_le(out, 19, self.max_absorb)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_open_encodes_its_golden_bytes() {
        let record = GcSash::open(true);
        let bytes = record.encode();
        assert_eq!(bytes.len(), GC_SASH_WIRE_SIZE);
        // Header 231, subheader OPEN, window true, price 0, pos 0,
        // tPos {INVENTORY = 1, 0}, vnum/min/max 0.
        let mut golden = vec![0xe7, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0];
        golden.extend_from_slice(&[0; 12]);
        assert_eq!(bytes, golden);
        let absorb = GcSash::open(false);
        assert_eq!(absorb.encode()[2], 0, "absorption clears the window byte");
    }

    #[test]
    fn the_fields_land_at_their_offsets() {
        let record = GcSash::new(
            SASH_SUBHEADER_GC_ADDED,
            true,
            0x0403_0201,
            0x05,
            ItemPos::new(0x06, 0x0807),
            0x0c0b_0a09,
            0x100f_0e0d,
            0x1413_1211,
        );
        let bytes = record.encode();
        assert_eq!(bytes[0], 0xe7);
        assert_eq!(bytes[1], SASH_SUBHEADER_GC_ADDED);
        assert_eq!(bytes[2], 1);
        assert_eq!(&bytes[3..7], &[1, 2, 3, 4]);
        assert_eq!(bytes[7], 5);
        assert_eq!(&bytes[8..11], &[6, 7, 8]);
        assert_eq!(&bytes[11..15], &[9, 10, 11, 12]);
        assert_eq!(&bytes[15..19], &[13, 14, 15, 16]);
        assert_eq!(&bytes[19..23], &[17, 18, 19, 20]);
    }

    #[test]
    fn a_short_buffer_is_an_error_naming_both_widths() {
        let record = GcSash::open(true);
        let mut out = vec![0; GC_SASH_WIRE_SIZE - 1];
        assert_eq!(
            record.encode_into(&mut out),
            Err(GcSashError::Truncated {
                needed: GC_SASH_WIRE_SIZE,
                available: GC_SASH_WIRE_SIZE - 1,
            })
        );
    }
}
