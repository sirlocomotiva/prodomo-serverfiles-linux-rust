//! SQL-free query and row-decoding boundary for the active `mob_proto` table.
//!
//! The legacy boot loader selects the 70 cells in
//! [`MOB_PROTO_QUERY_COLUMN_NAMES`]. The active wire record is 255 bytes. The
//! query omits `bMountCapacity` and `dwMobColor`, so both fields remain
//! explicitly zero. [`decode_mob_proto_query_row`] is the default strict
//! policy. [`decode_mob_proto_query_row_legacy`] is the separately named
//! compatibility policy for the source C conversion helpers.
//!
//! The exact SQL establishes `vnum` order. The pure section builder retains
//! acquired order and duplicate rows; it does not impose a second ordering
//! policy on rows that the source query already ordered. This module does not
//! execute SQL, choose a boot profile, populate a cache, or call a live boot
//! loader. A caller must acquire rows through an injected source or the sibling
//! `mob_proto_sqlx` adapter.

use std::error::Error;
use std::fmt;

use protocol::db_boot::{BootSection, BootSectionKind};
use protocol::db_records::{
    MobSkillRecord, MobTableRecord, MOB_FOLDER_BYTES, MOB_LOCALE_NAME_BYTES, MOB_NAME_BYTES,
    MOB_TABLE_RECORD_WIRE_SIZE,
};

use crate::postfix::{TablePostfix, TablePostfixError};

/// Base table name used by the source-fixed active boot query.
pub const MOB_PROTO_TABLE: &str = "mob_proto";

/// Number of cells selected by the active `mob_proto` query.
pub const MOB_PROTO_QUERY_COLUMN_COUNT: usize = 70;

/// Alias for [`MOB_PROTO_QUERY_COLUMN_COUNT`].
pub const MOB_PROTO_TABLE_QUERY_COLUMNS: usize = MOB_PROTO_QUERY_COLUMN_COUNT;

/// Maximum number of records representable by the legacy `u16` section count.
pub const MOB_PROTO_TABLE_MAX_RECORDS: usize = u16::MAX as usize;

/// Exact source-fixed packed width of one active `TMobTable` record.
pub const MOB_PROTO_TABLE_WIRE_SIZE: usize = MOB_TABLE_RECORD_WIRE_SIZE;

/// Alias for the source-fixed record width.
pub const MOB_PROTO_SECTION_RECORD_SIZE: u16 = 255;

const _: () = assert!(MOB_PROTO_SECTION_RECORD_SIZE as usize == MOB_TABLE_RECORD_WIRE_SIZE);

/// Maximum packed data bytes when all representable records are present.
pub const MOB_PROTO_TABLE_MAX_SECTION_BYTES: usize =
    MOB_PROTO_TABLE_WIRE_SIZE * MOB_PROTO_TABLE_MAX_RECORDS;

/// Maximum statement bytes that fit in the legacy `char[2048]` buffer,
/// excluding its terminating NUL.
pub const MAX_MOB_PROTO_QUERY_BYTES: usize = 2_047;

/// Maximum validated locale-column identifier bytes.
pub const MAX_MOB_PROTO_LOCALE_COLUMN_BYTES: usize = 255;

/// Default locale column selected by the active loader.
pub const DEFAULT_MOB_PROTO_LOCALE_COLUMN: &str = "name";

/// Exact query before the validated locale and postfix values are inserted.
///
/// This is a golden template, not an interpolation API. The two placeholders
/// are documented source-format positions, not arbitrary SQL fragments. With
/// the default `name` locale and empty postfix, the statement is 869 bytes.
pub const MOB_PROTO_QUERY_TEMPLATE: &str = "SELECT vnum, name, {locale}, type, rank, battle_type, level, size+0, ai_flag+0, setRaceFlag+0, setImmuneFlag+0, on_click, empire, drop_item, resurrection_vnum, folder, st, dx, ht, iq, damage_min, damage_max, max_hp, regen_cycle, regen_percent, exp, gold_min, gold_max, def, attack_speed, move_speed, aggressive_hp_pct, aggressive_sight, attack_range, polymorph_item, enchant_curse, enchant_slow, enchant_poison, enchant_stun, enchant_critical, enchant_penetrate, resist_sword, resist_twohand, resist_dagger, resist_bell, resist_fan, resist_bow, resist_fire, resist_elect, resist_magic, resist_wind, resist_poison, dam_multiply, summon, drain_sp, skill_vnum0, skill_level0, skill_vnum1, skill_level1, skill_vnum2, skill_level2, skill_vnum3, skill_level3, skill_vnum4, skill_level4, sp_berserk, sp_stoneskin, sp_godspeed, sp_deathblow, sp_revive FROM mob_proto{postfix} ORDER BY vnum;";

/// Exact query prefix before the validated locale identifier.
pub const MOB_PROTO_QUERY_PREFIX: &str = "SELECT vnum, name, ";

/// Exact query suffix after the validated locale identifier and before the
/// validated table postfix and fixed `ORDER BY vnum;` clause.
pub const MOB_PROTO_QUERY_SUFFIX: &str = ", type, rank, battle_type, level, size+0, ai_flag+0, setRaceFlag+0, setImmuneFlag+0, on_click, empire, drop_item, resurrection_vnum, folder, st, dx, ht, iq, damage_min, damage_max, max_hp, regen_cycle, regen_percent, exp, gold_min, gold_max, def, attack_speed, move_speed, aggressive_hp_pct, aggressive_sight, attack_range, polymorph_item, enchant_curse, enchant_slow, enchant_poison, enchant_stun, enchant_critical, enchant_penetrate, resist_sword, resist_twohand, resist_dagger, resist_bell, resist_fan, resist_bow, resist_fire, resist_elect, resist_magic, resist_wind, resist_poison, dam_multiply, summon, drain_sp, skill_vnum0, skill_level0, skill_vnum1, skill_level1, skill_vnum2, skill_level2, skill_vnum3, skill_level3, skill_vnum4, skill_level4, sp_berserk, sp_stoneskin, sp_godspeed, sp_deathblow, sp_revive FROM mob_proto";

/// Fixed active query cell names in positional order.
///
/// Index 2 is `locale_name` in the source enum. The concrete SQL identifier
/// is selected explicitly by [`MobProtoLocaleColumn`].
pub const MOB_PROTO_QUERY_COLUMN_NAMES: [&str; MOB_PROTO_QUERY_COLUMN_COUNT] = [
    "vnum",
    "name",
    "locale_name",
    "type",
    "rank",
    "battle_type",
    "level",
    "size+0",
    "ai_flag+0",
    "setRaceFlag+0",
    "setImmuneFlag+0",
    "on_click",
    "empire",
    "drop_item",
    "resurrection_vnum",
    "folder",
    "st",
    "dx",
    "ht",
    "iq",
    "damage_min",
    "damage_max",
    "max_hp",
    "regen_cycle",
    "regen_percent",
    "exp",
    "gold_min",
    "gold_max",
    "def",
    "attack_speed",
    "move_speed",
    "aggressive_hp_pct",
    "aggressive_sight",
    "attack_range",
    "polymorph_item",
    "enchant_curse",
    "enchant_slow",
    "enchant_poison",
    "enchant_stun",
    "enchant_critical",
    "enchant_penetrate",
    "resist_sword",
    "resist_twohand",
    "resist_dagger",
    "resist_bell",
    "resist_fan",
    "resist_bow",
    "resist_fire",
    "resist_elect",
    "resist_magic",
    "resist_wind",
    "resist_poison",
    "dam_multiply",
    "summon",
    "drain_sp",
    "skill_vnum0",
    "skill_level0",
    "skill_vnum1",
    "skill_level1",
    "skill_vnum2",
    "skill_level2",
    "skill_vnum3",
    "skill_level3",
    "skill_vnum4",
    "skill_level4",
    "sp_berserk",
    "sp_stoneskin",
    "sp_godspeed",
    "sp_deathblow",
    "sp_revive",
];

/// One lossless value from an `mob_proto` query row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobProtoQueryValue {
    /// Non-NULL UTF-8 text supplied by a caller.
    Text(String),
    /// Non-NULL raw bytes supplied by a database adapter.
    Bytes(Vec<u8>),
    /// SQL `NULL`.
    Null,
    /// A source extraction or conversion error.
    Error(String),
}

impl MobProtoQueryValue {
    /// Construct a text cell.
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    /// Construct a raw-byte cell without UTF-8 conversion.
    #[must_use]
    pub fn bytes(value: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(value.into())
    }

    /// Construct a SQL `NULL` cell.
    #[must_use]
    pub const fn null() -> Self {
        Self::Null
    }

    /// Construct a source-error cell.
    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error(message.into())
    }

    /// Borrow bytes for text or byte cells.
    #[must_use]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Text(value) => Some(value.as_bytes()),
            Self::Bytes(value) => Some(value.as_slice()),
            Self::Null | Self::Error(_) => None,
        }
    }
}

impl From<String> for MobProtoQueryValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for MobProtoQueryValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<Vec<u8>> for MobProtoQueryValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl From<&[u8]> for MobProtoQueryValue {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl<const N: usize> From<[u8; N]> for MobProtoQueryValue {
    fn from(value: [u8; N]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl From<Option<Vec<u8>>> for MobProtoQueryValue {
    fn from(value: Option<Vec<u8>>) -> Self {
        value.map_or(Self::Null, Self::Bytes)
    }
}

impl From<Option<String>> for MobProtoQueryValue {
    fn from(value: Option<String>) -> Self {
        value.map_or(Self::Null, Self::Text)
    }
}

/// One source-shaped `mob_proto` query row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobProtoQueryRow {
    columns: Vec<MobProtoQueryValue>,
}

impl MobProtoQueryRow {
    /// Construct a row while retaining its supplied width.
    #[must_use]
    pub fn new<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = MobProtoQueryValue>,
    {
        Self {
            columns: columns.into_iter().collect(),
        }
    }

    /// Alias for [`Self::new`].
    #[must_use]
    pub fn from_columns<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = MobProtoQueryValue>,
    {
        Self::new(columns)
    }

    /// Construct and validate the exact 70-column shape with bounded
    /// accumulation.
    ///
    /// The iterator is never advanced past cell 71. Its length hint is not
    /// trusted for allocation.
    ///
    /// # Errors
    ///
    /// Returns [`MobProtoRowError::ColumnCount`] for any other width, or
    /// [`MobProtoRowError::AllocationFailed`] if the bounded 70-cell
    /// reservation fails.
    pub fn try_new<I>(columns: I) -> Result<Self, MobProtoRowError>
    where
        I: IntoIterator<Item = MobProtoQueryValue>,
    {
        let mut row = Self {
            columns: Vec::new(),
        };
        row.columns
            .try_reserve_exact(MOB_PROTO_QUERY_COLUMN_COUNT)
            .map_err(|_| MobProtoRowError::AllocationFailed {
                requested: MOB_PROTO_QUERY_COLUMN_COUNT,
            })?;
        for value in columns {
            if row.columns.len() == MOB_PROTO_QUERY_COLUMN_COUNT {
                return Err(MobProtoRowError::ColumnCount {
                    expected: MOB_PROTO_QUERY_COLUMN_COUNT,
                    actual: MOB_PROTO_QUERY_COLUMN_COUNT + 1,
                });
            }
            row.columns.push(value);
        }
        check_row_width(&row)?;
        Ok(row)
    }

    /// Construct from a statically sized, correctly shaped column set.
    #[must_use]
    pub fn from_typed_columns(columns: [MobProtoQueryValue; MOB_PROTO_QUERY_COLUMN_COUNT]) -> Self {
        Self::new(columns)
    }

    /// Borrow all cells in query order.
    #[must_use]
    pub fn columns(&self) -> &[MobProtoQueryValue] {
        &self.columns
    }

    /// Consume the row and return all cells in query order.
    #[must_use]
    pub fn into_columns(self) -> Vec<MobProtoQueryValue> {
        self.columns
    }

    /// Return the supplied cell count.
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
}

impl From<[MobProtoQueryValue; MOB_PROTO_QUERY_COLUMN_COUNT]> for MobProtoQueryRow {
    fn from(columns: [MobProtoQueryValue; MOB_PROTO_QUERY_COLUMN_COUNT]) -> Self {
        Self::from_typed_columns(columns)
    }
}

impl TryFrom<Vec<MobProtoQueryValue>> for MobProtoQueryRow {
    type Error = MobProtoRowError;

    fn try_from(columns: Vec<MobProtoQueryValue>) -> Result<Self, Self::Error> {
        Self::try_new(columns)
    }
}

/// Alias emphasizing the physical table name.
pub type MobProtoTableQueryRow = MobProtoQueryRow;
/// Alias for one `mob_proto` query cell.
pub type MobProtoTableQueryValue = MobProtoQueryValue;

/// Return a stable diagnostic name for one selected expression.
#[must_use]
pub fn mob_proto_query_column_name(index: usize) -> &'static str {
    MOB_PROTO_QUERY_COLUMN_NAMES
        .get(index)
        .copied()
        .unwrap_or("unknown")
}

/// A failure while validating or decoding one `mob_proto` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobProtoRowError {
    /// The row did not contain exactly 70 cells.
    ColumnCount {
        /// Required cell count.
        expected: usize,
        /// Supplied cell count.
        actual: usize,
    },
    /// The bounded typed-row constructor could not reserve 70 cells.
    AllocationFailed {
        /// Number of cells requested by the failed bounded reservation.
        requested: usize,
    },
    /// A required cell was SQL `NULL` in the strict policy.
    Null {
        /// Zero-based query column.
        column: usize,
    },
    /// A source cell could not be obtained.
    Source {
        /// Zero-based query column.
        column: usize,
        /// Source diagnostic.
        message: String,
    },
    /// A strict numeric cell was not a complete decimal integer.
    InvalidNumber {
        /// Zero-based query column.
        column: usize,
        /// Lossy diagnostic representation of the source bytes.
        value: String,
        /// Target integer type.
        target: &'static str,
    },
    /// A syntactically valid integer exceeded its target.
    NumberOverflow {
        /// Zero-based query column.
        column: usize,
        /// Lossy diagnostic representation of the source bytes.
        value: String,
        /// Target integer type.
        target: &'static str,
    },
    /// A strict name was longer than the source content bound.
    NameTooLong {
        /// Zero-based name column, 1, 2, or 15.
        column: usize,
        /// Supplied byte length.
        length: usize,
        /// Maximum supplied length.
        maximum: usize,
    },
    /// A strict name contained a NUL before its final byte.
    NameInteriorNul {
        /// Zero-based name column, 1, 2, or 15.
        column: usize,
        /// Byte offset of the first NUL.
        index: usize,
    },
}

impl fmt::Display for MobProtoRowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ColumnCount { expected, actual } => write!(
                formatter,
                "mob-proto query row has {actual} columns; expected {expected}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "mob-proto query row could not reserve {requested} bounded cells"
            ),
            Self::Null { column } => write!(
                formatter,
                "mob-proto query column {} is NULL",
                mob_proto_query_column_name(*column)
            ),
            Self::Source { column, message } => write!(
                formatter,
                "mob-proto query column {} could not be read: {message}",
                mob_proto_query_column_name(*column)
            ),
            Self::InvalidNumber {
                column,
                value,
                target,
            } => write!(
                formatter,
                "mob-proto query column {} value {value:?} is not a strict {target}",
                mob_proto_query_column_name(*column)
            ),
            Self::NumberOverflow {
                column,
                value,
                target,
            } => write!(
                formatter,
                "mob-proto query column {} value {value:?} overflows {target}",
                mob_proto_query_column_name(*column)
            ),
            Self::NameTooLong {
                column,
                length,
                maximum,
            } => write!(
                formatter,
                "mob-proto name column {} is {length} bytes; maximum is {maximum}",
                mob_proto_query_column_name(*column)
            ),
            Self::NameInteriorNul { column, index } => write!(
                formatter,
                "mob-proto name column {} has an interior NUL at byte {index}",
                mob_proto_query_column_name(*column)
            ),
        }
    }
}

impl Error for MobProtoRowError {}

/// Bounds applied before allocating an `mob_proto` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobProtoSectionLimits {
    /// Maximum accepted source rows.
    pub max_records: usize,
    /// Maximum accepted packed record-data bytes, checked by the pure
    /// builder after acquisition. This is not a pre-acquisition row or
    /// memory cap; [`MobProtoLoader`] and `SQLx` acquisition cap rows with
    /// `max_records`.
    pub max_data_bytes: usize,
}

impl MobProtoSectionLimits {
    /// Construct a record cap and leave the byte cap at `usize::MAX`.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self {
            max_records,
            max_data_bytes: usize::MAX,
        }
    }

    /// Construct both record and packed-byte caps.
    #[must_use]
    pub const fn with_data_limit(max_records: usize, max_data_bytes: usize) -> Self {
        Self {
            max_records,
            max_data_bytes,
        }
    }

    /// Alias for [`Self::with_data_limit`].
    #[must_use]
    pub const fn with_limits(max_records: usize, max_data_bytes: usize) -> Self {
        Self::with_data_limit(max_records, max_data_bytes)
    }
}

impl Default for MobProtoSectionLimits {
    fn default() -> Self {
        Self {
            max_records: MOB_PROTO_TABLE_MAX_RECORDS,
            max_data_bytes: MOB_PROTO_TABLE_MAX_SECTION_BYTES,
        }
    }
}

/// A checked `mob_proto` section-build failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobProtoSectionError {
    /// The source supplied more rows than the caller allowed.
    TooManyRecords {
        /// Supplied row count.
        count: usize,
        /// Configured record cap.
        maximum: usize,
    },
    /// The row count does not fit the section's `u16` count.
    CountOverflow {
        /// Supplied row count.
        count: usize,
    },
    /// The fixed record width does not fit the section's `u16` width.
    RecordSizeOverflow {
        /// Fixed record width.
        size: usize,
    },
    /// Checked record-data size arithmetic overflowed `usize`.
    DataSizeOverflow {
        /// Row count used in multiplication.
        count: usize,
    },
    /// The fallible packed-output reservation failed.
    ///
    /// Protocol `record.encode()` temporaries remain the codec's existing
    /// fixed-width 255-byte operation and are outside this fallible output
    /// reservation.
    AllocationFailed {
        /// Requested packed output byte length.
        requested: usize,
    },
    /// Required packed data exceeds the configured byte cap.
    DataTooLarge {
        /// Required packed byte length.
        length: usize,
        /// Configured byte cap.
        maximum: usize,
    },
    /// A source row failed the selected conversion policy.
    Row {
        /// Zero-based source row.
        index: usize,
        /// Row failure.
        source: MobProtoRowError,
    },
    /// A protocol encoder returned an unexpected record width.
    RecordSizeMismatch {
        /// Zero-based source row.
        index: usize,
        /// Required packed width.
        expected: usize,
        /// Actual encoded width.
        actual: usize,
    },
    /// Generated data did not have its checked length.
    PackedDataLengthMismatch {
        /// Required packed length.
        expected: usize,
        /// Actual generated length.
        actual: usize,
    },
}

impl fmt::Display for MobProtoSectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRecords { count, maximum } => write!(
                formatter,
                "mob-proto section has {count} rows; configured limit is {maximum}"
            ),
            Self::CountOverflow { count } => {
                write!(
                    formatter,
                    "mob-proto section count {count} does not fit u16"
                )
            }
            Self::RecordSizeOverflow { size } => {
                write!(formatter, "mob-proto record width {size} does not fit u16")
            }
            Self::DataSizeOverflow { count } => {
                write!(
                    formatter,
                    "mob-proto section byte size overflows for {count} rows"
                )
            }
            Self::AllocationFailed { requested } => write!(
                formatter,
                "mob-proto section could not allocate {requested} output bytes"
            ),
            Self::DataTooLarge { length, maximum } => write!(
                formatter,
                "mob-proto section data length {length} exceeds limit {maximum}"
            ),
            Self::Row { index, source } => {
                write!(formatter, "mob-proto row {index} is invalid: {source}")
            }
            Self::RecordSizeMismatch {
                index,
                expected,
                actual,
            } => write!(
                formatter,
                "mob-proto row {index} encoded to {actual} bytes; expected {expected}"
            ),
            Self::PackedDataLengthMismatch { expected, actual } => write!(
                formatter,
                "mob-proto section packed {actual} bytes; expected {expected}"
            ),
        }
    }
}

impl Error for MobProtoSectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Row { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// A failure while validating a locale-column identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobProtoLocaleColumnError {
    /// The identifier was empty.
    Empty,
    /// The identifier exceeded the bounded byte length.
    TooLong {
        /// Supplied UTF-8 byte length.
        length: usize,
        /// Maximum accepted byte length.
        maximum: usize,
    },
    /// The first byte was not an ASCII letter or underscore.
    InvalidFirstByte {
        /// First byte value.
        byte: u8,
    },
    /// A later byte was not ASCII alphanumeric or underscore.
    InvalidByte {
        /// Zero-based byte offset.
        index: usize,
        /// Disallowed byte value.
        byte: u8,
    },
}

impl fmt::Display for MobProtoLocaleColumnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("mob-proto locale column is empty"),
            Self::TooLong { length, maximum } => write!(
                formatter,
                "mob-proto locale column is {length} bytes; maximum is {maximum}"
            ),
            Self::InvalidFirstByte { byte } => write!(
                formatter,
                "mob-proto locale column starts with non-identifier byte {byte:#04x}"
            ),
            Self::InvalidByte { index, byte } => write!(
                formatter,
                "mob-proto locale column byte {byte:#04x} at offset {index} is not ASCII alphanumeric or '_'"
            ),
        }
    }
}

impl Error for MobProtoLocaleColumnError {}

/// A validated standalone SQL identifier for the locale-name projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobProtoLocaleColumn {
    value: String,
}

impl MobProtoLocaleColumn {
    /// Validate one locale identifier.
    ///
    /// # Errors
    ///
    /// Returns [`MobProtoLocaleColumnError`] if the value is empty, too long,
    /// starts with a non-ASCII-letter, or contains a byte outside the ASCII
    /// identifier allowlist.
    pub fn parse(value: &str) -> Result<Self, MobProtoLocaleColumnError> {
        if value.is_empty() {
            return Err(MobProtoLocaleColumnError::Empty);
        }
        if value.len() > MAX_MOB_PROTO_LOCALE_COLUMN_BYTES {
            return Err(MobProtoLocaleColumnError::TooLong {
                length: value.len(),
                maximum: MAX_MOB_PROTO_LOCALE_COLUMN_BYTES,
            });
        }
        let first = value.as_bytes()[0];
        if !(first.is_ascii_alphabetic() || first == b'_') {
            return Err(MobProtoLocaleColumnError::InvalidFirstByte { byte: first });
        }
        if let Some((index, byte)) = value
            .bytes()
            .enumerate()
            .skip(1)
            .find(|(_, byte)| !byte.is_ascii_alphanumeric() && *byte != b'_')
        {
            return Err(MobProtoLocaleColumnError::InvalidByte { index, byte });
        }
        Ok(Self {
            value: value.to_owned(),
        })
    }

    /// Select the default `name` column, or validate an explicit override.
    ///
    /// `None` selects the legacy default. `Some("")` is rejected rather than
    /// silently normalized.
    ///
    /// # Errors
    ///
    /// Returns [`MobProtoLocaleColumnError`] for an invalid explicit value.
    pub fn from_config(value: Option<&str>) -> Result<Self, MobProtoLocaleColumnError> {
        value.map_or_else(|| Ok(Self::default()), Self::parse)
    }

    /// Borrow the validated identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl Default for MobProtoLocaleColumn {
    fn default() -> Self {
        Self {
            value: DEFAULT_MOB_PROTO_LOCALE_COLUMN.to_owned(),
        }
    }
}

impl TryFrom<&str> for MobProtoLocaleColumn {
    type Error = MobProtoLocaleColumnError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TryFrom<String> for MobProtoLocaleColumn {
    type Error = MobProtoLocaleColumnError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

/// Alias emphasizing the selected value's purpose.
pub type MobLocaleColumn = MobProtoLocaleColumn;

/// A defensive failure while composing the fixed `mob_proto` query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobProtoQueryBuildError {
    /// The final statement exceeded the legacy buffer bound.
    QueryTooLong {
        /// Generated statement byte length.
        length: usize,
        /// Maximum accepted statement byte length.
        maximum: usize,
    },
}

impl fmt::Display for MobProtoQueryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueryTooLong { length, maximum } => write!(
                formatter,
                "generated mob-proto query is {length} bytes; maximum is {maximum}"
            ),
        }
    }
}

impl Error for MobProtoQueryBuildError {}

/// A failure while validating query inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobProtoBoundaryError {
    /// `TABLE_POSTFIX` failed validation.
    Postfix(TablePostfixError),
    /// The locale-column identifier failed validation.
    LocaleColumn(MobProtoLocaleColumnError),
    /// A defensive query construction check failed.
    Query(MobProtoQueryBuildError),
}

impl fmt::Display for MobProtoBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postfix(source) => source.fmt(formatter),
            Self::LocaleColumn(source) => source.fmt(formatter),
            Self::Query(source) => source.fmt(formatter),
        }
    }
}

impl Error for MobProtoBoundaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Postfix(source) => Some(source),
            Self::LocaleColumn(source) => Some(source),
            Self::Query(source) => Some(source),
        }
    }
}

/// One immutable checked `mob_proto` read statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobProtoQuery {
    statement: String,
    table_name: String,
    postfix: TablePostfix,
    locale_column: MobProtoLocaleColumn,
}

impl MobProtoQuery {
    /// Build the exact active query from two validated identifier values.
    ///
    /// # Errors
    ///
    /// Returns [`MobProtoQueryBuildError`] if a future invariant change makes
    /// the final statement exceed the legacy `char[2048]` bound.
    pub fn new(
        postfix: &TablePostfix,
        locale_column: &MobProtoLocaleColumn,
    ) -> Result<Self, MobProtoQueryBuildError> {
        let table_name = format!("{MOB_PROTO_TABLE}{}", postfix.as_str());
        let capacity = MOB_PROTO_QUERY_PREFIX.len()
            + locale_column.as_str().len()
            + MOB_PROTO_QUERY_SUFFIX.len()
            + postfix.as_str().len()
            + " ORDER BY vnum;".len();
        let mut statement = String::with_capacity(capacity);
        statement.push_str(MOB_PROTO_QUERY_PREFIX);
        statement.push_str(locale_column.as_str());
        statement.push_str(MOB_PROTO_QUERY_SUFFIX);
        statement.push_str(postfix.as_str());
        statement.push_str(" ORDER BY vnum;");
        if statement.len() > MAX_MOB_PROTO_QUERY_BYTES {
            return Err(MobProtoQueryBuildError::QueryTooLong {
                length: statement.len(),
                maximum: MAX_MOB_PROTO_QUERY_BYTES,
            });
        }
        Ok(Self {
            statement,
            table_name,
            postfix: postfix.clone(),
            locale_column: locale_column.clone(),
        })
    }

    /// Build from optional validated configuration values.
    ///
    /// # Errors
    ///
    /// Returns an invalid postfix, locale identifier, or defensive query error.
    pub fn from_config(
        configured_postfix: Option<&str>,
        configured_locale_column: Option<&str>,
    ) -> Result<Self, MobProtoBoundaryError> {
        let postfix = TablePostfix::from_config(configured_postfix)
            .map_err(MobProtoBoundaryError::Postfix)?;
        let locale = MobProtoLocaleColumn::from_config(configured_locale_column)
            .map_err(MobProtoBoundaryError::LocaleColumn)?;
        Self::new(&postfix, &locale).map_err(MobProtoBoundaryError::Query)
    }

    /// Borrow the exact query text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.statement
    }

    /// Borrow the generated table identifier.
    #[must_use]
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    /// Borrow the validated postfix.
    #[must_use]
    pub const fn postfix(&self) -> &TablePostfix {
        &self.postfix
    }

    /// Borrow the validated locale-column identifier.
    #[must_use]
    pub const fn locale_column(&self) -> &MobProtoLocaleColumn {
        &self.locale_column
    }
}

/// Alias emphasizing the physical table name.
pub type MobProtoTableQuery = MobProtoQuery;
/// Alias emphasizing the physical table name.
pub type MobProtoTableQueryBuildError = MobProtoQueryBuildError;
/// Alias emphasizing the physical table name.
pub type MobProtoTableSectionLimits = MobProtoSectionLimits;
/// Alias emphasizing the physical table name.
pub type MobProtoTableRowError = MobProtoRowError;
/// Alias emphasizing the physical table name.
pub type MobProtoTableSectionError = MobProtoSectionError;

/// An injected source of checked `mob_proto` query rows.
pub trait MobProtoRowSource {
    /// Source-specific error type.
    type Error: fmt::Display;

    /// Obtain raw rows in the order required by the section.
    ///
    /// # Errors
    ///
    /// Returns the source error when rows cannot be obtained. A source failure
    /// must not become an empty row set.
    fn query_rows(&self, query: &MobProtoQuery) -> Result<Vec<MobProtoQueryRow>, Self::Error>;
}

impl<F, E> MobProtoRowSource for F
where
    F: Fn(&MobProtoQuery) -> Result<Vec<MobProtoQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn query_rows(&self, query: &MobProtoQuery) -> Result<Vec<MobProtoQueryRow>, Self::Error> {
        self(query)
    }
}

/// Alias emphasizing the physical table name.
pub use MobProtoRowSource as MobProtoTableRowSource;

/// A failure while obtaining or building rows through an injected loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobProtoLoadError<E> {
    /// The injected source failed.
    Source(E),
    /// The source returned zero rows, which the legacy loader rejects.
    EmptyResult,
    /// Raw rows failed the selected conversion or section limits.
    Rows(MobProtoSectionError),
}

impl<E: fmt::Display> fmt::Display for MobProtoLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write!(formatter, "mob-proto row source failed: {source}"),
            Self::EmptyResult => write!(formatter, "mob-proto source returned no rows"),
            Self::Rows(source) => {
                write!(formatter, "mob-proto rows could not be loaded: {source}")
            }
        }
    }
}

impl<E: Error + 'static> Error for MobProtoLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::EmptyResult => None,
            Self::Rows(source) => Some(source),
        }
    }
}

/// Reusable checked-query, row-limit, and packing-policy holder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobProtoLoader {
    query: MobProtoQuery,
    limits: MobProtoSectionLimits,
}

impl MobProtoLoader {
    /// Construct a loader with the default `name` locale projection.
    ///
    /// # Errors
    ///
    /// Returns a defensive query-construction error.
    pub fn new(
        postfix: &TablePostfix,
        limits: MobProtoSectionLimits,
    ) -> Result<Self, MobProtoQueryBuildError> {
        Self::new_with_locale(postfix, &MobProtoLocaleColumn::default(), limits)
    }

    /// Construct a loader with a validated locale projection.
    ///
    /// # Errors
    ///
    /// Returns a defensive query-construction error.
    pub fn new_with_locale(
        postfix: &TablePostfix,
        locale_column: &MobProtoLocaleColumn,
        limits: MobProtoSectionLimits,
    ) -> Result<Self, MobProtoQueryBuildError> {
        Ok(Self {
            query: MobProtoQuery::new(postfix, locale_column)?,
            limits,
        })
    }

    /// Construct from raw optional configuration values.
    ///
    /// # Errors
    ///
    /// Returns invalid postfix, locale, or defensive query input.
    pub fn from_config(
        configured_postfix: Option<&str>,
        configured_locale_column: Option<&str>,
        limits: MobProtoSectionLimits,
    ) -> Result<Self, MobProtoBoundaryError> {
        let postfix = TablePostfix::from_config(configured_postfix)
            .map_err(MobProtoBoundaryError::Postfix)?;
        let locale = MobProtoLocaleColumn::from_config(configured_locale_column)
            .map_err(MobProtoBoundaryError::LocaleColumn)?;
        Self::new_with_locale(&postfix, &locale, limits).map_err(MobProtoBoundaryError::Query)
    }

    /// Borrow the immutable checked query.
    #[must_use]
    pub const fn query(&self) -> &MobProtoQuery {
        &self.query
    }

    /// Return configured source and packed-byte limits.
    #[must_use]
    pub const fn limits(&self) -> MobProtoSectionLimits {
        self.limits
    }

    /// Acquire raw rows and strictly build a section.
    ///
    /// # Errors
    ///
    /// Returns source or selected row/section failure.
    pub fn load_section<S>(&self, source: &S) -> Result<BootSection, MobProtoLoadError<S::Error>>
    where
        S: MobProtoRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(MobProtoLoadError::Source)?;
        if rows.is_empty() {
            return Err(MobProtoLoadError::EmptyResult);
        }
        build_mob_proto_section_with_limits(&rows, self.limits).map_err(MobProtoLoadError::Rows)
    }

    /// Acquire rows and build with the explicit legacy conversion policy.
    ///
    /// # Errors
    ///
    /// Returns source or selected row/section failure.
    pub fn load_section_legacy<S>(
        &self,
        source: &S,
    ) -> Result<BootSection, MobProtoLoadError<S::Error>>
    where
        S: MobProtoRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(MobProtoLoadError::Source)?;
        if rows.is_empty() {
            return Err(MobProtoLoadError::EmptyResult);
        }
        build_mob_proto_section_legacy_with_limits(&rows, self.limits)
            .map_err(MobProtoLoadError::Rows)
    }
}

/// Alias emphasizing the physical table name.
pub type MobProtoTableLoader = MobProtoLoader;

fn check_row_width(row: &MobProtoQueryRow) -> Result<(), MobProtoRowError> {
    if row.columns.len() == MOB_PROTO_QUERY_COLUMN_COUNT {
        Ok(())
    } else {
        Err(MobProtoRowError::ColumnCount {
            expected: MOB_PROTO_QUERY_COLUMN_COUNT,
            actual: row.columns.len(),
        })
    }
}

fn cell_bytes(value: &MobProtoQueryValue, column: usize) -> Result<&[u8], MobProtoRowError> {
    match value {
        MobProtoQueryValue::Text(text) => Ok(text.as_bytes()),
        MobProtoQueryValue::Bytes(bytes) => Ok(bytes.as_slice()),
        MobProtoQueryValue::Null => Err(MobProtoRowError::Null { column }),
        MobProtoQueryValue::Error(message) => Err(MobProtoRowError::Source {
            column,
            message: message.clone(),
        }),
    }
}

fn diagnostic_value(value: &MobProtoQueryValue) -> String {
    const MAX_DIAGNOSTIC_BYTES: usize = 96;
    match value {
        MobProtoQueryValue::Text(text) => {
            if text.len() <= MAX_DIAGNOSTIC_BYTES {
                text.clone()
            } else {
                let mut bounded =
                    String::from_utf8_lossy(&text.as_bytes()[..MAX_DIAGNOSTIC_BYTES]).into_owned();
                bounded.push('…');
                bounded
            }
        }
        MobProtoQueryValue::Bytes(bytes) => {
            if bytes.len() <= MAX_DIAGNOSTIC_BYTES {
                String::from_utf8_lossy(bytes).into_owned()
            } else {
                let mut bounded =
                    String::from_utf8_lossy(&bytes[..MAX_DIAGNOSTIC_BYTES]).into_owned();
                bounded.push('…');
                bounded
            }
        }
        MobProtoQueryValue::Null => "<NULL>".to_owned(),
        MobProtoQueryValue::Error(message) => message.clone(),
    }
}

fn strict_unsigned_decimal(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit)
}

fn strict_signed_decimal(bytes: &[u8]) -> bool {
    let Some(first) = bytes.first().copied() else {
        return false;
    };
    let digits = if first == b'-' { &bytes[1..] } else { bytes };
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
}

fn decode_strict_u32(value: &MobProtoQueryValue, column: usize) -> Result<u32, MobProtoRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_unsigned_decimal(bytes) {
        return Err(MobProtoRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "u32",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| MobProtoRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target: "u32",
    })?;
    text.parse::<u32>()
        .map_err(|_| MobProtoRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target: "u32",
        })
}

fn decode_strict_u8(value: &MobProtoQueryValue, column: usize) -> Result<u8, MobProtoRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_unsigned_decimal(bytes) {
        return Err(MobProtoRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "u8",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| MobProtoRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target: "u8",
    })?;
    text.parse::<u8>()
        .map_err(|_| MobProtoRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target: "u8",
        })
}

fn decode_strict_u16(value: &MobProtoQueryValue, column: usize) -> Result<u16, MobProtoRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_unsigned_decimal(bytes) {
        return Err(MobProtoRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "u16",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| MobProtoRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target: "u16",
    })?;
    text.parse::<u16>()
        .map_err(|_| MobProtoRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target: "u16",
        })
}

fn decode_strict_i16(value: &MobProtoQueryValue, column: usize) -> Result<i16, MobProtoRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_signed_decimal(bytes) {
        return Err(MobProtoRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "i16",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| MobProtoRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target: "i16",
    })?;
    text.parse::<i16>()
        .map_err(|_| MobProtoRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target: "i16",
        })
}

fn decode_strict_i8(value: &MobProtoQueryValue, column: usize) -> Result<i8, MobProtoRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_signed_decimal(bytes) {
        return Err(MobProtoRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "i8",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| MobProtoRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target: "i8",
    })?;
    text.parse::<i8>()
        .map_err(|_| MobProtoRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target: "i8",
        })
}

fn is_strict_decimal_float(bytes: &[u8]) -> bool {
    let mut index = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        index += 1;
    }
    let integer_start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    let integer_digits = index - integer_start;
    let mut fraction_digits = 0;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        fraction_digits = index - fraction_start;
    }
    if integer_digits == 0 && fraction_digits == 0 {
        return false;
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }
    index == bytes.len()
}

fn decode_strict_f32(value: &MobProtoQueryValue, column: usize) -> Result<f32, MobProtoRowError> {
    let bytes = cell_bytes(value, column)?;
    if !is_strict_decimal_float(bytes) {
        return Err(MobProtoRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "f32",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| MobProtoRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target: "f32",
    })?;
    let parsed = text
        .parse::<f32>()
        .map_err(|_| MobProtoRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "f32",
        })?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(MobProtoRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target: "f32",
        })
    }
}

fn decode_fixed_strict<const N: usize>(
    value: &MobProtoQueryValue,
    column: usize,
) -> Result<[u8; N], MobProtoRowError> {
    let bytes = cell_bytes(value, column)?;
    if let Some(index) = bytes.iter().position(|byte| *byte == 0) {
        if bytes.len() != N || index != N - 1 {
            return Err(MobProtoRowError::NameInteriorNul { column, index });
        }
    }
    let maximum = if bytes.last() == Some(&0) { N } else { N - 1 };
    if bytes.len() > maximum {
        return Err(MobProtoRowError::NameTooLong {
            column,
            length: bytes.len(),
            maximum,
        });
    }
    let mut output = [0_u8; N];
    output[..bytes.len()].copy_from_slice(bytes);
    Ok(output)
}

fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn legacy_c_string_bytes(
    value: &MobProtoQueryValue,
    column: usize,
) -> Result<&[u8], MobProtoRowError> {
    let bytes = match value {
        MobProtoQueryValue::Null => &[][..],
        MobProtoQueryValue::Text(text) => text.as_bytes(),
        MobProtoQueryValue::Bytes(bytes) => bytes.as_slice(),
        MobProtoQueryValue::Error(message) => {
            return Err(MobProtoRowError::Source {
                column,
                message: message.clone(),
            });
        }
    };
    Ok(match bytes.iter().position(|byte| *byte == 0) {
        Some(index) => &bytes[..index],
        None => bytes,
    })
}

// One above `u32::MAX` is enough to distinguish all later magnitudes while
// retaining a bounded accumulator for x86 conversion endpoints.
#[allow(clippy::cast_lossless)] // `From::from` is not const on the workspace compiler.
const LEGACY_MAGNITUDE_CAP: u64 = u32::MAX as u64 + 1;

struct LegacyIntegerPrefix {
    negative: bool,
    has_digits: bool,
    low_bits: u32,
    magnitude_over_u32: bool,
}

fn legacy_integer_prefix(bytes: &[u8]) -> LegacyIntegerPrefix {
    let mut index = 0;
    while index < bytes.len() && is_c_space(bytes[index]) {
        index += 1;
    }
    let mut negative = false;
    if index < bytes.len() && matches!(bytes[index], b'+' | b'-') {
        negative = bytes[index] == b'-';
        index += 1;
    }
    let digit_start = index;
    let mut low_bits = 0_u32;
    let mut capped_magnitude = 0_u64;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        let digit = u32::from(bytes[index] - b'0');
        low_bits = low_bits.wrapping_mul(10).wrapping_add(digit);
        if u32::try_from(capped_magnitude).is_ok() {
            capped_magnitude = capped_magnitude
                .saturating_mul(10)
                .saturating_add(u64::from(digit))
                .min(LEGACY_MAGNITUDE_CAP);
        }
        index += 1;
    }
    LegacyIntegerPrefix {
        negative,
        has_digits: index != digit_start,
        low_bits,
        magnitude_over_u32: capped_magnitude > u64::from(u32::MAX),
    }
}

fn legacy_strtoul32(value: &MobProtoQueryValue, column: usize) -> Result<u32, MobProtoRowError> {
    let bytes = legacy_c_string_bytes(value, column)?;
    let parsed = legacy_integer_prefix(bytes);
    if !parsed.has_digits {
        return Ok(0);
    }
    if parsed.magnitude_over_u32 {
        Ok(u32::MAX)
    } else if parsed.negative {
        Ok(parsed.low_bits.wrapping_neg())
    } else {
        Ok(parsed.low_bits)
    }
}

fn legacy_strtol32(value: &MobProtoQueryValue, column: usize) -> Result<i32, MobProtoRowError> {
    let bytes = legacy_c_string_bytes(value, column)?;
    let parsed = legacy_integer_prefix(bytes);
    if !parsed.has_digits {
        return Ok(0);
    }
    if parsed.negative {
        if parsed.magnitude_over_u32 || parsed.low_bits > (1_u32 << 31) {
            // For a negative value, `2^31` is the representable minimum and
            // larger magnitudes clamp to the same endpoint.
            Ok(i32::MIN)
        } else if parsed.low_bits == (1_u32 << 31) {
            Ok(i32::MIN)
        } else {
            let magnitude =
                i32::try_from(parsed.low_bits).map_err(|_| MobProtoRowError::Source {
                    column,
                    message: "x86 strtol magnitude exceeded i32".to_owned(),
                })?;
            Ok(-magnitude)
        }
    } else if parsed.magnitude_over_u32 || parsed.low_bits > i32::MAX.unsigned_abs() {
        Ok(i32::MAX)
    } else {
        i32::try_from(parsed.low_bits).map_err(|_| MobProtoRowError::Source {
            column,
            message: "x86 strtol magnitude exceeded i32".to_owned(),
        })
    }
}

fn legacy_u8(value: &MobProtoQueryValue, column: usize) -> Result<u8, MobProtoRowError> {
    let low_bits = legacy_strtoul32(value, column)? & 0xff;
    u8::try_from(low_bits).map_err(|_| MobProtoRowError::Source {
        column,
        message: "u8 narrowing failed".to_owned(),
    })
}

fn legacy_u16(value: &MobProtoQueryValue, column: usize) -> Result<u16, MobProtoRowError> {
    let low_bits = legacy_strtoul32(value, column)? & 0xffff;
    u16::try_from(low_bits).map_err(|_| MobProtoRowError::Source {
        column,
        message: "u16 narrowing failed".to_owned(),
    })
}

fn legacy_u32(value: &MobProtoQueryValue, column: usize) -> Result<u32, MobProtoRowError> {
    legacy_strtoul32(value, column)
}

fn legacy_i8(value: &MobProtoQueryValue, column: usize) -> Result<i8, MobProtoRowError> {
    let parsed = legacy_strtol32(value, column)?;
    // x86 `char` conversion retains the low eight bits of the 32-bit result.
    Ok(i8::from_le_bytes([parsed.to_le_bytes()[0]]))
}

fn legacy_i16(value: &MobProtoQueryValue, column: usize) -> Result<i16, MobProtoRowError> {
    let parsed = legacy_strtol32(value, column)?;
    let bytes = parsed.to_le_bytes();
    // x86 `short` conversion retains the low sixteen bits of `strtol`.
    Ok(i16::from_le_bytes([bytes[0], bytes[1]]))
}

fn decode_fixed_legacy<const N: usize>(
    value: &MobProtoQueryValue,
    column: usize,
) -> Result<[u8; N], MobProtoRowError> {
    let bytes = legacy_c_string_bytes(value, column)?;
    let length = bytes.len().min(N - 1);
    let mut output = [0_u8; N];
    output[..length].copy_from_slice(&bytes[..length]);
    Ok(output)
}

fn ascii_case_prefix(bytes: &[u8], prefix: &str) -> bool {
    prefix.len() <= bytes.len() && bytes[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

fn saturating_i32(bytes: &[u8]) -> i32 {
    let mut value = 0_i32;
    for byte in bytes {
        value = value
            .saturating_mul(10)
            .saturating_add(i32::from(byte - b'0'));
    }
    value
}

#[allow(clippy::cast_possible_truncation)] // C `strtof` rounds this `f64` intermediate to `f32`.
fn legacy_hex_float(bytes: &[u8], start: usize, negative: bool) -> Option<f32> {
    if !bytes
        .get(start..start + 2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"0x"))
    {
        return None;
    }
    let mut index = start + 2;
    let mut mantissa = 0.0_f64;
    let mut binary_exponent = 0_i32;
    let mut digits = 0_usize;
    let mut fractional = false;
    while index < bytes.len() {
        let byte = bytes[index];
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            b'.' if !fractional => {
                fractional = true;
                index += 1;
                continue;
            }
            _ => break,
        };
        mantissa = mantissa.mul_add(16.0, f64::from(digit));
        digits += 1;
        if fractional {
            binary_exponent = binary_exponent.saturating_sub(4);
        }
        index += 1;
    }
    if digits == 0 {
        return None;
    }
    if matches!(bytes.get(index), Some(b'p' | b'P')) {
        index += 1;
        let exponent_negative = match bytes.get(index) {
            Some(b'+') => {
                index += 1;
                false
            }
            Some(b'-') => {
                index += 1;
                true
            }
            _ => false,
        };
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index != exponent_start {
            let exponent = saturating_i32(&bytes[exponent_start..index]);
            binary_exponent = binary_exponent.saturating_add(if exponent_negative {
                exponent.saturating_neg()
            } else {
                exponent
            });
        }
    }
    let magnitude = if mantissa == 0.0 || binary_exponent < -4_096 {
        0.0
    } else if mantissa.is_infinite() || binary_exponent > 4_096 {
        f32::INFINITY
    } else {
        (mantissa * 2.0_f64.powi(binary_exponent)) as f32
    };
    Some(if negative { -magnitude } else { magnitude })
}

fn legacy_decimal_float_prefix(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start;
    let mut integer_digits = 0_usize;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        integer_digits += 1;
        index += 1;
    }
    let mut fraction_digits = 0_usize;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            fraction_digits += 1;
            index += 1;
        }
    }
    if integer_digits + fraction_digits == 0 {
        return None;
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        let exponent_marker = index;
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == exponent_start {
            index = exponent_marker;
        }
    }
    Some(index)
}

fn legacy_strtof_prefix(bytes: &[u8]) -> f32 {
    let mut start = 0;
    while start < bytes.len() && is_c_space(bytes[start]) {
        start += 1;
    }
    let negative = match bytes.get(start) {
        Some(b'-') => {
            start += 1;
            true
        }
        Some(b'+') => {
            start += 1;
            false
        }
        _ => false,
    };
    if ascii_case_prefix(&bytes[start..], "nan") {
        return if negative {
            f32::from_bits(0xffc0_0000)
        } else {
            f32::from_bits(0x7fc0_0000)
        };
    }
    if ascii_case_prefix(&bytes[start..], "inf") {
        return if negative {
            f32::NEG_INFINITY
        } else {
            f32::INFINITY
        };
    }
    if let Some(hex) = legacy_hex_float(bytes, start, negative) {
        return hex;
    }
    let Some(end) = legacy_decimal_float_prefix(bytes, start) else {
        return 0.0;
    };
    let Ok(text) = std::str::from_utf8(&bytes[start..end]) else {
        return 0.0;
    };
    let magnitude = text.parse::<f32>().unwrap_or(0.0);
    if negative {
        -magnitude
    } else {
        magnitude
    }
}

fn legacy_f32(value: &MobProtoQueryValue, column: usize) -> Result<f32, MobProtoRowError> {
    let bytes = legacy_c_string_bytes(value, column)?;
    Ok(legacy_strtof_prefix(bytes))
}

/// Decode all 70 source cells with the default strict policy.
///
/// The row must be exact-width. Every cell must be non-NULL and not a source
/// error. Unsigned integers use complete ASCII decimal at the destination
/// width. Signed `i8` and `i16` values may have one leading `-`. The float
/// accepts a complete finite ASCII decimal or scientific value only. Raw
/// names and folders permit non-UTF-8 bytes but reject interior NULs and
/// content beyond the source buffer. No enum, range, or uniqueness policy is
/// added beyond each destination type's valid width.
///
/// # Errors
///
/// Returns [`MobProtoRowError`] for a wrong-width row or invalid source cell.
pub fn decode_mob_proto_query_row(
    row: &MobProtoQueryRow,
) -> Result<MobTableRecord, MobProtoRowError> {
    check_row_width(row)?;
    Ok(MobTableRecord {
        vnum: decode_strict_u32(&row.columns[0], 0)?,
        name: decode_fixed_strict::<MOB_NAME_BYTES>(&row.columns[1], 1)?,
        locale_name: decode_fixed_strict::<MOB_LOCALE_NAME_BYTES>(&row.columns[2], 2)?,
        mob_type: decode_strict_u8(&row.columns[3], 3)?,
        rank: decode_strict_u8(&row.columns[4], 4)?,
        battle_type: decode_strict_u8(&row.columns[5], 5)?,
        level: decode_strict_u8(&row.columns[6], 6)?,
        size: decode_strict_u8(&row.columns[7], 7)?,
        gold_min: decode_strict_u32(&row.columns[26], 26)?,
        gold_max: decode_strict_u32(&row.columns[27], 27)?,
        exp: decode_strict_u32(&row.columns[25], 25)?,
        max_hp: decode_strict_u32(&row.columns[22], 22)?,
        regen_cycle: decode_strict_u8(&row.columns[23], 23)?,
        regen_percent: decode_strict_u8(&row.columns[24], 24)?,
        def: decode_strict_u16(&row.columns[28], 28)?,
        ai_flag: decode_strict_u32(&row.columns[8], 8)?,
        race_flag: decode_strict_u32(&row.columns[9], 9)?,
        immune_flag: decode_strict_u32(&row.columns[10], 10)?,
        str: decode_strict_u8(&row.columns[16], 16)?,
        dex: decode_strict_u8(&row.columns[17], 17)?,
        con: decode_strict_u8(&row.columns[18], 18)?,
        int_: decode_strict_u8(&row.columns[19], 19)?,
        damage_range: [
            decode_strict_u32(&row.columns[20], 20)?,
            decode_strict_u32(&row.columns[21], 21)?,
        ],
        attack_speed: decode_strict_i16(&row.columns[29], 29)?,
        moving_speed: decode_strict_i16(&row.columns[30], 30)?,
        aggressive_hp_pct: decode_strict_u8(&row.columns[31], 31)?,
        aggressive_sight: decode_strict_u16(&row.columns[32], 32)?,
        attack_range: decode_strict_u16(&row.columns[33], 33)?,
        enchants: [
            decode_strict_i8(&row.columns[35], 35)?,
            decode_strict_i8(&row.columns[36], 36)?,
            decode_strict_i8(&row.columns[37], 37)?,
            decode_strict_i8(&row.columns[38], 38)?,
            decode_strict_i8(&row.columns[39], 39)?,
            decode_strict_i8(&row.columns[40], 40)?,
        ],
        resists: [
            decode_strict_i8(&row.columns[41], 41)?,
            decode_strict_i8(&row.columns[42], 42)?,
            decode_strict_i8(&row.columns[43], 43)?,
            decode_strict_i8(&row.columns[44], 44)?,
            decode_strict_i8(&row.columns[45], 45)?,
            decode_strict_i8(&row.columns[46], 46)?,
            decode_strict_i8(&row.columns[47], 47)?,
            decode_strict_i8(&row.columns[48], 48)?,
            decode_strict_i8(&row.columns[49], 49)?,
            decode_strict_i8(&row.columns[50], 50)?,
            decode_strict_i8(&row.columns[51], 51)?,
        ],
        resurrection_vnum: decode_strict_u32(&row.columns[14], 14)?,
        drop_item_vnum: decode_strict_u32(&row.columns[13], 13)?,
        mount_capacity: 0,
        on_click_type: decode_strict_u8(&row.columns[11], 11)?,
        empire: decode_strict_u8(&row.columns[12], 12)?,
        folder: decode_fixed_strict::<MOB_FOLDER_BYTES>(&row.columns[15], 15)?,
        dam_multiply: decode_strict_f32(&row.columns[52], 52)?,
        summon_vnum: decode_strict_u32(&row.columns[53], 53)?,
        drain_sp: decode_strict_u32(&row.columns[54], 54)?,
        mob_color: 0,
        polymorph_item_vnum: decode_strict_u32(&row.columns[34], 34)?,
        skills: [
            MobSkillRecord {
                vnum: decode_strict_u32(&row.columns[55], 55)?,
                level: decode_strict_u8(&row.columns[56], 56)?,
            },
            MobSkillRecord {
                vnum: decode_strict_u32(&row.columns[57], 57)?,
                level: decode_strict_u8(&row.columns[58], 58)?,
            },
            MobSkillRecord {
                vnum: decode_strict_u32(&row.columns[59], 59)?,
                level: decode_strict_u8(&row.columns[60], 60)?,
            },
            MobSkillRecord {
                vnum: decode_strict_u32(&row.columns[61], 61)?,
                level: decode_strict_u8(&row.columns[62], 62)?,
            },
            MobSkillRecord {
                vnum: decode_strict_u32(&row.columns[63], 63)?,
                level: decode_strict_u8(&row.columns[64], 64)?,
            },
        ],
        berserk_point: decode_strict_u8(&row.columns[65], 65)?,
        stone_skin_point: decode_strict_u8(&row.columns[66], 66)?,
        god_speed_point: decode_strict_u8(&row.columns[67], 67)?,
        death_blow_point: decode_strict_u8(&row.columns[68], 68)?,
        revive_point: decode_strict_u8(&row.columns[69], 69)?,
    })
}

/// Alias emphasizing the physical table name.
///
/// # Errors
///
/// Returns [`MobProtoRowError`] for a wrong-width row or invalid source cell.
pub fn decode_mob_proto_table_row(
    row: &MobProtoQueryRow,
) -> Result<MobTableRecord, MobProtoRowError> {
    decode_mob_proto_query_row(row)
}

/// Decode all 70 cells with the explicitly named legacy C policy.
///
/// SQL `NULL`, empty, or first-byte-NUL cells produce the zero value already
/// stored in the destination. Names and folders use first-NUL,
/// at-most-24/64-byte `strlcpy`-style copying with zero fill. Integers consume
/// the C whitespace/optional-sign/decimal prefix, then model x86 32-bit
/// `strtoul` or `strtol` and the destination cast. Floats consume the C
/// `strtof` prefix. Decimal/scientific values, infinities, NaNs, and a
/// practical hexadecimal prefix are accepted. Hex conversion uses `f64`
/// intermediates; special values are canonicalized while preserving their
/// sign. Invalid or nonnumeric prefixes become `0.0`, as in the source call,
/// and trailing bytes are ignored. `Error` cells and wrong row width still
/// fail. No source-robustness defect beyond conversion compatibility is
/// reproduced.
///
/// # Errors
///
/// Returns [`MobProtoRowError`] for a wrong-width row or source-cell error.
pub fn decode_mob_proto_query_row_legacy(
    row: &MobProtoQueryRow,
) -> Result<MobTableRecord, MobProtoRowError> {
    check_row_width(row)?;
    Ok(MobTableRecord {
        vnum: legacy_u32(&row.columns[0], 0)?,
        name: decode_fixed_legacy::<MOB_NAME_BYTES>(&row.columns[1], 1)?,
        locale_name: decode_fixed_legacy::<MOB_LOCALE_NAME_BYTES>(&row.columns[2], 2)?,
        mob_type: legacy_u8(&row.columns[3], 3)?,
        rank: legacy_u8(&row.columns[4], 4)?,
        battle_type: legacy_u8(&row.columns[5], 5)?,
        level: legacy_u8(&row.columns[6], 6)?,
        size: legacy_u8(&row.columns[7], 7)?,
        gold_min: legacy_u32(&row.columns[26], 26)?,
        gold_max: legacy_u32(&row.columns[27], 27)?,
        exp: legacy_u32(&row.columns[25], 25)?,
        max_hp: legacy_u32(&row.columns[22], 22)?,
        regen_cycle: legacy_u8(&row.columns[23], 23)?,
        regen_percent: legacy_u8(&row.columns[24], 24)?,
        def: legacy_u16(&row.columns[28], 28)?,
        ai_flag: legacy_u32(&row.columns[8], 8)?,
        race_flag: legacy_u32(&row.columns[9], 9)?,
        immune_flag: legacy_u32(&row.columns[10], 10)?,
        str: legacy_u8(&row.columns[16], 16)?,
        dex: legacy_u8(&row.columns[17], 17)?,
        con: legacy_u8(&row.columns[18], 18)?,
        int_: legacy_u8(&row.columns[19], 19)?,
        damage_range: [
            legacy_u32(&row.columns[20], 20)?,
            legacy_u32(&row.columns[21], 21)?,
        ],
        attack_speed: legacy_i16(&row.columns[29], 29)?,
        moving_speed: legacy_i16(&row.columns[30], 30)?,
        aggressive_hp_pct: legacy_u8(&row.columns[31], 31)?,
        aggressive_sight: legacy_u16(&row.columns[32], 32)?,
        attack_range: legacy_u16(&row.columns[33], 33)?,
        enchants: [
            legacy_i8(&row.columns[35], 35)?,
            legacy_i8(&row.columns[36], 36)?,
            legacy_i8(&row.columns[37], 37)?,
            legacy_i8(&row.columns[38], 38)?,
            legacy_i8(&row.columns[39], 39)?,
            legacy_i8(&row.columns[40], 40)?,
        ],
        resists: [
            legacy_i8(&row.columns[41], 41)?,
            legacy_i8(&row.columns[42], 42)?,
            legacy_i8(&row.columns[43], 43)?,
            legacy_i8(&row.columns[44], 44)?,
            legacy_i8(&row.columns[45], 45)?,
            legacy_i8(&row.columns[46], 46)?,
            legacy_i8(&row.columns[47], 47)?,
            legacy_i8(&row.columns[48], 48)?,
            legacy_i8(&row.columns[49], 49)?,
            legacy_i8(&row.columns[50], 50)?,
            legacy_i8(&row.columns[51], 51)?,
        ],
        resurrection_vnum: legacy_u32(&row.columns[14], 14)?,
        drop_item_vnum: legacy_u32(&row.columns[13], 13)?,
        mount_capacity: 0,
        on_click_type: legacy_u8(&row.columns[11], 11)?,
        empire: legacy_u8(&row.columns[12], 12)?,
        folder: decode_fixed_legacy::<MOB_FOLDER_BYTES>(&row.columns[15], 15)?,
        dam_multiply: legacy_f32(&row.columns[52], 52)?,
        summon_vnum: legacy_u32(&row.columns[53], 53)?,
        drain_sp: legacy_u32(&row.columns[54], 54)?,
        mob_color: 0,
        polymorph_item_vnum: legacy_u32(&row.columns[34], 34)?,
        skills: [
            MobSkillRecord {
                vnum: legacy_u32(&row.columns[55], 55)?,
                level: legacy_u8(&row.columns[56], 56)?,
            },
            MobSkillRecord {
                vnum: legacy_u32(&row.columns[57], 57)?,
                level: legacy_u8(&row.columns[58], 58)?,
            },
            MobSkillRecord {
                vnum: legacy_u32(&row.columns[59], 59)?,
                level: legacy_u8(&row.columns[60], 60)?,
            },
            MobSkillRecord {
                vnum: legacy_u32(&row.columns[61], 61)?,
                level: legacy_u8(&row.columns[62], 62)?,
            },
            MobSkillRecord {
                vnum: legacy_u32(&row.columns[63], 63)?,
                level: legacy_u8(&row.columns[64], 64)?,
            },
        ],
        berserk_point: legacy_u8(&row.columns[65], 65)?,
        stone_skin_point: legacy_u8(&row.columns[66], 66)?,
        god_speed_point: legacy_u8(&row.columns[67], 67)?,
        death_blow_point: legacy_u8(&row.columns[68], 68)?,
        revive_point: legacy_u8(&row.columns[69], 69)?,
    })
}

/// Legacy alias emphasizing the physical table name.
///
/// # Errors
///
/// Returns [`MobProtoRowError`] for a wrong-width row or source-cell error.
pub fn decode_mob_proto_table_row_legacy(
    row: &MobProtoQueryRow,
) -> Result<MobTableRecord, MobProtoRowError> {
    decode_mob_proto_query_row_legacy(row)
}

fn validate_section_size(
    count: usize,
    limits: MobProtoSectionLimits,
) -> Result<(u16, u16, usize), MobProtoSectionError> {
    if count > limits.max_records {
        return Err(MobProtoSectionError::TooManyRecords {
            count,
            maximum: limits.max_records,
        });
    }
    let wire_count =
        u16::try_from(count).map_err(|_| MobProtoSectionError::CountOverflow { count })?;
    let record_size = u16::try_from(MOB_PROTO_TABLE_WIRE_SIZE).map_err(|_| {
        MobProtoSectionError::RecordSizeOverflow {
            size: MOB_PROTO_TABLE_WIRE_SIZE,
        }
    })?;
    let data_len = MOB_PROTO_TABLE_WIRE_SIZE
        .checked_mul(count)
        .ok_or(MobProtoSectionError::DataSizeOverflow { count })?;
    if data_len > limits.max_data_bytes {
        return Err(MobProtoSectionError::DataTooLarge {
            length: data_len,
            maximum: limits.max_data_bytes,
        });
    }
    Ok((wire_count, record_size, data_len))
}

fn append_rows(
    rows: &[MobProtoQueryRow],
    limits: MobProtoSectionLimits,
    legacy: bool,
) -> Result<BootSection, MobProtoSectionError> {
    let (count, record_size, data_len) = validate_section_size(rows.len(), limits)?;
    let mut data = Vec::new();
    data.try_reserve_exact(data_len)
        .map_err(|_| MobProtoSectionError::AllocationFailed {
            requested: data_len,
        })?;

    for (index, row) in rows.iter().enumerate() {
        let record = if legacy {
            decode_mob_proto_query_row_legacy(row)
        } else {
            decode_mob_proto_query_row(row)
        }
        .map_err(|source| MobProtoSectionError::Row { index, source })?;
        let encoded = record.encode();
        if encoded.len() != MOB_PROTO_TABLE_WIRE_SIZE {
            return Err(MobProtoSectionError::RecordSizeMismatch {
                index,
                expected: MOB_PROTO_TABLE_WIRE_SIZE,
                actual: encoded.len(),
            });
        }
        data.extend_from_slice(&encoded);
    }
    if data.len() != data_len {
        return Err(MobProtoSectionError::PackedDataLengthMismatch {
            expected: data_len,
            actual: data.len(),
        });
    }
    Ok(BootSection {
        kind: BootSectionKind::Mob,
        record_size,
        count,
        data,
    })
}

/// Build a strict `mob_proto` section with default limits.
///
/// Rows are encoded in exactly the supplied acquired order. Duplicates are
/// retained without another Rust sort; the checked SQL's `ORDER BY vnum`
/// establishes the vnum order. An empty input is represented by a validated
/// empty section at this pure boundary; injected and `SQLx` acquisition reject
/// an empty source.
///
/// # Errors
///
/// Returns [`MobProtoSectionError`] for invalid rows or configured limits.
pub fn build_mob_proto_section(
    rows: &[MobProtoQueryRow],
) -> Result<BootSection, MobProtoSectionError> {
    build_mob_proto_section_with_limits(rows, MobProtoSectionLimits::default())
}

/// Build a strict section with a caller-selected row cap.
///
/// # Errors
///
/// Returns [`MobProtoSectionError`] for invalid rows or configured limits.
pub fn build_mob_proto_section_with_limit(
    rows: &[MobProtoQueryRow],
    max_records: usize,
) -> Result<BootSection, MobProtoSectionError> {
    build_mob_proto_section_with_limits(rows, MobProtoSectionLimits::new(max_records))
}

/// Build a strict typed active `mob_proto` boot section.
///
/// # Errors
///
/// Returns [`MobProtoSectionError`] for invalid rows, allocation failures, or
/// configured limit violations.
pub fn build_mob_proto_section_with_limits(
    rows: &[MobProtoQueryRow],
    limits: MobProtoSectionLimits,
) -> Result<BootSection, MobProtoSectionError> {
    append_rows(rows, limits, false)
}

/// Build a section with the explicitly named legacy conversion policy.
///
/// # Errors
///
/// Returns [`MobProtoSectionError`] for source, allocation, or configured
/// limit failures.
pub fn build_mob_proto_section_legacy(
    rows: &[MobProtoQueryRow],
) -> Result<BootSection, MobProtoSectionError> {
    build_mob_proto_section_legacy_with_limits(rows, MobProtoSectionLimits::default())
}

/// Build a legacy-policy section with a caller-selected row cap.
///
/// # Errors
///
/// Returns [`MobProtoSectionError`] for source, allocation, or configured
/// limit failures.
pub fn build_mob_proto_section_legacy_with_limit(
    rows: &[MobProtoQueryRow],
    max_records: usize,
) -> Result<BootSection, MobProtoSectionError> {
    build_mob_proto_section_legacy_with_limits(rows, MobProtoSectionLimits::new(max_records))
}

/// Build a legacy-policy section with caller-selected record and byte caps.
///
/// # Errors
///
/// Returns [`MobProtoSectionError`] for source, allocation, or configured
/// limit failures.
pub fn build_mob_proto_section_legacy_with_limits(
    rows: &[MobProtoQueryRow],
    limits: MobProtoSectionLimits,
) -> Result<BootSection, MobProtoSectionError> {
    append_rows(rows, limits, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::postfix::TablePostfix;
    use protocol::db_boot::decode_mob_table_section;

    fn text(value: &str) -> MobProtoQueryValue {
        MobProtoQueryValue::text(value)
    }

    fn valid_row() -> MobProtoQueryRow {
        let mut columns = vec![text("0"); MOB_PROTO_QUERY_COLUMN_COUNT];
        let values = [
            "101",      // 0 vnum
            "Wolf",     // 1 name
            "Loup",     // 2 locale name
            "1",        // 3 type
            "2",        // 4 rank
            "3",        // 5 battle type
            "4",        // 6 level
            "5",        // 7 size
            "8",        // 8 AI flag
            "9",        // 9 race flag
            "10",       // 10 immune flag
            "11",       // 11 on click
            "12",       // 12 empire
            "13",       // 13 drop item
            "14",       // 14 resurrection vnum
            "mob/wolf", // 15 folder
            "16",       // 16 STR
            "17",       // 17 DEX
            "18",       // 18 CON
            "19",       // 19 INT
            "20",       // 20 minimum damage
            "21",       // 21 maximum damage
            "22",       // 22 max HP
            "23",       // 23 regen cycle
            "24",       // 24 regen percent
            "25",       // 25 exp
            "26",       // 26 minimum gold
            "27",       // 27 maximum gold
            "28",       // 28 defense
            "-29",      // 29 attack speed
            "-30",      // 30 moving speed
            "31",       // 31 aggressive HP percent
            "32",       // 32 aggressive sight
            "33",       // 33 attack range
            "34",       // 34 polymorph item
            "-1",       // 35..40 enchants are assigned below
            "-2", "-3", "-4", "-5", "-6", "1", // 41..51 resists are assigned below
            "2", "3", "4", "5", "6", "7", "8", "9", "10", "11",
            "1.25", // 52 damage multiplier
            "53",   // 53 summon vnum
            "54",   // 54 drain SP
            "101",  // 55 skill 0 vnum
            "1",    // 56 skill 0 level
            "102",  // 57 skill 1 vnum
            "2",    // 58 skill 1 level
            "103",  // 59 skill 2 vnum
            "3",    // 60 skill 2 level
            "104",  // 61 skill 3 vnum
            "4",    // 62 skill 3 level
            "105",  // 63 skill 4 vnum
            "5",    // 64 skill 4 level
            "65",   // 65 berserk
            "66",   // 66 stone skin
            "67",   // 67 god speed
            "68",   // 68 death blow
            "69",   // 69 revive
        ];
        assert_eq!(values.len(), MOB_PROTO_QUERY_COLUMN_COUNT);
        for (column, value) in values.into_iter().enumerate() {
            columns[column] = text(value);
        }
        MobProtoQueryRow::from_columns(columns)
    }

    fn replace(row: &mut MobProtoQueryRow, column: usize, value: MobProtoQueryValue) {
        let mut columns = row.clone().into_columns();
        columns[column] = value;
        *row = MobProtoQueryRow::new(columns);
    }

    fn assert_f32_bits(actual: f32, expected: f32) {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }

    #[test]
    fn try_new_stops_at_the_first_extra_cell() {
        use std::cell::Cell;

        let pulls = Cell::new(0_usize);
        let cells = (0..10_000)
            .inspect(|_| pulls.set(pulls.get() + 1))
            .map(|_| MobProtoQueryValue::bytes(b"0".to_vec()));
        assert_eq!(
            MobProtoQueryRow::try_new(cells),
            Err(MobProtoRowError::ColumnCount {
                expected: 70,
                actual: 71
            })
        );
        assert_eq!(pulls.get(), 71);
        assert!(MobProtoQueryRow::try_new(vec![text("0"); 70]).is_ok());
    }

    #[test]
    fn exact_query_metadata_and_lengths_are_source_fixed() {
        assert_eq!(MOB_PROTO_QUERY_COLUMN_COUNT, 70);
        assert_eq!(MOB_PROTO_TABLE_QUERY_COLUMNS, 70);
        assert_eq!(MOB_PROTO_QUERY_COLUMN_NAMES.len(), 70);
        assert_eq!(MOB_PROTO_TABLE_WIRE_SIZE, 255);
        assert_eq!(MOB_PROTO_SECTION_RECORD_SIZE, 255);
        assert_eq!(MOB_PROTO_TABLE_MAX_RECORDS, 65_535);
        assert_eq!(MOB_PROTO_TABLE_MAX_SECTION_BYTES, 16_711_425);
        assert_eq!(MOB_PROTO_QUERY_COLUMN_NAMES[7], "size+0");
        assert_eq!(MOB_PROTO_QUERY_COLUMN_NAMES[8], "ai_flag+0");
        assert_eq!(MOB_PROTO_QUERY_COLUMN_NAMES[9], "setRaceFlag+0");
        assert_eq!(MOB_PROTO_QUERY_COLUMN_NAMES[10], "setImmuneFlag+0");

        let base = MOB_PROTO_QUERY_TEMPLATE
            .replace("{locale}", "")
            .replace("{postfix}", "");
        assert_eq!(base.len(), 865);
        let default = MobProtoQuery::from_config(None, None).unwrap();
        assert_eq!(default.table_name(), "mob_proto");
        assert_eq!(default.locale_column().as_str(), "name");
        assert_eq!(default.as_str().len(), 869);
        assert_eq!(
            default.as_str(),
            MOB_PROTO_QUERY_TEMPLATE
                .replace("{locale}", "name")
                .replace("{postfix}", "")
        );

        let custom = MobProtoQuery::from_config(Some("_eu2"), Some("gb2312name")).unwrap();
        assert_eq!(custom.table_name(), "mob_proto_eu2");
        assert!(custom
            .as_str()
            .contains("SELECT vnum, name, gb2312name, type,"));
        assert!(custom
            .as_str()
            .ends_with(" FROM mob_proto_eu2 ORDER BY vnum;"));

        let long_locale = "a".repeat(MAX_MOB_PROTO_LOCALE_COLUMN_BYTES);
        let long_postfix = "p".repeat(crate::postfix::MAX_TABLE_POSTFIX_BYTES);
        let maximum = MobProtoQuery::from_config(Some(&long_postfix), Some(&long_locale)).unwrap();
        assert_eq!(maximum.as_str().len(), 1_375);
        assert!(maximum.as_str().len() <= MAX_MOB_PROTO_QUERY_BYTES);
    }

    #[test]
    fn locale_and_postfix_are_validated_before_interpolation() {
        assert_eq!(MobProtoLocaleColumn::default().as_str(), "name");
        for invalid in ["", "1name", "na-me", "na me", "na.me", "na;me", "é"] {
            assert!(MobProtoLocaleColumn::parse(invalid).is_err(), "{invalid:?}");
        }
        assert!(MobProtoLocaleColumn::parse("_gb2312_2").is_ok());
        let longest = "a".repeat(MAX_MOB_PROTO_LOCALE_COLUMN_BYTES);
        assert!(MobProtoLocaleColumn::parse(&longest).is_ok());
        assert!(MobProtoLocaleColumn::parse(&format!("{longest}b")).is_err());
        assert!(matches!(
            MobProtoQuery::from_config(None, Some("bad-name")),
            Err(MobProtoBoundaryError::LocaleColumn(_))
        ));
        assert!(matches!(
            MobProtoQuery::from_config(Some("bad-name"), None),
            Err(MobProtoBoundaryError::Postfix(_))
        ));
    }

    #[test]
    fn strict_row_maps_all_cells_and_explicit_omitted_zeroes() {
        let record = decode_mob_proto_query_row(&valid_row()).unwrap();
        assert_eq!(record.vnum, 101);
        assert_eq!(&record.name[..4], b"Wolf");
        assert_eq!(&record.locale_name[..4], b"Loup");
        assert_eq!(record.mob_type, 1);
        assert_eq!(record.rank, 2);
        assert_eq!(record.battle_type, 3);
        assert_eq!(record.level, 4);
        assert_eq!(record.size, 5);
        assert_eq!(record.gold_min, 26);
        assert_eq!(record.gold_max, 27);
        assert_eq!(record.exp, 25);
        assert_eq!(record.max_hp, 22);
        assert_eq!(record.regen_cycle, 23);
        assert_eq!(record.regen_percent, 24);
        assert_eq!(record.def, 28);
        assert_eq!(record.ai_flag, 8);
        assert_eq!(record.race_flag, 9);
        assert_eq!(record.immune_flag, 10);
        assert_eq!(record.str, 16);
        assert_eq!(record.dex, 17);
        assert_eq!(record.con, 18);
        assert_eq!(record.int_, 19);
        assert_eq!(record.damage_range, [20, 21]);
        assert_eq!(record.attack_speed, -29);
        assert_eq!(record.moving_speed, -30);
        assert_eq!(record.aggressive_hp_pct, 31);
        assert_eq!(record.aggressive_sight, 32);
        assert_eq!(record.attack_range, 33);
        assert_eq!(record.enchants, [-1, -2, -3, -4, -5, -6]);
        assert_eq!(record.resists, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
        assert_eq!(record.resurrection_vnum, 14);
        assert_eq!(record.drop_item_vnum, 13);
        assert_eq!(record.mount_capacity, 0);
        assert_eq!(record.on_click_type, 11);
        assert_eq!(record.empire, 12);
        assert_eq!(&record.folder[..8], b"mob/wolf");
        assert_f32_bits(record.dam_multiply, 1.25);
        assert_eq!(record.summon_vnum, 53);
        assert_eq!(record.drain_sp, 54);
        assert_eq!(record.mob_color, 0);
        assert_eq!(record.polymorph_item_vnum, 34);
        assert_eq!(record.skills[0].vnum, 101);
        assert_eq!(record.skills[0].level, 1);
        assert_eq!(record.skills[1].vnum, 102);
        assert_eq!(record.skills[1].level, 2);
        assert_eq!(record.skills[2].vnum, 103);
        assert_eq!(record.skills[2].level, 3);
        assert_eq!(record.skills[3].vnum, 104);
        assert_eq!(record.skills[3].level, 4);
        assert_eq!(record.skills[4].vnum, 105);
        assert_eq!(record.skills[4].level, 5);
        assert_eq!(record.berserk_point, 65);
        assert_eq!(record.stone_skin_point, 66);
        assert_eq!(record.god_speed_point, 67);
        assert_eq!(record.death_blow_point, 68);
        assert_eq!(record.revive_point, 69);
        assert_eq!(record.encode().len(), MOB_PROTO_TABLE_WIRE_SIZE);
    }

    #[test]
    fn strict_policy_rejects_width_null_source_and_empty_cells() {
        assert_eq!(
            decode_mob_proto_query_row(&MobProtoQueryRow::new(Vec::new())),
            Err(MobProtoRowError::ColumnCount {
                expected: 70,
                actual: 0
            })
        );
        let mut row = valid_row();
        replace(&mut row, 5, MobProtoQueryValue::Null);
        assert_eq!(
            decode_mob_proto_query_row(&row),
            Err(MobProtoRowError::Null { column: 5 })
        );
        replace(&mut row, 5, MobProtoQueryValue::error("driver failed"));
        assert_eq!(
            decode_mob_proto_query_row(&row),
            Err(MobProtoRowError::Source {
                column: 5,
                message: "driver failed".to_owned()
            })
        );
        for invalid in ["", " 1", "1 ", "1x", "+1", "-1"] {
            let mut row = valid_row();
            replace(&mut row, 0, text(invalid));
            assert!(matches!(
                decode_mob_proto_query_row(&row),
                Err(MobProtoRowError::InvalidNumber { column: 0, .. })
            ));
        }
    }

    #[test]
    fn strict_policy_enforces_each_destination_integer_width() {
        for (column, value) in [
            (0, "4294967296"),
            (3, "256"),
            (8, "4294967296"),
            (28, "65536"),
            (29, "32768"),
            (29, "-32769"),
            (35, "128"),
            (35, "-129"),
        ] {
            let mut row = valid_row();
            replace(&mut row, column, text(value));
            assert!(matches!(
                decode_mob_proto_query_row(&row),
                Err(MobProtoRowError::NumberOverflow {
                    column: actual,
                    ..
                }) if actual == column
            ));
        }
    }

    #[test]
    fn strict_fixed_strings_allow_raw_bytes_only_at_the_source_bounds() {
        let mut row = valid_row();
        replace(
            &mut row,
            1,
            MobProtoQueryValue::bytes(vec![0xff, 0x80, b'X']),
        );
        let record = decode_mob_proto_query_row(&row).unwrap();
        assert_eq!(&record.name[..3], &[0xff, 0x80, b'X']);
        assert!(record.name[3..].iter().all(|byte| *byte == 0));

        for (column, length) in [(1, MOB_NAME_BYTES), (15, MOB_FOLDER_BYTES)] {
            let mut row = valid_row();
            let mut value = vec![b'A'; length];
            value[length - 1] = 0;
            replace(&mut row, column, MobProtoQueryValue::bytes(value));
            assert!(decode_mob_proto_query_row(&row).is_ok());

            let mut row = valid_row();
            let value = vec![b'A'; length + 1];
            replace(&mut row, column, MobProtoQueryValue::bytes(value));
            assert!(matches!(
                decode_mob_proto_query_row(&row),
                Err(MobProtoRowError::NameTooLong {
                    column: actual,
                    length: actual_length,
                    ..
                }) if actual == column && actual_length == length + 1
            ));

            let mut row = valid_row();
            replace(
                &mut row,
                column,
                MobProtoQueryValue::bytes(vec![b'A', 0, b'B']),
            );
            assert!(matches!(
                decode_mob_proto_query_row(&row),
                Err(MobProtoRowError::NameInteriorNul {
                    column: actual,
                    index: 1
                }) if actual == column
            ));

            let mut row = valid_row();
            replace(&mut row, column, MobProtoQueryValue::bytes(vec![b'A', 0]));
            assert!(matches!(
                decode_mob_proto_query_row(&row),
                Err(MobProtoRowError::NameInteriorNul {
                    column: actual,
                    index: 1
                }) if actual == column
            ));
        }
    }

    #[test]
    fn strict_float_accepts_only_complete_finite_ascii_decimal_or_scientific() {
        for (source, expected) in [
            ("0", 0.0_f32),
            ("-0", -0.0_f32),
            ("+2.5", 2.5),
            (".5", 0.5),
            ("1.", 1.0),
            ("1e2", 100.0),
            ("-2E-2", -0.02),
        ] {
            let mut row = valid_row();
            replace(&mut row, 52, text(source));
            let record = decode_mob_proto_query_row(&row).unwrap();
            assert_eq!(
                record.dam_multiply.to_bits(),
                expected.to_bits(),
                "{source}"
            );
        }
        for invalid in [
            "",
            " 1",
            "1 ",
            "1x",
            "inf",
            "-Infinity",
            "NaN",
            "0x1p0",
            "1e",
            "1e+",
            ".",
        ] {
            let mut row = valid_row();
            replace(&mut row, 52, text(invalid));
            assert!(matches!(
                decode_mob_proto_query_row(&row),
                Err(MobProtoRowError::InvalidNumber { column: 52, .. })
            ));
        }
        for overflow in ["1e100", "-1e100"] {
            let mut row = valid_row();
            replace(&mut row, 52, text(overflow));
            assert!(matches!(
                decode_mob_proto_query_row(&row),
                Err(MobProtoRowError::NumberOverflow { column: 52, .. })
            ));
        }
    }

    #[test]
    fn legacy_null_empty_and_first_nul_cells_leave_zero_record() {
        let all_null = MobProtoQueryRow::new(vec![MobProtoQueryValue::Null; 70]);
        assert_eq!(
            decode_mob_proto_query_row_legacy(&all_null).unwrap(),
            MobTableRecord::default()
        );

        let mut row = valid_row();
        for column in 0..70 {
            replace(&mut row, column, text(""));
        }
        assert_eq!(
            decode_mob_proto_query_row_legacy(&row).unwrap(),
            MobTableRecord::default()
        );

        for column in [0, 1, 15, 29, 35, 52, 69] {
            let mut row = MobProtoQueryRow::new(vec![MobProtoQueryValue::Null; 70]);
            replace(
                &mut row,
                column,
                MobProtoQueryValue::bytes(vec![0, b'9', b'9']),
            );
            assert_eq!(
                decode_mob_proto_query_row_legacy(&row).unwrap(),
                MobTableRecord::default()
            );
        }
    }

    #[test]
    fn legacy_policy_models_x86_integer_prefixes_and_destination_casts() {
        let mut row = valid_row();
        let cases = [
            (0, " -1rest"),
            (3, "257rest"),
            (8, "4294967296rest"),
            (13, "8589934591"),
            (29, "-129rest"),
            (30, "-65537rest"),
            (35, "255rest"),
            (36, "-129rest"),
            (41, "257rest"),
            (53, "-4294967296rest"),
        ];
        for (column, source) in cases {
            replace(&mut row, column, text(source));
        }
        replace(&mut row, 52, text("  -1.25e1rest"));
        let record = decode_mob_proto_query_row_legacy(&row).unwrap();
        assert_eq!(record.vnum, u32::MAX);
        assert_eq!(record.mob_type, 1);
        assert_eq!(record.ai_flag, u32::MAX);
        assert_eq!(record.drop_item_vnum, u32::MAX);
        assert_eq!(record.attack_speed, -129);
        assert_eq!(record.moving_speed, -1);
        assert_eq!(record.enchants[0], -1);
        assert_eq!(record.enchants[1], 127);
        assert_eq!(record.resists[0], 1);
        assert_eq!(record.summon_vnum, u32::MAX);
        assert_f32_bits(record.dam_multiply, -12.5);
        assert_eq!(record.mount_capacity, 0);
        assert_eq!(record.mob_color, 0);
    }

    #[test]
    fn legacy_float_covers_specials_hex_prefix_and_invalid_prefixes() {
        let mut row = valid_row();
        replace(
            &mut row,
            52,
            MobProtoQueryValue::bytes(b" 0x1.8p1rest\0ignored".to_vec()),
        );
        assert_f32_bits(
            decode_mob_proto_query_row_legacy(&row)
                .unwrap()
                .dam_multiply,
            3.0,
        );
        replace(&mut row, 52, text("0x1.8p0"));
        assert_f32_bits(
            decode_mob_proto_query_row_legacy(&row)
                .unwrap()
                .dam_multiply,
            1.5,
        );
        replace(&mut row, 52, text("-1.25e-2rest"));
        assert_f32_bits(
            decode_mob_proto_query_row_legacy(&row)
                .unwrap()
                .dam_multiply,
            -0.0125,
        );
        replace(&mut row, 52, text("-0x1.8p1rest"));
        assert_f32_bits(
            decode_mob_proto_query_row_legacy(&row)
                .unwrap()
                .dam_multiply,
            -3.0,
        );
        replace(&mut row, 52, text("-0.0rest"));
        let negative_zero = decode_mob_proto_query_row_legacy(&row)
            .unwrap()
            .dam_multiply;
        assert_eq!(negative_zero.to_bits(), (-0.0_f32).to_bits());
        replace(&mut row, 52, text("12.5rest"));
        assert_f32_bits(
            decode_mob_proto_query_row_legacy(&row)
                .unwrap()
                .dam_multiply,
            12.5,
        );
        replace(&mut row, 52, text("bad"));
        assert_f32_bits(
            decode_mob_proto_query_row_legacy(&row)
                .unwrap()
                .dam_multiply,
            0.0,
        );
        replace(&mut row, 52, text("1e100"));
        assert_f32_bits(
            decode_mob_proto_query_row_legacy(&row)
                .unwrap()
                .dam_multiply,
            f32::INFINITY,
        );
        replace(&mut row, 52, text("-INFINITYrest"));
        assert_f32_bits(
            decode_mob_proto_query_row_legacy(&row)
                .unwrap()
                .dam_multiply,
            f32::NEG_INFINITY,
        );
        replace(&mut row, 52, text("-NaN(payload)rest"));
        let nan = decode_mob_proto_query_row_legacy(&row)
            .unwrap()
            .dam_multiply;
        assert!(nan.is_nan() && nan.is_sign_negative());
    }

    #[test]
    fn legacy_fixed_strings_use_first_nul_at_most_content_and_zero_fill() {
        let mut row = valid_row();
        replace(
            &mut row,
            1,
            MobProtoQueryValue::bytes(vec![0xff, b'A', b'B', 0, b'X']),
        );
        replace(&mut row, 15, MobProtoQueryValue::bytes(vec![b'F'; 80]));
        let record = decode_mob_proto_query_row_legacy(&row).unwrap();
        assert_eq!(&record.name[..3], &[0xff, b'A', b'B']);
        assert!(record.name[3..].iter().all(|byte| *byte == 0));
        assert_eq!(&record.folder[..64], &[b'F'; 64]);
        assert_eq!(record.folder[64], 0);
    }

    #[test]
    fn legacy_policy_still_rejects_source_errors_and_wrong_width() {
        let mut row = valid_row();
        replace(&mut row, 52, MobProtoQueryValue::error("source failed"));
        assert!(matches!(
            decode_mob_proto_query_row_legacy(&row),
            Err(MobProtoRowError::Source { column: 52, .. })
        ));
        assert_eq!(
            decode_mob_proto_query_row_legacy(&MobProtoQueryRow::new(vec![text("0"); 69])),
            Err(MobProtoRowError::ColumnCount {
                expected: 70,
                actual: 69
            })
        );
    }

    #[test]
    fn section_builder_preserves_acquired_order_duplicates_and_exact_metadata() {
        let mut last = valid_row();
        replace(&mut last, 0, text("20"));
        replace(&mut last, 1, text("last"));
        let mut first = valid_row();
        replace(&mut first, 0, text("10"));
        replace(&mut first, 1, text("first-a"));
        let mut duplicate = valid_row();
        replace(&mut duplicate, 0, text("10"));
        replace(&mut duplicate, 1, text("first-b"));

        let section =
            build_mob_proto_section_with_limit(&[last.clone(), first.clone(), duplicate], 3)
                .unwrap();
        assert_eq!(section.kind, BootSectionKind::Mob);
        assert_eq!(section.record_size, 255);
        assert_eq!(section.count, 3);
        assert_eq!(section.data.len(), 3 * 255);
        let records = decode_mob_table_section(&section).unwrap();
        assert_eq!(
            records.iter().map(|record| record.vnum).collect::<Vec<_>>(),
            [20, 10, 10]
        );
        assert_eq!(&records[1].name[..7], b"first-a");
        assert_eq!(&records[2].name[..7], b"first-b");
    }

    #[test]
    fn section_limits_and_empty_pure_section_are_explicit() {
        let empty = build_mob_proto_section(&[]).unwrap();
        assert_eq!(empty.kind, BootSectionKind::Mob);
        assert_eq!(empty.record_size, 255);
        assert_eq!(empty.count, 0);
        assert!(empty.data.is_empty());

        let rows = [valid_row(), valid_row()];
        assert!(matches!(
            build_mob_proto_section_with_limit(&rows, 1),
            Err(MobProtoSectionError::TooManyRecords {
                count: 2,
                maximum: 1
            })
        ));
        assert!(matches!(
            build_mob_proto_section_with_limits(
                &rows,
                MobProtoSectionLimits::with_data_limit(2, 509)
            ),
            Err(MobProtoSectionError::DataTooLarge {
                length: 510,
                maximum: 509
            })
        ));
    }

    #[test]
    fn injected_loader_distinguishes_empty_source_from_database_style_failure() {
        let postfix = TablePostfix::default();
        let loader =
            MobProtoLoader::new(&postfix, MobProtoSectionLimits::with_data_limit(7, 1_000))
                .unwrap();
        let empty = |_: &MobProtoQuery| Ok::<_, &'static str>(Vec::new());
        assert!(matches!(
            loader.load_section(&empty),
            Err(MobProtoLoadError::EmptyResult)
        ));
        assert!(matches!(
            loader.load_section_legacy(&empty),
            Err(MobProtoLoadError::EmptyResult)
        ));

        let failed = |_: &MobProtoQuery| Err::<Vec<MobProtoQueryRow>, _>("database unavailable");
        assert!(matches!(
            loader.load_section(&failed),
            Err(MobProtoLoadError::Source("database unavailable"))
        ));
    }

    #[test]
    fn diagnostics_name_exact_expressions() {
        assert_eq!(mob_proto_query_column_name(2), "locale_name");
        assert_eq!(mob_proto_query_column_name(9), "setRaceFlag+0");
        assert_eq!(mob_proto_query_column_name(69), "sp_revive");
        let error = MobProtoRowError::Null { column: 10 };
        assert!(error.to_string().contains("setImmuneFlag+0"));
        assert_eq!(mob_proto_query_column_name(70), "unknown");
    }
}
