//! `HEADER_GC_CL` (234): the changelook (Transmutation) window's answers.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:2844-2853` declares the record every changelook
//! send writes:
//!
//! ```ignore
//! typedef struct SPacketChangeLook
//! {
//!     BYTE    header;
//!     BYTE    subheader;
//!     DWORD   dwCost;
//!     BYTE    bPos;
//!     TItemPos    tPos;
//! } TPacketChangeLook;
//! ```
//!
//! The subheaders are the enum at `packet.h:2836-2843`: `CL_SUBHEADER_OPEN` (0),
//! `CLOSE` (1), `ADD` (2), `REMOVE` (3) and `REFINE` (4). The struct is
//! bidirectional: the client sends the same bytes back with `HEADER_CG_CL`
//! (233).
//!
//! `CHARACTER::ChangeLookWindow(true, _)` (`server/server/game/char.cpp:10504-10535`)
//! sends the open: `subheader = CL_SUBHEADER_OPEN`, `dwCost =
//! CL_TRANSMUTATION_PRICE` (50,000,000, `C/length.h:180`), `bPos` 0, and
//! `tPos = { INVENTORY, 0 }`. The guards chat instead of sending: opening the
//! already-open window chats `[Transmutation] The window is already opened`,
//! and closing the not-open window chats `[Transmutation] The window is not
//! opened` (unless the close was requested). `LC_TEXT` of an unknown key is the
//! key itself (`G/locale.cpp:71-82`, `LOCALE_YMIR` arm), so the client hears the
//! raw keys.
//!
//! # Width
//!
//! `TItemPos` is 3 bytes (`BYTE` then `WORD` under `#pragma pack(1)`), so the
//! record is 1 + 1 + 4 + 1 + 3 = 10 bytes. The subheader domain is 0..=4, but
//! all 256 values encode: the client decodes what the server sends.

use std::fmt;

use crate::cg_sash::ItemPos;
use crate::gc_inventory::HEADER_GC_CL;

/// `sizeof(TPacketChangeLook)`: the whole record, header byte included.
pub const GC_CL_WIRE_SIZE: usize = 10;

/// `INVENTORY` in `EWindows` (`C/length.h:657-678`): `RESERVED_WINDOW` is 0,
/// so the inventory the open record's `tPos` names is 1.
pub const INVENTORY_WINDOW: u8 = 1;

/// `CL_TRANSMUTATION_PRICE` (`C/length.h:180`): the open record's cost.
pub const CL_TRANSMUTATION_PRICE: u32 = 50_000_000;

/// `CL_SUBHEADER_OPEN`: the window opens.
pub const CL_SUBHEADER_OPEN: u8 = 0;
/// `CL_SUBHEADER_CLOSE`: the window closes.
pub const CL_SUBHEADER_CLOSE: u8 = 1;
/// `CL_SUBHEADER_ADD`: a material lands.
pub const CL_SUBHEADER_ADD: u8 = 2;
/// `CL_SUBHEADER_REMOVE`: a material leaves.
pub const CL_SUBHEADER_REMOVE: u8 = 3;
/// `CL_SUBHEADER_REFINE`: the refine answer.
pub const CL_SUBHEADER_REFINE: u8 = 4;

/// Every way the changelook encoder can refuse its inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcClError {
    /// The output buffer is shorter than the fixed record.
    Truncated {
        /// The fixed width the encoder requires.
        needed: usize,
        /// How many bytes were actually offered.
        available: usize,
    },
}

impl fmt::Display for GcClError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(f, "GC record needs {needed} bytes, got {available}")
            }
        }
    }
}

impl std::error::Error for GcClError {}

fn write_u32_le(out: &mut [u8], at: usize, value: u32) -> Result<(), GcClError> {
    let end = at + 4;
    if out.len() < end {
        return Err(GcClError::Truncated {
            needed: end,
            available: out.len(),
        });
    }
    out[at..end].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

/// The server-to-client changelook record.
///
/// Field order is `subheader: u8` at offset 1, `cost: u32` at offset 2,
/// `pos: u8` at offset 6, and `item_pos: ItemPos` at offset 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GcChangeLook {
    /// The sub-command. The domain is 0..=4; every value encodes.
    pub subheader: u8,
    /// The transmutation cost.
    pub cost: u32,
    /// The material slot.
    pub pos: u8,
    /// The material position.
    pub item_pos: ItemPos,
}

impl GcChangeLook {
    /// Build the record.
    pub const fn new(subheader: u8, cost: u32, pos: u8, item_pos: ItemPos) -> Self {
        Self {
            subheader,
            cost,
            pos,
            item_pos,
        }
    }

    /// `CHARACTER::ChangeLookWindow`'s open record: subheader `OPEN`, the
    /// 50,000,000 cost, position 0, and `tPos = { INVENTORY, 0 }`.
    #[must_use]
    pub fn open() -> Self {
        Self::new(
            CL_SUBHEADER_OPEN,
            CL_TRANSMUTATION_PRICE,
            0,
            ItemPos::new(INVENTORY_WINDOW, 0),
        )
    }

    /// Encode to a fresh buffer.
    ///
    /// # Panics
    ///
    /// Panics only if the fresh buffer is somehow short, which cannot happen.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![0; GC_CL_WIRE_SIZE];
        self.encode_into(&mut out)
            .expect("a fresh 10-byte buffer holds the record");
        out
    }

    /// Encode into `out`.
    ///
    /// # Errors
    ///
    /// Returns [`GcClError::Truncated`] when `out` is shorter than
    /// [`GC_CL_WIRE_SIZE`].
    pub fn encode_into(&self, out: &mut [u8]) -> Result<(), GcClError> {
        if out.len() < GC_CL_WIRE_SIZE {
            return Err(GcClError::Truncated {
                needed: GC_CL_WIRE_SIZE,
                available: out.len(),
            });
        }
        out[0] = HEADER_GC_CL.value();
        out[1] = self.subheader;
        write_u32_le(out, 2, self.cost)?;
        out[6] = self.pos;
        out[7] = self.item_pos.window_type;
        out[8..10].copy_from_slice(&self.item_pos.cell.to_le_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_open_record_pins_the_legacy_bytes() {
        // `ChangeLookWindow(true)` with `tPos = { INVENTORY, 0 }`: header 234,
        // subheader OPEN, cost 50,000,000 (0x02FAF080), pos 0, window 1, cell 0.
        let record = GcChangeLook::open();
        assert_eq!(
            record.encode(),
            vec![234, 0, 0x80, 0xF0, 0xFA, 0x02, 0, 1, 0, 0]
        );
    }

    #[test]
    fn fields_encode_in_source_order() {
        // Distinct values per field: the subheader, the cost's four distinct
        // bytes, the pos, and the window and cell with distinct halves.
        let record = GcChangeLook::new(3, 0x0403_0201, 5, ItemPos::new(7, 0x0908));
        assert_eq!(record.encode(), vec![234, 3, 1, 2, 3, 4, 5, 7, 8, 9]);
    }

    #[test]
    fn a_short_buffer_names_its_widths() {
        let record = GcChangeLook::open();
        let mut out = vec![0; GC_CL_WIRE_SIZE - 1];
        assert_eq!(
            record.encode_into(&mut out),
            Err(GcClError::Truncated {
                needed: GC_CL_WIRE_SIZE,
                available: GC_CL_WIRE_SIZE - 1,
            })
        );
    }
}
