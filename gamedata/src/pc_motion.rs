//! The player motions: the `.msa` clips that give each race and weapon its walk and run speed.
//!
//! Legacy `CMotionManager::Build` (`server/server/game/motion.cpp:264-357`) loads sixteen clips
//! for each of the eight main races: a run and a walk for each of the eight
//! [`crate::pc_motion::MotionMode`]s. Races 0 to 3 read `data/pc/<job>` and races 4 to 7 read
//! `data/pc2/<job>`, where `<job>` is `race % 4` in `warrior`, `assassin`, `sura`, `shaman`
//! order. A clip is `<mode folder>/run.msa` or `walk.msa`.
//! A clip that does not load is not inserted (`CMotionSet::Load`, `:386-399`), and a speed that
//! finds no clip falls back to 300 (`GetMoveMotionSpeed`, `G/char.cpp:3576`).
//!
//! A clip is a flat file of keys, read by `CMotion::LoadFromFile` (`motion.cpp:524-547`) through
//! the legacy text reader ([`crate::text_file`]). Two keys matter. `MotionDuration` is the clip's
//! length in seconds, and a clip without it does not load. `Accumulation` is how far the animation
//! carries the body, and its `y` is the distance over the clip, so a player's speed is
//! `-accY / duration` units a second (`GetMoveMotionSpeed`, `G/char.cpp:3574`). Legacy reads `x`
//! and `z` too, but no speed uses them.
//!
//! Legacy reads each number with `atof` as a double and stores it as a `float`. Here the decimal
//! token is parsed straight to an `f32`. The two agree on the 22 distinct numbers in the owner's
//! player clips that were checked. A token that double rounding moves is not checked beyond them.
//! The reader takes the decimal form only (`atof`'s hex and `inf` are not read). A horse clip has
//! `accY` 0, so its speed is 0. No mount is ported, so no speed reads the horse mode, though
//! `Build` loads it and so does this.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use crate::text_file::{self, TextFileError};

/// `MAIN_RACE_MAX_NUM`, the number of races `Build` loads clips for.
pub const MAIN_RACES: u8 = 8;

/// The job folder names, indexed by `race % 4` (`motion.cpp:266-273`).
const JOBS: [&str; 4] = ["warrior", "assassin", "sura", "shaman"];

/// `EMotionMode` (`server/server/game/motion.h:8-17`): the stance a clip is for, which the weapon
/// in the weapon slot chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MotionMode {
    /// `MOTION_MODE_GENERAL`: no weapon, or one whose sub-type has no stance of its own.
    General,
    /// `MOTION_MODE_ONEHAND_SWORD`.
    OneHandSword,
    /// `MOTION_MODE_TWOHAND_SWORD`.
    TwoHandSword,
    /// `MOTION_MODE_DUALHAND_SWORD`, the stance of a dagger.
    DualHandSword,
    /// `MOTION_MODE_BOW`.
    Bow,
    /// `MOTION_MODE_BELL`.
    Bell,
    /// `MOTION_MODE_FAN`.
    Fan,
    /// `MOTION_MODE_HORSE`, the stance of a mounted body.
    Horse,
}

impl MotionMode {
    /// The eight modes in legacy's order, which `Build` loads them in.
    pub const ALL: [Self; 8] = [
        Self::General,
        Self::OneHandSword,
        Self::TwoHandSword,
        Self::DualHandSword,
        Self::Bow,
        Self::Bell,
        Self::Fan,
        Self::Horse,
    ];

    /// The folder under a job folder that holds the mode's clips (`motion.cpp:264-323`).
    pub const fn folder(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::OneHandSword => "onehand_sword",
            Self::TwoHandSword => "twohand_sword",
            Self::DualHandSword => "dualhand_sword",
            Self::Bow => "bow",
            Self::Bell => "bell",
            Self::Fan => "fan",
            Self::Horse => "horse",
        }
    }
}

/// The two numbers of a clip that a speed reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionClip {
    /// `MotionDuration`, in seconds.
    pub duration: f32,
    /// The `y` of `Accumulation`, in units. A clip without a three-value `Accumulation` has 0.
    pub accumulation_y: f32,
}

impl MotionClip {
    /// `-pkMotion->GetAccumVector().y / pkMotion->GetDuration()` (`G/char.cpp:3574`), in units a
    /// second.
    pub fn speed(self) -> f32 {
        -self.accumulation_y / self.duration
    }
}

/// `CMotion::LoadFromFile` (`motion.cpp:524-547`) on one clip's bytes. `Ok(None)` is a clip legacy
/// would not load, which is one with no `MotionDuration`.
///
/// # Errors
///
/// [`TextFileError`] for a file the legacy text reader would exit on.
pub fn parse_clip(data: &[u8]) -> Result<Option<MotionClip>, TextFileError> {
    let file = text_file::parse(data)?;
    // `GetTokenFloat` reads the key's first value, and fails when the key is missing.
    let Some(duration) = file
        .get(b"motionduration")
        .and_then(|values| values.first())
        .map(|token| atof(token))
    else {
        return Ok(None);
    };
    // `GetTokenPosition` is `GetTokenVector3`, which reads only a key of exactly three values.
    // Otherwise `accY` keeps the zero its constructor gave it (`motion.cpp:401-405`).
    let accumulation_y = match file.get(b"accumulation") {
        Some([_, y, _]) => atof(y),
        _ => 0.0,
    };
    Ok(Some(MotionClip {
        duration,
        accumulation_y,
    }))
}

/// `atof` (`stdlib`) on one token, which the legacy float getters use: the decimal number at the
/// start of the token, and 0 when no digit starts it.
///
/// Only the decimal form is read. `strtod` also reads hex and `inf`, which no clip has, so a token
/// that starts with one reads its leading `0` here, or 0 where no digit starts it.
fn atof(token: &[u8]) -> f32 {
    let sign = usize::from(matches!(token.first(), Some(b'+' | b'-')));
    let int_end = digits_from(token, sign);
    let mut digits = int_end - sign;
    let mut end = int_end;
    if token.get(end) == Some(&b'.') {
        let frac_end = digits_from(token, end + 1);
        digits += frac_end - (end + 1);
        end = frac_end;
    }
    if digits == 0 {
        return 0.0;
    }
    if let Some(exponent_end) = exponent_end(token, end) {
        end = exponent_end;
    }
    std::str::from_utf8(&token[..end])
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(0.0)
}

/// The index just past the ASCII digits that start at `from`.
fn digits_from(token: &[u8], from: usize) -> usize {
    let digits = token
        .get(from..)
        .unwrap_or_default()
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    from + digits
}

/// The index just past an exponent (`e`, an optional sign and digits) that starts at `at`, or
/// `None` when no digit follows the `e`.
fn exponent_end(token: &[u8], at: usize) -> Option<usize> {
    if !matches!(token.get(at), Some(b'e' | b'E')) {
        return None;
    }
    let digits_at = at + 1 + usize::from(matches!(token.get(at + 1), Some(b'+' | b'-')));
    let end = digits_from(token, digits_at);
    (end > digits_at).then_some(end)
}

/// The folder of a race's clips under the `data` folder: `pc` for races 0 to 3 and `pc2` for 4 to
/// 7, then the job, which is `race % 4`.
fn race_folder(race: u8) -> PathBuf {
    let set = if race < MAIN_RACES / 2 { "pc" } else { "pc2" };
    Path::new(set).join(JOBS[usize::from(race % 4)])
}

/// The file a pace's clip is in: `walk.msa` or `run.msa`.
const fn clip_file(walk: bool) -> &'static str {
    if walk {
        "walk.msa"
    } else {
        "run.msa"
    }
}

/// Every clip that loaded, by race, mode and whether it is the walk.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PcMotions {
    clips: BTreeMap<(u8, MotionMode, bool), MotionClip>,
    /// The clips that were absent or had no `MotionDuration`, in the order `Build` tries them. A
    /// speed that finds one of these falls back to 300.
    pub skipped: Vec<PathBuf>,
}

impl PcMotions {
    /// The clip of `race` in `mode`, the walk when `walk` is set, or `None` where legacy has no
    /// such clip.
    pub fn get(&self, race: u8, mode: MotionMode, walk: bool) -> Option<MotionClip> {
        self.clips.get(&(race, mode, walk)).copied()
    }

    /// How many clips loaded.
    pub fn loaded(&self) -> usize {
        self.clips.len()
    }

    /// Stores `clip` as the clip of `race` in `mode`, the walk when `walk` is set.
    pub fn insert(&mut self, race: u8, mode: MotionMode, walk: bool, clip: MotionClip) {
        self.clips.insert((race, mode, walk), clip);
    }

    /// Reads every clip under `data_dir`, the legacy `data` folder (`CMotionManager::Build`).
    ///
    /// A clip that is absent, or has no `MotionDuration`, is listed in [`Self::skipped`], and the
    /// others still load, as legacy loads them. Legacy inserts a clip only when it loads
    /// (`motion.cpp:386-399`), so an unreadable one is simply missing there. Here a clip that
    /// exists but cannot be read is an error, so a damaged install stops the server at startup.
    ///
    /// # Errors
    ///
    /// [`PcMotionError`] for a clip that exists but cannot be read, and for one that the legacy
    /// text reader refuses, which legacy exits on.
    pub fn load(data_dir: &Path) -> Result<Self, PcMotionError> {
        let mut motions = Self::default();
        for race in 0..MAIN_RACES {
            let job_dir = data_dir.join(race_folder(race));
            for mode in MotionMode::ALL {
                for walk in [false, true] {
                    let path = job_dir.join(mode.folder()).join(clip_file(walk));
                    match fs::read(&path) {
                        Ok(data) => match parse_clip(&data) {
                            Ok(Some(clip)) => motions.insert(race, mode, walk, clip),
                            Ok(None) => motions.skipped.push(path),
                            Err(source) => return Err(PcMotionError::Text { path, source }),
                        },
                        Err(source) if source.kind() == ErrorKind::NotFound => {
                            motions.skipped.push(path);
                        }
                        Err(source) => return Err(PcMotionError::Unreadable { path, source }),
                    }
                }
            }
        }
        Ok(motions)
    }
}

/// A clip [`PcMotions::load`] refuses.
#[derive(Debug)]
pub enum PcMotionError {
    /// The clip exists but cannot be read.
    Unreadable {
        /// The clip's path.
        path: PathBuf,
        /// Why it could not be read.
        source: io::Error,
    },
    /// The clip is not a legacy text file.
    Text {
        /// The clip's path.
        path: PathBuf,
        /// Why the text reader refused it.
        source: TextFileError,
    },
}

impl fmt::Display for PcMotionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Text { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl Error for PcMotionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Unreadable { source, .. } => Some(source),
            Self::Text { source, .. } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_data() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/data")
    }

    #[test]
    fn atof_reads_the_decimal_number_at_the_start_of_a_token() {
        for (token, value) in [
            ("0.666667", 0.666_667_f32),
            ("-300.00", -300.0),
            ("+5", 5.0),
            ("-.5", -0.5),
            ("5.", 5.0),
            ("1e2", 100.0),
            ("1E+2", 100.0),
            ("1.5e-1", 0.15),
            ("1e+2x", 100.0),
            ("12abc", 12.0),
            ("1e", 1.0),
        ] {
            assert_eq!(atof(token.as_bytes()).to_bits(), value.to_bits(), "{token}");
        }
    }

    #[test]
    fn atof_reads_zero_where_no_digit_starts_the_token() {
        for token in ["", "-", ".", "abc", "x1", "-inf"] {
            assert_eq!(
                atof(token.as_bytes()).to_bits(),
                0.0_f32.to_bits(),
                "{token:?}"
            );
        }
    }

    #[test]
    fn a_clip_reads_its_duration_and_the_y_of_its_accumulation() {
        let clip = parse_clip(
            b"ScriptType\tMotionData\n\nMotionFileName \"d:/x.gr2\"\nMotionDuration 0.8\n\
              Accumulation 0.00\t-176.86\t0.00\n",
        )
        .unwrap()
        .expect("a duration makes a clip");
        assert_eq!(clip.duration.to_bits(), 0.8_f32.to_bits());
        assert_eq!(clip.accumulation_y.to_bits(), (-176.86_f32).to_bits());
    }

    #[test]
    fn a_clip_with_no_duration_is_not_loaded() {
        let clip = parse_clip(b"Accumulation 0.00 -300.00 0.00\n").unwrap();
        assert_eq!(clip, None);
    }

    #[test]
    fn the_first_duration_of_a_clip_wins() {
        let clip = parse_clip(b"MotionDuration 1.0\nMotionDuration 2.0\n")
            .unwrap()
            .unwrap();
        assert_eq!(clip.duration.to_bits(), 1.0_f32.to_bits());
    }

    #[test]
    fn a_duration_with_two_values_takes_the_first() {
        let clip = parse_clip(b"MotionDuration 1.0 2.0\n").unwrap().unwrap();
        assert_eq!(clip.duration.to_bits(), 1.0_f32.to_bits());
    }

    #[test]
    fn an_accumulation_with_two_values_leaves_y_at_zero() {
        let clip = parse_clip(b"MotionDuration 1.0\nAccumulation 0 -300\n")
            .unwrap()
            .unwrap();
        assert_eq!(clip.accumulation_y.to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn a_key_is_found_whatever_its_case_in_the_file() {
        let clip = parse_clip(b"MOTIONDURATION 0.5\nACCUMULATION 0 -10 0\n")
            .unwrap()
            .unwrap();
        assert_eq!((clip.duration, clip.accumulation_y), (0.5, -10.0));
    }

    #[test]
    fn the_speed_is_minus_y_over_the_duration() {
        let clip = MotionClip {
            duration: 0.8,
            accumulation_y: -176.86,
        };
        assert!((clip.speed() - 221.075).abs() < 1e-3, "{}", clip.speed());
    }

    #[test]
    fn a_race_folder_is_the_pc_set_of_its_job() {
        assert_eq!(race_folder(0), Path::new("pc/warrior"));
        assert_eq!(race_folder(3), Path::new("pc/shaman"));
        assert_eq!(race_folder(4), Path::new("pc2/warrior"));
        assert_eq!(race_folder(7), Path::new("pc2/shaman"));
    }

    #[test]
    fn each_mode_folder_is_its_legacy_name() {
        let folders = MotionMode::ALL.map(MotionMode::folder);
        assert_eq!(
            folders,
            [
                "general",
                "onehand_sword",
                "twohand_sword",
                "dualhand_sword",
                "bow",
                "bell",
                "fan",
                "horse",
            ]
        );
    }

    #[test]
    fn the_legacy_data_loads_all_128_clips_and_skips_none() {
        let motions = PcMotions::load(&legacy_data()).expect("the owner's clips read");
        assert_eq!(motions.clips.len(), 128);
        assert!(motions.skipped.is_empty(), "{:?}", motions.skipped);
        let shaman_walk = motions.get(3, MotionMode::General, true).unwrap();
        assert!((shaman_walk.speed() - 155.74).abs() < 1e-3);
        let warrior_run = motions.get(4, MotionMode::General, false).unwrap();
        assert!((warrior_run.speed() - 484.95).abs() < 1e-2);
    }

    #[test]
    fn a_horse_clip_loads_with_a_speed_of_zero() {
        let motions = PcMotions::load(&legacy_data()).unwrap();
        let horse = motions.get(0, MotionMode::Horse, false).unwrap();
        assert_eq!(horse.speed().abs().to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn a_data_folder_that_is_not_there_skips_every_clip() {
        let motions = PcMotions::load(Path::new("/nonexistent/legacy/data")).unwrap();
        assert!(motions.clips.is_empty());
        assert_eq!(motions.skipped.len(), 128);
    }

    #[test]
    fn a_clip_with_no_duration_is_skipped_and_the_rest_load() {
        let dir =
            std::env::temp_dir().join(format!("pc-motion-no-duration-{}", std::process::id()));
        let general = dir.join("pc").join("warrior").join("general");
        fs::create_dir_all(&general).unwrap();
        fs::write(general.join("run.msa"), b"Accumulation 0.00 -300.00 0.00\n").unwrap();
        fs::write(
            general.join("walk.msa"),
            b"MotionDuration 1.0\nAccumulation 0.00 -200.00 0.00\n",
        )
        .unwrap();
        let motions = PcMotions::load(&dir);
        fs::remove_dir_all(&dir).unwrap();
        let motions = motions.unwrap();
        assert!(motions.skipped.contains(&general.join("run.msa")));
        assert_eq!(motions.loaded(), 1);
        let walk = motions.get(0, MotionMode::General, true).unwrap();
        assert_eq!(walk.speed().to_bits(), 200.0_f32.to_bits());
    }

    #[test]
    fn a_clip_the_text_reader_refuses_stops_the_load() {
        let dir =
            std::env::temp_dir().join(format!("pc-motion-text-refused-{}", std::process::id()));
        let general = dir.join("pc").join("warrior").join("general");
        fs::create_dir_all(&general).unwrap();
        fs::write(general.join("run.msa"), b"group a b\n").unwrap();
        let motions = PcMotions::load(&dir);
        fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(
            motions,
            Err(PcMotionError::Text { ref path, .. }) if *path == general.join("run.msa")
        ));
    }
}
