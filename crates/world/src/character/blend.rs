//! The blend item's world step: spending one plain use.
//!
//! The gates live on the caller ([`prodomo`](crate) reads the blend table and the
//! character's affects); the step only spends the item at `at` when it is still
//! the item `id`, as [`super::potion::use_one`] does for a potion.

use protocol::item_pos::ItemPos;

use super::equip::{use_source, Trail};
use super::item_move::{done, MoveDone, MoveKind, MoveRefused};
use super::items::CharacterItems;
use crate::item::ItemId;

/// Spend one of the blend item `id` at `at` (`item->SetCount(item->GetCount() - 1)`,
/// `G/char_item.cpp:7113`).
///
/// # Errors
///
/// [`MoveRefused`]: the cell holds no item, or another item stands where `id` did.
pub fn spend_blend(
    items: &mut CharacterItems,
    at: ItemPos,
    id: ItemId,
) -> Result<MoveDone, MoveRefused> {
    let item = use_source(items, at)?;
    if item.id != id {
        return Err(MoveRefused::Empty);
    }
    let mut trail = Trail::default();
    super::potion::use_one(items, &item, false, &mut trail)?;
    Ok(done(MoveKind::Used, trail))
}
