//! The legacy brace-delimited text reader, `CTextFileLoader` and `CMemoryTextFileLoader`.
//!
//! [`crate::special_item_group`] is the first reader that needs it, but the legacy file format is
//! its own contract and is kept here on its own, because three other legacy loaders read the same
//! way. The parse is reproduced exactly, including the three places it loses data:
//!
//! - Lines are cut at `\n`, `\r`, or `\r\n` and a byte with the high bit set is one character
//!   (`file_loader.cpp:99-102`), so a two-byte code page is never split. A lone `\r` and a lone
//!   `\n` both end a line, which is why the owner's CRLF files load.
//! - Fields are split on space and tab only (`file_loader.h:15`). A `#` at the first character of a
//!   field ends the **whole line** unless the first four characters there are `#--#`, and a field
//!   between `"` quotes runs to the next `"`.
//! - Keys and group names are lowercased; values are not.
//!
//! Two things legacy does that a reader must choose about, because neither is a value:
//!
//! - A group line is `group <name>`, and a name with a space makes it three tokens, which legacy
//!   answers with `exit(1)` (`text_file_loader.cpp:74-83`). A name may not contain a space.
//! - A key written twice in one group keeps the **first** value, because the store is a
//!   `std::map` filled with `insert`, and `insert` does not overwrite (`text_file_loader.cpp:137`).

use std::error::Error;
use std::fmt;

/// The bytes [`split_line`] cuts a field on.
const DELIMITER: &[u8] = b" \t";

/// A file the legacy text reader would misread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextFileError {
    /// A `group` line was not exactly `group <name>`.
    ///
    /// Legacy prints the tokens and calls `exit(1)` (`text_file_loader.cpp:74-83`), so a group name
    /// with a space in it ends the process.
    GroupNameHasSpace {
        /// Legacy's own `m_dwcurLineIndex`, so the first line of a file is 0.
        line: usize,
        /// The tokens on it, already lowercased.
        tokens: Vec<Vec<u8>>,
    },
}

impl fmt::Display for TextFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GroupNameHasSpace { line, tokens } => write!(
                f,
                "line {line} (0-based, as legacy counts it): a group name may not contain a space; \
                 legacy exits the process ({} token(s) on the line, not 2)",
                tokens.len()
            ),
        }
    }
}

impl Error for TextFileError {}

/// One `group` block: its lowercased name and its keys, in the order the file wrote them first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextGroup {
    /// The lowercased name after the `group` keyword.
    pub name: Vec<u8>,
    /// Each key's lowercased text and its values, raw bytes.
    ///
    /// A key written twice keeps the **first** value, as `std::map::insert` does. Iteration order
    /// is the map's, which is sorted by key and not the file order; a caller that needs the file
    /// order must keep it, and none of the current readers do.
    pub entries: Vec<(Vec<u8>, Vec<Vec<u8>>)>,
    /// The groups nested inside this one, in file order.
    ///
    /// The special item groups and the stack attributes do not nest, so nothing in the Rewrite
    /// reads this yet; it is here because the format has it and a reader that dropped it would
    /// silently lose a block.
    pub children: Vec<TextGroup>,
}

impl TextGroup {
    /// The first value of `key`, or `None` when the group has no such key.
    pub fn get(&self, key: &[u8]) -> Option<&[Vec<u8>]> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_slice())
    }
}

/// Every `group` block in a file, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextFile {
    /// The child groups of the implicit root, in the order they appear.
    pub groups: Vec<TextGroup>,
}

impl TextFile {
    /// The number of child groups.
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Whether the file held no group.
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// The first group named `name`.
    pub fn group(&self, name: &[u8]) -> Option<&TextGroup> {
        self.groups.iter().find(|g| g.name == name)
    }
}

/// [`CMemoryTextFileLoader::Bind`](server/server/game/file_loader.cpp:78-111): cut the file
/// into lines.
///
/// A `\n`, a `\r`, or a `\r\n` ends a line, and `\r\n` counts once because the second byte is
/// taken with the first. A byte with the high bit set takes the byte after it as well (`:99-102`),
/// so a two-byte code page is never split. A last byte with the high bit set takes one byte
/// **past** the end of the buffer, which is legacy's read of a truncated character; the Rust
/// reader cannot do that, and the difference is recorded in the ledger.
pub fn bind(data: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut pos = 0;
    while pos < data.len() {
        let c = data[pos];
        pos += 1;
        if c == b'\n' || c == b'\r' {
            // The line ends before the delimiter: legacy's `stLine` never holds the `\n` or the
            // `\r`, so neither does a field cut from a line.
            let end = pos - 1;
            if pos < data.len() && (data[pos] == b'\n' || data[pos] == b'\r') {
                pos += 1;
            }
            lines.push(&data[start..end]);
            start = pos;
        } else if c >= 0x80 {
            // A high byte and the byte after it are one character.
            pos += 1;
        }
    }
    lines.push(&data[start..]);
    lines
}

/// [`CMemoryTextFileLoader::SplitLine`](server/server/game/file_loader.cpp:12-57): one line into
/// its fields, or `None` for a line legacy drops.
///
/// A line is dropped when it holds no token at all, when a token starts with `#` and the first
/// four characters there are not `#--#`, and when a `"` is never closed. The `#` test is inside
/// the token loop, so a `#` in **any** field position drops the line and the tokens already read
/// with it. A field between quotes runs to the next `"` and keeps its spaces.
pub fn split_line(line: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut base = 0;
    loop {
        // Reached only on the first pass, because the tail test below leaves before a second
        // exhausted scan. This is how a blank line is skipped.
        let begin = find_not(line, DELIMITER, base)?;
        // The field itself. A quoted field starts **after** its opening quote (`:35`), which is
        // the one place the two arms differ on where the field begins.
        let (from, to, next) = if line[begin] == b'#' && !line[begin..].starts_with(b"#--#") {
            return None;
        } else if line[begin] == b'"' {
            let from = begin + 1;
            let to = find_from(line, b'"', from)?;
            (from, to, to + 1)
        } else {
            let to = find_from_any(line, DELIMITER, begin);
            (begin, to, to)
        };
        out.push(line[from..to].to_vec());
        base = next;
        if find_not(line, DELIMITER, base).is_none() || base >= line.len() {
            return Some(out);
        }
    }
}

/// The first byte at or after `from` that is not in `set`.
fn find_not(hay: &[u8], set: &[u8], from: usize) -> Option<usize> {
    (from..hay.len()).find(|&i| !set.contains(&hay[i]))
}

/// The first byte at or after `from` that is in `set`.
fn find_from_any(hay: &[u8], set: &[u8], from: usize) -> usize {
    (from..hay.len())
        .find(|&i| set.contains(&hay[i]))
        .unwrap_or(hay.len())
}

/// The first `needle` at or after `from`.
fn find_from(hay: &[u8], needle: u8, from: usize) -> Option<usize> {
    (from..hay.len()).find(|&i| hay[i] == needle)
}

/// [`CTextFileLoader::Load`](server/server/game/text_file_loader.cpp:30-57) followed by
/// [`LoadGroup`](server/server/game/text_file_loader.cpp:59-143): the whole file.
///
/// The parse is recursive over a cursor shared by every level, which is what `m_dwcurLineIndex` is.
/// The returned groups are the child groups of the implicit root, in file order, so a file with no
/// `group` line at all yields none. A `list` block collects every following non-brace line into
/// one flat token list, and a line holding only `{` is skipped.
///
/// # Errors
///
/// Only [`TextFileError::GroupNameHasSpace`]. Legacy prints the tokens of a `group` line whose name
/// holds a space and then calls `exit(1)` (`text_file_loader.cpp:74-82`), so the process ends
/// there; the reader reports it and lets the caller decide. Everything else legacy misreads quietly
/// is reproduced rather than reported.
pub fn parse(data: &[u8]) -> Result<TextFile, TextFileError> {
    let lines = bind(data);
    let mut cursor = 0;
    let root = load_group(&lines, &mut cursor, 0)?;
    Ok(TextFile {
        groups: root.children,
    })
}

/// [`CTextFileLoader::LoadGroup`](server/server/game/text_file_loader.cpp:59-143) for one group.
///
/// `line_base` is added to the cursor for the one error that reports a line. Legacy prints
/// `m_dwcurLineIndex` (`:77`), which `Bind` leaves 0-based, so the first line of a file is line 0
/// here too.
/// The cursor is left on the `}` that ended the group, or on the last line, because the legacy
/// `for` loop advances past it on the way out.
fn load_group(
    lines: &[&[u8]],
    cursor: &mut usize,
    line_base: usize,
) -> Result<TextGroup, TextFileError> {
    let mut group = TextGroup::default();
    while *cursor < lines.len() {
        let line = *cursor + line_base;
        let Some(mut tokens) = split_line(lines[*cursor]) else {
            *cursor += 1;
            continue;
        };
        lower(&mut tokens[0]);
        let first = tokens[0].clone();
        if first.first() == Some(&b'{') {
            *cursor += 1;
            continue;
        }
        if first.first() == Some(&b'}') {
            break;
        }
        if first == b"group" {
            if tokens.len() != 2 {
                return Err(TextFileError::GroupNameHasSpace { line, tokens });
            }
            // The name is lowercased on its way into the node
            // (`text_file_loader.cpp:86-87`). The arm advances the shared cursor past the `group`
            // line before recursing (`:90-92`), and the parent's `for` loop advances once more
            // after the child returns, which is the `}` the child stopped on.
            let mut name = tokens[1].clone();
            lower(&mut name);
            *cursor += 1;
            let mut child = load_group(lines, cursor, line_base)?;
            child.name = name;
            group.children.push(child);
            *cursor += 1;
            continue;
        }
        if first == b"list" {
            if tokens.len() != 2 {
                // The legacy `list` arm asserts and continues (`text_file_loader.cpp:88-93`).
                *cursor += 1;
                continue;
            }
            // The list name is lowercased too (`text_file_loader.cpp:105-106`), and it is a map
            // key, so two names differing only in case are one key.
            let mut key = tokens[1].clone();
            lower(&mut key);
            let mut flat: Vec<Vec<u8>> = Vec::new();
            *cursor += 1;
            while *cursor < lines.len() {
                let Some(sub) = split_line(lines[*cursor]) else {
                    *cursor += 1;
                    continue;
                };
                if sub[0].first() == Some(&b'{') {
                    *cursor += 1;
                    continue;
                }
                if sub[0].first() == Some(&b'}') {
                    break;
                }
                flat.extend(sub);
                *cursor += 1;
            }
            insert_first(&mut group.entries, key, flat);
            continue;
        }
        // A key with no value is an error that breaks out of the group
        // (`text_file_loader.cpp:133-140`), so the rest of the block is not read.
        if tokens.len() == 1 {
            break;
        }
        // The key is read **before** the front is erased (`text_file_loader.cpp:131` and `:142`),
        // so it is the first field and the value is everything after it.
        let key = tokens[0].clone();
        tokens.remove(0);
        insert_first(&mut group.entries, key, tokens);
        *cursor += 1;
    }
    Ok(group)
}

/// `std::map::insert` (`text_file_loader.cpp:137`, `:118`): a key already present keeps its first
/// value, and the map orders keys, so the stored order is sorted rather than the file order.
fn insert_first(entries: &mut Vec<(Vec<u8>, Vec<Vec<u8>>)>, key: Vec<u8>, value: Vec<Vec<u8>>) {
    if entries.iter().any(|(k, _)| *k == key) {
        return;
    }
    let at = entries
        .iter()
        .position(|(k, _)| *k > key)
        .unwrap_or(entries.len());
    entries.insert(at, (key, value));
}

/// `stl_lowers` on ASCII, which is all it does (`server/server/common/stl.h`).
fn lower(bytes: &mut [u8]) {
    for b in bytes {
        if b.is_ascii_uppercase() {
            *b += 32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's special item groups, the first file the Rewrite reads this way.
    fn owners_group_file() -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe/special_item_group.txt");
        std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
    }

    fn as_str(v: &[Vec<u8>]) -> Vec<&str> {
        v.iter().map(|f| std::str::from_utf8(f).unwrap()).collect()
    }

    // ---- bind, `file_loader.cpp:78-111` ----

    #[test]
    fn bind_cuts_on_lf_and_keeps_the_delimiter_out_of_the_line() {
        assert_eq!(
            bind(b"a\nbb\n"),
            [b"a".as_slice(), b"bb".as_slice(), b"".as_slice()]
        );
    }

    #[test]
    fn bind_cuts_crlf_once() {
        // `:92-94` takes the second byte with the first, so a CRLF file does not gain a blank line.
        assert_eq!(
            bind(b"a\r\nbb\r\n"),
            [b"a".as_slice(), b"bb".as_slice(), b"".as_slice()]
        );
    }

    #[test]
    fn bind_cuts_a_lone_carriage_return_too() {
        assert_eq!(
            bind(b"a\rbb\r"),
            [b"a".as_slice(), b"bb".as_slice(), b"".as_slice()]
        );
    }

    #[test]
    fn bind_treats_two_carriage_returns_as_one_break() {
        // `:93` tests the next byte for either of the two, so `\r\r` also counts once.
        assert_eq!(bind(b"a\r\rb"), [b"a".as_slice(), b"b".as_slice()]);
    }

    #[test]
    fn bind_always_pushes_the_last_line_even_without_a_delimiter() {
        // `:110` runs after the loop, so a file with no newline still yields its content.
        assert_eq!(bind(b"only"), [b"only".as_slice()]);
        assert_eq!(
            bind(b""),
            [b"".as_slice()],
            "an empty file is one empty line"
        );
    }

    #[test]
    fn bind_counts_two_newlines_as_one_break_so_a_blank_line_is_lost() {
        // `:92-94` takes the byte after a break with it when that byte is itself a break, so a
        // blank line written as `\n\n` is swallowed whole. A blank line survives only when
        // something else sits between the two newlines.
        assert_eq!(
            bind(b"a\n\nb\n"),
            [b"a".as_slice(), b"b".as_slice(), b"".as_slice()]
        );
        assert_eq!(
            bind(b"a\n \nb\n"),
            [
                b"a".as_slice(),
                b" ".as_slice(),
                b"b".as_slice(),
                b"".as_slice()
            ],
            "a space between the two newlines keeps the line"
        );
    }

    #[test]
    fn bind_takes_a_high_byte_and_the_byte_after_it_as_one_character() {
        // `:99-102` pairs them, so a two-byte code page is never split. The bytes come back
        // unchanged; the point is that the pairing happens at all.
        assert_eq!(
            bind(b"\xea\xb5\xad\xff\n"),
            [b"\xea\xb5\xad\xff".as_slice(), b"".as_slice()]
        );
    }

    #[test]
    fn bind_pairs_the_byte_after_a_high_byte_even_when_it_is_the_break() {
        // `:101` appends two bytes, so a high byte eats whatever follows it. Legacy reads one byte
        // past the end of a buffer that ends on a high byte, which the Rust reader cannot do; the
        // last byte stands alone instead.
        assert_eq!(bind(b"a\xff"), [b"a\xff".as_slice()]);
        assert_eq!(
            bind(b"a\xff\n"),
            [b"a\xff\n".as_slice()],
            "the newline is eaten as the second byte of the pair, so the break is lost"
        );
    }

    // ---- split_line, `file_loader.cpp:12-57` ----

    #[test]
    fn split_line_cuts_on_a_space_and_a_tab_and_on_nothing_else() {
        // `file_loader.h:15` is `" \t"`, so no other byte separates.
        assert_eq!(as_str(&split_line(b"a b\tc").unwrap()), ["a", "b", "c"]);
        assert_eq!(as_str(&split_line(b"a,b").unwrap()), ["a,b"]);
    }

    #[test]
    fn split_line_ignores_runs_of_delimiters_at_both_ends() {
        assert_eq!(as_str(&split_line(b"  \ta \t b \t ").unwrap()), ["a", "b"]);
    }

    #[test]
    fn split_line_drops_a_line_holding_nothing() {
        assert_eq!(split_line(b""), None);
        assert_eq!(split_line(b"  \t "), None);
    }

    #[test]
    fn split_line_keeps_the_bytes_of_a_quoted_field() {
        // `:33-41` runs to the next `"` and keeps everything between, delimiters included.
        assert_eq!(as_str(&split_line(b"\"a b\"\tc").unwrap()), ["a b", "c"]);
        assert_eq!(as_str(&split_line(b"key\t\"v\"").unwrap()), ["key", "v"]);
    }

    #[test]
    fn split_line_drops_a_line_whose_quote_never_closes() {
        assert_eq!(split_line(b"\"a b"), None);
    }

    #[test]
    fn split_line_drops_the_whole_line_when_any_field_starts_with_a_hash() {
        // The test is inside the token loop (`:29-32`), so a `#` in the second field drops the line
        // and the first field with it.
        assert_eq!(split_line(b"a #b"), None);
        assert_eq!(split_line(b"a\tb\t#c\td"), None);
        assert_eq!(split_line(b"a #"), None);
    }

    #[test]
    fn split_line_keeps_a_field_that_starts_with_the_hash_marker() {
        // `#--#` is the exception, and the comparison is on the first four characters from the `#`.
        assert_eq!(as_str(&split_line(b"#--# 1").unwrap()), ["#--#", "1"]);
        assert_eq!(as_str(&split_line(b"#--#rest").unwrap()), ["#--#rest"]);
    }

    #[test]
    fn split_line_keeps_a_hash_that_is_not_at_the_start_of_a_field() {
        assert_eq!(as_str(&split_line(b"a#b").unwrap()), ["a#b"]);
        assert_eq!(as_str(&split_line(b"\"a#b\"").unwrap()), ["a#b"]);
    }

    #[test]
    fn split_line_does_not_emit_a_trailing_empty_field() {
        // `:52-53` leaves once the rest of the line is only delimiters.
        assert_eq!(as_str(&split_line(b"a\t\t").unwrap()), ["a"]);
    }

    // ---- parse, `text_file_loader.cpp:30-145` ----

    #[test]
    fn parse_reads_the_keys_of_a_group() {
        let f = parse(b"group g\n{\n\tvnum 50011\n\t1 71084 15 40\n}\n").unwrap();
        assert_eq!(f.len(), 1);
        let g = f.group(b"g").expect("the group is named g");
        assert_eq!(as_str(g.get(b"vnum").unwrap()), ["50011"]);
        assert_eq!(as_str(g.get(b"1").unwrap()), ["71084", "15", "40"]);
    }

    #[test]
    fn parse_lowercases_keys_and_group_names_but_not_values() {
        let f = parse(b"GROUP MyName\n{\n\tVnum 50011\n}\n").unwrap();
        let g = f.group(b"myname").expect("the name is lowercased");
        assert_eq!(
            as_str(g.get(b"vnum").unwrap()),
            ["50011"],
            "the value keeps its case"
        );
        assert!(g.get(b"Vnum").is_none(), "a key is stored lowercased");
    }

    #[test]
    fn parse_keeps_the_first_value_of_a_repeated_key() {
        // `:137` fills the map with `insert`, and `insert` does not overwrite.
        let f = parse(b"group g\n{\n\tk one\n\tk two\n}\n").unwrap();
        assert_eq!(as_str(f.group(b"g").unwrap().get(b"k").unwrap()), ["one"]);
    }

    #[test]
    fn parse_orders_the_keys_the_way_a_std_map_would() {
        // The map is sorted, not in file order, and a byte with the high bit set sorts above ASCII
        // because `char_traits` compares as `unsigned char`.
        let f = parse(b"group g\n{\n\tb 2\n\ta 1\n\t\xff 3\n\tA 4\n}\n").unwrap();
        let keys: Vec<Vec<u8>> = f
            .group(b"g")
            .unwrap()
            .entries
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        assert_eq!(
            keys,
            [b"a".to_vec(), b"b".to_vec(), b"\xff".to_vec()],
            "sorted, and `A` folded into `a`"
        );
    }

    #[test]
    fn parse_gives_a_nested_group_to_its_parent() {
        let f = parse(b"group outer\n{\n\tgroup inner\n\t{\n\t\tk v\n\t}\n\tk2 v2\n}\n").unwrap();
        let outer = f.group(b"outer").expect("outer");
        assert_eq!(
            as_str(outer.get(b"k2").unwrap()),
            ["v2"],
            "the parent keeps its own keys"
        );
        assert_eq!(outer.children.len(), 1);
        assert_eq!(outer.children[0].name, b"inner");
        assert_eq!(as_str(outer.children[0].get(b"k").unwrap()), ["v"]);
    }

    #[test]
    fn parse_collects_a_list_block_into_one_flat_value() {
        // `:110-125` pushes every field of every following line into one vector.
        let f = parse(b"group g\n{\n\tlist L\n\ta 1\n\tb 2 3\n\tk v\n}\n").unwrap();
        let g = f.group(b"g").unwrap();
        assert_eq!(
            as_str(g.get(b"l").unwrap()),
            ["a", "1", "b", "2", "3", "k", "v"]
        );
    }

    #[test]
    fn parse_ends_a_list_at_the_closing_brace_and_reads_no_further() {
        let f = parse(b"group g\n{\n\tlist l\n\ta 1\n}\nk v\n").unwrap();
        assert_eq!(
            as_str(f.group(b"g").unwrap().get(b"l").unwrap()),
            ["a", "1"]
        );
        assert_eq!(
            f.len(),
            1,
            "`k` is after the root's closing brace, so it is a root key"
        );
    }

    #[test]
    fn parse_skips_a_line_holding_only_an_opening_brace() {
        // `:66-67` continues on `{`, which is why the brace may be on its own line.
        let f = parse(b"group g\n{\nk v\n}\n").unwrap();
        assert_eq!(as_str(f.group(b"g").unwrap().get(b"k").unwrap()), ["v"]);
    }

    #[test]
    fn parse_ignores_a_blank_line_and_a_comment_line() {
        let f = parse(b"group g\n{\n\n\t# a comment\n\tk v\n}\n").unwrap();
        assert_eq!(as_str(f.group(b"g").unwrap().get(b"k").unwrap()), ["v"]);
    }

    #[test]
    fn parse_reports_a_group_name_with_a_space() {
        // `:75-82` answers a name with a space by calling `exit(1)`. The line it reports is legacy's
        // own `m_dwcurLineIndex`, so the first line of a file is 0.
        let err = parse(b"group my name\n{\n}\n").unwrap_err();
        assert_eq!(
            err,
            TextFileError::GroupNameHasSpace {
                line: 0,
                tokens: vec![b"group".to_vec(), b"my".to_vec(), b"name".to_vec()]
            }
        );
        assert_eq!(
            parse(b"group g\n{\n\tgroup bad name\n\t{\n\t}\n}\n").unwrap_err(),
            TextFileError::GroupNameHasSpace {
                line: 2,
                tokens: vec![b"group".to_vec(), b"bad".to_vec(), b"name".to_vec()]
            },
            "the cursor is the whole file's, so a nested group reports its own line"
        );
    }

    #[test]
    fn parse_reports_a_bare_group_keyword() {
        assert!(matches!(
            parse(b"group\n{\n}\n"),
            Err(TextFileError::GroupNameHasSpace { line: 0, .. })
        ));
    }

    #[test]
    fn parse_stops_a_group_at_a_key_with_no_value() {
        // `:133-140` breaks out of the group, so the keys after it are not read.
        let f = parse(b"group g\n{\n\ta 1\n\tlonely\n\tb 2\n}\n").unwrap();
        let g = f.group(b"g").unwrap();
        assert_eq!(as_str(g.get(b"a").unwrap()), ["1"]);
        assert!(
            g.get(b"lonely").is_none(),
            "a key with no value is not stored"
        );
        assert!(g.get(b"b").is_none(), "and the group ends there");
    }

    #[test]
    fn parse_yields_nothing_for_a_file_with_no_group() {
        let f = parse(b"k v\n").unwrap();
        assert!(f.is_empty());
    }

    #[test]
    fn parse_reads_the_owners_group_file() {
        // The positive control for every other test in this module: the real file loads, and its
        // rows come out as the file's own bytes.
        let f = parse(&owners_group_file()).expect("the owner's group file loads");
        assert_eq!(f.len(), 1);
        let g = &f.groups[0];
        assert_eq!(g.name, b"cufar_lumina_lunii", "the name is lowercased");
        assert_eq!(
            g.get(b"vnum").map(as_str),
            Some(
                ["50011".to_string()]
                    .map(|s| Box::leak(s.into_boxed_str()) as &str)
                    .into_iter()
                    .collect()
            )
        );
        let rows: Vec<&[Vec<u8>]> = g
            .entries
            .iter()
            .filter(|(k, _)| k != b"vnum")
            .map(|(_, v)| v.as_slice())
            .collect();
        assert_eq!(rows.len(), 29);
        assert_eq!(as_str(rows[0]), ["71084", "15", "40"]);
        assert_eq!(as_str(rows[28]), ["30060", "2", "40"]);
    }

    #[test]
    fn no_carriage_return_survives_into_a_field_of_a_crlf_file() {
        let raw = owners_group_file();
        assert!(
            raw.windows(2).any(|w| w == b"\r\n"),
            "the file really is CRLF"
        );
        let lines = bind(&raw);
        assert_eq!(lines[3], b"\t1\t71084\t15\t40", "no CR reached a field");
    }
}
