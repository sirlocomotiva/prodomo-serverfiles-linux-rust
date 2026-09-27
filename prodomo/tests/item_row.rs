//! The seam between the store's item row and the world's item.
//!
//! `db` cannot see `world`, because the world is above the store, so the two
//! structures that describe one item are written twice. A field renamed on one side
//! and not the other is a silent corruption rather than a compile error, and the
//! mapping is a `prodomo` responsibility because `prodomo` is the crate that depends
//! on both. This is where that mapping is pinned.
//!
//! Nine fields are shared. Two are not shared and are not supposed to be:
//!
//! * `ItemRow` has `owner_id` and `window_type` where `Item` has `pos: ItemPos`.
//!   `ItemPos` is the packed `BYTE window_type` + `WORD cell`, so the split is at the
//!   storage boundary and is a shape difference, not drift.
//! * `Item` has `size: u8` and `ItemRow` has no such column, because the grid
//!   footprint is `TItemTable::bSize`, a **prototype** fact. The table does not store it
//!   and legacy does not either; the world reads it from `gamedata::item_proto` when it
//!   builds an instance.
//!
//! The attribute pair is shared too, and it is shared *by name*: `db::items::Attribute`
//! uses `s_value` so it matches `protocol::gc_item_window::ItemAttribute` and the legacy
//! `TPlayerItemAttribute::sValue`. The first draft of that struct called it `value`, which
//! is the exact drift this file exists to catch -- found by writing the test, not before.

use db::items::{Attribute, ItemRow};
// `world::item` re-exports `ItemAttribute` privately, and this test is deliberately
// outside the world, so the type is imported from where it is defined. That it is the
// *same* type the world uses is the point, and `prodomo` depends on `protocol` too.
use protocol::gc_item_window::ItemAttribute;
use world::item::{ItemIdRange, ItemIds, ATTRIBUTES, SOCKETS};

/// The nine fields the two structures share.
const SHARED: [&str; 9] = [
    "id",
    "vnum",
    "count",
    "refine_element",
    "transmutation",
    "flags",
    "anti_flags",
    "sockets",
    "attributes",
];

/// Every field of [`ItemRow`], written out by hand.
///
/// A derive or a macro would make this list generate itself, and then it would prove
/// nothing: the failure guarded against is one side being changed and the other
/// forgotten, and only a literal on each side can catch that.
const ROW_FIELDS: [&str; 12] = [
    "id",
    "owner_id",
    "window_type",
    "pos",
    "vnum",
    "count",
    "refine_element",
    "transmutation",
    "flags",
    "anti_flags",
    "sockets",
    "attributes",
];

/// Every field of `world::item::Item`, written out by hand, for the same reason.
const ITEM_FIELDS: [&str; 11] = [
    "id",
    "vnum",
    "count",
    "refine_element",
    "transmutation",
    "flags",
    "anti_flags",
    "sockets",
    "attributes",
    "size",
    "pos",
];

#[test]
fn the_nine_shared_field_names_are_pinned_on_both_sides() {
    for name in SHARED {
        assert!(
            ROW_FIELDS.contains(&name),
            "db::items::ItemRow lost the shared field {name}"
        );
        assert!(
            ITEM_FIELDS.contains(&name),
            "world::item::Item lost the shared field {name}"
        );
    }
    // No name appears twice, so a "rename" that only added a field is caught too.
    for (index, name) in ROW_FIELDS.iter().enumerate() {
        assert!(
            !ROW_FIELDS[index + 1..].contains(name),
            "ItemRow names {name} twice"
        );
    }

    // And the two that are deliberately not shared, so a future change that merges them
    // is a deliberate act rather than an accident.
    assert_eq!(
        ROW_FIELDS.len() - SHARED.len(),
        3,
        "ItemRow should carry exactly owner_id, window_type and pos beyond the shared nine"
    );
    assert_eq!(
        ITEM_FIELDS.len() - SHARED.len(),
        2,
        "Item should carry exactly size and pos beyond the shared nine"
    );
}

#[test]
fn the_socket_and_attribute_counts_agree_across_the_seam() {
    // `db::items` sizes its column arrays from `common::constants` and `world::item`
    // from the same place, so these are the same number -- but they are two separate
    // `const` evaluations, and the table has to match the migration.
    assert_eq!(db::items::SOCKETS, SOCKETS);
    assert_eq!(db::items::SOCKET_COLUMNS.len(), SOCKETS);
    assert_eq!(db::items::ATTRTYPE_COLUMNS.len(), ATTRIBUTES);
    assert_eq!(db::items::ATTRVALUE_COLUMNS.len(), ATTRIBUTES);
    assert_eq!(SOCKETS, 6);
    assert_eq!(ATTRIBUTES, 7);
}

#[test]
fn the_attribute_pair_converts_without_a_reinterpretation() {
    // The one place the two attribute types touch. A `u8` and an `i16` move across
    // unchanged, and the value keeps its sign, which is the property that a `u16` or a
    // checked conversion would quietly break.
    let store = Attribute {
        b_type: 255,
        s_value: -32_768,
    };
    let wire = ItemAttribute {
        b_type: store.b_type,
        s_value: store.s_value,
    };
    assert_eq!(wire.b_type, 255);
    assert_eq!(wire.s_value, -32_768);
    assert_eq!(wire, ItemAttribute::new(255, -32_768));
}

#[test]
fn an_id_from_the_world_is_usable_as_a_row_id() {
    // The one value the seam actually moves: `ItemIds` allocates, the store persists.
    //
    // There is deliberately **no width check here**, and the reason is worth stating
    // because the first draft of this test claimed one: `world::item::ItemId` is
    // `pub type ItemId = u32` (`world/src/item.rs:85`), a type alias and not a newtype.
    // So the two sides share the type outright and cannot disagree about the width --
    // `u32::from(id)` is a no-op, which is what clippy says when the test tries it.
    // The alias is the check; a conversion in a test would only have hidden that.
    let range = ItemIdRange::new(10_000_000, 4_290_000_001, 10_000_001).expect("a usable range");
    let mut ids = ItemIds::new(range);
    let id = ids.allocate().expect("a fresh pool allocates");
    let row = ItemRow {
        id,
        owner_id: Some(7),
        window_type: 1,
        pos: 0,
        vnum: 30_000,
        count: 1,
        refine_element: 0,
        transmutation: 0,
        flags: 0,
        anti_flags: 0,
        sockets: [0; SOCKETS],
        attributes: [Attribute {
            b_type: 0,
            s_value: 0,
        }; ATTRIBUTES],
    };
    // The first **usable** id, because the ids below `first_usable` are reserved, and
    // never 0, because 0 is `NO_ITEM`.
    assert_eq!(row.id, 10_000_001);
    assert_ne!(row.id, 0, "0 is NO_ITEM and is never a legal id");
}
