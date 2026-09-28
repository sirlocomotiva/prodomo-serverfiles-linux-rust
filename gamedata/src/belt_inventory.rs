//! Which items a belt cell accepts.
//!
//! This is `CBeltInventoryHelper::CanMoveIntoBeltInventory` (`belt_inventory_helper.h:87-110`).
//! It reads nothing but the proto's type and sub-type, so it is a Game data rule and lives
//! beside the proto reader rather than in the world. The cell-by-grade table the same helper
//! holds is the world's, because it is asked about a worn belt.

use crate::item_proto::ItemProto;
use crate::item_proto_value::{sub_type_value, type_value, SubType};

/// The `ITEM_USE` sub-types a belt cell accepts, in the order the legacy `switch` lists them
/// (`belt_inventory_helper.h:99-103`).
const BELT_USE_SUB_TYPES: [&str; 5] = [
    "USE_POTION",
    "USE_POTION_NODELAY",
    "USE_ABILITY_UP",
    "USE_AFFECT",
    "USE_SPECIAL",
];

/// May an item of this proto be moved into a belt cell?
///
/// `ENABLE_AFFECT_RENEWAL` is defined (`prodomodefines.h:158`), so a blend item is accepted
/// whatever its sub-type (`belt_inventory_helper.h:89-92`). Otherwise the item must be an
/// `ITEM_USE` of one of five sub-types. The names are resolved through the proto reader's own
/// tables, so a reordering of those tables cannot move the rule onto other items.
#[must_use]
pub fn can_move_into_belt_inventory(proto: &ItemProto) -> bool {
    if type_value(b"ITEM_BLEND") == Some(proto.item_type) {
        return true;
    }
    let Some(item_use) = type_value(b"ITEM_USE") else {
        return false;
    };
    proto.item_type == item_use
        && BELT_USE_SUB_TYPES
            .iter()
            .any(|name| sub_type_value(item_use, name.as_bytes()) == SubType::Value(proto.sub_type))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item_proto_value::{SUB_TYPE, TYPE};

    fn index(table: &[&str], name: &str) -> i32 {
        let at = table
            .iter()
            .position(|entry| *entry == name)
            .unwrap_or_else(|| panic!("no {name}"));
        i32::try_from(at).expect("a table index always fits an i32")
    }

    fn proto(ty: &str, sub: &str) -> ItemProto {
        let column = index(TYPE, ty);
        let table = SUB_TYPE[usize::try_from(column).expect("an index is not negative")];
        let sub = if sub.is_empty() { 0 } else { index(table, sub) };
        ItemProto::for_category_rule(27001, column, sub)
    }

    #[test]
    fn the_five_use_sub_types_are_accepted() {
        for sub in BELT_USE_SUB_TYPES {
            assert!(
                can_move_into_belt_inventory(&proto("ITEM_USE", sub)),
                "{sub} should be accepted"
            );
        }
    }

    #[test]
    fn the_legacy_numbers_are_the_ones_the_names_resolve_to() {
        // The hand values from `item_length.h`: ITEM_USE 3, ITEM_BLEND 27, and the five
        // sub-types 0, 11, 7, 8 and 10. A table reordering would move these, and the rule
        // with them, so they are pinned here as the independent witness.
        assert_eq!(type_value(b"ITEM_USE"), Some(3));
        assert_eq!(type_value(b"ITEM_BLEND"), Some(27));
        let numbers: Vec<SubType> = BELT_USE_SUB_TYPES
            .iter()
            .map(|name| sub_type_value(3, name.as_bytes()))
            .collect();
        assert_eq!(
            numbers,
            [0, 11, 7, 8, 10].map(SubType::Value).to_vec(),
            "the belt sub-types moved"
        );
    }

    #[test]
    fn a_blend_item_is_accepted_whatever_its_sub_type() {
        let mut blend = proto("ITEM_BLEND", "");
        assert!(can_move_into_belt_inventory(&blend));
        blend.sub_type = 9;
        assert!(can_move_into_belt_inventory(&blend));
    }

    #[test]
    fn other_use_sub_types_and_other_types_are_refused() {
        for sub in ["USE_TUNING", "USE_CHANGE_ATTRIBUTE", "USE_TALISMAN"] {
            assert!(
                !can_move_into_belt_inventory(&proto("ITEM_USE", sub)),
                "{sub} should be refused"
            );
        }
        assert!(!can_move_into_belt_inventory(&proto(
            "ITEM_WEAPON",
            "WEAPON_SWORD"
        )));
        assert!(!can_move_into_belt_inventory(&proto(
            "ITEM_MATERIAL",
            "MATERIAL_LEATHER"
        )));
        // A potion's sub-type number on a type that is not ITEM_USE is not a potion.
        let mut weapon = proto("ITEM_WEAPON", "WEAPON_SWORD");
        weapon.sub_type = 0;
        assert!(!can_move_into_belt_inventory(&weapon));
    }
}
