//! The blend table (`locale/europe/blend.json`), read at boot
//! (`CBlendItem::Initialize`, `G/blend_item.cpp:48-178`).
//!
//! Each entry names an `ITEM_BLEND` vnum, an apply type (resolved through
//! `FN_get_apply_type`, so 0 refuses the file), and five values and five
//! durations. A duplicate vnum refuses the file, as does a wrong array size.
//! The commented-out member check (`:107-121`) never runs: a missing member
//! reads as the wrong shape below it instead.
//!
//! # Divergences
//!
//! - **Boot failure.** A missing or malformed file stops the server, as for
//!   every other Game data file. Legacy only logs (`sys_err`) and boots with
//!   whatever it read so far.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::Path;

use super::item_proto_value::APPLY_TYPE_NAMES;
use super::json::{parse, Json, JsonError};

/// The file's name under the locale directory.
pub const BLEND_FILE: &str = "blend.json";

/// `blend::item_max_value` (`G/blend_item.h:17`): five values, five durations.
pub const BLEND_LEVELS: usize = 5;

/// `51002`, the energy crystal, whose roll runs the ECS table
/// (`CBlendItem::SetItemValue`, `G/blend_item.cpp:189`).
pub const ECS_BLEND_VNUM: u32 = 51_002;

/// One blend row: the apply type and the five values and durations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlendRow {
    /// `FN_get_apply_type` of the `apply_type` name.
    pub apply_type: u8,
    /// The five `apply_value` entries.
    pub values: [u32; BLEND_LEVELS],
    /// The five `apply_duration` entries.
    pub durations: [u32; BLEND_LEVELS],
}

/// The blend table, by item vnum.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlendTable {
    /// The rows, by item vnum.
    pub rows: BTreeMap<u32, BlendRow>,
}

impl BlendTable {
    /// `CBlendItem::FindItem`: whether `vnum` has a row.
    #[must_use]
    pub fn find(&self, vnum: u32) -> bool {
        self.rows.contains_key(&vnum)
    }

    /// `FN_random_index` (`G/blend_item.cpp:16-30`) and `FN_ECS_random_index`
    /// (`:32-46`): the level `percent` (1-100) rolls, the ECS table for
    /// [`ECS_BLEND_VNUM`].
    #[must_use]
    pub fn level_of(vnum: u32, percent: i32) -> usize {
        if vnum == ECS_BLEND_VNUM {
            if percent <= 5 {
                0
            } else if percent <= 15 {
                1
            } else if percent <= 60 {
                2
            } else if percent <= 85 {
                3
            } else {
                4
            }
        } else if percent <= 10 {
            0
        } else if percent <= 30 {
            1
        } else if percent <= 70 {
            2
        } else if percent <= 90 {
            3
        } else {
            4
        }
    }
}

/// Why the blend bytes would not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlendError {
    /// The file cannot be read.
    Unreadable {
        /// The path the boot tried.
        path: String,
        /// The I/O failure.
        error: String,
    },
    /// The bytes are not the JSON envelope the file needs.
    Malformed {
        /// The path the boot tried, when one was read.
        path: String,
        /// What the parse found.
        error: JsonError,
    },
    /// The root is not an array.
    NotArray {
        /// The path the boot tried, when one was read.
        path: String,
    },
    /// Entry `index` is unusable.
    BadEntry {
        /// The path the boot tried, when one was read.
        path: String,
        /// The entry's array index.
        index: usize,
        /// What the entry got wrong.
        error: BlendEntryError,
    },
}

/// Why one blend entry is unusable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlendEntryError {
    /// The entry is not an object.
    NotObject,
    /// The `item_vnum` is not a `u32` spelled as plain digits.
    BadVnum,
    /// The vnum already has a row.
    DuplicateVnum(u32),
    /// The `apply_type` is not a string, or names no apply type.
    BadApplyType,
    /// The `apply_value` is not a five-`u32` array.
    BadValues,
    /// The `apply_duration` is not a five-`u32` array.
    BadDurations,
}

impl BlendError {
    /// Name the file a raw [`read`] failure came from.
    #[must_use]
    pub fn with_path(self, path: String) -> Self {
        match self {
            Self::Malformed { error, .. } => Self::Malformed { path, error },
            Self::NotArray { .. } => Self::NotArray { path },
            Self::BadEntry { index, error, .. } => Self::BadEntry { path, index, error },
            unreadable @ Self::Unreadable { .. } => unreadable,
        }
    }
}

impl fmt::Display for BlendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, error } => {
                write!(f, "blend file is unreadable in {path}: {error}")
            }
            Self::Malformed { path, error } => {
                write!(f, "blend file is unusable in {path}: {error}")
            }
            Self::NotArray { path } => {
                write!(f, "blend file is not an array in {path}")
            }
            Self::BadEntry { path, index, error } => {
                write!(f, "blend file entry {index} is unusable in {path}: {error}")
            }
        }
    }
}

impl Error for BlendError {}

impl fmt::Display for BlendEntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "the entry is not an object"),
            Self::BadVnum => write!(f, "the `item_vnum` is not an integer"),
            Self::DuplicateVnum(vnum) => write!(f, "vnum {vnum} already exists"),
            Self::BadApplyType => write!(f, "the `apply_type` is unknown"),
            Self::BadValues => write!(f, "the `apply_value` is not five integers"),
            Self::BadDurations => write!(f, "the `apply_duration` is not five integers"),
        }
    }
}

impl Error for BlendEntryError {}

/// `FN_get_apply_type` (`G/constants.cpp:1324-1333`): the first table row whose
/// name matches case-insensitively, or 0.
fn apply_type_of(name: &[u8]) -> u32 {
    APPLY_TYPE_NAMES
        .iter()
        .find(|(known, _)| known.as_bytes().eq_ignore_ascii_case(name))
        .map_or(0, |(_, value)| *value)
}

fn read_uints(entry: &Json, name: &[u8]) -> Option<[u32; BLEND_LEVELS]> {
    let Json::Array(items) = entry.member(name)? else {
        return None;
    };
    if items.len() != BLEND_LEVELS {
        return None;
    }
    let mut out = [0_u32; BLEND_LEVELS];
    for (slot, item) in out.iter_mut().zip(items.iter()) {
        *slot = item.as_uint()?;
    }
    Some(out)
}

/// `CBlendItem::Initialize` on the file's bytes: the rows by vnum.
///
/// # Errors
///
/// [`BlendError`]: the bytes refuse the JSON parse, or an entry is unusable.
pub fn read(data: &[u8]) -> Result<BlendTable, BlendError> {
    let root = parse(data).map_err(|error| BlendError::Malformed {
        path: String::new(),
        error,
    })?;
    let Json::Array(entries) = &root else {
        return Err(BlendError::NotArray {
            path: String::new(),
        });
    };
    let mut table = BlendTable::default();
    for (index, entry) in entries.iter().enumerate() {
        let failed = |error| BlendError::BadEntry {
            path: String::new(),
            index,
            error,
        };
        let Json::Object(_) = entry else {
            return Err(failed(BlendEntryError::NotObject));
        };
        let Some(vnum) = entry.member(b"item_vnum").and_then(Json::as_uint) else {
            return Err(failed(BlendEntryError::BadVnum));
        };
        if table.rows.contains_key(&vnum) {
            return Err(failed(BlendEntryError::DuplicateVnum(vnum)));
        }
        let apply = match entry.member(b"apply_type") {
            Some(Json::Text(name)) => apply_type_of(name),
            _ => 0,
        };
        let Ok(apply_type) = u8::try_from(apply) else {
            return Err(failed(BlendEntryError::BadApplyType));
        };
        if apply_type == 0 {
            return Err(failed(BlendEntryError::BadApplyType));
        }
        let Some(values) = read_uints(entry, b"apply_value") else {
            return Err(failed(BlendEntryError::BadValues));
        };
        let Some(durations) = read_uints(entry, b"apply_duration") else {
            return Err(failed(BlendEntryError::BadDurations));
        };
        table.rows.insert(
            vnum,
            BlendRow {
                apply_type,
                values,
                durations,
            },
        );
    }
    Ok(table)
}

/// Read the locale directory's [`BLEND_FILE`].
///
/// # Errors
///
/// [`BlendError`]: the file cannot be read, or [`read`] refuses it.
pub fn load(locale_dir: &Path) -> Result<BlendTable, BlendError> {
    let path = locale_dir.join(BLEND_FILE);
    let data = std::fs::read(&path).map_err(|error| BlendError::Unreadable {
        path: path.display().to_string(),
        error: error.to_string(),
    })?;
    read(&data).map_err(|error| error.with_path(path.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's file reads: 24 rows, and 50821's critical row lands whole.
    #[test]
    fn the_owner_file_reads() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let table =
            load(&root.join("../legacy/gamedata/locale/europe")).expect("the owner file reads");
        assert_eq!(table.rows.len(), 24);
        let row = table.rows.get(&50_821).expect("50821 blends");
        assert_eq!(row.apply_type, 15, "APPLY_CRITICAL_PCT");
        assert_eq!(row.values, [20, 20, 20, 20, 20]);
        assert_eq!(row.durations, [600, 600, 600, 600, 600]);
        assert!(table.find(51_002), "the energy crystal blends");
        assert!(!table.find(50_000), "no row blends");
    }

    /// The roll tables: the plain edges and the ECS edges.
    #[test]
    fn the_levels_roll_the_two_tables() {
        assert_eq!(BlendTable::level_of(50_821, 1), 0);
        assert_eq!(BlendTable::level_of(50_821, 10), 0);
        assert_eq!(BlendTable::level_of(50_821, 11), 1);
        assert_eq!(BlendTable::level_of(50_821, 30), 1);
        assert_eq!(BlendTable::level_of(50_821, 31), 2);
        assert_eq!(BlendTable::level_of(50_821, 70), 2);
        assert_eq!(BlendTable::level_of(50_821, 71), 3);
        assert_eq!(BlendTable::level_of(50_821, 90), 3);
        assert_eq!(BlendTable::level_of(50_821, 91), 4);
        assert_eq!(BlendTable::level_of(ECS_BLEND_VNUM, 5), 0);
        assert_eq!(BlendTable::level_of(ECS_BLEND_VNUM, 6), 1);
        assert_eq!(BlendTable::level_of(ECS_BLEND_VNUM, 15), 1);
        assert_eq!(BlendTable::level_of(ECS_BLEND_VNUM, 16), 2);
        assert_eq!(BlendTable::level_of(ECS_BLEND_VNUM, 60), 2);
        assert_eq!(BlendTable::level_of(ECS_BLEND_VNUM, 61), 3);
        assert_eq!(BlendTable::level_of(ECS_BLEND_VNUM, 85), 3);
        assert_eq!(BlendTable::level_of(ECS_BLEND_VNUM, 86), 4);
    }

    /// A duplicate vnum, a wrong array size and an unknown apply type each
    /// refuse the file, as legacy's `sys_err` returns do.
    #[test]
    fn bad_entries_refuse_the_file() {
        let row = |vnum: u32, apply: &str, values: &str| {
            format!(
                "{{\"item_vnum\": {vnum}, \"apply_type\": \"{apply}\", \
                 \"apply_value\": [{values}], \"apply_duration\": [1, 1, 1, 1, 1]}}"
            )
        };
        let good = row(50_821, "CRITICAL_PCT", "20, 20, 20, 20, 20");
        let dup = format!("[{good}, {good}]");
        assert_eq!(
            read(dup.as_bytes()),
            Err(BlendError::BadEntry {
                path: String::new(),
                index: 1,
                error: BlendEntryError::DuplicateVnum(50_821),
            })
        );
        let short = format!("[{}]", row(50_821, "CRITICAL_PCT", "20, 20, 20, 20"));
        assert!(matches!(
            read(short.as_bytes()),
            Err(BlendError::BadEntry {
                error: BlendEntryError::BadValues,
                ..
            })
        ));
        let unknown = format!("[{}]", row(50_821, "NO_SUCH_APPLY", "20, 20, 20, 20, 20"));
        assert!(matches!(
            read(unknown.as_bytes()),
            Err(BlendError::BadEntry {
                error: BlendEntryError::BadApplyType,
                ..
            })
        ));
        assert!(matches!(read(b"{}"), Err(BlendError::NotArray { .. })));
    }
}
