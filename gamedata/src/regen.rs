//! The regen files: which mobs a map spawns, where, and how often.
//!
//! Legacy `SECTREE_MANAGER::Build` (`G/sectree_manager.cpp:691-775`) reads, for every map the
//! process hosts, the map folder's `regen.txt`, `npc.txt`, `boss.txt` and `stone.txt` in that
//! order with `regen_load` (`G/regen.cpp:601-720`). A file that cannot be opened is logged as not
//! found and skipped.
//!
//! A file is a stream of words (`get_word`, `G/regen.cpp:27-78`). Space, tab, CR and LF separate
//! words; a word that starts with `"` runs to the next `"`, across any separator; and a word whose
//! first two bytes are `//` ends there, after which the rest of the line is skipped (`next_line`).
//! An entry is eleven words (`read_line`, `:89-247`): its type, the centre `x` and `y` and the half
//! widths in metres, the z section, the direction, the regen time, a percent that is ignored, the
//! count, and the vnum. An exception entry (`e`) stops after the z section. An entry the file ends
//! in the middle of is dropped, as legacy drops it.
//!
//! The type is the first byte of its word: `m` is one mob, `g` a group (`ga` an aggressive one),
//! `e` an exception area, `r` a group of groups, and `s` a mob anywhere on the map. Legacy exits
//! the process on any other byte; this module refuses the file.
//!
//! Numbers are read with `str_to_number` (`common/utils.h`): `strtol` for an `int` and the low
//! byte of `strtoul` for a `BYTE`, with a 32-bit `long`. The regen time adds each run of digits
//! times 3600, 60 or 1 when an `h`, `m` or `s` follows it; other bytes are ignored, so digits with
//! no unit after them add nothing. The half widths turn the centre into a box in centimetres:
//! `sx -= w; ex = sx + 2 * w; sx *= 100; ex *= 100`.
//!
//! `regen_load` then adds the map's base position to a spawning entry's box and swaps a reversed
//! bound. An exception entry keeps its box as read.
//!
//! A word of 256 bytes or more overruns legacy's `szTmp[256]`, and a sum or product past a C `int`
//! is undefined there; this module refuses both, and it refuses a spawning entry with a negative
//! count, which legacy spawns 2^32 minus that many times. Those are Defects not reproduced. A file
//! that exists but cannot be read is refused rather than skipped.

use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::map_atlas::MapRegion;
use crate::mob_proto::{low_byte, strtol};

/// The files `SECTREE_MANAGER::Build` reads for a hosted map, in its order.
pub const REGEN_FILES: [&str; 4] = ["regen.txt", "npc.txt", "boss.txt", "stone.txt"];

/// The size of legacy `read_line`'s word buffer, `szTmp[256]`, terminator included.
const WORD_BUFFER: usize = 256;

/// Metres to centimetres, the `* 100` of `read_line`.
const METRE: i32 = 100;

/// What an entry spawns, legacy `REGEN_TYPE_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegenKind {
    /// `m`: one mob.
    Mob,
    /// `g`: a mob group; `ga` makes it aggressive.
    Group {
        /// Whether the word's second byte was `a`.
        aggressive: bool,
    },
    /// `e`: an area no regen spawns in.
    Exception,
    /// `r`: a group of groups.
    GroupGroup,
    /// `s`: one mob anywhere on the map.
    Anywhere,
}

/// One entry as `read_line` fills legacy `REGEN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegenEntry {
    /// What the entry spawns.
    pub kind: RegenKind,
    /// Left edge in centimetres.
    pub sx: i32,
    /// Top edge in centimetres.
    pub sy: i32,
    /// Right edge in centimetres.
    pub ex: i32,
    /// Bottom edge in centimetres.
    pub ey: i32,
    /// The z section.
    pub z_section: u8,
    /// 0 for a random facing, or 1 to 8 for `(direction - 1) * 45` degrees.
    pub direction: u8,
    /// Seconds between regens; 0 means the entry never spawns.
    pub time: u32,
    /// How many the entry keeps alive.
    pub max_count: i32,
    /// The mob or group vnum.
    pub vnum: i32,
}

impl RegenEntry {
    /// Whether `regen_load` places the entry and may spawn it: every kind but an exception.
    #[must_use]
    pub fn spawns(&self) -> bool {
        self.kind != RegenKind::Exception
    }

    /// The centre of the box, `(sx + ex) / 2` and `(sy + ey) / 2` as C divides.
    #[must_use]
    pub fn centre(&self) -> (i32, i32) {
        (midpoint(self.sx, self.ex), midpoint(self.sy, self.ey))
    }
}

/// `(a + b) / 2` in C `int` arithmetic, which truncates toward zero. [`load_file`] refuses a box
/// whose sum leaves an `int`, so the sum here is exact in 64 bits and the answer fits.
fn midpoint(a: i32, b: i32) -> i32 {
    i32::try_from((i64::from(a) + i64::from(b)) / 2).unwrap_or(i32::MAX)
}

/// Why a regen file was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegenFault {
    /// The type word starts with a byte that names no type; legacy exits.
    UnknownType {
        /// The entry, counted from 1.
        entry: usize,
        /// The byte.
        byte: u8,
    },
    /// A word overruns legacy's 256-byte buffer.
    LongWord {
        /// The entry, counted from 1.
        entry: usize,
        /// The word's length in bytes.
        length: usize,
    },
    /// A value leaves a C `int`, or the regen time a `DWORD`.
    Overflow {
        /// The entry, counted from 1.
        entry: usize,
        /// The field being computed.
        field: &'static str,
    },
    /// A spawning entry with a regen time keeps a negative number alive.
    NegativeCount {
        /// The entry, counted from 1.
        entry: usize,
        /// The count.
        max_count: i32,
    },
}

impl fmt::Display for RegenFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownType { entry, byte } => {
                write!(f, "entry {entry}: unknown regen type byte 0x{byte:02x}")
            }
            Self::LongWord { entry, length } => write!(
                f,
                "entry {entry}: a {length}-byte word overruns the {WORD_BUFFER}-byte buffer"
            ),
            Self::Overflow { entry, field } => {
                write!(f, "entry {entry}: {field} leaves its 32-bit type")
            }
            Self::NegativeCount { entry, max_count } => {
                write!(f, "entry {entry}: negative count {max_count}")
            }
        }
    }
}

impl Error for RegenFault {}

/// A regen file that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegenError {
    /// The file exists but could not be read.
    Io {
        /// The file.
        path: PathBuf,
        /// The operating system's message.
        message: String,
    },
    /// The file was refused.
    Invalid {
        /// The file.
        path: PathBuf,
        /// Why.
        fault: RegenFault,
    },
}

impl fmt::Display for RegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "cannot read {}: {message}", path.display()),
            Self::Invalid { path, fault } => write!(f, "{}: {fault}", path.display()),
        }
    }
}

impl Error for RegenError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { .. } => None,
            Self::Invalid { fault, .. } => Some(fault),
        }
    }
}

/// The entries of a hosted map's four regen files, in the order legacy loads them.
///
/// # Errors
///
/// Returns [`RegenError`] for a file that exists but cannot be read or is refused.
pub fn load_map(map_dir: &Path, region: &MapRegion) -> Result<Vec<RegenEntry>, RegenError> {
    let folder = map_dir.join(OsStr::from_bytes(&region.name));
    let mut entries = Vec::new();
    for file in REGEN_FILES {
        entries.extend(load_file(&folder.join(file), region.sx, region.sy)?);
    }
    Ok(entries)
}

/// One regen file's entries as `regen_load` keeps them: a spawning entry moved by the map's base
/// position with its bounds in order. A missing file has none.
///
/// # Errors
///
/// Returns [`RegenError`] for a file that exists but cannot be read or is refused.
pub fn load_file(path: &Path, base_x: i32, base_y: i32) -> Result<Vec<RegenEntry>, RegenError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(RegenError::Io {
                path: path.to_path_buf(),
                message: error.to_string(),
            })
        }
    };
    let invalid = |fault| RegenError::Invalid {
        path: path.to_path_buf(),
        fault,
    };
    let mut entries = parse(&bytes).map_err(invalid)?;
    for (number, entry) in entries.iter_mut().enumerate() {
        place(entry, number + 1, base_x, base_y).map_err(invalid)?;
    }
    Ok(entries)
}

/// Add the base position to a spawning entry's box and swap reversed bounds, as `regen_load`
/// does, refusing a sum past a C `int`.
fn place(
    entry: &mut RegenEntry,
    number: usize,
    base_x: i32,
    base_y: i32,
) -> Result<(), RegenFault> {
    if !entry.spawns() {
        return Ok(());
    }
    let overflow = |field| RegenFault::Overflow {
        entry: number,
        field,
    };
    entry.sx = entry.sx.checked_add(base_x).ok_or_else(|| overflow("sx"))?;
    entry.ex = entry.ex.checked_add(base_x).ok_or_else(|| overflow("ex"))?;
    entry.sy = entry.sy.checked_add(base_y).ok_or_else(|| overflow("sy"))?;
    entry.ey = entry.ey.checked_add(base_y).ok_or_else(|| overflow("ey"))?;
    if entry.sx > entry.ex {
        std::mem::swap(&mut entry.sx, &mut entry.ex);
    }
    if entry.sy > entry.ey {
        std::mem::swap(&mut entry.sy, &mut entry.ey);
    }
    entry
        .sx
        .checked_add(entry.ex)
        .ok_or_else(|| overflow("x centre"))?;
    entry
        .sy
        .checked_add(entry.ey)
        .ok_or_else(|| overflow("y centre"))?;
    if entry.time != 0 && entry.max_count < 0 {
        return Err(RegenFault::NegativeCount {
            entry: number,
            max_count: entry.max_count,
        });
    }
    Ok(())
}

/// Every whole entry of a regen file as `read_line` reads it, before `regen_load` places it.
///
/// # Errors
///
/// Returns the [`RegenFault`] of the first entry refused.
pub fn parse(bytes: &[u8]) -> Result<Vec<RegenEntry>, RegenFault> {
    let mut words = Words { bytes, at: 0 };
    let mut entries = Vec::new();
    while let Some(entry) = read_line(&mut words, entries.len() + 1)? {
        entries.push(entry);
    }
    Ok(entries)
}

/// The fields of an entry, legacy `ERegenModes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Type,
    Sx,
    Sy,
    Ex,
    Ey,
    ZSection,
    Direction,
    RegenTime,
    RegenPercent,
    MaxCount,
    Vnum,
}

/// `read_line`: the next entry, or `None` when the file ends first.
fn read_line(words: &mut Words<'_>, entry: usize) -> Result<Option<RegenEntry>, RegenFault> {
    let mut regen = RegenEntry {
        kind: RegenKind::Mob,
        sx: 0,
        sy: 0,
        ex: 0,
        ey: 0,
        z_section: 0,
        direction: 0,
        time: 0,
        max_count: 0,
        vnum: 0,
    };
    let overflow = |field| RegenFault::Overflow { entry, field };
    let mut mode = Mode::Type;
    while let Some(word) = words.word() {
        if word.len() >= WORD_BUFFER {
            return Err(RegenFault::LongWord {
                entry,
                length: word.len(),
            });
        }
        let text = c_string(word);
        if text.starts_with(b"//") {
            words.next_line();
            continue;
        }
        mode = match mode {
            Mode::Type => {
                regen.kind = kind(text).ok_or(RegenFault::UnknownType {
                    entry,
                    byte: text.first().copied().unwrap_or(0),
                })?;
                Mode::Sx
            }
            Mode::Sx => {
                regen.sx = strtol(text);
                Mode::Sy
            }
            Mode::Sy => {
                regen.sy = strtol(text);
                Mode::Ex
            }
            Mode::Ex => {
                (regen.sx, regen.ex) =
                    widen(regen.sx, strtol(text)).ok_or_else(|| overflow("x"))?;
                Mode::Ey
            }
            Mode::Ey => {
                (regen.sy, regen.ey) =
                    widen(regen.sy, strtol(text)).ok_or_else(|| overflow("y"))?;
                Mode::ZSection
            }
            Mode::ZSection => {
                regen.z_section = low_byte(text);
                if regen.kind == RegenKind::Exception {
                    return Ok(Some(regen));
                }
                Mode::Direction
            }
            Mode::Direction => {
                regen.direction = low_byte(text);
                Mode::RegenTime
            }
            Mode::RegenTime => {
                regen.time = regen_time(text).ok_or_else(|| overflow("time"))?;
                Mode::RegenPercent
            }
            Mode::RegenPercent => Mode::MaxCount,
            Mode::MaxCount => {
                regen.max_count = strtol(text);
                Mode::Vnum
            }
            Mode::Vnum => {
                regen.vnum = strtol(text);
                return Ok(Some(regen));
            }
        };
    }
    Ok(None)
}

/// The type a word names by its first byte, or `None` where legacy exits.
fn kind(text: &[u8]) -> Option<RegenKind> {
    match text.first()? {
        b'm' => Some(RegenKind::Mob),
        b'g' => Some(RegenKind::Group {
            aggressive: text.get(1) == Some(&b'a'),
        }),
        b'e' => Some(RegenKind::Exception),
        b'r' => Some(RegenKind::GroupGroup),
        b's' => Some(RegenKind::Anywhere),
        _ => None,
    }
}

/// The box edges from a centre and half width in metres, in centimetres:
/// `s -= w; e = s + w * 2; s *= 100; e *= 100`.
fn widen(centre: i32, half: i32) -> Option<(i32, i32)> {
    let start = centre.checked_sub(half)?;
    let end = start.checked_add(half.checked_mul(2)?)?;
    Some((start.checked_mul(METRE)?, end.checked_mul(METRE)?))
}

/// The regen time in seconds: each run of digits times the unit after it.
fn regen_time(text: &[u8]) -> Option<u32> {
    let mut time: u32 = 0;
    let mut run: i32 = 0;
    for &byte in text {
        let unit = match byte {
            b'h' => 3600,
            b'm' => 60,
            b's' => 1,
            b'0'..=b'9' => {
                run = run.checked_mul(10)?.checked_add(i32::from(byte - b'0'))?;
                continue;
            }
            _ => continue,
        };
        time = time.checked_add(u32::try_from(run.checked_mul(unit)?).ok()?)?;
        run = 0;
    }
    Some(time)
}

/// The bytes before the first NUL, as `strtol`, `strlen` and `strncmp` see a word.
fn c_string(word: &[u8]) -> &[u8] {
    word.iter()
        .position(|&byte| byte == 0)
        .map_or(word, |end| &word[..end])
}

/// Whether `get_word` treats a byte as a separator.
fn separator(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

/// A cursor over a file that reads the way `get_word` and `next_line` read with `fgetc`.
struct Words<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Words<'a> {
    /// `get_word`: the next word, or `None` when the file ends before one starts.
    ///
    /// Until a word has a byte, a `"` makes it quoted and separators are skipped. After that a
    /// quoted word ends at `"` and a plain one at a separator, and either ends once its first two
    /// bytes are `//`. Every byte of a word lies between its first byte and its end, so the word
    /// is a slice of the file.
    fn word(&mut self) -> Option<&'a [u8]> {
        let mut quoted = false;
        let mut start = None;
        while let Some(&byte) = self.bytes.get(self.at) {
            let here = self.at;
            self.at += 1;
            let Some(from) = start else {
                if byte == b'"' {
                    quoted = true;
                } else if !separator(byte) {
                    start = Some(here);
                }
                continue;
            };
            if (quoted && byte == b'"') || (!quoted && separator(byte)) {
                return Some(&self.bytes[from..here]);
            }
            if self.at - from == 2 && self.bytes[from..self.at] == *b"//" {
                return Some(&self.bytes[from..self.at]);
            }
        }
        start.map(|from| &self.bytes[from..])
    }

    /// `next_line`: skip past the next LF, or to the end of the file.
    fn next_line(&mut self) {
        match self.bytes[self.at..].iter().position(|&byte| byte == b'\n') {
            Some(offset) => self.at += offset + 1,
            None => self.at = self.bytes.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map_atlas::MapAtlas;

    fn owners_map_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/locale/europe/map")
    }

    fn mob(sx: i32, sy: i32, time: u32, vnum: i32) -> RegenEntry {
        RegenEntry {
            kind: RegenKind::Mob,
            sx,
            sy,
            ex: sx,
            ey: sy,
            z_section: 0,
            direction: 0,
            time,
            max_count: 1,
            vnum,
        }
    }

    fn only(file: &[u8]) -> RegenEntry {
        let entries = parse(file).expect("the entry reads");
        assert_eq!(entries.len(), 1, "{entries:?}");
        entries[0]
    }

    /// Map 1's `npc.txt` reads entry for entry, with its comments skipped and its base added.
    #[test]
    fn the_owners_map_a1_npcs_read_as_legacy_reads_them() {
        let path = owners_map_dir().join("metin2_map_a1/npc.txt");
        let entries = load_file(&path, 409_600, 896_000).expect("the owner's file reads");
        assert_eq!(entries.len(), 50);
        let first = RegenEntry {
            direction: 1,
            ..mob(471_800, 951_600, 1, 20_300)
        };
        assert_eq!(entries[0], first);
        let vnums: Vec<i32> = entries.iter().map(|entry| entry.vnum).collect();
        let after = |vnum| vnums[vnums.iter().position(|v| *v == vnum).unwrap() + 1];
        assert_eq!(after(9_001), 9_002, "the commented-out 9007 is skipped");
        assert_eq!(after(20_354), 20_001, "a comment after a vnum is skipped");
        let group = entries
            .iter()
            .find(|entry| entry.vnum == 12_007)
            .expect("the group entry");
        assert_eq!(group.kind, RegenKind::Group { aggressive: false });
        assert_eq!((group.sx, group.ex), (480_900, 481_100));
        assert_eq!((group.sy, group.ey), (952_500, 952_700));
        let anywhere = entries.iter().find(|entry| entry.vnum == 5_004).unwrap();
        assert_eq!(anywhere.kind, RegenKind::Anywhere);
        assert_eq!((anywhere.time, anywhere.max_count), (600, 10));
    }

    /// A hosted map reads its four files in legacy's order, a missing one adding nothing.
    #[test]
    fn a_map_reads_its_four_files_in_order() {
        let atlas = MapAtlas::load(&owners_map_dir()).expect("the owner's atlas reads");
        let region = atlas.region(1).expect("map 1");
        let entries = load_map(&owners_map_dir(), region).expect("map 1's files read");
        assert_eq!(entries.len(), 1_013 + 50 + 23 + 32);
        assert_eq!(entries[0].kind, RegenKind::GroupGroup);
        assert_eq!(entries[1_013].vnum, 20_300, "npc.txt follows regen.txt");
        let missing = MapRegion {
            name: b"no_such_map".to_vec(),
            ..region.clone()
        };
        assert_eq!(load_map(&owners_map_dir(), &missing), Ok(Vec::new()));
    }

    /// A file that exists but cannot be read is refused and named.
    #[test]
    fn an_unreadable_file_is_refused() {
        let error = load_file(&owners_map_dir(), 0, 0).unwrap_err();
        assert!(matches!(error, RegenError::Io { .. }), "{error:?}");
        assert!(error.to_string().contains("europe/map"), "{error}");
        assert!(error.source().is_none());
    }

    /// Words split on space, tab, CR and LF; quotes hold separators; `//` starts a comment
    /// wherever a word starts, and an empty quote pair is no word.
    #[test]
    fn words_split_as_get_word_splits_them() {
        let entry = only(b"\r\n m \"1 0\"\t2 0 0 0 0 1s 100 1 7\n");
        assert_eq!(
            (entry.sx, entry.sy),
            (100, 200),
            "a quoted word reads to its separator"
        );
        let entry = only(b"m 1 2 0 0 0 0 1s 100 1 7 //comment 9 9\nm");
        assert_eq!(entry.vnum, 7, "a comment runs to the end of its line");
        let entry = only(b"m 1 2 0 0 0 0 1s 100 1 7//x");
        assert_eq!(entry.vnum, 7, "`//` inside a word is part of it");
        let entry = only(b"m 1 2 0 0 0 0 1s \"\"1 0 0\" 3 7");
        assert_eq!(
            entry.max_count, 3,
            "a second opening quote keeps the word open"
        );
        let entry = only(b"m 1 2 0 0 0 0 \" 1s\" 100 1 \"7 //\"");
        assert_eq!(
            entry.time, 1,
            "separators before a quoted word's first byte are skipped"
        );
        assert_eq!(
            entry.vnum, 7,
            "`//` after a quoted word's first byte is part of it"
        );
        let entry = only(b"m 1 2 0 0 0 0 1s 100 //x\n 3 7");
        assert_eq!(entry.max_count, 3, "a comment may split an entry");
        let entry = only(b"m 1 2 0 0 0 0 1s 100 1 7 m /x");
        assert_eq!(entry.vnum, 7, "one slash starts a plain word");
    }

    /// An entry the file ends in the middle of is dropped, as `read_line` drops it.
    #[test]
    fn a_truncated_entry_is_dropped() {
        let file = b"m 1 2 0 0 0 0 1s 100 1 7\nm 1 2 0 0 0 0 1s 100 1";
        assert_eq!(parse(file).unwrap().len(), 1);
        assert_eq!(parse(b"").unwrap(), Vec::new());
        assert_eq!(parse(b"  // only a comment").unwrap(), Vec::new());
    }

    /// The type is the first byte of its word; `ga` is aggressive; any other byte is refused.
    #[test]
    fn the_type_is_the_first_byte() {
        let tail = b" 0 0 0 0 0 0 1s 100 1 7";
        let kind_of = |word: &[u8]| parse(&[word, &tail[..]].concat()).map(|e| e[0].kind);
        assert_eq!(kind_of(b"mob"), Ok(RegenKind::Mob));
        assert_eq!(kind_of(b"g"), Ok(RegenKind::Group { aggressive: false }));
        assert_eq!(kind_of(b"gx"), Ok(RegenKind::Group { aggressive: false }));
        assert_eq!(kind_of(b"ga"), Ok(RegenKind::Group { aggressive: true }));
        assert_eq!(kind_of(b"r"), Ok(RegenKind::GroupGroup));
        assert_eq!(kind_of(b"s"), Ok(RegenKind::Anywhere));
        assert_eq!(
            kind_of(b"M"),
            Err(RegenFault::UnknownType {
                entry: 1,
                byte: b'M'
            })
        );
        assert_eq!(
            kind_of(b"\0m"),
            Err(RegenFault::UnknownType { entry: 1, byte: 0 }),
            "a word is read as a C string"
        );
        let second = parse(b"m 0 0 0 0 0 0 1s 100 1 7 x").unwrap_err();
        assert_eq!(
            second,
            RegenFault::UnknownType {
                entry: 2,
                byte: b'x'
            }
        );
        assert_eq!(second.to_string(), "entry 2: unknown regen type byte 0x78");
    }

    /// An exception entry stops after its z section and keeps its box as read.
    #[test]
    fn an_exception_stops_after_the_z_section() {
        let entries = parse(b"e 10 20 2 3 4 m 1 2 0 0 0 0 1s 100 1 7").unwrap();
        let exception = entries[0];
        assert_eq!(exception.kind, RegenKind::Exception);
        assert_eq!((exception.sx, exception.ex), (800, 1_200));
        assert_eq!((exception.sy, exception.ey), (1_700, 2_300));
        assert_eq!(exception.z_section, 4);
        assert_eq!(entries[1].vnum, 7);
        let path = std::env::temp_dir().join(format!("regen-exception-{}", std::process::id()));
        std::fs::write(&path, b"e 10 20 2 3 4\n").unwrap();
        let placed = load_file(&path, 1_000, 2_000);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(placed.unwrap(), vec![exception], "the base is not added");
    }

    /// Numbers read as `strtol` and the low byte of `strtoul` read them.
    #[test]
    fn numbers_read_as_str_to_number_reads_them() {
        let entry = only(b"m 5x 7 2 1 257 -1 1s 100 -3 +42z");
        assert_eq!((entry.sx, entry.ex), (300, 700));
        assert_eq!((entry.sy, entry.ey), (600, 800));
        assert_eq!(entry.z_section, 1);
        assert_eq!(entry.direction, 255);
        assert_eq!(entry.max_count, -3);
        assert_eq!(entry.vnum, 42);
        let entry = only(b"m 10 10 -2 0 0 0 0 0 1 99999999999");
        assert_eq!(
            (entry.sx, entry.ex),
            (1_200, 800),
            "a negative width reverses the box"
        );
        assert_eq!(entry.vnum, i32::MAX, "strtol saturates");
    }

    /// The regen time sums each run of digits times the unit after it.
    #[test]
    fn the_regen_time_reads_its_units() {
        let time = |word: &[u8]| {
            parse(&[&b"m 0 0 0 0 0 0 "[..], word, b" 100 1 7"].concat()).map(|e| e[0].time)
        };
        assert_eq!(time(b"1h2m3s"), Ok(3_723));
        assert_eq!(time(b"30"), Ok(0), "digits with no unit add nothing");
        assert_eq!(time(b"1x5s"), Ok(15), "other bytes are skipped");
        assert_eq!(time(b"1\0s"), Ok(0), "the word ends at its NUL");
        assert_eq!(time(b"2m30"), Ok(120));
        assert_eq!(time(b"0s"), Ok(0));
        assert_eq!(time(b"596523h"), Ok(2_147_482_800));
        assert_eq!(time(b"596523h596523h"), Ok(4_294_965_600));
        let overflow = Err(RegenFault::Overflow {
            entry: 1,
            field: "time",
        });
        assert_eq!(time(b"596524h"), overflow, "the product leaves an int");
        assert_eq!(
            time(b"596523h596523h596523h"),
            overflow,
            "the sum leaves a DWORD"
        );
        assert_eq!(time(b"2147483648s"), overflow, "the digits leave an int");
        assert_eq!(time(b"2147483647s"), Ok(2_147_483_647));
    }

    /// A word of 256 bytes overruns `szTmp[256]` and is refused; 255 bytes fit.
    #[test]
    fn a_word_past_the_buffer_is_refused() {
        let file =
            |length: usize| [&b"m 0 0 0 0 0 0 1s "[..], &vec![b'1'; length], b" 1 7"].concat();
        assert!(parse(&file(255)).is_ok());
        let fault = parse(&file(256)).unwrap_err();
        assert_eq!(
            fault,
            RegenFault::LongWord {
                entry: 1,
                length: 256
            }
        );
        assert!(fault.to_string().contains("256-byte word"), "{fault}");
        let comment = [&b"//"[..], &vec![b'x'; 300], b"\nm 0 0 0 0 0 0 1s 1 1 7"].concat();
        assert_eq!(
            parse(&comment).unwrap().len(),
            1,
            "a comment is cut at its `//`"
        );
    }

    /// A box past a C `int` is refused, in the widths, the metres and the base.
    #[test]
    fn a_box_past_an_int_is_refused() {
        let x = |field| RegenFault::Overflow { entry: 1, field };
        assert_eq!(
            parse(b"m 99999999999 0 0 0 0 0 1s 1 1 7").unwrap_err(),
            x("x")
        );
        assert_eq!(
            parse(b"m 0 -2147483648 0 1 0 0 1s 1 1 7").unwrap_err(),
            x("y")
        );
        assert_eq!(
            parse(b"m 0 0 1073741824 0 0 0 1s 1 1 7").unwrap_err(),
            x("x")
        );
        assert!(parse(b"m 21474836 0 0 0 0 0 1s 1 1 7").is_ok());
        assert_eq!(parse(b"m 21474837 0 0 0 0 0 1s 1 1 7").unwrap_err(), x("x"));
        let mut entry = mob(i32::MAX - 5, 0, 1, 7);
        assert_eq!(place(&mut entry, 1, 6, 0), Err(x("sx")));
        let mut entry = mob(0, i32::MIN + 5, 1, 7);
        assert_eq!(place(&mut entry, 1, 0, -6), Err(x("sy")));
        let mut entry = RegenEntry {
            ex: i32::MAX,
            ..mob(0, 0, 1, 7)
        };
        assert_eq!(place(&mut entry, 1, 1, 0), Err(x("ex")));
        let mut entry = mob(i32::MAX / 2 + 1, 0, 1, 7);
        assert_eq!(
            place(&mut entry, 3, 0, 0),
            Err(RegenFault::Overflow {
                entry: 3,
                field: "x centre"
            })
        );
        let mut entry = RegenEntry {
            ey: i32::MAX,
            ..mob(0, i32::MAX / 2 + 1, 1, 7)
        };
        assert!(matches!(
            place(&mut entry, 1, 0, 0),
            Err(RegenFault::Overflow { .. })
        ));
    }

    /// The base is added to a spawning box and reversed bounds are put in order.
    #[test]
    fn the_base_is_added_and_bounds_ordered() {
        let mut entry = RegenEntry {
            ex: 100,
            ey: -50,
            ..mob(900, 300, 1, 7)
        };
        place(&mut entry, 1, 1_000, 2_000).unwrap();
        assert_eq!((entry.sx, entry.ex), (1_100, 1_900));
        assert_eq!((entry.sy, entry.ey), (1_950, 2_300));
        assert_eq!(entry.centre(), (1_500, 2_125));
        let negative = RegenEntry {
            ex: 0,
            ..mob(-3, -3, 1, 7)
        };
        assert_eq!(
            negative.centre(),
            (-1, -3),
            "C division truncates toward zero"
        );
    }

    /// A spawning entry that would regen a negative count is refused; one that never regens is
    /// not.
    #[test]
    fn a_negative_count_is_refused_where_it_spawns() {
        let mut entry = RegenEntry {
            max_count: -1,
            ..mob(0, 0, 1, 7)
        };
        let fault = place(&mut entry, 4, 0, 0).unwrap_err();
        assert_eq!(
            fault,
            RegenFault::NegativeCount {
                entry: 4,
                max_count: -1
            }
        );
        assert_eq!(fault.to_string(), "entry 4: negative count -1");
        let mut idle = RegenEntry { time: 0, ..entry };
        assert_eq!(place(&mut idle, 4, 0, 0), Ok(()));
    }

    /// A placement fault names its entry by its place in the file, counting from 1.
    #[test]
    fn a_placement_fault_counts_entries_from_one() {
        let path = std::env::temp_dir().join(format!("regen-placed-{}", std::process::id()));
        std::fs::write(
            &path,
            b"m 0 0 0 0 0 0 1s 100 1 7\nm 0 0 0 0 0 0 1s 100 -1 7\n",
        )
        .unwrap();
        let error = load_file(&path, 0, 0).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert!(
            error.to_string().ends_with("entry 2: negative count -1"),
            "{error}"
        );
    }

    /// A refused file names itself and its fault.
    #[test]
    fn a_refused_file_is_named() {
        let path = std::env::temp_dir().join(format!("regen-refused-{}", std::process::id()));
        std::fs::write(&path, b"q 0 0 0 0 0 0 1s 1 1 7").unwrap();
        let error = load_file(&path, 0, 0).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert!(
            error.to_string().starts_with(&path.display().to_string()),
            "{error}"
        );
        assert!(
            error.to_string().ends_with("unknown regen type byte 0x71"),
            "{error}"
        );
        assert!(error.source().is_some());
    }
}
