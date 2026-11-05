//! The forked map index: `forkedmapindex.txt` under the Locale folder.
//!
//! `CThreeWayWar::LoadSetting` (`G/threeway_war.cpp:137-197`) reads the file at boot;
//! a file it cannot open stops the game server (`G/main.cpp:658-665`). The `sungzi:`
//! lines name the SungZi maps whose players skip the exit's `IsHack` check
//! (`CThreeWayWar::IsSungZiMapIndex`, `G/threeway_war.cpp:220-232`, read by `do_cmd`,
//! `G/cmd_general.cpp:421-423`); the `pass:` lines name the flame-pass maps of the
//! three-way war itself, which is not ported, so they are skipped. Only the SungZi
//! map indices are kept.
//!
//! Each `sungzi:` line starts with the map index (`sscanf(szLine+7, "%d ...")`,
//! `:160-165`): leading whitespace skipped, an optional sign, then the digit run,
//! trailing garbage ignored. A line without a digit run keeps legacy's
//! uninitialised `sungziinfo.m_iForkedSung` — a Defect this build does not
//! reproduce: the line is skipped instead.
//!
//! `G/` = `server/server/game/`.

use std::collections::BTreeSet;
use std::path::Path;

/// The forked map index, under the Locale folder.
pub const FORKED_FILE: &str = "forkedmapindex.txt";

/// The `sungzi:` line prefix (`G/threeway_war.cpp:155`).
const SUNGZI_PREFIX: &[u8] = b"sungzi:";

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
}
