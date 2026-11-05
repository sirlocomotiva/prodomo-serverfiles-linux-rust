//! The dragon soul table: `locale/europe/dragon_soul_table.txt`, read by `DSManager`.
//!
//! Legacy loads it once at boot (`LoadLocaleFile`, `G/input_db.cpp:953-957`) and only logs on a
//! failure; the Rewrite loads it the same way, but a file it cannot read stops the server, as
//! every other Game data file does. The parse sits on [`crate::text_file`], which is the
//! `CGroupTextParseTreeLoader` port: group names are lowercased, keys keep their first value, and
//! the `#--#` header line maps column names to indexes.
//!
//! Only the groups `ChangeAttributes` reads are kept: the type names (`VnumMapper`), the basic
//! and additional applies, the apply-number settings, and the weight tables. The refine and
//! extract groups ride with their systems.
//!
//! Three legacy behaviours are reproduced rather than improved:
//!
//! - Column lookups go through the `#--#` header, and a grade or step name the header lacks
//!   refuses the table. The owner's `GRADE_BRILLIANT` weight group has no header line, so its
//!   lookups run against the empty map and refuse; that group's weights are therefore read by
//!   position, as the row's own order has them.
//! - `GetGroupValue` falls back to the `default` child when the named child is missing; for the
//!   numbers and weights the owner's file has only `default`, so every lookup lands there.
//! - `ReadBasicApplys` reads rows `1..=count` by key, so a gap or a non-numeric key refuses the
//!   table, where `ReadAdditionalApplys` walks rows `0..count` by file position, so any keys
//!   do. The owner's rows are `1..=n` in both, and the walks agree.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::Path;

use crate::item_proto_value::fn_get_apply_type;
use crate::text_file::{self, TextFile, TextGroup};

/// The file under the locale directory.
pub const DRAGON_SOUL_TABLE_FILE: &str = "dragon_soul_table.txt";

/// `DRAGON_SOUL_GRADE_MAX` (`item_length.h:194`): the six grades, mythic last.
pub const GRADES: usize = 6;

/// `DRAGON_SOUL_STEP_MAX` (`item_length.h:205`): the five steps.
pub const STEPS: usize = 5;

/// `DRAGON_SOUL_STRENGTH_MAX` (`item_length.h:207`): the seven strengths.
pub const STRENGTHS: usize = 7;

/// The grade names `GetApplyNumSettings` and `GetWeight` look up (`g_astGradeName`,
/// `G/dragon_soul_table.cpp:7-16`).
pub const GRADE_NAMES: [&str; GRADES] = [
    "grade_normal",
    "grade_brilliant",
    "grade_rare",
    "grade_ancient",
    "grade_legendary",
    "grade_mythic",
];

/// The step names `GetWeight` looks up (`g_astStepName`, `G/dragon_soul_table.cpp:18-25`).
pub const STEP_NAMES: [&str; STEPS] = [
    "step_lowest",
    "step_low",
    "step_mid",
    "step_high",
    "step_highest",
];

/// One `SApply` (`G/dragon_soul_table.h:6-11`): an apply type, its value, and, for an
/// additional apply, its draw weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragonSoulApply {
    /// The `FN_get_apply_type` value of the `apply_type` column.
    pub apply_type: u32,
    /// The `apply_value` column.
    pub apply_value: i32,
    /// The `prob` column, or 0 for a basic apply.
    pub prob: f32,
}

/// What `ChangeAttributes` reads of the table: the applies by soul type, the apply numbers
/// by grade, and the weights by grade, step and strength.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DragonSoulTable {
    /// The basic applies of each soul type, in row order.
    basic: BTreeMap<u8, Vec<DragonSoulApply>>,
    /// The additional applies of each soul type, in row order.
    additional: BTreeMap<u8, Vec<DragonSoulApply>>,
    /// The `(basis, add_min, add_max)` of each grade.
    numbers: [(i32, i32, i32); GRADES],
    /// The weight of each grade, step and strength.
    weights: [[[f32; STRENGTHS]; STEPS]; GRADES],
}

/// Why the dragon soul table could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DragonSoulTableError {
    /// The file could not be opened.
    Unreadable(std::path::PathBuf, String),
    /// The text reader refused the file.
    Text(text_file::TextFileError),
    /// A group the change-attribute read needs is missing.
    MissingGroup(&'static str),
    /// A row, column or value the change-attribute read needs is missing or malformed.
    BadValue {
        /// Where the read was: the group, row and column that name the value.
        at: String,
    },
}

impl fmt::Display for DragonSoulTableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(path, error) => write!(f, "{}: {error}", path.display()),
            Self::Text(error) => write!(f, "{error}"),
            Self::MissingGroup(group) => write!(f, "the table has no {group} group"),
            Self::BadValue { at } => write!(f, "the table's {at} is missing or malformed"),
        }
    }
}

impl Error for DragonSoulTableError {}

impl From<text_file::TextFileError> for DragonSoulTableError {
    fn from(error: text_file::TextFileError) -> Self {
        Self::Text(error)
    }
}

impl DragonSoulTable {
    /// Load the table from the locale directory.
    ///
    /// # Errors
    ///
    /// [`DragonSoulTableError::Unreadable`] when the file cannot be opened, and the table's
    /// own errors when a group the change-attribute read needs is missing or malformed.
    pub fn load(locale_dir: &Path) -> Result<Self, DragonSoulTableError> {
        let path = locale_dir.join(DRAGON_SOUL_TABLE_FILE);
        let bytes = std::fs::read(&path)
            .map_err(|error| DragonSoulTableError::Unreadable(path.clone(), error.to_string()))?;
        Self::parse(&bytes)
    }

    /// Parse the table's bytes.
    ///
    /// # Errors
    ///
    /// When a group the change-attribute read needs is missing or malformed.
    pub fn parse(bytes: &[u8]) -> Result<Self, DragonSoulTableError> {
        let file = text_file::parse(bytes)?;
        let names = vnum_map(&file)?;
        let mut table = Self::default();
        read_applies(&file, &names, &mut table)?;
        read_numbers(&file, &mut table)?;
        read_weights(&file, &mut table)?;
        Ok(table)
    }

    /// `GetBasicApplys` (`G/dragon_soul_table.cpp:581-590`): the basic applies of `ds_type`,
    /// in row order, or `None` when the type has none.
    #[must_use]
    pub fn basic_applies(&self, ds_type: u8) -> Option<&[DragonSoulApply]> {
        self.basic.get(&ds_type).map(Vec::as_slice)
    }

    /// `GetAdditionalApplys` (`G/dragon_soul_table.cpp:592-601`): the additional applies of
    /// `ds_type`, in row order, or `None` when the type has none.
    #[must_use]
    pub fn additional_applies(&self, ds_type: u8) -> Option<&[DragonSoulApply]> {
        self.additional.get(&ds_type).map(Vec::as_slice)
    }

    /// `GetApplyNumSettings` (`G/dragon_soul_table.cpp:603-638`): the `(basis, add_min,
    /// add_max)` of `grade`, or `None` past the grades.
    #[must_use]
    pub fn apply_numbers(&self, grade: u8) -> Option<(i32, i32, i32)> {
        self.numbers.get(usize::from(grade)).copied()
    }

    /// `GetWeight` (`G/dragon_soul_table.cpp:640-678`): the weight of `grade`, `step` and
    /// `strength`, or `None` past the tables.
    #[must_use]
    pub fn weight(&self, grade: u8, step: u8, strength: u8) -> Option<f32> {
        self.weights
            .get(usize::from(grade))
            .and_then(|grades| grades.get(usize::from(step)))
            .and_then(|steps| steps.get(usize::from(strength)))
            .copied()
    }
}

/// `ReadVnumMapper` (`G/dragon_soul_table.cpp:95-157`): the `(name, type)` rows in file order.
///
/// Rows walk by position (`GetRow(i)`), and the values come from the `dragonsoulname` and
/// `type` columns. A duplicated type refuses the table; a duplicated name keeps its first
/// row, as the text reader's `insert` does.
fn vnum_map(file: &TextFile) -> Result<Vec<(Vec<u8>, u8)>, DragonSoulTableError> {
    let group = file
        .group(b"vnummapper")
        .ok_or(DragonSoulTableError::MissingGroup("VnumMapper"))?;
    let mut names = Vec::new();
    let mut seen = Vec::new();
    for (key, _) in group.entries.iter().filter(|(key, _)| key != b"#--#") {
        let name = column(group, key, b"dragonsoulname", "VnumMapper")?;
        let raw = column(group, key, b"type", "VnumMapper")?;
        // `from_string<BYTE>`: the text as an `int`, then the low byte.
        let parsed = parse_int(&raw).ok_or_else(|| bad("VnumMapper", key, b"type"))?;
        let soul_type = u8::try_from(parsed & 0xff).unwrap_or(u8::MAX);
        if seen.contains(&soul_type) {
            return Err(bad("VnumMapper", key, b"type"));
        }
        seen.push(soul_type);
        names.push((name, soul_type));
    }
    if names.is_empty() {
        return Err(DragonSoulTableError::MissingGroup("VnumMapper"));
    }
    Ok(names)
}

/// `ReadBasicApplys` and `ReadAdditionalApplys` (`G/dragon_soul_table.cpp:159-275`): the
/// applies of every mapped name, in row order.
///
/// The basic rows read `1..=count` by key; the additional rows walk by position. An apply
/// name `FN_get_apply_type` answers 0 for refuses the table.
fn read_applies(
    file: &TextFile,
    names: &[(Vec<u8>, u8)],
    table: &mut DragonSoulTable,
) -> Result<(), DragonSoulTableError> {
    let basic = file
        .group(b"basicapplys")
        .ok_or(DragonSoulTableError::MissingGroup("BasicApplys"))?;
    let additional = file
        .group(b"additionalapplys")
        .ok_or(DragonSoulTableError::MissingGroup("AdditionalApplys"))?;
    for (name, soul_type) in names {
        let child = basic
            .children
            .iter()
            .find(|child| child.name == *name)
            .ok_or(DragonSoulTableError::MissingGroup("BasicApplys"))?;
        table.basic.insert(*soul_type, read_basic(child, name)?);
        let child = additional
            .children
            .iter()
            .find(|child| child.name == *name)
            .ok_or(DragonSoulTableError::MissingGroup("AdditionalApplys"))?;
        table
            .additional
            .insert(*soul_type, read_additional(child, name)?);
    }
    Ok(())
}

/// The basic applies of one soul group: rows `1..=count` by key.
fn read_basic(
    group: &TextGroup,
    name: &[u8],
) -> Result<Vec<DragonSoulApply>, DragonSoulTableError> {
    let rows: Vec<&(Vec<u8>, Vec<Vec<u8>>)> = group
        .entries
        .iter()
        .filter(|(key, _)| key != b"#--#")
        .collect();
    let mut applies = Vec::new();
    for index in 1..=rows.len() {
        let key = index.to_string().into_bytes();
        if !rows.iter().any(|(row, _)| *row == key) {
            return Err(DragonSoulTableError::BadValue {
                at: format!(
                    "BasicApplys group {} row {index}",
                    String::from_utf8_lossy(name)
                ),
            });
        }
        applies.push(read_apply(group, &key, name, "BasicApplys", false)?);
    }
    Ok(applies)
}

/// The additional applies of one soul group: the rows in file order.
fn read_additional(
    group: &TextGroup,
    name: &[u8],
) -> Result<Vec<DragonSoulApply>, DragonSoulTableError> {
    let mut applies = Vec::new();
    for (key, _) in group.entries.iter().filter(|(key, _)| key != b"#--#") {
        applies.push(read_apply(group, key, name, "AdditionalApplys", true)?);
    }
    Ok(applies)
}

/// One apply row: the type by name, the value, and the weight for an additional apply.
fn read_apply(
    group: &TextGroup,
    key: &[u8],
    name: &[u8],
    section: &str,
    weighted: bool,
) -> Result<DragonSoulApply, DragonSoulTableError> {
    let raw = column(group, key, b"apply_type", section)?;
    let apply_type = fn_get_apply_type(&raw);
    if apply_type == 0 {
        return Err(DragonSoulTableError::BadValue {
            at: format!(
                "{section} group {} row {} apply_type",
                String::from_utf8_lossy(name),
                String::from_utf8_lossy(key)
            ),
        });
    }
    let raw = column(group, key, b"apply_value", section)?;
    let apply_value = parse_int(&raw).ok_or_else(|| bad(section, key, b"apply_value"))?;
    let prob = if weighted {
        let raw = column(group, key, b"prob", section)?;
        parse_float(&raw).ok_or_else(|| bad(section, key, b"prob"))?
    } else {
        0.0
    };
    Ok(DragonSoulApply {
        apply_type,
        apply_value,
        prob,
    })
}

/// `GetApplyNumSettings`' rows (`G/dragon_soul_table.cpp:603-638`): the `basis`, `add_min`
/// and `add_max` of every grade, from the `default` child, whose columns are the grade names.
fn read_numbers(file: &TextFile, table: &mut DragonSoulTable) -> Result<(), DragonSoulTableError> {
    let settings = file
        .group(b"applynumsettings")
        .ok_or(DragonSoulTableError::MissingGroup("ApplyNumSettings"))?;
    let defaults = settings
        .children
        .iter()
        .find(|child| child.name == b"default")
        .ok_or(DragonSoulTableError::MissingGroup("ApplyNumSettings"))?;
    for (index, grade) in GRADE_NAMES.iter().enumerate() {
        let basis =
            column(defaults, b"basis", grade.as_bytes(), "ApplyNumSettings").and_then(|raw| {
                parse_int(&raw).ok_or_else(|| bad("ApplyNumSettings", b"basis", grade.as_bytes()))
            })?;
        let add_min =
            column(defaults, b"add_min", grade.as_bytes(), "ApplyNumSettings").and_then(|raw| {
                parse_int(&raw).ok_or_else(|| bad("ApplyNumSettings", b"add_min", grade.as_bytes()))
            })?;
        let add_max =
            column(defaults, b"add_max", grade.as_bytes(), "ApplyNumSettings").and_then(|raw| {
                parse_int(&raw).ok_or_else(|| bad("ApplyNumSettings", b"add_max", grade.as_bytes()))
            })?;
        table.numbers[index] = (basis, add_min, add_max);
    }
    Ok(())
}

/// `GetWeight`'s rows (`G/dragon_soul_table.cpp:640-678`): the weight of every grade, step
/// and strength, from the `default` child's grade groups, whose columns are the strengths.
///
/// The owner's `grade_brilliant` group has no `#--#` header, so its columns have no names and
/// the lookup by strength refuses; the weights there read by position instead, in the row's
/// own order.
fn read_weights(file: &TextFile, table: &mut DragonSoulTable) -> Result<(), DragonSoulTableError> {
    let weights = file
        .group(b"weighttables")
        .ok_or(DragonSoulTableError::MissingGroup("WeightTables"))?;
    let defaults = weights
        .children
        .iter()
        .find(|child| child.name == b"default")
        .ok_or(DragonSoulTableError::MissingGroup("WeightTables"))?;
    for (grade, name) in GRADE_NAMES.iter().enumerate() {
        let group = defaults
            .children
            .iter()
            .find(|child| child.name == name.as_bytes())
            .ok_or(DragonSoulTableError::MissingGroup("WeightTables"))?;
        for (step, step_name) in STEP_NAMES.iter().enumerate() {
            let values = group
                .get(step_name.as_bytes())
                .ok_or_else(|| bad("WeightTables", step_name.as_bytes(), name.as_bytes()))?;
            let headered = group.get(b"#--#").is_some();
            for strength in 0..STRENGTHS {
                let raw = if headered {
                    let key = strength.to_string().into_bytes();
                    column(group, step_name.as_bytes(), &key, "WeightTables")?
                } else {
                    values.get(strength).cloned().ok_or_else(|| {
                        bad(
                            "WeightTables",
                            step_name.as_bytes(),
                            strength.to_string().as_bytes(),
                        )
                    })?
                };
                let weight = parse_float(&raw).ok_or_else(|| {
                    bad(
                        "WeightTables",
                        step_name.as_bytes(),
                        strength.to_string().as_bytes(),
                    )
                })?;
                table.weights[grade][step][strength] = weight;
            }
        }
    }
    Ok(())
}

/// The value of `column` in `row`: the header names the index, `from_string` reads the text.
///
/// The header names compare lowercased: legacy lowers them on the way in
/// (`group_text_parse_tree.cpp:100`), where [`text_file`] keeps values raw.
fn column(
    group: &TextGroup,
    row: &[u8],
    column: &[u8],
    section: &str,
) -> Result<Vec<u8>, DragonSoulTableError> {
    let header = group
        .get(b"#--#")
        .ok_or_else(|| bad(section, row, column))?;
    let index = header
        .iter()
        .position(|name| name.eq_ignore_ascii_case(column))
        .ok_or_else(|| bad(section, row, column))?;
    let values = group.get(row).ok_or_else(|| bad(section, row, column))?;
    values
        .get(index)
        .cloned()
        .ok_or_else(|| bad(section, row, column))
}

/// `operator>>(istringstream, int)`: leading whitespace, an optional sign, then digits; the
/// rest of the text is ignored and no digits refuses.
fn parse_int(text: &[u8]) -> Option<i32> {
    let text = skip_space(text);
    let (digits, negative) = match text.first() {
        Some(b'+') => (&text[1..], false),
        Some(b'-') => (&text[1..], true),
        _ => (text, false),
    };
    let mut value: i64 = 0;
    let mut any = false;
    for byte in digits {
        let Some(digit) = byte.checked_sub(b'0').filter(|digit| *digit < 10) else {
            break;
        };
        any = true;
        value = value.saturating_mul(10).saturating_add(i64::from(digit));
    }
    if !any {
        return None;
    }
    let value = if negative { -value } else { value };
    let clamped = value.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    Some(i32::try_from(clamped).unwrap_or(i32::MAX))
}

/// `operator>>(istringstream, float)`: the longest float prefix; anything else refuses.
fn parse_float(text: &[u8]) -> Option<f32> {
    let text = skip_space(text);
    let mut end = 0;
    if text
        .first()
        .is_some_and(|byte| *byte == b'+' || *byte == b'-')
    {
        end += 1;
    }
    let digits = |byte: u8| byte.is_ascii_digit();
    let before = text[end..].iter().take_while(|byte| digits(**byte)).count();
    end += before;
    if text.get(end) == Some(&b'.') {
        end += 1;
        end += text[end..].iter().take_while(|byte| digits(**byte)).count();
    }
    if before == 0 && text[..end].iter().all(|byte| !digits(*byte)) {
        return None;
    }
    if matches!(text.get(end), Some(b'e' | b'E')) {
        let mut rest = end + 1;
        if matches!(text.get(rest), Some(b'+' | b'-')) {
            rest += 1;
        }
        let exponent = text[rest..]
            .iter()
            .take_while(|byte| digits(**byte))
            .count();
        if exponent == 0 {
            return None;
        }
        end = rest + exponent;
    }
    if end == 0 {
        return None;
    }
    core::str::from_utf8(&text[..end]).ok()?.parse().ok()
}

/// The text past its leading `isspace` bytes.
fn skip_space(text: &[u8]) -> &[u8] {
    let end = text
        .iter()
        .take_while(|byte| byte.is_ascii_whitespace())
        .count();
    &text[end..]
}

/// A missing or malformed value at `group` row `row` column `column`.
fn bad(group: &str, row: &[u8], column: &[u8]) -> DragonSoulTableError {
    DragonSoulTableError::BadValue {
        at: format!(
            "{group} row {} column {}",
            String::from_utf8_lossy(row),
            String::from_utf8_lossy(column)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's file, two directories up from this crate.
    fn owner_bytes() -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe/dragon_soul_table.txt");
        std::fs::read(&path).expect("the owner's table reads")
    }

    #[test]
    fn the_owner_table_maps_six_types_with_their_applies() {
        let table = DragonSoulTable::parse(&owner_bytes()).expect("the owner's table parses");
        // Diamond is type 11 with two basic applies (`INT 10`, `RESIST_ICE 10`).
        let basic = table.basic_applies(11).expect("diamond's basics");
        assert_eq!(basic.len(), 2);
        assert_eq!(basic[0].apply_type, fn_get_apply_type(b"INT"));
        assert_eq!(basic[0].apply_value, 10);
        assert_eq!(basic[1].apply_type, fn_get_apply_type(b"RESIST_ICE"));
        assert_eq!(basic[1].apply_value, 10);
        // Diamond's four additionals each weigh 10.
        let additional = table.additional_applies(11).expect("diamond's additionals");
        assert_eq!(additional.len(), 4);
        assert_eq!(
            additional[0].apply_type,
            fn_get_apply_type(b"MAGIC_ATT_GRADE")
        );
        assert_eq!(additional[0].apply_value, 15);
        assert_eq!(additional[0].prob.to_bits(), 10.0f32.to_bits());
        // Onyx is type 16; no type 10 exists.
        assert_eq!(
            table.basic_applies(16).map(<[DragonSoulApply]>::len),
            Some(2)
        );
        assert_eq!(
            table.additional_applies(16).map(<[DragonSoulApply]>::len),
            Some(5)
        );
        assert_eq!(table.basic_applies(10), None);
        assert_eq!(table.additional_applies(10), None);
    }

    #[test]
    fn the_owner_numbers_and_weights_read() {
        let table = DragonSoulTable::parse(&owner_bytes()).expect("the owner's table parses");
        assert_eq!(table.apply_numbers(0), Some((1, 0, 0)));
        assert_eq!(table.apply_numbers(1), Some((1, 0, 1)));
        assert_eq!(table.apply_numbers(5), Some((4, 3, 5)));
        assert_eq!(table.apply_numbers(6), None);
        assert_eq!(table.weight(0, 0, 0), Some(1.0));
        assert_eq!(table.weight(0, 4, 4), Some(10.0));
        // The headerless brilliant group reads by position.
        assert_eq!(table.weight(1, 0, 0), Some(3.0));
        assert_eq!(table.weight(1, 4, 4), Some(20.0));
        assert_eq!(table.weight(5, 4, 6), Some(160.0));
        assert_eq!(table.weight(6, 0, 0), None);
        assert_eq!(table.weight(0, 5, 0), None);
        assert_eq!(table.weight(0, 0, 7), None);
    }

    #[test]
    fn a_missing_group_and_a_bad_apply_refuse() {
        let missing = b"Group VnumMapper\n{\n}\n";
        assert_eq!(
            DragonSoulTable::parse(missing),
            Err(DragonSoulTableError::MissingGroup("VnumMapper"))
        );
        let mut bytes = owner_bytes();
        // The first apply name becomes one the table never heard of.
        let needle = b"INT\t10";
        let at = bytes
            .windows(needle.len())
            .position(|window| window == needle)
            .expect("the owner's first apply");
        bytes[at..at + 3].copy_from_slice(b"ZZZ");
        assert!(matches!(
            DragonSoulTable::parse(&bytes),
            Err(DragonSoulTableError::BadValue { .. })
        ));
    }

    #[test]
    fn the_int_and_float_reads_match_the_stream() {
        assert_eq!(parse_int(b"42"), Some(42));
        assert_eq!(parse_int(b"  -7rest"), Some(-7));
        assert_eq!(parse_int(b"+12"), Some(12));
        assert_eq!(parse_int(b""), None);
        assert_eq!(parse_int(b"abc"), None);
        assert_eq!(parse_int(b"9999999999999"), Some(i32::MAX));
        assert_eq!(parse_float(b"10"), Some(10.0));
        assert_eq!(parse_float(b"  -2.5 "), Some(-2.5));
        assert_eq!(parse_float(b"1e2"), Some(100.0));
        assert_eq!(parse_float(b""), None);
        assert_eq!(parse_float(b"abc"), None);
        assert_eq!(parse_float(b"."), None);
    }
}
