//! The `server_attr` reader: the attribute of every cell of a map.
//!
//! Legacy `SECTREE_MANAGER::LoadAttribute` (`server/server/game/sectree_manager.cpp:392-491`)
//! reads a map folder's `server_attr` after building the map's sectrees. The file is two
//! little-endian `int`s, the sectree columns and rows, then one block per sectree, rows outer and
//! columns inner: a `u32` byte count and that many bytes of one LZO1X stream. Each stream holds
//! the sectree's 128 x 128 cells of 50 x 50 units as `DWORD`s, rows outer. Block `(x, y)` belongs
//! to the sectree `(base_x / 6400 + x, base_y / 6400 + y)`.
//!
//! Each block becomes a `CAttribute` (`server/server/libgame/attribute.cpp:94-147`): one value
//! when every cell holds it, otherwise the cells in the narrowest of a byte, a word or a
//! `DWORD` that holds every set bit. [`CellBlock`] keeps the same shapes, which no caller can
//! observe except by the memory they take.
//!
//! Where legacy is lenient the reader refuses the file instead, and the server does not start
//! (Divergences): legacy
//!
//! - never checks its reads, so a short file decodes whatever its buffers held before;
//! - reads a block into a 69,699-byte stack buffer without checking its length (a Defect);
//! - ignores the decoder's result, keeping a block that fails once 65,536 bytes are out;
//! - on a block of the wrong length stops reading, and a sectree after it keeps no attribute;
//! - `abort()`s on a header wider or taller than the map's sectrees, which [`load_map`] refuses;
//! - reads no block after a header with a negative width or height, so the map has no
//!   attributes;
//! - builds a map with a negative base, or with sectrees past the 16 bits `SECTREEID` keeps of
//!   each axis, under truncated ids, and one with a negative size without sectrees, which
//!   [`load_map`] refuses ([`SectreeGrid::of`]);
//! - on a missing file logs and builds the map without attributes, which [`load_map`] refuses.
//!
//! A header narrower or shorter than the map, as on the owner's map 216, leaves the sectrees past
//! it without attributes, as legacy does. Bytes after the last block are ignored, as legacy
//! ignores them.

use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::lzo::{self, LzoError};
use crate::map_atlas::MapRegion;

/// A sectree's side in position units (`SECTREE_SIZE`).
pub const SECTREE_SIZE: i32 = 6400;

/// A cell's side in position units (`CELL_SIZE`).
pub const CELL_SIZE: i32 = 50;

/// A sectree's side in cells.
pub const SECTREE_CELLS: usize = 128;

/// `ATTR_BLOCK`: nothing may stand on the cell.
pub const ATTR_BLOCK: u32 = 1;
/// `ATTR_WATER`: the cell is water.
pub const ATTR_WATER: u32 = 1 << 1;
/// `ATTR_BANPK`: nobody on the cell may fight.
pub const ATTR_BANPK: u32 = 1 << 2;
/// `ATTR_OBJECT`: a building stands on the cell.
pub const ATTR_OBJECT: u32 = 1 << 7;

/// The cells in one block.
const BLOCK_CELLS: usize = SECTREE_CELLS * SECTREE_CELLS;

/// The bytes one block decodes to.
const BLOCK_BYTES: usize = BLOCK_CELLS * 4;

/// Legacy's input buffer: `LZOManager::GetMaxCompressedSize(65536)`
/// (`server/server/game/lzo_manager.cpp:42-45`).
pub const MAX_BLOCK_BYTES: usize = BLOCK_BYTES + (BLOCK_BYTES >> 4) + 64 + 3;

/// The output room legacy gives the decoder: `sizeof(DWORD) * maxMemSize`
/// (`sectree_manager.cpp:465`).
const DECODE_CAPACITY: usize = 4 * MAX_BLOCK_BYTES;

/// One sectree's cells, as `CAttribute` keeps them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellBlock {
    /// Every cell holds the value.
    Uniform(u32),
    /// No cell sets a bit above the eighth, rows outer.
    Bytes(Box<[u8]>),
    /// No cell sets a bit above the sixteenth, rows outer.
    Words(Box<[u16]>),
    /// The cells, rows outer.
    Dwords(Box<[u32]>),
}

impl CellBlock {
    /// Keep a block's cells in the narrowest shape that holds them.
    #[must_use]
    pub fn from_cells(cells: &[u32]) -> Self {
        let Some(&first) = cells.first() else {
            return Self::Uniform(0);
        };
        if cells.iter().all(|&cell| cell == first) {
            return Self::Uniform(first);
        }
        let bits = cells.iter().fold(0, |bits, &cell| bits | cell);
        if bits & 0xffff_ff00 == 0 {
            Self::Bytes(cells.iter().map(|cell| cell.to_le_bytes()[0]).collect())
        } else if bits & 0xffff_0000 == 0 {
            Self::Words(
                cells
                    .iter()
                    .map(|cell| {
                        let [low, high, _, _] = cell.to_le_bytes();
                        u16::from_le_bytes([low, high])
                    })
                    .collect(),
            )
        } else {
            Self::Dwords(cells.into())
        }
    }

    /// The attribute of cell `(x, y)` of the sectree, or 0 for a cell past its side.
    ///
    /// Legacy `CAttribute::Get` tests `x > width`, so a column of 128 reads the next row's first
    /// cell, or one past the cells on row 127, and row 128 follows a row pointer read one past
    /// the block's row table (a Defect); a block of one value answers it. No caller passes one:
    /// `SECTREE::GetAttribute` asks for `(x % 6400) / 50`, at most 127.
    #[must_use]
    pub fn get(&self, x: usize, y: usize) -> u32 {
        if x >= SECTREE_CELLS || y >= SECTREE_CELLS {
            return 0;
        }
        let at = y * SECTREE_CELLS + x;
        match self {
            Self::Uniform(value) => *value,
            Self::Bytes(cells) => cells.get(at).copied().map_or(0, u32::from),
            Self::Words(cells) => cells.get(at).copied().map_or(0, u32::from),
            Self::Dwords(cells) => cells.get(at).copied().unwrap_or(0),
        }
    }
}

/// A map's cell attributes, one block per sectree the file covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAttr {
    columns: usize,
    rows: usize,
    blocks: Vec<CellBlock>,
}

impl ServerAttr {
    /// Read a `server_attr` file's bytes.
    ///
    /// # Errors
    ///
    /// [`ServerAttrFault`] for a file legacy would misread.
    pub fn parse(bytes: &[u8]) -> Result<Self, ServerAttrFault> {
        let mut reader = Reader { bytes, at: 0 };
        let (Some(columns), Some(rows)) = (reader.int(), reader.int()) else {
            return Err(ServerAttrFault::ShortHeader);
        };
        let (Ok(width), Ok(height)) = (usize::try_from(columns), usize::try_from(rows)) else {
            return Err(ServerAttrFault::NegativeSize { columns, rows });
        };
        let count = width.saturating_mul(height);
        let mut blocks = Vec::with_capacity(count.min(bytes.len() / 4));
        for block in 0..count {
            let size = reader
                .u32()
                .and_then(|size| usize::try_from(size).ok())
                .ok_or(ServerAttrFault::ShortBlock { block })?;
            if size > MAX_BLOCK_BYTES {
                return Err(ServerAttrFault::Oversized { block, size });
            }
            let stream = reader
                .take(size)
                .ok_or(ServerAttrFault::ShortBlock { block })?;
            let cells = lzo::decompress(stream, DECODE_CAPACITY)
                .map_err(|error| ServerAttrFault::Corrupt { block, error })?;
            if cells.len() != BLOCK_BYTES {
                return Err(ServerAttrFault::WrongLength {
                    block,
                    length: cells.len(),
                });
            }
            let cells: Vec<u32> = cells
                .chunks_exact(4)
                .map(|cell| u32::from_le_bytes([cell[0], cell[1], cell[2], cell[3]]))
                .collect();
            blocks.push(CellBlock::from_cells(&cells));
        }
        Ok(Self {
            columns: width,
            rows: height,
            blocks,
        })
    }

    /// Attributes made of `blocks`, rows outer and columns inner, or `None` unless there is one
    /// block for each of `columns` x `rows` sectrees.
    #[must_use]
    pub fn from_blocks(columns: usize, rows: usize, blocks: Vec<CellBlock>) -> Option<Self> {
        (columns.checked_mul(rows) == Some(blocks.len())).then_some(Self {
            columns,
            rows,
            blocks,
        })
    }

    /// Read a `server_attr` file.
    ///
    /// # Errors
    ///
    /// [`ServerAttrError`] naming the file.
    pub fn load(path: &Path) -> Result<Self, ServerAttrError> {
        let bytes = std::fs::read(path).map_err(|source| ServerAttrError::Io {
            path: path.to_path_buf(),
            message: source.to_string(),
        })?;
        Self::parse(&bytes).map_err(|fault| ServerAttrError::Invalid {
            path: path.to_path_buf(),
            fault,
        })
    }

    /// The sectree columns the file covers.
    #[must_use]
    pub fn columns(&self) -> usize {
        self.columns
    }

    /// The sectree rows the file covers.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// The block of the sectree `column` right of and `row` below the map's first, or `None`
    /// past the file.
    #[must_use]
    pub fn block(&self, column: usize, row: usize) -> Option<&CellBlock> {
        if column >= self.columns || row >= self.rows {
            return None;
        }
        self.blocks.get(row * self.columns + column)
    }
}

/// The sectrees legacy `BuildSectreeFromSetting` (`sectree_manager.cpp:224-262`) builds for a
/// map: from `(base_x / 6400, base_y / 6400)`, one for every 6400 units of its width and height,
/// counting a part.
///
/// Legacy also builds up to two trees past the map's corner, meant for a width or height that is
/// not a whole number of sectrees; one misses by a column, and a typo at `:257` puts the other at
/// a row taken from the base's x. Neither is in the map, neither gets an attribute, and the
/// Rewrite builds neither (a provisional Divergence, owner question 10). On a map whose base is
/// not a multiple of 6400 the last column and row the map reaches therefore have no sectree, as
/// in legacy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectreeGrid {
    /// The first sectree's column (`base_x / 6400`).
    pub x: u32,
    /// The first sectree's row (`base_y / 6400`).
    pub y: u32,
    /// The sectree columns.
    pub columns: u32,
    /// The sectree rows.
    pub rows: u32,
}

impl SectreeGrid {
    /// The grid of a map region, or `None` for a region legacy's sectree ids cannot place: a
    /// negative base, or sectrees past the 16 bits `SECTREEID` keeps of each axis.
    #[must_use]
    pub fn of(region: &MapRegion) -> Option<Self> {
        let side = u32::try_from(SECTREE_SIZE).ok()?;
        let base_x = u32::try_from(region.sx).ok()?;
        let base_y = u32::try_from(region.sy).ok()?;
        let width = u32::try_from(region.ex.checked_sub(region.sx)?).ok()?;
        let height = u32::try_from(region.ey.checked_sub(region.sy)?).ok()?;
        let grid = Self {
            x: base_x / side,
            y: base_y / side,
            columns: width.div_ceil(side),
            rows: height.div_ceil(side),
        };
        let fits =
            |first: u32, count: u32| first.checked_add(count).is_some_and(|end| end <= 1 << 16);
        (fits(grid.x, grid.columns) && fits(grid.y, grid.rows)).then_some(grid)
    }

    /// The sectree holding a position, as `(column, row)` from the grid's first, or `None` where
    /// the map built none.
    #[must_use]
    pub fn sectree_at(&self, x: i32, y: i32) -> Option<(u32, u32)> {
        let side = u32::try_from(SECTREE_SIZE).ok()?;
        let column = (u32::try_from(x).ok()? / side).checked_sub(self.x)?;
        let row = (u32::try_from(y).ok()? / side).checked_sub(self.y)?;
        (column < self.columns && row < self.rows).then_some((column, row))
    }
}

/// A hosted map's `server_attr`, checked against the sectrees the map builds.
///
/// # Errors
///
/// [`ServerAttrError`] for a missing or refused file, a map whose sectrees cannot be placed
/// ([`SectreeGrid::of`]), or a file covering more sectrees than the map builds.
pub fn load_map(map_dir: &Path, region: &MapRegion) -> Result<ServerAttr, ServerAttrError> {
    let path = map_dir
        .join(OsStr::from_bytes(&region.name))
        .join("server_attr");
    let invalid = |fault| ServerAttrError::Invalid {
        path: path.clone(),
        fault,
    };
    let grid = SectreeGrid::of(region).ok_or_else(|| invalid(ServerAttrFault::Unplaced))?;
    let attr = ServerAttr::load(&path)?;
    let fits = |covered: usize, built: u32| u32::try_from(covered).is_ok_and(|n| n <= built);
    if !fits(attr.columns, grid.columns) || !fits(attr.rows, grid.rows) {
        return Err(invalid(ServerAttrFault::PastMap {
            columns: attr.columns,
            rows: attr.rows,
            grid,
        }));
    }
    Ok(attr)
}

/// Why a `server_attr` file was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerAttrFault {
    /// Fewer than eight bytes: no header.
    ShortHeader,
    /// A negative column or row count.
    NegativeSize {
        /// The columns read.
        columns: i32,
        /// The rows read.
        rows: i32,
    },
    /// The file ends inside a block, counted from 0.
    ShortBlock {
        /// The block.
        block: usize,
    },
    /// A block longer than legacy's buffer.
    Oversized {
        /// The block.
        block: usize,
        /// Its byte count.
        size: usize,
    },
    /// A block the decoder refuses.
    Corrupt {
        /// The block.
        block: usize,
        /// The decoder's fault.
        error: LzoError,
    },
    /// A block that decodes to other than 128 x 128 cells.
    WrongLength {
        /// The block.
        block: usize,
        /// The bytes it decodes to.
        length: usize,
    },
    /// A map region whose sectrees legacy's ids cannot place: a negative base, or one past
    /// 16 bits of sectree.
    Unplaced,
    /// A header covering more sectrees than the map builds.
    PastMap {
        /// The columns the file covers.
        columns: usize,
        /// The rows the file covers.
        rows: usize,
        /// The sectrees the map builds.
        grid: SectreeGrid,
    },
}

impl fmt::Display for ServerAttrFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShortHeader => f.write_str("the file is shorter than its header"),
            Self::NegativeSize { columns, rows } => {
                write!(f, "the header's size {columns} x {rows} is negative")
            }
            Self::ShortBlock { block } => write!(f, "the file ends inside block {block}"),
            Self::Oversized { block, size } => write!(
                f,
                "block {block} is {size} bytes, past legacy's {MAX_BLOCK_BYTES}"
            ),
            Self::Corrupt { block, error } => write!(f, "block {block} is corrupt: {error}"),
            Self::WrongLength { block, length } => {
                write!(
                    f,
                    "block {block} decodes to {length} bytes, not {BLOCK_BYTES}"
                )
            }
            Self::Unplaced => f.write_str("the map's sectrees are outside 16-bit sectree ids"),
            Self::PastMap {
                columns,
                rows,
                grid,
            } => write!(
                f,
                "the file covers {columns} x {rows} sectrees, past the map's {} x {}",
                grid.columns, grid.rows
            ),
        }
    }
}

impl Error for ServerAttrFault {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Corrupt { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// A `server_attr` file that could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerAttrError {
    /// The file could not be read.
    Io {
        /// The file.
        path: PathBuf,
        /// The cause.
        message: String,
    },
    /// The file was refused.
    Invalid {
        /// The file.
        path: PathBuf,
        /// Why.
        fault: ServerAttrFault,
    },
}

impl fmt::Display for ServerAttrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "{}: {message}", path.display()),
            Self::Invalid { path, fault } => write!(f, "{}: {fault}", path.display()),
        }
    }
}

impl Error for ServerAttrError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { .. } => None,
            Self::Invalid { fault, .. } => Some(fault),
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(count)?;
        let taken = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(taken)
    }

    fn int(&mut self) -> Option<i32> {
        let bytes = self.take(4)?;
        Some(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u32(&mut self) -> Option<u32> {
        let bytes = self.take(4)?;
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use super::{
        load_map, CellBlock, SectreeGrid, ServerAttr, ServerAttrError, ServerAttrFault,
        BLOCK_CELLS, MAX_BLOCK_BYTES, SECTREE_CELLS,
    };
    use crate::lzo::{LzoError, LzoFault};
    use crate::map_atlas::{MapAtlas, MapRegion};

    fn owners_map_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/locale/europe/map")
    }

    fn owners_region(index: i32) -> MapRegion {
        let atlas = MapAtlas::load(&owners_map_dir()).expect("the owner's atlas");
        atlas.region(index).expect("a hosted map").clone()
    }

    /// A block of four literals repeated by one overlapping match: every cell holds `value`.
    fn uniform_block(value: u32) -> Vec<u8> {
        let mut stream = vec![21];
        stream.extend(value.to_le_bytes());
        stream.push(32);
        stream.extend([0; 256]);
        stream.extend([219, 12, 0, 0x11, 0, 0]);
        stream
    }

    /// A block of one literal run holding `cells`.
    fn literal_block(cells: &[u32]) -> Vec<u8> {
        let mut stream = vec![0; 257];
        stream.push(238);
        stream.extend(cells.iter().flat_map(|cell| cell.to_le_bytes()));
        stream.extend([0x11, 0, 0]);
        stream
    }

    fn file(columns: i32, rows: i32, blocks: &[&[u8]]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(columns.to_le_bytes());
        bytes.extend(rows.to_le_bytes());
        for block in blocks {
            bytes.extend(
                u32::try_from(block.len())
                    .expect("a short block")
                    .to_le_bytes(),
            );
            bytes.extend(*block);
        }
        bytes
    }

    fn cells(cell: impl Fn(u32) -> u32) -> Vec<u32> {
        (0..u32::try_from(BLOCK_CELLS).expect("16384"))
            .map(cell)
            .collect()
    }

    /// FNV-1a 64 over every block's cells as the file decodes them, and the count of each
    /// value.
    fn summary(attr: &ServerAttr) -> (u64, BTreeMap<u32, u64>) {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut values = BTreeMap::new();
        for row in 0..attr.rows() {
            for column in 0..attr.columns() {
                let block = attr.block(column, row).expect("a covered sectree");
                for y in 0..SECTREE_CELLS {
                    for x in 0..SECTREE_CELLS {
                        let cell = block.get(x, y);
                        *values.entry(cell).or_insert(0) += 1;
                        for byte in cell.to_le_bytes() {
                            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
                        }
                    }
                }
            }
        }
        (hash, values)
    }

    fn shapes(attr: &ServerAttr) -> [usize; 4] {
        let mut shapes = [0; 4];
        for row in 0..attr.rows() {
            for column in 0..attr.columns() {
                let shape = match attr.block(column, row).expect("a covered sectree") {
                    CellBlock::Uniform(_) => 0,
                    CellBlock::Bytes(_) => 1,
                    CellBlock::Words(_) => 2,
                    CellBlock::Dwords(_) => 3,
                };
                shapes[shape] += 1;
            }
        }
        shapes
    }

    /// A map's `server_attr` as liblzo2 2.10's `lzo1x_decompress_safe` decodes it: its header,
    /// the blocks of each shape (uniform, bytes, words, dwords), FNV-1a 64 of its decoded bytes,
    /// and the count of each cell value.
    struct Golden {
        index: i32,
        size: (usize, usize),
        shapes: [usize; 4],
        hash: u64,
        values: &'static [(u32, u64)],
    }

    const GOLDEN: [Golden; 5] = [
        Golden {
            index: 1,
            size: (16, 20),
            shapes: [62, 258, 0, 0],
            hash: 0x9c79_b385_e4e9_93a5,
            values: &[
                (0, 2_598_968),
                (1, 1_816_772),
                (2, 9616),
                (3, 799_348),
                (4, 18112),
                (5, 64),
            ],
        },
        Golden {
            index: 3,
            size: (16, 16),
            shapes: [40, 216, 0, 0],
            hash: 0x2aaf_4b28_72ee_2605,
            values: &[
                (72, 1_853_100),
                (73, 1_941_828),
                (74, 8968),
                (75, 378_092),
                (76, 12128),
                (77, 188),
            ],
        },
        Golden {
            index: 216,
            size: (24, 24),
            shapes: [169, 407, 0, 0],
            hash: 0x7163_0a11_8244_80c5,
            values: &[(0, 9_229_908), (1, 207_276)],
        },
        Golden {
            index: 12,
            size: (8, 8),
            shapes: [64, 0, 0, 0],
            hash: 0xf8e3_e56c_e922_2325,
            values: &[(0, 1_048_576)],
        },
        Golden {
            index: 200,
            size: (4, 4),
            shapes: [0, 16, 0, 0],
            hash: 0x8972_6d9a_7083_4a45,
            values: &[(6, 209_812), (64, 5760), (203, 46572)],
        },
    ];

    #[test]
    fn reads_the_owners_maps_as_the_library_decodes_them() {
        for golden in &GOLDEN {
            let index = golden.index;
            let region = owners_region(index);
            let attr = load_map(&owners_map_dir(), &region).expect("the owner's server_attr");
            assert_eq!((attr.columns(), attr.rows()), golden.size, "map {index}");
            assert_eq!(shapes(&attr), golden.shapes, "map {index}");
            let (hash, counts) = summary(&attr);
            assert_eq!(hash, golden.hash, "map {index}");
            assert_eq!(
                counts,
                golden.values.iter().copied().collect(),
                "map {index}"
            );
        }
    }

    #[test]
    fn places_the_owners_sectrees() {
        // Map 1 starts on a sectree boundary and spans whole sectrees.
        let a1 = owners_region(1);
        let grid = SectreeGrid::of(&a1).expect("a placed map");
        assert_eq!((grid.columns, grid.rows), (16, 20));
        assert_eq!(grid.sectree_at(a1.sx, a1.sy), Some((0, 0)));
        assert_eq!(grid.sectree_at(a1.ex - 1, a1.ey - 1), Some((15, 19)));
        assert_eq!(grid.sectree_at(a1.ex, a1.sy), None);
        assert_eq!(grid.sectree_at(a1.sx - 1, a1.sy), None);
        assert_eq!(grid.sectree_at(a1.sx, a1.sy - 1), None);
        assert_eq!(grid.sectree_at(-1, a1.sy), None);

        // Map 12's base (2100000, 1600000) is 800 and 0 units past a sectree boundary: its
        // first sectree starts 800 units left of it, and the last 800 units of its width have
        // none.
        let moon = owners_region(12);
        assert_eq!((moon.sx, moon.ex), (2_100_000, 2_151_200));
        let grid = SectreeGrid::of(&moon).expect("a placed map");
        assert_eq!((grid.x, grid.y, grid.columns, grid.rows), (328, 250, 8, 8));
        assert_eq!(grid.sectree_at(moon.sx, moon.sy), Some((0, 0)));
        assert_eq!(grid.sectree_at(2_150_399, moon.sy), Some((7, 0)));
        assert_eq!(grid.sectree_at(2_150_400, moon.sy), None);
        assert_eq!(grid.sectree_at(moon.ex - 1, moon.sy), None);

        // Map 216's file covers 24 x 24 of the 32 x 32 sectrees it builds.
        let catacomb = owners_region(216);
        let grid = SectreeGrid::of(&catacomb).expect("a placed map");
        assert_eq!((grid.columns, grid.rows), (32, 32));
        let attr = load_map(&owners_map_dir(), &catacomb).expect("the owner's server_attr");
        assert!(attr.block(23, 23).is_some());
        assert_eq!(attr.block(24, 0), None);
        assert_eq!(attr.block(0, 24), None);

        // Sectree ids keep 16 bits of each axis: the last sectree a map may take is 65535,
        // which ends at 419,430,399.
        let mut edge = owners_region(1);
        (edge.sx, edge.ex, edge.sy, edge.ey) = (419_424_000, 419_430_400, 0, 6400);
        let grid = SectreeGrid::of(&edge).expect("the last sectree column");
        assert_eq!((grid.x, grid.columns), (65535, 1));
        assert_eq!(grid.sectree_at(419_430_399, 0), Some((0, 0)));
        assert_eq!(grid.sectree_at(419_430_400, 0), None);
        edge.ex += 1;
        assert_eq!(SectreeGrid::of(&edge), None);
        (edge.sx, edge.ex, edge.sy, edge.ey) = (0, 6400, 419_424_000, 419_430_401);
        assert_eq!(SectreeGrid::of(&edge), None);
        edge.ey -= 1;
        assert!(SectreeGrid::of(&edge).is_some());
        (edge.sx, edge.ex) = (6400, 0);
        assert_eq!(SectreeGrid::of(&edge), None);
    }

    #[test]
    fn refuses_a_file_past_the_maps_sectrees() {
        let mut region = owners_region(1);
        region.ex -= 6400;
        let grid = SectreeGrid::of(&region).expect("a placed map");
        let error = load_map(&owners_map_dir(), &region).expect_err("a header past the map");
        let ServerAttrError::Invalid { path, fault } = error else {
            panic!("a refused file");
        };
        assert!(path.ends_with("metin2_map_a1/server_attr"));
        assert_eq!(
            fault,
            ServerAttrFault::PastMap {
                columns: 16,
                rows: 20,
                grid
            }
        );
        assert_eq!(
            fault.to_string(),
            "the file covers 16 x 20 sectrees, past the map's 15 x 20"
        );
        let mut region = owners_region(1);
        region.ey -= 1;
        assert!(load_map(&owners_map_dir(), &region).is_ok());
        region.ey -= 6400;
        assert!(load_map(&owners_map_dir(), &region).is_err());

        let mut region = owners_region(1);
        region.sx = -6400;
        let error = load_map(&owners_map_dir(), &region).expect_err("a negative base");
        assert!(matches!(
            error,
            ServerAttrError::Invalid {
                fault: ServerAttrFault::Unplaced,
                ..
            }
        ));
    }

    #[test]
    fn refuses_a_missing_file() {
        let mut region = owners_region(1);
        region.name = b"no_such_map".to_vec();
        let error = load_map(&owners_map_dir(), &region).expect_err("a missing file");
        let ServerAttrError::Io { path, .. } = &error else {
            panic!("an unreadable file");
        };
        assert!(path.ends_with("no_such_map/server_attr"));
        assert!(error.to_string().contains("no_such_map/server_attr: "));
    }

    #[test]
    fn keeps_each_block_in_the_narrowest_shape() {
        let bytes = cells(|at| at % 7);
        let words = cells(|at| at * 3 % 0x1_0000);
        let dwords = cells(|at| at * 0x1_0001);
        let blocks = [
            uniform_block(5),
            literal_block(&bytes),
            literal_block(&words),
            literal_block(&dwords),
        ];
        let refs: Vec<&[u8]> = blocks.iter().map(Vec::as_slice).collect();
        let attr = ServerAttr::parse(&file(2, 2, &refs)).expect("a valid file");
        assert_eq!((attr.columns(), attr.rows()), (2, 2));
        assert_eq!(attr.block(0, 0), Some(&CellBlock::Uniform(5)));
        assert!(matches!(attr.block(1, 0), Some(CellBlock::Bytes(_))));
        assert!(matches!(attr.block(0, 1), Some(CellBlock::Words(_))));
        assert!(matches!(attr.block(1, 1), Some(CellBlock::Dwords(_))));
        assert_eq!(attr.block(2, 0), None);
        assert_eq!(attr.block(0, 2), None);
        for (column, row, expected) in [(1, 0, &bytes), (0, 1, &words), (1, 1, &dwords)] {
            let block = attr.block(column, row).expect("a block");
            for (at, &cell) in expected.iter().enumerate() {
                assert_eq!(block.get(at % SECTREE_CELLS, at / SECTREE_CELLS), cell);
            }
        }
        let block = attr.block(1, 1).expect("a block");
        assert_eq!(block.get(127, 127), 16383 * 0x1_0001);
        assert_eq!(block.get(128, 0), 0);
        assert_eq!(block.get(0, 128), 0);
        let uniform = attr.block(0, 0).expect("a block");
        assert_eq!(uniform.get(127, 127), 5);
        assert_eq!(uniform.get(128, 0), 0);
        assert_eq!(uniform.get(0, 128), 0);
    }

    #[test]
    fn picks_the_shape_by_the_bits_every_cell_sets() {
        assert_eq!(CellBlock::from_cells(&[]), CellBlock::Uniform(0));
        assert_eq!(CellBlock::from_cells(&[7, 7, 7]), CellBlock::Uniform(7));
        assert_eq!(
            CellBlock::from_cells(&[0, 0xff]),
            CellBlock::Bytes(Box::new([0, 0xff]))
        );
        assert_eq!(
            CellBlock::from_cells(&[0, 0x100]),
            CellBlock::Words(Box::new([0, 0x100]))
        );
        assert_eq!(
            CellBlock::from_cells(&[1, 0xffff]),
            CellBlock::Words(Box::new([1, 0xffff]))
        );
        assert_eq!(
            CellBlock::from_cells(&[0, 0x1_0000]),
            CellBlock::Dwords(Box::new([0, 0x1_0000]))
        );
        assert_eq!(
            CellBlock::from_cells(&[0x8000_0000, 1]),
            CellBlock::Dwords(Box::new([0x8000_0000, 1]))
        );
    }

    #[test]
    fn builds_from_one_block_per_sectree() {
        let blocks = || vec![CellBlock::Uniform(1), CellBlock::Uniform(2)];
        let attr = ServerAttr::from_blocks(2, 1, blocks()).expect("two blocks");
        assert_eq!(attr.block(1, 0), Some(&CellBlock::Uniform(2)));
        let attr = ServerAttr::from_blocks(1, 2, blocks()).expect("two blocks");
        assert_eq!(attr.block(0, 1), Some(&CellBlock::Uniform(2)));
        assert_eq!(ServerAttr::from_blocks(1, 1, blocks()), None);
        assert_eq!(ServerAttr::from_blocks(3, 1, blocks()), None);
        assert_eq!(ServerAttr::from_blocks(usize::MAX, 2, blocks()), None);
        assert!(ServerAttr::from_blocks(0, 0, Vec::new()).is_some());
    }

    #[test]
    fn ignores_bytes_after_the_last_block() {
        let mut bytes = file(1, 1, &[&uniform_block(1)]);
        bytes.extend([1, 2, 3]);
        let attr = ServerAttr::parse(&bytes).expect("a valid file");
        assert_eq!(attr.block(0, 0), Some(&CellBlock::Uniform(1)));
        let attr = ServerAttr::parse(&file(0, 0, &[])).expect("an empty file");
        assert_eq!(
            (attr.columns(), attr.rows(), attr.block(0, 0)),
            (0, 0, None)
        );
        let attr = ServerAttr::parse(&file(0, 7, &[])).expect("an empty file");
        assert_eq!((attr.columns(), attr.rows()), (0, 7));
        let attr = ServerAttr::parse(&file(7, 0, &[])).expect("an empty file");
        assert_eq!(
            (attr.columns(), attr.rows(), attr.block(0, 0)),
            (7, 0, None)
        );
    }

    #[test]
    fn refuses_what_legacy_would_misread() {
        let parse = |bytes: &[u8]| ServerAttr::parse(bytes).expect_err("a refused file");
        assert_eq!(parse(&[0; 7]), ServerAttrFault::ShortHeader);
        assert_eq!(
            parse(&file(-1, 1, &[])),
            ServerAttrFault::NegativeSize {
                columns: -1,
                rows: 1
            }
        );
        assert_eq!(
            parse(&file(1, -2, &[])),
            ServerAttrFault::NegativeSize {
                columns: 1,
                rows: -2
            }
        );
        assert_eq!(
            parse(&file(1, 1, &[])),
            ServerAttrFault::ShortBlock { block: 0 }
        );
        let mut short_size = file(1, 1, &[]);
        short_size.extend([1, 0, 0]);
        assert_eq!(parse(&short_size), ServerAttrFault::ShortBlock { block: 0 });
        let uniform = uniform_block(2);
        let mut cut = file(2, 1, &[&uniform, &uniform]);
        cut.pop();
        assert_eq!(parse(&cut), ServerAttrFault::ShortBlock { block: 1 });

        // Legacy's buffer holds 69,699 bytes: a longer block is refused before it is read.
        let mut oversized = file(1, 1, &[]);
        oversized.extend(
            u32::try_from(MAX_BLOCK_BYTES + 1)
                .expect("small")
                .to_le_bytes(),
        );
        oversized.extend(vec![0; MAX_BLOCK_BYTES + 1]);
        assert_eq!(
            parse(&oversized),
            ServerAttrFault::Oversized {
                block: 0,
                size: 69_700
            }
        );
        let mut largest = file(1, 1, &[]);
        largest.extend(u32::try_from(MAX_BLOCK_BYTES).expect("small").to_le_bytes());
        assert_eq!(parse(&largest), ServerAttrFault::ShortBlock { block: 0 });
        let mut padded = literal_block(&cells(|at| at % 3));
        padded.resize(MAX_BLOCK_BYTES, 0);
        assert_eq!(
            parse(&file(1, 1, &[&padded])),
            ServerAttrFault::Corrupt {
                block: 0,
                error: LzoError {
                    fault: LzoFault::InputNotConsumed,
                    written: 65536
                }
            }
        );

        let fault = parse(&file(2, 1, &[&uniform, b"\x11\x00\x00\x00"]));
        assert_eq!(
            fault,
            ServerAttrFault::Corrupt {
                block: 1,
                error: LzoError {
                    fault: LzoFault::InputNotConsumed,
                    written: 0
                }
            }
        );
        assert_eq!(
            fault.to_string(),
            "block 1 is corrupt: bytes follow the end of the stream (after 0 bytes out)"
        );
        assert!(std::error::Error::source(&fault).is_some());

        assert_eq!(
            parse(&file(1, 1, &[b"\x15abcd\x11\x00\x00"])),
            ServerAttrFault::WrongLength {
                block: 0,
                length: 4
            }
        );
        let mut long = literal_block(&cells(|_| 9));
        long[257] = 239;
        long.insert(258, 9);
        assert_eq!(
            parse(&file(1, 1, &[&long])),
            ServerAttrFault::WrongLength {
                block: 0,
                length: 65537
            }
        );
    }
}
