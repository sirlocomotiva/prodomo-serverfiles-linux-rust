//! The item type, sub-type and limit numbers the game rules compare an [`ItemProto`] against.
//!
//! Each is the value of a legacy enum member in `server/server/common/item_length.h`:
//! `EItemTypes` (`:68-107`), `EWeaponSubTypes` (`:124-132`), `EArmorSubTypes` (`:136-148`),
//! `ECostumeSubTypes` (`:150-171`), `EItemAntiFlag` (`:376-396`), `EItemWearableFlag`
//! (`:398-425`) and `ELimitTypes` (`:427-451`). The Game data reader turns a name in
//! `item_proto.txt` into its
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
/// `ITEM_USE`, an item used up by `CG_ITEM_USE`: potions among others.
pub const ITEM_USE: i32 = 3;
/// `ITEM_MATERIAL`, a refine material.
pub const ITEM_MATERIAL: i32 = 5;
/// `ITEM_ELK`, gold as an item, which `CreateItem` neither numbers nor counts.
pub const ITEM_ELK: i32 = 9;
/// `ITEM_METIN`, a stone that sits in another item's socket.
pub const ITEM_METIN: i32 = 10;
/// `ITEM_FISH`, a caught fish, which the campfire grills.
pub const ITEM_FISH: i32 = 12;
/// `ITEM_ROD`, a fishing rod.
pub const ITEM_ROD: i32 = 13;
/// `FISH_ALIVE`.
pub const FISH_ALIVE: i32 = 0;
/// `FISH_DEAD`.
pub const FISH_DEAD: i32 = 1;
/// `ITEM_CAMPFIRE`, which `CG_ITEM_USE` stands on the ground.
pub const ITEM_CAMPFIRE: i32 = 15;
/// `ITEM_UNIQUE`.
pub const ITEM_UNIQUE: i32 = 16;
/// `ITEM_SKILLBOOK` (17): a master-training book, read by `LearnSkillByBook`.
pub const ITEM_SKILLBOOK: i32 = 17;
/// `ITEM_SKILLFORGET`, a book that makes a skill forgotten.
pub const ITEM_SKILLFORGET: i32 = 22;
/// `ITEM_QUEST`, an item a quest reads.
pub const ITEM_QUEST: i32 = 18;
/// `ITEM_GIFTBOX` (23): a box whose use draws its special group.
pub const ITEM_GIFTBOX: i32 = 23;
/// `ITEM_POLYMORPH` (19): a ball or book that morphs its user.
pub const ITEM_POLYMORPH: i32 = 19;
/// `ITEM_TREASURE_BOX` (20): a box a matching key opens.
pub const ITEM_TREASURE_BOX: i32 = 20;
/// `ITEM_TREASURE_KEY` (21): a key a matching box takes.
pub const ITEM_TREASURE_KEY: i32 = 21;
/// `ITEM_PICK`, a pickaxe.
pub const ITEM_PICK: i32 = 24;
/// `ITEM_TOTEM`, which `FindEquipCell` refuses whatever its wear flags say.
pub const ITEM_TOTEM: i32 = 26;
/// `ITEM_BLEND`, a potion `CreateItem` gives a bonus from the blend table.
pub const ITEM_BLEND: i32 = 27;
/// `ITEM_COSTUME`.
pub const ITEM_COSTUME: i32 = 28;
/// `ITEM_DS`, a dragon soul stone.
pub const ITEM_DS: i32 = 29;
/// `ITEM_SPECIAL_DS`.
pub const ITEM_SPECIAL_DS: i32 = 30;
/// `ITEM_EXTRACT` (31): an extractor used on a dragon soul stone.
pub const ITEM_EXTRACT: i32 = 31;
/// `EXTRACT_DRAGON_SOUL`: the extractor that pulls a worn stone out (`PullOut`).
pub const EXTRACT_DRAGON_SOUL: i32 = 0;
/// `EXTRACT_DRAGON_HEART`: the extractor that melts a stone into a charged heart.
pub const EXTRACT_DRAGON_HEART: i32 = 1;
/// `ITEM_SECONDARY_COIN` (32): the secondary-coin items a renewal buy with a
/// `SHOPEX_SECONDARY` price spends (`G/shopEx.cpp:180-182`).
pub const ITEM_SECONDARY_COIN: i32 = 32;
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

/// `WEAPON_SWORD`, the one-handed sword: its stance is `MOTION_MODE_ONEHAND_SWORD`.
pub const WEAPON_SWORD: i32 = 0;
/// `WEAPON_DAGGER`: its stance is `MOTION_MODE_DUALHAND_SWORD`.
pub const WEAPON_DAGGER: i32 = 1;
/// `WEAPON_BOW`: its stance is `MOTION_MODE_BOW`.
pub const WEAPON_BOW: i32 = 2;
/// `WEAPON_TWO_HANDED`: its stance is `MOTION_MODE_TWOHAND_SWORD`.
pub const WEAPON_TWO_HANDED: i32 = 3;
/// `WEAPON_BELL`: its stance is `MOTION_MODE_BELL`.
pub const WEAPON_BELL: i32 = 4;
/// `WEAPON_FAN`: its stance is `MOTION_MODE_FAN`.
pub const WEAPON_FAN: i32 = 5;
/// `WEAPON_ARROW`: arrows roll no magic attributes (`item_attribute.cpp:18-19`).
pub const WEAPON_ARROW: i32 = 6;
/// `WEAPON_MOUNT_SPEAR`, the mounted spear: `CalcMeleeDamage` swings it as a melee weapon.
pub const WEAPON_MOUNT_SPEAR: i32 = 7;

/// `USE_POTION`: a potion whose recovery runs over the following seconds.
pub const USE_POTION: i32 = 0;
/// `USE_TALISMAN` (`C/item_length.h:255`): 1. A town or memory portal, which marks
/// or recalls (the owner's 22000/22001 mark town, 22010/22011/22020/22030 memory).
pub const USE_TALISMAN: i32 = 1;
/// `USE_TUNING`: a refine scroll, which the smith refuses to refine.
pub const USE_TUNING: i32 = 2;
/// `USE_DETACHMENT` (`C/item_length.h:268`): a stone or sash detachment scroll.
pub const USE_DETACHMENT: i32 = 14;
/// `USE_CLEAN_SOCKET` (`C/item_length.h:271`): a broken-stone cleaner (the owner's
/// 39027 and 71083).
pub const USE_CLEAN_SOCKET: i32 = 17;
/// `USE_ELEMENT_UPGRADE` (`C/item_length.h:293`): an element upgrade scroll.
pub const USE_ELEMENT_UPGRADE: i32 = 36;
/// `USE_ELEMENT_DOWNGRADE` (`C/item_length.h:294`): an element downgrade scroll.
pub const USE_ELEMENT_DOWNGRADE: i32 = 37;
/// `USE_ELEMENT_CHANGE` (`C/item_length.h:295`): an element change scroll.
pub const USE_ELEMENT_CHANGE: i32 = 38;
/// `USE_BAIT`: a bait, which `UseItemEx` puts on the worn rod (`G/char_item.cpp:6800-6825`).
pub const USE_BAIT: i32 = 6;
/// `USE_ABILITY_UP`: a potion that raises a point for a while.
pub const USE_ABILITY_UP: i32 = 7;
/// `USE_AFFECT` (`C/item_length.h:262`): a timed affect by value (50813/50814 ride
/// the 508xx branch; the rest land in the main switch).
pub const USE_AFFECT: i32 = 8;
/// `USE_POTION_NODELAY`: a potion whose recovery is immediate.
pub const USE_POTION_NODELAY: i32 = 11;
/// `USE_SPECIAL` (`C/item_length.h:264`): 10. The skill books' sub type (all the 230g book
/// vnums carry it).
pub const USE_SPECIAL: i32 = 10;
/// `USE_CHANGE_ATTRIBUTE` (`C/item_length.h:272`): a bonus change scroll.
pub const USE_CHANGE_ATTRIBUTE: i32 = 18;
/// `USE_ADD_ATTRIBUTE` (`C/item_length.h:273`): a bonus add scroll.
pub const USE_ADD_ATTRIBUTE: i32 = 19;
/// `USE_ADD_ACCESSORY_SOCKET` (`C/item_length.h:274`): a socket-adding scroll.
pub const USE_ADD_ACCESSORY_SOCKET: i32 = 20;
/// `USE_PUT_INTO_ACCESSORY_SOCKET` (`C/item_length.h:275`): a jewel for an
/// accessory's socket.
pub const USE_PUT_INTO_ACCESSORY_SOCKET: i32 = 21;
/// `USE_ADD_ATTRIBUTE2` (`C/item_length.h:276`): a 5th-bonus add scroll.
pub const USE_ADD_ATTRIBUTE2: i32 = 22;
/// `USE_CHANGE_ATTRIBUTE2` (`C/item_length.h:278`), behind `__ATTR_6TH_7TH__`: a rare
/// bonus change scroll.
pub const USE_CHANGE_ATTRIBUTE2: i32 = 24;
/// `USE_ADD_ATTRIBUTE_TALISMAN` (`C/item_length.h:301`): a talisman bonus-add scroll
/// (the owner's 50890).
pub const USE_ADD_ATTRIBUTE_TALISMAN: i32 = 43;
/// `USE_CHANGE_ATTRIBUTE_TALISMAN` (`C/item_length.h:302`): a talisman bonus-change
/// scroll (the owner's 50889).
pub const USE_CHANGE_ATTRIBUTE_TALISMAN: i32 = 44;
/// `USE_ADD_ATTRIBUTE_GLOVE` (`C/item_length.h:303`), behind `ENABLE_GLOVE_SYSTEM`: a
/// glove bonus-add scroll (the owner's 50891).
pub const USE_ADD_ATTRIBUTE_GLOVE: i32 = 45;
/// `USE_CHANGE_ATTRIBUTE_GLOVE` (`C/item_length.h:304`), behind `ENABLE_GLOVE_SYSTEM`:
/// a glove bonus-change scroll (the owner's 50892).
pub const USE_CHANGE_ATTRIBUTE_GLOVE: i32 = 46;
/// `USE_CHANGE_COSTUME_ATTR` (`C/item_length.h:285`): a costume bonus-change scroll
/// (the owner's 70064).
pub const USE_CHANGE_COSTUME_ATTR: i32 = 31;
/// `USE_RESET_COSTUME_ATTR` (`C/item_length.h:286`): a costume bonus-reset scroll
/// (the owner's 70063).
pub const USE_RESET_COSTUME_ATTR: i32 = 32;
/// `USE_SET_ATT_COSTUME` (`C/item_length.h:297`), behind `NEW_ATTR_RANFORSARI`: a
/// costume-body reinforcement (the owner's 50898).
pub const USE_SET_ATT_COSTUME: i32 = 39;
/// `USE_SET_ATT_PET` (`C/item_length.h:298`), behind `NEW_ATTR_RANFORSARI`: a
/// costume-pet reinforcement (the owner's 50897).
pub const USE_SET_ATT_PET: i32 = 40;
/// `USE_SET_ATT_MOUNT` (`C/item_length.h:299`), behind `NEW_ATTR_RANFORSARI`: a
/// costume-mount reinforcement (the owner's 50896).
pub const USE_SET_ATT_MOUNT: i32 = 41;
/// `USE_SET_ATT_COSTUME_WEAPON` (`C/item_length.h:300`), behind
/// `NEW_ATTR_RANFORSARI`: a costume-weapon reinforcement (no owner scroll).
pub const USE_SET_ATT_COSTUME_WEAPON: i32 = 42;
/// `USE_PUT_INTO_BELT_SOCKET` (`C/item_length.h:283`): a jewel for a belt's socket.
pub const USE_PUT_INTO_BELT_SOCKET: i32 = 29;
/// `USE_CLEAR` (`C/item_length.h:266`): a bad-affect cure (27124, 27874).
pub const USE_CLEAR: i32 = 12;
/// `USE_INVISIBILITY` (`C/item_length.h:267`): a 300 s cloak (27877, 70026).
pub const USE_INVISIBILITY: i32 = 13;
/// `USE_UNLOCK_SHAMAN` (`C/item_length.h:306`): 47. The buff-premium item (69699), which
/// legacy answers with the success line and keeps (520: its training arm is commented out).
pub const USE_UNLOCK_SHAMAN: i32 = 47;
/// `TOGGLE_SHAMAN` (`C/item_length.h:118`): 0, the only `EToggleSubTypes` member. The buff
/// follower's summon item (69700).
pub const TOGGLE_SHAMAN: i32 = 0;

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
/// `LIMIT_STR`: the wearer's strength must be at least the limit's value.
pub const LIMIT_STR: i32 = 2;
/// `LIMIT_DEX`: the wearer's dexterity must be at least the limit's value.
pub const LIMIT_DEX: i32 = 3;
/// `LIMIT_INT`: the wearer's intelligence must be at least the limit's value.
pub const LIMIT_INT: i32 = 4;
/// `LIMIT_CON`: the wearer's vitality must be at least the limit's value.
pub const LIMIT_CON: i32 = 5;
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
/// `LIMIT_TIMER_BASED_ON_WEAR` (`item_length.h:447`): the item's time runs while it is worn.
pub const LIMIT_TIMER_BASED_ON_WEAR: i32 = 8;
/// `LIMIT_CHAMPION` (`item_length.h:448`): the wearer's conqueror level must be at least the
/// limit's value.
pub const LIMIT_CHAMPION: i32 = 9;

/// `ITEM_ANTIFLAG_FEMALE`: a female character may not wear it.
pub const ITEM_ANTIFLAG_FEMALE: u32 = 1 << 0;
/// `ITEM_ANTIFLAG_MALE`: a male character may not wear it.
pub const ITEM_ANTIFLAG_MALE: u32 = 1 << 1;
/// `ITEM_ANTIFLAG_WARRIOR`, which the file spells `ANTI_MUSA`.
pub const ITEM_ANTIFLAG_WARRIOR: u32 = 1 << 2;
/// `ITEM_ANTIFLAG_ASSASSIN`.
pub const ITEM_ANTIFLAG_ASSASSIN: u32 = 1 << 3;
/// `ITEM_ANTIFLAG_SURA`.
pub const ITEM_ANTIFLAG_SURA: u32 = 1 << 4;
/// `ITEM_ANTIFLAG_SHAMAN`, which the file spells `ANTI_MUDANG`.
pub const ITEM_ANTIFLAG_SHAMAN: u32 = 1 << 5;

/// `WEARABLE_BODY`.
pub const WEARABLE_BODY: u32 = 1 << 0;
/// `WEARABLE_HEAD`.
pub const WEARABLE_HEAD: u32 = 1 << 1;
/// `WEARABLE_FOOTS`.
pub const WEARABLE_FOOTS: u32 = 1 << 2;
/// `WEARABLE_WRIST`.
pub const WEARABLE_WRIST: u32 = 1 << 3;
/// `WEARABLE_WEAPON`.
pub const WEARABLE_WEAPON: u32 = 1 << 4;
/// `WEARABLE_NECK`.
pub const WEARABLE_NECK: u32 = 1 << 5;
/// `WEARABLE_EAR`.
pub const WEARABLE_EAR: u32 = 1 << 6;
/// `WEARABLE_UNIQUE` (`item_length.h:407`).
///
/// The reader's name table puts `WEAR_UNIQUE` at bit 8 and `WEAR_SHIELD` at bit 7
/// (`arWearrFlag`, `server/server/db/ProtoReader.cpp:426`), the other way round from this enum,
/// so an item whose file cell says `WEAR_SHIELD` gets this bit. The owner's `item_proto.txt` is
/// written for that: its shields say `WEAR_UNIQUE` and its unique accessories say `WEAR_SHIELD`.
/// The tests pin the swap, because "fixing" either side alone would put every shield in a
/// unique cell.
pub const WEARABLE_UNIQUE: u32 = 1 << 7;
/// `WEARABLE_SHIELD` (`item_length.h:408`); see [`WEARABLE_UNIQUE`] for its file spelling.
pub const WEARABLE_SHIELD: u32 = 1 << 8;
/// `WEARABLE_ARROW`.
pub const WEARABLE_ARROW: u32 = 1 << 9;
/// `WEARABLE_ABILITY`.
pub const WEARABLE_ABILITY: u32 = 1 << 11;
/// `WEARABLE_FIRE`, the fire talisman.
pub const WEARABLE_FIRE: u32 = 1 << 13;
/// `WEARABLE_ICE`.
pub const WEARABLE_ICE: u32 = 1 << 14;
/// `WEARABLE_EARTH`.
pub const WEARABLE_EARTH: u32 = 1 << 15;
/// `WEARABLE_DARK`.
pub const WEARABLE_DARK: u32 = 1 << 16;
/// `WEARABLE_WIND`.
pub const WEARABLE_WIND: u32 = 1 << 17;
/// `WEARABLE_ELEC`.
pub const WEARABLE_ELEC: u32 = 1 << 18;
/// `WEARABLE_GLOVE` (`ENABLE_GLOVE_SYSTEM`).
pub const WEARABLE_GLOVE: u32 = 1 << 19;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item_proto_value::{ANTI_FLAG, LIMIT_TYPE, SUB_TYPE, TYPE, WEAR_FLAG};

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
            (ITEM_USE, "ITEM_USE"),
            (ITEM_MATERIAL, "ITEM_MATERIAL"),
            (ITEM_ELK, "ITEM_ELK"),
            (ITEM_METIN, "ITEM_METIN"),
            (ITEM_FISH, "ITEM_FISH"),
            (ITEM_ROD, "ITEM_ROD"),
            (ITEM_CAMPFIRE, "ITEM_CAMPFIRE"),
            (ITEM_UNIQUE, "ITEM_UNIQUE"),
            (ITEM_SKILLBOOK, "ITEM_SKILLBOOK"),
            (ITEM_SKILLFORGET, "ITEM_SKILLFORGET"),
            (ITEM_QUEST, "ITEM_QUEST"),
            (ITEM_TREASURE_BOX, "ITEM_TREASURE_BOX"),
            (ITEM_TREASURE_KEY, "ITEM_TREASURE_KEY"),
            (ITEM_GIFTBOX, "ITEM_GIFTBOX"),
            (ITEM_PICK, "ITEM_PICK"),
            (ITEM_TOTEM, "ITEM_TOTEM"),
            (ITEM_BLEND, "ITEM_BLEND"),
            (ITEM_COSTUME, "ITEM_COSTUME"),
            (ITEM_DS, "ITEM_DS"),
            (ITEM_SPECIAL_DS, "ITEM_SPECIAL_DS"),
            (ITEM_EXTRACT, "ITEM_EXTRACT"),
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
        let weapon = SUB_TYPE[usize::try_from(ITEM_WEAPON).expect("a small index")];
        for (value, name) in [
            (WEAPON_SWORD, "WEAPON_SWORD"),
            (WEAPON_DAGGER, "WEAPON_DAGGER"),
            (WEAPON_BOW, "WEAPON_BOW"),
            (WEAPON_TWO_HANDED, "WEAPON_TWO_HANDED"),
            (WEAPON_BELL, "WEAPON_BELL"),
            (WEAPON_FAN, "WEAPON_FAN"),
            (WEAPON_MOUNT_SPEAR, "WEAPON_MOUNT_SPEAR"),
        ] {
            assert_eq!(value, index(weapon, name), "{name}");
        }
        let fish = SUB_TYPE[usize::try_from(ITEM_FISH).expect("a small index")];
        for (value, name) in [(FISH_ALIVE, "FISH_ALIVE"), (FISH_DEAD, "FISH_DEAD")] {
            assert_eq!(value, index(fish, name), "{name}");
        }
        let usable = SUB_TYPE[usize::try_from(ITEM_USE).expect("a small index")];
        for (value, name) in [
            (USE_POTION, "USE_POTION"),
            (USE_TALISMAN, "USE_TALISMAN"),
            (USE_TUNING, "USE_TUNING"),
            (USE_DETACHMENT, "USE_DETACHMENT"),
            (USE_CLEAN_SOCKET, "USE_CLEAN_SOCKET"),
            (USE_ABILITY_UP, "USE_ABILITY_UP"),
            (USE_POTION_NODELAY, "USE_POTION_NODELAY"),
            (USE_BAIT, "USE_BAIT"),
            (USE_SPECIAL, "USE_SPECIAL"),
            (USE_CHANGE_ATTRIBUTE, "USE_CHANGE_ATTRIBUTE"),
            (USE_ADD_ATTRIBUTE, "USE_ADD_ATTRIBUTE"),
            (USE_ADD_ACCESSORY_SOCKET, "USE_ADD_ACCESSORY_SOCKET"),
            (
                USE_PUT_INTO_ACCESSORY_SOCKET,
                "USE_PUT_INTO_ACCESSORY_SOCKET",
            ),
            (USE_ADD_ATTRIBUTE2, "USE_ADD_ATTRIBUTE2"),
            (USE_CHANGE_ATTRIBUTE2, "USE_CHANGE_ATTRIBUTE2"),
            (USE_PUT_INTO_BELT_SOCKET, "USE_PUT_INTO_BELT_SOCKET"),
            (USE_ADD_ATTRIBUTE_TALISMAN, "USE_ADD_ATTRIBUTE_TALISMAN"),
            (
                USE_CHANGE_ATTRIBUTE_TALISMAN,
                "USE_CHANGE_ATTRIBUTE_TALISMAN",
            ),
            (USE_ADD_ATTRIBUTE_GLOVE, "USE_ADD_ATTRIBUTE_GLOVE"),
            (USE_CHANGE_ATTRIBUTE_GLOVE, "USE_CHANGE_ATTRIBUTE_GLOVE"),
            (USE_CHANGE_COSTUME_ATTR, "USE_CHANGE_COSTUME_ATTR"),
            (USE_RESET_COSTUME_ATTR, "USE_RESET_COSTUME_ATTR"),
            (USE_SET_ATT_COSTUME, "USE_SET_ATT_COSTUME"),
            (USE_SET_ATT_PET, "USE_SET_ATT_PET"),
            (USE_SET_ATT_MOUNT, "USE_SET_ATT_MOUNT"),
            (USE_SET_ATT_COSTUME_WEAPON, "USE_SET_ATT_COSTUME_WEAPON"),
        ] {
            assert_eq!(value, index(usable, name), "{name}");
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
        let extract = SUB_TYPE[usize::try_from(ITEM_EXTRACT).expect("a small index")];
        for (value, name) in [
            (EXTRACT_DRAGON_SOUL, "EXTRACT_DRAGON_SOUL"),
            (EXTRACT_DRAGON_HEART, "EXTRACT_DRAGON_HEART"),
        ] {
            assert_eq!(value, index(extract, name), "{name}");
        }
    }

    /// The limit table spells two members differently from the enum, so those two are pinned
    /// to the table's own spelling: the reader stores the table index.
    #[test]
    fn each_limit_is_its_name_index() {
        for (value, name) in [
            (LIMIT_NONE, "LIMIT_NONE"),
            (LIMIT_LEVEL, "LEVEL"),
            (LIMIT_STR, "STR"),
            (LIMIT_DEX, "DEX"),
            (LIMIT_INT, "INT"),
            (LIMIT_CON, "CON"),
            (LIMIT_REAL_TIME, "REAL_TIME"),
            (LIMIT_REAL_TIME_START_FIRST_USE, "REAL_TIME_FIRST_USE"),
            (LIMIT_TIMER_BASED_ON_WEAR, "TIMER_BASED_ON_WEAR"),
            (LIMIT_CHAMPION, "CHAMPION"),
        ] {
            assert_eq!(value, index(LIMIT_TYPE, name), "{name}");
        }
    }

    /// The bit a name in the file's flag column sets.
    fn bit(table: &[&str], name: &str) -> u32 {
        1 << index(table, name)
    }

    #[test]
    fn each_anti_flag_is_the_bit_of_its_file_name() {
        for (value, name) in [
            (ITEM_ANTIFLAG_FEMALE, "ANTI_FEMALE"),
            (ITEM_ANTIFLAG_MALE, "ANTI_MALE"),
            (ITEM_ANTIFLAG_WARRIOR, "ANTI_MUSA"),
            (ITEM_ANTIFLAG_ASSASSIN, "ANTI_ASSASSIN"),
            (ITEM_ANTIFLAG_SURA, "ANTI_SURA"),
            (ITEM_ANTIFLAG_SHAMAN, "ANTI_MUDANG"),
        ] {
            assert_eq!(value, bit(ANTI_FLAG, name), "{name}");
        }
    }

    /// Every wear bit is the bit of its own file name except the two the reader swaps, which
    /// are pinned crossed: `WEAR_SHIELD` in the file is `WEARABLE_UNIQUE` in the game.
    #[test]
    fn each_wear_flag_is_the_bit_of_its_file_name_with_shield_and_unique_crossed() {
        for (value, name) in [
            (WEARABLE_BODY, "WEAR_BODY"),
            (WEARABLE_HEAD, "WEAR_HEAD"),
            (WEARABLE_FOOTS, "WEAR_FOOTS"),
            (WEARABLE_WRIST, "WEAR_WRIST"),
            (WEARABLE_WEAPON, "WEAR_WEAPON"),
            (WEARABLE_NECK, "WEAR_NECK"),
            (WEARABLE_EAR, "WEAR_EAR"),
            (WEARABLE_UNIQUE, "WEAR_SHIELD"),
            (WEARABLE_SHIELD, "WEAR_UNIQUE"),
            (WEARABLE_ARROW, "WEAR_ARROW"),
            (WEARABLE_ABILITY, "WEAR_ABILITY"),
            (WEARABLE_FIRE, "WEAR_TALISMAN_FIRE"),
            (WEARABLE_ICE, "WEAR_TALISMAN_ICE"),
            (WEARABLE_EARTH, "WEAR_TALISMAN_EARTH"),
            (WEARABLE_DARK, "WEAR_TALISMAN_DARK"),
            (WEARABLE_WIND, "WEAR_TALISMAN_WIND"),
            (WEARABLE_ELEC, "WEAR_TALISMAN_ELEC"),
            (WEARABLE_GLOVE, "WEAR_GLOVE"),
        ] {
            assert_eq!(value, bit(WEAR_FLAG, name), "{name}");
        }
    }
}
