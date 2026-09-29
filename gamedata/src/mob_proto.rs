//! The mob proto, read as the legacy DB server reads it.
//!
//! `mob_proto.txt` holds one `TMobTable` per data row: 72 header columns, of which legacy reads
//! 71, over 1,725 data rows. `mob_names.txt` in the same folder supplies `szLocaleName`. Both are
//! read with the tab-separated CSV reader in [`crate::csv_table`].
//!
//! # The load path
//!
//! `CClientManager::InitializeMobTable` (`server/server/db/ClientManagerBoot.cpp:220-297`) reads
//! `mob_names.txt` into a `map<int, const char*>` with `localMap[atoi(column 0)] = column 1`, so
//! the **last** row for a vnum wins. It then fills one zeroed `TMobTable` per `mob_proto.txt` row
//! through `Set_Proto_Mob_Table` (`server/server/db/ProtoReader.cpp:755-854`) and sorts the
//! vector by `dwVnum` with `std::sort`. The game process then inserts every row into
//! `m_map_pkMobByVnum` (`server/server/game/mob_manager.cpp:57-110`), and `std::map::insert`
//! keeps the first row for a vnum.
//!
//! The owner's file lists 36 vnums twice, all of them monsters, and every pair differs. `std::sort`
//! is not stable, so which copy legacy keeps depends on its standard library. This module sorts by
//! `(vnum, line)` and [`MobProtos::get`] answers with the first row in file order, which is the
//! same resolution [`crate::item_proto`] makes for its duplicates; see [`MobProtos::duplicates`].
//!
//! # The columns
//!
//! Every numeric column goes through the `str_to_number` overload of its field's type
//! (`server/server/common/utils.h:4-106`). An empty field leaves the zero `memset` put there, and
//! any other field is `strtoul` for an unsigned field and `strtol` for a signed one, cut to the
//! field's width. So `17abc` is 17, `NONE` is 0, and `-1` in a `BYTE` is 255. `strtol` and
//! `strtoul` are 32 bits wide on the legacy i686 target, so a number too large saturates there
//! before it is cut.
//!
//! The `RANK`, `TYPE` and `BATTLE_TYPE` columns are looked up in their tables after a trim, and a
//! name that is not there is `-1` in a `BYTE`, which is 255. The owner's file has twelve rows of
//! `TYPE` `PET`, which no table holds, so those twelve rows are type 255; they are loaded, as
//! legacy loads them. `SIZE` answers 0 for a name it does not know.
//!
//! The three flag columns are split on `,` by `StringSplit` (`ProtoReader.cpp:26-53`), which drops
//! empty tokens and trims the rest. The flag resolvers (`:680-753`) then stop at the first token
//! that is empty after its trim, so `AGGR, ,NOMOVE` is `AGGR` alone.
//!
//! `DAM_MULTIPLY` is `strtof`. It reads the leading decimal number in the C locale, so the
//! `3,84` that 30 rows of the owner's file spell is 3.0; that is the value legacy used.
//!
//! Two columns are skipped, `MOUNT_CAPACITY` and `MOB_COLOR`, so their fields keep the `memset`
//! zero.
//!
//! # What this module refuses
//!
//! Legacy reads whatever `Set_Proto_Mob_Table` can reach. This module refuses what would have been
//! undefined there, and names the file line:
//!
//! - a row with fewer than 71 fields, where `AsStringByIndex` reads past the row;
//! - a flag column with more than 30 tokens, which overruns `StringSplit`'s `new string[30]`;
//! - a `DAM_MULTIPLY` that `strtof` would read as infinity, NaN or a hexadecimal float, which
//!   this reader does not support, or one past `f32`'s range. The owner's file has none.
//!
//! A flag name that appears twice in a column is summed twice by legacy, which carries it into the
//! next flag's bit. That is a Defect, and this module sets the bit once.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::csv_table::{self, CsvError};
use crate::item_proto_value::{trim, MAX_FLAG_TOKENS};
use crate::records::{
    MobSkillRecord, MobTableRecord, MOB_ENCHANTS_MAX_NUM, MOB_FOLDER_MAX_LEN, MOB_NAME_MAX_LEN,
    MOB_RESISTS_MAX_NUM, MOB_SKILL_MAX_NUM,
};

/// The columns legacy reads: `VNUM` at 0 to `SP_REVIVE` at 70.
pub const COLUMNS: usize = 71;

/// `CHAR_TYPE_MONSTER` (`server/server/common/length.h:470`).
pub const CHAR_TYPE_MONSTER: u8 = 0;
/// `CHAR_TYPE_NPC`.
pub const CHAR_TYPE_NPC: u8 = 1;
/// `CHAR_TYPE_STONE`.
pub const CHAR_TYPE_STONE: u8 = 2;
/// `CHAR_TYPE_WARP`.
pub const CHAR_TYPE_WARP: u8 = 3;
/// `CHAR_TYPE_PC`, the type a player's character carries.
pub const CHAR_TYPE_PC: u8 = 6;
/// `CHAR_TYPE_GOTO`.
pub const CHAR_TYPE_GOTO: u8 = 9;

/// `get_Mob_Rank_Value` (`ProtoReader.cpp:589-610`).
pub const RANK: &[&str] = &["PAWN", "S_PAWN", "KNIGHT", "S_KNIGHT", "BOSS", "KING"];

/// `get_Mob_Type_Value` (`ProtoReader.cpp:613-633`), indexed by `ECharType`.
pub const TYPE: &[&str] = &[
    "MONSTER",
    "NPC",
    "STONE",
    "WARP",
    "DOOR",
    "BUILDING",
    "PC",
    "POLYMORPH_PC",
    "HORSE",
    "GOTO",
];

/// `get_Mob_BattleType_Value` (`ProtoReader.cpp:635-654`).
pub const BATTLE_TYPE: &[&str] = &[
    "MELEE",
    "RANGE",
    "MAGIC",
    "SPECIAL",
    "POWER",
    "TANKER",
    "SUPER_POWER",
    "SUPER_TANKER",
];

/// `get_Mob_Size_Value` (`ProtoReader.cpp:656-676`). The value is the index plus one.
pub const SIZE: &[&str] = &["SMALL", "MEDIUM", "BIG"];

/// `get_Mob_AIFlag_Value` (`ProtoReader.cpp:678-702`), bit by bit.
pub const AI_FLAG: &[&str] = &[
    "AGGR",
    "NOMOVE",
    "COWARD",
    "NOATTSHINSU",
    "NOATTCHUNJO",
    "NOATTJINNO",
    "ATTMOB",
    "BERSERK",
    "STONESKIN",
    "GODSPEED",
    "DEATHBLOW",
    "REVIVE",
    "UNK13",
    "UNK14",
];

/// `get_Mob_RaceFlag_Value` (`ProtoReader.cpp:703-727`), bit by bit.
pub const RACE_FLAG: &[&str] = &[
    "ANIMAL",
    "UNDEAD",
    "DEVIL",
    "HUMAN",
    "ORC",
    "MILGYO",
    "INSECT",
    "FIRE",
    "ICE",
    "DESERT",
    "TREE",
    "ATT_ELEC",
    "ATT_FIRE",
    "ATT_ICE",
    "ATT_WIND",
    "ATT_EARTH",
    "ATT_DARK",
    "METIN",
    "BOSS",
];

/// `get_Mob_ImmuneFlag_Value` (`ProtoReader.cpp:728-753`), bit by bit.
pub const IMMUNE_FLAG: &[&str] = &[
    "STUN", "SLOW", "FALL", "CURSE", "POISON", "TERROR", "REFLECT",
];

/// One row of `mob_proto.txt` and the file line it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct MobProto {
    /// The 1-based line of `mob_proto.txt`, header included: the tiebreaker for a repeated vnum.
    pub line: usize,
    /// The `TMobTable` `Set_Proto_Mob_Table` filled.
    pub table: MobTableRecord,
}

/// The mob protos, in the order the game process sees them: sorted by vnum.
#[derive(Debug, Clone, Default)]
pub struct MobProtos {
    by_vnum: Vec<MobProto>,
}

impl MobProtos {
    /// Read `mob_proto.txt` and `mob_names.txt` from the proto folder.
    ///
    /// # Errors
    ///
    /// Returns [`MobProtoError::Io`] when a file is missing or unreadable, and any other
    /// [`MobProtoError`] for a row this reader refuses.
    pub fn load(proto_dir: &Path) -> Result<Self, MobProtoError> {
        let read = |name: &str| {
            let path = proto_dir.join(name);
            std::fs::read(&path).map_err(|source| MobProtoError::Io {
                path: path.clone(),
                message: source.to_string(),
            })
        };
        let proto = read(PROTO_FILE)?;
        let locale_names = read(NAMES_FILE)?;
        parse(&proto, &locale_names)
    }

    /// A table of the given rows, sorted by `(vnum, line)` the way [`parse`] sorts the file.
    #[must_use]
    pub fn from_rows(mut by_vnum: Vec<MobProto>) -> Self {
        by_vnum.sort_by_key(|p| (p.table.vnum, p.line));
        Self { by_vnum }
    }

    /// The number of rows, which is the file's data-row count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_vnum.len()
    }

    /// Whether the file held no data rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_vnum.is_empty()
    }

    /// Every row, sorted by vnum.
    #[must_use]
    pub fn rows(&self) -> &[MobProto] {
        &self.by_vnum
    }

    /// `CMobManager::Get` (`mob_manager.cpp:112-120`): the first row for `vnum` in sorted order.
    #[must_use]
    pub fn get(&self, vnum: u32) -> Option<&MobTableRecord> {
        let index = self.by_vnum.partition_point(|p| p.table.vnum < vnum);
        self.by_vnum
            .get(index)
            .filter(|p| p.table.vnum == vnum)
            .map(|p| &p.table)
    }

    /// The vnums the file lists more than once, each with the line of the copy [`Self::get`]
    /// answers with.
    #[must_use]
    pub fn duplicates(&self) -> Vec<(u32, usize)> {
        let mut out: Vec<(u32, usize)> = self
            .by_vnum
            .windows(2)
            .filter(|w| w[0].table.vnum == w[1].table.vnum)
            .map(|w| (w[0].table.vnum, w[0].line))
            .collect();
        out.dedup_by_key(|(vnum, _)| *vnum);
        out
    }
}

/// The proto file's name in the proto folder.
const PROTO_FILE: &str = "mob_proto.txt";

/// The locale-name file's name in the proto folder.
const NAMES_FILE: &str = "mob_names.txt";

/// A mob proto this reader refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobProtoError {
    /// A file could not be read.
    Io {
        /// The file.
        path: PathBuf,
        /// The operating system's message.
        message: String,
    },
    /// A file did not parse as the tab-separated CSV the legacy reader accepts.
    Csv {
        /// The file's name.
        file: &'static str,
        /// The cause.
        source: CsvError,
    },
    /// A row with too few fields for the columns legacy reads from it.
    ShortRow {
        /// The file's name.
        file: &'static str,
        /// The 1-based line, the header included.
        line: usize,
        /// The fields the row has.
        found: usize,
    },
    /// A flag column with more tokens than `StringSplit`'s buffer holds.
    TooManyFlagTokens {
        /// The 1-based line.
        line: usize,
        /// The row's vnum.
        vnum: u32,
        /// The column's name.
        column: &'static str,
        /// The tokens the column has.
        found: usize,
    },
    /// A `DAM_MULTIPLY` that `strtof` would read as infinity, NaN, or a hexadecimal float.
    UnsupportedFloat {
        /// The 1-based line.
        line: usize,
        /// The row's vnum.
        vnum: u32,
        /// The field as the file spells it.
        field: Vec<u8>,
    },
}

impl fmt::Display for MobProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "{}: {message}", path.display()),
            Self::Csv { file, source } => write!(f, "{file}: {source}"),
            Self::ShortRow { file, line, found } => write!(
                f,
                "{file} line {line} has {found} fields; legacy reads more and would read past it"
            ),
            Self::TooManyFlagTokens {
                line,
                vnum,
                column,
                found,
            } => write!(
                f,
                "{PROTO_FILE} line {line} (vnum {vnum}): {column} has {found} tokens; \
                 legacy's StringSplit holds {MAX_FLAG_TOKENS}"
            ),
            Self::UnsupportedFloat { line, vnum, field } => write!(
                f,
                "{PROTO_FILE} line {line} (vnum {vnum}): DAM_MULTIPLY {} is not a decimal number",
                String::from_utf8_lossy(field)
            ),
        }
    }
}

impl Error for MobProtoError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Csv { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Read `mob_proto.txt` and `mob_names.txt` the way the legacy DB server reads them.
///
/// # Errors
///
/// Returns the first row this reader refuses, or a CSV error.
pub fn parse(proto: &[u8], locale_names: &[u8]) -> Result<MobProtos, MobProtoError> {
    let names = read_locale_names(locale_names)?;
    let rows =
        csv_table::parse_numbered(proto, b'\t', b'"').map_err(|source| MobProtoError::Csv {
            file: PROTO_FILE,
            source,
        })?;
    let mut by_vnum = Vec::with_capacity(rows.len().saturating_sub(1));
    // Row 0 is the header, which `ClientManagerBoot.cpp:260` skips with a `Next()`.
    for numbered in rows.iter().skip(1) {
        by_vnum.push(MobProto {
            line: numbered.line,
            table: read_row(numbered.line, &numbered.fields, &names)?,
        });
    }
    Ok(MobProtos::from_rows(by_vnum))
}

/// `mob_names.txt`: `localMap[atoi(column 0)] = column 1`, the last row for a vnum winning.
///
/// Legacy reads column 1 of every data row, which a one-column row does not have.
fn read_locale_names(bytes: &[u8]) -> Result<HashMap<i32, Vec<u8>>, MobProtoError> {
    let rows =
        csv_table::parse_numbered(bytes, b'\t', b'"').map_err(|source| MobProtoError::Csv {
            file: NAMES_FILE,
            source,
        })?;
    let mut names = HashMap::new();
    for numbered in rows.iter().skip(1) {
        let [vnum, name, ..] = numbered.fields.as_slice() else {
            return Err(MobProtoError::ShortRow {
                file: NAMES_FILE,
                line: numbered.line,
                found: numbered.fields.len(),
            });
        };
        names.insert(strtol(vnum), name.clone());
    }
    Ok(names)
}

/// One data row, as `Set_Proto_Mob_Table` fills a zeroed `TMobTable`.
fn read_row(
    line: usize,
    row: &[Vec<u8>],
    locale_names: &HashMap<i32, Vec<u8>>,
) -> Result<MobTableRecord, MobProtoError> {
    if row.len() < COLUMNS {
        return Err(MobProtoError::ShortRow {
            file: PROTO_FILE,
            line,
            found: row.len(),
        });
    }
    let vnum = strtoul(&row[0]);
    let flag = |column: &'static str, index: usize, table: &[&str]| {
        flag_mask(&row[index], table).map_err(|found| MobProtoError::TooManyFlagTokens {
            line,
            vnum,
            column,
            found,
        })
    };
    let mut table = MobTableRecord {
        vnum,
        mob_type: enum_byte(TYPE, &row[3]),
        rank: enum_byte(RANK, &row[2]),
        battle_type: enum_byte(BATTLE_TYPE, &row[4]),
        level: low_byte(&row[5]),
        size: size_byte(&row[6]),
        ai_flag: flag("AI_FLAG", 7, AI_FLAG)?,
        race_flag: flag("RACE_FLAG", 9, RACE_FLAG)?,
        immune_flag: flag("IMMUNE_FLAG", 10, IMMUNE_FLAG)?,
        empire: low_byte(&row[11]),
        on_click_type: low_byte(&row[13]),
        str: low_byte(&row[14]),
        dex: low_byte(&row[15]),
        con: low_byte(&row[16]),
        int_: low_byte(&row[17]),
        damage_range: [strtoul(&row[18]), strtoul(&row[19])],
        max_hp: strtoul(&row[20]),
        regen_cycle: low_byte(&row[21]),
        regen_percent: low_byte(&row[22]),
        gold_min: strtoul(&row[23]),
        gold_max: strtoul(&row[24]),
        exp: strtoul(&row[25]),
        def: low_word(&row[26]),
        attack_speed: signed_word(&row[27]),
        moving_speed: signed_word(&row[28]),
        aggressive_hp_pct: low_byte(&row[29]),
        aggressive_sight: low_word(&row[30]),
        attack_range: low_word(&row[31]),
        drop_item_vnum: strtoul(&row[32]),
        resurrection_vnum: strtoul(&row[33]),
        dam_multiply: strtof(&row[51]).ok_or_else(|| MobProtoError::UnsupportedFloat {
            line,
            vnum,
            field: row[51].clone(),
        })?,
        summon_vnum: strtoul(&row[52]),
        drain_sp: strtoul(&row[53]),
        polymorph_item_vnum: strtoul(&row[55]),
        berserk_point: low_byte(&row[66]),
        stone_skin_point: low_byte(&row[67]),
        god_speed_point: low_byte(&row[68]),
        death_blow_point: low_byte(&row[69]),
        revive_point: low_byte(&row[70]),
        ..MobTableRecord::default()
    };
    strlcpy(&mut table.name, &row[1], MOB_NAME_MAX_LEN);
    // `nameMap.find(dwVnum)` converts the `DWORD` to the map's `int` key, so the lookup is by the
    // vnum's 32-bit pattern.
    let locale = locale_names
        .get(&i32::from_le_bytes(vnum.to_le_bytes()))
        .unwrap_or(&row[1]);
    strlcpy(&mut table.locale_name, locale, MOB_NAME_MAX_LEN);
    strlcpy(&mut table.folder, &row[12], MOB_FOLDER_MAX_LEN);
    for (slot, enchant) in table.enchants.iter_mut().enumerate() {
        *enchant = signed_byte(&row[34 + slot]);
    }
    for (slot, resist) in table.resists.iter_mut().enumerate() {
        *resist = signed_byte(&row[34 + MOB_ENCHANTS_MAX_NUM + slot]);
    }
    for (slot, skill) in table.skills.iter_mut().enumerate() {
        *skill = MobSkillRecord {
            level: low_byte(&row[56 + slot * 2]),
            vnum: strtoul(&row[57 + slot * 2]),
        };
    }
    Ok(table)
}

const _: () = assert!(34 + MOB_ENCHANTS_MAX_NUM + MOB_RESISTS_MAX_NUM == 51);
const _: () = assert!(56 + MOB_SKILL_MAX_NUM * 2 == 66);

/// `strlcpy(dst, src, max + 1)`: at most `max` bytes, the rest of `dst` staying zero.
fn strlcpy(dst: &mut [u8], src: &[u8], max: usize) {
    let end = src.len().min(max);
    dst[..end].copy_from_slice(&src[..end]);
}

/// An enum column: the index of the trimmed field in `table`, or `-1` as a `BYTE`.
fn enum_byte(table: &[&str], field: &[u8]) -> u8 {
    let field = trim(field);
    table
        .iter()
        .position(|entry| entry.as_bytes() == field)
        .and_then(|index| u8::try_from(index).ok())
        .unwrap_or(u8::MAX)
}

/// `get_Mob_Size_Value`: the index plus one, or 0 for a name it does not know.
fn size_byte(field: &[u8]) -> u8 {
    let field = trim(field);
    SIZE.iter()
        .position(|entry| entry.as_bytes() == field)
        .and_then(|index| u8::try_from(index + 1).ok())
        .unwrap_or(0)
}

/// A flag column: `StringSplit(field, ",")`, then the resolver's scan.
///
/// The resolver walks the 30 slots and stops at the first empty one, which is either past the last
/// token or a token that was only whitespace before its trim.
///
/// Answers the token count when it overruns the 30-slot buffer.
fn flag_mask(field: &[u8], table: &[&str]) -> Result<u32, usize> {
    let tokens: Vec<&[u8]> = field
        .split(|b| *b == b',')
        .filter(|token| !token.is_empty())
        .map(trim)
        .collect();
    if tokens.len() > MAX_FLAG_TOKENS {
        return Err(tokens.len());
    }
    let read: Vec<&[u8]> = tokens
        .into_iter()
        .take_while(|token| !token.is_empty())
        .collect();
    let mut mask = 0;
    for (bit, entry) in table.iter().enumerate() {
        if read.iter().any(|token| *token == entry.as_bytes()) {
            mask |= 1 << bit;
        }
    }
    Ok(mask)
}

/// Whether `strtol` skips `byte` before a number: C `isspace` in the C locale.
fn c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// The sign and the digits `strtol` and `strtoul` read: C whitespace, an optional sign, and the
/// decimal digits after it.
fn sign_and_digits(field: &[u8]) -> (bool, &[u8]) {
    let start = field
        .iter()
        .position(|b| !c_space(*b))
        .unwrap_or(field.len());
    let field = &field[start..];
    let (negative, rest) = match field.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, field),
    };
    let end = rest
        .iter()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(rest.len());
    (negative, &rest[..end])
}

/// The magnitude of `digits`, stopping once it is past every 32-bit answer.
fn magnitude(digits: &[u8]) -> u64 {
    let ceiling = u64::from(u32::MAX) + 1;
    digits.iter().fold(0, |value, digit| {
        (value * 10 + u64::from(digit - b'0')).min(ceiling)
    })
}

/// `strtol(field, NULL, 10)` with a 32-bit `long`: saturating at both ends.
pub(crate) fn strtol(field: &[u8]) -> i32 {
    let (negative, digits) = sign_and_digits(field);
    let value = i64::try_from(magnitude(digits)).unwrap_or(i64::MAX);
    let value = if negative { -value } else { value };
    i32::try_from(value).unwrap_or(if negative { i32::MIN } else { i32::MAX })
}

/// `strtoul(field, NULL, 10)` with a 32-bit `unsigned long`.
///
/// A magnitude past the type is `ULONG_MAX`, and a negative one is negated in the type, so `-1` is
/// `0xFFFF_FFFF`.
fn strtoul(field: &[u8]) -> u32 {
    let (negative, digits) = sign_and_digits(field);
    match u32::try_from(magnitude(digits)) {
        Ok(value) if negative => value.wrapping_neg(),
        Ok(value) => value,
        Err(_) => u32::MAX,
    }
}

/// `(unsigned char) strtoul(field)`: the low byte.
pub(crate) fn low_byte(field: &[u8]) -> u8 {
    strtoul(field).to_le_bytes()[0]
}

/// `(char) strtol(field)`: the low byte, signed.
fn signed_byte(field: &[u8]) -> i8 {
    i8::from_le_bytes([strtol(field).to_le_bytes()[0]])
}

/// `(unsigned short) strtoul(field)`: the low 16 bits.
fn low_word(field: &[u8]) -> u16 {
    let [a, b, _, _] = strtoul(field).to_le_bytes();
    u16::from_le_bytes([a, b])
}

/// `(short) strtol(field)`: the low 16 bits, signed.
fn signed_word(field: &[u8]) -> i16 {
    let [a, b, _, _] = strtol(field).to_le_bytes();
    i16::from_le_bytes([a, b])
}

/// `strtof(field, NULL)` in the C locale, or `None` for the forms this reader does not support.
///
/// The leading decimal number is read: digits with an optional `.` and fraction, then an optional
/// exponent that is only taken when a digit follows it. Anything after it is ignored, and a field
/// with no digit there is 0. `inf`, `nan`, `0x` and a number past `f32`'s range, which `strtof`
/// answers with infinity, are refused rather than read.
fn strtof(field: &[u8]) -> Option<f32> {
    let start = field
        .iter()
        .position(|b| !c_space(*b))
        .unwrap_or(field.len());
    let text = &field[start..];
    let sign = usize::from(matches!(text.first(), Some(b'-' | b'+')));
    let body = &text[sign..];
    let lower: Vec<u8> = body.iter().take(3).map(u8::to_ascii_lowercase).collect();
    if lower.starts_with(b"inf") || lower.starts_with(b"nan") || lower.starts_with(b"0x") {
        return None;
    }
    let digits = |from: usize| {
        body[from..]
            .iter()
            .position(|b| !b.is_ascii_digit())
            .map_or(body.len(), |n| from + n)
    };
    let mut end = digits(0);
    let whole = end;
    if body.get(end) == Some(&b'.') {
        end = digits(end + 1);
    }
    if whole == 0 && end <= 1 {
        return Some(0.0);
    }
    if matches!(body.get(end), Some(b'e' | b'E')) {
        let exponent_sign = usize::from(matches!(body.get(end + 1), Some(b'-' | b'+')));
        let first = end + 1 + exponent_sign;
        let last = digits(first);
        if last > first {
            end = last;
        }
    }
    let number = std::str::from_utf8(&text[..sign + end]).ok()?;
    number.parse().ok().filter(|value: &f32| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's proto folder.
    fn owners_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto")
    }

    fn owners() -> MobProtos {
        MobProtos::load(&owners_dir()).expect("the owner's mob proto loads")
    }

    /// A 71-field row whose values are all zero, so a test can change one column.
    fn blank_row(vnum: &str) -> Vec<Vec<u8>> {
        let mut row: Vec<Vec<u8>> = (0..COLUMNS).map(|_| b"0".to_vec()).collect();
        row[0] = vnum.as_bytes().to_vec();
        row[1] = b"Mob".to_vec();
        row[2] = b"PAWN".to_vec();
        row[3] = b"MONSTER".to_vec();
        row[4] = b"MELEE".to_vec();
        row
    }

    fn proto_of(rows: &[Vec<Vec<u8>>]) -> Vec<u8> {
        let mut out = b"VNUM\tNAME\tRANK\n".to_vec();
        for row in rows {
            out.extend(row.join(&b'\t'));
            out.push(b'\n');
        }
        out
    }

    fn names_of(rows: &[(&str, &str)]) -> Vec<u8> {
        let mut out = b"VNUM\tLOCALE_NAME\n".to_vec();
        for (vnum, name) in rows {
            out.extend(format!("{vnum}\t{name}\n").bytes());
        }
        out
    }

    fn one(row: Vec<Vec<u8>>) -> MobTableRecord {
        let protos = parse(&proto_of(&[row]), &names_of(&[])).expect("the row loads");
        protos.rows()[0].table
    }

    /// The whole file loads: 1,725 data rows, sorted, with nothing refused.
    #[test]
    fn the_owners_file_loads() {
        let protos = owners();
        assert_eq!(protos.len(), 1_725);
        assert!(!protos.is_empty());
        assert!(protos
            .rows()
            .windows(2)
            .all(|w| (w[0].table.vnum, w[0].line) < (w[1].table.vnum, w[1].line)));
    }

    /// An NPC the Channel spawns on map 1, column by column against the file's line.
    #[test]
    fn an_owners_npc_row_fills_every_column_it_names() {
        let protos = owners();
        let npc = protos.get(20_016).expect("20016 is in the file");
        assert_eq!(npc.mob_type, CHAR_TYPE_NPC);
        assert_eq!(npc.vnum, 20_016);
        assert_eq!(npc.name[..5], *b"????\0", "the file's own bytes");
        assert_eq!(npc.locale_name[..7], *b"Fierar\0");
        assert_eq!(npc.rank, 5);
        assert_eq!(npc.battle_type, 0);
        assert_eq!(npc.level, 1);
        assert_eq!(npc.ai_flag, 1 << 1);
        assert_eq!(npc.immune_flag, 0b10_1011);
        assert_eq!(npc.empire, 0);
        assert_eq!(npc.on_click_type, 2);
        assert_eq!(npc.max_hp, 120);
        assert_eq!([npc.regen_cycle, npc.regen_percent], [3, 1]);
        assert_eq!([npc.exp, u32::from(npc.def)], [10, 4]);
        assert_eq!([npc.attack_speed, npc.moving_speed], [100, 100]);
        assert_eq!([npc.aggressive_sight, npc.attack_range], [2_000, 175]);
    }

    /// The twelve `PET` rows are type 255, as `-1` in a `BYTE`, and still load.
    #[test]
    fn an_unknown_type_is_255_and_the_row_is_kept() {
        let protos = owners();
        let pets = protos
            .rows()
            .iter()
            .filter(|p| p.table.mob_type == u8::MAX)
            .count();
        assert_eq!(pets, 12);
        let mut row = blank_row("7");
        row[2] = b"NOT_A_RANK".to_vec();
        row[3] = b"PET".to_vec();
        row[4] = b" RANGE ".to_vec();
        let table = one(row);
        assert_eq!(table.rank, u8::MAX);
        assert_eq!(table.mob_type, u8::MAX);
        assert_eq!(
            table.battle_type, 1,
            "the field is trimmed before the lookup"
        );
    }

    /// A known type resolves to its `ECharType` index; the control for the test above.
    #[test]
    fn each_type_name_resolves_to_its_index() {
        for (index, name) in TYPE.iter().enumerate() {
            let mut row = blank_row("7");
            row[3] = name.as_bytes().to_vec();
            assert_eq!(usize::from(one(row).mob_type), index, "{name}");
        }
        assert_eq!(TYPE[usize::from(CHAR_TYPE_NPC)], "NPC");
        assert_eq!(TYPE[usize::from(CHAR_TYPE_WARP)], "WARP");
        assert_eq!(TYPE[usize::from(CHAR_TYPE_GOTO)], "GOTO");
        assert_eq!(TYPE[usize::from(CHAR_TYPE_PC)], "PC");
        assert_eq!(TYPE[usize::from(CHAR_TYPE_STONE)], "STONE");
        assert_eq!(TYPE[usize::from(CHAR_TYPE_MONSTER)], "MONSTER");
    }

    /// `SIZE` is the index plus one, and 0 for anything else, including `0` and `100`.
    #[test]
    fn size_is_the_index_plus_one_or_zero() {
        let sizes = [
            ("SMALL", 1),
            ("MEDIUM", 2),
            ("BIG", 3),
            (" BIG ", 3),
            ("100", 0),
            ("", 0),
        ];
        for (field, size) in sizes {
            let mut row = blank_row("7");
            row[6] = field.as_bytes().to_vec();
            assert_eq!(one(row).size, size, "{field}");
        }
    }

    /// The flag columns: comma-split, trimmed, unknown names ignored, and the scan stopping at a
    /// token that was only whitespace.
    #[test]
    fn flags_are_split_on_commas_and_stop_at_a_blank_token() {
        let mut row = blank_row("7");
        row[7] = b"AGGR, NOMOVE ,NORECOVERY,REVIVE".to_vec();
        row[9] = b"ANIMAL,,BOSS".to_vec();
        row[10] = b"STUN, ,REFLECT".to_vec();
        let table = one(row);
        assert_eq!(table.ai_flag, 0b1000_0000_0011);
        assert_eq!(
            table.race_flag, 0b100_0000_0000_0000_0001,
            "an empty token is dropped"
        );
        assert_eq!(table.immune_flag, 1, "a blank token ends the scan");
    }

    /// A name twice sets its bit once; legacy's sum would carry it into the next flag.
    #[test]
    fn a_repeated_flag_name_sets_its_bit_once() {
        let mut row = blank_row("7");
        row[7] = b"AGGR,AGGR".to_vec();
        assert_eq!(one(row).ai_flag, 1);
    }

    /// Thirty tokens load; thirty-one overrun `StringSplit` and are refused.
    #[test]
    fn a_flag_column_past_thirty_tokens_is_refused() {
        let mut row = blank_row("7");
        row[10] = vec![&b"STUN"[..]; MAX_FLAG_TOKENS].join(&b',');
        assert_eq!(one(row).immune_flag, 1);
        let mut row = blank_row("7");
        row[10] = vec![&b"STUN"[..]; MAX_FLAG_TOKENS + 1].join(&b',');
        assert_eq!(
            parse(&proto_of(&[row]), &names_of(&[])).unwrap_err(),
            MobProtoError::TooManyFlagTokens {
                line: 2,
                vnum: 7,
                column: "IMMUNE_FLAG",
                found: MAX_FLAG_TOKENS + 1,
            }
        );
    }

    /// Every numeric column lands in its own field, read with its own width and sign.
    #[test]
    fn each_numeric_column_reaches_its_field() {
        let mut row = blank_row("4294967295");
        for (index, field) in row.iter_mut().enumerate().skip(5) {
            if ![6, 7, 9, 10, 12].contains(&index) {
                *field = (index + 1).to_string().into_bytes();
            }
        }
        row[12] = b"folder".to_vec();
        let t = one(row);
        assert_eq!(t.vnum, u32::MAX);
        assert_eq!(t.level, 6);
        assert_eq!(t.mount_capacity, 0, "column 8 is skipped");
        assert_eq!(t.empire, 12);
        assert_eq!(&t.folder[..7], b"folder\0");
        assert_eq!(t.on_click_type, 14);
        assert_eq!([t.str, t.dex, t.con, t.int_], [15, 16, 17, 18]);
        assert_eq!(t.damage_range, [19, 20]);
        assert_eq!(t.max_hp, 21);
        assert_eq!([t.regen_cycle, t.regen_percent], [22, 23]);
        assert_eq!([t.gold_min, t.gold_max, t.exp], [24, 25, 26]);
        assert_eq!(t.def, 27);
        assert_eq!([t.attack_speed, t.moving_speed], [28, 29]);
        assert_eq!(t.aggressive_hp_pct, 30);
        assert_eq!([t.aggressive_sight, t.attack_range], [31, 32]);
        assert_eq!([t.drop_item_vnum, t.resurrection_vnum], [33, 34]);
        assert_eq!(t.enchants, [35, 36, 37, 38, 39, 40]);
        assert_eq!(t.resists, [41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51]);
        assert_eq!(t.dam_multiply.to_bits(), 52f32.to_bits());
        assert_eq!([t.summon_vnum, t.drain_sp], [53, 54]);
        assert_eq!(t.mob_color, 0, "column 54 is skipped");
        assert_eq!(t.polymorph_item_vnum, 56);
        let skills: Vec<(u8, u32)> = t.skills.iter().map(|s| (s.level, s.vnum)).collect();
        assert_eq!(skills, [(57, 58), (59, 60), (61, 62), (63, 64), (65, 66)]);
        assert_eq!(
            [
                t.berserk_point,
                t.stone_skin_point,
                t.god_speed_point,
                t.death_blow_point,
                t.revive_point
            ],
            [67, 68, 69, 70, 71]
        );
    }

    /// Every numeric column is cut to its field's width and read with its field's sign.
    #[test]
    fn each_numeric_column_is_read_at_its_width() {
        let filled = |value: &[u8]| {
            let mut row = blank_row("5");
            for (index, field) in row.iter_mut().enumerate().skip(5) {
                if ![6, 7, 9, 10, 12].contains(&index) {
                    *field = value.to_vec();
                }
            }
            one(row)
        };
        let bytes = |t: &MobTableRecord| {
            [
                t.level,
                t.empire,
                t.on_click_type,
                t.str,
                t.dex,
                t.con,
                t.int_,
                t.regen_cycle,
                t.regen_percent,
                t.aggressive_hp_pct,
                t.berserk_point,
                t.stone_skin_point,
                t.god_speed_point,
                t.death_blow_point,
                t.revive_point,
            ]
        };
        let dwords = |t: &MobTableRecord| {
            [
                t.damage_range[0],
                t.damage_range[1],
                t.max_hp,
                t.gold_min,
                t.gold_max,
                t.exp,
                t.drop_item_vnum,
                t.resurrection_vnum,
                t.summon_vnum,
                t.drain_sp,
                t.polymorph_item_vnum,
            ]
        };
        let skills = |t: &MobTableRecord| -> Vec<(u8, u32)> {
            t.skills.iter().map(|s| (s.level, s.vnum)).collect()
        };
        let t = filled(b"65793");
        assert_eq!(bytes(&t), [1; 15]);
        assert_eq!([t.def, t.aggressive_sight, t.attack_range], [257; 3]);
        assert_eq!([t.attack_speed, t.moving_speed], [257; 2]);
        assert_eq!(dwords(&t), [65_793; 11]);
        assert_eq!((t.enchants, t.resists), ([1; 6], [1; 11]));
        assert_eq!(skills(&t), [(1, 65_793); 5]);
        assert_eq!(t.dam_multiply.to_bits(), 65_793f32.to_bits());
        let t = filled(b"-2");
        assert_eq!(bytes(&t), [254; 15]);
        assert_eq!([t.def, t.aggressive_sight, t.attack_range], [65_534; 3]);
        assert_eq!([t.attack_speed, t.moving_speed], [-2; 2]);
        assert_eq!(dwords(&t), [4_294_967_294; 11]);
        assert_eq!((t.enchants, t.resists), ([-2; 6], [-2; 11]));
        assert_eq!(skills(&t), [(254, 4_294_967_294); 5]);
        assert_eq!(t.dam_multiply.to_bits(), (-2f32).to_bits());
    }

    /// The conversions: `strtoul` then a cut for the unsigned fields, `strtol` then a cut for the
    /// signed ones, both 32 bits wide and saturating.
    #[test]
    fn numbers_convert_the_way_str_to_number_converts_them() {
        assert_eq!(strtoul(b"-1"), u32::MAX);
        assert_eq!(strtoul(b"99999999999"), u32::MAX);
        assert_eq!(strtoul(b"-99999999999"), u32::MAX);
        assert_eq!(strtoul(b" +17abc"), 17);
        assert_eq!(strtoul(b"NONE"), 0);
        assert_eq!(strtoul(b""), 0);
        assert_eq!(strtol(b"-99999999999"), i32::MIN);
        assert_eq!(strtol(b"99999999999"), i32::MAX);
        assert_eq!(strtol(b"\x0c-5"), -5);
        for space in [b' ', b'\t', b'\n', 0x0b, 0x0c, b'\r'] {
            assert_eq!(strtol(&[space, b'7']), 7, "{space:#04x} is C whitespace");
            assert_eq!(strtoul(&[space, b'7']), 7, "{space:#04x} is C whitespace");
        }
        assert_eq!(strtol(b"\x087"), 0, "a backspace is not");
        assert_eq!(strtol(b"\x0e7"), 0, "nor is 0x0e");
        assert_eq!(low_byte(b"257"), 1);
        assert_eq!(low_byte(b"-1"), 255);
        assert_eq!(signed_byte(b"200"), -56);
        assert_eq!(signed_byte(b"-25"), -25);
        assert_eq!(low_word(b"65537"), 1);
        assert_eq!(signed_word(b"40000"), -25_536);
        assert_eq!(signed_word(b"-5"), -5);
    }

    /// `strtof` reads the leading decimal number in the C locale, so a decimal comma ends it.
    #[test]
    fn dam_multiply_reads_the_leading_decimal_number() {
        for (field, value) in [
            ("3.5", 3.5f32),
            ("3,84", 3.0),
            ("", 0.0),
            ("x", 0.0),
            (".", 0.0),
            (".5", 0.5),
            ("2.", 2.0),
            ("-1.25", -1.25),
            ("1e2", 100.0),
            ("1E+2x", 100.0),
            ("1e", 1.0),
            ("1e+", 1.0),
            (" 7", 7.0),
            ("0.1", 0.1),
        ] {
            assert_eq!(
                strtof(field.as_bytes()).map(f32::to_bits),
                Some(value.to_bits()),
                "{field}"
            );
        }
        for field in ["inf", "-INFINITY", "nan", "0x1p3", "+NaN", "1e39", "-4e38"] {
            assert_eq!(strtof(field.as_bytes()), None, "{field}");
        }
        let owners = owners();
        for (vnum, value) in [(6_311, 3.0f32), (6_400, 4.0)] {
            let table = owners.get(vnum).expect("the row is in the file");
            assert_eq!(table.dam_multiply.to_bits(), value.to_bits(), "{vnum}");
        }
    }

    /// An unsupported float is refused with its line and field.
    #[test]
    fn an_infinite_dam_multiply_is_refused() {
        let mut row = blank_row("9");
        row[51] = b"inf".to_vec();
        let error = parse(&proto_of(&[row]), &names_of(&[])).unwrap_err();
        assert_eq!(
            error,
            MobProtoError::UnsupportedFloat {
                line: 2,
                vnum: 9,
                field: b"inf".to_vec(),
            }
        );
        assert!(error.to_string().contains("DAM_MULTIPLY inf"), "{error}");
    }

    /// A row one field short is refused with its line; a row of exactly 71 loads.
    #[test]
    fn a_row_short_of_seventy_one_fields_is_refused() {
        let mut row = blank_row("9");
        row.pop();
        assert_eq!(
            parse(&proto_of(&[row]), &names_of(&[])).unwrap_err(),
            MobProtoError::ShortRow {
                file: "mob_proto.txt",
                line: 2,
                found: COLUMNS - 1,
            }
        );
        assert_eq!(one(blank_row("9")).vnum, 9);
    }

    /// The locale name: the last `mob_names.txt` row for the vnum, keyed by `atoi`, or `NAME`.
    #[test]
    fn the_locale_name_is_the_last_names_row_or_the_own_name() {
        let rows = [blank_row("5"), blank_row("6")];
        let names = names_of(&[("5x", "First"), ("5", "Second"), ("7", "Other")]);
        let protos = parse(&proto_of(&rows), &names).unwrap();
        assert_eq!(&protos.get(5).unwrap().locale_name[..7], b"Second\0");
        assert_eq!(&protos.get(6).unwrap().locale_name[..4], b"Mob\0");
        let short = b"VNUM\tLOCALE_NAME\n5\n".to_vec();
        assert_eq!(
            parse(&proto_of(&rows), &short).unwrap_err(),
            MobProtoError::ShortRow {
                file: "mob_names.txt",
                line: 2,
                found: 1,
            }
        );
    }

    /// A vnum past `INT_MAX` finds its name by its 32-bit pattern, the `int` key `find` makes.
    #[test]
    fn a_vnum_past_int_max_finds_its_name_by_its_bits() {
        let rows = [blank_row("4294967295"), blank_row("2147483648")];
        let names = names_of(&[("-1", "Wrapped"), ("-2147483648", "Lowest"), ("0", "Zero")]);
        let protos = parse(&proto_of(&rows), &names).unwrap();
        assert_eq!(
            &protos.get(u32::MAX).unwrap().locale_name[..8],
            b"Wrapped\0"
        );
        assert_eq!(&protos.get(1 << 31).unwrap().locale_name[..7], b"Lowest\0");
    }

    /// Names are cut to 24 bytes and the folder to 64, as `strlcpy` cuts them.
    #[test]
    fn names_and_folder_are_cut_to_their_fields() {
        let mut row = blank_row("5");
        row[1] = vec![b'n'; 30];
        row[12] = vec![b'f'; 70];
        let table = one(row);
        assert_eq!(table.name[..24], [b'n'; 24]);
        assert_eq!(table.name[24], 0);
        assert_eq!(table.locale_name[..24], [b'n'; 24]);
        assert_eq!(table.folder[..64], [b'f'; 64]);
        assert_eq!(table.folder[64], 0);
        let owners = owners();
        let cut = owners
            .rows()
            .iter()
            .filter(|p| p.table.locale_name[MOB_NAME_MAX_LEN - 1] != 0)
            .count();
        assert!(cut > 0, "some owner names fill the whole field");
    }

    /// A repeated vnum answers with the first row in file order, and is reported.
    #[test]
    fn a_duplicate_vnum_answers_with_the_first_file_row() {
        let protos = owners();
        let duplicates = protos.duplicates();
        assert_eq!(duplicates.len(), 36);
        assert_eq!(duplicates[0], (3_501, 443));
        let mut first = blank_row("5");
        first[5] = b"1".to_vec();
        let mut second = blank_row("5");
        second[5] = b"2".to_vec();
        let protos = parse(&proto_of(&[second.clone(), first.clone()]), &names_of(&[])).unwrap();
        assert_eq!(protos.get(5).unwrap().level, 2, "line 2 is the first row");
        assert_eq!(protos.duplicates(), [(5, 2)]);
        assert!(protos.get(4).is_none());
        assert!(protos.get(6).is_none());
        let thrice = parse(&proto_of(&[second, first.clone(), first]), &names_of(&[])).unwrap();
        assert_eq!(thrice.duplicates(), [(5, 2)], "a vnum is reported once");
    }

    /// A missing file is named.
    #[test]
    fn a_missing_file_names_itself() {
        let empty = std::env::temp_dir().join("prodomo-no-such-mob-proto-folder");
        let error = MobProtos::load(&empty).unwrap_err();
        assert!(error.to_string().contains("mob_proto.txt"), "{error}");
        assert!(error.source().is_none());
    }
}
