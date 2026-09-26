//! The mob names a character Name may not take.
//!
//! Legacy `check_name_independent` (`server/server/game/locale_service.cpp:86-99`) refuses a
//! Name whose lowercase form is exactly a mob's `szLocaleName`, the key of
//! `CMobManager::m_map_pkMobByName` (`mob_manager.cpp:72`, looked up at `:143`).
//!
//! `szLocaleName` comes from the text protos (`ClientManagerBoot.cpp:246-292` and
//! `ProtoReader.cpp:755-769`): the header row of `mob_names.txt` is skipped and every other row
//! maps `atoi(column 0)` to column 1. Then every row of `mob_proto.txt` after its header gets the
//! name mapped to its `VNUM` (column 0, read by `strtoul`), or its own `NAME` (column 1) when
//! `mob_names.txt` has none. Both fields are 25-byte `char` arrays filled by `strlcpy`, so a name
//! keeps at most 24 bytes. The map is keyed by `int`, so a vnum is compared as its 32-bit pattern.
//!
//! Legacy reads column 1 with `std::vector::at`, which throws on a row with one column; such a
//! row is refused here.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::csv_table::{self, CsvError};

/// The bytes `strlcpy` keeps in `szName` and `szLocaleName` (`CHARACTER_NAME_MAX_LEN`).
const NAME_BYTES: usize = 24;

/// The mob locale names, as legacy `m_map_pkMobByName` holds them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MobNames {
    names: HashSet<Vec<u8>>,
}

/// Mob names that could not be read.
#[derive(Debug)]
pub enum MobNamesError {
    /// A file could not be read.
    Io {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// A file the legacy CSV reader would misread.
    Csv {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: CsvError,
    },
    /// A data row with fewer than two columns.
    ShortRow {
        /// The file.
        path: PathBuf,
        /// The 0-based row, counting the header.
        row: usize,
    },
}

impl fmt::Display for MobNamesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::Csv { path, source } => write!(f, "{}: {source}", path.display()),
            Self::ShortRow { path, row } => write!(
                f,
                "{}: row {row} has fewer than two columns",
                path.display()
            ),
        }
    }
}

impl Error for MobNamesError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Csv { source, .. } => Some(source),
            Self::ShortRow { .. } => None,
        }
    }
}

impl MobNames {
    /// Read `mob_proto.txt` and `mob_names.txt` from the proto folder.
    ///
    /// # Errors
    ///
    /// Returns [`MobNamesError`] when a file is missing, misread, or has a short row.
    pub fn load(proto_dir: &Path) -> Result<Self, MobNamesError> {
        let read = |name: &str| {
            let path = proto_dir.join(name);
            std::fs::read(&path)
                .map(|bytes| (path.clone(), bytes))
                .map_err(|source| MobNamesError::Io { path, source })
        };
        let (proto_path, proto) = read("mob_proto.txt")?;
        let (names_path, names) = read("mob_names.txt")?;
        Self::from_files(&proto_path, &proto, &names_path, &names)
    }

    /// Build the names from the two files' bytes; the paths only name them in errors.
    ///
    /// # Errors
    ///
    /// Returns [`MobNamesError`] when a file is misread or has a short row.
    pub fn from_files(
        proto_path: &Path,
        proto: &[u8],
        names_path: &Path,
        names: &[u8],
    ) -> Result<Self, MobNamesError> {
        let mut locale = HashMap::new();
        for (vnum, name) in data_rows(names_path, names)? {
            locale.insert(atoi(&vnum), name);
        }
        let names = data_rows(proto_path, proto)?
            .into_iter()
            .map(|(vnum, own)| {
                let mut name = locale.get(&strtoul(&vnum)).cloned().unwrap_or(own);
                name.truncate(NAME_BYTES);
                name
            })
            .collect();
        Ok(Self { names })
    }

    /// Whether legacy refuses a Name because a mob is called by its lowercase form.
    #[must_use]
    pub fn refuses(&self, name: &[u8]) -> bool {
        self.names.contains(&name.to_ascii_lowercase())
    }
}

/// A data row's vnum and name columns.
type VnumName = (Vec<u8>, Vec<u8>);

/// The first two columns of every row after the header.
fn data_rows(path: &Path, bytes: &[u8]) -> Result<Vec<VnumName>, MobNamesError> {
    let rows = csv_table::parse(bytes, b'\t', b'"').map_err(|source| MobNamesError::Csv {
        path: path.to_path_buf(),
        source,
    })?;
    rows.into_iter()
        .enumerate()
        .skip(1)
        .map(|(row, mut fields)| {
            if fields.len() < 2 {
                return Err(MobNamesError::ShortRow {
                    path: path.to_path_buf(),
                    row,
                });
            }
            fields.truncate(2);
            let name = fields.pop().unwrap_or_default();
            let vnum = fields.pop().unwrap_or_default();
            Ok((vnum, name))
        })
        .collect()
}

/// C `atoi` on i686: `strtol` into a 32-bit `long`, saturating on overflow, then read as the
/// map's `int` key.
fn atoi(field: &[u8]) -> u32 {
    let (negative, magnitude) = c_integer(field);
    let value = if negative {
        magnitude.map_or(i64::from(i32::MIN), |m| (-m).max(i64::from(i32::MIN)))
    } else {
        magnitude.map_or(i64::from(i32::MAX), |m| m.min(i64::from(i32::MAX)))
    };
    let value = i32::try_from(value).unwrap_or_default();
    u32::from_ne_bytes(value.to_ne_bytes())
}

/// C `strtoul` on i686: a 32-bit `unsigned long`, `ULONG_MAX` on overflow, and a leading `-`
/// negating the value modulo 2^32.
fn strtoul(field: &[u8]) -> u32 {
    let (negative, magnitude) = c_integer(field);
    match magnitude.and_then(|m| u32::try_from(m).ok()) {
        None => u32::MAX,
        Some(value) if negative => value.wrapping_neg(),
        Some(value) => value,
    }
}

/// Skip C whitespace, read an optional sign and the decimal digits after it. The magnitude is
/// `None` when it passes `u32::MAX`, which overflows both callers.
fn c_integer(field: &[u8]) -> (bool, Option<i64>) {
    let mut rest = field;
    while let Some((first, tail)) = rest.split_first() {
        if !matches!(first, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r') {
            break;
        }
        rest = tail;
    }
    let negative = rest.first() == Some(&b'-');
    if matches!(rest.first(), Some(b'-' | b'+')) {
        rest = &rest[1..];
    }
    let mut magnitude = Some(0_i64);
    for digit in rest.iter().take_while(|byte| byte.is_ascii_digit()) {
        magnitude = magnitude
            .map(|m| m * 10 + i64::from(digit - b'0'))
            .filter(|m| *m <= i64::from(u32::MAX));
    }
    (negative, magnitude)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(proto: &[u8], names: &[u8]) -> Result<MobNames, MobNamesError> {
        MobNames::from_files(
            Path::new("mob_proto.txt"),
            proto,
            Path::new("mob_names.txt"),
            names,
        )
    }

    #[test]
    fn a_locale_name_replaces_the_proto_name() {
        let names = names(
            b"VNUM\tNAME\n101\tkorean\n102\twolf\n",
            b"VNUM\tLOCALE_NAME\n101\tpony\n999\tghost\n",
        )
        .unwrap();
        assert!(names.refuses(b"pony"));
        assert!(
            names.refuses(b"Pony"),
            "the Name is lowercased before the lookup"
        );
        assert!(
            names.refuses(b"WOLF"),
            "a proto row without a locale name keeps its own"
        );
        assert!(
            !names.refuses(b"korean"),
            "the replaced proto name is not kept"
        );
        assert!(
            !names.refuses(b"ghost"),
            "a locale name without a proto row is not a mob"
        );
        assert!(!names.refuses(b"VNUM"), "the header rows are skipped");
        assert!(!names.refuses(b"NAME"));
        assert!(!names.refuses(b"LOCALE_NAME"));
    }

    #[test]
    fn a_stored_name_that_is_not_lowercase_never_matches() {
        let names = names(b"V\tN\n1\tx\n", b"V\tN\n1\tLup\n").unwrap();
        assert!(!names.refuses(b"lup"));
        assert!(!names.refuses(b"Lup"));
    }

    #[test]
    fn a_name_keeps_24_bytes() {
        let long = b"abcdefghijklmnopqrstuvwxyz";
        let mut file = b"V\tN\n7\t".to_vec();
        file.extend_from_slice(long);
        let names = names(b"V\tN\n7\tx\n", &file).unwrap();
        assert!(names.refuses(&long[..24]));
        assert!(!names.refuses(&long[..23]));
        assert!(!names.refuses(long));
    }

    #[test]
    fn vnums_are_read_as_c_reads_them() {
        let names = names(
            b"V\tN\n 12abc\tx\n4294967295\ty\n-1\tz\n99999999999\tw\n",
            b"V\tN\n12\ttwelve\n-1\tminus\n",
        )
        .unwrap();
        assert!(names.refuses(b"twelve"));
        assert!(
            names.refuses(b"minus"),
            "strtoul(\"4294967295\") is atoi(\"-1\")"
        );
        assert!(!names.refuses(b"y"));
        assert!(
            !names.refuses(b"z"),
            "strtoul(\"-1\") wraps to the same key"
        );
        assert!(
            !names.refuses(b"w"),
            "an overflowing vnum saturates to the -1 key"
        );
        assert_eq!(
            atoi(b"99999999999"),
            u32::from_ne_bytes(i32::MAX.to_ne_bytes())
        );
        assert_eq!(
            atoi(b"-99999999999"),
            u32::from_ne_bytes(i32::MIN.to_ne_bytes())
        );
        assert_eq!(atoi(b"x"), 0);
        assert_eq!(strtoul(b"99999999999"), u32::MAX);
        assert_eq!(strtoul(b"-2"), u32::MAX - 1);
    }

    #[test]
    fn a_short_row_is_refused() {
        let error = names(b"V\tN\n1\tx\n", b"V\tN\n1\n").unwrap_err();
        assert!(
            matches!(error, MobNamesError::ShortRow { row: 1, .. }),
            "{error}"
        );
    }

    #[test]
    fn the_owner_files_reserve_their_lowercase_mob_names() {
        let proto = Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        let names = MobNames::load(&proto).unwrap();
        assert!(names.refuses(b"blackpony"));
        assert!(names.refuses(b"Teowahdan"));
        assert!(
            !names.refuses(b"Lup"),
            "only a lowercase locale name can match"
        );
        assert!(!names.refuses(b"Alpha"));
    }
}
