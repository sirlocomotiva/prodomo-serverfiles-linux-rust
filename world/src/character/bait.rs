//! The bait arm of `UseItemEx` (`case USE_BAIT`, `G/char_item.cpp:6800-6825`).
//!
//! A bait used while the character fishes is refused with `[LS;418]`. Without a rod in the
//! weapon cell nothing happens and nothing is said. Otherwise the line tells whether the rod
//! held a bait (`[LS;419;%s]`) or not (`[LS;420;%s]`), the rod's socket 2 takes the bait's
//! `value0`, and one bait is used up. The caller sends the line, which goes before the
//! records, as legacy's `ChatPacket` goes before both `UpdatePacket`s.

use common::enums::EWearPositions;
use common::item_slots::{EWindows, INVENTORY_MAX_NUM};
use gamedata::item_kind::{ITEM_ROD, ITEM_USE, USE_BAIT};
use gamedata::item_proto::ItemProto;
use protocol::item_pos::ItemPos;

use super::equip::{use_checks, use_source, Gear, Trail};
use super::item_move::{done, MoveDone, MoveKind, MoveRefused, MoveRules, Unported};
use super::items::CharacterItems;
use super::quickslot::chains_when_used_up;

/// The rod's socket that holds the bait's level (`GetFishingLevel`, `G/fishing.cpp:546`).
pub const ROD_BAIT_SOCKET: usize = 2;

/// The inventory cell of the worn weapon, where a rod is held.
pub const WEAPON_CELL: u16 = INVENTORY_MAX_NUM + EWearPositions::Weapon as u16;

/// What a bait's use came to, for the line the caller sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Baited {
    /// The character fishes: `[LS;418]`, and nothing changed.
    WhileFishing,
    /// No rod is worn: nothing is said, and nothing changed.
    NoRod,
    /// The rod held a bait, which the new one replaced: `[LS;419;%s]`.
    Replaced,
    /// The rod held none: `[LS;420;%s]`.
    Put,
}

/// Whether a prototype is a bait.
#[must_use]
pub fn is_bait(proto: &ItemProto) -> bool {
    proto.item_type == ITEM_USE && proto.sub_type == USE_BAIT
}

/// Use the bait at `at`: `UseItem`'s checks, then the bait arm. `fishing` is whether the
/// character's fishing event runs.
///
/// # Errors
///
/// As [`super::use_item`]'s checks, and [`MoveRefused::NotPorted`] without `gear` or for an
/// item that is no bait.
pub fn use_bait(
    items: &mut CharacterItems,
    at: ItemPos,
    rules: &MoveRules,
    gear: Option<&mut Gear<'_>>,
    fishing: bool,
) -> Result<(MoveDone, Baited), MoveRefused> {
    let item = use_source(items, at)?;
    let gear = gear.ok_or(MoveRefused::NotPorted(Unported::Equipment))?;
    let proto = gear
        .protos
        .get(item.vnum)
        .ok_or(MoveRefused::UnknownVnum(item.vnum))?;
    if !is_bait(proto) {
        return Err(MoveRefused::NotPorted(Unported::UseSubType(proto.sub_type)));
    }
    use_checks(&item, proto, rules, gear.points)?;
    let unchanged = |baited| Ok((done(MoveKind::Used, Trail::default()), baited));
    if fishing {
        return unchanged(Baited::WhileFishing);
    }
    let rod = items
        .item_at(ItemPos::new(EWindows::Inventory as u8, WEAPON_CELL))
        .filter(|rod| {
            gear.protos
                .get(rod.vnum)
                .is_some_and(|rod_proto| rod_proto.item_type == ITEM_ROD)
        })
        .cloned();
    let Some(rod) = rod else {
        return unchanged(Baited::NoRod);
    };
    let baited = if rod.sockets[ROD_BAIT_SOCKET] == 0 {
        Baited::Put
    } else {
        Baited::Replaced
    };
    let socketed = super::set_item_socket(items, rod.id, (ROD_BAIT_SOCKET, proto.values[0]));
    let mut trail = Trail {
        records: socketed.records,
        changes: socketed.changes,
    };
    super::potion::use_one(items, &item, chains_when_used_up(proto), &mut trail)?;
    Ok((done(MoveKind::Used, trail), baited))
}
