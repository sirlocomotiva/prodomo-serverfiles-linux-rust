//! The item proto and the item locale names, read as the legacy DB server reads them.
//!
//! `item_proto.txt` is the largest text proto in the game: 35 header columns, of which legacy reads
//! 33, over 7,305 data rows. `item_names.txt` supplies the locale name for a vnum. Both are read
//! with the tab-separated CSV reader in [`crate::csv_table`], which is the port of
//! `cCsvFile::Load` (`server/server/db/CsvReader.cpp:105-209`).
//!
//! # The load path
//!
//! `CClientManager::InitializeItemTable` (`server/server/db/ClientManagerBoot.cpp:548-645`) reads
//! the two files, fills a `TItemTable` per row through `Set_Proto_Item_Table`
//! (`server/server/db/ProtoReader.cpp:861-1010`), and finally sorts the vector by `dwVnum` with
//! `std::sort`. The sorted order is what reaches the game process and what every lookup sees.
//!
//! Three things about that path are easy to get wrong, so they are called out here and pinned by
//! the tests at the end of this file.
//!
//! **A vnum column is a vnum or a range.** `110000~110099` is one row covering a hundred vnums, and
//! `Set_Proto_Item_Table:931-952` stores it as `dwVnum = 110000`, `dwVnumRange = end - start`. A
//! range therefore excludes **both** of its endpoints in the lookup at `item_manager.cpp:674-690`,
//! which tests `dwVnum < vnum && vnum < dwVnum + dwVnumRange`. `110000` and `110099` are both
//! outside it. Legacy also never complains that the start is inside the range, so the endpoints
//! are only reachable through a separate exact row, if one exists.
//!
//! **The sorted order is not the file order.** Two rows share a vnum, `std::sort` is not stable,
//! and the file gives the two copies of `71224` and `71225` different statistics. Legacy's answer
//! for those four vnums therefore depends on its standard library's introsort. This module sorts
//! by `(vnum, line)`, where `line` is the file line, so the **first** row in file order wins, which
//! is also what the two name lookups in `item_manager.cpp:721-756` do when they take the first
//! match while scanning the sorted vector. That is the smallest change that makes all three lookups
//! agree; see [`ItemProtos::duplicates`] for the data defect itself. The line is in the key rather
//! than the sort being stable on purpose: no two rows share `(vnum, line)`, so the key is a total
//! order and the answer does not depend on which algorithm sorts it.
//!
//! **`bSpecular`, `alSockets` and `bWeight` are never read.** The column loop fills
//! `dataArray[0..33]`, and column 30, the `Specular` column, is then dropped: the comment
//! `//column for 'Specular'` at `:1005` sits above the `bGainSocketPct` assignment, so
//! `bSpecular` keeps the value `memset` put there at boot (`:589`), and `alSockets` is never
//! mentioned at all. `bWeight` is forced with `str_to_number(itemTable->bWeight, "0")` (`:1008`).
//! All three are modelled here as always zero rather than left to a default, so that a later
//! reader of the file cannot quietly start populating them.
//!
//! # What this module refuses
//!
//! Legacy reacts to a name it cannot resolve by calling `exit(0)` (`ProtoReader.cpp:917-923`),
//! which takes the server down with a success status and no message on stdout. This module returns
//! [`ItemProtoError`] naming the **file line**, the vnum, and the column, and the caller exits
//! non-zero. Legacy names the column and the index and not the row, so an operator had to count
//! rows by hand; the file line is reported here because the reader skips blank and `#` lines and a
//! row ordinal would point at the wrong line. The owner's `item_proto.txt` resolves every row
//! through [`parse`], which is the test `the_owners_file_loads` asserts.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::csv_table::{self, CsvError};
use crate::item_kind::{LIMIT_REAL_TIME_START_FIRST_USE, LIMIT_TIMER_BASED_ON_WEAR};
use crate::item_proto_value::{
    self, apply_type_value, flag_mask, limit_type_value, sub_type_value, type_value, SubType,
    ANTI_FLAG, FLAG, IMMUNE, WEAR_FLAG,
};

/// `ITEM_NAME_MAX_LEN` (`server/server/common/item_length.h:8`).
///
/// `szName` and `szLocaleName` are `char[ITEM_NAME_MAX_LEN + 1]`, and `strlcpy` truncates to 36
/// bytes, so a longer name in the file reaches the game cut. The owner's file has no name that
/// long, so this is pinned with a synthetic row instead.
pub const ITEM_NAME_MAX_LEN: usize = 36;

/// The number of columns legacy reads: `int dataArray[33]` (`ProtoReader.cpp:866`).
///
/// The owner's file has 35 header columns, so a row with 33 to 35 fields is accepted and the
/// extras are dropped, exactly as the fixed-length loop drops them.
pub const COLUMNS: usize = 33;

/// `ITEM_LIMIT_MAX_NUM` (`item_length.h:11`), the `aLimits` pair count.
pub const LIMITS: usize = 2;

/// `ITEM_APPLY_MAX_NUM` (`item_length.h:12`), the `aApplies` pair count.
pub const APPLIES: usize = 3;

/// `ITEM_VALUES_MAX_NUM` (`item_length.h:9`), the `alValues` count.
pub const VALUES: usize = 6;

/// `ITEM_SOCKET_MAX_NUM` under `ENABLE_EXTENDED_SOCKETS` (`item_length.h:13-18`).
///
/// `prodomodefines.h:76` defines it. Nothing ever writes these six entries, so the count is kept
/// only so the constant has a meaning; [`ItemProto::sockets`] is always all zero.
const SOCKETS: usize = 6;

/// One `TItemLimit` / `TItemApply` pair: an index into a value table and its amount.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ItemValue {
    /// The index into [`LIMIT_TYPE`](item_proto_value::LIMIT_TYPE) or
    /// [`APPLY_TYPE`](item_proto_value::APPLY_TYPE).
    pub kind: i32,
    /// The amount, from the column after the name.
    pub value: i32,
}

/// One row of `item_proto.txt`, as `Set_Proto_Item_Table` fills `TItemTable`.
///
/// Field names are the legacy ones, because this type is the compatibility representation of
/// `TItemTable` and the two differ: `flags` is `dwFlags`, `sockets` is `alSockets`, and `weight`
/// is the forced-zero `bWeight`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemProto {
    /// `dwVnum`. For a range row this is the **start** of the range.
    pub vnum: u32,
    /// The 1-based line of `item_proto.txt` this row was read from, header included.
    ///
    /// This is the tiebreaker that makes a repeated vnum deterministic, and it is a field rather
    /// than a sort detail so that nothing downstream has to trust the sort to be stable.
    pub line: usize,
    /// `dwVnumRange`, which `Set_Proto_Item_Table:951` sets to `end - start`.
    ///
    /// Zero for an ordinary row. A range's two endpoints are both outside it; see the module
    /// documentation.
    pub vnum_range: u32,
    /// `szName`, the original name, as the exact bytes the file holds.
    pub name: Vec<u8>,
    /// `szLocaleName`, from `item_names.txt`, falling back to [`ItemProto::name`].
    pub locale_name: Vec<u8>,
    /// `bType`, the index into [`TYPE`](item_proto_value::TYPE).
    pub item_type: i32,
    /// `bSubType`, the index into the sub-type table of [`ItemProto::item_type`].
    ///
    /// Zero for a type with no sub-type table, which is what the file's `0` column means too.
    pub sub_type: i32,
    /// `bWeight`. Legacy forces this to `0` (`ProtoReader.cpp:1008`) and reads no column for it.
    pub weight: i32,
    /// `bSize`, the `SIZE` column.
    pub size: i32,
    /// `dwAntiFlags`, the bitmask of the `ANTI_FLAG` column.
    pub anti_flags: u32,
    /// `dwFlags`, the bitmask of the `FLAG` column.
    pub flags: u32,
    /// `dwWearFlags`, the bitmask of the `ITEM_WEAR` column.
    pub wear_flags: u32,
    /// `dwImmuneFlag`, the bitmask of the `IMMUNE` column.
    pub immune_flags: u32,
    /// `dwGold`, the `GOLD` column.
    pub gold: u32,
    /// `dwShopBuyPrice`, the `SHOP_BUY_PRICE` column.
    pub shop_buy_price: u32,
    /// `aLimits`, the two `LIMIT_TYPE` / `LIMIT_VALUE` pairs.
    pub limits: [ItemValue; LIMITS],
    /// `aApplies`, the three `ADDON_TYPE` / `ADDON_VALUE` pairs.
    pub applies: [ItemValue; APPLIES],
    /// `alValues`, the six `VALUE0`..`VALUE5` columns.
    pub values: [i32; VALUES],
    /// `alSockets`. Never assigned by `Set_Proto_Item_Table`, so always zero.
    pub sockets: [i32; SOCKETS],
    /// `dwRefinedVnum`, the `REFINE` column.
    pub refined_vnum: u32,
    /// `wRefineSet`, the `REFINESET` column.
    pub refine_set: u16,
    /// `bAlterToMagicItemPct`, the `MAGIC_PCT` column.
    pub alter_to_magic_item_pct: i32,
    /// `bSpecular`. Column 30 is read into `dataArray[30]` and then dropped, so always zero.
    pub specular: i32,
    /// `bGainSocketPct`, the `SOCKET` column.
    pub gain_socket_pct: i32,
    /// `sAddonType`, the `ATTU_ADDON` column.
    pub addon_type: i32,
    /// `cLimitRealTimeFirstUseIndex`: which of [`ItemProto::limits`] is the real-time limit.
    pub real_time_first_use: Option<usize>,
    /// `cLimitTimerBasedOnWearIndex`: which of [`ItemProto::limits`] is the wear timer.
    pub timer_based_on_wear: Option<usize>,
}

impl ItemProto {
    /// Whether this row is a range, which legacy decides by `dwVnumRange != 0`
    /// (`item_manager.cpp:77-80`).
    pub fn is_range(&self) -> bool {
        self.vnum_range != 0
    }

    /// A proto carrying only a vnum, a type and a sub-type, and zero everywhere else.
    ///
    /// For the rules that read nothing but those three -- `item_custom_category` is the
    /// first -- so a test can vary one field and know the rest hold. It is public because
    /// those rules live in their own module and each needs its own handful of shapes, and a
    /// `pub(crate)` constructor under `#[cfg(test)]` would put a test-only type in the
    /// crate's public surface for every other reader to see in the docs.
    #[must_use]
    pub fn for_category_rule(vnum: u32, item_type: i32, sub_type: i32) -> Self {
        Self {
            vnum,
            line: 0,
            vnum_range: 0,
            name: Vec::new(),
            locale_name: Vec::new(),
            item_type,
            sub_type,
            weight: 0,
            size: 1,
            anti_flags: 0,
            flags: 0,
            wear_flags: 0,
            immune_flags: 0,
            gold: 0,
            shop_buy_price: 0,
            limits: Default::default(),
            applies: Default::default(),
            values: Default::default(),
            sockets: Default::default(),
            refined_vnum: 0,
            refine_set: 0,
            alter_to_magic_item_pct: 0,
            specular: 0,
            gain_socket_pct: 0,
            addon_type: 0,
            real_time_first_use: None,
            timer_based_on_wear: None,
        }
    }
}

/// The item protos of one file, in the order the game process sees them: sorted by vnum.
#[derive(Debug, Clone, Default)]
pub struct ItemProtos {
    /// Every row, sorted by vnum, with a duplicate vnum keeping its file order.
    by_vnum: Vec<ItemProto>,
    /// The indices of [`ItemProtos::by_vnum`] that are ranges, in the same order.
    ranges: Vec<usize>,
}

impl ItemProtos {
    /// Read `item_proto.txt` and `item_names.txt` from the proto folder.
    ///
    /// The two file names live here rather than in each caller, because a caller that
    /// spelled them itself would be free to spell one of them differently and get a
    /// table that parses. [`crate::mob_names::MobNames::load`] and the other proto
    /// readers each own their own pair the same way.
    ///
    /// # Errors
    ///
    /// Returns [`ItemProtoError::Io`] when a file is missing or unreadable, and any
    /// other [`ItemProtoError`] when a row is one legacy would have answered with
    /// `exit(0)`.
    pub fn load(proto_dir: &Path) -> Result<Self, ItemProtoError> {
        let read = |name: &str| {
            let path = proto_dir.join(name);
            std::fs::read(&path).map_err(|source| ItemProtoError::Io {
                path: path.clone(),
                message: source.to_string(),
            })
        };
        let proto = read("item_proto.txt")?;
        let locale_names = read("item_names.txt")?;
        parse(&proto, &locale_names)
    }

    /// A table of the given rows, sorted and indexed the way [`parse`] sorts and indexes the
    /// file's rows.
    ///
    /// The sort key is `(vnum, line)`, so two rows that share a vnum resolve to the one with
    /// the lower [`ItemProto::line`] exactly as they do in a parsed file. A caller that builds
    /// its own rows (a test that needs one proto shape, say) gets the same lookups a parsed
    /// file would give it.
    #[must_use]
    pub fn from_rows(mut by_vnum: Vec<ItemProto>) -> Self {
        by_vnum.sort_by_key(|p| (p.vnum, p.line));
        let ranges = (0..by_vnum.len())
            .filter(|i| by_vnum[*i].is_range())
            .collect();
        Self { by_vnum, ranges }
    }

    /// The number of rows, which is the file's data-row count and not the header.
    pub fn len(&self) -> usize {
        self.by_vnum.len()
    }

    /// Whether the file held no data rows.
    pub fn is_empty(&self) -> bool {
        self.by_vnum.is_empty()
    }

    /// Every row, sorted by vnum.
    pub fn rows(&self) -> &[ItemProto] {
        &self.by_vnum
    }

    /// `ITEM_MANAGER::GetTable` (`server/server/game/item_manager.cpp:674-690`).
    ///
    /// An exact vnum wins. Failing that, legacy scans the range rows in ascending vnum and takes
    /// the first whose bounds **strictly** contain the vnum, so neither endpoint of a range is
    /// found this way. `dwVnum + dwVnumRange` is a `DWORD` sum, so it wraps.
    ///
    /// Legacy finds the exact row with `RealNumber` (`item_manager.cpp:697-719`), a binary search
    /// that indexes the vector before it checks its bounds and so reads past the end for a vnum
    /// that is not there. That is a Defect and is not reproduced; this returns `None`.
    pub fn get(&self, vnum: u32) -> Option<&ItemProto> {
        // `binary_search_by_key` returns *an* equal element, not the first, so it would answer with
        // whichever half of a duplicated vnum the search happened to land on. Legacy's `RealNumber`
        // walks the vector from the front and returns the first equal row, so the lower bound is
        // what a caller sees.
        let index = self.by_vnum.partition_point(|p| p.vnum < vnum);
        if self.by_vnum.get(index).is_some_and(|p| p.vnum == vnum) {
            return self.by_vnum.get(index);
        }
        self.ranges
            .iter()
            .map(|i| &self.by_vnum[*i])
            .find(|p| p.vnum < vnum && vnum < p.vnum.wrapping_add(p.vnum_range))
    }

    /// `ITEM_MANAGER::GetVnum` (`item_manager.cpp:721-738`): a vnum by locale name.
    ///
    /// Legacy compares the **argument's** first `len` bytes against `szLocaleName` with
    /// `strncasecmp`, so the argument is a case-insensitive prefix and the first row in sorted
    /// order matches. An empty argument has `len == 0`, which `strncasecmp` calls equal, so it
    /// returns the first row; that is reproduced here. A prefix of nothing is not a real lookup
    /// and callers that do not want it must not pass an empty name.
    pub fn vnum_by_locale_name(&self, name: &[u8]) -> Option<u32> {
        self.by_vnum
            .iter()
            .find(|p| starts_with_ignore_case(&p.locale_name, name))
            .map(|p| p.vnum)
    }
    /// `ITEM_MANAGER::GetVnumByOriginalName` (`item_manager.cpp:740-756`), against `szName`.
    pub fn vnum_by_original_name(&self, name: &[u8]) -> Option<u32> {
        self.by_vnum
            .iter()
            .find(|p| starts_with_ignore_case(&p.name, name))
            .map(|p| p.vnum)
    }

    /// The rows that share a vnum with an earlier row, with the line number of the first copy.
    ///
    /// The owner's file has four of these: `71224` and `71225` each appear twice with different
    /// statistics, and legacy's unstable sort leaves the winner up to its standard library. A
    /// non-empty result is a data defect, and [`ItemProtos::get`] resolves it by taking the first
    /// row in file order.
    pub fn duplicates(&self) -> Vec<(u32, usize)> {
        let mut out: Vec<(u32, usize)> = self
            .by_vnum
            .windows(2)
            .filter(|w| w[0].vnum == w[1].vnum)
            .map(|w| {
                (
                    w[0].vnum,
                    self.by_vnum
                        .binary_search_by_key(&w[0].vnum, |p| p.vnum)
                        .unwrap_or(0),
                )
            })
            .collect();
        out.dedup();
        out
    }
}

/// Whether `name` is a case-insensitive prefix of `haystack`, which is what
/// `strncasecmp(name, haystack, name.len()) == 0` means.
///
/// The fold is ASCII only, because that is what `strncasecmp` does in the C locale the server
/// runs in. The locale names in `item_names.txt` are Latin-9 bytes, so a non-ASCII byte compares
/// as itself and a Romanian letter is never folded.
fn starts_with_ignore_case(haystack: &[u8], name: &[u8]) -> bool {
    if name.len() > haystack.len() {
        return false;
    }
    haystack[..name.len()]
        .iter()
        .zip(name)
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// A row legacy could not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemProtoError {
    /// A proto file could not be read.
    ///
    /// The operating system's message is kept as text rather than as an
    /// [`std::io::Error`], because that type is neither `Clone` nor `Eq` and this
    /// enum is both. The path is kept because "No such file or directory" without
    /// a name is not a diagnostic an Operator can act on.
    Io {
        /// The file that could not be read.
        path: PathBuf,
        /// The operating system's message.
        message: String,
    },
    /// The file did not parse as the tab-separated CSV the legacy reader accepts.
    Csv(CsvError),
    /// A row had fewer than [`COLUMNS`] fields, so `AsStringByIndex` would read past the row.
    ShortRow {
        /// The 1-based line the row was read from, the header included.
        line: usize,
        /// The fields the row has.
        found: usize,
    },
    /// The vnum column was neither a vnum nor a `start~end` range, or the range ran backwards.
    BadVnum {
        /// The 1-based line.
        line: usize,
        /// The column as the file spells it.
        field: Vec<u8>,
    },
    /// `ITEM_TYPE` is not in the type table, which legacy answers with `exit(0)`.
    UnknownType {
        /// The 1-based line.
        line: usize,
        /// The vnum, so the diagnostic is about an item rather than a line number.
        vnum: i32,
        /// The name as the file spells it.
        field: Vec<u8>,
    },
    /// `SUB_TYPE` is not in the sub-type table of its own type, which legacy answers with
    /// `exit(0)`. A type with no table at all is not this: that is a `0` and is fine.
    UnknownSubType {
        /// The 1-based line.
        line: usize,
        /// The vnum.
        vnum: i32,
        /// The name as the file spells it.
        field: Vec<u8>,
    },
    /// A `LIMIT_TYPE` column is not in the limit table, which legacy answers with `exit(0)`.
    UnknownLimitType {
        /// The 1-based line.
        line: usize,
        /// The vnum.
        vnum: i32,
        /// Which of the two limits, 0 or 1.
        slot: usize,
        /// The name as the file spells it.
        field: Vec<u8>,
    },
    /// An `ADDON_TYPE` column is not in the apply table, which legacy answers with `exit(0)`.
    UnknownApplyType {
        /// The 1-based line.
        line: usize,
        /// The vnum.
        vnum: i32,
        /// Which of the three applies, 0, 1, or 2.
        slot: usize,
        /// The name as the file spells it.
        field: Vec<u8>,
    },
    /// A flag column has more `|`-separated tokens than legacy's `StringSplit` buffer holds.
    TooManyFlagTokens {
        /// The 1-based line.
        line: usize,
        /// The vnum.
        vnum: i32,
        /// The column's name, so the diagnostic names the field rather than an index.
        column: &'static str,
        /// The error from [`item_proto_value::flag_mask`].
        source: item_proto_value::TooManyFlagTokens,
    },
}

/// A proto column in a diagnostic, as the Operator reads it. The columns these errors name hold
/// ASCII enum spellings, and a file whose bytes are not UTF-8 still has to produce a message that
/// points at the offending field rather than a list of byte values.
fn as_text(field: &[u8]) -> String {
    String::from_utf8_lossy(field).into_owned()
}

impl fmt::Display for ItemProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => {
                write!(f, "{}: {message}", path.display())
            }
            Self::Csv(source) => write!(f, "could not read the table: {source}"),
            Self::ShortRow { line, found } => write!(
                f,
                "line {line} has {found} fields; legacy reads {COLUMNS} and would read past the row"
            ),
            Self::BadVnum { line, field } => {
                write!(
                    f,
                    "line {line}: vnum column {} is not a vnum or a start~end range",
                    as_text(field)
                )
            }
            Self::UnknownType { line, vnum, field } => write!(
                f,
                "line {line} (vnum {vnum}): ITEM_TYPE {} is not a known item type",
                as_text(field)
            ),
            Self::UnknownSubType { line, vnum, field } => write!(
                f,
                "line {line} (vnum {vnum}): SUB_TYPE {} is not a sub-type of its ITEM_TYPE",
                as_text(field)
            ),
            Self::UnknownLimitType {
                line,
                vnum,
                slot,
                field,
            } => write!(
                f,
                "line {line} (vnum {vnum}): LIMIT_TYPE {} of limit {slot} is not known",
                as_text(field)
            ),
            Self::UnknownApplyType {
                line,
                vnum,
                slot,
                field,
            } => write!(
                f,
                "line {line} (vnum {vnum}): ADDON_TYPE {} of apply {slot} is not known",
                as_text(field)
            ),
            Self::TooManyFlagTokens {
                line,
                vnum,
                column,
                source,
            } => write!(f, "line {line} (vnum {vnum}): {column} column: {source}"),
        }
    }
}

impl Error for ItemProtoError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Csv(source) => Some(source),
            Self::TooManyFlagTokens { source, .. } => Some(source),
            // The `io::Error` was flattened into text when it was read, so there is
            // no source left to hand back and this arm says so rather than guessing.
            _ => None,
        }
    }
}

/// `str_to_number(int&, const char*)` (`server/server/common/utils.h:44-50`) on one field.
///
/// Legacy assigns `(int) strtol(field, NULL, 10)`, which reads the leading integer and ignores
/// everything after it, so `17abc` is 17 and `NONE` is 0. A field of only whitespace, or an empty
/// one, leaves the value alone, and every field starts at the `0` `memset` put there at boot. Both
/// paths land on 0, so there is one result.
///
/// `strtol` is `long`, which is 4 bytes on the legacy i686 target, so a number too large for
/// `i32` saturates at `i32::MAX` rather than wrapping the way a 64-bit reading would.
fn number(field: &[u8]) -> i32 {
    let field = item_proto_value::trim(field);
    let (negative, digits) = match field.split_first() {
        Some((b'-', rest)) => (true, rest),
        _ => (false, field.strip_prefix(b"+").unwrap_or(field)),
    };
    let end = digits
        .iter()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(digits.len());
    if end == 0 {
        return 0;
    }
    // `strtol` saturates at the `long` limit and is signed, so accumulate the magnitude and clamp
    // to the `i32` range at the end. Clamping the magnitude first and negating it would overflow
    // on the one input that matters most for the test.
    let mut value: i64 = 0;
    for b in &digits[..end] {
        value = (value * 10 + i64::from(b - b'0')).min(i64::from(u32::MAX));
    }
    if negative {
        i32::try_from(-value).unwrap_or(i32::MIN)
    } else {
        i32::try_from(value).unwrap_or(i32::MAX)
    }
}

/// The `DWORD` a numeric column holds, which is the `int` above with the same 32 bits.
///
/// `dataArray` is an `int` array and every `DWORD` field is assigned from it, so the conversion is
/// a reinterpretation rather than a second parse. Going through the bytes says exactly that, and
/// it is what a value of `-1` becoming `0xFFFF_FFFF` is: one value, two spellings.
fn dword(field: &[u8]) -> u32 {
    u32::from_le_bytes(number(field).to_le_bytes())
}

/// The `WORD` a numeric column holds: the low 16 bits of the `int` above.
///
/// `wRefineSet` is a `short` and the column is read as an `int`, so this is a truncation and the
/// high half is dropped. No column in the owner's file needs it, and it is pinned synthetically.
fn word(field: &[u8]) -> u16 {
    u16::from_le_bytes(number(field).to_le_bytes()[..2].try_into().unwrap())
}

/// `strlcpy(dst, src, ITEM_NAME_MAX_LEN + 1)` into `char[37]`.
fn strlcpy36(field: &[u8]) -> Vec<u8> {
    let end = field.len().min(ITEM_NAME_MAX_LEN);
    field[..end].to_vec()
}

/// The vnum and range of a `vnum` or `start~end` column.
///
/// `Set_Proto_Item_Table:931-952` splits on the first `~`. A column with no `~` is `dataArray[0]`,
/// which `strtol` produced. A column with one is two `atoi` values, and the row is refused when the
/// start is 0 or the end is non-zero and **less** than the start. An end of 0 is accepted and
/// becomes `dwVnumRange = 0 - start`, which wraps in the `DWORD` the field is stored in and
/// therefore matches almost nothing; legacy does the same, so it is reproduced rather than
/// refused.
fn parse_vnum(line: usize, field: &[u8]) -> Result<(u32, u32), ItemProtoError> {
    let bad = || ItemProtoError::BadVnum {
        line,
        field: field.to_vec(),
    };
    let text = std::str::from_utf8(field).map_err(|_| bad())?;
    match text.split_once('~') {
        None => Ok((dword(field), 0)),
        Some((start, end)) => {
            let start_vnum = atoi(start.as_bytes());
            let end_vnum = atoi(end.as_bytes());
            if start_vnum == 0 || (end_vnum != 0 && end_vnum < start_vnum) {
                return Err(bad());
            }
            // `dwVnumRange = end_vnum - start_vnum` is an `int` subtraction assigned to a
            // `DWORD` (`ProtoReader.cpp:944`), so a `start~0` row stores a wrapped value and the
            // lookup's `dwVnum + dwVnumRange` wraps back to the start.
            let range = end_vnum.wrapping_sub(start_vnum);
            Ok((
                u32::from_le_bytes(start_vnum.to_le_bytes()),
                u32::from_le_bytes(range.to_le_bytes()),
            ))
        }
    }
}

/// `atoi` (`ProtoReader.cpp:945-946`): `strtol` with base 10, stopping at the first non-digit.
fn atoi(field: &[u8]) -> i32 {
    number(field)
}

/// One data row of `item_proto.txt`, as `Set_Proto_Item_Table` turns it into a `TItemTable`.
/// The `LIMITS` and `APPLIES` columns, which are the same shape: `N` pairs of a name and a number.
///
/// Legacy runs two loops over `dataArray[14+i*2]` and `dataArray[18+i*2]`
/// (`ProtoReader.cpp:976-990`): a name column and a value column, stepping by two, `N` times. The
/// only difference between the two loops is which resolver names the first column and which error
/// reports it, so one function parameterised on the pair count and the first column keeps them from
/// drifting and keeps `read_row` readable.
/// The two constructors [`named_values`] needs, which the enum cannot pass as a function pointer.
fn unknown_limit(line: usize, vnum: i32, slot: usize, field: Vec<u8>) -> ItemProtoError {
    ItemProtoError::UnknownLimitType {
        line,
        vnum,
        slot,
        field,
    }
}

/// See [`unknown_limit`].
fn unknown_apply(line: usize, vnum: i32, slot: usize, field: Vec<u8>) -> ItemProtoError {
    ItemProtoError::UnknownApplyType {
        line,
        vnum,
        slot,
        field,
    }
}

fn named_values<const N: usize, const FIRST: usize>(
    line: usize,
    vnum: i32,
    row: &[Vec<u8>],
    resolve: fn(&[u8]) -> Option<i32>,
    unknown: fn(usize, i32, usize, Vec<u8>) -> ItemProtoError,
) -> Result<[ItemValue; N], ItemProtoError> {
    let mut values = [ItemValue::default(); N];
    for (slot, value) in values.iter_mut().enumerate() {
        let column = &row[FIRST + slot * 2];
        let kind = resolve(column).ok_or_else(|| unknown(line, vnum, slot, column.clone()))?;
        *value = ItemValue {
            kind,
            value: number(&row[FIRST + slot * 2 + 1]),
        };
    }
    Ok(values)
}

fn read_row(
    line: usize,
    row: &[Vec<u8>],
    locale_names: &HashMap<i32, Vec<u8>>,
) -> Result<ItemProto, ItemProtoError> {
    if row.len() < COLUMNS {
        return Err(ItemProtoError::ShortRow {
            line,
            found: row.len(),
        });
    }
    let (vnum, vnum_range) = parse_vnum(line, &row[0])?;
    // The locale map is keyed by the `int` `atoi` produced and the lookup passes the `DWORD`, so a
    // vnum above `i32::MAX` is a key no row can hold.
    let vnum_signed = i32::from_le_bytes(vnum.to_le_bytes());

    let item_type = type_value(&row[2]).ok_or_else(|| ItemProtoError::UnknownType {
        line,
        vnum: vnum_signed,
        field: row[2].clone(),
    })?;
    let sub_type = match sub_type_value(item_type, &row[3]) {
        SubType::Unregistered => 0,
        SubType::Value(v) => v,
        SubType::Unknown => {
            return Err(ItemProtoError::UnknownSubType {
                line,
                vnum: vnum_signed,
                field: row[3].clone(),
            })
        }
        SubType::TypeOutOfRange => {
            return Err(ItemProtoError::UnknownType {
                line,
                vnum: vnum_signed,
                field: row[2].clone(),
            })
        }
    };

    let flag = |column: &'static str, index: usize, table: &[&str]| {
        flag_mask(&row[index], table).map_err(|source| ItemProtoError::TooManyFlagTokens {
            line,
            vnum: vnum_signed,
            column,
            source,
        })
    };

    let limits =
        named_values::<LIMITS, 14>(line, vnum_signed, row, limit_type_value, unknown_limit)?;

    let applies =
        named_values::<APPLIES, 18>(line, vnum_signed, row, apply_type_value, unknown_apply)?;

    let name = strlcpy36(&row[1]);
    // `nameMap.find(itemTable->dwVnum)` compares a DWORD against an `int` key
    // (`ProtoReader.cpp:957-963`), so a vnum above `i32::MAX` finds nothing and falls back to the
    // original name. Modelling the map as keyed by `i32` reproduces that without a special case.
    let locale_name = locale_names
        .get(&vnum_signed)
        .map_or_else(|| name.clone(), |n| strlcpy36(n));

    let real_time_first_use = limits
        .iter()
        .position(|l| l.kind == LIMIT_REAL_TIME_START_FIRST_USE);
    let timer_based_on_wear = limits
        .iter()
        .position(|l| l.kind == LIMIT_TIMER_BASED_ON_WEAR);

    let mut values = [0i32; VALUES];
    for (slot, value) in values.iter_mut().enumerate() {
        *value = number(&row[24 + slot]);
    }

    Ok(ItemProto {
        vnum,
        line,
        vnum_range,
        name,
        locale_name,
        item_type,
        sub_type,
        // `str_to_number(itemTable->bWeight, "0")` at `ProtoReader.cpp:1008`.
        weight: 0,
        size: number(&row[4]),
        anti_flags: flag("ANTI_FLAG", 5, ANTI_FLAG)?,
        flags: flag("FLAG", 6, FLAG)?,
        wear_flags: flag("ITEM_WEAR", 7, WEAR_FLAG)?,
        immune_flags: flag("IMMUNE", 8, IMMUNE)?,
        gold: dword(&row[9]),
        shop_buy_price: dword(&row[10]),
        limits,
        applies,
        values,
        sockets: [0; SOCKETS],
        refined_vnum: dword(&row[11]),
        refine_set: word(&row[12]),
        alter_to_magic_item_pct: number(&row[13]),
        // `dataArray[30]` is the Specular column and is then dropped; the field keeps the `memset`.
        specular: 0,
        gain_socket_pct: number(&row[31]),
        addon_type: number(&row[32]),
        real_time_first_use,
        timer_based_on_wear,
    })
}

/// The locale names, from `item_names.txt`.
///
/// Legacy reads it with `atoi` on the vnum column and a plain assignment into a `std::map`
/// (`ClientManagerBoot.cpp:555-561`), so a vnum column with trailing junk still keys on the digits
/// before it, and the **last** row for a vnum wins. Both are reproduced: `162000O` keys on 162000,
/// and there are 36 vnums the file lists twice.
fn read_locale_names(bytes: &[u8]) -> Result<HashMap<i32, Vec<u8>>, ItemProtoError> {
    let rows = csv_table::parse(bytes, b'\t', b'"').map_err(ItemProtoError::Csv)?;
    let mut names = HashMap::new();
    for row in rows.iter().skip(1) {
        if row.len() < 2 {
            continue;
        }
        names.insert(atoi(&row[0]), row[1].clone());
    }
    Ok(names)
}

/// Read `item_proto.txt` and `item_names.txt` the way the legacy DB server reads them.
///
/// # Errors
///
/// Returns the first row that legacy would have answered with `exit(0)`, or a CSV error. Legacy
/// does not continue past such a row, so neither does this.
pub fn parse(proto: &[u8], locale_names: &[u8]) -> Result<ItemProtos, ItemProtoError> {
    let names = read_locale_names(locale_names)?;
    let rows = csv_table::parse_numbered(proto, b'\t', b'"').map_err(ItemProtoError::Csv)?;
    let mut by_vnum = Vec::with_capacity(rows.len().saturating_sub(1));
    // `row 0` is the header, which `ClientManagerBoot.cpp:581` skips with a `Next()`.
    for numbered in rows.iter().skip(1) {
        by_vnum.push(read_row(numbered.line, &numbered.fields, &names)?);
    }
    // `sort(m_vec_itemTable.begin(), m_vec_itemTable.end(), FCompareVnum())` at
    // `ClientManagerBoot.cpp:644` compares `dwVnum` alone. `std::sort` is not stable, so legacy's
    // order for a repeated vnum is unspecified; the `(vnum, line)` key of `from_rows` makes the
    // Rewrite answer with the first row in file order, which is what `RealNumber`'s forward walk
    // and both name lookups reach. See the module documentation.
    // Every row is kept, duplicates included, because `m_vec_itemTable` holds them all and
    // `ItemProtos::duplicates` reports the repeats rather than hiding them. The key is a **total**
    // order: no two rows share `(vnum, line)`, because `line` is a file line. That is what makes
    // the answer independent of the sort, so an unstable sort is equivalent here rather than a
    // different answer. See the module documentation.
    Ok(ItemProtos::from_rows(by_vnum))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::item_proto_value::{APPLY_TYPE, LIMIT_TYPE, TYPE};

    /// The owner's `item_proto.txt` and `item_names.txt`, read fresh.
    fn owners_files() -> (Vec<u8>, Vec<u8>) {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        let read = |name: &str| {
            let path = dir.join(name);
            std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
        };
        (read("item_proto.txt"), read("item_names.txt"))
    }

    fn owners() -> ItemProtos {
        let (proto, names) = owners_files();
        parse(&proto, &names).expect("the owner's item proto loads")
    }

    /// The owner's proto folder, as a path.
    fn owners_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto")
    }

    #[test]
    fn load_reads_the_same_table_parse_reads() {
        // `load` exists so that no caller spells a file name. The property that
        // justifies it is that it and `parse` are the same reader, which is only
        // checkable by comparing their answers over the whole table.
        let by_load = ItemProtos::load(&owners_dir()).expect("the owner's proto folder loads");
        let by_parse = owners();
        assert_eq!(by_load.len(), by_parse.len());
        assert_eq!(by_load.len(), 7_305, "the owner's data-row count");
        for (loaded, parsed) in by_load.rows().iter().zip(by_parse.rows()) {
            assert_eq!(loaded.vnum, parsed.vnum);
            assert_eq!(loaded.size, parsed.size);
            assert_eq!(loaded.name, parsed.name);
            assert_eq!(loaded.locale_name, parsed.locale_name);
        }
    }

    #[test]
    fn a_missing_proto_file_names_itself_rather_than_only_saying_io_failed() {
        // The negative path of the new `Io` variant. The message alone would be
        // "No such file or directory", which an Operator cannot act on; the path is
        // the part that tells them which file.
        let empty = std::env::temp_dir().join("prodomo-no-such-proto-folder");
        let error = ItemProtos::load(&empty).expect_err("an empty folder has no item_proto.txt");
        let ItemProtoError::Io { path, message } = &error else {
            panic!("expected Io, got {error:?}");
        };
        assert!(
            path.ends_with("item_proto.txt"),
            "the path names the file that was missing, got {}",
            path.display()
        );
        assert!(
            !message.is_empty(),
            "the operating system's message is kept"
        );
        // The first file read is the one named, so the error is about
        // `item_proto.txt` and not about a file the reader never reached.
        assert!(error.to_string().contains("item_proto.txt"), "{error}");
    }

    /// A synthetic 35-column row whose resolved values are all zero, so a test can change exactly
    /// one column and know what the rest hold.
    fn blank_row(vnum: &str) -> Vec<Vec<u8>> {
        // Every column holds `0` rather than nothing, because the CSV reader trims the line and a
        // trailing empty column would be stripped before the row is ever seen. The owner's file
        // relies on that: its narrowest rows are 33 fields because columns 33 and 34 are empty.
        let mut row: Vec<Vec<u8>> = (0..35).map(|_| b"0".to_vec()).collect();
        row[0] = vnum.as_bytes().to_vec();
        row[1] = b"SWORD".to_vec();
        row[2] = b"ITEM_WEAPON".to_vec();
        row[3] = b"WEAPON_SWORD".to_vec();
        // The five name columns have no numeric spelling, so they need the "none" member of their
        // own table. `LIMIT_NONE` is index 0 and `APPLY_NONE` is index 0, so a blank row still
        // reports `real_time_first_use == Some(0)`, which is what legacy computes.
        for column in [14, 16] {
            row[column] = b"LIMIT_NONE".to_vec();
        }
        for column in [18, 20, 22] {
            row[column] = b"APPLY_NONE".to_vec();
        }
        row
    }

    fn proto_of(rows: &[Vec<Vec<u8>>]) -> Vec<u8> {
        let mut out = b"VNUM\tNAME\tITEM_TYPE\n".to_vec();
        for row in rows {
            out.extend(row.join(&b'\t'));
            out.push(b'\n');
        }
        out
    }

    fn names_of(rows: &[(u32, &str)]) -> Vec<u8> {
        let mut out = b"VNUM\tLOCALE_NAME\n".to_vec();
        for (vnum, name) in rows {
            out.extend(format!("{vnum}\t{name}\n").bytes());
        }
        out
    }

    /// A clean load: the owner's proto has 35 header columns, 7,305 data rows, and nothing in it
    /// that legacy would have answered with `exit(0)`.
    #[test]
    fn the_owners_file_loads() {
        let protos = owners();
        assert_eq!(protos.len(), 7_305);
        assert!(!protos.is_empty());
        assert_eq!(protos.rows().len(), protos.len());
        assert!(protos.rows().windows(2).all(|w| w[0].vnum <= w[1].vnum));
        assert_eq!(protos.rows()[0].vnum, 1);
        assert_eq!(protos.rows().last().unwrap().vnum, 165_400);
    }

    /// The negative control for the load above: a name the table does not know is refused, and the
    /// diagnostic names the item rather than only a line number.
    #[test]
    fn an_unknown_type_name_is_refused() {
        let mut row = blank_row("1");
        row[2] = b"ITEM_NOT_A_TYPE".to_vec();
        let err = parse(&proto_of(&[row]), &names_of(&[])).unwrap_err();
        assert_eq!(
            err,
            ItemProtoError::UnknownType {
                line: 2,
                vnum: 1,
                field: b"ITEM_NOT_A_TYPE".to_vec(),
            }
        );
        assert!(err.to_string().contains("ITEM_NOT_A_TYPE"));
    }

    /// A sub-type that is not in its own type's table is refused, while a type with no table at
    /// all keeps `0` and is not refused. Both halves matter: the first is legacy's `exit(0)`, the
    /// second is most of the file.
    #[test]
    fn a_bad_subtype_is_refused_but_an_unregistered_type_is_not() {
        let mut bad = blank_row("1");
        bad[3] = b"WEAPON_AXE".to_vec();
        assert_eq!(
            parse(&proto_of(&[bad]), &names_of(&[])).unwrap_err(),
            ItemProtoError::UnknownSubType {
                line: 2,
                vnum: 1,
                field: b"WEAPON_AXE".to_vec(),
            }
        );

        let mut elk = blank_row("50026");
        elk[2] = b"ITEM_ELK".to_vec();
        elk[3] = b"0".to_vec();
        let protos = parse(&proto_of(&[elk]), &names_of(&[])).unwrap();
        assert_eq!(protos.get(50_026).unwrap().sub_type, 0);
    }

    /// The one row in the owner's file that depends on the CSV reader's quote handling: vnum 30341
    /// spells its type as `"ITEM_UNIQUE"`, quotes included. Legacy's `get_Item_Type_Value` would
    /// return -1 for that and `Set_Proto_Item_Table` would call `exit(0)`, so the QUOTE state of
    /// `cCsvFile::Load` is load-bearing here rather than cosmetic. `ITEM_UNIQUE` is index 16 and
    /// its only sub-type is `UNIQUE_NONE` at index 0.
    #[test]
    fn a_quoted_type_field_is_unquoted_by_the_reader() {
        let (proto, _) = owners_files();
        let protos = parse(&proto, &names_of(&[])).unwrap();
        let unique = protos.get(30_341).expect("vnum 30341 is in the file");
        assert_eq!(
            TYPE[usize::try_from(unique.item_type).unwrap()],
            "ITEM_UNIQUE"
        );
        assert_eq!(unique.item_type, 16);
        assert_eq!(unique.sub_type, 0);
    }

    /// The control for the test above: exactly one data line in the file contains a quote, so that
    /// row is the only place the quote handling is observable. A reader that ignored quoting would
    /// fail the row above; one that mis-handled it elsewhere would fail this.
    #[test]
    fn the_only_quoted_data_line_is_the_unique_row() {
        let (proto, _) = owners_files();
        // The NAME column is a legacy code page, so the file is not UTF-8 and the scan is on bytes.
        // Split 0 is the header, so the row on file line N is at index N - 1.
        let lines: Vec<&[u8]> = proto.split(|b| *b == b'\n').collect();
        let quoted: Vec<usize> = lines
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, l)| l.contains(&b'"'))
            .map(|(i, _)| i + 1)
            .collect();
        assert_eq!(quoted, vec![3_886]);
        // And that line is vnum 30341, the row the test above reads.
        assert_eq!(
            lines[3_885].split(|b| *b == b'\t').next(),
            Some(&b"30341"[..])
        );
    }

    /// A row with fewer than the 33 columns legacy reads is refused rather than read past. The
    /// owner's file is ragged but never this short: its narrowest row has 33 fields.
    #[test]
    fn a_short_row_is_refused() {
        let row = blank_row("1")[..12].to_vec();
        assert_eq!(
            parse(&proto_of(&[row]), &names_of(&[])).unwrap_err(),
            ItemProtoError::ShortRow { line: 2, found: 12 }
        );
    }

    /// The owner's file is ragged at the tail, and every extra field is one legacy never reads
    /// (`int dataArray[33]` stops at column 32). A row with 37 or 57 fields loads the same as one
    /// with 35.
    #[test]
    fn extra_columns_beyond_the_thirty_third_are_ignored() {
        let wide: [Vec<Vec<u8>>; 2] = [37usize, 57].map(|n| {
            let mut row = blank_row("1");
            row.resize(n, b"X".to_vec());
            row
        });
        let protos = parse(&proto_of(&wide), &names_of(&[])).unwrap();
        assert_eq!(protos.len(), 2);
        assert_eq!(protos.get(1).unwrap().item_type, 1);
    }

    /// `get_Item_Type_Value` is an exact match, not a substring match, and the one row that proves
    /// it is 30341: a reader that treated it as a prefix or suffix match would resolve
    /// `"ITEM_UNIQUE"` differently, and a reader that matched substrings would resolve nothing.
    #[test]
    fn the_type_column_is_matched_exactly() {
        let mut row = blank_row("1");
        row[2] = b"ITEM_WEAPON_SWORD".to_vec();
        assert_eq!(
            parse(&proto_of(&[row]), &names_of(&[])).unwrap_err(),
            ItemProtoError::UnknownType {
                line: 2,
                vnum: 1,
                field: b"ITEM_WEAPON_SWORD".to_vec(),
            }
        );
    }

    /// A range row is one row covering many vnums, and the stored range excludes **both**
    /// endpoints: `dwVnumRange = end - start` and the lookup is
    /// `dwVnum < vnum < dwVnum + dwVnumRange`. This is the off-by-one a reader of the column
    /// alone would get wrong.
    #[test]
    fn a_range_row_excludes_both_of_its_endpoints() {
        let mut row = blank_row("110000~110099");
        row[1] = b"RANGE".to_vec();
        let protos = parse(&proto_of(&[row]), &names_of(&[])).unwrap();
        let range = &protos.rows()[0];
        assert_eq!(range.vnum, 110_000);
        assert_eq!(range.vnum_range, 99);
        assert!(range.is_range());

        // The start is found as an exact vnum; the interior is found by the range scan.
        assert!(protos.get(110_000).is_some());
        assert_eq!(
            protos.get(110_050).map(|p| p.name.clone()),
            Some(b"RANGE".to_vec())
        );
        // The end is neither: `110099 < 110099` is false, and there is no exact row for it.
        assert!(protos.get(110_099).is_none());
        // A vnum before the start or after the end is not in the row either.
        assert!(protos.get(109_999).is_none());
        assert!(protos.get(110_100).is_none());
    }

    /// An exact row wins over a range row, and between two rows covering the same vnums the one
    /// written **first** wins, because the sort by vnum leaves equal keys in file order. Both are
    /// what the game process sees, because it sorts before it scans.
    #[test]
    fn an_exact_row_wins_over_a_range_and_the_first_written_range_wins() {
        let written_first = {
            let mut r = blank_row("100~199");
            r[1] = b"WRITTEN_FIRST".to_vec();
            r
        };
        let written_second = {
            let mut r = blank_row("100~199");
            r[1] = b"WRITTEN_SECOND".to_vec();
            r
        };
        let exact = {
            let mut r = blank_row("150");
            r[1] = b"EXACT".to_vec();
            r
        };
        // The exact row is written first but has the highest vnum, so the file order is not the
        // sorted order and the first two assertions pin that the sort ran. The two ranges share a
        // vnum, so no sort can reorder them: file order decides, and a reader that searched the
        // ranges before the exact rows would answer a range for 150.
        let protos = parse(
            &proto_of(&[exact, written_first, written_second]),
            &names_of(&[]),
        )
        .unwrap();
        assert_eq!(protos.rows()[0].vnum, 100);
        assert_eq!(protos.rows()[1].vnum, 100);
        assert_eq!(protos.rows()[2].vnum, 150);
        assert_eq!(protos.rows()[0].name, b"WRITTEN_FIRST");
        assert_eq!(
            protos.get(150).map(|p| p.name.clone()),
            Some(b"EXACT".to_vec())
        );
        assert_eq!(
            protos.get(120).map(|p| p.name.clone()),
            Some(b"WRITTEN_FIRST".to_vec())
        );
    }

    /// Two rows share a vnum in the owner's file, with different statistics, and legacy's
    /// `std::sort` is not stable, so its answer is unspecified. This module keeps the first row in
    /// file order; the duplicate is reported so the data defect stays visible.
    #[test]
    fn a_duplicate_vnum_keeps_the_first_row_in_file_order() {
        let first = {
            let mut r = blank_row("7");
            r[1] = b"FIRST".to_vec();
            r[9] = b"10".to_vec();
            r
        };
        let second = {
            let mut r = blank_row("7");
            r[1] = b"SECOND".to_vec();
            r[9] = b"20".to_vec();
            r
        };
        let protos = parse(&proto_of(&[first, second]), &names_of(&[])).unwrap();
        assert_eq!(protos.len(), 2);
        let found = protos.get(7).unwrap();
        assert_eq!(found.name, b"FIRST");
        assert_eq!(found.gold, 10);
        assert_eq!(protos.duplicates().len(), 1);
        assert_eq!(protos.duplicates()[0].0, 7);
    }

    /// The owner's file really does have those duplicates, so the rule above is not hypothetical.
    #[test]
    fn the_owners_file_has_four_duplicate_vnum_rows() {
        let protos = owners();
        let vnums: Vec<u32> = protos.duplicates().iter().map(|(v, _)| *v).collect();
        assert_eq!(vnums, vec![71_224, 71_225]);
        // Two vnums, four rows: each appears twice.
        assert_eq!(vnums.len(), 2);
        let (a, b) = (protos.get(71_224).unwrap(), protos.get(71_225).unwrap());
        let copies: Vec<&ItemProto> = protos
            .rows()
            .iter()
            .filter(|p| p.vnum == 71_224 || p.vnum == 71_225)
            .collect();
        assert_eq!(copies.len(), 4);
        // The two copies of a duplicated vnum share a name and differ only in their applies, which
        // is why the order matters: the kept copy of 71224 moves at 60% and the copy that loses
        // moves at 100%.
        let pair: Vec<&ItemProto> = copies
            .iter()
            .filter(|p| p.vnum == 71_224)
            .copied()
            .collect();
        assert_eq!(pair[0].name, pair[1].name);
        assert_eq!(pair[0].applies[0].value, 60);
        assert_eq!(pair[1].applies[0].value, 100);
        // The kept copies are the ones on lines 5779 and 5780, the first of each pair in the file.
        assert_eq!(a.vnum, 71_224);
        assert_eq!(b.vnum, 71_225);
        assert_eq!(a.applies[0].value, 60);
        assert_eq!(b.limits[0].value, 1_209_600);
        assert_eq!(a.values[1], 20_246);
        assert_eq!(b.values[1], 20_247);
    }

    /// `GetVnum` and `GetVnumByOriginalName` are case-insensitive **prefix** matches against the
    /// first row in sorted order, and they look at different columns: the first at `szLocaleName`,
    /// the second at `szName`.
    #[test]
    fn a_name_lookup_is_a_case_insensitive_prefix() {
        let protos = parse(
            &proto_of(&[{
                let mut r = blank_row("10");
                r[1] = b"Original".to_vec();
                r
            }]),
            &names_of(&[(10, "LocaleName")]),
        )
        .unwrap();
        assert_eq!(protos.vnum_by_locale_name(b"locale"), Some(10));
        assert_eq!(protos.vnum_by_locale_name(b"LOCALE"), Some(10));
        assert_eq!(protos.vnum_by_locale_name(b"Locale"), Some(10));
        assert_eq!(protos.vnum_by_locale_name(b"LocaleNameX"), None);
        // The other direction reads the original name, not the locale name.
        assert_eq!(protos.vnum_by_original_name(b"orig"), Some(10));
        assert_eq!(protos.vnum_by_original_name(b"locale"), None);
    }

    /// An empty argument has length 0, which `strncasecmp` calls equal, so legacy returns the
    /// first row. Reproduced, and pinned, because a caller that turns an empty item name into a
    /// lookup would otherwise get a real vnum instead of a miss.
    #[test]
    fn an_empty_name_matches_the_first_row() {
        let protos = parse(
            &proto_of(&[{
                let mut r = blank_row("10");
                r[1] = b"Original".to_vec();
                r
            }]),
            &names_of(&[(10, "Locale")]),
        )
        .unwrap();
        assert_eq!(protos.vnum_by_locale_name(b""), Some(10));
        assert_eq!(protos.vnum_by_original_name(b""), Some(10));
    }

    /// The prefix fold is ASCII, because that is what `strncasecmp` does in the C locale. The
    /// locale names are Latin-9 bytes, so a non-ASCII byte compares as itself.
    #[test]
    fn only_ascii_letters_are_folded() {
        assert!(starts_with_ignore_case(b"Poarta", b"poarta"));
        assert!(starts_with_ignore_case(b"POARTA", b"Poarta"));
        // 0xE1 is 'a' with an acute in Latin-9 and is not 'a'.
        assert!(!starts_with_ignore_case(&[0xE1, b'b'], b"ab"));
        assert!(!starts_with_ignore_case(&[0xC0, b'B'], b"Ab"));
        assert!(!starts_with_ignore_case(b"ab", b"abc"));
    }

    /// The locale name comes from `item_names.txt`, keyed by the vnum, and falls back to the
    /// original name when the vnum is not listed. The key is `atoi`, so trailing junk is ignored
    /// and the last row for a vnum wins.
    #[test]
    fn the_locale_name_is_keyed_by_atoi_and_the_last_row_wins() {
        let names = names_of(&[(10, "first"), (10, "second"), (11, "third")]);
        let protos = parse(
            &proto_of(&[{
                let mut r = blank_row("10");
                r[1] = b"Original".to_vec();
                r
            }]),
            &names,
        )
        .unwrap();
        assert_eq!(protos.get(10).unwrap().locale_name, b"second");

        // A vnum with no row falls back to its own name.
        let protos = parse(&proto_of(&[blank_row("12")]), &names).unwrap();
        assert_eq!(protos.get(12).unwrap().locale_name, b"SWORD");
    }

    /// The owner's names file has one vnum column with trailing junk, `162000O`, which `atoi`
    /// reads as 162000. A stricter integer parse would drop the name and fall back to the original
    /// Korean name for that item.
    #[test]
    fn the_owners_names_file_has_one_vnum_with_trailing_junk() {
        let (_, names) = owners_files();
        let map = read_locale_names(&names).unwrap();
        assert_eq!(
            map.get(&162_000).map(Vec::as_slice),
            Some(&b"nyx  Dragon Rar"[..])
        );
        // The junk really is in the file, so the test above is not passing by accident.
        assert_eq!(
            owners().get(162_000).map(|p| p.locale_name.clone()),
            Some(b"nyx  Dragon Rar".to_vec())
        );
    }

    /// `str_to_number` is `strtol` with no end pointer, so a field keeps its leading digits and
    /// everything after them is dropped. `APPLY_NONE` and `NONE` appear in the owner's giftbox and
    /// unique rows and read as 0, which is why those rows load at all.
    #[test]
    fn a_numeric_column_keeps_its_leading_digits() {
        assert_eq!(number(b"17"), 17);
        assert_eq!(number(b"17abc"), 17);
        assert_eq!(number(b"NONE"), 0);
        assert_eq!(number(b"APPLY_NONE"), 0);
        assert_eq!(number(b""), 0);
        assert_eq!(number(b"   "), 0);
        assert_eq!(number(b"  -5 "), -5);
        assert_eq!(number(b"+3"), 3);
        assert_eq!(number(b"0x1f"), 0);
        // `strtol` is `long`, which is 4 bytes on the legacy i686 target, so this saturates rather
        // than wrapping the way a 64-bit reading would.
        assert_eq!(number(b"4294967295"), i32::MAX);
        assert_eq!(number(b"99999999999999999999"), i32::MAX);
        assert_eq!(number(b"-99999999999999999999"), i32::MIN);
    }

    /// The four columns the owner's giftbox rows spell as words read as zero, and the rows still
    /// load. Without that, four giftbox items would keep legacy's server from booting.
    #[test]
    fn a_word_in_a_numeric_column_is_zero_and_the_row_still_loads() {
        let mut row = blank_row("71203");
        row[2] = b"ITEM_GIFTBOX".to_vec();
        row[3] = b"0".to_vec();
        row[18] = b"APPLY_NONE".to_vec();
        row[22] = b"APPLY_NONE".to_vec();
        row[29] = b"NONE".to_vec();
        row[30] = b"NONE".to_vec();
        let protos = parse(&proto_of(&[row]), &names_of(&[])).unwrap();
        let giftbox = protos.get(71_203).unwrap();
        assert_eq!(giftbox.applies[0].value, 0);
        assert_eq!(giftbox.applies[2].value, 0);
        assert_eq!(giftbox.values[5], 0);
    }

    /// `bSpecular`, `alSockets` and `bWeight` are never assigned. Column 30 is read into
    /// `dataArray[30]` and then dropped, and the other two are never mentioned, so all three keep
    /// the `memset` zero. A reader that populated `bSpecular` from the file's `Specular` column
    /// would give every item a socket slot legacy never grants.
    #[test]
    fn specular_sockets_and_weight_are_always_zero() {
        let mut row = blank_row("1");
        row[30] = b"7".to_vec();
        row[4] = b"3".to_vec();
        let protos = parse(&proto_of(&[row]), &names_of(&[])).unwrap();
        let item = protos.get(1).unwrap();
        assert_eq!(item.specular, 0);
        assert_eq!(item.sockets, [0; SOCKETS]);
        assert_eq!(item.weight, 0);
        // The SIZE column is a real column and is read.
        assert_eq!(item.size, 3);
    }

    /// The two limit indices are compared against the `ELimitTypes` enum member, not against the
    /// spelling in the file. The file says `REAL_TIME_FIRST_USE` where the enum says
    /// `LIMIT_REAL_TIME_START_FIRST_USE`, and both are index 7, so a reader that matched the name
    /// would leave every real-time limit in the game switched off.
    #[test]
    fn the_limit_indices_come_from_the_enum_numbering() {
        assert_eq!(LIMIT_TYPE[7], "REAL_TIME_FIRST_USE");
        assert_eq!(LIMIT_TYPE[8], "TIMER_BASED_ON_WEAR");

        let mut row = blank_row("1");
        row[14] = b"REAL_TIME_FIRST_USE".to_vec();
        row[16] = b"TIMER_BASED_ON_WEAR".to_vec();
        let protos = parse(&proto_of(&[row]), &names_of(&[])).unwrap();
        let item = protos.get(1).unwrap();
        assert_eq!(item.real_time_first_use, Some(0));
        assert_eq!(item.timer_based_on_wear, Some(1));
        assert_eq!(item.limits[0].kind, 7);
        assert_eq!(item.limits[1].kind, 8);

        // The other slot reports the other index, and a row with neither reports neither.
        let mut row = blank_row("1");
        row[16] = b"REAL_TIME_FIRST_USE".to_vec();
        let protos = parse(&proto_of(&[row]), &names_of(&[])).unwrap();
        let item = protos.get(1).unwrap();
        assert_eq!(item.real_time_first_use, Some(1));
        assert_eq!(item.timer_based_on_wear, None);
    }

    /// A name that is not in a limit or apply table is legacy's `exit(0)`, and each of the five
    /// columns is reported separately so the diagnostic says which one.
    #[test]
    fn every_named_column_is_reported_separately() {
        for (column, error) in [(14, 0), (16, 1), (18, 0), (20, 1), (22, 2)] {
            let mut row = blank_row("1");
            row[column] = b"NOT_A_NAME".to_vec();
            let got = parse(&proto_of(&[row]), &names_of(&[])).unwrap_err();
            let expected = if column < 18 {
                ItemProtoError::UnknownLimitType {
                    line: 2,
                    vnum: 1,
                    slot: error,
                    field: b"NOT_A_NAME".to_vec(),
                }
            } else {
                ItemProtoError::UnknownApplyType {
                    line: 2,
                    vnum: 1,
                    slot: error,
                    field: b"NOT_A_NAME".to_vec(),
                }
            };
            assert_eq!(got, expected, "column {column}");
        }
    }

    /// A flag column is a bitmask, so a name the table does not know is dropped rather than
    /// refused. Legacy behaves the same way, which is why a typo in the data silently loses a
    /// flag instead of stopping the server.
    #[test]
    fn an_unknown_flag_name_is_dropped_and_known_ones_still_count() {
        let mut row = blank_row("1");
        row[5] = b"ANTI_STACK|NOT_A_FLAG|ANTI_DROP".to_vec();
        row[7] = b"WEAR_BODY|WEAR_HEAD".to_vec();
        row[8] = b"PARA".to_vec();
        let protos = parse(&proto_of(&[row]), &names_of(&[])).unwrap();
        let item = protos.get(1).unwrap();
        assert_eq!(item.anti_flags, (1 << 15) | (1 << 7));
        assert_eq!(item.wear_flags, (1 << 0) | (1 << 1));
        assert_eq!(item.immune_flags, 1);
    }

    /// More `|`-separated tokens than legacy's `new string[30]` holds is a buffer overrun there,
    /// and a refusal here. The widest flag column in the owner's file has 9 tokens, so this is a
    /// guard on the data rather than on a row that exists.
    #[test]
    fn a_flag_column_wider_than_the_legacy_buffer_is_refused() {
        let wide = vec![&b"ANTI_DROP"[..]; 31].join(&b'|');
        let mut row = blank_row("1");
        row[5] = wide;
        let got = parse(&proto_of(&[row]), &names_of(&[])).unwrap_err();
        assert!(matches!(
            got,
            ItemProtoError::TooManyFlagTokens {
                column: "ANTI_FLAG",
                ..
            }
        ));
        assert!(got.to_string().contains("ANTI_FLAG"));

        let (proto, _) = owners_files();
        let rows = csv_table::parse(&proto, b'\t', b'"').unwrap();
        let widest = rows
            .iter()
            .skip(1)
            .flat_map(|row| [5usize, 6, 7, 8].map(|c| (row, c)))
            .map(|(row, c)| {
                item_proto_value::split_flags(row.get(c).map_or(&[][..], Vec::as_slice)).len()
            })
            .max()
            .unwrap();
        assert!(widest <= item_proto_value::MAX_FLAG_TOKENS);
    }

    /// The plain numeric columns land where `Set_Proto_Item_Table:964-1006` puts them. A shift by
    /// one column would still produce a loadable file, so the values are pinned individually.
    #[test]
    fn the_plain_numeric_columns_land_in_the_right_fields() {
        let mut row = blank_row("1");
        row[4] = b"4".to_vec(); // SIZE
        row[9] = b"90".to_vec(); // GOLD
        row[10] = b"91".to_vec(); // SHOP_BUY_PRICE
        row[11] = b"92".to_vec(); // REFINE
        row[12] = b"93".to_vec(); // REFINESET
        row[13] = b"94".to_vec(); // MAGIC_PCT
        row[15] = b"95".to_vec(); // LIMIT_VALUE 0
        row[17] = b"96".to_vec(); // LIMIT_VALUE 1
        row[19] = b"97".to_vec(); // ADDON_VALUE 0
        row[21] = b"98".to_vec(); // ADDON_VALUE 1
        row[23] = b"99".to_vec(); // ADDON_VALUE 2
        for slot in 0..6 {
            row[24 + slot] = format!("{}", 100 + slot).into_bytes();
        }
        row[31] = b"30".to_vec(); // SOCKET
        row[32] = b"31".to_vec(); // ATTU_ADDON
        let protos = parse(&proto_of(&[row]), &names_of(&[])).unwrap();
        let item = protos.get(1).unwrap();
        assert_eq!(item.size, 4);
        assert_eq!(item.gold, 90);
        assert_eq!(item.shop_buy_price, 91);
        assert_eq!(item.refined_vnum, 92);
        assert_eq!(item.refine_set, 93);
        assert_eq!(item.alter_to_magic_item_pct, 94);
        assert_eq!(item.limits[0].value, 95);
        assert_eq!(item.limits[1].value, 96);
        assert_eq!(item.applies[0].value, 97);
        assert_eq!(item.applies[1].value, 98);
        assert_eq!(item.applies[2].value, 99);
        assert_eq!(item.values, [100, 101, 102, 103, 104, 105]);
        assert_eq!(item.gain_socket_pct, 30);
        assert_eq!(item.addon_type, 31);
    }

    /// An apply value can be negative, and a `BYTE` in the legacy struct does not stop it: the
    /// column is read with `str_to_number(int&)` and then narrowed by the assignment.
    #[test]
    fn an_apply_value_may_be_negative() {
        let mut row = blank_row("1");
        row[18] = b"APPLY_MOV_SPEED".to_vec();
        row[19] = b"-3".to_vec();
        let protos = parse(&proto_of(&[row]), &names_of(&[])).unwrap();
        let item = protos.get(1).unwrap();
        assert_eq!(item.applies[0].kind, 8);
        assert_eq!(item.applies[0].value, -3);
        assert_eq!(
            APPLY_TYPE[usize::try_from(item.applies[0].kind).unwrap()],
            "APPLY_MOV_SPEED"
        );
    }

    /// `strlcpy` into `char[ITEM_NAME_MAX_LEN + 1]` truncates. No name in the owner's file is that
    /// long, so the truncation is pinned with a synthetic row instead of being left untested.
    #[test]
    fn a_name_longer_than_the_struct_is_truncated() {
        let long = "N".repeat(50);
        let mut row = blank_row("1");
        row[1] = long.as_bytes().to_vec();
        let protos = parse(&proto_of(&[row]), &names_of(&[(1, "L")])).unwrap();
        let item = protos.get(1).unwrap();
        assert_eq!(item.name.len(), ITEM_NAME_MAX_LEN);
        assert_eq!(item.locale_name, b"L");
    }

    /// A range whose end is below its start is refused, and a range starting at 0 is refused. An
    /// end of 0 is accepted by legacy and wraps the range, which is reproduced rather than fixed.
    #[test]
    fn a_backwards_or_zero_started_range_is_refused() {
        for field in ["0~99", "200~100"] {
            let row = blank_row(field);
            assert_eq!(
                parse(&proto_of(&[row]), &names_of(&[])).unwrap_err(),
                ItemProtoError::BadVnum {
                    line: 2,
                    field: field.as_bytes().to_vec(),
                },
                "{field}"
            );
        }
        let row = blank_row("100~0");
        let protos = parse(&proto_of(&[row]), &names_of(&[])).unwrap();
        // `0u32 - 100u32` wraps, and so does the `DWORD` sum the scan compares against, so
        // `dwVnum + dwVnumRange` is 0 and the row covers nothing at all. Legacy stores and adds in
        // the same width, so a `start~0` row is a silent no-op rather than an open-ended range.
        assert_eq!(protos.rows()[0].vnum_range, u32::MAX - 99);
        assert!(protos.get(100).is_some());
        assert!(protos.get(101).is_none());
        assert!(protos.get(u32::MAX).is_none());
    }

    /// The names file is optional in the sense that an empty one loads, and its header is skipped
    /// the way `ClientManagerBoot.cpp:555` skips it. An empty proto file is a table with no rows.
    #[test]
    fn an_empty_names_file_and_a_header_only_proto_are_both_accepted() {
        let protos = parse(&proto_of(&[]), &names_of(&[])).unwrap();
        assert!(protos.is_empty());
        assert_eq!(protos.len(), 0);
        assert!(protos.get(1).is_none());
        assert!(protos.vnum_by_locale_name(b"anything").is_none());
    }

    /// The exact lookup answers with the **first** row of a duplicated vnum. Legacy's `RealNumber`
    /// walks the vector from the front, so this is what it returns; a binary search that answers
    /// with whichever equal element it lands on would return the second copy's statistics, and the
    /// two copies of 71224 in the owner's file disagree.
    #[test]
    fn the_exact_lookup_answers_with_the_first_copy_of_a_duplicate() {
        let mut row = {
            let mut r = blank_row("50");
            r[1] = b"WINNER".to_vec();
            r[9] = b"1".to_vec();
            r
        };
        let winner = row.clone();
        row[1] = b"LOSER".to_vec();
        row[9] = b"2".to_vec();
        let protos = parse(&proto_of(&[winner, row]), &names_of(&[])).unwrap();
        assert_eq!(protos.get(50).unwrap().gold, 1);

        // A miss on either side of the duplicate still misses, so the lower bound did not widen the
        // match to a neighbouring vnum.
        assert!(protos.get(49).is_none());
        assert!(protos.get(51).is_none());
    }

    /// A diagnostic names the offending field as text an Operator can search the file for, not as a
    /// list of byte values. The columns these errors name hold ASCII enum spellings, so the message
    /// is exact.
    #[test]
    fn a_diagnostic_names_the_field_as_text() {
        let mut row = blank_row("1");
        row[18] = b"APPLY_NOT_REAL".to_vec();
        let err = parse(&proto_of(&[row]), &names_of(&[])).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("APPLY_NOT_REAL"), "{text}");
        assert!(text.contains("ADDON_TYPE"), "{text}");
        assert!(text.contains("apply 0"), "{text}");
        assert!(!text.contains('['), "{text}");
    }

    /// A field of bytes that are not UTF-8 still produces a message rather than a panic, and keeps
    /// the replacement character where the bytes are not text. The NAME column of the owner's file
    /// is a legacy code page, so a future column could be too.
    #[test]
    fn a_diagnostic_survives_a_field_that_is_not_utf8() {
        let mut row = blank_row("1");
        row[2] = vec![0x80, 0xFE, b'X'];
        let err = parse(&proto_of(&[row]), &names_of(&[])).unwrap_err();
        let text = err.to_string();
        assert!(text.contains('X'), "{text}");
        assert!(text.contains('\u{FFFD}'), "{text}");
    }

    /// The rows are ordered by `(vnum, line)`, and the second half of that key is what makes a
    /// repeated vnum deterministic. Asserting the property rather than the sort's stability is the
    /// point: replacing the sort with an unstable one, or dropping the line, is then a failure even
    /// though `sort_by_key` happens to be stable today.
    #[test]
    fn the_rows_are_ordered_by_vnum_and_then_by_file_line() {
        let protos = owners();
        assert!(protos
            .rows()
            .windows(2)
            .all(|w| (w[0].vnum, w[0].line) < (w[1].vnum, w[1].line)));
        // The two duplicated vnums are the only places the second half of the key does any work, and
        // their line numbers are far apart, so the file order is not what the sort produced.
        let first_71_224 = protos.rows().iter().find(|p| p.vnum == 71_224).unwrap();
        let second_71_224 = protos
            .rows()
            .iter()
            .find(|p| p.vnum == 71_224 && p.line != first_71_224.line)
            .unwrap();
        assert_eq!(first_71_224.line, 5_779);
        assert_eq!(second_71_224.line, 6_913);
        // And the lookup answers with the lower line, whatever the sort did with the two.
        assert_eq!(protos.get(71_224).unwrap().line, 5_779);
    }
}
