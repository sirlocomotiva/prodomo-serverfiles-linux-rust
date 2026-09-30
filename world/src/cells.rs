//! The cell attributes of a hosted map, and the positions a character may stand on.
//!
//! # What legacy does
//!
//! Every sectree of a map keeps the attributes its `server_attr` block gave it
//! ([`gamedata::server_attr`]). `SECTREE::GetAttribute(x, y)` (`G/sectree.cpp:202-206`) reads the
//! cell `((x % 6400) / 50, (y % 6400) / 50)` of the sectree holding the absolute position, and
//! `IsAttr` (`:208-214`) tests flags on it. On top of them `SECTREE_MANAGER`
//! (`G/sectree_manager.cpp:779-814`) answers:
//!
//! - `IsMovablePosition`: the position has a sectree and neither `ATTR_BLOCK` nor
//!   `ATTR_OBJECT`;
//! - `GetMovablePosition`: the first of 161 points around a position that is movable, walking
//!   `aArroundCoords` (`G/constants.cpp:426-589`): the position itself, then 20 rings of 8 points
//!   50 to 1000 units out. It draws no number.
//!
//! # Divergences
//!
//! - **Sectrees past the file.** A map whose `server_attr` covers fewer sectrees than the map
//!   builds, as the owner's map 216 does (24 x 24 of 32 x 32), leaves the rest without an
//!   attribute, and legacy dereferences NULL on the first read there. The Rewrite reads them as
//!   attribute 0 (provisional, an owner question).
//! - **The sectree.** Legacy keys a sectree by 16 bits of `x / 6400` and `y / 6400`, so a
//!   position past 419,430,399 aliases one near the origin; the Rewrite finds no sectree there
//!   (a Defect, not reproduced), as [`SectreeGrid::sectree_at`] finds none.
//!
//! # What is not here yet
//!
//! Legacy writes `ATTR_OBJECT` at run time over a guild building's footprint
//! (`SECTREE_MANAGER::ForAttrRegion`, `G/building.cpp:52-59`, `:177-184`). A private map's
//! sectrees share its base map's attributes (`SECTREE_MAP::SECTREE_MAP(SECTREE_MAP &)`,
//! `G/sectree_manager.cpp:41-59`, calls `SECTREE::CloneAttribute`, which copies the pointer,
//! `G/sectree.cpp:184-188`), so such a write on either reaches both. Buildings are not ported
//! (`sys.world.objects`), so nothing writes a cell and every map reads the file's attributes.

use std::collections::BTreeMap;
use std::sync::Arc;

use gamedata::server_attr::{
    SectreeGrid, ServerAttr, ATTR_BLOCK, ATTR_OBJECT, CELL_SIZE, SECTREE_SIZE,
};

/// The diagonal offsets of `aArroundCoords`' 20 rings: each ring's radius over `sqrt(2)`, rounded
/// to the nearest, as legacy writes them out.
const DIAGONALS: [i32; 20] = [
    35, 71, 106, 141, 177, 212, 247, 283, 318, 354, 389, 424, 460, 495, 530, 566, 601, 636, 672,
    707,
];

/// `ARROUND_COORD_MAX_NUM` (`G/constants.h:121`).
pub const AROUND_POINTS: usize = 161;

/// `aArroundCoords`: the offset `(0, 0)`, then for each radius from 50 to 1000 in steps of 50
/// the points `(0, r), (d, d), (r, 0), (d, -d), (0, -r), (-d, -d), (-r, 0), (-d, d)`.
pub const AROUND: [(i32, i32); AROUND_POINTS] = around();

const fn around() -> [(i32, i32); AROUND_POINTS] {
    let mut table = [(0, 0); AROUND_POINTS];
    let mut ring = 0;
    let mut radius = 0;
    while ring < DIAGONALS.len() {
        radius += 50;
        let d = DIAGONALS[ring];
        let at = 1 + ring * 8;
        table[at] = (0, radius);
        table[at + 1] = (d, d);
        table[at + 2] = (radius, 0);
        table[at + 3] = (d, -d);
        table[at + 4] = (0, -radius);
        table[at + 5] = (-d, -d);
        table[at + 6] = (-radius, 0);
        table[at + 7] = (-d, d);
        ring += 1;
    }
    table
}

/// One hosted map's cells: the sectrees it builds and the attributes its file gave them.
///
/// A dungeon instance shares its base map's attributes, as legacy's private maps share the
/// base map's `CAttribute`s, so the attributes sit behind an [`Arc`].
#[derive(Debug, Clone)]
pub struct MapCells {
    grid: SectreeGrid,
    attr: Arc<ServerAttr>,
}

impl MapCells {
    /// The cells of a map built on `grid` from its `server_attr`.
    #[must_use]
    pub fn new(grid: SectreeGrid, attr: Arc<ServerAttr>) -> Self {
        Self { grid, attr }
    }

    /// The sectrees the map builds.
    #[must_use]
    pub fn grid(&self) -> SectreeGrid {
        self.grid
    }

    /// The attribute of the cell holding a position (`SECTREE::GetAttribute`), or `None` where
    /// the map built no sectree.
    #[must_use]
    pub fn attribute(&self, x: i32, y: i32) -> Option<u32> {
        let (column, row) = self.grid.sectree_at(x, y)?;
        let Some(block) = self
            .attr
            .block(usize::try_from(column).ok()?, usize::try_from(row).ok()?)
        else {
            return Some(0);
        };
        let cell = |at: i32| usize::try_from(at % SECTREE_SIZE / CELL_SIZE).ok();
        Some(block.get(cell(x)?, cell(y)?))
    }

    /// Whether the cell holding a position sets any of `flags` (`SECTREE::IsAttr`); a position
    /// with no sectree sets none.
    #[must_use]
    pub fn is_attr(&self, x: i32, y: i32, flags: u32) -> bool {
        self.attribute(x, y).is_some_and(|attr| attr & flags != 0)
    }

    /// `SECTREE_MANAGER::IsMovablePosition`: the position has a sectree, and its cell is neither
    /// blocked nor built on.
    #[must_use]
    pub fn is_movable(&self, x: i32, y: i32) -> bool {
        self.attribute(x, y)
            .is_some_and(|attr| attr & (ATTR_BLOCK | ATTR_OBJECT) == 0)
    }

    /// `SECTREE_MANAGER::GetMovablePosition`: the first movable point of [`AROUND`] offset
    /// from a position, or `None` when none of the 161 is.
    #[must_use]
    pub fn movable_position(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        AROUND.iter().find_map(|&(dx, dy)| {
            let point = (x.checked_add(dx)?, y.checked_add(dy)?);
            self.is_movable(point.0, point.1).then_some(point)
        })
    }
}

/// The first map index legacy gives a private map: `CreatePrivateMap` numbers the instances of
/// map `m` from `m * 10000` (`G/sectree_manager.cpp:943-1026`).
pub const PRIVATE_MAP_BASE: i32 = 10_000;

/// The cells of every map a process hosts, by map index.
#[derive(Debug, Clone, Default)]
pub struct HostedCells {
    maps: BTreeMap<i32, MapCells>,
}

impl HostedCells {
    /// Host a map's cells, answering the cells it replaces.
    pub fn insert(&mut self, index: i32, cells: MapCells) -> Option<MapCells> {
        self.maps.insert(index, cells)
    }

    /// The cells of a map index. A private map's index (10000 and up) reads its base map's
    /// cells, which its sectrees are cloned from.
    #[must_use]
    pub fn get(&self, index: i32) -> Option<&MapCells> {
        let base = if index >= PRIVATE_MAP_BASE {
            index / PRIVATE_MAP_BASE
        } else {
            index
        };
        self.maps.get(&base)
    }

    /// How many maps are hosted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.maps.len()
    }

    /// Whether no map is hosted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.maps.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gamedata::server_attr::{
        CellBlock, SectreeGrid, ServerAttr, ATTR_BANPK, ATTR_BLOCK, ATTR_OBJECT, ATTR_WATER,
        SECTREE_CELLS,
    };

    use super::{HostedCells, MapCells, AROUND};
    use crate::character::legacy_source::{legacy, span, strip_comments};

    /// A map of 2 x 1 sectrees from sectree (1, 2), whose file covers only the first.
    fn cells(block: CellBlock) -> MapCells {
        let grid = SectreeGrid {
            x: 1,
            y: 2,
            columns: 2,
            rows: 1,
        };
        let attr = ServerAttr::from_blocks(1, 1, vec![block]).expect("one block");
        MapCells::new(grid, Arc::new(attr))
    }

    /// A block whose cells are all `fill`, except `set`.
    fn block(fill: u32, set: &[((usize, usize), u32)]) -> CellBlock {
        let mut cells = vec![fill; SECTREE_CELLS * SECTREE_CELLS];
        for &((x, y), value) in set {
            cells[y * SECTREE_CELLS + x] = value;
        }
        CellBlock::from_cells(&cells)
    }

    /// The centre of cell `(x, y)` of the map's first sectree.
    fn centre(x: i32, y: i32) -> (i32, i32) {
        (6400 + x * 50 + 25, 12800 + y * 50 + 25)
    }

    #[test]
    fn walks_legacys_table_of_points_around() {
        let text = strip_comments(&legacy("game/constants.cpp"));
        let table: Vec<(i32, i32)> = span(&text, "Coord aArroundCoords[", "};")
            .iter()
            .filter_map(|line| {
                let inner = line.trim().strip_prefix('{')?.trim_end_matches(',');
                let (x, y) = inner.strip_suffix('}')?.split_once(',')?;
                Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
            })
            .collect();
        assert_eq!(table.len(), 161);
        assert_eq!(table, AROUND);
    }

    #[test]
    fn reads_the_cell_holding_a_position() {
        let dwords: Vec<u32> = (0..u32::try_from(SECTREE_CELLS * SECTREE_CELLS).expect("small"))
            .map(|at| 0x1_0000 + at)
            .collect();
        let map = cells(CellBlock::from_cells(&dwords));
        assert_eq!(map.attribute(6400, 12800), Some(0x1_0000));
        let (x, y) = centre(3, 5);
        assert_eq!(map.attribute(x, y), Some(0x1_0000 + 5 * 128 + 3));
        assert_eq!(map.attribute(x + 24, y - 25), Some(0x1_0000 + 5 * 128 + 3));
        assert_eq!(map.attribute(x + 25, y), Some(0x1_0000 + 5 * 128 + 4));
        assert_eq!(map.attribute(12_799, 19_199), Some(0x1_0000 + 16383));
        // The second sectree is past the file: attribute 0.
        assert_eq!(map.attribute(12_800, 12_800), Some(0));
        assert_eq!(map.attribute(19_199, 19_199), Some(0));
        // No sectree left of, right of, above or below the map, nor at a negative position.
        for (x, y) in [
            (6399, 12800),
            (19_200, 12800),
            (6400, 12_799),
            (6400, 19_200),
            (-1, -1),
        ] {
            assert_eq!(map.attribute(x, y), None, "{x}, {y}");
        }
    }

    #[test]
    fn tests_flags_on_the_cell() {
        let map = cells(block(
            0,
            &[
                ((0, 0), ATTR_BLOCK),
                ((1, 0), ATTR_OBJECT),
                ((2, 0), ATTR_WATER),
                ((3, 0), ATTR_BANPK),
                ((4, 0), ATTR_BLOCK | ATTR_BANPK),
            ],
        ));
        let at = |x| centre(x, 0);
        let movable: Vec<bool> = (0..6).map(|x| map.is_movable(at(x).0, at(x).1)).collect();
        assert_eq!(movable, [false, false, true, true, false, true]);
        let banned: Vec<bool> = (0..6)
            .map(|x| map.is_attr(at(x).0, at(x).1, ATTR_BANPK))
            .collect();
        assert_eq!(banned, [false, false, false, true, true, false]);
        assert!(map.is_attr(at(1).0, at(1).1, ATTR_BLOCK | ATTR_OBJECT));
        assert!(!map.is_attr(at(1).0, at(1).1, ATTR_BLOCK | ATTR_WATER));
        // Past the file every cell is movable; with no sectree nothing is, and no flag is set.
        assert!(map.is_movable(13_000, 13_000));
        assert!(!map.is_movable(6399, 12_800));
        assert!(!map.is_attr(6399, 12_800, u32::MAX));
    }

    #[test]
    fn takes_the_first_movable_point_around() {
        let (x, y) = centre(64, 64);
        let blocked = |free: &[(usize, usize)]| {
            let set: Vec<((usize, usize), u32)> = free.iter().map(|&cell| (cell, 0)).collect();
            cells(block(ATTR_BLOCK, &set))
        };
        // The position itself comes first.
        assert_eq!(
            blocked(&[(64, 64), (64, 65)]).movable_position(x, y),
            Some((x, y))
        );
        // Then the first ring: (0, 50), (35, 35), (50, 0), (35, -35), (0, -50) ...
        assert_eq!(
            blocked(&[(64, 65)]).movable_position(x, y),
            Some((x, y + 50))
        );
        let map = blocked(&[(65, 63), (64, 63)]);
        assert_eq!(map.movable_position(x, y), Some((x + 35, y - 35)));
        let map = blocked(&[(64, 63), (63, 64)]);
        assert_eq!(map.movable_position(x, y), Some((x, y - 50)));
        // The last point of the last ring, (-707, 707).
        let map = blocked(&[(50, 78)]);
        assert_eq!(map.movable_position(x, y), Some((x - 707, y + 707)));
        assert_eq!(blocked(&[]).movable_position(x, y), None);
        // A point off the map is skipped: from the first cell, (35, -35) to (-35, 35) of the
        // first ring leave the map above or to the left, and (0, 100) is cell (0, 2).
        let map = cells(block(ATTR_OBJECT, &[((0, 2), 0)]));
        let (x, y) = centre(0, 0);
        assert_eq!(map.movable_position(x, y), Some((x, y + 100)));
        let map = cells(block(ATTR_OBJECT, &[((0, 1), 0)]));
        assert_eq!(map.movable_position(x, y), Some((x, y + 50)));
        // A sum past i32 is skipped too.
        assert_eq!(map.movable_position(i32::MAX, y), None);
    }

    #[test]
    fn a_private_map_reads_its_base_maps_cells() {
        let mut hosted = HostedCells::default();
        assert!(hosted.is_empty());
        assert!(hosted.insert(72, cells(block(ATTR_BLOCK, &[]))).is_none());
        assert!(hosted.insert(1, cells(block(0, &[]))).is_none());
        assert_eq!(hosted.len(), 2);
        let (x, y) = centre(3, 3);
        for (index, movable) in [(72, false), (720_000, false), (729_999, false), (1, true)] {
            let map = hosted.get(index).expect("hosted");
            assert_eq!(map.is_movable(x, y), movable, "{index}");
        }
        // 10000 is the first private index of map 1; 9999 and 73 are not hosted.
        assert!(hosted.get(10_000).is_some_and(|map| map.is_movable(x, y)));
        for index in [9_999, 73, 0, -1, 730_000] {
            assert!(hosted.get(index).is_none(), "{index}");
        }
        assert!(hosted.insert(1, cells(block(ATTR_BLOCK, &[]))).is_some());
        assert!(!hosted.get(1).expect("hosted").is_movable(x, y));
    }
}
