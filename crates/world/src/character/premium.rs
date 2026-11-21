//! The multi-farm premiums (`ENABLE_MULTI_FARM_BLOCK`, `G/char_item.cpp:3901-3923`): the
//! consume half of `CG_ITEM_USE` on vnum 55610-55615. The affect half runs in
//! `prodomo`'s `multi_farm`, beside the registry the use re-checks.

use protocol::item_pos::ItemPos;

use super::equip::{use_checks, use_source, Gear, Trail};
use super::item_move::{
    done, is_flat_window, ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord, MoveRefused,
    MoveRules, Unported,
};
use super::items::CharacterItems;
use crate::item::gc_item_clear;
use gamedata::item_kind::{ITEM_UNIQUE, USE_SPECIAL};
use gamedata::item_proto::ItemProto;

/// The multi-farm premium vnums (`G/char_item.cpp:3901-3906`).
pub const PREMIUM_VNUMS: [u32; 6] = [55_610, 55_611, 55_612, 55_613, 55_614, 55_615];

/// Whether `CG_ITEM_USE` on an item of this prototype consumes a multi-farm premium.
#[must_use]
pub fn is_premium(proto: &ItemProto) -> bool {
    proto.item_type == ITEM_UNIQUE
        && proto.sub_type == USE_SPECIAL
        && PREMIUM_VNUMS.contains(&proto.vnum)
}

/// Run the consume half of `CG_ITEM_USE` on a multi-farm premium: the checks, then one
/// item goes (`SetCount(GetCount() - 1)`). The affect landed before it, as legacy's
/// `AddAffect` runs before its `SetCount`.
///
/// # Errors
///
/// [`MoveRefused::NotPorted`] when the item is no premium, otherwise any refusal of
/// `use_checks`, which changes nothing.
pub fn use_premium(
    items: &mut CharacterItems,
    at: ItemPos,
    rules: &MoveRules,
    gear: Option<&mut Gear<'_>>,
) -> Result<MoveDone, MoveRefused> {
    let item = use_source(items, at)?;
    let gear = gear.ok_or(MoveRefused::NotPorted(Unported::Equipment))?;
    let proto = gear
        .protos
        .get(item.vnum)
        .ok_or(MoveRefused::UnknownVnum(item.vnum))?;
    if !is_premium(proto) {
        return Err(MoveRefused::NotPorted(Unported::UseVnum(item.vnum)));
    }
    use_checks(&item, proto, rules, gear.points)?;
    if !is_flat_window(at.window_type) {
        return Err(MoveRefused::NotPorted(Unported::SourceWindow(
            at.window_type,
        )));
    }
    let mut trail = Trail::default();
    if item.count <= 1 {
        let pos = items.release(item.id).map_err(MoveRefused::Storage)?;
        trail
            .records
            .push(MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos))));
        trail.changes.push(ItemChange::Destroyed { id: item.id });
    } else {
        let count = item.count - 1;
        items
            .set_count(item.id, u32::from(count))
            .map_err(MoveRefused::Count)?;
        let kept = items.item(item.id).ok_or(MoveRefused::Empty)?;
        trail
            .records
            .push(MoveRecord::Item(ItemRecord::Update(kept.gc_item_update())));
        trail.changes.push(ItemChange::Count { id: item.id, count });
    }
    Ok(done(MoveKind::Used, trail))
}
