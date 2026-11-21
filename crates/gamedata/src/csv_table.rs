//! The legacy CSV reader, `cCsvFile::Load` (`server/server/db/CsvReader.cpp:105-209`).
//!
//! The text protos (`mob_proto.txt`, `mob_names.txt`, and their item twins) are read with it,
//! using a tab separator and a `"` quote. This module keeps its parse:
//!
//! - A line is what `std::ifstream::getline(buf, 2048)` returns, cut at its first NUL because the
//!   buffer is then read as a C string.
//! - The line is trimmed of spaces, tabs, `\r`, and `\n` at both ends. An empty line is skipped,
//!   and so is a line starting with `#` outside a quoted field.
//! - Outside a quote, the separator ends a field and the quote opens one. Inside, two quotes are
//!   one quote byte and a single quote closes the field.
//! - A quoted field that is still open at the end of a line continues on the next line, joined by
//!   `\r\n`.
//!
//! Two inputs legacy mishandles are refused instead:
//!
//! - A line of 2,048 bytes or more. `getline` sets `failbit`, so legacy parses the first 2,047
//!   bytes and silently stops reading the file.
//! - A quoted field still open at the end of the file. Legacy drops the whole unfinished row.

use std::error::Error;
use std::fmt;

/// The buffer `getline` fills, including its terminator.
const GETLINE_BUFFER: usize = 2048;

/// The bytes `Trim` removes from both ends of a line.
const TRIMMED: &[u8] = b" \t\r\n";

/// One parsed row: its fields as raw bytes.
pub type CsvRow = Vec<Vec<u8>>;

/// A file the legacy reader would misread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsvError {
    /// A line reached 2,048 bytes without a newline.
    LineTooLong {
        /// The 1-based line.
        line: usize,
    },
    /// The file ended inside a quoted field.
    UnterminatedQuote,
}

impl fmt::Display for CsvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LineTooLong { line } => write!(
                f,
                "line {line} is {GETLINE_BUFFER} bytes or longer; legacy would stop reading there"
            ),
            Self::UnterminatedQuote => {
                f.write_str("the file ends inside a quoted field; legacy would drop that row")
            }
        }
    }
}

impl Error for CsvError {}

/// One parsed row and the 1-based file line it starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberedRow {
    /// The 1-based line of the file this row **starts** on, counting the header and every blank
    /// and `#` line.
    ///
    /// A row whose quoted field spans several lines reports the first of them, which is the line
    /// an operator opens in an editor to see the row.
    pub line: usize,
    /// The fields, as raw bytes.
    pub fields: CsvRow,
}

/// Parse a file the way `cCsvFile::Load(file, separator, quote)` does.
///
/// # Errors
///
/// Returns [`CsvError`] for a line legacy would cut or a quote legacy would leave open.
pub fn parse(bytes: &[u8], separator: u8, quote: u8) -> Result<Vec<CsvRow>, CsvError> {
    Ok(parse_numbered(bytes, separator, quote)?
        .into_iter()
        .map(|numbered| numbered.fields)
        .collect())
}

/// [`parse`], with the file line each row starts on.
///
/// Legacy reports no line at all: `sys_err` in `Set_Proto_Item_Table` names the column and the
/// index, not the row, so an operator reading a rejection of one item had to count rows by hand.
/// The number here is the file line, not the row ordinal, because the reader skips blank lines and
/// `#` lines and a row ordinal would point at the wrong line in any file that has one.
///
/// # Errors
///
/// Returns [`CsvError`] for a line legacy would cut or a quote legacy would leave open.
pub fn parse_numbered(
    bytes: &[u8],
    separator: u8,
    quote: u8,
) -> Result<Vec<NumberedRow>, CsvError> {
    let mut rows = Vec::new();
    let mut row: CsvRow = Vec::new();
    let mut token: Vec<u8> = Vec::new();
    let mut quoted = false;
    let mut start_line = 0;
    for (index, raw) in lines(bytes)?.into_iter().enumerate() {
        let line = trim(until_nul(raw));
        if line.is_empty() || (!quoted && line[0] == b'#') {
            continue;
        }
        if row.is_empty() && token.is_empty() {
            start_line = index + 1;
        }
        let mut cursor = 0;
        while cursor < line.len() {
            let byte = line[cursor];
            if quoted {
                if byte == quote {
                    if line.get(cursor + 1) == Some(&quote) {
                        token.push(quote);
                        cursor += 1;
                    } else {
                        quoted = false;
                    }
                } else {
                    token.push(byte);
                }
            } else if byte == separator {
                row.push(std::mem::take(&mut token));
            } else if byte == quote {
                quoted = true;
            } else {
                token.push(byte);
            }
            cursor += 1;
        }
        if quoted {
            token.extend_from_slice(b"\r\n");
        } else {
            row.push(std::mem::take(&mut token));
            rows.push(NumberedRow {
                line: start_line,
                fields: std::mem::take(&mut row),
            });
        }
    }
    if quoted {
        return Err(CsvError::UnterminatedQuote);
    }
    Ok(rows)
}

/// Split a file into the lines `getline(buf, 2048)` returns, without their newlines.
fn lines(bytes: &[u8]) -> Result<Vec<&[u8]>, CsvError> {
    let mut lines = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let end = rest.iter().position(|&byte| byte == b'\n');
        let line = &rest[..end.unwrap_or(rest.len())];
        if line.len() >= GETLINE_BUFFER {
            return Err(CsvError::LineTooLong {
                line: lines.len() + 1,
            });
        }
        lines.push(line);
        rest = end.map_or(&[][..], |end| &rest[end + 1..]);
    }
    Ok(lines)
}

/// The bytes before the first NUL, as a C string reads them.
fn until_nul(line: &[u8]) -> &[u8] {
    line.iter()
        .position(|&byte| byte == 0)
        .map_or(line, |end| &line[..end])
}

/// Legacy `Trim`: strip spaces, tabs, `\r`, and `\n` from both ends.
fn trim(line: &[u8]) -> &[u8] {
    let start = line.iter().position(|byte| !TRIMMED.contains(byte));
    let Some(start) = start else {
        return &[];
    };
    let end = line
        .iter()
        .rposition(|byte| !TRIMMED.contains(byte))
        .map_or(start, |end| end + 1);
    &line[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(row: &CsvRow) -> Vec<&[u8]> {
        row.iter().map(Vec::as_slice).collect()
    }

    #[test]
    fn a_tab_file_splits_into_rows_and_fields() {
        let rows = parse(b"VNUM\tNAME\r\n101\tLup\r\n102\t\tx\n", b'\t', b'"').unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(fields(&rows[0]), [&b"VNUM"[..], b"NAME"]);
        assert_eq!(fields(&rows[1]), [&b"101"[..], b"Lup"]);
        assert_eq!(fields(&rows[2]), [&b"102"[..], b"", b"x"]);
    }

    #[test]
    fn trimming_drops_blank_lines_and_trailing_empty_fields() {
        let rows = parse(b"  \t\r\n\n a\tb\t\t \r\n", b'\t', b'"').unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(fields(&rows[0]), [&b"a"[..], b"b"]);
    }

    #[test]
    fn a_comment_line_is_skipped_only_outside_a_quote() {
        let rows = parse(b"#x\ty\n1\t\"a\n#b\"\n", b'\t', b'"').unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(fields(&rows[0]), [&b"1"[..], b"a\r\n#b"]);
    }

    #[test]
    fn quotes_group_separators_and_a_doubled_quote_is_one_quote() {
        let rows = parse(b"\"a\tb\"\tc\"\"d\"\"\"e\"\tf\n", b'\t', b'"').unwrap();
        assert_eq!(fields(&rows[0]), [&b"a\tb"[..], b"cd\"e", b"f"]);
    }

    #[test]
    fn a_quote_closing_at_the_line_end_ends_the_row() {
        let rows = parse(b"1\t\"x\"\n2\n", b'\t', b'"').unwrap();
        assert_eq!(fields(&rows[0]), [&b"1"[..], b"x"]);
        assert_eq!(fields(&rows[1]), [&b"2"[..]]);
    }

    #[test]
    fn a_nul_ends_the_line() {
        let rows = parse(b"1\tab\0c\td\n", b'\t', b'"').unwrap();
        assert_eq!(fields(&rows[0]), [&b"1"[..], b"ab"]);
    }

    #[test]
    fn a_line_legacy_would_cut_is_refused() {
        let mut long = vec![b'a'; GETLINE_BUFFER - 1];
        long.push(b'\n');
        assert_eq!(parse(&long, b'\t', b'"').unwrap().len(), 1);
        let mut file = b"ok\n".to_vec();
        file.extend(vec![b'a'; GETLINE_BUFFER]);
        assert_eq!(
            parse(&file, b'\t', b'"'),
            Err(CsvError::LineTooLong { line: 2 })
        );
    }

    #[test]
    fn a_quote_left_open_is_refused() {
        assert_eq!(
            parse(b"1\t\"open\n", b'\t', b'"'),
            Err(CsvError::UnterminatedQuote)
        );
    }

    #[test]
    fn a_row_reports_the_file_line_it_starts_on() {
        let rows = parse_numbered(b"1\t2\n\n#skip\n3\t4\n", b'\t', b'"').unwrap();
        let lines: Vec<usize> = rows.iter().map(|r| r.line).collect();
        assert_eq!(lines, [1, 4]);
        assert_eq!(fields(&rows[1].fields), [&b"3"[..], b"4"]);
    }

    #[test]
    fn a_row_joined_from_several_lines_reports_the_first_of_them() {
        let rows = parse_numbered(b"y\t2\nx\t\"ab\ncd\"\n", b'\t', b'"').unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].line, 2);
        assert_eq!(fields(&rows[1].fields), [&b"x"[..], b"ab\r\ncd"]);
    }

    #[test]
    fn parse_and_parse_numbered_agree_on_the_fields() {
        let bytes = b"VNUM\tNAME\n1\t\"a b\"\n#c\n2\t\"x\ny\"\n";
        let plain = parse(bytes, b'\t', b'"').unwrap();
        let numbered = parse_numbered(bytes, b'\t', b'"').unwrap();
        let stripped: Vec<CsvRow> = numbered.into_iter().map(|r| r.fields).collect();
        assert_eq!(plain, stripped);
    }
}
