//! The forked map index: `forkedmapindex.txt` under the Locale folder.
//!
//! `CThreeWayWar::LoadSetting` (`G/threeway_war.cpp:137-197`) reads the file at boot;
//! a file it cannot open stops the game server (`G/main.cpp:658-665`). The `sungzi:`
//! lines name the SungZi maps whose players skip the exit's `IsHack` check
//! (`CThreeWayWar::IsSungZiMapIndex`, `G/threeway_war.cpp:220-232`, read by `do_cmd`,
//! `G/cmd_general.cpp:421-423`); the `pass:` lines name the flame-pass maps of the
//! three-way war itself, three per line (`ForkedPassMapInfo`, `G/threeway_war.h:13-19`).
//! `ForkedMaps` keeps both lists, as `SungZiInfoMap_` and `PassInfoMap_` do.
//!
//! Each line's `%d` fields scan as `sscanf` scans them (`:160-165`, `:177-184`):
//! leading whitespace skipped, an optional sign, then the digit run, trailing garbage
//! ignored. A `sungzi:` line without a digit run keeps legacy's uninitialised
//! `sungziinfo.m_iForkedSung` — a Defect this build does not reproduce: the line is
//! skipped instead. A `pass:` line without all three indices is skipped the same way,
//! against legacy's uninitialised struct.
//!
//! A line matches when its first bytes are the prefix, exactly as legacy's `strncmp`
//! tests it; the `#pass:` line stays a comment. Only the map indices are kept: the
//! SungZi starts, the boss vnum and the map names reach no ported reader yet.
//!
//! `G/` = `server/server/game/`.

use std::collections::BTreeSet;
use std::path::Path;

/// The forked map index, under the Locale folder.
pub const FORKED_FILE: &str = "forkedmapindex.txt";

/// The `sungzi:` line prefix (`G/threeway_war.cpp:155`).
const SUNGZI_PREFIX: &[u8] = b"sungzi:";

/// The `pass:` line prefix (`G/threeway_war.cpp:172`).
const PASS_PREFIX: &[u8] = b"pass:";

/// The SungZi map indices: the maps whose players skip the exit's `IsHack` check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SungziMaps {
    /// The indices in ascending order, duplicates dropped.
    indices: BTreeSet<i32>,
}

impl SungziMaps {
    /// Read the SungZi indices from the Locale folder, as `LoadSetting` does.
    ///
    /// # Errors
    ///
    /// When the file cannot be read. Legacy stops the game server then
    /// (`G/main.cpp:658-665`); the caller refuses the boot the same way.
    pub fn load(locale_dir: &Path) -> Result<Self, std::io::Error> {
        let bytes = std::fs::read(locale_dir.join(FORKED_FILE))?;
        Ok(Self::read(&bytes))
    }

    /// Parse the file's bytes: the map index of each `sungzi:` line.
    ///
    /// Lines are split on `\n` with a trailing `\r` stripped; a line is a SungZi
    /// line when its first seven bytes are `sungzi:`, exactly as legacy's
    /// `strncmp(szLine, "sungzi:", 7)` tests it — comments and `pass:` lines never
    /// match. The index is the `%d` at the rest: the digit run past whitespace and
    /// an optional sign, clamped to `i32`, trailing garbage ignored. A line with
    /// no digit run is skipped.
    ///
    /// [`ForkedMaps::read`] is the full read; this one keeps the SungZi half for the
    /// exit gate.
    #[must_use]
    pub fn read(bytes: &[u8]) -> Self {
        let mut indices = BTreeSet::new();
        for line in bytes.split(|byte| *byte == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let Some(rest) = line.strip_prefix(SUNGZI_PREFIX) else {
                continue;
            };
            if let Some(index) = scan_index(rest) {
                indices.insert(index);
            }
        }
        Self { indices }
    }

    /// Whether `map` is a SungZi map (`IsSungZiMapIndex`).
    #[must_use]
    pub fn contains(&self, map: i32) -> bool {
        self.indices.contains(&map)
    }

    /// The indices in ascending order.
    #[must_use]
    pub fn indices(&self) -> Vec<i32> {
        self.indices.iter().copied().collect()
    }
}

/// The `%d` at `rest`: whitespace skipped, an optional sign, then the digit run,
/// clamped to `i32`. `None` when there is no digit run.
fn scan_index(rest: &[u8]) -> Option<i32> {
    let start = rest.iter().position(|byte| !byte.is_ascii_whitespace())?;
    let rest = &rest[start..];
    let (negative, digits) = match rest.first() {
        Some(b'-') => (true, &rest[1..]),
        Some(b'+') => (false, &rest[1..]),
        _ => (false, rest),
    };
    let len = digits
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if len == 0 {
        return None;
    }
    let mut value: i64 = 0;
    for digit in &digits[..len] {
        value = value * 10 + i64::from(digit - b'0');
    }
    if negative {
        value = -value;
    }
    Some(i32::try_from(value.clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(i32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's file names the three SungZi maps and skips the rest.
    #[test]
    fn the_owner_file_names_the_three_sungzi_maps() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../legacy/gamedata/locale/europe/forkedmapindex.txt"),
        )
        .expect("the owner's forked index reads");
        let maps = SungziMaps::read(&bytes);
        assert_eq!(maps.indices(), vec![114, 121, 125]);
        assert!(maps.contains(114));
        assert!(!maps.contains(118), "a pass map is no SungZi map");
        assert!(!maps.contains(1));
    }

    /// The `%d` rules: whitespace, the sign, the digit run, trailing garbage, and
    /// the clamp; a digitless line is skipped, not zero.
    #[test]
    fn the_index_scans_like_percent_d() {
        assert_eq!(scan_index(b"  114 948100"), Some(114));
        assert_eq!(scan_index(b"+114x"), Some(114));
        assert_eq!(scan_index(b"-3"), Some(-3));
        assert_eq!(scan_index(b"9999999999"), Some(i32::MAX));
        assert_eq!(scan_index(b"-9999999999"), Some(i32::MIN));
        assert_eq!(scan_index(b"metin2"), None);
        assert_eq!(scan_index(b""), None);
        assert_eq!(scan_index(b"   "), None);
        let maps = SungziMaps::read(b"sungzi: metin2\nsungzi: 114 dup\nsungzi: 114\n");
        assert_eq!(maps.indices(), vec![114]);
    }

    /// Only `sungzi:` lines match: comments, `pass:` lines and CRLF ends never do.
    #[test]
    fn only_sungzi_lines_match() {
        let maps = SungziMaps::read(
            b"#sungzi: 1\npass: 118 1150400\nsungzi: 121\r\n sungzi: 125\nsungzi:114\n",
        );
        assert_eq!(maps.indices(), vec![114, 121]);
    }

    /// The owner's file: three SungZi maps and three pass triples, in file order,
    /// with the `#pass:` comment skipped.
    #[test]
    fn the_owner_file_reads_both_lists() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../legacy/gamedata/locale/europe/forkedmapindex.txt"),
        )
        .expect("the owner's forked index reads");
        let maps = ForkedMaps::read(&bytes);
        assert_eq!(maps.sungzi(), &[114, 121, 125]);
        assert_eq!(
            maps.passes(),
            &[[118, 119, 120], [122, 123, 124], [126, 127, 128]]
        );
        assert!(maps.is_sungzi(121));
        assert!(!maps.is_sungzi(119));
        assert!(maps.is_forked(114));
        assert!(maps.is_forked(127));
        assert!(!maps.is_forked(1));
    }

    /// A `pass:` line missing a number or a name is skipped, not zero-filled.
    #[test]
    fn a_short_pass_line_is_skipped() {
        let maps =
            ForkedMaps::read(b"pass: 118 1 2 hill_01 119 3\npass: 122 1 2 a 123 4 5 b 124 6 7 c\n");
        assert_eq!(maps.passes(), &[[122, 123, 124]]);
    }
}

/// The forked maps: the SungZi list and the pass list, as `LoadSetting` keeps them
/// (`SungZiInfoMap_`, `PassInfoMap_`, `G/threeway_war.h:64-65`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForkedMaps {
    /// The SungZi map indices, in file order.
    sungzi: Vec<i32>,
    /// The pass triples, in file order: one per `pass:` line.
    passes: Vec<[i32; 3]>,
}

impl ForkedMaps {
    /// Parse the file's bytes: the SungZi indices and the pass triples.
    ///
    /// A `sungzi:` line notes its `%d`; a `pass:` line notes its three pass maps'
    /// `%d` fields, which are the first, fourth and seventh numbers on the line
    /// (each index heads two start coordinates and a map name). A line missing its
    /// numbers is skipped.
    #[must_use]
    pub fn read(bytes: &[u8]) -> Self {
        let mut sungzi = Vec::new();
        let mut passes = Vec::new();
        for line in bytes.split(|byte| *byte == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if let Some(rest) = line.strip_prefix(SUNGZI_PREFIX) {
                if let Some(index) = scan_index(rest) {
                    sungzi.push(index);
                }
            } else if let Some(rest) = line.strip_prefix(PASS_PREFIX) {
                if let Some(triple) = scan_pass(rest) {
                    passes.push(triple);
                }
            }
        }
        Self { sungzi, passes }
    }

    /// Read both lists from the Locale folder, as `LoadSetting` does.
    ///
    /// # Errors
    ///
    /// When the file cannot be read. Legacy stops the game server then
    /// (`G/main.cpp:658-665`); the caller refuses the boot the same way.
    pub fn load(locale_dir: &Path) -> Result<Self, std::io::Error> {
        let bytes = std::fs::read(locale_dir.join(FORKED_FILE))?;
        Ok(Self::read(&bytes))
    }

    /// The SungZi indices, in file order.
    #[must_use]
    pub fn sungzi(&self) -> &[i32] {
        &self.sungzi
    }

    /// The pass triples, in file order.
    #[must_use]
    pub fn passes(&self) -> &[[i32; 3]] {
        &self.passes
    }

    /// Whether `map` is a SungZi map (`IsSungZiMapIndex`).
    #[must_use]
    pub fn is_sungzi(&self, map: i32) -> bool {
        self.sungzi.contains(&map)
    }

    /// Whether `map` is any forked map (`IsThreeWayWarMapIndex`).
    #[must_use]
    pub fn is_forked(&self, map: i32) -> bool {
        self.sungzi.contains(&map) || self.passes.iter().any(|pass| pass.contains(&map))
    }
}

/// The three pass indices on a `pass:` line's rest: each `%d %d %d %s` group heads
/// its index (`G/threeway_war.cpp:177-184`). The coordinates and the map name are
/// skipped; only the index is kept.
fn scan_pass(rest: &[u8]) -> Option<[i32; 3]> {
    let mut indices = [0; 3];
    let mut rest = rest;
    for slot in &mut indices {
        *slot = scan_index(rest)?;
        rest = past_number(rest);
        rest = past_number(rest);
        rest = past_number(rest);
        rest = past_word(rest);
    }
    Some(indices)
}

/// The bytes after the first `%d` on `rest`: past its whitespace, sign and digits.
fn past_number(rest: &[u8]) -> &[u8] {
    let mut rest = rest;
    while rest.first().is_some_and(u8::is_ascii_whitespace) {
        rest = &rest[1..];
    }
    if rest
        .first()
        .is_some_and(|byte| *byte == b'-' || *byte == b'+')
    {
        rest = &rest[1..];
    }
    while rest.first().is_some_and(u8::is_ascii_digit) {
        rest = &rest[1..];
    }
    rest
}

/// The bytes after the first `%s` on `rest`: past its whitespace and its word, as
/// `sscanf`'s `%s` scans the map name.
fn past_word(rest: &[u8]) -> &[u8] {
    let mut rest = rest;
    while rest.first().is_some_and(u8::is_ascii_whitespace) {
        rest = &rest[1..];
    }
    while rest.first().is_some_and(|byte| !byte.is_ascii_whitespace()) {
        rest = &rest[1..];
    }
    rest
}
