//! The fishing table: `fishing.txt` under the Locale folder.
//!
//! `fishing::Initialize` (`G/fishing.cpp:188-328`) reads the table at boot; a file it cannot
//! open is logged and the old table kept (empty at boot, which legacy's `sys_err` path in
//! `Compute` then guards per call). Each of the first 37 data lines fills one
//! [`Fish`]: the name and three vnums, the four map probs, the difficulty, the time type,
//! the three length bounds, and the ten use results.
//!
//! Every column goes through `trim_and_lower` (`libthecore/utils.cpp:122-160`): leading
//! whitespace skipped, the rest lowercased byte-wise (`LOWER` touches ASCII only, so the
//! Korean names in the legacy code page pass through unchanged). An empty or whitespace-only
//! column counts but writes nothing: the name keeps its zero fill, a number keeps its 0.
//! A line without a tab is logged and the backup restored — at boot the table stays empty —
//! and reading stops after 37 fish.
//!
//! `G/` = `server/server/game/`.

use std::error::Error;
use std::fmt;
use std::path::Path;

/// The fishing table, under the Locale folder.
pub const FISHING_FILE: &str = "fishing.txt";

/// `MAX_FISH` (`G/fishing.cpp:33`): the rows the loader reads.
pub const MAX_FISH: usize = 37;

/// `MAX_PROB` (`G/fishing.cpp:41`): the map probability columns.
pub const MAX_PROB: usize = 4;

/// `NUM_USE_RESULT_COUNT` (`G/fishing.cpp:34`): the use-result columns.
pub const USE_RESULTS: usize = 10;

/// One `SFishInfo` (`G/fishing.cpp:82-97`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fish {
    /// The lowercased name bytes, as the file holds them.
    pub name: Vec<u8>,
    /// The live, dead and grilled vnums.
    pub vnum: u32,
    /// The dead vnum.
    pub dead_vnum: u32,
    /// The grilled vnum.
    pub grill_vnum: u32,
    /// The four map probabilities.
    pub prob: [i32; MAX_PROB],
    /// The `number(1, difficulty) <= level` gate in `Compute`.
    pub difficulty: i32,
    /// The row of `aFishingTime` the catch rolls against.
    pub time_type: i32,
    /// The min, max and extra-max lengths.
    pub length_range: [i32; 3],
    /// The ten use results.
    pub used_table: [i32; USE_RESULTS],
}

/// The fishing table: the fish in file order, with the accumulated probability tables
/// `Initialize` builds (`g_prob_accumulate`, `g_prob_sum`, `G/fishing.cpp:316-328`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fishing {
    fish: Vec<Fish>,
    accumulate: Vec<[i32; MAX_FISH]>,
    sums: Vec<i32>,
}

/// Why the fishing table could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FishingError {
    /// The file could not be opened.
    Unreadable(std::path::PathBuf, String),
    /// A data line holds no tab, so the whole table is refused.
    TablessLine(usize),
}

impl fmt::Display for FishingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(path, error) => write!(f, "{}: {error}", path.display()),
            Self::TablessLine(line) => write!(f, "line {line} holds no tab"),
        }
    }
}

impl Error for FishingError {}

impl Fishing {
    /// Read the table from the Locale folder, as `Initialize` does.
    ///
    /// # Errors
    ///
    /// [`FishingError::Unreadable`] when the file cannot be opened, and
    /// [`FishingError::TablessLine`] for the first data line without a tab.
    pub fn load(locale_dir: &Path) -> Result<Self, FishingError> {
        let path = locale_dir.join(FISHING_FILE);
        let bytes = std::fs::read(&path)
            .map_err(|error| FishingError::Unreadable(path.clone(), error.to_string()))?;
        Self::parse(&bytes).map_err(|line| {
            if line == 0 {
                FishingError::Unreadable(path.clone(), "the file is empty".to_string())
            } else {
                FishingError::TablessLine(line)
            }
        })
    }

    /// Parse the table's bytes: the fish in file order, stopping after [`MAX_FISH`].
    /// A line starting with `#` is a comment; the returned `usize` is the 1-based number
    /// of the first tabless data line, or 0 when there is none.
    fn parse(bytes: &[u8]) -> Result<Self, usize> {
        let mut fish = Vec::new();
        for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() || line.starts_with(b"#") {
                continue;
            }
            if fish.len() >= MAX_FISH {
                break;
            }
            if !line.contains(&b'\t') {
                return Err(index + 1);
            }
            fish.push(Fish::parse(line));
        }
        Ok(Self::with_tables(fish))
    }

    /// Read the table's bytes, with a tabless line refusing the whole table.
    ///
    /// # Errors
    ///
    /// The 1-based number of the first data line without a tab.
    pub fn read(bytes: &[u8]) -> Result<Self, usize> {
        Self::parse(bytes)
    }

    fn with_tables(fish: Vec<Fish>) -> Self {
        let mut accumulate = vec![[0i32; MAX_FISH]; MAX_PROB];
        let mut sums = vec![0i32; MAX_PROB];
        for prob in 0..MAX_PROB {
            let mut running = 0i32;
            for (index, one) in fish.iter().enumerate() {
                running = running.wrapping_add(one.prob[prob]);
                accumulate[prob][index] = running;
            }
            sums[prob] = running;
        }
        Self {
            fish,
            accumulate,
            sums,
        }
    }

    /// The fish in file order.
    #[must_use]
    pub fn fish(&self) -> &[Fish] {
        &self.fish
    }

    /// The accumulated probability table for `prob_idx`, as
    /// `g_prob_accumulate[prob_idx]` holds it, or `None` past the tables.
    #[must_use]
    pub fn accumulate(&self, prob_idx: usize) -> Option<&[i32; MAX_FISH]> {
        self.accumulate.get(prob_idx)
    }

    /// The probability sum for `prob_idx`, as `g_prob_sum[prob_idx]` holds it, or `None`
    /// past the tables.
    #[must_use]
    pub fn sum(&self, prob_idx: usize) -> Option<i32> {
        self.sums.get(prob_idx).copied()
    }
}

impl Fish {
    fn parse(line: &[u8]) -> Self {
        let columns: Vec<&[u8]> = line.split(|byte| *byte == b'\t').collect();
        let mut fish = Self {
            name: Vec::new(),
            vnum: 0,
            dead_vnum: 0,
            grill_vnum: 0,
            prob: [0; MAX_PROB],
            difficulty: 0,
            time_type: 0,
            length_range: [0; 3],
            used_table: [0; USE_RESULTS],
        };
        for (index, column) in columns.iter().enumerate() {
            let lowered = lower_trimmed(column);
            if lowered.is_empty() {
                continue;
            }
            match index {
                0 => fish.name = lowered,
                1 => fish.vnum = strtoul(&lowered),
                2 => fish.dead_vnum = strtoul(&lowered),
                3 => fish.grill_vnum = strtoul(&lowered),
                4..=7 => fish.prob[index - 4] = strtol(&lowered),
                8 => fish.difficulty = strtol(&lowered),
                9 => fish.time_type = strtol(&lowered),
                10..=12 => fish.length_range[index - 10] = strtol(&lowered),
                13..=22 => fish.used_table[index - 13] = strtol(&lowered),
                _ => {}
            }
        }
        fish
    }
}

/// `trim_and_lower` (`libthecore/utils.cpp:122-160`): leading whitespace skipped, the rest
/// lowercased byte-wise.
fn lower_trimmed(column: &[u8]) -> Vec<u8> {
    let start = column
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(column.len());
    column[start..].iter().map(u8::to_ascii_lowercase).collect()
}

/// `strtol` of the first value, as the number columns read it.
fn strtol(text: &[u8]) -> i32 {
    crate::mob_proto::strtol(text)
}

/// `strtoul` of the first value, as the vnum columns read it.
fn strtoul(text: &[u8]) -> u32 {
    crate::mob_proto::strtoul(text)
}

/// `aFishingTime` (`G/fishing.cpp:73-80`): the catch chance per time type and 200ms step.
pub const FISHING_TIME: [[i32; 31]; 5] = [
    [
        0, 0, 0, 0, 0, 2, 4, 8, 12, 16, 20, 22, 25, 30, 50, 80, 50, 30, 25, 22, 20, 16, 12, 8, 4,
        2, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 8, 12, 16, 20, 22, 25, 30, 50, 80, 50,
        30, 25, 22, 20,
    ],
    [
        20, 22, 25, 30, 50, 80, 50, 30, 25, 22, 20, 16, 12, 8, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0,
    ],
    [
        100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100,
        100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100,
    ],
    [
        20, 20, 20, 20, 20, 22, 24, 28, 32, 36, 40, 42, 45, 50, 70, 100, 70, 50, 45, 42, 40, 36,
        32, 28, 24, 22, 20, 20, 20, 20, 20,
    ],
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const TWO_FISH: &[u8] = b"# name\n\xb2\xce\t0\t0\t0\t3000\t3000\t2100\t2000\t10\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\nABC\t50002\t0\t0\t50\t50\t200\t200\t200\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\n";

    #[test]
    fn the_table_reads_names_probs_and_use_results() {
        let table = Fishing::read(TWO_FISH).expect("two fish");
        assert_eq!(table.fish().len(), 2);
        assert_eq!(table.fish()[0].name, b"\xb2\xce");
        assert_eq!(table.fish()[1].name, b"abc");
        assert_eq!(table.fish()[1].vnum, 50002);
        assert_eq!(table.fish()[1].prob, [50, 50, 200, 200]);
        assert_eq!(table.fish()[1].difficulty, 200);
        assert_eq!(table.sum(0), Some(3050));
        assert_eq!(table.accumulate(0).expect("a table")[1], 3050);
    }

    #[test]
    fn a_tabless_line_refuses_the_table() {
        assert_eq!(Fishing::read(b"no tabs here\n"), Err(1));
    }

    #[test]
    fn a_missing_file_is_unreadable() {
        let missing = PathBuf::from("no-such-locale-dir");
        assert!(matches!(
            Fishing::load(&missing),
            Err(FishingError::Unreadable(_, _))
        ));
    }

    #[test]
    fn the_time_table_has_five_rows_of_thirty_one() {
        assert_eq!(FISHING_TIME.len(), 5);
        assert!(FISHING_TIME.iter().all(|row| row.len() == 31));
        assert_eq!(FISHING_TIME[3][0], 100);
        assert_eq!(FISHING_TIME[0][15], 80);
    }
}
