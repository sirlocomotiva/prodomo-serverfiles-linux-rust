//! Rows of the Game data tables, read from the owner's `mysqldump` files in `legacy/sql/gamedata`.
//!
//! The files were dumped with `--default-character-set=binary`, so a string value holds the exact
//! bytes the legacy table held. Only `INSERT INTO` statements are read: every other statement and
//! comment is skipped. A value is a quoted string, `NULL`, or a bare token such as a number.
//! Inside a string, `mysqldump` escapes with a backslash (`\0`, `\'`, `\"`, `\b`, `\n`, `\r`,
//! `\t`, `\Z`, `\\`); a doubled quote is also one quote, as MySQL reads it.
//!
//! Anything this reader does not understand is refused, never skipped, so a table is never
//! loaded short.

use std::error::Error;
use std::fmt;

/// One column value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqlValue {
    /// `NULL`.
    Null,
    /// A quoted string, unescaped, as raw bytes.
    Text(Vec<u8>),
    /// An unquoted token (a number), as written.
    Bare(Vec<u8>),
}

/// The rows one table's `INSERT` statements hold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SqlTable {
    /// The column names, in the statements' order.
    pub columns: Vec<String>,
    /// Every row, in file order.
    pub rows: Vec<Vec<SqlValue>>,
}

impl SqlTable {
    /// The position of a column.
    #[must_use]
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|column| column == name)
    }
}

/// A dump this reader cannot read faithfully.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlDumpError {
    /// The byte offset where reading stopped.
    pub offset: usize,
    /// What was expected there.
    pub expected: &'static str,
}

impl fmt::Display for SqlDumpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "byte {}: expected {}", self.offset, self.expected)
    }
}

impl Error for SqlDumpError {}

const INSERT: &[u8] = b"INSERT INTO `";

/// Read every `INSERT INTO` statement of one table in a dump.
///
/// # Errors
///
/// Returns [`SqlDumpError`] for a statement without a column list, a row whose width differs
/// from the column list, a statement whose columns differ from an earlier one, or an unfinished
/// string or statement.
pub fn read_table(dump: &[u8], table: &str) -> Result<SqlTable, SqlDumpError> {
    let mut found = SqlTable::default();
    let mut seen_columns = false;
    let mut cursor = 0;
    while let Some(start) = find(dump, cursor, INSERT) {
        let mut reader = Reader {
            bytes: dump,
            at: start + INSERT.len(),
        };
        let name = reader.until(b'`', "a table name")?;
        if name != table.as_bytes() {
            cursor = reader.at;
            continue;
        }
        reader.expect(b" (", "a column list")?;
        let columns = reader.columns()?;
        if seen_columns && columns != found.columns {
            return Err(reader.error("the same columns as the earlier statements"));
        }
        found.columns = columns;
        seen_columns = true;
        reader.expect(b" VALUES ", "VALUES")?;
        loop {
            let row = reader.row()?;
            if row.len() != found.columns.len() {
                return Err(reader.error("one value per column"));
            }
            found.rows.push(row);
            match reader.next("`,` or `;`")? {
                b',' => {}
                b';' => break,
                _ => return Err(reader.error("`,` or `;` after a row")),
            }
        }
        cursor = reader.at;
    }
    Ok(found)
}

fn find(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|position| from + position)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    const fn error(&self, expected: &'static str) -> SqlDumpError {
        SqlDumpError {
            offset: self.at,
            expected,
        }
    }

    fn next(&mut self, expected: &'static str) -> Result<u8, SqlDumpError> {
        let byte = *self
            .bytes
            .get(self.at)
            .ok_or_else(|| self.error(expected))?;
        self.at += 1;
        Ok(byte)
    }

    fn expect(&mut self, text: &[u8], expected: &'static str) -> Result<(), SqlDumpError> {
        if self.bytes.get(self.at..self.at + text.len()) != Some(text) {
            return Err(self.error(expected));
        }
        self.at += text.len();
        Ok(())
    }

    fn until(&mut self, end: u8, expected: &'static str) -> Result<&[u8], SqlDumpError> {
        let start = self.at;
        let length = self.bytes[start..]
            .iter()
            .position(|&byte| byte == end)
            .ok_or_else(|| self.error(expected))?;
        self.at = start + length + 1;
        Ok(&self.bytes[start..start + length])
    }

    /// `` `a`, `b`) ``
    fn columns(&mut self) -> Result<Vec<String>, SqlDumpError> {
        let mut columns = Vec::new();
        loop {
            self.expect(b"`", "a quoted column name")?;
            let name = self.until(b'`', "the end of a column name")?;
            let name =
                String::from_utf8(name.to_vec()).map_err(|_| self.error("an ASCII column name"))?;
            columns.push(name);
            match self.next("`,` or `)`")? {
                b',' => self.expect(b" ", "a space between column names")?,
                b')' => return Ok(columns),
                _ => return Err(self.error("`,` or `)` after a column name")),
            }
        }
    }

    /// `(value,value,...)`
    fn row(&mut self) -> Result<Vec<SqlValue>, SqlDumpError> {
        self.expect(b"(", "`(` opening a row")?;
        let mut row = Vec::new();
        loop {
            row.push(self.value()?);
            match self.next("`,` or `)`")? {
                b',' => {}
                b')' => return Ok(row),
                _ => return Err(self.error("`,` or `)` after a value")),
            }
        }
    }

    fn value(&mut self) -> Result<SqlValue, SqlDumpError> {
        if self.bytes.get(self.at) == Some(&b'\'') {
            self.at += 1;
            return self.text().map(SqlValue::Text);
        }
        let start = self.at;
        while self
            .bytes
            .get(self.at)
            .is_some_and(|byte| !matches!(byte, b',' | b')'))
        {
            self.at += 1;
        }
        let token = &self.bytes[start..self.at];
        if token.is_empty() {
            return Err(self.error("a value"));
        }
        if token == b"NULL" {
            return Ok(SqlValue::Null);
        }
        if !token
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'+' | b'.'))
        {
            return Err(self.error("a number, NULL, or a quoted string"));
        }
        Ok(SqlValue::Bare(token.to_vec()))
    }

    fn text(&mut self) -> Result<Vec<u8>, SqlDumpError> {
        let mut text = Vec::new();
        loop {
            match self.next("the end of a string")? {
                b'\\' => text.push(match self.next("an escaped byte")? {
                    b'0' => 0,
                    b'b' => 0x08,
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'Z' => 0x1a,
                    other => other,
                }),
                b'\'' if self.bytes.get(self.at) == Some(&b'\'') => {
                    self.at += 1;
                    text.push(b'\'');
                }
                b'\'' => return Ok(text),
                byte => text.push(byte),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> SqlValue {
        SqlValue::Text(value.as_bytes().to_vec())
    }

    #[test]
    fn statements_of_one_table_are_read_in_order() {
        let dump = b"-- c\nLOCK TABLES `t` WRITE;\n\
            INSERT INTO `t` (`a`, `b`) VALUES ('x',1),('y;),',-2);\n\
            INSERT INTO `other` (`a`) VALUES (9);\n\
            INSERT INTO `t` (`a`, `b`) VALUES (NULL,3.5);\n";
        let table = read_table(dump, "t").unwrap();
        assert_eq!(table.columns, ["a", "b"]);
        assert_eq!(table.column("b"), Some(1));
        assert_eq!(table.column("c"), None);
        assert_eq!(
            table.rows,
            [
                vec![text("x"), SqlValue::Bare(b"1".to_vec())],
                vec![text("y;),"), SqlValue::Bare(b"-2".to_vec())],
                vec![SqlValue::Null, SqlValue::Bare(b"3.5".to_vec())],
            ]
        );
        assert_eq!(read_table(dump, "missing").unwrap(), SqlTable::default());
    }

    #[test]
    fn string_escapes_are_undone_byte_for_byte() {
        let dump = b"INSERT INTO `t` (`a`) VALUES ('\\0\\'\\\"\\b\\n\\r\\t\\Z\\\\''\xb0\xa1');";
        let table = read_table(dump, "t").unwrap();
        assert_eq!(
            table.rows,
            [vec![SqlValue::Text(
                b"\0'\"\x08\n\r\t\x1a\\'\xb0\xa1".to_vec()
            )]]
        );
    }

    #[test]
    fn a_dump_it_cannot_read_faithfully_is_refused() {
        for dump in [
            &b"INSERT INTO `t` (`a`) VALUES ('open);"[..],
            b"INSERT INTO `t` (`a`) VALUES (1,2);",
            b"INSERT INTO `t` (`a`) VALUES (1)",
            b"INSERT INTO `t` VALUES (1);",
            b"INSERT INTO `t` (`a`) VALUES (1);INSERT INTO `t` (`b`) VALUES (1);",
            b"INSERT INTO `t` (`a`) VALUES (x'00');",
            b"INSERT INTO `t` (`a`) VALUES ();",
        ] {
            assert!(
                read_table(dump, "t").is_err(),
                "{}",
                String::from_utf8_lossy(dump)
            );
        }
    }

    #[test]
    fn the_owner_banwords_are_read() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../legacy/sql/gamedata/player.sql"
        );
        let table = read_table(&std::fs::read(path).unwrap(), "banword").unwrap();
        assert_eq!(table.columns, ["word"]);
        assert_eq!(table.rows.len(), 115);
        assert_eq!(table.rows[0], [text("aryan")]);
        assert_eq!(table.rows[114], [text("whoring")]);
    }

    /// `world::character::Points` takes the four passive-skill bonuses of `ComputePoints` as
    /// inputs and defaults them to 0. That is exact only while each formula is a multiple of
    /// `k` and the skill power at level 0 is 0, which is what the owner's rows say.
    #[test]
    fn the_passive_skill_formulas_are_zero_at_skill_level_zero() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../legacy/sql/gamedata/");
        let player = std::fs::read(format!("{root}player.sql")).unwrap();
        let skills = read_table(&player, "skill_proto").unwrap();
        let vnum = skills.column("dwVnum").unwrap();
        let poly = skills.column("szPointPoly").unwrap();
        let formula = |wanted: &[u8]| {
            let row = skills
                .rows
                .iter()
                .find(|row| row[vnum] == SqlValue::Bare(wanted.to_vec()))
                .unwrap();
            row[poly].clone()
        };
        assert_eq!(formula(b"141"), text("1333.3*k"));
        for bonus in [&b"164"[..], b"165", b"166"] {
            assert_eq!(formula(bonus), text("10 * k"));
        }

        let common = std::fs::read(format!("{root}common.sql")).unwrap();
        let locale = read_table(&common, "locale").unwrap();
        let key = locale.column("mKey").unwrap();
        let value = locale.column("mValue").unwrap();
        let powers: Vec<_> = locale
            .rows
            .iter()
            .filter(|row| {
                matches!(&row[key], SqlValue::Text(name) if name.starts_with(b"SKILL_POWER_BY_LEVEL"))
            })
            .map(|row| row[value].clone())
            .collect();
        assert!(!powers.is_empty());
        for power in powers {
            let SqlValue::Text(levels) = power else {
                panic!("{power:?}");
            };
            assert!(
                levels.starts_with(b"0 5 "),
                "{}",
                String::from_utf8_lossy(&levels)
            );
        }
    }

    #[test]
    fn every_owner_table_reads() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../legacy/sql/gamedata/");
        for (file, tables) in [
            ("common.sql", &["exp_table", "locale"][..]),
            (
                "player.sql",
                &[
                    "banword",
                    "item_attr",
                    "item_attr_rare",
                    "land",
                    "object_proto",
                    "refine_proto",
                    "shop",
                    "shop_item",
                    "shopex",
                    "shopex_item",
                    "skill_proto",
                ],
            ),
        ] {
            let dump = std::fs::read(format!("{root}{file}")).unwrap();
            for table in tables {
                let read = read_table(&dump, table).unwrap();
                assert!(!read.rows.is_empty(), "{file} {table}");
            }
        }
    }
}
