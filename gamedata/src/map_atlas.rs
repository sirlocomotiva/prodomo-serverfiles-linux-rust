//! The map atlas: every map's region and town spawn, read from the Game data
//! map folder.
//!
//! Legacy `SECTREE_MANAGER::Build` (`server/server/game/sectree_manager.cpp:
//! 691-775`) reads the `index` file, then each listed map's `Setting.txt`
//! (`LoadSettingFile`, `:177-221`) and `Town.txt` (`LoadMapRegion`,
//! `:315-390`), and keeps one region per line in file order. It builds the
//! region of every listed map, whether or not the process hosts it, and
//! `GetMapIndex` (`:666-687`) answers with the first region that holds a
//! position. This module keeps the same parse, the same order, and the same
//! first-match lookup.
//!
//! The files are read byte-exact. Lines are cut the way `fgets(buf, 256, fp)`
//! cuts them: at a `\n`, or after 255 bytes. Numbers are read the way
//! `sscanf`'s `%d` reads them: C whitespace is skipped, then an optional sign
//! and decimal digits, stopping at the first other byte.

use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// The widest line `fgets(buf, 256, fp)` returns, in bytes.
const FGETS_LINE_BYTES: usize = 255;

/// Legacy `CellScale * 128` is the width of one map cell in position units.
const CELL_UNITS: i32 = 128;

/// Legacy `Town.txt` coordinates are in metres; positions are centimetres.
const TOWN_UNIT: i32 = 100;

/// One map region as legacy `TMapRegion` held it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapRegion {
    /// The map index from the `index` file.
    pub index: i32,
    /// The map folder name, as the exact bytes of the `index` file.
    pub name: Vec<u8>,
    /// Left edge, inclusive.
    pub sx: i32,
    /// Top edge, inclusive.
    pub sy: i32,
    /// Right edge, exclusive.
    pub ex: i32,
    /// Bottom edge, exclusive.
    pub ey: i32,
    /// The town spawn position.
    pub spawn: (i32, i32),
    /// The per-empire town spawn positions (empires 1, 2, 3), when
    /// `Town.txt` lists all six numbers.
    pub empire_spawns: Option<[(i32, i32); 3]>,
}

impl MapRegion {
    /// Whether the region holds a position, as `GetMapIndex` tests it.
    #[must_use]
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.sx && y >= self.sy && x < self.ex && y < self.ey
    }
}

/// What `Setting.txt` contributes to a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapSetting {
    /// `BasePosition` x.
    pub base_x: i32,
    /// `BasePosition` y.
    pub base_y: i32,
    /// `CellScale * 128 * MapSize` width.
    pub width: i32,
    /// `CellScale * 128 * MapSize` height.
    pub height: i32,
}

/// What `Town.txt` contributes to a region, in metres from the base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TownFile {
    /// The town spawn offset.
    pub spawn: (i32, i32),
    /// The per-empire offsets, when all six numbers are present.
    pub empires: Option<[(i32, i32); 3]>,
}

/// Every map region in `index` file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapAtlas {
    regions: Vec<MapRegion>,
}

/// A map folder that could not be read into an atlas.
#[derive(Debug)]
pub enum MapAtlasError {
    /// A file could not be read.
    Io {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// An `index` line held a map index but no map name. Legacy would reuse
    /// the previous line's name, or read an uninitialised buffer.
    IndexLineWithoutName {
        /// The 1-based `fgets` line.
        line: usize,
    },
    /// A `Setting.txt` has no size or no cell scale.
    InvalidSetting {
        /// The file.
        path: PathBuf,
    },
    /// A size or position does not fit a 32-bit C `int`.
    OutOfRange {
        /// The file.
        path: PathBuf,
    },
}

impl fmt::Display for MapAtlasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::IndexLineWithoutName { line } => {
                write!(f, "map index line {line} has an index but no map name")
            }
            Self::InvalidSetting { path } => {
                write!(f, "{} has no map size or no cell scale", path.display())
            }
            Self::OutOfRange { path } => {
                write!(
                    f,
                    "{} holds a size or position out of range",
                    path.display()
                )
            }
        }
    }
}

impl Error for MapAtlasError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl MapAtlas {
    /// Read `index` and every listed map's `Setting.txt` and `Town.txt` from
    /// a map folder such as `legacy/gamedata/locale/europe/map`.
    ///
    /// # Errors
    ///
    /// Any unreadable file, an `index` line without a name, an invalid
    /// `Setting.txt`, or a value outside a C `int`. Legacy `Build` fails the
    /// same way on a missing or invalid `Setting.txt` or a missing `Town.txt`.
    pub fn load(map_dir: &Path) -> Result<Self, MapAtlasError> {
        let index_path = map_dir.join("index");
        let index = read(&index_path)?;
        let mut regions = Vec::new();
        for (map_index, name) in parse_index(&index)? {
            let folder = map_dir.join(OsStr::from_bytes(&name));
            let setting_path = folder.join("Setting.txt");
            let setting = parse_setting(&read(&setting_path)?)
                .map_err(|kind| kind.at(setting_path.clone()))?;
            let town_path = folder.join("Town.txt");
            let town = parse_town(&read(&town_path)?);
            let region = build_region(map_index, name, setting, town)
                .ok_or(MapAtlasError::OutOfRange { path: town_path })?;
            regions.push(region);
        }
        Ok(Self { regions })
    }

    /// An atlas made of the given regions, in lookup order.
    #[must_use]
    pub fn from_regions(regions: Vec<MapRegion>) -> Self {
        Self { regions }
    }

    /// Every region in `index` file order.
    #[must_use]
    pub fn regions(&self) -> &[MapRegion] {
        &self.regions
    }

    /// The index of the first region holding a position, as legacy
    /// `GetMapIndex` answers (it answers 0 where this answers `None`).
    #[must_use]
    pub fn index_at(&self, x: i32, y: i32) -> Option<i32> {
        self.regions
            .iter()
            .find(|region| region.contains(x, y))
            .map(|region| region.index)
    }

    /// The first region with a map index, as legacy `GetMapRegion` answers.
    #[must_use]
    pub fn region(&self, index: i32) -> Option<&MapRegion> {
        self.regions.iter().find(|region| region.index == index)
    }
}

fn read(path: &Path) -> Result<Vec<u8>, MapAtlasError> {
    std::fs::read(path).map_err(|source| MapAtlasError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Why a `Setting.txt` was refused, before its path is attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingError {
    /// No size or no cell scale.
    Invalid,
    /// The width or height does not fit a C `int`.
    OutOfRange,
}

impl SettingError {
    fn at(self, path: PathBuf) -> MapAtlasError {
        match self {
            Self::Invalid => MapAtlasError::InvalidSetting { path },
            Self::OutOfRange => MapAtlasError::OutOfRange { path },
        }
    }
}

/// Split bytes into the lines `fgets(buf, 256, fp)` returns.
fn fgets_lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = bytes;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let window = &rest[..rest.len().min(FGETS_LINE_BYTES)];
        let len = window
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(window.len(), |at| at + 1);
        let (line, tail) = rest.split_at(len);
        rest = tail;
        Some(line)
    })
}

/// Parse the `index` file into `(map index, map name)` pairs in file order.
///
/// A line without a `\n` is skipped (legacy `@fixme144`), as is a line
/// starting with `//` or `#`, and a line with no number. A NUL ends a line,
/// as it ends the C string `sscanf` reads.
///
/// # Errors
///
/// A line with an index but no name.
pub fn parse_index(bytes: &[u8]) -> Result<Vec<(i32, Vec<u8>)>, MapAtlasError> {
    let mut maps = Vec::new();
    for (number, line) in fgets_lines(bytes).enumerate() {
        let Some(line) = line.strip_suffix(b"\n") else {
            continue;
        };
        let line = c_string(line);
        if line.starts_with(b"//") || line.first() == Some(&b'#') {
            continue;
        }
        let mut scan = Scanner::new(line);
        let Some(index) = scan.int() else {
            continue;
        };
        let Some(name) = scan.word() else {
            return Err(MapAtlasError::IndexLineWithoutName { line: number + 1 });
        };
        maps.push((index, name.to_vec()));
    }
    Ok(maps)
}

/// Parse a `Setting.txt`.
///
/// Each line's first word is compared case-insensitively with `MapSize`,
/// `BasePosition`, and `CellScale`; every other line is ignored. A later
/// line overrides an earlier one, and a number that fails to scan keeps the
/// value it had, as legacy `sscanf` leaves its target untouched.
///
/// # Errors
///
/// No size (both zero), no cell scale, or a width or height outside a C
/// `int`.
pub fn parse_setting(bytes: &[u8]) -> Result<MapSetting, SettingError> {
    let (mut width, mut height, mut cell_scale) = (0_i32, 0_i32, 0_i32);
    let (mut base_x, mut base_y) = (0_i32, 0_i32);
    for line in fgets_lines(bytes) {
        let mut scan = Scanner::new(c_string(line));
        let Some(command) = scan.word() else {
            continue;
        };
        if command.eq_ignore_ascii_case(b"MapSize") {
            scan.int_into(&mut width);
            scan.int_into(&mut height);
        } else if command.eq_ignore_ascii_case(b"BasePosition") {
            scan.int_into(&mut base_x);
            scan.int_into(&mut base_y);
        } else if command.eq_ignore_ascii_case(b"CellScale") {
            scan.int_into(&mut cell_scale);
        }
    }
    if (width == 0 && height == 0) || cell_scale == 0 {
        return Err(SettingError::Invalid);
    }
    let cell = cell_scale
        .checked_mul(CELL_UNITS)
        .ok_or(SettingError::OutOfRange)?;
    Ok(MapSetting {
        base_x,
        base_y,
        width: cell.checked_mul(width).ok_or(SettingError::OutOfRange)?,
        height: cell.checked_mul(height).ok_or(SettingError::OutOfRange)?,
    })
}

/// Parse a `Town.txt`: two numbers for the town spawn, then optionally six
/// for the three empires. A missing number reads as 0, and fewer than six
/// empire numbers means no empire spawns.
#[must_use]
pub fn parse_town(bytes: &[u8]) -> TownFile {
    let mut scan = Scanner::new(c_string(bytes));
    let (mut x, mut y) = (0, 0);
    scan.int_into(&mut x);
    scan.int_into(&mut y);
    let mut empire = [0_i32; 6];
    let count = empire
        .iter_mut()
        .map(|slot| scan.int_into(slot))
        .filter(|&scanned| scanned)
        .count();
    TownFile {
        spawn: (x, y),
        empires: (count == empire.len()).then(|| {
            [
                (empire[0], empire[1]),
                (empire[2], empire[3]),
                (empire[4], empire[5]),
            ]
        }),
    }
}

/// Combine one map's files into its region, or `None` when a sum leaves a C
/// `int`.
#[must_use]
pub fn build_region(
    index: i32,
    name: Vec<u8>,
    setting: MapSetting,
    town: TownFile,
) -> Option<MapRegion> {
    let place = |(x, y): (i32, i32)| -> Option<(i32, i32)> {
        Some((
            setting.base_x.checked_add(x.checked_mul(TOWN_UNIT)?)?,
            setting.base_y.checked_add(y.checked_mul(TOWN_UNIT)?)?,
        ))
    };
    let empire_spawns = match town.empires {
        Some([a, b, c]) => Some([place(a)?, place(b)?, place(c)?]),
        None => None,
    };
    Some(MapRegion {
        index,
        name,
        sx: setting.base_x,
        sy: setting.base_y,
        ex: setting.base_x.checked_add(setting.width)?,
        ey: setting.base_y.checked_add(setting.height)?,
        spawn: place(town.spawn)?,
        empire_spawns,
    })
}

/// The bytes before the first NUL, as a C string reader sees them.
fn c_string(bytes: &[u8]) -> &[u8] {
    bytes
        .iter()
        .position(|&byte| byte == 0)
        .map_or(bytes, |end| &bytes[..end])
}

/// A cursor that reads the way `sscanf`'s `%d` and `%s` read.
struct Scanner<'a> {
    rest: &'a [u8],
}

impl<'a> Scanner<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { rest: bytes }
    }

    fn skip_space(&mut self) {
        let start = self
            .rest
            .iter()
            .position(|&byte| !is_c_space(byte))
            .unwrap_or(self.rest.len());
        self.rest = &self.rest[start..];
    }

    /// `%s`: a run of non-space bytes.
    fn word(&mut self) -> Option<&'a [u8]> {
        self.skip_space();
        let len = self
            .rest
            .iter()
            .position(|&byte| is_c_space(byte))
            .unwrap_or(self.rest.len());
        if len == 0 {
            return None;
        }
        let (word, rest) = self.rest.split_at(len);
        self.rest = rest;
        Some(word)
    }

    /// `%d`: an optional sign and decimal digits. A value outside a C `int`
    /// fails the scan rather than taking glibc's clamped value.
    fn int(&mut self) -> Option<i32> {
        self.skip_space();
        let sign = usize::from(matches!(self.rest.first(), Some(b'+' | b'-')));
        let digits = self.rest[sign..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits == 0 {
            return None;
        }
        let (number, rest) = self.rest.split_at(sign + digits);
        let value = std::str::from_utf8(number).ok()?.parse().ok()?;
        self.rest = rest;
        Some(value)
    }

    /// `%d` into a target that keeps its value when the scan fails. A failed
    /// scan does not move the cursor, so every later scan fails too, as
    /// `sscanf` stops at its first failed conversion.
    fn int_into(&mut self, target: &mut i32) -> bool {
        self.int().map(|value| *target = value).is_some()
    }
}

/// C `isspace` in the "C" locale.
fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_keeps_order_and_skips_comments_and_the_unterminated_last_line() {
        let bytes = b"1 metin2_map_a1\n// 2 skipped\n# 3 skipped\n\n358\tdefense\r\n4 tail";
        let maps = parse_index(bytes).unwrap();
        assert_eq!(
            maps,
            vec![(1, b"metin2_map_a1".to_vec()), (358, b"defense".to_vec())]
        );
    }

    #[test]
    fn index_cuts_lines_as_fgets_does() {
        // A 300-byte line: the first 255 bytes have no newline and are
        // skipped; the remaining 45 bytes are read as a line of their own.
        let mut bytes = vec![b'x'; 255];
        bytes.extend_from_slice(b"  7 cut_map");
        bytes.resize(299, b' ');
        bytes.push(b'\n');
        assert_eq!(parse_index(&bytes).unwrap(), vec![(7, b"cut_map".to_vec())]);
    }

    #[test]
    fn index_refuses_a_number_without_a_name() {
        let error = parse_index(b"1 a\n2\n").unwrap_err();
        assert!(matches!(
            error,
            MapAtlasError::IndexLineWithoutName { line: 2 }
        ));
    }

    #[test]
    fn setting_reads_the_three_keys_case_insensitively() {
        let bytes = b"ScriptType\tMapSetting\r\n\r\ncellscale\t200\r\nMAPSIZE\t4\t5\r\n\
            BasePosition\t409600\t896000\r\nTextureSet\tx.txt\r\n";
        let setting = parse_setting(bytes).unwrap();
        assert_eq!(
            setting,
            MapSetting {
                base_x: 409_600,
                base_y: 896_000,
                width: 200 * 128 * 4,
                height: 200 * 128 * 5,
            }
        );
    }

    #[test]
    fn setting_keeps_a_value_whose_scan_fails() {
        let bytes = b"CellScale 100\nMapSize 2 3\nMapSize 9 x\nBasePosition 5\n";
        let setting = parse_setting(bytes).unwrap();
        assert_eq!(
            (setting.width, setting.height),
            (100 * 128 * 9, 100 * 128 * 3)
        );
        assert_eq!((setting.base_x, setting.base_y), (5, 0));
    }

    #[test]
    fn setting_refuses_no_size_or_no_scale() {
        assert_eq!(
            parse_setting(b"CellScale 200\n"),
            Err(SettingError::Invalid)
        );
        assert_eq!(parse_setting(b"MapSize 0 1\n"), Err(SettingError::Invalid));
        assert!(parse_setting(b"CellScale 200\nMapSize 0 1\n").is_ok());
        assert_eq!(
            parse_setting(b"CellScale 20000000\nMapSize 1 1\n"),
            Err(SettingError::OutOfRange)
        );
    }

    #[test]
    fn town_reads_the_spawn_and_all_six_empire_numbers_or_none() {
        let full = parse_town(b"597 682\r\n1 2\r\n3 4\r\n-5 6\r\n");
        assert_eq!(full.spawn, (597, 682));
        assert_eq!(full.empires, Some([(1, 2), (3, 4), (-5, 6)]));
        let four = parse_town(b"700\t700\n700\t700\n");
        assert_eq!(four.spawn, (700, 700));
        assert_eq!(four.empires, None);
        let empty = parse_town(b"");
        assert_eq!(empty.spawn, (0, 0));
        assert_eq!(empty.empires, None);
    }

    #[test]
    fn region_places_spawns_in_centimetres_from_the_base() {
        let setting = MapSetting {
            base_x: 1000,
            base_y: 2000,
            width: 300,
            height: 400,
        };
        let town = TownFile {
            spawn: (1, 2),
            empires: Some([(3, 4), (5, 6), (7, 8)]),
        };
        let region = build_region(9, b"m".to_vec(), setting, town).unwrap();
        assert_eq!(
            (region.sx, region.sy, region.ex, region.ey),
            (1000, 2000, 1300, 2400)
        );
        assert_eq!(region.spawn, (1100, 2200));
        assert_eq!(
            region.empire_spawns,
            Some([(1300, 2400), (1500, 2600), (1700, 2800)])
        );
    }

    #[test]
    fn lookup_answers_the_first_region_with_inclusive_start_and_exclusive_end() {
        let region = |index, sx, ex| MapRegion {
            index,
            name: Vec::new(),
            sx,
            sy: 0,
            ex,
            ey: 10,
            spawn: (0, 0),
            empire_spawns: None,
        };
        let atlas = MapAtlas::from_regions(vec![region(5, 0, 10), region(6, 5, 20)]);
        assert_eq!(atlas.index_at(0, 0), Some(5));
        assert_eq!(atlas.index_at(9, 9), Some(5));
        assert_eq!(atlas.index_at(10, 9), Some(6));
        assert_eq!(atlas.index_at(20, 0), None);
        assert_eq!(atlas.index_at(0, 10), None);
        assert_eq!(atlas.region(6).map(|found| found.sx), Some(5));
    }

    #[test]
    fn the_legacy_map_folder_loads() {
        let dir =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/locale/europe/map");
        let atlas = MapAtlas::load(&dir).unwrap();
        assert_eq!(atlas.regions().len(), 59);
        let a1 = atlas.region(1).unwrap();
        assert_eq!(a1.name, b"metin2_map_a1");
        assert_eq!(
            (a1.sx, a1.sy, a1.ex, a1.ey),
            (409_600, 896_000, 512_000, 1_024_000)
        );
        assert_eq!(a1.spawn, (469_300, 964_200));
        // The empire start positions of `start_position.cpp` fall on the
        // start maps 1, 21, and 41.
        assert_eq!(atlas.index_at(469_300, 964_200), Some(1));
        assert_eq!(atlas.index_at(55_700, 157_900), Some(21));
        assert_eq!(atlas.index_at(969_600, 278_400), Some(41));
        assert_eq!(atlas.index_at(0, 0), None);
    }
}
