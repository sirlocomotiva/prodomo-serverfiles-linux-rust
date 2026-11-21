//! The refine window's record: `TPacketGCRefineInformation`, which
//! `CHARACTER::RefineInformation` sends (`server/server/game/char_item.cpp:1865-1962`).
//!
//! ```cpp
//! typedef struct SPacketGCRefineInformaion
//! {
//!     BYTE    header;
//!     BYTE    type;
//!     BYTE    pos;
//!     DWORD   src_vnum;
//!     DWORD   result_vnum;
//!     BYTE    material_count;
//!     int     cost;
//!     int     prob;
//!     TRefineMaterial materials[REFINE_MATERIAL_MAX_NUM];
//! } TPacketGCRefineInformation;
//! ```
//!
//! (`server/server/game/packet.h:2413-2424`; `TRefineMaterial` is a `DWORD vnum` and an
//! `int count`, `server/server/common/tables.h:1209-1213`, and `REFINE_MATERIAL_MAX_NUM` is 5,
//! `server/server/common/item_length.h:38`.)
//!
//! # The header byte is 119, not 95
//!
//! The server's `HEADER_GC_REFINE_INFORMATION` is 119 (`packet.h:192`), which the client's
//! table registers as `HEADER_GC_REFINE_INFORMATION_NEW`; 95 is the server's
//! `HEADER_GC_REFINE_INFORMATION_OLD` (`packet.h:179`), which no send site writes. The
//! record's width is the server's struct's: 60 bytes, measured by compiling the verbatim body
//! under `#pragma pack(1)` with `g++ -m32 -fsyntax-only` `static_assert`s (`material_count`
//! at offset 11, `materials` at 20, and a packed two-`long` control of 8; a 61-byte assertion
//! failed as it should). `i686-linux-gnu-g++-12` is not installed on the machine that
//! measured it.

use std::error::Error;
use std::fmt;

use crate::gc_inventory::HEADER_GC_REFINE_INFORMATION_NEW;

/// `REFINE_MATERIAL_MAX_NUM`: the material slots of a recipe.
pub const REFINE_MATERIAL_SLOTS: usize = 5;

/// `sizeof(TPacketGCRefineInformation)`.
pub const GC_REFINE_INFORMATION_WIRE_SIZE: usize = 60;

/// One `TRefineMaterial`: a vnum and a count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RefineMaterial {
    /// `vnum`.
    pub vnum: u32,
    /// `count`.
    pub count: i32,
}

/// `TPacketGCRefineInformation`, sent under byte 119.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GcRefineInformation {
    /// `type`: the refine type the window asks with.
    pub refine_type: u8,
    /// `pos`: the inventory cell of the item.
    pub pos: u8,
    /// `src_vnum`: the item's vnum.
    pub src_vnum: u32,
    /// `result_vnum`: the vnum a success makes.
    pub result_vnum: u32,
    /// `material_count`: how many material slots the window lists.
    pub material_count: u8,
    /// `cost`: the gold a refine takes.
    pub cost: i32,
    /// `prob`: the success percent.
    pub prob: i32,
    /// `materials`: the recipe's slots.
    pub materials: [RefineMaterial; REFINE_MATERIAL_SLOTS],
}

/// Why a buffer is not a `GcRefineInformation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcRefineError {
    /// The buffer is shorter than the record.
    Truncated {
        /// The bytes the record needs.
        needed: usize,
        /// Bytes actually present.
        actual: usize,
    },
    /// The leading byte is not the record's header.
    Header {
        /// The record's header.
        expected: u8,
        /// The byte that was found.
        actual: u8,
    },
}

impl fmt::Display for GcRefineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, actual } => write!(
                formatter,
                "a refine information record needs {needed} bytes, got {actual}"
            ),
            Self::Header { expected, actual } => write!(
                formatter,
                "expected refine information header {expected:#04x}, got byte {actual:#04x}"
            ),
        }
    }
}

impl Error for GcRefineError {}

/// The little-endian four bytes at `at`.
fn word(bytes: &[u8], at: usize) -> [u8; 4] {
    [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
}

impl GcRefineInformation {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_REFINE_INFORMATION_NEW.value()
    }

    /// Appends the whole packed record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.push(self.refine_type);
        out.push(self.pos);
        out.extend_from_slice(&self.src_vnum.to_le_bytes());
        out.extend_from_slice(&self.result_vnum.to_le_bytes());
        out.push(self.material_count);
        out.extend_from_slice(&self.cost.to_le_bytes());
        out.extend_from_slice(&self.prob.to_le_bytes());
        for material in &self.materials {
            out.extend_from_slice(&material.vnum.to_le_bytes());
            out.extend_from_slice(&material.count.to_le_bytes());
        }
    }

    /// Encodes to a fresh buffer.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_REFINE_INFORMATION_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Decodes the whole packed record from the start of `bytes`; trailing bytes are left
    /// alone.
    ///
    /// # Errors
    ///
    /// Returns [`GcRefineError::Truncated`] when the buffer is shorter than 60 bytes, and
    /// [`GcRefineError::Header`] when the leading byte is not 119.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcRefineError> {
        if bytes.len() < GC_REFINE_INFORMATION_WIRE_SIZE {
            return Err(GcRefineError::Truncated {
                needed: GC_REFINE_INFORMATION_WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header() {
            return Err(GcRefineError::Header {
                expected: Self::header(),
                actual: bytes[0],
            });
        }
        let mut materials = [RefineMaterial::default(); REFINE_MATERIAL_SLOTS];
        for (slot, material) in materials.iter_mut().enumerate() {
            let at = 20 + slot * 8;
            *material = RefineMaterial {
                vnum: u32::from_le_bytes(word(bytes, at)),
                count: i32::from_le_bytes(word(bytes, at + 4)),
            };
        }
        Ok(Self {
            refine_type: bytes[1],
            pos: bytes[2],
            src_vnum: u32::from_le_bytes(word(bytes, 3)),
            result_vnum: u32::from_le_bytes(word(bytes, 7)),
            material_count: bytes[11],
            cost: i32::from_le_bytes(word(bytes, 12)),
            prob: i32::from_le_bytes(word(bytes, 16)),
            materials,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> GcRefineInformation {
        GcRefineInformation {
            refine_type: 0x05,
            pos: 0x2a,
            src_vnum: 0x1122_3344,
            result_vnum: 0x5566_7788,
            material_count: 0x03,
            cost: -0x0102_0304,
            prob: 0x0a0b_0c0d,
            materials: [
                RefineMaterial {
                    vnum: 0x0100_0201,
                    count: 0x0300_0401,
                },
                RefineMaterial {
                    vnum: 0x0100_0202,
                    count: -2,
                },
                RefineMaterial::default(),
                RefineMaterial {
                    vnum: 0xa1b2_c3d4,
                    count: 0x7f00_0001,
                },
                RefineMaterial {
                    vnum: 0x0000_00ff,
                    count: 0,
                },
            ],
        }
    }

    #[test]
    fn the_record_is_sixty_bytes_in_the_servers_field_order() {
        let mut golden = vec![
            0x77, 0x05, 0x2a, 0x44, 0x33, 0x22, 0x11, 0x88, 0x77, 0x66, 0x55, 0x03, 0xfc, 0xfc,
            0xfd, 0xfe, 0x0d, 0x0c, 0x0b, 0x0a,
        ];
        golden.extend_from_slice(&[0x01, 0x02, 0x00, 0x01, 0x01, 0x04, 0x00, 0x03]);
        golden.extend_from_slice(&[0x02, 0x02, 0x00, 0x01, 0xfe, 0xff, 0xff, 0xff]);
        golden.extend_from_slice(&[0; 8]);
        golden.extend_from_slice(&[0xd4, 0xc3, 0xb2, 0xa1, 0x01, 0x00, 0x00, 0x7f]);
        golden.extend_from_slice(&[0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(golden.len(), GC_REFINE_INFORMATION_WIRE_SIZE);
        assert_eq!(sample().encode(), golden);
        assert_eq!(GcRefineInformation::decode(&golden), Ok(sample()));
    }

    #[test]
    fn a_short_or_foreign_buffer_is_refused() {
        let bytes = sample().encode();
        assert_eq!(
            GcRefineInformation::decode(&bytes[..59]),
            Err(GcRefineError::Truncated {
                needed: 60,
                actual: 59
            })
        );
        let mut foreign = bytes;
        foreign[0] = 0x5f;
        assert_eq!(
            GcRefineInformation::decode(&foreign),
            Err(GcRefineError::Header {
                expected: 0x77,
                actual: 0x5f
            })
        );
    }
}
