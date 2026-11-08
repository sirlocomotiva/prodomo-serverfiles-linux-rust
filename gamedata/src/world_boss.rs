//! The world-boss table (`locale/europe/world_boss.txt`), read at boot
//! (`CWorldBoss::ReadWorldBossTableFile`, `G/worldboss.cpp:192-219`, loaded from
//! `G/input_db.cpp:969-976`).
//!
//! The parse sits on [`crate::text_file`], as the other text-group readers do.
//! `Group Config` holds one row with `cooldown`, `runtime`, `mindamage` and
//! `minlevel` (`G/worldboss.cpp:221-278`, lowercase keys). `Group Map` holds one
//! row per map with `index` (`:280-314`). `Group Boss` holds one row per boss
//! with `vnum` (`:316-350`). `Group Ranking` holds one row per rank with
//! `points`, keyed by file position (`:352-386`): the map key is the row index
//! (`m_mapRankingDataGroup.insert(std::make_pair(iLine, dwPoints))`), and the
//! season flush reads it by position (`GetRankingPointsByPosition`).
//!
//! Every error here is a case where legacy refuses the whole file and returns
//! `false`: the load fails, a group is missing, a group is empty, or a column is
//! missing. A refused file leaves legacy's table zeroed and the boot goes on
//! (`G/input_db.cpp:974-977`); the Rewrite stops the server instead (a
//! Divergence, as for every other Game data file).

use std::error::Error;
use std::fmt;

use crate::text_file::{self, TextFile};

/// The world-boss table: the config, the map indices, the boss vnums, and the
/// season points by rank position.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorldBossTable {
    /// `Config`: the cooldown, the run time, the minimum damage and the minimum
    /// level.
    pub config: WorldBossConfig,
    /// `Map`: the map indices a boss may appear on.
    pub maps: Vec<i64>,
    /// `Boss`: the vnums a spawned boss may be.
    pub bosses: Vec<u32>,
    /// `Ranking`: the season points by rank position (row index to points).
    pub ranking: Vec<u32>,
}

/// `Config`: the cooldown, run time, minimum damage and minimum level
/// (`CWorldBoss::Config`, `G/worldboss.h:52-64`; the constructor's defaults are
/// the `EConfig` fallbacks, which a refused file never reaches).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorldBossConfig {
    /// `dwCooldownTime`: seconds of break time between runs.
    pub cooldown: u32,
    /// `dwRunTime`: seconds a run lasts.
    pub runtime: u32,
    /// `dwMinDamage`: damage a hunter must deal to rank.
    pub min_damage: u32,
    /// `dwMinLevel`: level a hunter must be to rank.
    pub min_level: u32,
}

/// A row, column or value the world-boss read needs is missing or malformed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldBossError {
    /// The text reader would not cut the file.
    Text(text_file::TextFileError),
    /// A group (`config`, `map`, `boss`, `ranking`) is missing or empty.
    MissingGroup(&'static str),
    /// A column (`cooldown`, `runtime`, `mindamage`, `minlevel`, `index`,
    /// `vnum`, `points`) is missing or not a number.
    BadValue {
        /// The group the read was in.
        group: &'static str,
        /// The row index the read was on.
        row: usize,
        /// The column the read needed.
        column: &'static str,
    },
}

impl fmt::Display for WorldBossError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(error) => write!(f, "unreadable world-boss table: {error}"),
            Self::MissingGroup(group) => write!(f, "group {group} is missing or empty"),
            Self::BadValue { group, row, column } => {
                write!(f, "in group {group}, row {row}: column {column} is missing")
            }
        }
    }
}

impl Error for WorldBossError {}

/// `CWorldBoss::ReadWorldBossTableFile` (`G/worldboss.cpp:192-219`) on the
/// file's bytes: the config, the maps, the bosses and the ranking, in that
/// order, each refusing the whole table when it fails.
///
/// # Errors
///
/// Every [`WorldBossError`]: a file the text reader will not cut, a missing or
/// empty group, or a missing column.
pub fn read(data: &[u8]) -> Result<WorldBossTable, WorldBossError> {
    let file: TextFile = text_file::parse(data).map_err(WorldBossError::Text)?;
    let mut table = WorldBossTable::default();
    read_config(&file, &mut table)?;
    read_maps(&file, &mut table)?;
    read_bosses(&file, &mut table)?;
    read_ranking(&file, &mut table)?;
    Ok(table)
}

/// `ReadWorldBossConfig` (`G/worldboss.cpp:221-278`): every row's four columns,
/// the last row winning (each row overwrites the whole config).
fn read_config(file: &TextFile, table: &mut WorldBossTable) -> Result<(), WorldBossError> {
    let group = file
        .group(b"config")
        .ok_or(WorldBossError::MissingGroup("config"))?;
    let count = row_count(group);
    if count == 0 {
        return Err(WorldBossError::MissingGroup("config"));
    }
    for index in 1..=count {
        let row = index.to_string().into_bytes();
        table.config = WorldBossConfig {
            cooldown: column(group, &row, b"cooldown")?,
            runtime: column(group, &row, b"runtime")?,
            min_damage: column(group, &row, b"mindamage")?,
            min_level: column(group, &row, b"minlevel")?,
        };
    }
    Ok(())
}

/// `ReadWorldBossMap` (`G/worldboss.cpp:280-314`): one `index` per row.
fn read_maps(file: &TextFile, table: &mut WorldBossTable) -> Result<(), WorldBossError> {
    let group = file
        .group(b"map")
        .ok_or(WorldBossError::MissingGroup("map"))?;
    let count = row_count(group);
    if count == 0 {
        return Err(WorldBossError::MissingGroup("map"));
    }
    for index in 1..=count {
        let row = index.to_string().into_bytes();
        table.maps.push(column_signed(group, &row, b"index")?);
    }
    Ok(())
}

/// `ReadWorldBossVNum` (`G/worldboss.cpp:316-350`): one `vnum` per row.
fn read_bosses(file: &TextFile, table: &mut WorldBossTable) -> Result<(), WorldBossError> {
    let group = file
        .group(b"boss")
        .ok_or(WorldBossError::MissingGroup("boss"))?;
    let count = row_count(group);
    if count == 0 {
        return Err(WorldBossError::MissingGroup("boss"));
    }
    for index in 1..=count {
        let row = index.to_string().into_bytes();
        table.bosses.push(column(group, &row, b"vnum")?);
    }
    Ok(())
}

/// `ReadWorldBossRanking` (`G/worldboss.cpp:352-386`): one `points` per row, in
/// row order.
fn read_ranking(file: &TextFile, table: &mut WorldBossTable) -> Result<(), WorldBossError> {
    let group = file
        .group(b"ranking")
        .ok_or(WorldBossError::MissingGroup("ranking"))?;
    let count = row_count(group);
    if count == 0 {
        return Err(WorldBossError::MissingGroup("ranking"));
    }
    for index in 1..=count {
        let row = index.to_string().into_bytes();
        table.ranking.push(column(group, &row, b"points")?);
    }
    Ok(())
}

/// The rows of a group: `GetRowCount` is the token rows minus the `#--#`
/// header (`text_file_loader.cpp:258-270`), and the row keys are `1..=n`.
fn row_count(group: &text_file::TextGroup) -> usize {
    group
        .entries
        .iter()
        .filter(|(key, _)| key != b"#--#")
        .count()
}
/// One unsigned column of one row: the `#--#` header names the columns, and
/// `GetValue("name", out)` reads the field at the column's position
/// (`CGroupNode::GetValue`, `text_file_loader.cpp:316-349`). A missing column
/// or a row with no field there refuses the table, as legacy's `false` does;
/// the `DWORD` read is unsigned (`str_to_number`), so a non-number reads 0.
fn column(group: &text_file::TextGroup, row: &[u8], name: &[u8]) -> Result<u32, WorldBossError> {
    let header = group.get(b"#--#").ok_or(WorldBossError::BadValue {
        group: group_name(group),
        row: row_number(row),
        column: column_name(name),
    })?;
    let index = header
        .iter()
        .position(|field| field.eq_ignore_ascii_case(name))
        .ok_or(WorldBossError::BadValue {
            group: group_name(group),
            row: row_number(row),
            column: column_name(name),
        })?;
    let values = group.get(row).ok_or(WorldBossError::BadValue {
        group: group_name(group),
        row: row_number(row),
        column: column_name(name),
    })?;
    let value = values.get(index).ok_or(WorldBossError::BadValue {
        group: group_name(group),
        row: row_number(row),
        column: column_name(name),
    })?;
    Ok(unsigned(value))
}

/// One signed column of one row: the map `index` is a `long`
/// (`G/worldboss.cpp:298-303`), read the same way.
fn column_signed(
    group: &text_file::TextGroup,
    row: &[u8],
    name: &[u8],
) -> Result<i64, WorldBossError> {
    let header = group.get(b"#--#").ok_or(WorldBossError::BadValue {
        group: group_name(group),
        row: row_number(row),
        column: column_name(name),
    })?;
    let index = header
        .iter()
        .position(|field| field.eq_ignore_ascii_case(name))
        .ok_or(WorldBossError::BadValue {
            group: group_name(group),
            row: row_number(row),
            column: column_name(name),
        })?;
    let values = group.get(row).ok_or(WorldBossError::BadValue {
        group: group_name(group),
        row: row_number(row),
        column: column_name(name),
    })?;
    let value = values.get(index).ok_or(WorldBossError::BadValue {
        group: group_name(group),
        row: row_number(row),
        column: column_name(name),
    })?;
    Ok(signed(value))
}

/// The row number for an error: the keys are `1..=n` in the owner's file.
fn row_number(row: &[u8]) -> usize {
    std::str::from_utf8(row)
        .ok()
        .and_then(|text| text.parse::<usize>().ok())
        .map_or(0, |number| number.saturating_sub(1))
}
/// The group name for an error: the four groups this reader knows.
fn group_name(group: &text_file::TextGroup) -> &'static str {
    if group.name == b"config" {
        "config"
    } else if group.name == b"map" {
        "map"
    } else if group.name == b"boss" {
        "boss"
    } else {
        "ranking"
    }
}

/// The column name for an error: the seven columns this reader knows.
fn column_name(name: &[u8]) -> &'static str {
    if name == b"cooldown" {
        "cooldown"
    } else if name == b"runtime" {
        "runtime"
    } else if name == b"mindamage" {
        "mindamage"
    } else if name == b"minlevel" {
        "minlevel"
    } else if name == b"index" {
        "index"
    } else if name == b"vnum" {
        "vnum"
    } else {
        "points"
    }
}

/// `str_to_number` on a `DWORD` (`common/utils.h`): the leading digit run, 0
/// when there is none.
fn unsigned(text: &[u8]) -> u32 {
    let mut value: u32 = 0;
    for byte in text.iter().take_while(|byte| byte.is_ascii_digit()) {
        value = value
            .saturating_mul(10)
            .saturating_add(u32::from(byte - b'0'));
    }
    value
}

/// `str_to_number` on a `long`: the sign and the leading digit run.
fn signed(text: &[u8]) -> i64 {
    let (digits, negative) = match text.first() {
        Some(b'-') => (&text[1..], true),
        Some(b'+') => (&text[1..], false),
        _ => (text, false),
    };
    let mut value: i64 = 0;
    for byte in digits.iter().take_while(|byte| byte.is_ascii_digit()) {
        value = value
            .saturating_mul(10)
            .saturating_add(i64::from(byte - b'0'));
    }
    if negative {
        -value
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::read;

    /// The owner's table reads: the config row, the map, the boss and the
    /// fifty ranking points.
    #[test]
    fn the_owner_table_reads() {
        let data = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../legacy/gamedata/locale/europe/world_boss.txt"),
        )
        .expect("the owner table loads");
        let table = read(&data).expect("the owner table reads");
        assert_eq!(
            (table.config.cooldown, table.config.runtime),
            (1, 14400),
            "the owner's cooldown and run time"
        );
        assert_eq!(
            (table.config.min_damage, table.config.min_level),
            (250_000, 30),
            "the owner's minimum damage and level"
        );
        assert_eq!(table.maps, [61]);
        assert_eq!(table.bosses, [6418]);
        assert_eq!(table.ranking.len(), 50);
        assert_eq!(
            &table.ranking[..3],
            &[100, 90, 85],
            "the first three ranking points"
        );
        assert_eq!(*table.ranking.last().expect("fifty rows"), 1);
    }

    /// A missing group refuses the whole table, as legacy's null arm does.
    #[test]
    fn a_missing_group_refuses_the_table() {
        let data = b"Group Config\n{\n1 1 14400 250000 30\n}\n";
        assert!(read(data).is_err(), "no map, boss or ranking");
    }
}
