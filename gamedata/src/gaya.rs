//! The Gaya table: `gaya.txt` (`CHARACTER::lOAD_GAYA`, `G/char_gaya.cpp:19-76`).
//!
//! Each line is whitespace-split (`strtok` on `" \t\r\n"`): `ITEM` takes three numbers
//! (vnum, Gaya price, count) onto the roll list, and each scalar keyword takes the first.
//! A `#` line is skipped; a line with no token is skipped too. The two struct fields the
//! line names after `count` (`gaya_values.gaya`, `gaya_values.count`) are the second and
//! third `ITEM` numbers; the scalar fields each keep their keyword's first number.

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

/// The scalar `gaya.txt` fields (`Gaya_Load_Values`, `G/char.h:2233-2248`), each its
/// keyword's first number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GayaValues {
    /// `GLIMMERSTONE`: the craft's stone vnum.
    pub glimmerstone: u32,
    /// `GAYA_EXPANSION`: the slot unlock's item vnum.
    pub expansion: u32,
    /// `GAYA_REFRESH`: the market refresh's item vnum.
    pub refresh: u32,
    /// `GLIMMERSTONE_COUNT`: the stones a craft spends.
    pub glimmerstone_count: u32,
    /// `GAYA_EXPANSION_COUNT`: the unlocks a slot spends.
    pub expansion_count: u32,
    /// `GAYA_REFRESH_COUNT`: the refreshers a refresh spends.
    pub refresh_count: u32,
    /// `GRADE_STONE`: the highest stone grade a craft minerals.
    pub grade_stone: u32,
    /// `GIVE_GAYA`: the Gaya a won craft grants.
    pub give_gaya: u32,
    /// `PROB_GAYA`: the craft's win chance in a hundred.
    pub prob_gaya: u32,
    /// `COST_GAYA_YANG`: the Yang a craft spends.
    pub cost_yang: u32,
}

/// The `ITEM` roll list, in file order, with the scalars.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GayaTable {
    /// The rows.
    pub items: Vec<GayaItem>,
    /// The scalar fields.
    pub values: GayaValues,
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
    /// Read the table: the `ITEM` rows in file order, each missing number 0, and the
    /// scalars, each its keyword's first number. A repeated keyword keeps the last line,
    /// as the loader's assignment does.
    #[must_use]
    pub fn load(text: &[u8]) -> Self {
        let mut items = Vec::new();
        let mut values = GayaValues::default();
        for line in text.split(|byte| *byte == b'\n') {
            if line.first() == Some(&b'#') {
                continue;
            }
            let mut words = line
                .split(u8::is_ascii_whitespace)
                .filter(|word| !word.is_empty());
            let Some(head) = words.next() else { continue };
            let rest: Vec<&[u8]> = words.collect();
            let at = |index: usize| rest.get(index).map_or(0, |word| number(word));
            if head == b"ITEM" {
                items.push(GayaItem {
                    vnum: at(0),
                    price: at(1),
                    count: at(2),
                });
                continue;
            }
            let first = at(0);
            match head {
                b"GLIMMERSTONE" => values.glimmerstone = first,
                b"GAYA_EXPANSION" => values.expansion = first,
                b"GAYA_REFRESH" => values.refresh = first,
                b"GLIMMERSTONE_COUNT" => values.glimmerstone_count = first,
                b"GAYA_EXPANSION_COUNT" => values.expansion_count = first,
                b"GAYA_REFRESH_COUNT" => values.refresh_count = first,
                b"GRADE_STONE" => values.grade_stone = first,
                b"GIVE_GAYA" => values.give_gaya = first,
                b"PROB_GAYA" => values.prob_gaya = first,
                b"COST_GAYA_YANG" => values.cost_yang = first,
                _ => {}
            }
        }
        Self { items, values }
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

    /// The owner's scalars: the craft, unlock and refresh numbers the Gaya record reads.
    #[test]
    fn the_owners_scalars() {
        let text = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../legacy/gamedata/locale/europe/gaya.txt"),
        )
        .expect("the owner's gaya.txt");
        let table = GayaTable::load(&text);
        assert_eq!(
            table.values,
            GayaValues {
                glimmerstone: 30400,
                expansion: 30401,
                refresh: 30402,
                glimmerstone_count: 10,
                expansion_count: 10,
                refresh_count: 10,
                grade_stone: 3,
                give_gaya: 1,
                prob_gaya: 30,
                cost_yang: 5000,
            }
        );
    }

    /// A `#` line and an empty line are skipped; a repeated scalar keeps the last line,
    /// an unknown keyword changes nothing, and missing numbers are 0.
    #[test]
    fn comments_repeats_and_short_rows() {
        let table = GayaTable::load(b"# c\n\nGIVE_GAYA\t1\nGIVE_GAYA\t2\nNOPE\t9\nITEM\t5\n");
        assert_eq!(table.values.give_gaya, 2);
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
