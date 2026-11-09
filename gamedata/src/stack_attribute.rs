//! The stack-attribute table: `locale/europe/item_stack_attribute.txt`
//! (`ITEM_STACK_ATTRIBUTE::ReadItemStackAttrFile`,
//! `server/server/game/item_stack_attribute.cpp:27-107`).
//!
//! Each `Group` names a worn item by `vnum` and up to three numbered bonus rows. A row is
//! five `str_to_number` ints: the vnum that feeds it (0 for any), the stack type
//! (`ETypeStackAttribute`, `item_stack_attribute.h:9-17`: monster 0, stone 1, boss 2, item
//! 3, fish 4), the apply type the bonus grants, the progress per feed, and the stack
//! ceiling. The rows are tried in order and the first missing number ends the scan, so a
//! group with rows 1 and 3 keeps row 1 only. A group without a `vnum` refuses the whole
//! file; a repeated vnum keeps the first group, as `std::map::insert` does.
//!
//! Legacy reads the file from a hardcoded `locale/germany/` path (`:25`); the Rewrite
//! reads it from the Locale folder like every other table, and the parse below is the
//! part that must match byte for byte.
//!
//! `G/` = `server/server/game/`.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::Path;

use crate::text_file;

/// The file name under the Locale folder.
pub const STACK_ATTRIBUTE_FILE: &str = "item_stack_attribute.txt";

/// The stack types (`ETypeStackAttribute`).
pub const STACK_BY_MONSTER: i32 = 0;
/// The stack types (`ETypeStackAttribute`).
pub const STACK_BY_STONE: i32 = 1;
/// The stack types (`ETypeStackAttribute`).
pub const STACK_BY_BOSS: i32 = 2;
/// The stack types (`ETypeStackAttribute`).
pub const STACK_BY_ITEM: i32 = 3;
/// The stack types (`ETypeStackAttribute`).
pub const STACK_BY_FISH: i32 = 4;

/// One bonus row: what feeds it, what it grants, and how far it stacks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StackBonus {
    /// The feeding vnum, or 0 for any.
    pub feed_vnum: i32,
    /// The stack type that feeds it.
    pub stack_type: i32,
    /// The apply type the bonus grants.
    pub apply: i32,
    /// The progress one feed adds.
    pub progress: i32,
    /// The stack ceiling.
    pub ceiling: i32,
}

/// One group's stack rows: `SInfoItem` without the map key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StackEntry {
    /// The worn item's vnum.
    pub vnum: i32,
    /// The bonus rows, in file order; missing tail rows are absent, not zero.
    pub rows: Vec<StackBonus>,
}

/// The table: worn vnum to its stack rows, first group wins on a repeat.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StackAttributes {
    entries: BTreeMap<i32, StackEntry>,
}

impl StackAttributes {
    /// The entry for `vnum`, or `None` when no group names it.
    #[must_use]
    pub fn get(&self, vnum: i32) -> Option<&StackEntry> {
        self.entries.get(&vnum)
    }

    /// The entries in vnum order.
    pub fn entries(&self) -> impl Iterator<Item = &StackEntry> {
        self.entries.values()
    }

    /// Read the table from the Locale folder.
    ///
    /// # Errors
    ///
    /// Returns [`StackError`] when the file cannot be read or a group has no `vnum`.
    pub fn load(locale_dir: &Path) -> Result<Self, StackError> {
        let path = locale_dir.join(STACK_ATTRIBUTE_FILE);
        let bytes = std::fs::read(&path).map_err(|source| StackError::Unreadable {
            path: path.display().to_string(),
            message: source.to_string(),
        })?;
        read(&bytes)
    }
}

/// Why the stack table refused to load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackError {
    /// The file could not be read.
    Unreadable {
        /// The path read.
        path: String,
        /// The I/O failure.
        message: String,
    },
    /// A group has no `vnum` token.
    NoVnum {
        /// The group's 0-based index.
        group: usize,
    },
    /// The text would not parse.
    Text(text_file::TextFileError),
}

impl fmt::Display for StackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, message } => {
                write!(
                    formatter,
                    "stack attributes are unreadable in {path}: {message}"
                )
            }
            Self::NoVnum { group } => {
                write!(formatter, "stack attribute group {group} has no vnum")
            }
            Self::Text(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for StackError {}

impl From<text_file::TextFileError> for StackError {
    fn from(error: text_file::TextFileError) -> Self {
        Self::Text(error)
    }
}

/// `str_to_number` into an `int`: leading space, an optional sign, then digits; anything
/// else reads 0 from there, and the low four bytes win.
fn str_to_i32(bytes: &[u8]) -> i32 {
    let mut rest = bytes.iter().peekable();
    while rest.peek().is_some_and(|b| b.is_ascii_whitespace()) {
        rest.next();
    }
    let negative = rest.peek() == Some(&&b'-');
    if negative || rest.peek() == Some(&&b'+') {
        rest.next();
    }
    let mut magnitude: u64 = 0;
    for b in rest {
        if !b.is_ascii_digit() {
            break;
        }
        magnitude = magnitude.wrapping_mul(10).wrapping_add(u64::from(b - b'0'));
    }
    let value = if negative {
        magnitude.wrapping_neg()
    } else {
        magnitude
    };
    let bytes = value.to_le_bytes();
    i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Read the table from `data` (`ReadItemStackAttrFile`).
///
/// # Errors
///
/// Returns [`StackError::NoVnum`] when a group has no `vnum` token.
pub fn read(data: &[u8]) -> Result<StackAttributes, StackError> {
    let file = text_file::parse(data)?;
    let mut table = StackAttributes::default();
    for (index, group) in file.groups.iter().enumerate() {
        let Some(vnum) = group.get(b"vnum").and_then(|tokens| tokens.first()) else {
            return Err(StackError::NoVnum { group: index });
        };
        let vnum = str_to_i32(vnum);
        let mut rows = Vec::new();
        for row in [b"1".as_slice(), b"2".as_slice(), b"3".as_slice()] {
            let Some(tokens) = group.get(row) else {
                break;
            };
            let mut numbers = tokens.iter().map(Vec::as_slice).map(str_to_i32);
            let Some(feed_vnum) = numbers.next() else {
                break;
            };
            rows.push(StackBonus {
                feed_vnum,
                stack_type: numbers.next().unwrap_or(0),
                apply: numbers.next().unwrap_or(0),
                progress: numbers.next().unwrap_or(0),
                ceiling: numbers.next().unwrap_or(0),
            });
        }
        table
            .entries
            .entry(vnum)
            .or_insert(StackEntry { vnum, rows });
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's file: one group, three rows, the kill-monster, kill-stone and
    /// kill-boss feeds of the Aquarian skin.
    #[test]
    fn owner_table_loads_one_group() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/locale/europe");
        let table = StackAttributes::load(&path).expect("stack attributes load");
        let entry = table.get(40130).expect("the skin's entry");
        assert_eq!(
            entry.rows,
            [
                StackBonus {
                    feed_vnum: 0,
                    stack_type: STACK_BY_MONSTER,
                    apply: 63,
                    progress: 25,
                    ceiling: 2500,
                },
                StackBonus {
                    feed_vnum: 0,
                    stack_type: STACK_BY_STONE,
                    apply: 61,
                    progress: 20,
                    ceiling: 2000,
                },
                StackBonus {
                    feed_vnum: 591,
                    stack_type: STACK_BY_BOSS,
                    apply: 63,
                    progress: 15,
                    ceiling: 1500,
                },
            ]
        );
        assert_eq!(table.get(19), None);
    }

    /// A group without a `vnum` refuses the file; a repeated vnum keeps the first
    /// group; a missing middle row ends the scan.
    #[test]
    fn the_vnum_is_required_and_the_first_wins() {
        assert!(matches!(
            read(b"Group a\n{\n 1 0 0 63 25 2500\n}\n"),
            Err(StackError::NoVnum { group: 0 })
        ));
        let table =
            read(b"Group a\n{\n vnum 7\n 1 0 0 1 1 10\n}\nGroup b\n{\n vnum 7\n 1 0 0 2 2 20\n}\n")
                .expect("a repeated vnum loads");
        assert_eq!(table.get(7).expect("row 7").rows.len(), 1);
        assert_eq!(table.get(7).expect("row 7").rows[0].apply, 1);
        let table = read(b"Group a\n{\n vnum 9\n 1 0 0 1 1 10\n 3 0 0 3 3 30\n}\n")
            .expect("a gapped group loads");
        assert_eq!(table.get(9).expect("row 9").rows.len(), 1);
    }
}
