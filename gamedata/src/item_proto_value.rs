//! The name-to-value tables the legacy DB server uses to read an item proto column.
//!
//! Legacy `Set_Proto_Item_Table` (`server/server/db/ProtoReader.cpp:861-1009`) reads 33 columns
//! and, for nine of them, does not use `str_to_number`: it hands the field to one of the eight
//! functions in this module, each of which is a linear scan over a C string array whose **array
//! index is the value**. A value of `-1` means "not found", and `Set_Proto_Item_Table` reacts to
//! `-1` by logging and calling `exit(0)` (`:906-923`). A Rewrite that copied that would kill the
//! process, so every resolver here returns an `Option` and the reader refuses the row.
//!
//! Three of the eight do not return `-1` at all, and that asymmetry is load-bearing:
//!
//! - [`type_value`] matches in **both** directions (`:92`): the field must contain the table entry
//!   *and* the table entry must contain the field, and it takes the **lowest** index that matches.
//!   A field of `ITEM` therefore resolves to `ITEM_NONE` at index 0.
//! - The four flag columns are **bitmasks**. The field is split on `|`, each token is compared for
//!   equality against the whole table, and the sum of `1 << i` over the matches is the value
//!   (`:380-390`, and the same shape in the other three). A name absent from the table contributes
//!   nothing, so a typo in the data silently loses a flag.
//! - [`sub_type_value`] returns `0` for a type with no sub-type table registered (`:344-347`),
//!   which is a value rather than a failure.
//!
//! The `APPLY_*` table has four entries behind `#if defined(__CONQUEROR_LEVEL__)` (`:531-536`).
//! `server/server/common/prodomodefines.h:53` defines it, so the four `APPLY_SUNGMA_*` names are in
//! the table at indices 98..=101 and every name after them sits four positions higher than the
//! stock Metin2 table. Reading that switch the other way would silently apply the wrong bonus to
//! most items in the game, so [`APPLY_TYPE`] carries them and a test pins the indices around them.
//!
//! Every field is a `&[u8]`, never a `&str`: the protos are read in a legacy code page
//! (`legacy/config/db/conf.txt` sets `LOCALE = latin1`) and AGENTS.md forbids transcoding them.

use std::error::Error;
use std::fmt;

/// The bytes `ProtoReader::trim` (`ProtoReader.cpp:14-24`) removes from both ends of a field.
const TRIMMED: &[u8] = b" \t\x0b\n\r";

/// The size of the buffer legacy's `StringSplit` allocates (`:31`).
///
/// Legacy writes one `std::string` per token into `new string[30]` and never checks the index, so
/// a field with more than 30 tokens is a buffer overrun. [`split_flags`] refuses such a field
/// instead, and [`ItemProto`](crate::item_proto::ItemProto)'s reader reports it.
pub const MAX_FLAG_TOKENS: usize = 30;

/// What a sub-type column resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubType {
    /// The type has no sub-type table, so legacy returns `0` (`ProtoReader.cpp:344-347`).
    ///
    /// A `SUB_TYPE` of `0` is the common spelling for these, and it means the same thing.
    Unregistered,
    /// The index of the matching name in the type's table.
    Value(i32),
    /// The name is not in the type's table; legacy returns `-1`.
    Unknown,
    /// The type itself is out of range, which legacy logs and treats as `-1` (`:329-333`).
    TypeOutOfRange,
}

/// A flag column with more tokens than legacy's `StringSplit` buffer holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TooManyFlagTokens {
    /// The number of tokens found.
    pub found: usize,
    /// The number legacy can hold.
    pub limit: usize,
}

impl fmt::Display for TooManyFlagTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a flag column has {} `|`-separated tokens; legacy's StringSplit buffer holds {}",
            self.found, self.limit
        )
    }
}

impl Error for TooManyFlagTokens {}

/// `ProtoReader::trim`: the field without leading or trailing whitespace.
///
/// Legacy builds a `std::string` from the field and trims `" \t\v\n\r"` from both ends
/// (`ProtoReader.cpp:14-24`). A field of only whitespace becomes empty.
pub fn trim(field: &[u8]) -> &[u8] {
    let start = field
        .iter()
        .position(|b| !TRIMMED.contains(b))
        .unwrap_or(field.len());
    let end = field
        .iter()
        .rposition(|b| !TRIMMED.contains(b))
        .map_or(start, |i| i + 1);
    &field[start..end]
}

/// `StringSplit(field, "|")` (`ProtoReader.cpp:26-53`), trimmed.
///
/// Legacy drops every empty token, so `"|A||B|"` yields `["A", "B"]` and a field of only
/// separators yields nothing. Each token is then trimmed.
pub fn split_flags(field: &[u8]) -> Vec<&[u8]> {
    field
        .split(|b| *b == b'|')
        .filter(|token| !token.is_empty())
        .map(trim)
        .collect()
}

/// `get_Item_Type_Value` (`ProtoReader.cpp:57-102`): the index of `ITEM_TYPE`.
///
/// Legacy tests `inputString.find(entry) != npos && entry.find(inputString) != npos` (`:92`), which
/// reads like a substring match but is not one: for two non-empty strings, each containing the other
/// means they are **equal**. So this is an exact match on the **untrimmed** field, against a
/// function-local array of 37 names, and it breaks at the first hit.
///
/// Two details follow from the code rather than from its intent. An empty field never matches,
/// because `entry.find("")` is 0 but `"".find(entry)` is `npos`. And a field with surrounding
/// whitespace does not match either, unlike every other column in the proto: `get_Item_Type_Value`
/// is the one resolver that does not call `trim` first, so `csv_table` stripping the line ends is
/// what keeps the file loadable.
pub fn type_value(field: &[u8]) -> Option<i32> {
    let index = TYPE.iter().position(|entry| entry.as_bytes() == field)?;
    // A table index is never negative and `TYPE` has 37 entries, so the conversion is exact.
    i32::try_from(index).ok()
}

/// `get_Item_SubType_Value` (`ProtoReader.cpp:104-359`): the index of `SUB_TYPE` within `type_`.
///
/// `type_` is a resolved [`type_value`]. Legacy does not use a `switch` here: it holds a 41-entry
/// array of sub-type name tables (`arSubType`, `ProtoReader.cpp:242`) plus a parallel count array,
/// and scans the table for the type with `compare` equality against `trim(field)`
/// (`ProtoReader.cpp:346-355`). An index past the array is refused with `-1`
/// (`ProtoReader.cpp:332-336`); an index inside it with a zero count yields
/// [`SubType::Unregistered`], which is how an item with no sub-types keeps a `SUB_TYPE` of `0`.
pub fn sub_type_value(type_: i32, field: &[u8]) -> SubType {
    // A negative `bType` is not an index, and `usize::try_from` refuses it the same way a C++
    // `std::vector::operator[]` with a negative subscript is undefined but in practice out of
    // bounds, so the arm is a refusal rather than a lookup.
    let Ok(index) = usize::try_from(type_) else {
        return SubType::TypeOutOfRange;
    };
    let Some(subtypes) = SUB_TYPE.get(index) else {
        return SubType::TypeOutOfRange;
    };
    if subtypes.is_empty() {
        return SubType::Unregistered;
    }
    let field = trim(field);
    subtypes
        .iter()
        .position(|entry| entry.as_bytes() == field)
        .and_then(|i| i32::try_from(i).ok())
        .map_or(SubType::Unknown, SubType::Value)
}

/// `get_Item_LimitType_Value` (`ProtoReader.cpp:488-507`): the index of a `LIMIT_TYPE` column.
///
/// Unlike [`type_value`], this compares for equality, against the trimmed field, and breaks at the
/// first match.
pub fn limit_type_value(field: &[u8]) -> Option<i32> {
    exact_index(LIMIT_TYPE, field)
}

/// `get_Item_ApplyType_Value` (`ProtoReader.cpp:510-583`): the index of an `ADDON_TYPE` column.
pub fn apply_type_value(field: &[u8]) -> Option<i32> {
    exact_index(APPLY_TYPE, field)
}

/// The bitmask legacy builds for the four flag columns.
///
/// `ANTI_FLAG`, `FLAG`, `ITEM_WEAR` and `IMMUNE` share one shape (`ProtoReader.cpp:365-483`): the
/// field is split on `|`, every token is compared for equality against every name in the table, and
/// the result is the sum of `1 << i` over the matches. Legacy accumulates with
/// `pow((float)2, (float)i)`, which is exact for the 21 names the widest table has, and it never
/// returns `-1`: a name that is not in the table is ignored rather than refused.
///
/// # Errors
///
/// Returns [`TooManyFlagTokens`] for a field with more than 30 tokens, which would overrun legacy's
/// `StringSplit` buffer.
pub fn flag_mask(field: &[u8], table: &[&str]) -> Result<u32, TooManyFlagTokens> {
    let tokens = split_flags(field);
    if tokens.len() > MAX_FLAG_TOKENS {
        return Err(TooManyFlagTokens {
            found: tokens.len(),
            limit: MAX_FLAG_TOKENS,
        });
    }
    // The four `DWORD` fields are assigned from the resolver's `int` return, and the widest table
    // has 21 names, so the mask never reaches bit 31 and the reinterpretation is the same value.
    // The `take` is what a name at index 32 or above would need: `pow((float)2, (float)i)` is a
    // `double` assigned to an `int`, which is not a bit past 31, so a 33rd name is not expressible
    // and is dropped rather than given a bit that legacy never sets.
    let mut mask = 0u32;
    for (bit, entry) in table.iter().take(u32::BITS as usize).enumerate() {
        if tokens.iter().any(|token| *token == entry.as_bytes()) {
            mask |= 1 << bit;
        }
    }
    Ok(mask)
}

/// The index of the first entry equal to the trimmed field.
fn exact_index(table: &[&str], field: &[u8]) -> Option<i32> {
    let field = trim(field);
    let index = table.iter().position(|entry| entry.as_bytes() == field)?;
    i32::try_from(index).ok()
}

/// The `ITEM_TYPE` names, indexed by `EItemTypes` (`ProtoReader.cpp:60-88`).
///
/// A function-local array of 37 names, so the compiler fixes the length and a test pins it.
/// `ITEM_TOGGLE` at index 36 has no `EItemTypes` member, which is why the comment at `:82`
/// says the empire enum only covers 34.
pub const TYPE: &[&str] = &[
    "ITEM_NONE",
    "ITEM_WEAPON",
    "ITEM_ARMOR",
    "ITEM_USE",
    "ITEM_AUTOUSE",
    "ITEM_MATERIAL",
    "ITEM_SPECIAL",
    "ITEM_TOOL",
    "ITEM_LOTTERY",
    "ITEM_ELK",
    "ITEM_METIN",
    "ITEM_CONTAINER",
    "ITEM_FISH",
    "ITEM_ROD",
    "ITEM_RESOURCE",
    "ITEM_CAMPFIRE",
    "ITEM_UNIQUE",
    "ITEM_SKILLBOOK",
    "ITEM_QUEST",
    "ITEM_POLYMORPH",
    "ITEM_TREASURE_BOX",
    "ITEM_TREASURE_KEY",
    "ITEM_SKILLFORGET",
    "ITEM_GIFTBOX",
    "ITEM_PICK",
    "ITEM_HAIR",
    "ITEM_TOTEM",
    "ITEM_BLEND",
    "ITEM_COSTUME",
    "ITEM_DS",
    "ITEM_SPECIAL_DS",
    "ITEM_EXTRACT",
    "ITEM_SECONDARY_COIN",
    "ITEM_RING",
    "ITEM_BELT",
    "ITEM_TALISMAN",
    "ITEM_TOGGLE",
];

/// The `ANTI_FLAG` names, whose bits become `dwAntiFlags` (`ProtoReader.cpp:365-393`).
///
/// 21 names, so `1 << 20` is the highest bit an item can carry.
pub const ANTI_FLAG: &[&str] = &[
    "ANTI_FEMALE",
    "ANTI_MALE",
    "ANTI_MUSA",
    "ANTI_ASSASSIN",
    "ANTI_SURA",
    "ANTI_MUDANG",
    "ANTI_GET",
    "ANTI_DROP",
    "ANTI_SELL",
    "ANTI_EMPIRE_A",
    "ANTI_EMPIRE_B",
    "ANTI_EMPIRE_C",
    "ANTI_SAVE",
    "ANTI_GIVE",
    "ANTI_PKDROP",
    "ANTI_STACK",
    "ANTI_MYSHOP",
    "ANTI_SAFEBOX",
    "ANTI_WOLFMAN",
    "ANTI_PET20",
    "ANTI_PET21",
];

/// The `FLAG` names, whose bits become `dwFlags` (`ProtoReader.cpp:395-422`).
///
/// 19 names. `ITEM_FLAG_QUEST_USE` and `ITEM_FLAG_QUEST_USE_MULTIPLE` are read out of this mask
/// by `ITEM_MANAGER::Initialize` (`item_manager.cpp:90-99`).
pub const FLAG: &[&str] = &[
    "ITEM_TUNABLE",
    "ITEM_SAVE",
    "ITEM_STACKABLE",
    "COUNT_PER_1GOLD",
    "ITEM_SLOW_QUERY",
    "ITEM_UNIQUE",
    "ITEM_MAKECOUNT",
    "ITEM_IRREMOVABLE",
    "CONFIRM_WHEN_USE",
    "QUEST_USE",
    "QUEST_USE_MULTIPLE",
    "QUEST_GIVE",
    "ITEM_QUEST",
    "LOG",
    "STACKABLE",
    "SLOW_QUERY",
    "REFINEABLE",
    "IRREMOVABLE",
    "ITEM_APPLICABLE",
];

/// The `ITEM_WEAR` names, whose bits become `dwWearFlags` (`ProtoReader.cpp:424-457`).
///
/// 21 names. Index 0 is `WEAR_BODY`, so the mask is not a `1 << slot` index at all and must be
/// looked up by name.
pub const WEAR_FLAG: &[&str] = &[
    "WEAR_BODY",
    "WEAR_HEAD",
    "WEAR_FOOTS",
    "WEAR_WRIST",
    "WEAR_WEAPON",
    "WEAR_NECK",
    "WEAR_EAR",
    "WEAR_SHIELD",
    "WEAR_UNIQUE",
    "WEAR_ARROW",
    "WEAR_HAIR",
    "WEAR_ABILITY",
    "WEAR_COSTUME_SASH",
    "WEAR_TALISMAN_FIRE",
    "WEAR_TALISMAN_ICE",
    "WEAR_TALISMAN_EARTH",
    "WEAR_TALISMAN_DARK",
    "WEAR_TALISMAN_WIND",
    "WEAR_TALISMAN_ELEC",
    "WEAR_GLOVE",
    "WEAR_COSTUME_SASH_SKIN",
];

/// The `IMMUNE` names, whose bits become `dwImmuneFlag` (`ProtoReader.cpp:459-483`).
///
/// 7 names, the whole status-immunity set.
pub const IMMUNE: &[&str] = &["PARA", "CURSE", "STUN", "SLEEP", "SLOW", "POISON", "TERROR"];

/// The `LIMIT_TYPE` names, indexed by `ELimitTypes` (`ProtoReader.cpp:488-507`).
///
/// 10 names. The **index** is the value, and the index order matches `ELimitTypes`
/// (`item_length.h:427-450`) exactly even though index 7 is spelled `REAL_TIME_FIRST_USE` here
/// and `LIMIT_REAL_TIME_START_FIRST_USE` in the enum. `Set_Proto_Item_Table:962-968` compares
/// against the enum member, so the numbering is what has to be right.
pub const LIMIT_TYPE: &[&str] = &[
    "LIMIT_NONE",
    "LEVEL",
    "STR",
    "DEX",
    "INT",
    "CON",
    "REAL_TIME",
    "REAL_TIME_FIRST_USE",
    "TIMER_BASED_ON_WEAR",
    "CHAMPION",
];

/// The `ADDON_TYPE` names, indexed by `EApplyTypes` (`ProtoReader.cpp:510-583`).
///
/// 130 names, of which four are behind `#if defined(__CONQUEROR_LEVEL__)` and so present. The
/// last one, index 129, is `APPLY_RESIST_COMBAT`.
pub const APPLY_TYPE: &[&str] = &[
    "APPLY_NONE",
    "APPLY_MAX_HP",
    "APPLY_MAX_SP",
    "APPLY_CON",
    "APPLY_INT",
    "APPLY_STR",
    "APPLY_DEX",
    "APPLY_ATT_SPEED",
    "APPLY_MOV_SPEED",
    "APPLY_CAST_SPEED",
    "APPLY_HP_REGEN",
    "APPLY_SP_REGEN",
    "APPLY_POISON_PCT",
    "APPLY_STUN_PCT",
    "APPLY_SLOW_PCT",
    "APPLY_CRITICAL_PCT",
    "APPLY_PENETRATE_PCT",
    "APPLY_ATTBONUS_HUMAN",
    "APPLY_ATTBONUS_ANIMAL",
    "APPLY_ATTBONUS_ORC",
    "APPLY_ATTBONUS_MILGYO",
    "APPLY_ATTBONUS_UNDEAD",
    "APPLY_ATTBONUS_DEVIL",
    "APPLY_STEAL_HP",
    "APPLY_STEAL_SP",
    "APPLY_MANA_BURN_PCT",
    "APPLY_DAMAGE_SP_RECOVER",
    "APPLY_BLOCK",
    "APPLY_DODGE",
    "APPLY_RESIST_SWORD",
    "APPLY_RESIST_TWOHAND",
    "APPLY_RESIST_DAGGER",
    "APPLY_RESIST_BELL",
    "APPLY_RESIST_FAN",
    "APPLY_RESIST_BOW",
    "APPLY_RESIST_FIRE",
    "APPLY_RESIST_ELEC",
    "APPLY_RESIST_MAGIC",
    "APPLY_RESIST_WIND",
    "APPLY_REFLECT_MELEE",
    "APPLY_REFLECT_CURSE",
    "APPLY_POISON_REDUCE",
    "APPLY_KILL_SP_RECOVER",
    "APPLY_EXP_DOUBLE_BONUS",
    "APPLY_GOLD_DOUBLE_BONUS",
    "APPLY_ITEM_DROP_BONUS",
    "APPLY_POTION_BONUS",
    "APPLY_KILL_HP_RECOVER",
    "APPLY_IMMUNE_STUN",
    "APPLY_IMMUNE_SLOW",
    "APPLY_IMMUNE_FALL",
    "APPLY_SKILL",
    "APPLY_BOW_DISTANCE",
    "APPLY_ATT_GRADE_BONUS",
    "APPLY_DEF_GRADE_BONUS",
    "APPLY_MAGIC_ATT_GRADE",
    "APPLY_MAGIC_DEF_GRADE",
    "APPLY_CURSE_PCT",
    "APPLY_MAX_STAMINA",
    "APPLY_ATTBONUS_WARRIOR",
    "APPLY_ATTBONUS_ASSASSIN",
    "APPLY_ATTBONUS_SURA",
    "APPLY_ATTBONUS_SHAMAN",
    "APPLY_ATTBONUS_MONSTER",
    "APPLY_MALL_ATTBONUS",
    "APPLY_MALL_DEFBONUS",
    "APPLY_MALL_EXPBONUS",
    "APPLY_MALL_ITEMBONUS",
    "APPLY_MALL_GOLDBONUS",
    "APPLY_MAX_HP_PCT",
    "APPLY_MAX_SP_PCT",
    "APPLY_SKILL_DAMAGE_BONUS",
    "APPLY_NORMAL_HIT_DAMAGE_BONUS",
    "APPLY_SKILL_DEFEND_BONUS",
    "APPLY_NORMAL_HIT_DEFEND_BONUS",
    "APPLY_EXTRACT_HP_PCT",
    "APPLY_RESIST_WARRIOR",
    "APPLY_RESIST_ASSASSIN",
    "APPLY_RESIST_SURA",
    "APPLY_RESIST_SHAMAN",
    "APPLY_ENERGY",
    "APPLY_DEF_GRADE",
    "APPLY_COSTUME_ATTR_BONUS",
    "APPLY_MAGIC_ATTBONUS_PER",
    "APPLY_MELEE_MAGIC_ATTBONUS_PER",
    "APPLY_RESIST_ICE",
    "APPLY_RESIST_EARTH",
    "APPLY_RESIST_DARK",
    "APPLY_ANTI_CRITICAL_PCT",
    "APPLY_ANTI_PENETRATE_PCT",
    "APPLY_ATTBONUS_BOSS",
    "APPLY_ATTBONUS_METIN",
    "APPLY_ENCHANT_ELECT",
    "APPLY_ENCHANT_FIRE",
    "APPLY_ENCHANT_ICE",
    "APPLY_ENCHANT_WIND",
    "APPLY_ENCHANT_EARTH",
    "APPLY_ENCHANT_DARK",
    "APPLY_SUNGMA_STR",
    "APPLY_SUNGMA_HP",
    "APPLY_SUNGMA_MOVE",
    "APPLY_SUNGMA_IMMUNE",
    "APPLY_ATTBONUS_ANIMAL_PCT",
    "APPLY_ATTBONUS_UNDEAD_PCT",
    "APPLY_ATTBONUS_DEVIL_PCT",
    "APPLY_ATTBONUS_ORC_PCT",
    "APPLY_ATTBONUS_MILGYO_PCT",
    "APPLY_ATTBONUS_DESERT_PCT",
    "APPLY_ATTBONUS_INSECT_PCT",
    "APPLY_ATTBONUS_TREE_PCT",
    "APPLY_ATTBONUS_BOSS_PCT",
    "APPLY_ATTBONUS_METIN_PCT",
    "APPLY_ATTBONUS_CZ_PCT",
    "APPLY_ATTBONUS_HUMAN_PCT",
    "APPLY_ATTBONUS_MONSTER_PCT",
    "APPLY_ENCHANT_ELECT_PCT",
    "APPLY_ENCHANT_FIRE_PCT",
    "APPLY_ENCHANT_ICE_PCT",
    "APPLY_ENCHANT_WIND_PCT",
    "APPLY_ENCHANT_EARTH_PCT",
    "APPLY_ENCHANT_DARK_PCT",
    "APPLY_RESIST_ELECT_PCT",
    "APPLY_RESIST_FIRE_PCT",
    "APPLY_RESIST_ICE_PCT",
    "APPLY_RESIST_WIND_PCT",
    "APPLY_RESIST_EARTH_PCT",
    "APPLY_RESIST_DARK_PCT",
    "APPLY_RESIST_HUMAN_PCT",
    "APPLY_RESIST_FALL",
    "APPLY_RESIST_COMBAT",
];

/// `MAX_APPLY_NUM` (`server/server/common/length.h:639`): the `EApplyTypes` member after the
/// last real apply type, and the one the attribute-group range check compares against.
///
/// The value is 130, which is the number of members before it. The comments beside two of those
/// members are stale: `length.h:591-592` writes `//92` and `//93` for `APPLY_ATTBONUS_BOSS` and
/// `APPLY_ATTBONUS_METIN`, which are really 90 and 91. The value here was measured by compiling
/// the enum, not by counting or by reading the comments.
pub const MAX_APPLY_NUM: u32 = 130;

/// The sentinel `c_aApplyTypeNames` carries for a duration that never runs out
/// (`server/server/game/constants.cpp:1269`).
///
/// It is a literal in the table rather than a member of `EApplyTypes`, so it is not an apply type
/// and is not in [`APPLY_TYPE`].
pub const INFINITE_AFFECT_DURATION: u32 = 0x1FFF_FFFF;

/// `c_aApplyTypeNames` (`server/server/game/constants.cpp:1186-1321`), as `(name, value)`.
///
/// This is **not** [`APPLY_TYPE`], and the two must not be merged:
///
/// - The names here have the `APPLY_` prefix stripped, so `FN_get_apply_type` answers 0 for
///   `APPLY_MAX_HP` and 1 for `MAX_HP`.
/// - The order is the table's own, which is not the enum's, and the first match wins.
/// - Five values are named twice. Four are deliberate aliases, because the source's own value
///   column points the short name at the `_PCT` member: `POISON`, `STUN`, `CRITICAL`, and
///   `PENETRATE` each resolve to their `_PCT` value. The fifth is `ATT_BONUS_TO_MONSTER` and
///   `ATT_BONUS_TO_MOB`, which both name `APPLY_ATTBONUS_MONSTER` in the source.
/// - One row, [`INFINITE_AFFECT_DURATION`], is not an apply type at all.
pub const APPLY_TYPE_NAMES: &[(&str, u32)] = &[
    ("STR", 5),
    ("DEX", 6),
    ("CON", 3),
    ("INT", 4),
    ("MAX_HP", 1),
    ("MAX_SP", 2),
    ("MAX_STAMINA", 58),
    ("POISON_REDUCE", 41),
    ("EXP_DOUBLE_BONUS", 43),
    ("GOLD_DOUBLE_BONUS", 44),
    ("ITEM_DROP_BONUS", 45),
    ("HP_REGEN", 10),
    ("SP_REGEN", 11),
    ("ATTACK_SPEED", 7),
    ("MOVE_SPEED", 8),
    ("CAST_SPEED", 9),
    ("ATT_BONUS", 53),
    ("DEF_BONUS", 54),
    ("MAGIC_ATT_GRADE", 55),
    ("MAGIC_DEF_GRADE", 56),
    ("SKILL", 51),
    ("ATTBONUS_ANIMAL", 18),
    ("ATTBONUS_UNDEAD", 21),
    ("ATTBONUS_DEVIL", 22),
    ("ATTBONUS_HUMAN", 17),
    ("ADD_BOW_DISTANCE", 52),
    ("DODGE", 28),
    ("BLOCK", 27),
    ("RESIST_SWORD", 29),
    ("RESIST_TWOHAND", 30),
    ("RESIST_DAGGER", 31),
    ("RESIST_BELL", 32),
    ("RESIST_FAN", 33),
    ("RESIST_BOW", 34),
    ("RESIST_FIRE", 35),
    ("RESIST_ELEC", 36),
    ("RESIST_MAGIC", 37),
    ("RESIST_WIND", 38),
    ("REFLECT_MELEE", 39),
    ("REFLECT_CURSE", 40),
    ("RESIST_ICE", 85),
    ("RESIST_EARTH", 86),
    ("RESIST_DARK", 87),
    ("RESIST_CRITICAL", 88),
    ("RESIST_PENETRATE", 89),
    ("POISON", 12),
    ("SLOW", 14),
    ("STUN", 13),
    ("STEAL_HP", 23),
    ("STEAL_SP", 24),
    ("MANA_BURN_PCT", 25),
    ("CRITICAL", 15),
    ("PENETRATE", 16),
    ("KILL_SP_RECOVER", 42),
    ("KILL_HP_RECOVER", 47),
    ("PENETRATE_PCT", 16),
    ("CRITICAL_PCT", 15),
    ("POISON_PCT", 12),
    ("STUN_PCT", 13),
    ("ATT_BONUS_TO_WARRIOR", 59),
    ("ATT_BONUS_TO_ASSASSIN", 60),
    ("ATT_BONUS_TO_SURA", 61),
    ("ATT_BONUS_TO_SHAMAN", 62),
    ("ATT_BONUS_TO_MONSTER", 63),
    ("ATT_BONUS_TO_MOB", 63),
    ("MALL_ATTBONUS", 64),
    ("MALL_EXPBONUS", 66),
    ("MALL_DEFBONUS", 65),
    ("MALL_ITEMBONUS", 67),
    ("MALL_GOLDBONUS", 68),
    ("MAX_HP_PCT", 69),
    ("MAX_SP_PCT", 70),
    ("SKILL_DAMAGE_BONUS", 71),
    ("NORMAL_HIT_DAMAGE_BONUS", 72),
    ("SKILL_DEFEND_BONUS", 73),
    ("NORMAL_HIT_DEFEND_BONUS", 74),
    ("RESIST_WARRIOR", 76),
    ("RESIST_ASSASSIN", 77),
    ("RESIST_SURA", 78),
    ("RESIST_SHAMAN", 79),
    ("INFINITE_AFFECT_DURATION", 0x1FFF_FFFF),
    ("ENERGY", 80),
    ("COSTUME_ATTR_BONUS", 82),
    ("MAGIC_ATTBONUS_PER", 83),
    ("MELEE_MAGIC_ATTBONUS_PER", 84),
    ("ATTBONUS_METIN", 91),
    ("ATTBONUS_BOSS", 90),
    ("ENCHANT_ELECT", 92),
    ("ENCHANT_FIRE", 93),
    ("ENCHANT_ICE", 94),
    ("ENCHANT_WIND", 95),
    ("ENCHANT_EARTH", 96),
    ("ENCHANT_DARK", 97),
    ("SUNGMA_STR", 98),
    ("SUNGMA_HP", 99),
    ("SUNGMA_MOVE", 100),
    ("SUNGMA_IMMUNE", 101),
    ("ATTBONUS_ANIMAL_PCT", 102),
    ("ATTBONUS_UNDEAD_PCT", 103),
    ("ATTBONUS_DEVIL_PCT", 104),
    ("ATTBONUS_ORC_PCT", 105),
    ("ATTBONUS_MILGYO_PCT", 106),
    ("ATTBONUS_DESERT_PCT", 107),
    ("ATTBONUS_INSECT_PCT", 108),
    ("ATTBONUS_TREE_PCT", 109),
    ("ATTBONUS_BOSS_PCT", 110),
    ("ATTBONUS_METIN_PCT", 111),
    ("ATTBONUS_CZ_PCT", 112),
    ("ATTBONUS_HUMAN_PCT", 113),
    ("ATTBONUS_MONSTER_PCT", 114),
    ("ENCHANT_ELECT_PCT", 115),
    ("ENCHANT_FIRE_PCT", 116),
    ("ENCHANT_ICE_PCT", 117),
    ("ENCHANT_WIND_PCT", 118),
    ("ENCHANT_EARTH_PCT", 119),
    ("ENCHANT_DARK_PCT", 120),
    ("RESIST_ELECT_PCT", 121),
    ("RESIST_FIRE_PCT", 122),
    ("RESIST_ICE_PCT", 123),
    ("RESIST_WIND_PCT", 124),
    ("RESIST_EARTH_PCT", 125),
    ("RESIST_DARK_PCT", 126),
    ("RESIST_HUMAN_PCT", 127),
    ("RESIST_FALL", 128),
    ("RESIST_COMBAT", 129),
];

/// `FN_get_apply_type` (`server/server/game/constants.cpp:1324-1333`): an apply type by name.
///
/// The comparison is `strcasecmp`, so it is ASCII case-insensitive, and the walk returns on the
/// first match, so [`APPLY_TYPE_NAMES`]'s order decides a name that appears twice. A name that is
/// in neither the table nor is a number answers 0, which the callers read as invalid.
pub fn fn_get_apply_type(name: &[u8]) -> u32 {
    APPLY_TYPE_NAMES
        .iter()
        .find(|(candidate, _)| candidate.as_bytes().eq_ignore_ascii_case(name))
        .map_or(0, |(_, value)| *value)
}

const TYPE_1: &[&str] = &[
    "WEAPON_SWORD",
    "WEAPON_DAGGER",
    "WEAPON_BOW",
    "WEAPON_TWO_HANDED",
    "WEAPON_BELL",
    "WEAPON_FAN",
    "WEAPON_ARROW",
    "WEAPON_MOUNT_SPEAR",
    "WEAPON_CLAW",
    "WEAPON_QUIVER",
    "WEAPON_BOUQUET",
];

const TYPE_2: &[&str] = &[
    "ARMOR_BODY",
    "ARMOR_HEAD",
    "ARMOR_SHIELD",
    "ARMOR_WRIST",
    "ARMOR_FOOTS",
    "ARMOR_NECK",
    "ARMOR_EAR",
    "ARMOR_GLOVE",
    "ARMOR_NUM_TYPES",
];

const TYPE_3: &[&str] = &[
    "USE_POTION",
    "USE_TALISMAN",
    "USE_TUNING",
    "USE_MOVE",
    "USE_TREASURE_BOX",
    "USE_MONEYBAG",
    "USE_BAIT",
    "USE_ABILITY_UP",
    "USE_AFFECT",
    "USE_CREATE_STONE",
    "USE_SPECIAL",
    "USE_POTION_NODELAY",
    "USE_CLEAR",
    "USE_INVISIBILITY",
    "USE_DETACHMENT",
    "USE_BUCKET",
    "USE_POTION_CONTINUE",
    "USE_CLEAN_SOCKET",
    "USE_CHANGE_ATTRIBUTE",
    "USE_ADD_ATTRIBUTE",
    "USE_ADD_ACCESSORY_SOCKET",
    "USE_PUT_INTO_ACCESSORY_SOCKET",
    "USE_ADD_ATTRIBUTE2",
    "USE_RECIPE",
    "USE_CHANGE_ATTRIBUTE2",
    "USE_BIND",
    "USE_UNBIND",
    "USE_TIME_CHARGE_PER",
    "USE_TIME_CHARGE_FIX",
    "USE_PUT_INTO_BELT_SOCKET",
    "USE_PUT_INTO_RING_SOCKET",
    "USE_CHANGE_COSTUME_ATTR",
    "USE_RESET_COSTUME_ATTR",
    "USE_UNK33",
    "USE_CHANGE_ATTRIBUTE_PLUS",
    "USE_PUT_INTO_AURA_SOCKET",
    "USE_ELEMENT_UPGRADE",
    "USE_ELEMENT_DOWNGRADE",
    "USE_ELEMENT_CHANGE",
    "USE_SET_ATT_COSTUME",
    "USE_SET_ATT_PET",
    "USE_SET_ATT_MOUNT",
    "USE_SET_ATT_COSTUME_WEAPON",
    "USE_ADD_ATTRIBUTE_TALISMAN",
    "USE_CHANGE_ATTRIBUTE_TALISMAN",
    "USE_ADD_ATTRIBUTE_GLOVE",
    "USE_CHANGE_ATTRIBUTE_GLOVE",
    "USE_UNLOCK_SHAMAN",
];

const TYPE_4: &[&str] = &[
    "AUTOUSE_POTION",
    "AUTOUSE_ABILITY_UP",
    "AUTOUSE_BOMB",
    "AUTOUSE_GOLD",
    "AUTOUSE_MONEYBAG",
    "AUTOUSE_TREASURE_BOX",
];

const TYPE_5: &[&str] = &[
    "MATERIAL_LEATHER",
    "MATERIAL_BLOOD",
    "MATERIAL_ROOT",
    "MATERIAL_NEEDLE",
    "MATERIAL_JEWEL",
    "MATERIAL_DS_REFINE_NORMAL",
    "MATERIAL_DS_REFINE_BLESSED",
    "MATERIAL_DS_REFINE_HOLLY",
];

const TYPE_6: &[&str] = &[
    "SPECIAL_MAP",
    "SPECIAL_KEY",
    "SPECIAL_DOC",
    "SPECIAL_SPIRIT",
];

const TYPE_7: &[&str] = &["TOOL_FISHING_ROD"];

const TYPE_8: &[&str] = &["LOTTERY_TICKET", "LOTTERY_INSTANT"];

const TYPE_10: &[&str] = &["METIN_NORMAL", "METIN_GOLD"];

const TYPE_12: &[&str] = &["FISH_ALIVE", "FISH_DEAD"];

const TYPE_14: &[&str] = &[
    "RESOURCE_FISHBONE",
    "RESOURCE_WATERSTONEPIECE",
    "RESOURCE_WATERSTONE",
    "RESOURCE_BLOOD_PEARL",
    "RESOURCE_BLUE_PEARL",
    "RESOURCE_WHITE_PEARL",
    "RESOURCE_BUCKET",
    "RESOURCE_CRYSTAL",
    "RESOURCE_GEM",
    "RESOURCE_STONE",
    "RESOURCE_METIN",
    "RESOURCE_ORE",
    "RESOURCE_AURA",
];

const TYPE_16: &[&str] = &[
    "UNIQUE_NONE",
    "UNIQUE_BOOK",
    "UNIQUE_SPECIAL_RIDE",
    "UNIQUE_3",
    "UNIQUE_4",
    "UNIQUE_5",
    "UNIQUE_6",
    "UNIQUE_7",
    "UNIQUE_8",
    "UNIQUE_9",
    "USE_SPECIAL",
];

const TYPE_28: &[&str] = &[
    "COSTUME_BODY",
    "COSTUME_HAIR",
    "COSTUME_MOUNT",
    "COSTUME_SASH",
    "COSTUME_WEAPON",
    "COSTUME_AURA",
    "COSTUME_PET",
    "COSTUME_SASH_SKIN",
];

const TYPE_29: &[&str] = &[
    "DS_SLOT1", "DS_SLOT2", "DS_SLOT3", "DS_SLOT4", "DS_SLOT5", "DS_SLOT6",
];

const TYPE_31: &[&str] = &["EXTRACT_DRAGON_SOUL", "EXTRACT_DRAGON_HEART"];

const TYPE_36: &[&str] = &["TOGGLE_SHAMAN"];

/// The sub-type table of each `ITEM_TYPE`, by index (`arSubType`, `ProtoReader.cpp:242`).
///
/// An empty slice is a type with no registered sub-types, which resolves to `0`. Index 30
/// (`ITEM_SPECIAL_DS`) reuses index 29's table; that is what legacy does, and a sub-type name that
/// only fits one of the two would resolve differently than a reader of the C++ would expect.
pub const SUB_TYPE: &[&[&str]] = &[
    &[],     // 0 ITEM_NONE
    TYPE_1,  // 1 ITEM_WEAPON
    TYPE_2,  // 2 ITEM_ARMOR
    TYPE_3,  // 3 ITEM_USE
    TYPE_4,  // 4 ITEM_AUTOUSE
    TYPE_5,  // 5 ITEM_MATERIAL
    TYPE_6,  // 6 ITEM_SPECIAL
    TYPE_7,  // 7 ITEM_TOOL
    TYPE_8,  // 8 ITEM_LOTTERY
    &[],     // 9 ITEM_ELK
    TYPE_10, // 10 ITEM_METIN
    &[],     // 11 ITEM_CONTAINER
    TYPE_12, // 12 ITEM_FISH
    &[],     // 13 ITEM_ROD
    TYPE_14, // 14 ITEM_RESOURCE
    &[],     // 15 ITEM_CAMPFIRE
    TYPE_16, // 16 ITEM_UNIQUE
    &[],     // 17 ITEM_SKILLBOOK
    &[],     // 18 ITEM_QUEST
    &[],     // 19 ITEM_POLYMORPH
    &[],     // 20 ITEM_TREASURE_BOX
    &[],     // 21 ITEM_TREASURE_KEY
    &[],     // 22 ITEM_SKILLFORGET
    &[],     // 23 ITEM_GIFTBOX
    &[],     // 24 ITEM_PICK
    &[],     // 25 ITEM_HAIR
    &[],     // 26 ITEM_TOTEM
    &[],     // 27 ITEM_BLEND
    TYPE_28, // 28 ITEM_COSTUME
    TYPE_29, // 29 ITEM_DS
    TYPE_29, // 30 ITEM_SPECIAL_DS
    TYPE_31, // 31 ITEM_EXTRACT
    &[],     // 32 ITEM_SECONDARY_COIN
    &[],     // 33 ITEM_RING
    &[],     // 34 ITEM_BELT
    &[],     // 35 ITEM_TALISMAN
    TYPE_36, // 36 ITEM_TOGGLE
    &[],     // 37
    &[],     // 38
    &[],     // 39
    &[],     // 40
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The counts `g++ -std=c++17` reports for the real initialisers, compiled from
    /// `ProtoReader.cpp` lines 57-583. This is the control that keeps a transcription slip from
    /// becoming a silently wrong item bonus: every number here came out of the C++ compiler, not
    /// out of a hand count.
    #[test]
    fn table_lengths_match_the_compiled_cxx_initialisers() {
        assert_eq!(TYPE.len(), 37);
        assert_eq!(SUB_TYPE.len(), 41);
        assert_eq!(ANTI_FLAG.len(), 21);
        assert_eq!(FLAG.len(), 19);
        assert_eq!(WEAR_FLAG.len(), 21);
        assert_eq!(IMMUNE.len(), 7);
        assert_eq!(LIMIT_TYPE.len(), 10);
        assert_eq!(APPLY_TYPE.len(), 130);
    }

    /// The per-type sub-type counts `g++` reports for `arNumberOfSubtype`.
    #[test]
    fn subtype_counts_match_the_compiled_cxx_initialisers() {
        let counts: Vec<usize> = SUB_TYPE.iter().map(|s| s.len()).collect();
        assert_eq!(
            counts,
            vec![
                0, 11, 9, 48, 6, 8, 4, 1, 2, 0, 2, 0, 2, 0, 13, 0, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 8, 6, 6, 2, 0, 0, 0, 0, 1, 0, 0, 0, 0,
            ]
        );
    }

    /// `__CONQUEROR_LEVEL__` is defined (`prodomodefines.h:53`), so the four SUNGMA names are in the
    /// table and sit four higher than the stock Metin2 table would put them.
    #[test]
    fn the_conqueror_level_gate_shifts_the_apply_table() {
        assert_eq!(APPLY_TYPE[96], "APPLY_ENCHANT_EARTH");
        assert_eq!(APPLY_TYPE[97], "APPLY_ENCHANT_DARK");
        assert_eq!(APPLY_TYPE[98], "APPLY_SUNGMA_STR");
        assert_eq!(APPLY_TYPE[99], "APPLY_SUNGMA_HP");
        assert_eq!(APPLY_TYPE[100], "APPLY_SUNGMA_MOVE");
        assert_eq!(APPLY_TYPE[101], "APPLY_SUNGMA_IMMUNE");
        assert_eq!(APPLY_TYPE[102], "APPLY_ATTBONUS_ANIMAL_PCT");
        assert_eq!(APPLY_TYPE[129], "APPLY_RESIST_COMBAT");
        assert_eq!(APPLY_TYPE[127], "APPLY_RESIST_HUMAN_PCT");
    }

    /// The first and last name of each table, so a shift that kept the length would still fail.
    #[test]
    fn table_ends_are_pinned() {
        assert_eq!(TYPE[0], "ITEM_NONE");
        assert_eq!(TYPE[36], "ITEM_TOGGLE");
        assert_eq!(ANTI_FLAG[0], "ANTI_FEMALE");
        assert_eq!(ANTI_FLAG[20], "ANTI_PET21");
        assert_eq!(FLAG[0], "ITEM_TUNABLE");
        assert_eq!(FLAG[18], "ITEM_APPLICABLE");
        assert_eq!(WEAR_FLAG[0], "WEAR_BODY");
        assert_eq!(WEAR_FLAG[18], "WEAR_TALISMAN_ELEC");
        assert_eq!(WEAR_FLAG[20], "WEAR_COSTUME_SASH_SKIN");
        assert_eq!(IMMUNE[0], "PARA");
        assert_eq!(IMMUNE[6], "TERROR");
        assert_eq!(LIMIT_TYPE[0], "LIMIT_NONE");
        assert_eq!(LIMIT_TYPE[9], "CHAMPION");
        assert_eq!(APPLY_TYPE[0], "APPLY_NONE");
    }

    /// The type match is substring containment in both directions, lowest index first.
    #[test]
    fn type_value_matches_in_both_directions_and_takes_the_lowest_index() {
        assert_eq!(type_value(b"ITEM_ELK"), Some(9));
        assert_eq!(type_value(b"ITEM_WEAPON"), Some(1));
        assert_eq!(type_value(b"ITEM_TOGGLE"), Some(36));
        // Legacy's two `find(..) != npos` tests are mutual containment, which for two non-empty
        // strings is equality. So a prefix, a suffix, and a substring all fail.
        assert_eq!(type_value(b"ITEM"), None);
        assert_eq!(type_value(b"ITEM_ELK_TAIL"), None);
        assert_eq!(type_value(b"ELK"), None);
        assert_eq!(type_value(b"NOT_AN_ITEM_TYPE"), None);
        // This is the one resolver that does not trim, so a padded field does not match.
        assert_eq!(type_value(b" ITEM_ELK "), None);
        assert_eq!(type_value(b""), None);
    }

    /// An unparseable type is `-1` in legacy and must not be accepted here.
    #[test]
    fn an_unknown_type_is_refused_rather_than_defaulted() {
        assert_eq!(type_value(b"\"ITEM_UNIQUE\""), None);
    }

    /// A type with no sub-type table is `0`, and the file spells that as `0` in the column.
    #[test]
    fn a_type_without_subtypes_resolves_to_zero() {
        assert_eq!(sub_type_value(9, b"0"), SubType::Unregistered);
        assert_eq!(sub_type_value(0, b"0"), SubType::Unregistered);
        assert_eq!(sub_type_value(24, b"0"), SubType::Unregistered);
    }

    /// A registered type compares for equality against the trimmed field.
    #[test]
    fn a_subtype_is_an_exact_match_in_its_own_table() {
        assert_eq!(sub_type_value(1, b"WEAPON_SWORD"), SubType::Value(0));
        assert_eq!(sub_type_value(1, b" WEAPON_BOW \r"), SubType::Value(2));
        assert_eq!(sub_type_value(1, b"WEAPON"), SubType::Unknown);
        assert_eq!(sub_type_value(3, b"USE_POTION"), SubType::Value(0));
        assert_eq!(sub_type_value(99, b"WEAPON_SWORD"), SubType::TypeOutOfRange);
        assert_eq!(sub_type_value(-1, b"WEAPON_SWORD"), SubType::TypeOutOfRange);
    }

    /// `ITEM_SPECIAL_DS` shares `ITEM_DS`'s table in legacy, so the same sub-type resolves at both
    /// indices. A reader of the enum alone would give the two types separate tables and every DS
    /// sub-type would come out as `-1`, which is a boot-killing value in legacy.
    #[test]
    fn item_special_ds_reuses_the_item_ds_table() {
        assert_eq!(sub_type_value(29, b"DS_SLOT1"), SubType::Value(0));
        assert_eq!(sub_type_value(29, b"DS_SLOT6"), SubType::Value(5));
        assert_eq!(sub_type_value(30, b"DS_SLOT1"), SubType::Value(0));
        assert_eq!(sub_type_value(30, b"DS_SLOT6"), SubType::Value(5));
        assert_eq!(sub_type_value(29, b"COSTUME_MOUNT"), SubType::Unknown);
    }

    /// The limit and apply columns compare for equality, not by substring.
    #[test]
    fn limit_and_apply_are_exact_matches() {
        assert_eq!(limit_type_value(b"LIMIT_NONE"), Some(0));
        assert_eq!(limit_type_value(b"REAL_TIME_FIRST_USE"), Some(7));
        assert_eq!(limit_type_value(b"REAL_TIME"), Some(6));
        assert_eq!(limit_type_value(b"REAL"), None);
        assert_eq!(limit_type_value(b" CHAMPION "), Some(9));
        assert_eq!(apply_type_value(b"APPLY_NONE"), Some(0));
        assert_eq!(apply_type_value(b"APPLY_MOV_SPEED"), Some(8));
        assert_eq!(apply_type_value(b"APPLY"), None);
        assert_eq!(apply_type_value(b"APPLY_SUNGMA_STR"), Some(98));
        assert_eq!(apply_type_value(b""), None);
    }

    /// A flag column is a bitmask, so order and repetition do not matter.
    #[test]
    fn a_flag_column_is_a_bitmask() {
        assert_eq!(flag_mask(b"NONE", ANTI_FLAG), Ok(0));
        assert_eq!(flag_mask(b"ANTI_STACK", ANTI_FLAG), Ok(1 << 15));
        assert_eq!(
            flag_mask(b"ANTI_MUDANG|ANTI_WOLFMAN", ANTI_FLAG),
            Ok((1 << 5) | (1 << 18))
        );
        assert_eq!(
            flag_mask(b"ANTI_DROP | ANTI_SELL", ANTI_FLAG),
            Ok((1 << 7) | (1 << 8))
        );
        // Repeats and order are absorbed.
        assert_eq!(
            flag_mask(b"ANTI_SELL|ANTI_DROP|ANTI_SELL", ANTI_FLAG),
            Ok((1 << 7) | (1 << 8))
        );
    }

    /// Legacy's `StringSplit` drops empty tokens, so a doubled or trailing separator is harmless.
    #[test]
    fn split_flags_drops_empty_tokens_and_trims_each() {
        assert_eq!(split_flags(b"A||B|"), vec![&b"A"[..], &b"B"[..]]);
        assert_eq!(split_flags(b"|A"), vec![&b"A"[..]]);
        assert_eq!(split_flags(b"|||"), Vec::<&[u8]>::new());
        assert_eq!(split_flags(b""), Vec::<&[u8]>::new());
        assert_eq!(split_flags(b" A | B "), vec![&b"A"[..], &b"B"[..]]);
    }

    /// A name that is not in the table is dropped, which is how a typo in the data loses a flag.
    #[test]
    fn an_unknown_flag_name_contributes_nothing() {
        assert_eq!(flag_mask(b"ANTI_STACK|NOT_A_FLAG", ANTI_FLAG), Ok(1 << 15));
        assert_eq!(flag_mask(b"NOT_A_FLAG", ANTI_FLAG), Ok(0));
    }

    /// More tokens than legacy's `new string[30]` holds is a buffer overrun, refused here.
    #[test]
    fn a_flag_column_wider_than_the_legacy_buffer_is_refused() {
        let wide = vec![&b"ANTI_DROP"[..]; MAX_FLAG_TOKENS + 1].join(&b'|');
        assert_eq!(
            flag_mask(&wide, ANTI_FLAG),
            Err(TooManyFlagTokens {
                found: MAX_FLAG_TOKENS + 1,
                limit: MAX_FLAG_TOKENS,
            })
        );
        let exact = vec![&b"ANTI_DROP"[..]; MAX_FLAG_TOKENS].join(&b'|');
        assert_eq!(flag_mask(&exact, ANTI_FLAG), Ok(1 << 7));
    }

    /// `trim` removes exactly the set `ProtoReader::trim` does, `\v` included.
    #[test]
    fn trim_removes_the_legacy_set() {
        assert_eq!(trim(b" \t\x0b\r\nA \t\x0b\r\n"), &b"A"[..]);
        assert_eq!(trim(b"   "), &b""[..]);
        assert_eq!(trim(b""), &b""[..]);
        assert_eq!(trim(b"A"), &b"A"[..]);
        // A byte that is not in the set is kept, so a legacy code page is never touched.
        assert_eq!(trim(b"\x00A\x00"), &b"\x00A\x00"[..]);
    }

    /// A `DWORD` has 32 bits, and `pow((float)2, (float)i)` assigned to an `int` cannot produce a
    /// bit past 31, so a table with more than 32 names has names legacy never gives a bit. The four
    /// real tables are all under 22, so this uses a synthetic one: without it the bound is untested
    /// and a table that grew past 32 would silently lose its last names.
    #[test]
    fn a_flag_name_past_the_thirty_second_bit_is_dropped() {
        let table: Vec<String> = (0..40).map(|i| format!("FLAG_{i}")).collect();
        let refs: Vec<&str> = table.iter().map(String::as_str).collect();
        let mask = flag_mask(b"FLAG_31", &refs).unwrap();
        assert_eq!(mask, 1 << 31);
        // Name 32 would be bit 32, which is not a `u32` at all, and legacy's `double` result
        // assigned to an `int` is not a bit there either.
        assert_eq!(flag_mask(b"FLAG_32", &refs).unwrap(), 0);
        assert_eq!(flag_mask(b"FLAG_39", &refs).unwrap(), 0);
        // The four real tables are all inside the bound, so nothing is lost on the owner's data.
        for table in [ANTI_FLAG, FLAG, WEAR_FLAG, IMMUNE] {
            assert!(table.len() <= 32, "a flag table of {} names", table.len());
        }
    }

    /// The two attack-bonus entries, which are the pair most likely to be read wrong.
    ///
    /// `length.h:591-592` writes them in the order `APPLY_ATTBONUS_BOSS` then
    /// `APPLY_ATTBONUS_METIN` and comments them `//92` and `//93`. Both comments are **stale**: the
    /// enum auto-increments, and compiling it puts `APPLY_ATTBONUS_BOSS` at 90 and
    /// `APPLY_ATTBONUS_METIN` at 91. The shipped data is right and the comments are wrong, so this
    /// pins the compiled values and the enum order together; reading the comments gives 90 as 92
    /// and shifts every apply type above them by two.
    #[test]
    fn the_attack_bonus_entries_are_boss_then_metin() {
        assert_eq!(APPLY_TYPE[90], "APPLY_ATTBONUS_BOSS");
        assert_eq!(APPLY_TYPE[91], "APPLY_ATTBONUS_METIN");
        assert_eq!(apply_type_value(b"APPLY_ATTBONUS_BOSS"), Some(90));
        assert_eq!(apply_type_value(b"APPLY_ATTBONUS_METIN"), Some(91));
    }

    /// The last few entries, read from the compiled initialiser, so a table that lost its tail
    /// cannot pass on length alone.
    #[test]
    fn the_tail_of_the_apply_table_is_in_compiled_order() {
        assert_eq!(APPLY_TYPE[124], "APPLY_RESIST_WIND_PCT");
        assert_eq!(APPLY_TYPE[125], "APPLY_RESIST_EARTH_PCT");
        assert_eq!(APPLY_TYPE[126], "APPLY_RESIST_DARK_PCT");
        assert_eq!(APPLY_TYPE[127], "APPLY_RESIST_HUMAN_PCT");
        assert_eq!(APPLY_TYPE[128], "APPLY_RESIST_FALL");
        assert_eq!(APPLY_TYPE[129], "APPLY_RESIST_COMBAT");
    }

    /// `c_aApplyTypeNames` has one name per apply value, and the loader's `FN_get_apply_type`
    /// looks names up in it. The two are different tables: this one holds stripped names like
    /// `MAX_HP`, not the `APPLY_`-prefixed enum names.
    #[test]
    fn the_apply_name_table_is_125_rows_in_the_sources_own_order() {
        // 124 rows name an apply value and one row is the duration sentinel, so the table is
        // neither the enum nor the indexes. Its first row is `STR`, not `NONE`.
        assert_eq!(APPLY_TYPE_NAMES.len(), 125);
        assert_eq!(APPLY_TYPE_NAMES[0], ("STR", 5));
        assert_eq!(APPLY_TYPE_NAMES[4], ("MAX_HP", 1));
        assert_eq!(APPLY_TYPE_NAMES[124], ("RESIST_COMBAT", 129));
        // Every name is distinct, so the answer for a name never depends on which row comes first.
        let mut names: Vec<&str> = APPLY_TYPE_NAMES.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "no name is in the table twice");
    }

    /// The table does not cover every apply value, and the gaps are how a caller reads those
    /// names. A port needs to know this, because a value with no name answers 0.
    #[test]
    fn the_apply_name_table_leaves_eleven_values_nameless() {
        let named: std::collections::BTreeSet<u32> =
            APPLY_TYPE_NAMES.iter().map(|(_, v)| *v).collect();
        assert_eq!(
            named.iter().filter(|v| **v < MAX_APPLY_NUM).count(),
            119,
            "119 of the 130 values are named at least once"
        );
        let unnamed: Vec<u32> = (0..MAX_APPLY_NUM).filter(|v| !named.contains(v)).collect();
        assert_eq!(unnamed, [0, 19, 20, 26, 46, 48, 49, 50, 57, 75, 81]);
        // The one row outside the range is the duration sentinel.
        assert_eq!(
            named
                .iter()
                .filter(|v| **v >= MAX_APPLY_NUM)
                .copied()
                .collect::<Vec<_>>(),
            [INFINITE_AFFECT_DURATION]
        );
    }

    #[test]
    fn fn_get_apply_type_matches_names_case_insensitively() {
        assert_eq!(fn_get_apply_type(b"MAX_HP"), 1);
        assert_eq!(
            fn_get_apply_type(b"max_hp"),
            1,
            "strcasecmp, so the case does not matter"
        );
        assert_eq!(
            fn_get_apply_type(b" Max_Hp "),
            0,
            "and it does not trim either"
        );
        assert_eq!(
            fn_get_apply_type(b"nope"),
            0,
            "0 is how a caller reads invalid"
        );
        assert_eq!(fn_get_apply_type(b""), 0);
        assert_eq!(
            fn_get_apply_type(b"APPLY_MAX_HP"),
            0,
            "the enum name is not the name here"
        );
    }

    /// The four aliases the legacy resolver holds that no enum member is named after, plus the
    /// duration sentinel. Each was read out of the compiled `FN_get_apply_type` body.
    #[test]
    fn the_apply_name_aliases_answer_their_own_values() {
        assert_eq!(
            fn_get_apply_type(b"POISON"),
            fn_get_apply_type(b"POISON_PCT")
        );
        assert_eq!(fn_get_apply_type(b"STUN"), fn_get_apply_type(b"STUN_PCT"));
        assert_eq!(
            fn_get_apply_type(b"CRITICAL"),
            fn_get_apply_type(b"CRITICAL_PCT")
        );
        assert_eq!(
            fn_get_apply_type(b"PENETRATE"),
            fn_get_apply_type(b"PENETRATE_PCT")
        );
        assert_eq!(
            fn_get_apply_type(b"ATT_BONUS_TO_MONSTER"),
            fn_get_apply_type(b"ATT_BONUS_TO_MOB")
        );
        assert_eq!(
            fn_get_apply_type(b"INFINITE_AFFECT_DURATION"),
            INFINITE_AFFECT_DURATION
        );
        assert_eq!(INFINITE_AFFECT_DURATION, 0x1FFF_FFFF);
    }

    /// The two numbers the legacy attribute loader range-checks against. Both came out of a
    /// compiled probe, not out of a header comment.
    #[test]
    fn the_apply_range_numbers_come_from_the_compiler() {
        assert_eq!(MAX_APPLY_NUM, 130);
        assert_eq!(APPLY_TYPE.len(), MAX_APPLY_NUM as usize);
    }
}
