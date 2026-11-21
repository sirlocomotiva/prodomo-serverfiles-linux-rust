//! Quickslots between the store, the world and the client (ledger 219).
//!
//! The load sets each stored slot the way `CInputDB::PlayerLoad` does
//! (`G/input_db.cpp:439-440`), and the loading burst sends what that set. The three client
//! requests (`CG_QUICKSLOT_ADD` 16, `CG_QUICKSLOT_DEL` 17 and `CG_QUICKSLOT_SWAP` 18) run on the
//! game thread, which holds the slots, and the save writes the slots the last answer left.

use db::quickslots::StoredQuickslot;
use protocol::gc_small::HEADER_GC_QUICKSLOT_DEL;
use protocol::gc_small::{GcHeaderAndByte, GcQuickSlot, GcQuickSlotAdd, GcQuickSlotSwap};
use world::character::{Quickslot, QuickslotRecord, Quickslots};

/// One client request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickslotStep {
    /// `CInputMain::QuickslotAdd` (`G/input_main.cpp:1083-1113`).
    Add {
        /// The slot's index.
        slot: u8,
        /// What the client asks it to hold.
        quickslot: Quickslot,
    },
    /// `CInputMain::QuickslotDelete` (`G/input_main.cpp:1116-1120`).
    Del {
        /// The slot's index.
        slot: u8,
    },
    /// `CInputMain::QuickslotSwap` (`G/input_main.cpp:1122-1126`).
    Swap {
        /// The first slot.
        slot: u8,
        /// The second slot.
        with: u8,
    },
}

/// What the world answers a request with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickslotAnswer {
    /// The records for the character's own client, encoded, in order.
    pub records: Vec<Vec<u8>>,
    /// The slots after the request, which the save writes.
    pub quickslots: Quickslots,
}

/// Encode the records a quickslot change answered.
#[must_use]
pub fn encode(records: &[QuickslotRecord]) -> Vec<Vec<u8>> {
    records
        .iter()
        .map(|record| {
            let mut out = Vec::new();
            match *record {
                QuickslotRecord::Add { slot, quickslot } => GcQuickSlotAdd {
                    pos: slot,
                    slot: GcQuickSlot {
                        slot_type: quickslot.kind,
                        position: quickslot.pos,
                    },
                }
                .encode_into(&mut out),
                QuickslotRecord::Del { slot } => {
                    GcHeaderAndByte::new(HEADER_GC_QUICKSLOT_DEL, slot).encode_into(&mut out);
                }
                QuickslotRecord::Swap { slot, with } => GcQuickSlotSwap {
                    pos: slot,
                    change_pos: with,
                }
                .encode_into(&mut out),
            }
            out
        })
        .collect()
}

/// The load's `SetQuickslot` of every stored slot, in slot order: the slots it left and the
/// records it sent, which open the loading burst's second half.
///
/// A stored slot `SetQuickslot` refuses is left empty and sends nothing, and the next save
/// drops it. An item slot is not checked against the items, which legacy has not loaded yet.
#[must_use]
pub fn load(stored: &[StoredQuickslot]) -> (Quickslots, Vec<Vec<u8>>) {
    let mut slots = Quickslots::default();
    let mut records = Vec::new();
    for row in stored {
        let quickslot = Quickslot {
            kind: row.kind,
            pos: row.pos,
        };
        let _set = slots.set(row.slot, quickslot, &mut records);
    }
    (slots, encode(&records))
}

/// The slots as the store keeps them: one row per slot that names something.
#[must_use]
pub fn stored(quickslots: &Quickslots) -> Vec<StoredQuickslot> {
    quickslots
        .set_slots()
        .map(|(slot, quickslot)| StoredQuickslot {
            slot,
            kind: quickslot.kind,
            pos: quickslot.pos,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(slot: u8, kind: u8, pos: u8) -> StoredQuickslot {
        StoredQuickslot { slot, kind, pos }
    }

    #[test]
    fn each_record_has_its_legacy_bytes() {
        let records = [
            QuickslotRecord::Add {
                slot: 5,
                quickslot: Quickslot { kind: 1, pos: 12 },
            },
            QuickslotRecord::Del { slot: 7 },
            QuickslotRecord::Swap { slot: 3, with: 35 },
        ];
        assert_eq!(
            encode(&records),
            vec![vec![28, 5, 1, 12], vec![29, 7], vec![30, 3, 35]]
        );
    }

    #[test]
    fn the_load_sets_each_stored_slot_in_order_and_drops_what_it_refuses() {
        let (slots, records) = load(&[row(0, 2, 9), row(1, 1, 200), row(4, 1, 3), row(6, 2, 9)]);
        // The cell 200 is worn, not carried, so it is refused without a record, and the second
        // copy of skill 9 empties the first.
        assert_eq!(
            records,
            vec![
                vec![28, 0, 2, 9],
                vec![28, 4, 1, 3],
                vec![29, 0],
                vec![28, 6, 2, 9],
            ]
        );
        assert_eq!(stored(&slots), vec![row(4, 1, 3), row(6, 2, 9)]);
        assert_eq!(load(&[]), (Quickslots::default(), Vec::new()));
    }
}
