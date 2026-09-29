//! The item type, sub-type and limit numbers the game rules compare an [`ItemProto`] against.
//!
//! Each is the value of a legacy enum member in `server/server/common/item_length.h`:
//! `EItemTypes` (`:68-107`), `EArmorSubTypes` (`:136-148`), `ECostumeSubTypes` (`:150-171`) and
//! `ELimitTypes` (`:427-451`). The Game data reader turns a name in `item_proto.txt` into its
//! **index** in the matching name table of [`item_proto_value`](crate::item_proto_value), and
//! the rules compare that index against the enum member, so the two numberings must agree. The
//! tests pin each constant to its name's index, which is the witness that they do.
//!
//! Only the members a ported rule reads are here. `common::enums` has Rust enums of the same
//! names, but they were written before the feature switches were read and they are wrong for
//! this build: `EArmorSubTypes` stops before `ARMOR_GLOVE`, which `ENABLE_GLOVE_SYSTEM` adds.
//!
//! [`ItemProto`]: crate::item_proto::ItemProto

/// `ITEM_NONE`.
pub const ITEM_NONE: i32 = 0;
/// `ITEM_WEAPON`.
pub const ITEM_WEAPON: i32 = 1;
/// `ITEM_ARMOR`.
pub const ITEM_ARMOR: i32 = 2;
/// `ITEM_METIN`, a stone that sits in another item's socket.
pub const ITEM_METIN: i32 = 10;
/// `ITEM_ROD`, a fishing rod.
pub const ITEM_ROD: i32 = 13;
/// `ITEM_UNIQUE`.
pub const ITEM_UNIQUE: i32 = 16;
/// `ITEM_PICK`, a pickaxe.
pub const ITEM_PICK: i32 = 24;
/// `ITEM_COSTUME`.
pub const ITEM_COSTUME: i32 = 28;
/// `ITEM_DS`, a dragon soul stone.
pub const ITEM_DS: i32 = 29;
/// `ITEM_SPECIAL_DS`.
pub const ITEM_SPECIAL_DS: i32 = 30;
/// `ITEM_RING`.
pub const ITEM_RING: i32 = 33;
/// `ITEM_BELT`.
pub const ITEM_BELT: i32 = 34;
/// `ITEM_TALISMAN`.
pub const ITEM_TALISMAN: i32 = 35;
/// `ITEM_TOGGLE`.
pub const ITEM_TOGGLE: i32 = 36;

/// `ARMOR_BODY`.
pub const ARMOR_BODY: i32 = 0;
/// `ARMOR_HEAD`.
pub const ARMOR_HEAD: i32 = 1;
/// `ARMOR_SHIELD`.
pub const ARMOR_SHIELD: i32 = 2;
/// `ARMOR_WRIST`.
pub const ARMOR_WRIST: i32 = 3;
/// `ARMOR_FOOTS`.
pub const ARMOR_FOOTS: i32 = 4;
/// `ARMOR_NECK`.
pub const ARMOR_NECK: i32 = 5;
/// `ARMOR_EAR`.
pub const ARMOR_EAR: i32 = 6;
/// `ARMOR_GLOVE`, behind `ENABLE_GLOVE_SYSTEM`, which `prodomodefines.h` defines.
pub const ARMOR_GLOVE: i32 = 7;

/// `COSTUME_BODY`, which the source pins to `ARMOR_BODY`.
pub const COSTUME_BODY: i32 = 0;
/// `COSTUME_HAIR`, which the source pins to `ARMOR_HEAD`.
pub const COSTUME_HAIR: i32 = 1;
/// `COSTUME_MOUNT`, behind `ENABLE_MOUNT_COSTUME_SYSTEM`.
pub const COSTUME_MOUNT: i32 = 2;
/// `COSTUME_SASH`, behind `__SASH_SYSTEM__`.
pub const COSTUME_SASH: i32 = 3;
/// `COSTUME_WEAPON`, behind `ENABLE_WEAPON_COSTUME_SYSTEM`.
pub const COSTUME_WEAPON: i32 = 4;
/// `COSTUME_AURA`, behind `__AURA_SYSTEM__`.
pub const COSTUME_AURA: i32 = 5;
/// `COSTUME_PET`, behind `ENABLE_PET_COSTUME_SYSTEM`.
pub const COSTUME_PET: i32 = 6;
/// `COSTUME_SASH_SKIN`, the member after `COSTUME_PET` with no initialiser.
pub const COSTUME_SASH_SKIN: i32 = 7;

/// `LIMIT_NONE`.
pub const LIMIT_NONE: i32 = 0;
/// `LIMIT_LEVEL`: the wearer's level must be at least the limit's value.
pub const LIMIT_LEVEL: i32 = 1;
/// `LIMIT_REAL_TIME`: the item expires at the time in its socket 0.
pub const LIMIT_REAL_TIME: i32 = 6;
/// `LIMIT_REAL_TIME_START_FIRST_USE` (`item_length.h:443`).
///
/// `Set_Proto_Item_Table:962-968` compares a limit's **index into `arLimitType`** against this
/// enum member. The two numberings coincide even though the names differ: the table spells index
/// 7 `REAL_TIME_FIRST_USE` where the enum spells it `LIMIT_REAL_TIME_START_FIRST_USE`. Comparing
/// against the file's spelling instead of the enum's would silently leave the index at -1 and
/// disable every real-time limit in the game.
pub const LIMIT_REAL_TIME_START_FIRST_USE: i32 = 7;
/// `LIMIT_TIMER_BASED_ON_WEAR` (`item_length.h:448`): the item's time runs while it is worn.
pub const LIMIT_TIMER_BASED_ON_WEAR: i32 = 8;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item_proto_value::{LIMIT_TYPE, SUB_TYPE, TYPE};

    /// The index of `name` in `table`, as the reader would store it.
    fn index(table: &[&str], name: &str) -> i32 {
        let at = table
            .iter()
            .position(|entry| *entry == name)
            .unwrap_or_else(|| panic!("no {name} in the table"));
        i32::try_from(at).expect("a table index fits an i32")
    }

    #[test]
    fn each_item_type_is_its_name_index() {
        for (value, name) in [
            (ITEM_NONE, "ITEM_NONE"),
            (ITEM_WEAPON, "ITEM_WEAPON"),
            (ITEM_ARMOR, "ITEM_ARMOR"),
            (ITEM_METIN, "ITEM_METIN"),
            (ITEM_ROD, "ITEM_ROD"),
            (ITEM_UNIQUE, "ITEM_UNIQUE"),
            (ITEM_PICK, "ITEM_PICK"),
            (ITEM_COSTUME, "ITEM_COSTUME"),
            (ITEM_DS, "ITEM_DS"),
            (ITEM_SPECIAL_DS, "ITEM_SPECIAL_DS"),
            (ITEM_RING, "ITEM_RING"),
            (ITEM_BELT, "ITEM_BELT"),
            (ITEM_TALISMAN, "ITEM_TALISMAN"),
            (ITEM_TOGGLE, "ITEM_TOGGLE"),
        ] {
            assert_eq!(value, index(TYPE, name), "{name}");
        }
    }

    #[test]
    fn each_sub_type_is_its_name_index_in_its_type_table() {
        let armor = SUB_TYPE[usize::try_from(ITEM_ARMOR).expect("a small index")];
        for (value, name) in [
            (ARMOR_BODY, "ARMOR_BODY"),
            (ARMOR_HEAD, "ARMOR_HEAD"),
            (ARMOR_SHIELD, "ARMOR_SHIELD"),
            (ARMOR_WRIST, "ARMOR_WRIST"),
            (ARMOR_FOOTS, "ARMOR_FOOTS"),
            (ARMOR_NECK, "ARMOR_NECK"),
            (ARMOR_EAR, "ARMOR_EAR"),
            (ARMOR_GLOVE, "ARMOR_GLOVE"),
        ] {
            assert_eq!(value, index(armor, name), "{name}");
        }
        let costume = SUB_TYPE[usize::try_from(ITEM_COSTUME).expect("a small index")];
        for (value, name) in [
            (COSTUME_BODY, "COSTUME_BODY"),
            (COSTUME_HAIR, "COSTUME_HAIR"),
            (COSTUME_MOUNT, "COSTUME_MOUNT"),
            (COSTUME_SASH, "COSTUME_SASH"),
            (COSTUME_WEAPON, "COSTUME_WEAPON"),
            (COSTUME_AURA, "COSTUME_AURA"),
            (COSTUME_PET, "COSTUME_PET"),
            (COSTUME_SASH_SKIN, "COSTUME_SASH_SKIN"),
        ] {
            assert_eq!(value, index(costume, name), "{name}");
        }
    }

    /// The limit table spells two members differently from the enum, so those two are pinned
    /// to the table's own spelling: the reader stores the table index.
    #[test]
    fn each_limit_is_its_name_index() {
        for (value, name) in [
            (LIMIT_NONE, "LIMIT_NONE"),
            (LIMIT_LEVEL, "LEVEL"),
            (LIMIT_REAL_TIME, "REAL_TIME"),
            (LIMIT_REAL_TIME_START_FIRST_USE, "REAL_TIME_FIRST_USE"),
            (LIMIT_TIMER_BASED_ON_WEAR, "TIMER_BASED_ON_WEAR"),
        ] {
            assert_eq!(value, index(LIMIT_TYPE, name), "{name}");
        }
    }
}
