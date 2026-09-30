//! The mob names the game process answers an NPC's name with: `LC_LOCALE_MOB_TEXT`.
//!
//! `LocaleService_LoadMobNameFile` (`G/locale_service.cpp:500-523`) reads one
//! `<base>/country/<get_locale>/mob_names.txt` per language through `locale_mob_init`
//! (`G/locale.cpp:188-206`). `CHARACTER::GetName` (`G/char.cpp:797-800`) answers a mob's name
//! with `LC_LOCALE_MOB_TEXT(race, LOCALE_YMIR)`, and `LOCALE_YMIR` reads the `LOCALE_DEFAULT`
//! file, which is `en`. That is the name every client is sent in an NPC's additional record,
//! whatever its own language, so this module reads that one file.
//!
//! This file is not the proto folder's `mob_names.txt`, which the DB server reads into
//! `szLocaleName` ([`crate::mob_proto`]). The two differ in how a repeated vnum resolves:
//! `locale_mob_init` inserts into a `std::map`, so the **first** row for a vnum wins here, where
//! the DB server's last row wins. The owner's `en` file repeats nine vnums, each with two
//! different names.
//!
//! The file is read with the CSV reader the game process shares with the DB server
//! ([`crate::csv_table`]). The header row is skipped, and a row with one column is skipped too
//! (`ColCount() > 1`). The key is `atoi` of the first column, stored as a `DWORD`. A vnum with no
//! row answers `NoName` (`locale_mob_find`, `G/locale.cpp:120-131`).
//!
//! Legacy logs a file it cannot open and answers `NoName` for every mob. This module refuses it
//! instead, the same Divergence as for `locale_string.txt`.
//!
//! The quest function `mob_name` (`G/questlua_global.cpp:673-695`) is the one reader that asks
//! in the player's own language (`LC_LOCALE_MOB_TEXT(vnum, GetLanguage())`), so
//! [`crate::mob_locale_names::MobNamesByLanguage`] reads every language's file, `LOCALE_YMIR`'s
//! from `en` as legacy does.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use common::enums::LOCALE_DEFAULT;

use crate::csv_table::{self, CsvError};
use crate::locale_string::{country_code, LOCALE_COUNT};

/// The file each language's folder holds.
pub const MOB_NAMES_FILE: &str = "mob_names.txt";

/// What `locale_mob_find` answers for a vnum the file does not name.
pub const NO_NAME: &[u8] = b"NoName";

/// The `LOCALE_YMIR` mob names, keyed by vnum.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MobLocaleNames {
    names: HashMap<u32, Vec<u8>>,
}

/// A `mob_names.txt` that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobLocaleNamesError {
    /// The file could not be read.
    Io {
        /// The file.
        path: PathBuf,
        /// The operating system's message.
        message: String,
    },
    /// The file did not parse as the tab-separated CSV the legacy reader accepts.
    Csv {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: CsvError,
    },
}

impl fmt::Display for MobLocaleNamesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "cannot read {}: {message}", path.display()),
            Self::Csv { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl Error for MobLocaleNamesError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { .. } => None,
            Self::Csv { source, .. } => Some(source),
        }
    }
}

impl MobLocaleNames {
    /// Read the `LOCALE_YMIR` file, `<country_dir>/en/mob_names.txt`.
    ///
    /// # Errors
    ///
    /// Returns [`MobLocaleNamesError`] when the file cannot be read or parsed.
    pub fn load(country_dir: &Path) -> Result<Self, MobLocaleNamesError> {
        Self::load_language(country_dir, LOCALE_DEFAULT as u8)
    }

    /// Read one language's file, `<country_dir>/<code>/mob_names.txt`.
    ///
    /// # Errors
    ///
    /// Returns [`MobLocaleNamesError`] when the file cannot be read or parsed.
    pub fn load_language(country_dir: &Path, language: u8) -> Result<Self, MobLocaleNamesError> {
        let path = country_dir
            .join(country_code(language))
            .join(MOB_NAMES_FILE);
        let file = std::fs::read(&path).map_err(|source| MobLocaleNamesError::Io {
            path: path.clone(),
            message: source.to_string(),
        })?;
        Self::parse(&file).map_err(|source| MobLocaleNamesError::Csv { path, source })
    }

    /// Read a `mob_names.txt` the way `locale_mob_init` reads it.
    ///
    /// # Errors
    ///
    /// Returns the [`CsvError`] of a file the legacy reader would misread.
    pub fn parse(file: &[u8]) -> Result<Self, CsvError> {
        let rows = csv_table::parse(file, b'\t', b'"')?;
        let mut names = HashMap::new();
        for row in rows.iter().skip(1) {
            let [vnum, name, ..] = row.as_slice() else {
                continue;
            };
            let vnum = u32::from_le_bytes(atoi(vnum).to_le_bytes());
            names.entry(vnum).or_insert_with(|| name.clone());
        }
        Ok(Self { names })
    }

    /// `locale_mob_find(vnum, LOCALE_YMIR)`: the name, or `NoName`.
    #[must_use]
    pub fn find(&self, vnum: u32) -> &[u8] {
        self.names.get(&vnum).map_or(NO_NAME, Vec::as_slice)
    }

    /// The number of vnums named.
    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Whether no vnum is named.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// Every language's mob names: legacy `localeMob`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MobNamesByLanguage {
    tables: Vec<MobLocaleNames>,
}

impl MobNamesByLanguage {
    /// Read the file of every language below `LOCALE_MAX_NUM`
    /// (`LocaleService_LoadMobNameFile`, `G/locale_service.cpp:502-522`).
    ///
    /// # Errors
    ///
    /// Returns [`MobLocaleNamesError`] when a file cannot be read or parsed.
    pub fn load(country_dir: &Path) -> Result<Self, MobLocaleNamesError> {
        let tables = (0..LOCALE_COUNT)
            .map(|language| MobLocaleNames::load_language(country_dir, language))
            .collect::<Result<_, _>>()?;
        Ok(Self { tables })
    }

    /// Build the table from each language's names, in language order. A language with no
    /// table reads English.
    #[must_use]
    pub fn from_tables(tables: Vec<MobLocaleNames>) -> Self {
        Self { tables }
    }

    /// `locale_mob_find(vnum, language)` (`G/locale.cpp:120-131`): the name, or `NoName`.
    ///
    /// Legacy tests `locale > LOCALE_MAX_NUM`, so language 12 would read past its array. Here
    /// every language past the last reads English, as the rest of them do in legacy.
    #[must_use]
    pub fn find(&self, vnum: u32, language: u8) -> &[u8] {
        self.tables
            .get(usize::from(language))
            .or_else(|| self.tables.get(usize::from(LOCALE_DEFAULT as u8)))
            .map_or(NO_NAME, |names| names.find(vnum))
    }
}

/// `atoi` with a 32-bit `int`: C whitespace, a sign, then decimal digits, saturating.
///
/// glibc's `atoi` is `(int) strtol`, and `long` is 32 bits on the legacy target.
fn atoi(field: &[u8]) -> i32 {
    let start = field
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'))
        .unwrap_or(field.len());
    let field = &field[start..];
    let (negative, rest) = match field.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, field),
    };
    let magnitude = rest
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .fold(0i64, |value, digit| {
            (value * 10 + i64::from(digit - b'0')).min(1 << 32)
        });
    let value = if negative { -magnitude } else { magnitude };
    i32::try_from(value).unwrap_or(if negative { i32::MIN } else { i32::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owners_country() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/locale/europe/country")
    }

    /// The owner's `en` file loads, byte-exact, with the NPCs of map 1 named.
    #[test]
    fn the_owners_en_file_names_the_npcs() {
        let names = MobLocaleNames::load(&owners_country()).expect("the owner's file reads");
        assert!(!names.is_empty());
        assert_eq!(names.find(20_016), b"Fierar");
        assert_eq!(names.find(9_001), b"Negustor Arme");
        assert_eq!(
            names.find(101),
            b"C\xe2ine s\xe3lbatic",
            "Latin bytes, not transcoded"
        );
        assert_eq!(names.find(1), NO_NAME);
    }

    /// A repeated vnum answers with its first row: `std::map::insert` keeps the first.
    #[test]
    fn the_first_row_for_a_vnum_wins() {
        let names = MobLocaleNames::load(&owners_country()).unwrap();
        assert_eq!(names.find(3_401), b"Tritonic Moray");
        let parsed = MobLocaleNames::parse(b"VNUM\tNAME\n5\tFirst\n5\tSecond\n6\tOther\n").unwrap();
        assert_eq!(parsed.find(5), b"First");
        assert_eq!(parsed.find(6), b"Other");
        assert_eq!(parsed.len(), 2);
    }

    /// The header is skipped, a one-column row is skipped, and the key is `atoi`'s.
    #[test]
    fn rows_are_read_as_locale_mob_init_reads_them() {
        let file = b"7\tHeader\n8\n 9x\tNine\n-1\tMinus\n99999999999\tBig\n";
        let names = MobLocaleNames::parse(file).unwrap();
        assert_eq!(names.find(7), NO_NAME, "the first row is the header");
        assert_eq!(names.find(8), NO_NAME);
        assert_eq!(names.find(9), b"Nine");
        assert_eq!(names.find(u32::MAX), b"Minus");
        assert_eq!(names.find(0x7fff_ffff), b"Big");
        assert_eq!(names.len(), 3);
    }

    /// `atoi` skips C's six whitespace bytes and nothing else, then reads one sign.
    #[test]
    fn atoi_skips_c_whitespace_and_reads_one_sign() {
        for space in [b' ', b'\t', b'\n', 0x0b, 0x0c, b'\r'] {
            assert_eq!(
                atoi(&[space, space, b'7']),
                7,
                "{space:#04x} is C whitespace"
            );
        }
        assert_eq!(atoi(b"\x087"), 0, "a backspace is not");
        assert_eq!(atoi(b"\x0e7"), 0, "nor is 0x0e");
        assert_eq!(atoi(b"+7"), 7);
        assert_eq!(atoi(b"-7"), -7);
        assert_eq!(atoi(b"+-7"), 0, "one sign only");
        assert_eq!(atoi(b"+2147483648"), i32::MAX);
        assert_eq!(atoi(b"-2147483649"), i32::MIN);
    }

    /// Every language's file loads, in language order.
    #[test]
    fn every_language_file_loads() {
        let names = MobNamesByLanguage::load(&owners_country()).unwrap();
        assert_eq!(names.tables.len(), usize::from(LOCALE_COUNT));
        for language in 0..LOCALE_COUNT {
            let own = MobLocaleNames::load_language(&owners_country(), language).unwrap();
            assert_eq!(names.tables[usize::from(language)], own, "{language}");
        }
        assert_eq!(names.find(20_016, 5), b"Fierar");
        assert_eq!(names.find(1, 5), NO_NAME);
        let error = MobNamesByLanguage::load(Path::new("/nonexistent/country")).unwrap_err();
        assert!(error.to_string().contains("/en/mob_names.txt"), "{error}");
    }

    /// A language reads its own table, and every language past the last reads English.
    #[test]
    fn a_language_past_the_last_reads_english() {
        let table = |name: &[u8]| {
            let mut file = b"VNUM\tNAME\n20016\t".to_vec();
            file.extend_from_slice(name);
            MobLocaleNames::parse(&file).unwrap()
        };
        let names = MobNamesByLanguage::from_tables(
            (0..LOCALE_COUNT)
                .map(|language| table(country_code(language).as_bytes()))
                .collect(),
        );
        assert_eq!(names.find(20_016, 0), b"en");
        assert_eq!(names.find(20_016, 5), country_code(5).as_bytes());
        assert_ne!(names.find(20_016, 5), names.find(20_016, 1));
        for language in [LOCALE_COUNT, LOCALE_COUNT + 1, u8::MAX] {
            assert_eq!(
                names.find(20_016, language),
                country_code(LOCALE_DEFAULT as u8).as_bytes(),
                "{language}"
            );
        }
        assert_eq!(MobNamesByLanguage::default().find(20_016, 1), NO_NAME);
        let short = MobNamesByLanguage::from_tables(vec![MobLocaleNames::default(), table(b"En")]);
        assert_eq!(short.find(20_016, 7), b"En");
        assert_eq!(short.find(20_016, 0), NO_NAME);
    }

    /// A missing file is refused and named.
    #[test]
    fn a_missing_file_is_refused() {
        let error = MobLocaleNames::load(Path::new("/nonexistent/country")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("/nonexistent/country/en/mob_names.txt"),
            "{error}"
        );
        assert!(error.source().is_none());
    }

    /// A file the CSV reader refuses is refused with its cause.
    #[test]
    fn an_unterminated_quote_is_refused() {
        assert_eq!(
            MobLocaleNames::parse(b"VNUM\tNAME\n5\t\"open\n"),
            Err(CsvError::UnterminatedQuote)
        );
    }
}
