//! The cube recipes: `cube.txt` (`Cube_load` and `Cube_InformationInitialize`,
//! `G/cuberenewal.cpp:123-315`).
//!
//! Each line is `strtok` on `" \t\r\n"`: the first word is the token (case-insensitive,
//! `strcasecmp`), the next two are the numbers (`str_to_number` into
//! `unsigned long long`, garbage 0). A `#` first byte skips the line; a line with no
//! token is skipped too. `section` starts a recipe; `npc`, `item` and `reward` append
//! their rows; `percent`, `gold` and `category` assign (the category is the verbatim
//! second word, missing when the value word is); `end` validates and files the recipe.
//!
//! Validation (`FN_check_cube_data`, `:74-99`) refuses a recipe with a zero NPC vnum, a
//! zero material vnum or count, or a zero reward vnum or count: no proto consult, so a
//! recipe naming a vnum no proto has still loads. Anything outside a section is
//! dereferenced (a Defect kept out: a token with no open recipe is skipped). A repeated
//! keyword keeps the last line, as the loader's assignment does, and a second `section`
//! abandons the open one, as legacy's leak does.
//!
//! The built map keeps the first NPC vnum only (`cubeData->npc_vnum.at(0)`, `:283`) and
//! skips a recipe whose reward count is not exactly one (`:269-276`), logging `CubeInfo`
//! there. The recipe lists are in file order; the client's `index` names the place in
//! its NPC's list. The open send names the place too, but never writes its `index` word
//! (a Defect kept, `:539-553`): every date record's `index` word is 0 on the wire.

use std::collections::BTreeMap;
use std::path::Path;

/// The file under the locale directory.
pub const CUBE_FILE: &str = "cube.txt";

/// One `item` or `reward` row: a vnum and a count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CubeStack {
    /// The first number.
    pub vnum: u32,
    /// The second number.
    pub count: u32,
}

/// One filed recipe: `SCubeMaterialInfo` (`G/cuberenewal.cpp:34-55`), without the unused
/// complicate fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CubeRecipe {
    /// The materials, in file order.
    pub material: Vec<CubeStack>,
    /// The one reward.
    pub reward: CubeStack,
    /// The `gold` keyword's number, 0 when the recipe has none.
    pub gold: u64,
    /// The `percent` keyword's number, 0 when the recipe has none.
    pub percent: i32,
    /// The verbatim `category` word, `"WORLDARD"` (the constructor's `:325-329`) when the
    /// recipe has none.
    pub category: Vec<u8>,
}

/// The recipes by their first NPC vnum, each list in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CubeRecipes {
    /// The map.
    pub by_npc: BTreeMap<u32, Vec<CubeRecipe>>,
}

/// The low four bytes of `value`, as a `DWORD` read of a `str_to_number` word is.
const fn low_4(value: u64) -> [u8; 4] {
    let bytes = value.to_le_bytes();
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

/// `str_to_number` into an `unsigned long long`: the digits read, garbage 0, modulo 2^64.
fn number(word: &[u8]) -> u64 {
    let mut value: u64 = 0;
    for &byte in word {
        if !byte.is_ascii_digit() {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(u64::from(byte - b'0'));
    }
    value
}

/// One open `section`: the rows so far.
#[derive(Debug, Default)]
struct Open {
    /// The NPC vnums.
    npc: Vec<u32>,
    /// The material rows.
    item: Vec<CubeStack>,
    /// The reward rows.
    reward: Vec<CubeStack>,
    /// The last `percent` number, or `None` without one.
    percent: Option<u64>,
    /// The last `gold` number, or `None` without one.
    gold: Option<u64>,
    /// The last `category` word, or `None` without one.
    category: Option<Vec<u8>>,
}

impl Open {
    /// `FN_check_cube_data`: no zero NPC, material or reward vnum or count.
    fn valid(&self) -> bool {
        !self.npc.contains(&0)
            && !self.item.iter().any(|row| row.vnum == 0 || row.count == 0)
            && !self
                .reward
                .iter()
                .any(|row| row.vnum == 0 || row.count == 0)
    }

    /// File the recipe when it is valid, has an NPC and has exactly one reward.
    fn file(self, map: &mut BTreeMap<u32, Vec<CubeRecipe>>) {
        if !self.valid() {
            return;
        }
        let (Some(npc), [reward]) = (self.npc.first(), self.reward.as_slice()) else {
            return;
        };
        map.entry(*npc).or_default().push(CubeRecipe {
            material: self.item,
            reward: *reward,
            gold: self.gold.unwrap_or(0),
            percent: i32::from_le_bytes(low_4(self.percent.unwrap_or(0))),
            category: self.category.unwrap_or_else(|| b"WORLDARD".to_vec()),
        });
    }
}

impl CubeRecipes {
    /// Parse the file's bytes into the map.
    #[must_use]
    pub fn read(bytes: &[u8]) -> Self {
        let mut map = BTreeMap::new();
        let mut open: Option<Open> = None;
        for line in bytes.split(|byte| *byte == b'\n') {
            if line.first() == Some(&b'#') {
                continue;
            }
            let mut words = line
                .split(u8::is_ascii_whitespace)
                .filter(|word| !word.is_empty());
            let Some(head) = words.next() else { continue };
            let rest: Vec<&[u8]> = words.collect();
            let at = |index: usize| rest.get(index).map_or(0, |word| number(word));
            if head.eq_ignore_ascii_case(b"section") {
                open = Some(Open::default());
                continue;
            }
            let Some(current) = open.as_mut() else {
                continue;
            };
            if head.eq_ignore_ascii_case(b"npc") {
                current.npc.push(u32::from_le_bytes(low_4(at(0))));
            } else if head.eq_ignore_ascii_case(b"item") {
                current.item.push(CubeStack {
                    vnum: u32::from_le_bytes(low_4(at(0))),
                    count: u32::from_le_bytes(low_4(at(1))),
                });
            } else if head.eq_ignore_ascii_case(b"reward") {
                current.reward.push(CubeStack {
                    vnum: u32::from_le_bytes(low_4(at(0))),
                    count: u32::from_le_bytes(low_4(at(1))),
                });
            } else if head.eq_ignore_ascii_case(b"percent") {
                current.percent = Some(at(0));
            } else if head.eq_ignore_ascii_case(b"gold") {
                current.gold = Some(at(0));
            } else if head.eq_ignore_ascii_case(b"category") {
                current.category = rest.first().map(|word| word.to_vec());
            } else if head.eq_ignore_ascii_case(b"end") {
                if let Some(done) = open.take() {
                    done.file(&mut map);
                }
            }
        }
        Self { by_npc: map }
    }

    /// Load the file under `locale_dir`.
    ///
    /// # Errors
    ///
    /// The read failure, when the file cannot be read.
    pub fn load(locale_dir: &Path) -> Result<Self, String> {
        let path = locale_dir.join(CUBE_FILE);
        let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(Self::read(&bytes))
    }

    /// `FN_check_valid_npc` (`G/cuberenewal.cpp:62-71`): whether any recipe opens for
    /// `npc`. The information map already holds only the valid filed recipes, so a
    /// lookup is the check.
    #[must_use]
    pub fn has_npc(&self, npc: u32) -> bool {
        self.by_npc.contains_key(&npc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's file: 45 recipes on five NPCs, the first and a six-material one pinned.
    #[test]
    fn the_owners_file_files_forty_five() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../legacy/gamedata/locale/europe/cube.txt"),
        )
        .expect("the owner's cube.txt");
        let recipes = CubeRecipes::read(&bytes);
        let total: usize = recipes.by_npc.values().map(Vec::len).sum();
        assert_eq!(total, 45);
        assert_eq!(recipes.by_npc.len(), 5);
        let first = &recipes.by_npc[&20001][0];
        assert_eq!(
            *first,
            CubeRecipe {
                material: vec![CubeStack {
                    vnum: 51001,
                    count: 50
                }],
                reward: CubeStack {
                    vnum: 51002,
                    count: 1
                },
                gold: 0,
                percent: 70,
                category: b"ENERGY".to_vec(),
            }
        );
        let six = recipes
            .by_npc
            .values()
            .flatten()
            .find(|recipe| recipe.material.len() == 6)
            .expect("a six-material recipe");
        assert_eq!(six.material.len(), 6);
    }

    /// The load rules: a zero count is refused, a second reward is skipped, the second
    /// NPC vnum is never the key, and tokens outside a section are skipped.
    #[test]
    fn the_load_rules_refuse_skip_and_key_the_first_npc() {
        let bytes = b"item 1 2\nsection\nnpc 7\nitem 1 0\nreward 2 1\nend\n\
            section\nnpc 8\nnpc 9\nitem 1 1\nreward 2 1\nreward 3 1\nend\n\
            section\nnpc 9\nnpc 10\nitem 1 1\nreward 2 1\nend\n";
        let recipes = CubeRecipes::read(bytes);
        assert!(!recipes.by_npc.contains_key(&7));
        assert!(!recipes.by_npc.contains_key(&8));
        assert_eq!(recipes.by_npc.get(&9).map(Vec::len), Some(1));
        assert!(!recipes.by_npc.contains_key(&10));
    }
}
