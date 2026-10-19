//! The item vnum mask table: `ori_to_new_table.txt`
//! (`G/item_manager_read_tables.cpp:938-953`).
//!
//! Each line holds `ori_vnum new_vnum`, read with `fscanf(fp, "%u %u")`. The map runs
//! new-to-ori (`m_map_new_to_ori`): `GetMaskVnum(vnum)` answers the ori vnum when the
//! created vnum is a new-vnum, else 0. `std::map::insert` keeps the first row on a
//! repeated new-vnum. A missing file only logs (`G/input_db.cpp:932-935`), so it loads
//! as empty; an empty file loads fine.
//!
//! Legacy never sees the bytes after a malformed line the way this reader does:
//! `fscanf` stops at the first conversion failure and keeps what it read before. A line
//! that is not two unsigned 32-bit numbers refuses the load, which is stricter than the
//! silent stop — a Divergence, while the owner's file (62 rows) loads.
//!
//! `G/` = `server/server/game/`.

use std::error::Error;
use std::fmt;
use std::path::Path;

/// The file name under the Locale folder.
pub const MASK_TABLE_FILE: &str = "ori_to_new_table.txt";

/// The mask table: `(new_vnum, ori_vnum)` pairs, in file order, first-wins on a repeat.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemMasks {
    pairs: Vec<(u32, u32)>,
}

/// What reading the mask table reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemMaskError {
    /// The file could not be read.
    Io {
        /// The file.
        path: std::path::PathBuf,
        /// The failure.
        source: String,
    },
    /// A line is not two unsigned 32-bit numbers.
    BadLine {
        /// The 1-based line.
        line: usize,
    },
}

impl fmt::Display for ItemMaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::BadLine { line } => write!(f, "mask table line {line} is not two vnums"),
        }
    }
}

impl Error for ItemMaskError {}

impl ItemMasks {
    /// Read the table from the Locale folder. A missing file loads as empty, as legacy's
    /// boot does; an unreadable or malformed one refuses.
    ///
    /// # Errors
    ///
    /// Returns [`ItemMaskError`] when the file cannot be read (other than missing) or a
    /// line is not two vnums.
    pub fn load(locale_dir: &Path) -> Result<Self, ItemMaskError> {
        let path = locale_dir.join(MASK_TABLE_FILE);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(source) => {
                return Err(ItemMaskError::Io {
                    path,
                    source: source.to_string(),
                });
            }
        };
        Self::from_bytes(&bytes)
    }

    /// Build the table from the file's bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ItemMaskError::BadLine`] for a line that is not two unsigned 32-bit
    /// numbers. Blank lines (only whitespace) are skipped, as `fscanf`'s whitespace run
    /// does; anything else must convert twice.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ItemMaskError> {
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        for (at, line) in bytes.split(|&b| b == b'\n').enumerate() {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let mut parts = line
                .split(u8::is_ascii_whitespace)
                .filter(|part| !part.is_empty());
            let (Some(ori), Some(new), None) = (parts.next(), parts.next(), parts.next()) else {
                return Err(ItemMaskError::BadLine { line: at + 1 });
            };
            let number = |part: &[u8]| {
                if part.is_empty() || !part.iter().all(u8::is_ascii_digit) {
                    return None;
                }
                let mut value: u64 = 0;
                for &digit in part {
                    value = value * 10 + u64::from(digit - b'0');
                    if value > u64::from(u32::MAX) {
                        return None;
                    }
                }
                u32::try_from(value).ok()
            };
            let (Some(ori), Some(new)) = (number(ori), number(new)) else {
                return Err(ItemMaskError::BadLine { line: at + 1 });
            };
            // First-wins, as `std::map::insert` keeps the first.
            if !pairs.iter().any(|&(have, _)| have == new) {
                pairs.push((new, ori));
            }
        }
        Ok(Self { pairs })
    }

    /// `ITEM_MANAGER::GetMaskVnum` (`G/item_manager.cpp:1937-1946`): the ori vnum for a
    /// new-vnum, else 0.
    #[must_use]
    pub fn get(&self, vnum: u32) -> u32 {
        self.pairs
            .iter()
            .find(|&&(new, _)| new == vnum)
            .map_or(0, |&(_, ori)| ori)
    }

    /// The pairs, in file order, for the creation call.
    #[must_use]
    pub fn pairs(&self) -> &[(u32, u32)] {
        &self.pairs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_table_loads_62() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/locale/europe");
        let masks = ItemMasks::load(&path).expect("mask table loads");
        assert_eq!(masks.pairs().len(), 62);
        assert_eq!(masks.get(39001), 25041);
        assert_eq!(masks.get(25041), 0);
        assert_eq!(masks.get(30077), 0);
    }

    #[test]
    fn first_row_wins() {
        let masks = ItemMasks::from_bytes(b"1 10\n2 10\n").unwrap();
        assert_eq!(masks.pairs(), &[(10, 1)]);
    }

    #[test]
    fn blank_lines_skip() {
        let masks = ItemMasks::from_bytes(b"\n  \n1 10\r\n\n").unwrap();
        assert_eq!(masks.pairs(), &[(10, 1)]);
    }

    #[test]
    fn bad_line_refuses() {
        assert_eq!(
            ItemMasks::from_bytes(b"1 10\nnope\n"),
            Err(ItemMaskError::BadLine { line: 2 })
        );
        assert_eq!(
            ItemMasks::from_bytes(b"1 10 11\n"),
            Err(ItemMaskError::BadLine { line: 1 })
        );
        assert_eq!(
            ItemMasks::from_bytes(b"1 4294967296\n"),
            Err(ItemMaskError::BadLine { line: 1 })
        );
    }

    #[test]
    fn missing_file_loads_empty() {
        let masks = ItemMasks::load(Path::new("/nonexistent-229q")).expect("empty");
        assert_eq!(masks.pairs(), &[]);
    }
}
