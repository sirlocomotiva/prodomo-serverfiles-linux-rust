//! The battle-pass table (`locale/europe/battlepass.json`), read at boot
//! (`CBattlePass::BootBattlePassMap`, `G/BattlePassManager.cpp:80-245`).
//!
//! The file holds a `Settings` object (the mission caps, the pass `endTime`,
//! and the free and premium reward tiers) and a `missions` array whose index
//! is the mission id (`missionMap[i] = mission`, `:237`). The parse below
//! reads exactly that shape and nothing else.
//!
//! The legacy read uses rapidjson (`G/BattlePassManager.cpp:18-21`), so the
//! number rules below are its `IsUint`/`GetUint` rules: a value counts as a
//! number only when it is ASCII digits with no sign, fraction, or exponent
//! and fits in a `u32`. Anything else where a number belongs skips the
//! mission (or refuses the file for the three `Settings` scalars). Strings
//! decode rapidjson's escapes, including `\u` pairs, and a raw control byte
//! or a bad escape refuses the file, as rapidjson's validating reader does.
//!
//! # Divergences
//!
//! - **Boot failure.** A missing, malformed, or incomplete file stops the
//!   server, as for every other Game data file. Legacy only logs (`sys_err`)
//!   and boots with an empty (or half-cleared) map.
//! - **Day math in UTC.** The mission end times count UTC midnights and weeks,
//!   while legacy's `localtime`/`mktime` counts the host's zone.
//! - **Strict envelope.** Trailing bytes past the root value and duplicate
//!   object keys refuse the file; legacy ignores trailing bytes and debug
//!   asserts on duplicates.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::Path;

use super::json::{parse, Json, JsonError};

/// `MISSION_TYPE_KILL` (`C/tables.h:493`).
pub const MISSION_TYPE_KILL: u8 = 0;
/// `MISSION_TYPE_SELL` (`C/tables.h:494`).
pub const MISSION_TYPE_SELL: u8 = 1;
/// `MISSION_TYPE_ITEM_DESTROY` (`C/tables.h:496`).
pub const MISSION_TYPE_ITEM_DESTROY: u8 = 2;
/// `MISSION_TYPE_ITEM_DROP` (`C/tables.h:497`).
pub const MISSION_TYPE_ITEM_DROP: u8 = 3;
/// `MISSION_TYPE_CRAFT` (`C/tables.h:499`).
pub const MISSION_TYPE_CRAFT: u8 = 4;
/// `MISSION_TYPE_CHEST_OPEN` (`C/tables.h:500`).
pub const MISSION_TYPE_CHEST_OPEN: u8 = 5;
/// `MISSION_TYPE_USE_ITEM` (`C/tables.h:502`).
pub const MISSION_TYPE_USE_ITEM: u8 = 6;
/// `MISSION_TYPE_FINISH_DUNGEON` (`C/tables.h:503`).
pub const MISSION_TYPE_FINISH_DUNGEON: u8 = 7;
/// `MISSION_TYPE_POLYMORPH` (`C/tables.h:506`).
pub const MISSION_TYPE_POLYMORPH: u8 = 8;
/// `MISSION_TYPE_MESSAGES` (`C/tables.h:507`).
pub const MISSION_TYPE_MESSAGES: u8 = 9;
/// `MISSION_TYPE_PLAYTIME` (`C/tables.h:509`).
pub const MISSION_TYPE_PLAYTIME: u8 = 10;
/// `MISSION_TYPE_SPECIFIC` (`C/tables.h:514`).
pub const MISSION_TYPE_SPECIFIC: u8 = 0;
/// `MISSION_TYPE_GLOBAL` (`C/tables.h:515`).
pub const MISSION_TYPE_GLOBAL: u8 = 1;
/// `MISSION_TIME_DAILY` (`C/tables.h:520`).
pub const MISSION_TIME_DAILY: u8 = 0;
/// `MISSION_TIME_WEEKLY` (`C/tables.h:521`).
pub const MISSION_TIME_WEEKLY: u8 = 1;

/// The battle-pass file boot reads, by its locale path.
pub const BATTLE_PASS_FILE: &str = "battlepass.json";

/// One `TBattlePassParser` row (`C/tables.h:524-540`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BattlePassMission {
    /// `id`: the `missions` array index.
    pub id: u16,
    /// `missionType`.
    pub mission_type: u8,
    /// `missionSpecific`.
    pub specific: u8,
    /// `vnums`: read only for `SPECIFIC` missions.
    pub vnums: Vec<u32>,
    /// `maxProgress`.
    pub max_progress: u32,
    /// `timeInfo`: daily or weekly.
    pub time_info: u8,
    /// `pointsReward` (from `points`).
    pub points: u16,
    /// `name` (from `desc`).
    pub name: Vec<u8>,
}

/// `TBattlePassSettings` (`C/tables.h:555-563`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BattlePassSettings {
    /// `maxDailyMissions`, parsed but never read by the mission draw.
    pub max_daily: u8,
    /// `maxWeeklyMissions`, parsed but never read by the mission draw.
    pub max_weekly: u8,
    /// `endTime`: past it no progress is recorded.
    pub end_time: u32,
    /// `rewardsFree`: `(vnum, count)` tiers by level.
    pub rewards_free: Vec<(i32, i32)>,
    /// `rewardsPremium`: `(vnum, count)` tiers by level.
    pub rewards_premium: Vec<(i32, i32)>,
}

/// The booted table: the settings and the mission map, keyed by mission id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BattlePassTable {
    /// `battlePassSettings`.
    pub settings: BattlePassSettings,
    /// `missionMap`.
    pub missions: BTreeMap<u16, BattlePassMission>,
}

impl BattlePassTable {
    /// Look a mission up by id. An unknown id answers the zeroed row, as
    /// legacy's `missionMap[mission]` default-constructs one
    /// (`G/BattlePassManager.cpp:287`, `:315`, `G/char.cpp:12861`).
    #[must_use]
    pub fn mission(&self, id: u16) -> BattlePassMission {
        self.missions.get(&id).cloned().unwrap_or_default()
    }

    /// Load the table from the locale directory
    /// (`LocaleService_GetBasePath() + "/battlepass.json"`, `:88`).
    ///
    /// # Errors
    ///
    /// [`BattlePassError`]: the file cannot be read or refuses the parse.
    pub fn load(locale_dir: &Path) -> Result<Self, BattlePassError> {
        let path = locale_dir.join(BATTLE_PASS_FILE);
        let bytes = std::fs::read(&path).map_err(|error| BattlePassError::Unreadable {
            path: path.display().to_string(),
            error: error.to_string(),
        })?;
        read(&bytes).map_err(|error| error.with_path(path.display().to_string()))
    }
}

/// Why the battle-pass file would not load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BattlePassError {
    /// The file cannot be read.
    Unreadable {
        /// The path the boot tried.
        path: String,
        /// The I/O failure.
        error: String,
    },
    /// The bytes are not the JSON envelope the file needs.
    Malformed {
        /// The path the boot tried, when one was read.
        path: String,
        /// What the parse found.
        error: JsonError,
    },
    /// The `Settings` object is missing or unusable.
    MissingSettings {
        /// The path the boot tried, when one was read.
        path: String,
    },
    /// The `missions` array is missing.
    MissingMissions {
        /// The path the boot tried, when one was read.
        path: String,
    },
}

impl BattlePassError {
    /// Name the file a raw [`read`] failure came from.
    #[must_use]
    pub fn with_path(self, path: String) -> Self {
        match self {
            Self::Malformed { error, .. } => Self::Malformed { path, error },
            Self::MissingSettings { .. } => Self::MissingSettings { path },
            Self::MissingMissions { .. } => Self::MissingMissions { path },
            unreadable @ Self::Unreadable { .. } => unreadable,
        }
    }
}

impl fmt::Display for BattlePassError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, error } => {
                write!(f, "battle-pass file is unreadable in {path}: {error}")
            }
            Self::Malformed { path, error } => {
                write!(f, "battle-pass file is unusable in {path}: {error}")
            }
            Self::MissingSettings { path } => {
                write!(f, "battle-pass file has no Settings object in {path}")
            }
            Self::MissingMissions { path } => {
                write!(f, "battle-pass file has no missions array in {path}")
            }
        }
    }
}

impl Error for BattlePassError {}

/// `BootBattlePassMap` (`G/BattlePassManager.cpp:80-245`) on the file's bytes:
/// the settings, then the missions keyed by array index.
///
/// # Errors
///
/// [`BattlePassError`]: the bytes refuse the JSON parse, or the `Settings`
/// object or the `missions` array is missing or unusable.
pub fn read(data: &[u8]) -> Result<BattlePassTable, BattlePassError> {
    let root = parse(data).map_err(|error| BattlePassError::Malformed {
        path: String::new(),
        error,
    })?;
    let mut table = BattlePassTable::default();
    read_settings(&root, &mut table)?;
    read_missions(&root, &mut table);
    Ok(table)
}

/// Read the `Settings` object (`:104-171`).
fn read_settings(root: &Json, table: &mut BattlePassTable) -> Result<(), BattlePassError> {
    let settings = root
        .member(b"Settings")
        .filter(|member| matches!(member, Json::Object(_)))
        .ok_or_else(|| BattlePassError::MissingSettings {
            path: String::new(),
        })?;
    let scalar = |name: &[u8]| {
        settings
            .member(name)
            .and_then(Json::as_uint)
            .ok_or_else(|| BattlePassError::MissingSettings {
                path: String::new(),
            })
    };
    let max_daily = scalar(b"maxDailyMissions")?;
    let max_weekly = scalar(b"maxWeeklyMissions")?;
    let end_time = scalar(b"endTime")?;
    let rewards_free = read_rewards(settings, b"rewardsFree")?;
    let rewards_premium = read_rewards(settings, b"rewardsPremium")?;
    table.settings = BattlePassSettings {
        max_daily: low_byte(max_daily),
        max_weekly: low_byte(max_weekly),
        end_time,
        rewards_free,
        rewards_premium,
    };
    Ok(())
}

/// Read a reward tier array: `[vnum, count]` uint pairs, skipping bad entries
/// (`:128-166`). A missing or non-array member refuses the settings.
fn read_rewards(settings: &Json, name: &[u8]) -> Result<Vec<(i32, i32)>, BattlePassError> {
    let missing = || BattlePassError::MissingSettings {
        path: String::new(),
    };
    let tiers = settings.member(name).ok_or_else(missing)?;
    match tiers {
        Json::Array(items) => {
            let mut rewards = Vec::new();
            for item in items {
                if let Json::Array(pair) = item {
                    let mut cells = pair.iter();
                    if let (Some(first), Some(second), None) =
                        (cells.next(), cells.next(), cells.next())
                    {
                        if let (Some(vnum), Some(count)) = (first.as_uint(), second.as_uint()) {
                            rewards.push((wrapping_i32(vnum), wrapping_i32(count)));
                        }
                    }
                }
            }
            Ok(rewards)
        }
        Json::Object(_) | Json::Text(_) | Json::Number { .. } | Json::Atom => Err(missing()),
    }
}

/// Read the `missions` array (`:173-244`): each object becomes the mission at
/// its array index, and any mission with a missing field is skipped. A
/// missing or non-array member refuses the file.
fn read_missions(root: &Json, table: &mut BattlePassTable) {
    let missions = root.member(b"missions");
    let items = match missions {
        Some(Json::Array(items)) => items,
        Some(Json::Object(_) | Json::Text(_) | Json::Number { .. } | Json::Atom) | None => return,
    };
    for (index, item) in items.iter().enumerate() {
        if let Some(mission) = read_mission(index, item) {
            table.missions.insert(mission.id, mission);
        }
    }
}

/// Read one mission object. `None` skips the mission, as legacy's `continue`
/// does (`:183-221`).
fn read_mission(index: usize, item: &Json) -> Option<BattlePassMission> {
    match item {
        Json::Object(_) => {}
        Json::Array(_) | Json::Text(_) | Json::Number { .. } | Json::Atom => return None,
    }
    let text = |name: &[u8]| match item.member(name) {
        Some(Json::Text(bytes)) => Some(bytes.clone()),
        Some(Json::Object(_) | Json::Array(_) | Json::Number { .. } | Json::Atom) | None => None,
    };
    let number = |name: &[u8]| item.member(name).and_then(Json::as_uint);
    let mission_type = mission_type_of(&text(b"missionType")?);
    let specific = specific_of(&text(b"missionSpecific")?);
    let max_progress = number(b"maxProgress")?;
    let time_info = time_of(&text(b"timeInfo")?);
    let points = number(b"points")?;
    let name = text(b"desc")?;
    let mut vnums = Vec::new();
    if specific == MISSION_TYPE_SPECIFIC {
        let array = item.member(b"vnum")?;
        match array {
            Json::Array(items) => {
                for entry in items {
                    if let Some(vnum) = entry.as_uint() {
                        vnums.push(vnum);
                    }
                }
            }
            Json::Object(_) | Json::Text(_) | Json::Number { .. } | Json::Atom => {}
        }
    }
    Some(BattlePassMission {
        id: low_word(index),
        mission_type,
        specific,
        vnums,
        max_progress,
        time_info,
        points: low_half(points),
        name,
    })
}

/// `getMissionType` (`:25-46`): unknown strings fall back to `KILL`.
fn mission_type_of(name: &[u8]) -> u8 {
    // `KILL` is both a mapped word and the unknown-string fallback; the early
    // return keeps the two roles apart for the lint.
    if name == b"KILL" {
        return MISSION_TYPE_KILL;
    }
    match name {
        b"SELL" => MISSION_TYPE_SELL,
        b"DESTROY" => MISSION_TYPE_ITEM_DESTROY,
        b"DROP" => MISSION_TYPE_ITEM_DROP,
        b"CRAFT" => MISSION_TYPE_CRAFT,
        b"CHEST_OPEN" => MISSION_TYPE_CHEST_OPEN,
        b"USE_ITEM" => MISSION_TYPE_USE_ITEM,
        b"FINISH_DUNGEON" => MISSION_TYPE_FINISH_DUNGEON,
        b"POLYMORPH" => MISSION_TYPE_POLYMORPH,
        b"MESSAGES" => MISSION_TYPE_MESSAGES,
        b"PLAYTIME" => MISSION_TYPE_PLAYTIME,
        _ => MISSION_TYPE_KILL,
    }
}

/// `getMissionTypeSpecific` (`:48-57`): unknown strings fall back to `SPECIFIC`.
fn specific_of(name: &[u8]) -> u8 {
    match name {
        b"GLOBAL" => MISSION_TYPE_GLOBAL,
        _ => MISSION_TYPE_SPECIFIC,
    }
}

/// `getMissionTime` (`:59-68`): unknown strings fall back to `DAILY`.
fn time_of(name: &[u8]) -> u8 {
    match name {
        b"WEEKLY" => MISSION_TIME_WEEKLY,
        _ => MISSION_TIME_DAILY,
    }
}

/// `static_cast<uint8_t>`: the low byte, as legacy's cast keeps it.
fn low_byte(value: u32) -> u8 {
    u8::try_from(value & 0xFF).unwrap_or(u8::MAX)
}

/// The mission id (`mission.id = i`, `:179`): the low word, as legacy's
/// narrowing keeps it.
fn low_word(index: usize) -> u16 {
    u16::try_from(index & 0xFFFF).unwrap_or(u16::MAX)
}

/// `pointsReward` (`:212`): the low half, as legacy's narrowing keeps it.
fn low_half(value: u32) -> u16 {
    u16::try_from(value & 0xFFFF).unwrap_or(u16::MAX)
}

/// A reward tier cell (`std::pair<int32_t, int32_t>`, `:561-562`): the bits
/// as legacy's conversion leaves them.
fn wrapping_i32(value: u32) -> i32 {
    i32::from_ne_bytes(value.to_ne_bytes())
}

/// The seconds from `now` until the next UTC midnight:
/// `getRemainingSecondsUntilMidnight` (`G/utils.h:44-60`) without the host zone.
#[must_use]
pub fn seconds_until_midnight(now: u32) -> u32 {
    const DAY: u32 = 86_400;
    DAY - now % DAY
}

/// The seconds from `now` until Sunday 23:59:59 UTC:
/// `getRemainingSecondsUntilEndOfWeek` (`G/utils.h:65-85`) without the host zone.
#[must_use]
pub fn seconds_until_end_of_week(now: u32) -> u32 {
    const DAY: u32 = 86_400;
    const WEEK_END: u32 = 86_399;
    let midnight = now - now % DAY;
    let days = midnight / DAY;
    let weekday = (days + 4) % 7;
    let remaining = 7 - weekday;
    midnight
        .saturating_add(remaining.saturating_mul(DAY))
        .saturating_add(WEEK_END)
        .saturating_sub(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest well-formed file.
    fn minimal() -> Vec<u8> {
        br#"{
            "Settings": {
                "maxDailyMissions": 5,
                "maxWeeklyMissions": 10,
                "endTime": 1704578399,
                "rewardsFree": [[25425, 2]],
                "rewardsPremium": [[25422, 10]]
            },
            "missions": [
                {
                    "desc": "Kill.",
                    "missionType": "KILL",
                    "missionSpecific": "SPECIFIC",
                    "timeInfo": "DAILY",
                    "points": 1300,
                    "vnum": [101],
                    "maxProgress": 8
                }
            ]
        }"#
        .to_vec()
    }

    #[test]
    fn the_minimal_file_parses() {
        let table = read(&minimal()).expect("the file parses");
        assert_eq!(table.settings.max_daily, 5);
        assert_eq!(table.settings.max_weekly, 10);
        assert_eq!(table.settings.end_time, 1_704_578_399);
        assert_eq!(table.settings.rewards_free, vec![(25425, 2)]);
        assert_eq!(table.settings.rewards_premium, vec![(25422, 10)]);
        let mission = table.mission(0);
        assert_eq!(mission.mission_type, MISSION_TYPE_KILL);
        assert_eq!(mission.specific, MISSION_TYPE_SPECIFIC);
        assert_eq!(mission.vnums, vec![101]);
        assert_eq!(mission.max_progress, 8);
        assert_eq!(mission.time_info, MISSION_TIME_DAILY);
        assert_eq!(mission.points, 1300);
        assert_eq!(mission.name, b"Kill.");
    }

    #[test]
    fn unknown_mission_strings_fall_back() {
        let mut file = minimal();
        let text = String::from_utf8(file.clone()).expect("ascii");
        file = text
            .replace("\"KILL\"", "\"BOGUS\"")
            .replace("\"SPECIFIC\"", "\"BOGUS\"")
            .replace("\"DAILY\"", "\"BOGUS\"")
            .into_bytes();
        let table = read(&file).expect("unknown strings fall back");
        let mission = table.mission(0);
        assert_eq!(mission.mission_type, MISSION_TYPE_KILL);
        assert_eq!(mission.specific, MISSION_TYPE_SPECIFIC);
        assert_eq!(mission.time_info, MISSION_TIME_DAILY);
    }

    #[test]
    fn a_global_mission_keeps_no_vnums() {
        let text = String::from_utf8(minimal()).expect("ascii");
        let file = text.replace("SPECIFIC", "GLOBAL").into_bytes();
        let table = read(&file).expect("the global mission parses");
        assert_eq!(table.mission(0).specific, MISSION_TYPE_GLOBAL);
        assert!(table.mission(0).vnums.is_empty());
    }

    #[test]
    fn a_mission_missing_a_field_is_skipped() {
        let text = String::from_utf8(minimal()).expect("ascii");
        let file = text
            .replace("\"maxProgress\": 8", "\"nope\": 8")
            .into_bytes();
        let table = read(&file).expect("the file parses");
        assert!(table.missions.is_empty());
    }

    #[test]
    fn a_non_uint_number_is_not_a_number() {
        for repair in [
            "\"maxProgress\": 8.0",
            "\"maxProgress\": -8",
            "\"maxProgress\": 4294967296",
            "\"maxProgress\": \"8\"",
        ] {
            let text = String::from_utf8(minimal()).expect("ascii");
            let file = text.replace("\"maxProgress\": 8", repair).into_bytes();
            let table = read(&file).expect("the file parses");
            assert!(table.missions.is_empty(), "{repair} skips the mission");
        }
    }

    #[test]
    fn settings_scalars_refuse_the_file() {
        for repair in [
            "\"maxDailyMissions\": 5,",
            "\"maxWeeklyMissions\": 10,",
            "\"endTime\": 1704578399,",
        ] {
            let text = String::from_utf8(minimal()).expect("ascii");
            let file = text.replace(repair, "").into_bytes();
            assert!(
                matches!(read(&file), Err(BattlePassError::MissingSettings { .. })),
                "{repair} refuses the file"
            );
        }
    }

    #[test]
    fn bad_reward_pairs_are_skipped() {
        let text = String::from_utf8(minimal()).expect("ascii");
        let file = text
            .replace("[[25425, 2]]", "[[25425, 2], [7], [8, 9, 10], [\"x\", 1]]")
            .into_bytes();
        let table = read(&file).expect("the file parses");
        assert_eq!(table.settings.rewards_free, vec![(25425, 2)]);
    }

    #[test]
    fn malformed_bytes_refuse_the_file() {
        for file in [
            b"{".to_vec(),
            b"{}".to_vec(),
            minimal()
                .into_iter()
                .chain(b" trailing".iter().copied())
                .collect::<Vec<u8>>(),
            vec![0xFF, 0xFE, 0xFD],
        ] {
            assert!(read(&file).is_err(), "malformed bytes refuse the file");
        }
    }

    #[test]
    fn escapes_decode() {
        let text = String::from_utf8(minimal()).expect("ascii");
        let file = text.replace("Kill.", r"A\/B").into_bytes();
        let table = read(&file).expect("escapes decode");
        assert_eq!(table.mission(0).name, b"A/B");
    }

    #[test]
    fn unicode_escapes_decode() {
        let file = br#"{
            "Settings": {
                "maxDailyMissions": 5,
                "maxWeeklyMissions": 10,
                "endTime": 1704578399,
                "rewardsFree": [],
                "rewardsPremium": []
            },
            "missions": [
                {
                    "desc": "A \"\u00e9\" \/",
                    "missionType": "KILL",
                    "missionSpecific": "SPECIFIC",
                    "timeInfo": "DAILY",
                    "points": 1300,
                    "vnum": [101],
                    "maxProgress": 8
                }
            ]
        }"#;
        let table = read(file).expect("escapes decode");
        assert_eq!(table.mission(0).name, "A \"é\" /".as_bytes());
    }

    #[test]
    fn an_unknown_id_answers_the_zeroed_row() {
        let table = read(&minimal()).expect("the file parses");
        assert_eq!(table.mission(77), BattlePassMission::default());
    }

    /// The owner's file: 158 missions, 30 tiers a side, the past end time.
    #[test]
    fn the_owners_file_holds_158_missions() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../legacy/gamedata/locale/europe/battlepass.json"),
        )
        .expect("the owner's battlepass.json");
        let table = read(&bytes).expect("the owner's file parses");
        assert_eq!(table.missions.len(), 158);
        assert_eq!(table.settings.max_daily, 5);
        assert_eq!(table.settings.max_weekly, 10);
        assert_eq!(table.settings.end_time, 1_704_578_399);
        assert_eq!(table.settings.rewards_free.len(), 30);
        assert_eq!(table.settings.rewards_premium.len(), 30);
        assert_eq!(table.settings.rewards_free[0], (25425, 2));
        assert_eq!(table.mission(0).vnums, vec![2492]);
        assert_eq!(table.mission(0).mission_type, MISSION_TYPE_KILL);
        assert_eq!(table.mission(0).max_progress, 8);
    }

    #[test]
    fn midnight_and_week_maths_hold() {
        assert_eq!(seconds_until_midnight(0), 86_400);
        assert_eq!(seconds_until_midnight(1), 86_399);
        assert_eq!(seconds_until_midnight(86_399), 1);
        assert_eq!(seconds_until_midnight(86_400), 86_400);
        // 2024-01-01 00:00:00 UTC is a Monday: six days and a day-end remain.
        assert_eq!(
            seconds_until_end_of_week(1_704_067_200),
            6 * 86_400 + 86_399
        );
        // 2024-01-07 00:00:00 UTC is a Sunday: the pass runs a full week on.
        assert_eq!(
            seconds_until_end_of_week(1_704_585_600),
            7 * 86_400 + 86_399
        );
    }
}
