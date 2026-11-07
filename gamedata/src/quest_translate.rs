//! The quest translations: `locale_quest.txt` and the `LC_LOCALE_QUEST_TEXT` lookup.
//!
//! `LocaleService_LoadQuestTranslateFile` (`G/locale_service.cpp:462-479`) fills each language
//! from `<base>/country/<get_locale>/locale_quest.txt`, where `<base>` is `locale/europe`.
//! `LOCALE_YMIR` loads the `en` file. `locale_quest_translate_init` (`G/locale.cpp:146-166`)
//! reads the file with the tab-separated CSV reader, skips the header row, and inserts
//! `atoi` of the first column to the second; the first pair for a key wins. A file legacy
//! cannot load leaves that language empty.
//!
//! `locale_quest_translate_find` (`G/locale.cpp:90-103`) answers `en`'s table for a language
//! past `LOCALE_MAX_NUM`, and `NoName` for a key no row names. `SendLocaleNotice` reads it
//! when the notice's first part is all digits (`G/cmd_gm.cpp:1265-1276`).

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use super::csv_table;
use super::locale_string::{country_code, LOCALE_COUNT};

/// The file each language's folder holds.
pub const LOCALE_QUEST_FILE: &str = "locale_quest.txt";

/// What a missed key answers.
pub const QUEST_TRANSLATE_MISS: &[u8] = b"NoName";

/// Every language's quest table: legacy `localeQuestTranslate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestTranslate {
    tables: Vec<BTreeMap<u32, Vec<u8>>>,
}

impl Default for QuestTranslate {
    /// Twelve empty tables, so a build without the files answers `NoName`.
    fn default() -> Self {
        Self {
            tables: vec![BTreeMap::new(); usize::from(LOCALE_COUNT)],
        }
    }
}

/// A `locale_quest.txt` that could not be read.
#[derive(Debug)]
pub struct QuestTranslateError {
    /// The file.
    pub path: PathBuf,
    /// The cause.
    pub source: QuestTranslateCause,
}

/// Why a `locale_quest.txt` could not be read.
#[derive(Debug)]
pub enum QuestTranslateCause {
    /// The file could not be opened.
    Io(std::io::Error),
    /// The file misreads under the CSV reader.
    Csv(csv_table::CsvError),
}

impl fmt::Display for QuestTranslateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.source {
            QuestTranslateCause::Io(source) => {
                write!(f, "cannot read {}: {source}", self.path.display())
            }
            QuestTranslateCause::Csv(source) => {
                write!(f, "{} misreads: {source}", self.path.display())
            }
        }
    }
}

impl Error for QuestTranslateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.source {
            QuestTranslateCause::Io(source) => Some(source),
            QuestTranslateCause::Csv(source) => Some(source),
        }
    }
}

/// `atoi` of a key column: the leading digits, or 0 with none.
fn atoi(key: &[u8]) -> u32 {
    let mut value: u32 = 0;
    for &byte in key {
        let Some(digit) = byte.checked_sub(b'0').filter(|digit| *digit < 10) else {
            break;
        };
        value = value.saturating_mul(10).saturating_add(u32::from(digit));
    }
    value
}

/// The byte offset of the row a quoted field still open at the end of the file
/// started on: the line scan of [`csv_table::parse`], tracking only the quote, the
/// line starts, and the row starts. A file with no unterminated quote answers its
/// own length.
fn unfinished_row_start(file: &[u8]) -> usize {
    let mut quoted = false;
    let mut row_start = 0;
    let mut row_open = false;
    let mut offset = 0;
    while offset < file.len() {
        let end = file[offset..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(file.len(), |at| offset + at);
        let raw = &file[offset..end];
        let raw = &raw[..raw.iter().position(|&byte| byte == 0).unwrap_or(raw.len())];
        let line = trim(raw);
        if !line.is_empty() && (quoted || line[0] != b'#') {
            if !quoted && !row_open {
                row_start = offset;
                row_open = true;
            }
            let mut cursor = 0;
            while cursor < line.len() {
                if quoted {
                    if line[cursor] == b'"' {
                        if line.get(cursor + 1) == Some(&b'"') {
                            cursor += 1;
                        } else {
                            quoted = false;
                        }
                    }
                } else if line[cursor] == b'"' {
                    quoted = true;
                }
                cursor += 1;
            }
            if !quoted {
                row_open = false;
            }
        }
        offset = end + 1;
    }
    if quoted {
        row_start
    } else {
        file.len()
    }
}

/// The line without the spaces, tabs and CRs `Trim` removes from both ends.
fn trim(line: &[u8]) -> &[u8] {
    let mut line = line;
    while line
        .last()
        .is_some_and(|byte| *byte == b' ' || *byte == b'\t' || *byte == b'\r')
    {
        line = &line[..line.len() - 1];
    }
    while line
        .first()
        .is_some_and(|byte| *byte == b' ' || *byte == b'\t' || *byte == b'\r')
    {
        line = &line[1..];
    }
    line
}

impl QuestTranslate {
    /// Read every language's `locale_quest.txt` from the `country` folder.
    ///
    /// `LOCALE_YMIR`'s table loads the `en` file, as legacy loads it. A quoted field
    /// still open at the end of the file drops its unfinished row, as legacy's reader
    /// does: the owner's `pl` file opens one at line 8018 (an unclosed `...` of
    /// dialog), so its table ends at key 7258, and the `cz` file's stray quote at line
    /// 10842 never closes cleanly (line 11154 opens another), so its table ends at key
    /// 9825.
    ///
    /// # Errors
    ///
    /// Returns [`QuestTranslateError`] when a file cannot be read or misreads.
    pub fn load(country_dir: &Path) -> Result<Self, QuestTranslateError> {
        let mut tables: Vec<BTreeMap<u32, Vec<u8>>> =
            vec![BTreeMap::new(); usize::from(LOCALE_COUNT)];
        for language in 0..LOCALE_COUNT {
            let code = if language == 0 {
                "en"
            } else {
                country_code(language)
            };
            let path = country_dir.join(code).join(LOCALE_QUEST_FILE);
            let file = std::fs::read(&path).map_err(|source| QuestTranslateError {
                path: path.clone(),
                source: QuestTranslateCause::Io(source),
            })?;
            let rows = match csv_table::parse(&file, b'\t', b'"') {
                Ok(rows) => rows,
                Err(csv_table::CsvError::UnterminatedQuote) => {
                    let head = &file[..unfinished_row_start(&file)];
                    csv_table::parse(head, b'\t', b'"').map_err(|source| QuestTranslateError {
                        path: path.clone(),
                        source: QuestTranslateCause::Csv(source),
                    })?
                }
                Err(source) => {
                    return Err(QuestTranslateError {
                        path: path.clone(),
                        source: QuestTranslateCause::Csv(source),
                    });
                }
            };
            let table = &mut tables[usize::from(language)];
            for row in rows.iter().skip(1) {
                if row.len() > 1 {
                    table.entry(atoi(&row[0])).or_insert_with(|| row[1].clone());
                }
            }
        }
        Ok(Self { tables })
    }

    /// The translation of `key` in `language`: `en`'s table answers past the last
    /// language, and a key no row names answers [`QUEST_TRANSLATE_MISS`].
    #[must_use]
    pub fn find(&self, key: u32, language: u8) -> &[u8] {
        let table = self
            .tables
            .get(usize::from(language))
            .unwrap_or(&self.tables[1]);
        table.get(&key).map_or(QUEST_TRANSLATE_MISS, Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn owners_country() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/locale/europe/country")
    }

    #[test]
    fn every_language_of_the_owners_data_reads_as_legacy_reads_it() {
        let tables = QuestTranslate::load(&owners_country()).expect("the owner's files read");
        // Pairs, then the bytes of every value, as an independent reading of the same
        // files counted them. Duplicated keys keep their first row; the `pl` file's
        // unclosed dialog drops its rows from 8018 on, and the `cz` file's stray quotes
        // merge rows 10842-11154 and drop the rest.
        let expected = [
            ("en", 11161, 850_728),
            ("pt", 10998, 876_236),
            ("es", 11006, 878_840),
            ("fr", 10997, 935_977),
            ("de", 11021, 938_734),
            ("ro", 8717, 836_461),
            ("pl", 7861, 583_031),
            ("it", 10999, 887_388),
            ("cz", 10682, 738_578),
            ("hu", 11011, 817_181),
            ("tr", 7305, 791_830),
        ];
        for (language, (code, pairs, value_bytes)) in (1..).zip(expected) {
            assert_eq!(country_code(language), code);
            let table = &tables.tables[usize::from(language)];
            assert_eq!(table.len(), pairs, "{code}");
            let values: usize = table.values().map(Vec::len).sum();
            assert_eq!(values, value_bytes, "{code}");
        }
        assert_eq!(tables.tables[0], tables.tables[1], "LOCALE_YMIR loads en");
    }

    #[test]
    fn the_owners_tables_answer_their_golden_keys() {
        let tables = QuestTranslate::load(&owners_country()).expect("the owner's files read");
        assert_eq!(tables.find(1, 1), b"A special cloth");
        assert!(tables.find(2, 1).starts_with(b"I've already told you"));
        assert_eq!(tables.find(1, 0), b"A special cloth", "LOCALE_YMIR");
        assert_eq!(tables.find(1, 200), b"A special cloth", "past the last");
        assert_eq!(tables.find(999_999_999, 1), b"NoName", "a key no row names");
        assert!(
            tables.find(7258, 7) != b"NoName",
            "the pl row before the unclosed dialog"
        );
        assert_eq!(
            tables.find(7259, 7),
            b"NoName",
            "the pl unfinished row is dropped"
        );
        assert!(
            tables
                .find(9825, 9)
                .starts_with(b"Ty se pot\xf8ebuje\x9a nejprve p\xf8ipravit? Fajn, ale[ENTER]"),
            "the cz row before the stray quote"
        );
        assert_eq!(
            tables.find(9826, 9),
            b"NoName",
            "the stray quote's unfinished row is dropped"
        );
    }

    #[test]
    fn an_unclosed_quote_keeps_the_rows_before_it() {
        let dir = std::env::temp_dir().join("prodomo-quest-translate-probe");
        for code in [
            "en", "pt", "es", "fr", "de", "ro", "pl", "it", "cz", "hu", "tr",
        ] {
            let folder = dir.join(code);
            std::fs::create_dir_all(&folder).expect("a scratch folder");
            let body = if code == "en" {
                b"KEY\tSTRING\n1\tone\n2\t\"two\n".as_slice()
            } else {
                b"KEY\tSTRING\n1\tone\n".as_slice()
            };
            std::fs::write(folder.join(LOCALE_QUEST_FILE), body).expect("a scratch file");
        }
        let tables = QuestTranslate::load(&dir).expect("the unfinished row drops");
        assert_eq!(tables.find(1, 1), b"one");
        assert_eq!(tables.find(2, 1), b"NoName");
        std::fs::remove_dir_all(&dir).expect("the scratch folder removes");
    }
}
