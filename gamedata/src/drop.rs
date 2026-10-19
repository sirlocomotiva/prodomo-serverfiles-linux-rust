//! The drop tables: what a monster's death may pay
//! (`server/server/game/item_manager_read_tables.cpp`).
//!
//! Four files under `locale/europe/`, read at boot by `BSelectCalendar`
//! (`G/input_db.cpp:899-942`): a missing file stops the server, and an empty one loads fine.
//!
//! | file | reader | owner content |
//! |---|---|---|
//! | `common_drop_item.txt` | [`CommonDrops`] | 0 bytes |
//! | `etc_drop_item.txt` | [`EtcDrops`] | 0 bytes |
//! | `drop_item_group.txt` | [`DropGroups`] | 0 bytes |
//! | `mob_drop_item.txt` | [`MobDropGroups`] | 1 `limit` group, mob 2493, 21 entries |
//!
//! Every reader resolves an item by the original name first (`strncasecmp` prefix match,
//! [`ItemProtos::vnum_by_original_name`]), then as a vnum; an item neither answers for
//! refuses the whole file. Percents are floats ×10000 truncated to `u32`.
//!
//! Legacy behaviours reproduced rather than improved:
//!
//! - A common-drop rank with `lv_start == 0` is skipped silently.
//! - An etc row without a tab, with an empty name, or with a 0.0 probability is skipped
//!   silently.
//! - A `drop_item_group.txt` mob written twice appends to the first group.
//! - A `mob_drop_item.txt` group with `kill_drop == 0` is skipped silently.
//! - A `kill`, `limit` or `thiefgloves` mob written twice keeps the first group.
//! - A `CTextFileLoader` key written twice keeps the first row (key 12 of the owner's
//!   group is vnum 12540; 12680 is lost).
//!
//! Defects NOT reproduced:
//!
//! - The `kr_` skip reads the name before `SetChildNode`, so it tests the parent's name
//!   and never fires (`:530-535`). The Rewrite tests the child's own name.
//! - A `kill` short row indexes `pTok->at(1..3)` unchecked and throws
//!   `std::out_of_range`. The Rewrite refuses the file with the row named.
//! - A `drop` duplicate mob shadows `pkGroup` and uses the outer uninitialised pointer
//!   (`:663-667`, undefined behaviour). The Rewrite appends to the existing group.
//! - The common-table sort is `std::sort`, unstable on tied level ends. The Rewrite
//!   sorts stably (a Divergence that needs tied ends to show; the owner's file is empty).

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::Path;

use crate::item_proto::ItemProtos;
use crate::text_file::{self, TextFile};

/// `MOB_RANK_S_KNIGHT` (`common/length.h:459-462`): only ranks 0-3 have common-drop
/// columns, so bosses and kings never get common drops.
pub const COMMON_RANKS: usize = 4;

/// The last row key the group loops read: `for (int k = 1; k < 256; ++k)`.
pub const MAX_ROW: usize = 255;

/// One common-drop row: a level band, a percent ×10000, an item and a count
/// (`CItemDropInfo`, `G/item_manager_private_types.h:1-18`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommonDrop {
    /// The band's first level.
    pub lv_start: i32,
    /// The band's last level; the sort key.
    pub lv_end: i32,
    /// The percent ×10000.
    pub pct: u32,
    /// The item's vnum.
    pub vnum: u32,
    /// How many drop.
    pub count: i32,
}

/// `common_drop_item.txt`: one sorted vector per rank (`g_vec_pkCommonDropItem`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommonDrops {
    /// The four rank columns, each sorted by level end.
    pub ranks: [Vec<CommonDrop>; COMMON_RANKS],
}

/// `etc_drop_item.txt`: the proto's `DROP_ITEM` vnum to its probability ×10000
/// (`m_map_dwEtcItemDropProb`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EtcDrops {
    /// The vnum to its probability.
    pub probs: BTreeMap<u32, u32>,
}

/// One group row: an item, a percent ×10000 and a count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropItem {
    /// The item's vnum.
    pub vnum: u32,
    /// The percent ×10000.
    pub pct: u32,
    /// How many drop.
    pub count: i32,
}

/// One `drop_item_group.txt` group (`CDropItemGroup`), keyed by mob vnum.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DropGroup {
    /// The group's `vnum` token (0 for a `mob_drop_item.txt` `drop` group).
    pub vnum: i32,
    /// The rows in key order.
    pub items: Vec<DropItem>,
}

/// `drop_item_group.txt` plus the `drop` groups of `mob_drop_item.txt`: both files append
/// to `m_map_pkDropItemGroup`, keyed by mob vnum.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DropGroups {
    /// The mob vnum to its group.
    pub groups: BTreeMap<u32, DropGroup>,
}

/// One `kill` row (`CMobItemGroup::AddItem`): an item, a count, a part percent and a rare
/// percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobItem {
    /// The item's vnum.
    pub vnum: u32,
    /// How many drop.
    pub count: i32,
    /// The part percent (an integer, never 0).
    pub part_pct: i32,
    /// The rare percent, clamped 0..=100.
    pub rare_pct: i32,
}

/// One `mob_drop_item.txt` `kill` group (`CMobItemGroup`), keyed by mob vnum.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MobGroup {
    /// The `kill_drop` token.
    pub kill_drop: i32,
    /// The rows in key order.
    pub items: Vec<MobItem>,
}

/// One `mob_drop_item.txt` `limit` group (`CLevelItemGroup`), keyed by mob vnum.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LevelGroup {
    /// The `level_limit` token: the killer must be at least this level.
    pub level_limit: i32,
    /// The rows in key order.
    pub items: Vec<DropItem>,
}

/// `mob_drop_item.txt`: the `kill`, `limit` and `thiefgloves` groups by mob vnum. The
/// `drop` groups land in [`DropGroups`], as legacy's shared map does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MobDropGroups {
    /// `m_map_pkMobItemGroup`.
    pub kill: BTreeMap<u32, MobGroup>,
    /// `m_map_pkLevelItemGroup`.
    pub limit: BTreeMap<u32, LevelGroup>,
    /// `m_map_pkGloveItemGroup`.
    pub gloves: BTreeMap<u32, DropGroup>,
}

/// The four drop tables of one Locale, as [`DropTables::load`] reads them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DropTables {
    /// `common_drop_item.txt`.
    pub common: CommonDrops,
    /// `etc_drop_item.txt`.
    pub etc: EtcDrops,
    /// `drop_item_group.txt` and the `mob_drop_item.txt` `drop` groups.
    pub groups: DropGroups,
    /// `mob_drop_item.txt`.
    pub mob: MobDropGroups,
}

/// A drop file legacy could not read: every one of these refuses the whole file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropError {
    /// The file is missing or unreadable; legacy logs `Cannot open` and shuts down.
    Unreadable {
        /// The file's name.
        file: &'static str,
    },
    /// The text reader would not cut the file.
    Text(crate::text_file::TextFileError),
    /// A row names an item neither the original names nor the vnums answer for.
    UnknownItem {
        /// The file's name.
        file: &'static str,
        /// The group, or the empty string for the flat files.
        group: Vec<u8>,
        /// The row's key, or the 1-based line number for the flat files.
        row: usize,
        /// The name as the file wrote it.
        name: Vec<u8>,
    },
    /// A group token is missing.
    MissingToken {
        /// The file's name.
        file: &'static str,
        /// The group.
        group: Vec<u8>,
        /// The token legacy asked for.
        token: &'static str,
    },
    /// A `kill` row is too short for legacy's `at(1..3)` (which would throw).
    ShortRow {
        /// The group.
        group: Vec<u8>,
        /// The row's key.
        row: usize,
        /// How many fields the row holds.
        fields: usize,
    },
    /// A count below 1 where legacy requires one.
    BadCount {
        /// The file's name.
        file: &'static str,
        /// The group.
        group: Vec<u8>,
        /// The row's key.
        row: usize,
    },
    /// A `kill` part percent of 0.
    BadPartPct {
        /// The group.
        group: Vec<u8>,
        /// The row's key.
        row: usize,
    },
    /// A `mob_drop_item.txt` group whose type is none of the four.
    BadType {
        /// The group.
        group: Vec<u8>,
        /// The type as the file wrote it.
        word: Vec<u8>,
    },
}

impl fmt::Display for DropError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { file } => write!(f, "{file} is missing or unreadable"),
            Self::Text(error) => write!(f, "{error}"),
            Self::UnknownItem {
                file,
                group,
                row,
                name,
            } => write!(
                f,
                "{file} row {row} of {} names no item: {}",
                String::from_utf8_lossy(group),
                String::from_utf8_lossy(name),
            ),
            Self::MissingToken { file, group, token } => write!(
                f,
                "{file} group {} has no {token}",
                String::from_utf8_lossy(group),
            ),
            Self::ShortRow { group, row, fields } => write!(
                f,
                "group {} row {row} holds {fields} fields, less than a kill row's 4",
                String::from_utf8_lossy(group),
            ),
            Self::BadCount { file, group, row } => write!(
                f,
                "{file} group {} row {row} has no count",
                String::from_utf8_lossy(group),
            ),
            Self::BadPartPct { group, row } => write!(
                f,
                "group {} row {row} has no drop percent",
                String::from_utf8_lossy(group),
            ),
            Self::BadType { group, word } => write!(
                f,
                "group {} has type {}, not kill, drop, limit or thiefgloves",
                String::from_utf8_lossy(group),
                String::from_utf8_lossy(word),
            ),
        }
    }
}

impl Error for DropError {}

impl From<crate::text_file::TextFileError> for DropError {
    fn from(error: crate::text_file::TextFileError) -> Self {
        Self::Text(error)
    }
}

/// `str_to_number` into an `int`: leading space, an optional sign, then digits; anything
/// else reads 0 from there.
fn str_to_i32(bytes: &[u8]) -> i32 {
    let (magnitude, negative) = scan_decimal(bytes);
    let value = if negative {
        magnitude.wrapping_neg()
    } else {
        magnitude
    };
    i32::from_le_bytes(low_4(value))
}

/// `str_to_number` into a `DWORD`.
fn str_to_u32(bytes: &[u8]) -> u32 {
    let (magnitude, negative) = scan_decimal(bytes);
    let value = if negative {
        magnitude.wrapping_neg()
    } else {
        magnitude
    };
    u32::from_le_bytes(low_4(value))
}

/// The decimal scan both readers share: the magnitude and whether it was negative.
fn scan_decimal(bytes: &[u8]) -> (u64, bool) {
    let mut bytes = bytes.iter().peekable();
    while bytes.peek().is_some_and(|b| b.is_ascii_whitespace()) {
        bytes.next();
    }
    let negative = bytes.peek() == Some(&&b'-');
    if negative || bytes.peek() == Some(&&b'+') {
        bytes.next();
    }
    let mut magnitude: u64 = 0;
    for b in bytes {
        if !b.is_ascii_digit() {
            break;
        }
        magnitude = magnitude.wrapping_mul(10).wrapping_add(u64::from(b - b'0'));
    }
    (magnitude, negative)
}

/// The low four bytes of `value`.
const fn low_4(value: u64) -> [u8; 4] {
    let bytes = value.to_le_bytes();
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

/// `atof` on a token: what a float read of the bytes gives, or 0.0 when no float starts
/// there.
fn str_to_f32(bytes: &[u8]) -> f32 {
    let text = bytes
        .iter()
        .take_while(|b| !b.is_ascii_whitespace())
        .copied()
        .collect::<Vec<_>>();
    let text = String::from_utf8_lossy(&text);
    text.parse::<f32>().unwrap_or(0.0)
}

/// `(DWORD)(percent * 10000.0f)`: the truncation every percent keeps.
///
/// Out of range the x86 conversion saturates to `u32::MAX`; a negative value or NaN keeps
/// 0. The conversion decodes the float's own bits, so no float-to-int cast appears: the
/// mantissa shifts into a `u64` by the unbiased exponent, and anything past 32 bits
/// saturates.
fn pct_of(percent: f32) -> u32 {
    float_trunc_to_u32((percent * 10000.0).trunc())
}

/// Truncate `value` toward zero into a `u32`, saturating out-of-range values.
fn float_trunc_to_u32(value: f32) -> u32 {
    const MANTISSA_BITS: u32 = 23;
    const EXPONENT_BIAS: i32 = 127;
    const IMPLICIT_ONE: u64 = 1 << MANTISSA_BITS;
    let bits = value.to_bits();
    if bits & 0x8000_0000 != 0 {
        return 0;
    }
    let exponent = (bits >> MANTISSA_BITS) & 0xFF;
    if exponent == 0xFF {
        // Infinity saturates, as the x86 conversion does; NaN keeps 0, which fails
        // closed on a garbage percent rather than paying the ceiling.
        let mantissa_bits = bits << (32 - MANTISSA_BITS);
        return if mantissa_bits == 0 { u32::MAX } else { 0 };
    }
    let shift = i32::try_from(exponent).unwrap_or(i32::MAX) - EXPONENT_BIAS - 23;
    let mantissa = u64::from(bits & 0x7F_FFFF) | IMPLICIT_ONE;
    if shift >= 64 || (shift >= 0 && mantissa > (u64::from(u32::MAX) >> shift)) {
        return u32::MAX;
    }
    let truncated = if shift >= 0 {
        mantissa << shift
    } else if shift > -64 {
        mantissa >> (-shift)
    } else {
        return 0;
    };
    u32::try_from(truncated).unwrap_or(u32::MAX)
}

/// The item `name` means: the original name first, then the vnum, else `None`
/// (`GetVnumByOriginalName`, then `str_to_number` and `GetTable`).
fn resolve_item(protos: &ItemProtos, name: &[u8]) -> Option<u32> {
    if let Some(vnum) = protos.vnum_by_original_name(name) {
        return Some(vnum);
    }
    let vnum = str_to_u32(name);
    protos.get(vnum).map(|_| vnum)
}

/// `ReadCommonDropItemFile` (`item_manager_read_tables.cpp:25-117`).
fn read_common(data: &[u8], protos: &ItemProtos) -> Result<CommonDrops, DropError> {
    let mut out = CommonDrops::default();
    for (index, line) in data.split(|b| *b == b'\n').enumerate() {
        // `fgets` keeps the `\n`; the `*buf == '\n'` test drops the empty line.
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        if line.is_empty() || line == b"\n" {
            continue;
        }
        let mut fields = line.split(|b| *b == b'\t');
        // The `strchr` walk: a missing tab breaks the inner loop and the rank keeps what
        // it read.
        let mut next = move || fields.next();
        for rank in out.ranks.iter_mut().take(COMMON_RANKS) {
            let mut cells: [Option<&[u8]>; 6] = [None; 6];
            for cell in &mut cells {
                let Some(field) = next() else {
                    break;
                };
                *cell = Some(field);
            }
            // `d[i].szItemName` starts zeroed; an unread name resolves as the empty
            // string, which no proto answers for — but only a rank with a nonzero
            // start is kept, and an unread start is 0, so the rank is skipped below.
            let name = cells[4].unwrap_or_default();
            let lv_start = cells[1].map_or(0, str_to_i32);
            if lv_start == 0 {
                continue;
            }
            let vnum = resolve_item(protos, name).ok_or_else(|| DropError::UnknownItem {
                file: "common_drop_item.txt",
                group: Vec::new(),
                row: index + 1,
                name: name.to_vec(),
            })?;
            rank.push(CommonDrop {
                lv_start,
                lv_end: cells[2].map_or(0, str_to_i32),
                pct: pct_of(cells[3].map_or(0.0, str_to_f32)),
                vnum,
                count: cells[5].map_or(0, str_to_i32),
            });
        }
    }
    // `std::sort` by level end; the Rewrite's sort is stable (a Divergence on ties).
    for rank in &mut out.ranks {
        rank.sort_by_key(|row| row.lv_end);
    }
    Ok(out)
}

/// `ReadEtcDropItemFile` (`item_manager_read_tables.cpp:461-517`).
fn read_etc(data: &[u8], protos: &ItemProtos) -> Result<EtcDrops, DropError> {
    let mut out = EtcDrops::default();
    for (index, line) in data.split(|b| *b == b'\n').enumerate() {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        if line.is_empty() || line == b"\n" {
            continue;
        }
        // `strrchr(szItemName, '\t')`: the name runs to the LAST tab.
        let Some(tab) = line.iter().rposition(|b| *b == b'\t') else {
            continue;
        };
        let (name, prob) = (&line[..tab], str_to_f32(&line[tab + 1..]));
        if name.is_empty() || prob == 0.0 {
            continue;
        }
        let vnum = resolve_item(protos, name).ok_or_else(|| DropError::UnknownItem {
            file: "etc_drop_item.txt",
            group: Vec::new(),
            row: index + 1,
            name: name.to_vec(),
        })?;
        out.probs.insert(vnum, pct_of(prob));
    }
    Ok(out)
}

/// A `GetTokenInteger` read: false only when the key is missing or holds nothing; a value
/// that is not a number reads 0.
fn token_i32(group: &text_file::TextGroup, key: &[u8]) -> Option<i32> {
    group
        .get(key)
        .filter(|v| !v.is_empty())
        .map(|v| str_to_i32(&v[0]))
}

/// A `GetTokenString` read.
fn token_string<'a>(group: &'a text_file::TextGroup, key: &[u8]) -> Option<&'a [u8]> {
    group.get(key).and_then(|v| v.first().map(Vec::as_slice))
}

/// One resolved `GetTokenVector` row: its item's vnum and its raw fields.
type ResolvedRow = (u32, Vec<Vec<u8>>);

/// A resolved row with its 1-based key: `read_rows` counts from 0, the file from 1.
type KeyedRow = (usize, ResolvedRow);

/// A `GetTokenVector` row read: the numbered keys `1..=255`, ending at the first missing
/// one.
fn read_rows(
    group: &text_file::TextGroup,
    protos: &ItemProtos,
    file: &'static str,
) -> Result<Vec<ResolvedRow>, DropError> {
    let mut rows = Vec::new();
    for key in 1..=MAX_ROW {
        let Some(row) = group.get(key.to_string().as_bytes()) else {
            break;
        };
        if row.is_empty() {
            break;
        }
        let vnum = resolve_item(protos, &row[0]).ok_or_else(|| DropError::UnknownItem {
            file,
            group: group.name.clone(),
            row: key,
            name: row[0].clone(),
        })?;
        rows.push((vnum, row.to_vec()));
    }
    Ok(rows)
}

/// [`read_rows`] with each row's 1-based key, for the error that names the row.
fn keyed_rows(
    group: &text_file::TextGroup,
    protos: &ItemProtos,
    file: &'static str,
) -> Result<Vec<KeyedRow>, DropError> {
    read_rows(group, protos, file).map(|rows| {
        rows.into_iter()
            .enumerate()
            .map(|(index, row)| (index + 1, row))
            .collect()
    })
}

/// One `(name, count, percent)` row of a `drop`, `limit` or `thiefgloves` group: the
/// count is required, the percent defaults to 0.
fn count_pct_row(
    group: &text_file::TextGroup,
    key: usize,
    vnum: u32,
    row: &[Vec<u8>],
) -> Result<DropItem, DropError> {
    let count = row.get(1).map_or(0, |b| str_to_i32(b));
    if count < 1 {
        return Err(DropError::BadCount {
            file: "mob_drop_item.txt",
            group: group.name.clone(),
            row: key,
        });
    }
    let pct = pct_of(row.get(2).map_or(0.0, |b| str_to_f32(b)));
    Ok(DropItem { vnum, pct, count })
}

/// `ReadDropItemGroup` (`item_manager_read_tables.cpp:834-936`): rows are
/// (name, percent, count?), appended to the mob's group across duplicate mobs.
fn read_group_file(
    data: &[u8],
    protos: &ItemProtos,
    into: &mut DropGroups,
) -> Result<(), DropError> {
    let file: TextFile = text_file::parse(data)?;
    for group in &file.groups {
        let vnum = token_i32(group, b"vnum").ok_or_else(|| DropError::MissingToken {
            file: "drop_item_group.txt",
            group: group.name.clone(),
            token: "vnum",
        })?;
        let mob = token_i32(group, b"mob").ok_or_else(|| DropError::MissingToken {
            file: "drop_item_group.txt",
            group: group.name.clone(),
            token: "mob",
        })?;
        let entry = into
            .groups
            .entry(u32::from_ne_bytes(mob.to_ne_bytes()))
            .or_insert_with(|| DropGroup {
                vnum,
                items: Vec::new(),
            });
        for (key, (vnum, row)) in read_rows(group, protos, "drop_item_group.txt")?
            .into_iter()
            .enumerate()
            .map(|(i, r)| (i + 1, r))
        {
            let pct = pct_of(row.get(1).map_or(0.0, |b| str_to_f32(b)));
            let count = row.get(2).map_or(1, |b| str_to_i32(b));
            if count < 1 {
                return Err(DropError::BadCount {
                    file: "drop_item_group.txt",
                    group: group.name.clone(),
                    row: key,
                });
            }
            entry.items.push(DropItem { vnum, pct, count });
        }
    }
    Ok(())
}

/// `ReadMonsterDropItemGroup` (`item_manager_read_tables.cpp:519-832`).
fn read_mob_file(
    data: &[u8],
    protos: &ItemProtos,
    groups: &mut DropGroups,
) -> Result<MobDropGroups, DropError> {
    let file: TextFile = text_file::parse(data)?;
    let mut out = MobDropGroups::default();
    for group in &file.groups {
        read_mob_group(group, protos, groups, &mut out)?;
    }
    Ok(out)
}

/// One `kill` group: the `kill_drop` skip, the four-field rows, kept first-wins.
fn read_kill_group(
    group: &text_file::TextGroup,
    mob: i32,
    protos: &ItemProtos,
    out: &mut MobDropGroups,
) -> Result<(), DropError> {
    let kill_drop = token_i32(group, b"kill_drop").ok_or_else(|| DropError::MissingToken {
        file: "mob_drop_item.txt",
        group: group.name.clone(),
        token: "kill_drop",
    })?;
    if kill_drop == 0 {
        return Ok(());
    }
    let mut bag = MobGroup {
        kill_drop,
        items: Vec::new(),
    };
    for (key, (vnum, row)) in keyed_rows(group, protos, "mob_drop_item.txt")? {
        if row.len() < 4 {
            return Err(DropError::ShortRow {
                group: group.name.clone(),
                row: key,
                fields: row.len(),
            });
        }
        let count = str_to_i32(&row[1]);
        if count < 1 {
            return Err(DropError::BadCount {
                file: "mob_drop_item.txt",
                group: group.name.clone(),
                row: key,
            });
        }
        let part_pct = str_to_i32(&row[2]);
        if part_pct == 0 {
            return Err(DropError::BadPartPct {
                group: group.name.clone(),
                row: key,
            });
        }
        let rare_pct = str_to_i32(&row[3]).clamp(0, 100);
        bag.items.push(MobItem {
            vnum,
            count,
            part_pct,
            rare_pct,
        });
    }
    // `insert` keeps the first group; the second is lost (and leaks).
    out.kill
        .entry(u32::from_ne_bytes(mob.to_ne_bytes()))
        .or_insert(bag);
    Ok(())
}

/// One `mob_drop_item.txt` group: dispatch on its `type` token.
fn read_mob_group(
    group: &text_file::TextGroup,
    protos: &ItemProtos,
    groups: &mut DropGroups,
    out: &mut MobDropGroups,
) -> Result<(), DropError> {
    // The `kr_` skip, tested on the child's own name (legacy tests the parent's, a
    // Defect not reproduced).
    if group.name.starts_with(b"kr_") {
        return Ok(());
    }
    let word = token_string(group, b"type").ok_or_else(|| DropError::MissingToken {
        file: "mob_drop_item.txt",
        group: group.name.clone(),
        token: "type",
    })?;
    let mob = token_i32(group, b"mob").ok_or_else(|| DropError::MissingToken {
        file: "mob_drop_item.txt",
        group: group.name.clone(),
        token: "mob",
    })?;
    if word == b"kill" {
        read_kill_group(group, mob, protos, out)?;
    } else if word == b"drop" {
        let entry = groups
            .groups
            .entry(u32::from_ne_bytes(mob.to_ne_bytes()))
            .or_insert_with(|| DropGroup {
                vnum: 0,
                items: Vec::new(),
            });
        for (key, (vnum, row)) in keyed_rows(group, protos, "mob_drop_item.txt")? {
            entry.items.push(count_pct_row(group, key, vnum, &row)?);
        }
    } else if word == b"limit" {
        let level_limit =
            token_i32(group, b"level_limit").ok_or_else(|| DropError::MissingToken {
                file: "mob_drop_item.txt",
                group: group.name.clone(),
                token: "level_limit",
            })?;
        // `kill_drop` is not read for `limit` (it defaults to 1), so the 0 skip
        // never fires here.
        let mut bag = LevelGroup {
            level_limit,
            items: Vec::new(),
        };
        for (key, (vnum, row)) in keyed_rows(group, protos, "mob_drop_item.txt")? {
            bag.items.push(count_pct_row(group, key, vnum, &row)?);
        }
        out.limit
            .entry(u32::from_ne_bytes(mob.to_ne_bytes()))
            .or_insert(bag);
    } else if word == b"thiefgloves" {
        let mut bag = DropGroup {
            vnum: 0,
            items: Vec::new(),
        };
        for (key, (vnum, row)) in keyed_rows(group, protos, "mob_drop_item.txt")? {
            bag.items.push(count_pct_row(group, key, vnum, &row)?);
        }
        out.gloves
            .entry(u32::from_ne_bytes(mob.to_ne_bytes()))
            .or_insert(bag);
    } else {
        return Err(DropError::BadType {
            group: group.name.clone(),
            word: word.to_vec(),
        });
    }
    Ok(())
}

impl DropTables {
    /// Read the four drop files from the Locale folder (`G/input_db.cpp:899-942`).
    ///
    /// A missing file stops the load, as legacy's boot shuts down; an empty one loads
    /// fine. `protos` is the already-read `item_proto.txt`.
    ///
    /// # Errors
    ///
    /// [`DropError::Unreadable`] for a missing file, and the row errors above for a file
    /// legacy refuses.
    pub fn load(locale_dir: &Path, protos: &ItemProtos) -> Result<Self, DropError> {
        let read_file = |name: &'static str| {
            std::fs::read(locale_dir.join(name)).map_err(|_| DropError::Unreadable { file: name })
        };
        let common = read_common(&read_file("common_drop_item.txt")?, protos)?;
        let etc = read_etc(&read_file("etc_drop_item.txt")?, protos)?;
        let mut groups = DropGroups::default();
        read_group_file(&read_file("drop_item_group.txt")?, protos, &mut groups)?;
        let mob = read_mob_file(&read_file("mob_drop_item.txt")?, protos, &mut groups)?;
        Ok(Self {
            common,
            etc,
            groups,
            mob,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn proto_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto")
    }

    fn locale_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/locale/europe")
    }

    fn protos() -> ItemProtos {
        ItemProtos::load(&proto_dir()).expect("the owner's item protos load")
    }
    /// The bit-decoded truncation matches the x86 conversion's edges: fractions drop,
    /// negatives and NaN keep 0, and anything past the 32-bit range saturates.
    #[test]
    fn the_float_truncation_keeps_the_x86_edges() {
        assert_eq!(float_trunc_to_u32(75.9), 75);
        assert_eq!(float_trunc_to_u32(1_000_000.0), 1_000_000);
        assert_eq!(float_trunc_to_u32(0.9), 0);
        assert_eq!(float_trunc_to_u32(-3.5), 0);
        assert_eq!(float_trunc_to_u32(f32::NAN), 0);
        assert_eq!(float_trunc_to_u32(f32::INFINITY), u32::MAX);
        assert_eq!(float_trunc_to_u32(5_000_000_000.0), u32::MAX);
        assert_eq!(float_trunc_to_u32(4_294_967_295.0), u32::MAX);
        assert_eq!(pct_of(75.0), 750_000);
        assert_eq!(pct_of(100.0), 1_000_000);
    }

    /// The owner's three drop files are empty and the fourth holds one `limit` group.
    #[test]
    fn the_owners_drop_files_load_with_one_limit_group() {
        let tables = DropTables::load(&locale_dir(), &protos()).expect("the files load");
        for rank in &tables.common.ranks {
            assert!(rank.is_empty());
        }
        assert!(tables.etc.probs.is_empty());
        assert!(tables.groups.groups.is_empty());
        assert!(tables.mob.kill.is_empty());
        assert!(tables.mob.gloves.is_empty());
        let group = tables.mob.limit.get(&2493).expect("mob 2493's group");
        assert_eq!(group.level_limit, 75);
        assert_eq!(group.items.len(), 21);
        // Key 12 written twice keeps the first row (12540); 12680 is lost.
        assert_eq!(group.items[11].vnum, 12540);
        assert_eq!(group.items[11].count, 1);
        assert_eq!(group.items[11].pct, 100_000);
        assert!(!group.items.iter().any(|row| row.vnum == 12680));
        // A pct of 1000 always drops against `number(1, 1000000)`.
        assert_eq!(group.items[0].pct, 10_000_000);
    }

    /// A missing drop file stops the load, as legacy's boot shuts down.
    #[test]
    fn a_missing_drop_file_is_unreadable() {
        let empty = Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        let error = DropTables::load(&empty, &protos()).expect_err("no drop files there");
        assert_eq!(
            error,
            DropError::Unreadable {
                file: "common_drop_item.txt"
            }
        );
    }

    /// A `kill` short row refuses the file with the row named (legacy throws).
    #[test]
    fn a_kill_short_row_refuses_with_the_row_named() {
        let data = b"Group\tshort\n{\n\tMob\t101\n\tType\tkill\n\tkill_drop\t1\n\t1\t19\t1\n}\n";
        let mut groups = DropGroups::default();
        let error = read_mob_file(data, &protos(), &mut groups).expect_err("short row");
        assert!(matches!(
            error,
            DropError::ShortRow {
                row: 1,
                fields: 2,
                ..
            }
        ));
    }

    /// A `drop` duplicate mob appends to the existing group (legacy's is UB).
    #[test]
    fn a_drop_duplicate_mob_appends() {
        let data = b"Group\ta\n{\n\tMob\t101\n\tType\tdrop\n\t1\t19\t1\t50\n}\nGroup\tb\n{\n\tMob\t101\n\tType\tdrop\n\t1\t20\t1\t25\n}\n";
        let mut groups = DropGroups::default();
        read_mob_file(data, &protos(), &mut groups).expect("duplicates append");
        let group = groups.groups.get(&101).expect("mob 101's group");
        assert_eq!(group.items.len(), 2);
        assert_eq!(group.items[1].vnum, 20);
    }

    /// A `kill` duplicate mob keeps the first group, as `insert` does.
    #[test]
    fn a_kill_duplicate_mob_keeps_the_first() {
        let data = b"Group\ta\n{\n\tMob\t101\n\tType\tkill\n\tkill_drop\t5\n\t1\t19\t1\t10\t0\n}\nGroup\tb\n{\n\tMob\t101\n\tType\tkill\n\tkill_drop\t9\n\t1\t20\t1\t10\t0\n}\n";
        let mut groups = DropGroups::default();
        let out = read_mob_file(data, &protos(), &mut groups).expect("duplicates keep first");
        let group = out.kill.get(&101).expect("mob 101's group");
        assert_eq!(group.kill_drop, 5);
        assert_eq!(group.items[0].vnum, 19);
    }

    /// An unknown type refuses the whole file.
    #[test]
    fn an_unknown_mob_type_refuses() {
        let data = b"Group\ta\n{\n\tMob\t101\n\tType\tsplat\n}\n";
        let mut groups = DropGroups::default();
        let error = read_mob_file(data, &protos(), &mut groups).expect_err("bad type");
        assert!(matches!(error, DropError::BadType { .. }));
    }

    /// An unknown item refuses the whole file.
    #[test]
    fn an_unknown_item_refuses() {
        let data =
            b"Group\ta\n{\n\tMob\t101\n\tType\tlimit\n\tlevel_limit\t75\n\t1\t99999999\t1\t10\n}\n";
        let mut groups = DropGroups::default();
        let error = read_mob_file(data, &protos(), &mut groups).expect_err("bad vnum");
        assert!(matches!(error, DropError::UnknownItem { .. }));
    }
}
