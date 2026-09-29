//! The locale strings: `locale_string.txt` and the `LC_LOCALE_TEXT` lookup.
//!
//! `__MULTI_LANGUAGE_SYSTEM__` is defined (`common/prodomodefines.h:133`), so legacy keeps one
//! table per language (`localeString[LOCALE_MAX_NUM]`, `G/locale.cpp:18`).
//! `LocaleService_LoadLocaleStringFile` (`G/locale_service.cpp:418-455`) fills each language
//! from `<base>/country/<get_locale>/locale_string.txt`, where `<base>` is `locale/europe`.
//! The auth server loads none. The `locale_string.txt` beside `country` is not read.
//!
//! # The file
//!
//! `locale_init` (`G/locale.cpp:351-440`) reads the file into a NUL-terminated buffer. A pair is
//! two strings, each ended by a `;` outside quotes (`quote_find_end`, `:260-288`). Each is
//! converted by `locale_convert` (`:290-346`), which keeps the bytes between its quotes, turns
//! `\n` into a newline, and keeps the backslash of `\"`. Spaces, CRs and LFs after each `;` are
//! skipped, but not tabs. A line that does not start with `"` is skipped whole.
//!
//! A pair legacy cannot read ends the file, and the rest of it is dropped:
//! - a string with no `;` after it;
//! - a value that does not start with `"` (legacy logs "invalid format");
//! - a string with nothing between its quotes, because `locale_convert` answers `NULL`;
//! - a NUL byte, where the C string ends.
//!
//! The first pair for a key wins (`locale_add`, `:61-69`). A key and a value are C strings, so
//! each ends at its first NUL.
//!
//! The owner's eleven files are CP949 keys with Latin values and CRLF line ends. The bytes are
//! kept as they are; no encoding is assumed.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use common::enums::{ELocale, LOCALE_DEFAULT};

/// The file each language's folder holds.
pub const LOCALE_STRING_FILE: &str = "locale_string.txt";

/// `LOCALE_MAX_NUM`: one table for each language, `LOCALE_YMIR` included.
pub const LOCALE_COUNT: u8 = ELocale::MaxNum as u8;

/// The folder under `country` for a language: legacy `get_locale` (`G/locale.cpp:30-59`).
///
/// `LOCALE_YMIR` and anything past `LOCALE_TR` answer `en`, as legacy's `default:` does.
#[must_use]
pub const fn country_code(language: u8) -> &'static str {
    match language {
        2 => "pt",
        3 => "es",
        4 => "fr",
        5 => "de",
        6 => "ro",
        7 => "pl",
        8 => "it",
        9 => "cz",
        10 => "hu",
        11 => "tr",
        _ => "en",
    }
}

/// Why a file's read ended before the end of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// A string with no `;` outside quotes after it.
    Unterminated,
    /// A value that does not start with `"`: legacy's "invalid format".
    InvalidFormat,
    /// A string with nothing between its quotes.
    Empty,
    /// A NUL byte.
    Nul,
}

/// How a file's read ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Ending {
    /// The whole file was read.
    #[default]
    Complete,
    /// The read stopped at this byte offset, and the rest of the file was dropped. The offset
    /// is where the unreadable pair starts, or the NUL.
    Stopped {
        /// The byte offset.
        at: usize,
        /// Why the read stopped.
        cause: Stop,
    },
}

/// One language's pairs: legacy `localeString[locale]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanguageTable {
    strings: HashMap<Vec<u8>, Vec<u8>>,
    pairs: usize,
    ending: Ending,
}

impl LanguageTable {
    /// Read one `locale_string.txt` as `locale_init` does.
    #[must_use]
    pub fn parse(file: &[u8]) -> Self {
        let mut table = Self::default();
        let mut at = 0;
        loop {
            if byte(file, at) == b'"' {
                let start = at;
                match read_pair(file, &mut at) {
                    Ok((key, value)) => {
                        table.pairs += 1;
                        table.strings.entry(key).or_insert(value);
                    }
                    Err(cause) => {
                        table.ending = Ending::Stopped { at: start, cause };
                        return table;
                    }
                }
            } else {
                // `strchr(tmp, '\n')`, which stops at the NUL that ends the C string.
                let rest = file.get(at..).unwrap_or_default();
                match rest.iter().position(|&next| next == b'\n' || next == 0) {
                    Some(offset) if rest[offset] == b'\n' => at += offset + 1,
                    Some(offset) => at += offset,
                    None => at = file.len(),
                }
            }
            if byte(file, at) == 0 {
                if at < file.len() {
                    table.ending = Ending::Stopped {
                        at,
                        cause: Stop::Nul,
                    };
                }
                return table;
            }
        }
    }

    /// The value for a key, if the file had one.
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.strings.get(key).map(Vec::as_slice)
    }

    /// How many keys the table holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Whether the table holds no key.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }

    /// How many pairs were read, a repeated key included.
    #[must_use]
    pub const fn pairs(&self) -> usize {
        self.pairs
    }

    /// How the read ended.
    #[must_use]
    pub const fn ending(&self) -> Ending {
        self.ending
    }
}

/// The byte at `at`, or the NUL that ends the buffer.
fn byte(file: &[u8], at: usize) -> u8 {
    file.get(at).copied().unwrap_or(0)
}

/// The two strings of the pair that starts at `*at`, which is a `"`, as the `for` loop of
/// `locale_init` reads them. `*at` is left after the whitespace that follows the pair.
fn read_pair(file: &[u8], at: &mut usize) -> Result<(Vec<u8>, Vec<u8>), Stop> {
    let mut strings: [Option<Vec<u8>>; 2] = [None, None];
    for (index, string) in strings.iter_mut().enumerate() {
        let end = quote_find_end(file, *at).ok_or(Stop::Unterminated)?;
        *string = convert(file, *at, end - *at);
        *at = end + 1;
        while matches!(byte(file, *at), b'\n' | b'\r' | b' ') {
            *at += 1;
        }
        // Legacy tests the key only after the value, so an empty key still reads a value.
        if index == 0 && byte(file, *at) != b'"' {
            return Err(Stop::InvalidFormat);
        }
    }
    let [Some(key), Some(value)] = strings else {
        return Err(Stop::Empty);
    };
    Ok((key, value))
}

/// `quote_find_end`: the first `;` outside quotes at or after `from`.
///
/// Inside quotes, `\"` is skipped whole. Any other backslash is skipped alone, so the byte after
/// it is read as usual.
fn quote_find_end(file: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    let mut quoted = false;
    while byte(file, at) != 0 {
        let current = byte(file, at);
        if quoted && current == b'\\' && byte(file, at + 1) != 0 {
            if byte(file, at + 1) == b'"' {
                at += 2;
                continue;
            }
        } else if current == b'"' {
            quoted = !quoted;
        } else if !quoted && current == b';' {
            return Some(at);
        }
        at += 1;
    }
    None
}

/// `locale_convert`: the string in the `len` bytes from `from`, or `None` when it wrote nothing.
///
/// `last` is the last byte written, not the last byte read, so a `"` after a written backslash
/// is kept, and so is the backslash. `\n` becomes a newline and uses two bytes for one step of
/// the count; the `;` that ends the string stops the loop before it can read further.
fn convert(file: &[u8], from: usize, len: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(len);
    let mut quoted = false;
    let mut last = 0;
    let mut at = from;
    for _ in 0..len {
        let current = byte(file, at);
        let keep = match current {
            b'"' if last == b'\\' => true,
            b'"' => {
                quoted = !quoted;
                false
            }
            b';' if last != b'\\' && !quoted => break,
            b';' => true,
            _ => quoted,
        };
        if keep {
            if current == b'\\' && byte(file, at + 1) == b'n' {
                out.push(b'\n');
                at += 1;
                last = b'\n';
            } else {
                out.push(current);
                last = current;
            }
        }
        at += 1;
    }
    if out.is_empty() {
        return None;
    }
    // The caller reads the result as a C string.
    if let Some(nul) = out.iter().position(|&written| written == 0) {
        out.truncate(nul);
    }
    Some(out)
}

/// Every language's table: legacy `localeString`.
///
/// The default holds no pair, so every text is answered unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocaleStrings {
    tables: [LanguageTable; LOCALE_COUNT as usize],
}

/// A `locale_string.txt` that could not be read.
#[derive(Debug)]
pub struct LocaleStringsError {
    /// The file.
    pub path: PathBuf,
    /// The cause.
    pub source: std::io::Error,
}

impl fmt::Display for LocaleStringsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot read {}: {}", self.path.display(), self.source)
    }
}

impl Error for LocaleStringsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

impl LocaleStrings {
    /// Read every language's `locale_string.txt` from the `country` folder.
    ///
    /// `LOCALE_YMIR`'s table is left empty. Legacy fills it from the `en` file, but
    /// `locale_find` never reads it.
    ///
    /// # Errors
    ///
    /// Returns [`LocaleStringsError`] when a file cannot be read. Legacy's `fopen` failure
    /// leaves that language with no table, so every line goes out untranslated.
    pub fn load(country_dir: &Path) -> Result<Self, LocaleStringsError> {
        let mut strings = Self::default();
        for language in 1..LOCALE_COUNT {
            let path = country_dir
                .join(country_code(language))
                .join(LOCALE_STRING_FILE);
            let file = std::fs::read(&path).map_err(|source| LocaleStringsError {
                path: path.clone(),
                source,
            })?;
            strings.tables[usize::from(language)] = LanguageTable::parse(&file);
        }
        Ok(strings)
    }

    /// Replace one language's table. A language past the last is ignored.
    #[must_use]
    pub fn with_table(mut self, language: u8, table: LanguageTable) -> Self {
        if let Some(slot) = self.tables.get_mut(usize::from(language)) {
            *slot = table;
        }
        self
    }

    /// One language's table.
    #[must_use]
    pub fn table(&self, language: u8) -> Option<&LanguageTable> {
        self.tables.get(usize::from(language))
    }

    /// `LC_LOCALE_TEXT(text, language)`: `locale_find` (`G/locale.cpp:71-88`).
    ///
    /// An empty text and `LOCALE_YMIR` answer the text itself, and so does a text the
    /// language's table lacks. A language past the last reads the English table.
    ///
    /// Legacy tests `locale > LOCALE_MAX_NUM`, so language 12 would read past the table array.
    /// The auth login refuses 12, so no descriptor holds it, and here it reads English too.
    #[must_use]
    pub fn find<'a>(&'a self, text: &'a [u8], language: u8) -> &'a [u8] {
        if text.is_empty() || language == ELocale::Ymir as u8 {
            return text;
        }
        let language = if language < LOCALE_COUNT {
            language
        } else {
            LOCALE_DEFAULT as u8
        };
        self.tables[usize::from(language)].get(text).unwrap_or(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(table: &LanguageTable) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut pairs: Vec<_> = table
            .strings
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        pairs.sort();
        pairs
    }

    fn pair(key: &[u8], value: &[u8]) -> (Vec<u8>, Vec<u8>) {
        (key.to_vec(), value.to_vec())
    }

    #[test]
    fn a_pair_is_two_quoted_strings_each_ended_by_a_semicolon() {
        let table = LanguageTable::parse(b"\"key\";\r\n\"value\";\r\n\"k2\"; \"v2\";");
        assert_eq!(pairs(&table), [pair(b"k2", b"v2"), pair(b"key", b"value")]);
        assert_eq!(table.pairs(), 2);
        assert_eq!(table.ending(), Ending::Complete);
        assert_eq!(table.get(b"key"), Some(&b"value"[..]));
        assert_eq!(table.get(b"value"), None);
    }

    #[test]
    fn a_line_that_does_not_start_with_a_quote_is_skipped_whole() {
        let file = b"# \"no\";\"pair\";\n\t\"tab\";\"skipped\";\n\"a\";\"b\";\nplain";
        let table = LanguageTable::parse(file);
        assert_eq!(pairs(&table), [pair(b"a", b"b")]);
        assert_eq!(table.ending(), Ending::Complete);
    }

    #[test]
    fn the_first_pair_for_a_key_wins() {
        let table = LanguageTable::parse(b"\"a\";\"first\";\n\"a\";\"second\";\n");
        assert_eq!(pairs(&table), [pair(b"a", b"first")]);
        assert_eq!(table.pairs(), 2);
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn a_string_keeps_what_is_inside_its_quotes() {
        let table = LanguageTable::parse(
            b"\"line\\none\";\"x\\n\";\n\"a;b\";\"c\"d\"e\";\n\"q\\\"t\";\"back\\slash\";\n",
        );
        assert_eq!(
            pairs(&table),
            [
                // Between the quotes only: `d` is outside them.
                pair(b"a;b", b"ce"),
                // `\n` is a newline; any other backslash is kept.
                pair(b"line\none", b"x\n"),
                // `\"` keeps its backslash and its quote.
                pair(b"q\\\"t", b"back\\slash"),
            ]
        );
    }

    #[test]
    fn an_unreadable_pair_ends_the_file() {
        let rest = b"\"after\";\"dropped\";\n";
        for (head, cause) in [
            (&b"\"k\";\"\";\n"[..], Stop::Empty),
            (&b"\"\";\"v\";\n"[..], Stop::Empty),
            (&b"\"k\";\n\t\"v\";\n"[..], Stop::InvalidFormat),
            (&b"\"k\";v;\n"[..], Stop::InvalidFormat),
        ] {
            let mut file = b"\"a\";\"b\";\n".to_vec();
            file.extend_from_slice(head);
            file.extend_from_slice(rest);
            let table = LanguageTable::parse(&file);
            assert_eq!(pairs(&table), [pair(b"a", b"b")], "{head:?}");
            assert_eq!(table.ending(), Ending::Stopped { at: 9, cause });
        }
        let table = LanguageTable::parse(b"\"a\";\"b\";\n\"k\";\"no end\"\n");
        assert_eq!(pairs(&table), [pair(b"a", b"b")]);
        assert_eq!(
            table.ending(),
            Ending::Stopped {
                at: 9,
                cause: Stop::Unterminated
            }
        );
    }

    #[test]
    fn a_nul_ends_the_file() {
        let table = LanguageTable::parse(b"\"a\";\"b\";\n\0\"c\";\"d\";\n");
        assert_eq!(pairs(&table), [pair(b"a", b"b")]);
        assert_eq!(
            table.ending(),
            Ending::Stopped {
                at: 9,
                cause: Stop::Nul
            }
        );
        let table = LanguageTable::parse(b"# note\0\n\"c\";\"d\";\n");
        assert!(table.is_empty());
        assert_eq!(
            table.ending(),
            Ending::Stopped {
                at: 6,
                cause: Stop::Nul
            }
        );
        // A NUL inside the quotes ends the `;` search, so the string is unterminated.
        let table = LanguageTable::parse(b"\"a\0\";\"b\";");
        assert_eq!(
            table.ending(),
            Ending::Stopped {
                at: 0,
                cause: Stop::Unterminated
            }
        );
    }

    #[test]
    fn a_file_with_no_pair_is_an_empty_table() {
        for file in [&b""[..], b"\n", b"no pair here"] {
            let table = LanguageTable::parse(file);
            assert!(table.is_empty());
            assert_eq!(table.ending(), Ending::Complete);
        }
    }

    #[test]
    fn get_locale_names_each_language_folder() {
        let codes: Vec<_> = (0..=LOCALE_COUNT).map(country_code).collect();
        assert_eq!(
            codes,
            ["en", "en", "pt", "es", "fr", "de", "ro", "pl", "it", "cz", "hu", "tr", "en"]
        );
    }

    #[test]
    fn find_answers_the_text_itself_unless_the_language_translates_it() {
        let english = LanguageTable::parse(b"\"k\";\"english\";");
        let german = LanguageTable::parse(b"\"k\";\"deutsch\";");
        // Legacy fills table 0, and `locale_find` never reads it (`G/locale.cpp:73`).
        let strings = LocaleStrings::default()
            .with_table(0, LanguageTable::parse(b"\"k\";\"ymir\";"))
            .with_table(1, english)
            .with_table(5, german)
            .with_table(LOCALE_COUNT, LanguageTable::parse(b"\"k\";\"no\";"));
        assert_eq!(strings.find(b"k", 5), b"deutsch");
        assert_eq!(strings.find(b"k", 1), b"english");
        assert_eq!(strings.find(b"k", 0), b"k", "LOCALE_YMIR");
        assert_eq!(strings.find(b"k", 2), b"k", "a table without the key");
        assert_eq!(strings.find(b"other", 5), b"other");
        assert_eq!(strings.find(b"", 5), b"");
        for language in [LOCALE_COUNT, 200, u8::MAX] {
            assert_eq!(strings.find(b"k", language), b"english");
        }
        assert!(strings.table(LOCALE_COUNT).is_none());
    }

    fn owners_country() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/locale/europe/country")
    }

    #[test]
    fn every_language_of_the_owners_data_reads_to_its_end() {
        let strings = LocaleStrings::load(&owners_country()).expect("the owner's files read");
        assert!(strings.table(0).is_some_and(LanguageTable::is_empty));
        // Pairs, then the bytes of every key and of every value, as an independent reading of
        // the same files counted them. Every pair has its own key.
        let expected = [
            ("en", 757, 26_564, 32_048),
            ("pt", 758, 26_572, 33_622),
            ("es", 757, 26_564, 33_631),
            ("fr", 757, 26_564, 36_651),
            ("de", 757, 26_564, 35_736),
            ("ro", 757, 26_564, 31_602),
            ("pl", 757, 26_564, 31_818),
            ("it", 757, 26_564, 33_887),
            ("cz", 757, 26_564, 28_361),
            ("hu", 757, 26_564, 29_976),
            ("tr", 757, 26_564, 28_341),
        ];
        for (language, (code, pairs, key_bytes, value_bytes)) in (1..).zip(expected) {
            assert_eq!(country_code(language), code);
            let table = strings.table(language).expect("a table");
            assert_eq!(table.ending(), Ending::Complete, "{code}");
            assert_eq!((table.pairs(), table.len()), (pairs, pairs), "{code}");
            let keys: usize = table.strings.keys().map(Vec::len).sum();
            let values: usize = table.strings.values().map(Vec::len).sum();
            assert_eq!((keys, values), (key_bytes, value_bytes), "{code}");
        }
        // The one key in plain ASCII.
        assert_eq!(strings.find(b"Pregunta.", 1), b"Question.");
        assert_eq!(strings.find(b"Pregunta.", 0), b"Pregunta.");
        // No text the Rewrite sends yet is a key.
        for text in [
            &b"[LS;443]"[..],
            b"Shout can only be used at level %d or higher.",
        ] {
            for language in 1..LOCALE_COUNT {
                assert_eq!(strings.find(text, language), text);
            }
        }
    }

    #[test]
    fn a_missing_file_is_an_error_naming_it() {
        let error = LocaleStrings::load(Path::new("/nonexistent/country")).expect_err("missing");
        assert_eq!(
            error.path,
            Path::new("/nonexistent/country/en/locale_string.txt")
        );
        assert!(error.to_string().contains("en/locale_string.txt"));
    }
}
