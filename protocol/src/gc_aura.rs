//! `HEADER_GC_AURA` (214): the aura refine window's answers.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:3049-3055` declares the head every aura send writes:
//!
//! ```ignore
//! typedef struct SPacketGCAura
//! {
//!     SPacketGCAura() : bHeader(HEADER_GC_AURA) {}
//!     BYTE bHeader;
//!     WORD wSize;
//!     BYTE bSubHeader;
//! } TPacketGCAura;
//! ```
//!
//! The subheaders are the enum at `packet.h:2981-2989`: `AURA_SUBHEADER_GC_OPEN` (0),
//! `CLOSE` (1), `SET_ITEM` (2), `CLEAR_SLOT` (3), `CLEAR_ALL` (4), `CLEAR_RIGHT` (5) and
//! `REFINE_INFO` (6). `CHARACTER::OpenAuraRefineWindow`
//! (`server/server/game/char_aura.cpp:14-71`) sends the open: `wSize` counts the whole
//! record, the subheader is `OPEN`, and the one body byte is the window type
//! (`EAuraWindowType`, `C/length.h:1087-1093`: absorb 0, growth 1, evolve 2).
//! `AURA_WINDOW_TYPE_MAX` (3) returns without sending.
//!
//! # Width
//!
//! The open is 1 + 2 + 1 + 1 = 5 bytes. Only the open is encoded; the other
//! subheaders wait for their systems.

/// `HEADER_GC_AURA`.
pub const HEADER: u8 = 214;

/// `AURA_SUBHEADER_GC_OPEN`.
pub const SUB_OPEN: u8 = 0;

/// `AURA_WINDOW_TYPE_ABSORB`.
pub const WINDOW_ABSORB: u8 = 0;

/// `AURA_WINDOW_TYPE_GROWTH`.
pub const WINDOW_GROWTH: u8 = 1;

/// `AURA_WINDOW_TYPE_EVOLVE`.
pub const WINDOW_EVOLVE: u8 = 2;

/// The open's width: the head and the one window-type byte.
pub const OPEN_LEN: usize = 5;

/// An aura window open: `OpenAuraRefineWindow`'s record for window `window`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcAuraOpen {
    /// The window type: 0 absorb, 1 growth, 2 evolve.
    pub window: u8,
}

impl GcAuraOpen {
    /// Encode the 5-byte open: the header, the size word counting the whole record,
    /// the `OPEN` subheader, and the window type.
    ///
    /// # Panics
    ///
    /// Never: the open is 5 bytes, so the size word fits.
    #[must_use]
    pub fn encode(self) -> [u8; OPEN_LEN] {
        let size = u16::try_from(OPEN_LEN).expect("the open is short");
        let [lo, hi] = size.to_le_bytes();
        [HEADER, lo, hi, SUB_OPEN, self.window]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The golden open: header 214, size 5, subheader 0, growth 1.
    #[test]
    fn the_open_counts_itself() {
        assert_eq!(
            GcAuraOpen {
                window: WINDOW_GROWTH
            }
            .encode(),
            [214, 5, 0, 0, 1]
        );
        assert_eq!(
            GcAuraOpen {
                window: WINDOW_EVOLVE
            }
            .encode()[4],
            2
        );
    }
}
