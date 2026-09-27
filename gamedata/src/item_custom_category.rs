//! Which of the six custom-inventory categories an item belongs in, if any.
//!
//! The category decides **where** an item goes when it is picked up or given to a
//! character, so the free-cell search and the client's own page layout both depend on it.
//! Legacy computes it in `CItem::IsCustomCategory` (`server/server/game/item.cpp:1255-1440`)
//! from three things, in this order, stopping at the first that answers:
//!
//! 1. a hand-written table of vnums,
//! 2. the item's `bType`,
//! 3. its `bSubType`.
//!
//! **It is not in `item_proto.txt`.** The Rewrite reads that file in ledger 191 and there is
//! no column for this, so it is code here rather than a column there. That is deliberate: a
//! column would be a second source of truth for a rule the client also hardcodes, and the
//! owner's data file does not carry it.
//!
//! Two of the vnum tables are **exclusion** lists, and they are checked *before* the type
//! and sub-type rules, so an excluded vnum is refused even when its type would otherwise
//! place it. That ordering is the whole reason the exclusions exist, and reversing it would
//! put the wrong items in the wrong category.
//!
//! # The two facts a caller needs to know
//!
//! * **A category is not exclusive, and that is measurable.** [`item_category`] returns the
//!   *first* category that matches, because that is what `CItem::GetItemCategory` does
//!   (`item.cpp:1415-1424`), but [`is_custom_category`] answers per category and the two
//!   genuinely differ on 5 of the 7,305 rows in the owner's file: `27987` is a gift box *and*
//!   is listed in category 2, `30270` is leather *and* is listed in category 3, and `55003`,
//!   `55004` and `55005` are leather *and* are listed in category 0. Legacy's
//!   `GetEmptyInventory` handles this by walking all six categories and using the **first one
//!   with a free cell** (`char_item.cpp:1214-1225`), not the first one that matches at all, so
//!   an item matching two categories can land in either depending on what is full. A caller
//!   that wants legacy's placement must scan, not take [`item_category`].
//! * **Category 5's vnum table is dead.** Legacy declares `dwCategory_5_Items` and then
//!   comments out the loop that reads it (`item.cpp:1437-1441`), so `90000`, `70063` and
//!   `70064` are placed by their `ITEM_COSTUME` sub-type or not at all. This module keeps the
//!   numbers and marks them unread rather than pretending the loop is live, because deleting
//!   them would hide that the owner's data holds three vnums whose placement depends on a
//!   loop someone disabled.

use std::sync::OnceLock;

use crate::item_proto::ItemProto;
use crate::item_proto_value::SUB_TYPE;

/// `CUSTOM_INVENTORY_CATEGORY_NUM` = 6 (`length.h:31`). Six categories, and the client draws
/// its custom-inventory pages one category at a time.
pub const CATEGORY_NUM: u8 = 6;

/// The vnums `item.cpp` places in category 0, the skill-book page, before the type rule
/// (`item.cpp:1260-1263`).
const CATEGORY_0_ITEMS: &[u32] = &[
    70800, 71178, 71203, 71205, 71207, 71209, 71211, 71213, 71215, 71217, 55034, 55035, 55036,
    55037, 55038, 55003, 55004, 55005, 55010, 55011, 55012, 55013, 55014, 55015, 55016, 55017,
    55018, 55019, 55020, 55021, 55022, 55023, 55024, 55025, 55026, 55027, 50301, 50302, 50303,
];

/// The vnums `item.cpp` places in category 2, before the type and sub-type rules
/// (`item.cpp:1265-1269`).
const CATEGORY_2_ITEMS: &[u32] = &[
    25040, 27987, 28982, 28980, 28985, 28986, 28981, 28983, 28984, 28987, 70602, 70603, 28995,
    28996,
];

/// The vnums `item.cpp` places in category 3 (`item.cpp:1271-1274`).
const CATEGORY_3_ITEMS: &[u32] = &[54705, 54702, 54703, 30270];

/// The vnums **excluded** from category 3, checked first (`item.cpp:1275-1279`).
const CATEGORY_3_EXCLUDED: &[u32] = &[
    70600, 70800, 71178, 71203, 71205, 71207, 71209, 71211, 71213, 71215, 71217, 49377,
];

/// The vnums `item.cpp` places in category 4 (`item.cpp:1281-1284`).
const CATEGORY_4_ITEMS: &[u32] = &[
    70102, 72001, 72002, 72003, 71015, 72049, 72050, 70005, 39002, 72303, 79507, 38058, 71181,
    71180, 72004, 72005, 72006, 76037, 79508, 70043, 71016, 50815, 50816, 50817, 50818, 50819,
    50820,
];

/// The vnum `item.cpp:1432-1434` skips inside the category 4 list, which is **not** in
/// [`CATEGORY_4_ITEMS`].
///
/// The guard is `if (71083 == GetVnum()) continue;` inside a loop over the list, and `71083`
/// appears in no list in this function. So the branch is unreachable and the vnum is placed by
/// its type like any other. It is carried here because it is in the source and a reader
/// comparing the two will meet it, and because "this is dead" is a claim that needs a test
/// rather than a comment.
const CATEGORY_4_SKIPPED_VNUM: u32 = 71083;

/// The vnums **excluded** from category 4, checked first (`item.cpp:1285-1290`).
const CATEGORY_4_EXCLUDED: &[u32] = &[
    50301, 50302, 50303, 70038, 79900, 50289, 72326, 72341, 79998, 79999, 72723, 72727, 71300,
    80014, 80015, 80016, 80017, 79507, 79508, 79700, 79701, 79702, 79703, 79704, 79705, 27987,
    72321,
];

// `dwCategory_5_Items` (`item.cpp:1292`) is transcribed in the test module, not here. It is
// never read by `is_custom_category` -- the loop that would read it is commented out in the
// source -- so a constant in this module would be dead code, and a constant that is dead
// code is worse than no constant: a future reader finds the numbers in the source anyway,
// and here they would look live.

/// `bType` indices, which are positions in `item_proto_value::TYPE` and therefore the same
/// values legacy's `EItemTypes` gives (`item_length.h:1137-1175`). Read from `TYPE` rather
/// than restated, so a reordering of that table cannot silently misplace an item.
mod item_type {
    use crate::item_proto_value::TYPE;

    /// The index of a `bType` name, or a panic if `TYPE` lost it.
    fn of(name: &str) -> i32 {
        let index = TYPE
            .iter()
            .position(|entry| *entry == name)
            .unwrap_or_else(|| panic!("item_proto_value::TYPE has no {name}"));
        // `TYPE` has a handful of entries, so the index always fits an `i32`. Saying that in
        // the source is better than a truncating cast, which reads as though a large index
        // were possible and would then answer -1 rather than fail.
        i32::try_from(index).expect("an item type index always fits an i32")
    }

    pub fn item_use() -> i32 {
        of("ITEM_USE")
    }
    pub fn item_material() -> i32 {
        of("ITEM_MATERIAL")
    }
    pub fn item_metin() -> i32 {
        of("ITEM_METIN")
    }
    pub fn item_skillbook() -> i32 {
        of("ITEM_SKILLBOOK")
    }
    pub fn item_polymorph() -> i32 {
        of("ITEM_POLYMORPH")
    }
    pub fn item_giftbox() -> i32 {
        of("ITEM_GIFTBOX")
    }
    pub fn item_costume() -> i32 {
        of("ITEM_COSTUME")
    }
}

/// The index of a sub-type name in its item type's table, resolved once.
///
/// A name this module asks for and the table does not have would mean the two disagree, and
/// answering with a number would place every such item in the wrong category. So a miss
/// aborts the process rather than guessing.
///
/// It is a `OnceLock` rather than a `const fn` because comparing `&str` is not a const
/// operation on this toolchain. The cost is one atomic read per call, which is nothing next
/// to the 1370-cell scan the answer is used for.
fn sub_index(ty: &str, name: &str) -> i32 {
    static CACHE: OnceLock<Vec<(&'static str, &'static str, i32)>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        let resolve = |ty: &'static str, name: &'static str| {
            let column = crate::item_proto_value::TYPE
                .iter()
                .position(|entry| *entry == ty)
                .unwrap_or_else(|| panic!("item_proto_value::TYPE has no {ty}"));
            let table = SUB_TYPE[column];
            let index = table
                .iter()
                .position(|entry| *entry == name)
                .unwrap_or_else(|| panic!("SUB_TYPE for {ty} has no {name}"));
            (
                ty,
                name,
                i32::try_from(index).expect("a type index always fits an i32"),
            )
        };
        [
            resolve("ITEM_METIN", "METIN_NORMAL"),
            resolve("ITEM_MATERIAL", "MATERIAL_LEATHER"),
            resolve("ITEM_USE", "USE_AFFECT"),
            resolve("ITEM_USE", "USE_SPECIAL"),
            resolve("ITEM_USE", "USE_CLEAN_SOCKET"),
            resolve("ITEM_USE", "USE_CHANGE_ATTRIBUTE"),
            resolve("ITEM_USE", "USE_ADD_ATTRIBUTE"),
            resolve("ITEM_USE", "USE_ADD_ATTRIBUTE2"),
            resolve("ITEM_COSTUME", "COSTUME_BODY"),
            resolve("ITEM_COSTUME", "COSTUME_HAIR"),
            resolve("ITEM_COSTUME", "COSTUME_SASH"),
        ]
        .to_vec()
    });
    cache
        .iter()
        .find(|&&(key_ty, key_name, _)| key_ty == ty && key_name == name)
        .map_or_else(
            || panic!("sub_index was asked for a pair this module does not list: {ty} {name}"),
            |&(_, _, index)| index,
        )
}

/// Does `proto` belong in custom-inventory category `category`?
///
/// `category` is 0 to 5. Anything else is `false`, which is what legacy does: the `else if`
/// chain falls through to `return false` for a category it does not know
/// (`item.cpp:1434-1436`).
///
/// Only the fields a category needs are read, and each arm returns as soon as it answers,
/// so this matches legacy's short-circuit order rather than being a re-derivation.
#[must_use]
pub fn is_custom_category(proto: &ItemProto, category: u8) -> bool {
    let vnum = proto.vnum;
    let (ty, sub) = (proto.item_type, proto.sub_type);
    match category {
        // A skill book, or one of the hand-listed skill vnums.
        0 => ty == item_type::item_skillbook() || CATEGORY_0_ITEMS.contains(&vnum),
        // Metin, and the metin page. **No vnum table**: legacy's category 1 has no array at
        // all, so the sub-type is the whole rule.
        1 => ty == item_type::item_metin() && sub == sub_index("ITEM_METIN", "METIN_NORMAL"),
        // A listed vnum, or a piece of leather.
        2 => {
            CATEGORY_2_ITEMS.contains(&vnum)
                || (ty == item_type::item_material()
                    && sub == sub_index("ITEM_MATERIAL", "MATERIAL_LEATHER"))
        }
        // A gift box, or one of four listed vnums -- but never an excluded one, and the
        // exclusion is checked first.
        3 => {
            !CATEGORY_3_EXCLUDED.contains(&vnum)
                && (ty == item_type::item_giftbox() || CATEGORY_3_ITEMS.contains(&vnum))
        }
        // An attribute-changing or effect item, or one of the listed vnums, and never an
        // excluded one. The `71083` guard is carried because legacy has it; see
        // [`CATEGORY_4_SKIPPED_VNUM`].
        4 => {
            let use_attribute_item = ty == item_type::item_use()
                && matches!(
                    sub,
                    _ if sub
                        == sub_index("ITEM_USE", "USE_CHANGE_ATTRIBUTE")
                        || sub == sub_index("ITEM_USE", "USE_ADD_ATTRIBUTE")
                        || sub == sub_index("ITEM_USE", "USE_ADD_ATTRIBUTE2")
                        || sub == sub_index("ITEM_USE", "USE_CLEAN_SOCKET")
                        || sub == sub_index("ITEM_USE", "USE_AFFECT")
                        || sub == sub_index("ITEM_USE", "USE_SPECIAL")
                );
            !CATEGORY_4_EXCLUDED.contains(&vnum)
                && (use_attribute_item
                    || ty == item_type::item_polymorph()
                    || (CATEGORY_4_ITEMS.contains(&vnum) && vnum != CATEGORY_4_SKIPPED_VNUM))
        }
        // Costumes only. The vnum list is dead; see [`CATEGORY_5_ITEMS_DEAD`].
        5 => {
            ty == item_type::item_costume()
                && (sub == sub_index("ITEM_COSTUME", "COSTUME_BODY")
                    || sub == sub_index("ITEM_COSTUME", "COSTUME_HAIR")
                    || sub == sub_index("ITEM_COSTUME", "COSTUME_SASH"))
        }
        _ => false,
    }
}

/// The first category `proto` belongs in, or `None` for the base inventory.
///
/// This is `CItem::GetItemCategory` (`item.cpp:1415-1424`): ascending, first match wins, and
/// `-1` becomes `None`. Legacy's `-1` is a real return value that callers compare against, so
/// it is not an error and not a full inventory -- it means "this item lives in the ordinary
/// inventory".
#[must_use]
pub fn item_category(proto: &ItemProto) -> Option<u8> {
    (0..CATEGORY_NUM).find(|&category| is_custom_category(proto, category))
}

#[cfg(test)]
mod tests {

    /// `dwCategory_5_Items` (`item.cpp:1292`), transcribed so the test below can prove it is
    /// never read. It is here and not in the parent module because it is the only place it
    /// appears, and a constant in the parent that nothing reads is dead code.
    const CATEGORY_5_ITEMS_DEAD: &[u32] = &[90000, 70063, 70064];
    use super::*;
    use crate::item_proto::{parse, ItemProtos};
    use crate::item_proto_value::{SUB_TYPE, TYPE};

    /// The owner's `item_proto.txt` and `item_names.txt`, read fresh, so these tests run
    /// against the real data rather than a fixture that could drift away from it.
    fn owners() -> ItemProtos {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        let read = |name: &str| {
            let path = dir.join(name);
            std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
        };
        parse(&read("item_proto.txt"), &read("item_names.txt"))
            .expect("the owner's item proto loads")
    }

    /// A proto with only the three fields this module reads, so a test cannot pass because of
    /// an unrelated field.
    fn proto_of(vnum: u32, ty: &str, sub: &str) -> ItemProto {
        // The `i32` conversions are checked rather than cast. `TYPE` and every `SUB_TYPE` are
        // short, so a failing conversion is impossible, and a checked one says so in the
        // source instead of leaving a cast a reader has to reason about. A `usize` index is
        // also never taken from an `i32`, which would need the sign restored.
        let column = TYPE
            .iter()
            .position(|entry| *entry == ty)
            .unwrap_or_else(|| panic!("TYPE has no {ty}"));
        let table = SUB_TYPE[column];
        // A type with no sub-type table resolves to 0, which is what legacy returns
        // (`ProtoReader.cpp:344-347`). So an empty `sub` means "the file said 0", and a test
        // that means a specific sub-type always names one.
        let sub_index = if sub.is_empty() {
            0
        } else {
            table
                .iter()
                .position(|entry| *entry == sub)
                .unwrap_or_else(|| panic!("SUB_TYPE for {ty} has no {sub}"))
        };
        ItemProto::for_category_rule(
            vnum,
            i32::try_from(column).expect("a type index always fits an i32"),
            i32::try_from(sub_index).expect("a sub-type index always fits an i32"),
        )
    }

    #[test]
    fn a_skill_book_lands_in_category_0() {
        // The only rule category 0 has for a type, and the reason a fresh character's first
        // book shows on the skill page rather than in the base inventory.
        assert!(is_custom_category(&proto_of(1, "ITEM_SKILLBOOK", ""), 0));
        // And it is not a metin, which is category 1.
        assert!(!is_custom_category(&proto_of(1, "ITEM_SKILLBOOK", ""), 1));
    }

    #[test]
    fn a_plain_metin_lands_in_category_1_and_only_there() {
        let metin = proto_of(60000, "ITEM_METIN", "METIN_NORMAL");
        assert!(is_custom_category(&metin, 1));
        for category in [0u8, 2, 3, 4, 5] {
            assert!(
                !is_custom_category(&metin, category),
                "metin should not be in category {category}"
            );
        }
    }

    #[test]
    fn leather_lands_in_category_2() {
        assert!(is_custom_category(
            &proto_of(1, "ITEM_MATERIAL", "MATERIAL_LEATHER"),
            2
        ));
        // A different material is not.
        assert!(!is_custom_category(
            &proto_of(1, "ITEM_MATERIAL", "MATERIAL_BLOOD"),
            2
        ));
    }

    #[test]
    fn a_giftbox_lands_in_category_3() {
        assert!(is_custom_category(&proto_of(1, "ITEM_GIFTBOX", ""), 3));
    }

    #[test]
    fn an_excluded_vnum_loses_to_its_type_in_category_3() {
        // `70800` is in BOTH the category 0 list and the category 3 exclusion list. The
        // exclusion is checked first, so the type rule never gets to place it in category 3.
        // This is the ordering that a re-implementation gets wrong.
        let proto = proto_of(70800, "ITEM_GIFTBOX", "");
        assert!(!is_custom_category(&proto, 3), "exclusion must win");
        assert!(
            is_custom_category(&proto, 0),
            "and it is still a category 0 item"
        );
    }

    #[test]
    fn an_excluded_vnum_loses_to_its_type_in_category_4() {
        // `50301` is in the category 4 exclusion list. Paired with a type that category 4
        // would otherwise accept, it must still be refused.
        let proto = proto_of(50301, "ITEM_USE", "USE_ADD_ATTRIBUTE");
        assert!(!is_custom_category(&proto, 4));
        // The same type with a vnum in no list at all is accepted, which is what shows the
        // exclusion is the reason and not the type. The vnum has to be in neither list, and
        // the obvious candidate next door is not: `50301`, `50302` and `50303` are all three
        // excluded, so picking `50302` would have tested the exclusion twice and passed for
        // the wrong reason.
        let control = 1;
        assert!(!CATEGORY_4_EXCLUDED.contains(&control));
        assert!(!CATEGORY_4_ITEMS.contains(&control));
        assert!(is_custom_category(
            &proto_of(control, "ITEM_USE", "USE_ADD_ATTRIBUTE"),
            4
        ));
    }

    #[test]
    fn every_attribute_and_effect_item_type_lands_in_category_4() {
        // The six sub-types `item.cpp:1398-1428` accepts, one test each, so removing one arm
        // of that `if` chain is a failure rather than a silent loss.
        for sub in [
            "USE_CHANGE_ATTRIBUTE",
            "USE_ADD_ATTRIBUTE",
            "USE_ADD_ATTRIBUTE2",
            "USE_CLEAN_SOCKET",
            "USE_AFFECT",
            "USE_SPECIAL",
        ] {
            let proto = proto_of(1, "ITEM_USE", sub);
            assert!(
                is_custom_category(&proto, 4),
                "{sub} should be a category 4 item"
            );
            // And not in 0, which is where a careless "any ITEM_USE" would put it.
            assert!(!is_custom_category(&proto, 0));
        }
    }

    #[test]
    fn a_polymorph_lands_in_category_4() {
        assert!(is_custom_category(&proto_of(1, "ITEM_POLYMORPH", ""), 4));
    }

    #[test]
    fn the_skipped_vnum_71083_is_in_no_list_at_all() {
        // Legacy guards the category 4 list with `if (71083 == GetVnum()) continue;`
        // (`item.cpp:1432-1434`), which only matters if 71083 is in that list. It is not: it
        // appears in no list in the function. So the guard is unreachable, and the vnum is
        // placed by its type alone.
        //
        // The test is written as two assertions rather than one so that both failures are
        // distinguishable: "it is in the list" means the transcription is wrong, and "it is
        // placed by its totem type" means the rule is wrong.
        assert!(
            !CATEGORY_4_ITEMS.contains(&CATEGORY_4_SKIPPED_VNUM),
            "71083 is not in dwCategory_4_Items, so the legacy skip is unreachable"
        );
        assert!(
            !is_custom_category(&proto_of(71083, "ITEM_TOTEM", ""), 4),
            "and a totem is not placed by anything else in category 4 either"
        );
        // A different vnum that IS in the list is placed, so the assertion above is about
        // this vnum and not about the list being unread.
        assert!(CATEGORY_4_ITEMS.contains(&70102));
        assert!(is_custom_category(&proto_of(70102, "ITEM_TOTEM", ""), 4));
    }

    #[test]
    fn a_body_costume_lands_in_category_5() {
        for sub in ["COSTUME_BODY", "COSTUME_HAIR", "COSTUME_SASH"] {
            assert!(
                is_custom_category(&proto_of(1, "ITEM_COSTUME", sub), 5),
                "{sub} should be a category 5 item"
            );
        }
        // A costume weapon is in the table but `item.cpp:1429-1434` does not accept it.
        assert!(!is_custom_category(
            &proto_of(1, "ITEM_COSTUME", "COSTUME_WEAPON"),
            5
        ));
    }

    #[test]
    fn category_fives_vnum_list_is_never_read() {
        // The three vnums legacy left in the table with the loop commented out. They are
        // carried here so the fact is visible, and this test is what stops them from being
        // quietly treated as live: a non-costume vnum is in no category at all.
        for &vnum in CATEGORY_5_ITEMS_DEAD {
            let proto = proto_of(vnum, "ITEM_TOTEM", "");
            assert!(
                !is_custom_category(&proto, 5),
                "{vnum} is in the dead list and must not be placed by it"
            );
            assert_eq!(item_category(&proto), None, "{vnum} lands nowhere");
        }
    }

    #[test]
    fn a_category_past_the_last_is_never_a_match() {
        // Legacy's `else if` chain falls through to `return false`, so 6 and 255 are both
        // simply "no". This is what makes `0..CATEGORY_NUM` the right scan range.
        let proto = proto_of(1, "ITEM_GIFTBOX", "");
        for category in [6u8, 7, 100, 255] {
            assert!(!is_custom_category(&proto, category));
        }
    }

    #[test]
    fn the_first_matching_category_wins() {
        // `CItem::GetItemCategory` is ascending and first-match. `25040` is in the category 2
        // list, so it is category 2 and not anything later.
        let proto = proto_of(25040, "ITEM_TOTEM", "");
        assert!(is_custom_category(&proto, 2));
        assert_eq!(item_category(&proto), Some(2));
    }

    #[test]
    fn an_item_in_no_category_reports_none() {
        // `None` is legacy's `-1`: not an error and not a full inventory, just "the base
        // inventory holds this".
        let proto = proto_of(1, "ITEM_ARMOR", "");
        assert_eq!(item_category(&proto), None);
    }

    #[test]
    fn every_owners_item_gets_a_category_and_the_two_answers_agree() {
        // The real data, all 7,305 rows of it.
        //
        // What this proves is **not** that every item lands somewhere. Most do not: an item
        // with no category is the ordinary case and belongs in the base inventory, so the
        // assertion is that the two ways of asking give the same answer for every row, and
        // that neither is answering for an item the scan could not name.
        let protos = owners();
        let mut in_a_category = 0usize;
        let mut in_none = 0usize;
        let mut in_two = 0usize;
        for proto in protos.rows() {
            let matches: Vec<u8> = (0..CATEGORY_NUM)
                .filter(|&c| is_custom_category(proto, c))
                .collect();
            let first = item_category(proto);
            assert_eq!(
                first,
                matches.first().copied(),
                "item_category and is_custom_category disagree for vnum {}",
                proto.vnum
            );
            // Legacy's own search walks every category and takes the first with a free cell
            // (`char_item.cpp:1214-1225`), so a row in two categories is a real case and not a
            // defect. What would be a defect is a row in *more* than two, which no arm of the
            // rule can produce.
            assert!(
                matches.len() <= 2,
                "vnum {} is in categories {matches:?}; the rule can place an item in at most two",
                proto.vnum
            );
            if matches.len() == 2 {
                in_two += 1;
            }
            if first.is_some() {
                in_a_category += 1;
            } else {
                in_none += 1;
            }
        }
        assert!(in_a_category > 0, "no item landed in a category at all");
        assert!(in_none > 0, "no item fell back to the base inventory");
        // These are a fingerprint of the owner's data, not a target. A change to the rule
        // moves them, and the test then says by how much instead of leaving someone to guess
        // whether 5 is still right. They were measured on 2026-09-27.
        assert_eq!(in_two, 5, "the number of items in two categories moved");
        assert_eq!(in_a_category + in_none, 7305, "the row count moved");
        eprintln!("owners: {in_a_category} in a category, {in_none} in the base inventory, {in_two} in two");
    }

    #[test]
    fn the_five_items_in_two_categories_are_the_ones_the_legacy_lists_overlap() {
        // The overlap is not a coincidence of the data: it is what the tables say. Each of
        // these is a vnum from one list *and* a type the other category accepts, so both arms
        // answer and legacy's `GetItemCategory` reports the lower one while its
        // `GetEmptyInventory` uses the first with room. Naming them keeps the count in the
        // test above honest.
        let protos = owners();
        let both: Vec<(u32, Vec<u8>)> = protos
            .rows()
            .iter()
            .filter_map(|proto| {
                let matches: Vec<u8> = (0..CATEGORY_NUM)
                    .filter(|&c| is_custom_category(proto, c))
                    .collect();
                (matches.len() == 2).then_some((proto.vnum, matches))
            })
            .collect();
        let vnums: Vec<u32> = both.iter().map(|&(v, _)| v).collect();
        assert_eq!(vnums, vec![27987, 30270, 55003, 55004, 55005]);
        // And the categories each pair is in, so a change in either rule is visible here
        // rather than only in the total.
        assert_eq!(
            both[0].1,
            vec![2, 3],
            "27987 is a gift box and is listed in category 2"
        );
        assert_eq!(
            both[1].1,
            vec![2, 3],
            "30270 is leather and is listed in category 3"
        );
        for entry in &both[2..] {
            assert_eq!(
                entry.1,
                vec![0, 2],
                "{} is leather and is listed in category 0",
                entry.0
            );
        }
    }

    #[test]
    fn the_sub_type_names_this_module_asks_for_are_in_the_table() {
        // The positive control for the `OnceLock` in `sub_index`. Without it, a rename in
        // `item_proto_value` would abort at first use and the abort would look like a crash
        // rather than a lost table.
        for (ty, name) in [
            ("ITEM_METIN", "METIN_NORMAL"),
            ("ITEM_MATERIAL", "MATERIAL_LEATHER"),
            ("ITEM_USE", "USE_AFFECT"),
            ("ITEM_USE", "USE_SPECIAL"),
            ("ITEM_USE", "USE_CLEAN_SOCKET"),
            ("ITEM_USE", "USE_CHANGE_ATTRIBUTE"),
            ("ITEM_USE", "USE_ADD_ATTRIBUTE"),
            ("ITEM_USE", "USE_ADD_ATTRIBUTE2"),
            ("ITEM_COSTUME", "COSTUME_BODY"),
            ("ITEM_COSTUME", "COSTUME_HAIR"),
            ("ITEM_COSTUME", "COSTUME_SASH"),
        ] {
            let resolved = sub_index(ty, name);
            // The index has to name a real slot, and it has to name *this* one. The first
            // assertion alone would pass for any non-negative number.
            let column = TYPE
                .iter()
                .position(|entry| *entry == ty)
                .expect("TYPE has the type");
            let slot = usize::try_from(resolved).expect("sub_index is never negative");
            assert_eq!(
                SUB_TYPE[column][slot], name,
                "{ty}/{name} resolved to the wrong slot"
            );
        }
    }
}
