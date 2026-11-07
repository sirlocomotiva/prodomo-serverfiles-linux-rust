//! The Gaya table: `gaya.txt` (`CHARACTER::lOAD_GAYA`, `G/char_gaya.cpp:19-76`).
//!
//! Each line is whitespace-split (`strtok` on `" \t\r\n"`): `ITEM` takes three numbers
//! (vnum, Gaya price, count) onto the roll list, and each scalar keyword takes one. A `#`
//! line is skipped; a line with no token is skipped too. Only the roll list is read: the
//! scalars wait for the craft and market lines.

/// One `ITEM` row: vnum, Gaya price, count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GayaItem {
    /// The item vnum.
    pub vnum: u32,
    /// Its Gaya price.
    pub price: u32,
    /// Its count.
    pub count: u32,
}

/// The `ITEM` roll list, in file order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GayaTable {
    /// The rows.
    pub items: Vec<GayaItem>,
}

/// `str_to_number` on a word: leading space skipped, the digits read, garbage 0.
fn number(word: &[u8]) -> u32 {
    let mut value: u32 = 0;
    let mut started = false;
    for &byte in word {
        if !started && byte.is_ascii_whitespace() {
            continue;
        }
        started = true;
        if !byte.is_ascii_digit() {
            break;
        }
        value = value
            .saturating_mul(10)
            .saturating_add(u32::from(byte - b'0'));
    }
    value
}

impl GayaTable {
    /// Read the table: the `ITEM` rows in file order, each missing number 0.
    #[must_use]
    pub fn load(text: &[u8]) -> Self {
        let mut items = Vec::new();
        for line in text.split(|byte| *byte == b'\n') {
            if line.first() == Some(&b'#') {
                continue;
            }
            let mut words = line
                .split(u8::is_ascii_whitespace)
                .filter(|word| !word.is_empty());
            let Some(head) = words.next() else { continue };
            if head != b"ITEM" {
                continue;
            }
            let rest: Vec<&[u8]> = words.collect();
            let at = |index: usize| rest.get(index).map_or(0, |word| number(word));
            items.push(GayaItem {
                vnum: at(0),
                price: at(1),
                count: at(2),
            });
        }
        Self { items }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's table: 21 `ITEM` rows, the scalars skipped, the first and last pinned.
    #[test]
    fn the_owners_table_rolls_twenty_one() {
        let text = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../legacy/gamedata/locale/europe/gaya.txt"),
        )
        .expect("the owner's gaya.txt");
        let table = GayaTable::load(&text);
        assert_eq!(table.items.len(), 21);
        assert_eq!(
            table.items[0],
            GayaItem {
                vnum: 70036,
                price: 102,
                count: 1
            }
        );
        assert_eq!(
            table.items[20],
            GayaItem {
                vnum: 55002,
                price: 133,
                count: 1
            }
        );
    }

    /// A `#` line, an empty line and a scalar line are skipped; missing numbers are 0.
    #[test]
    fn comments_scalars_and_short_rows() {
        let table = GayaTable::load(b"# c\n\nGIVE_GAYA\t1\nITEM\t5\n");
        assert_eq!(
            table.items,
            [GayaItem {
                vnum: 5,
                price: 0,
                count: 0
            }]
        );
    }
}
