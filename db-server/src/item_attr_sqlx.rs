//! `SQLx` acquisition for the source-fixed `item_attr` and `item_attr_rare`
//! boot tables.
//!
//! Query construction and `TABLE_POSTFIX` validation live in
//! [`crate::postfix`]. This module only executes those checked, immutable
//! statements through [`ConnectionPool::query_up_to`], bounds acquisition before
//! it accumulates rows, checks the exact normal/rare result width, and hands
//! lossless cells to the pure [`crate::item_attr`] policy. It does not choose a
//! boot profile, replace a table, sort or deduplicate rows, or turn a failure
//! into an empty section.
//!
//! The pool uses the no-bind `sqlx::raw_sql` text-protocol path. The unchecked
//! positional read below bypasses only `SQLx`'s numeric type-metadata check. Values are read as
//! `Option<Vec<u8>>`, so the adapter never applies UTF-8 conversion or
//! normalization. SQL `NULL`, raw bytes (including an interior NUL), source
//! order, and duplicate rows remain available to the selected pure policy.

use std::error::Error;
use std::fmt;

use db::sqlx::Row;
use db::{ConnectionPool, DbError};
use protocol::db_boot::BootSection;

use crate::item_attr::{
    build_item_attr_section_legacy_with_limits, build_item_attr_section_with_limits,
    ItemAttrQueryValue, ItemAttrSectionError, ItemAttrSectionLimits, ItemAttrTableKind,
    ItemAttrTableQueryRow, ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS, ITEM_ATTR_TABLE_MAX_RECORDS,
    ITEM_ATTR_TABLE_QUERY_COLUMNS,
};
use crate::postfix::{ItemAttrLoader, ItemAttrQuery, ItemRareLoader, ItemRareQuery};

/// A failure while acquiring or bounding a source-fixed item-attribute table.
///
/// The normal and rare queries share this error type because their acquisition
/// and pure-section boundaries are identical; the query is selected by the
/// loader or [`ItemAttrSqlxQuery`] passed to the function.
#[derive(Debug)]
pub enum ItemAttrSqlxLoadError {
    /// The pool could not execute the checked query after its retry policy.
    Database(DbError),
    /// The configured source-row cap cannot fit the legacy `u16` count.
    InvalidSourceLimit {
        /// Configured maximum number of source rows.
        maximum: usize,
        /// Maximum representable source-row cap.
        limit: usize,
    },
    /// The bounded stream observed a row beyond the configured cap.
    SourceLimitExceeded {
        /// Configured maximum number of source rows.
        maximum: usize,
    },
    /// The source returned zero rows. The legacy item-attribute loaders reject
    /// an empty table, so this is not represented as a fabricated section.
    EmptyResult,
    /// A returned row did not have the fixed kind-specific column shape.
    RowShape {
        /// Zero-based row index in the bounded result.
        row: usize,
        /// Required column count.
        expected: usize,
        /// Actual column count.
        actual: usize,
    },
    /// A source cell could not be decoded as optional raw bytes.
    ColumnDecode {
        /// Zero-based row index in the bounded result.
        row: usize,
        /// Zero-based source column index.
        column: usize,
        /// Underlying `SQLx` decode error.
        source: db::sqlx::Error,
    },
    /// A source-row vector or a per-row cell vector could not reserve its
    /// bounded result.
    AllocationFailed {
        /// Number of elements requested by the failed reservation.
        requested: usize,
    },
    /// The selected pure row/section policy rejected the acquired rows.
    Section(ItemAttrSectionError),
}

impl fmt::Display for ItemAttrSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => {
                write!(formatter, "item-attribute database query failed: {source}")
            }
            Self::InvalidSourceLimit { maximum, limit } => write!(
                formatter,
                "item-attribute source-row limit {maximum} exceeds the maximum {limit}"
            ),
            Self::SourceLimitExceeded { maximum } => {
                write!(
                    formatter,
                    "item-attribute source returned more than {maximum} rows"
                )
            }
            Self::EmptyResult => {
                write!(formatter, "item-attribute source returned no rows")
            }
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "item-attribute row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                row,
                column,
                source,
            } => write!(
                formatter,
                "item-attribute row {row} column {column} decode failed: {source}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "item-attribute adapter could not allocate {requested} bounded element(s)"
            ),
            Self::Section(source) => {
                write!(
                    formatter,
                    "item-attribute section construction failed: {source}"
                )
            }
        }
    }
}

impl Error for ItemAttrSqlxLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(source) => Some(source),
            Self::ColumnDecode { source, .. } => Some(source),
            Self::Section(source) => Some(source),
            Self::InvalidSourceLimit { .. }
            | Self::SourceLimitExceeded { .. }
            | Self::EmptyResult
            | Self::RowShape { .. }
            | Self::AllocationFailed { .. } => None,
        }
    }
}

impl From<ItemAttrSectionError> for ItemAttrSqlxLoadError {
    fn from(source: ItemAttrSectionError) -> Self {
        Self::Section(source)
    }
}

/// Compatibility spelling for callers that omit the `Sqlx` infix in the error
/// type.
pub type ItemAttrSqlxError = ItemAttrSqlxLoadError;

/// Compatibility spelling for callers that use the rare table in the error
/// type name. Both tables intentionally share one acquisition error enum.
pub type ItemRareSqlxLoadError = ItemAttrSqlxLoadError;

/// A checked query selected for a generic normal/rare acquisition.
///
/// The variants contain only the immutable query types built by
/// [`crate::postfix`]. There is no public variant carrying arbitrary SQL text.
#[derive(Debug, Clone, Copy)]
pub enum ItemAttrSqlxQuery<'query> {
    /// The checked normal `item_attr` query.
    Normal(&'query ItemAttrQuery),
    /// The checked rare `item_attr_rare` query.
    Rare(&'query ItemRareQuery),
}

impl<'query> ItemAttrSqlxQuery<'query> {
    /// Return the source-fixed table kind selected by this query.
    #[must_use]
    pub const fn kind(self) -> ItemAttrTableKind {
        match self {
            Self::Normal(_) => ItemAttrTableKind::Normal,
            Self::Rare(_) => ItemAttrTableKind::Rare,
        }
    }

    /// Borrow the immutable checked statement.
    #[must_use]
    pub fn as_str(self) -> &'query str {
        match self {
            Self::Normal(query) => query.as_str(),
            Self::Rare(query) => query.as_str(),
        }
    }

    /// Return the source-fixed table name selected by this query.
    #[must_use]
    pub fn table_name(self) -> &'query str {
        match self {
            Self::Normal(query) => query.table_name(),
            Self::Rare(query) => query.table_name(),
        }
    }
}

/// Compatibility spelling for a checked generic query.
pub type CheckedItemAttrQuery<'query> = ItemAttrSqlxQuery<'query>;

/// Acquire raw rows for a checked normal query without decoding or projecting
/// them.
///
/// The returned vector preserves the pool's order, duplicates, SQL `NULL`
/// values, and raw byte cells. The query and limits are read from the validated
/// loader; callers cannot supply a new statement or alter the source cap.
///
/// # Errors
///
/// Returns [`ItemAttrSqlxLoadError`] for an invalid source cap, a database
/// failure, an extra row, an empty source, a wrong-width row, a raw-cell
/// decode failure, or a failed fallible reservation.
pub async fn load_item_attr_table_rows_sqlx(
    pool: &ConnectionPool,
    loader: &ItemAttrLoader,
) -> Result<Vec<ItemAttrTableQueryRow>, ItemAttrSqlxLoadError> {
    acquire_rows(
        pool,
        ItemAttrTableKind::Normal,
        loader.query().as_str(),
        loader.limits(),
    )
    .await
}

/// Acquire raw rows for a checked rare query without decoding or projecting
/// them.
///
/// The returned vector preserves source order, duplicates, SQL `NULL`, and raw
/// bytes until the selected pure policy runs.
///
/// # Errors
///
/// Returns the same typed acquisition errors as
/// [`load_item_attr_table_rows_sqlx`].
pub async fn load_item_rare_table_rows_sqlx(
    pool: &ConnectionPool,
    loader: &ItemRareLoader,
) -> Result<Vec<ItemAttrTableQueryRow>, ItemAttrSqlxLoadError> {
    acquire_rows(
        pool,
        ItemAttrTableKind::Rare,
        loader.query().as_str(),
        loader.limits(),
    )
    .await
}

/// Compatibility spelling for a normal row acquisition without the table
/// suffix.
///
/// # Errors
///
/// Returns the same typed acquisition errors as
/// [`load_item_attr_table_rows_sqlx`].
pub async fn load_item_attr_rows_sqlx(
    pool: &ConnectionPool,
    loader: &ItemAttrLoader,
) -> Result<Vec<ItemAttrTableQueryRow>, ItemAttrSqlxLoadError> {
    load_item_attr_table_rows_sqlx(pool, loader).await
}

/// Compatibility spelling for a rare row acquisition without the table
/// suffix.
///
/// # Errors
///
/// Returns the same typed acquisition errors as
/// [`load_item_rare_table_rows_sqlx`].
pub async fn load_item_rare_rows_sqlx(
    pool: &ConnectionPool,
    loader: &ItemRareLoader,
) -> Result<Vec<ItemAttrTableQueryRow>, ItemAttrSqlxLoadError> {
    load_item_rare_table_rows_sqlx(pool, loader).await
}

/// Acquire and strictly build the source-fixed normal item-attribute section.
///
/// The legacy loader rejects zero rows, so an empty source is an explicit
/// [`ItemAttrSqlxLoadError::EmptyResult`] rather than an empty section. Raw
/// rows are handed to [`crate::item_attr::build_item_attr_section_with_limits`]
/// without sorting, deduplication, ID filtering, or map projection.
///
/// # Errors
///
/// Returns [`ItemAttrSqlxLoadError`] for any acquisition failure, an empty
/// source, a pure row/section failure, or a configured limit violation.
pub async fn load_item_attr_table_section_sqlx(
    pool: &ConnectionPool,
    loader: &ItemAttrLoader,
) -> Result<BootSection, ItemAttrSqlxLoadError> {
    let limits = loader.limits();
    let rows = load_item_attr_table_rows_sqlx(pool, loader).await?;
    build_item_attr_section_with_limits(ItemAttrTableKind::Normal, &rows, limits)
        .map_err(ItemAttrSqlxLoadError::Section)
}

/// Compatibility spelling for the normal section loader.
///
/// # Errors
///
/// Returns the same typed errors as
/// [`load_item_attr_table_section_sqlx`].
pub async fn load_item_attr_section_sqlx(
    pool: &ConnectionPool,
    loader: &ItemAttrLoader,
) -> Result<BootSection, ItemAttrSqlxLoadError> {
    load_item_attr_table_section_sqlx(pool, loader).await
}

/// Compatibility spelling without the `SQLx` suffix.
///
/// # Errors
///
/// Returns the same typed errors as
/// [`load_item_attr_table_section_sqlx`].
pub async fn load_item_attr_section(
    pool: &ConnectionPool,
    loader: &ItemAttrLoader,
) -> Result<BootSection, ItemAttrSqlxLoadError> {
    load_item_attr_table_section_sqlx(pool, loader).await
}

/// Acquire and strictly build the source-fixed rare item-attribute section.
///
/// The rare query has sixteen columns; the pure builder supplies the two
/// missing set-limit bytes as zero. No row is filtered or reordered before that
/// builder runs.
///
/// # Errors
///
/// Returns the same typed acquisition and section errors as
/// [`load_item_attr_table_section_sqlx`].
pub async fn load_item_rare_table_section_sqlx(
    pool: &ConnectionPool,
    loader: &ItemRareLoader,
) -> Result<BootSection, ItemAttrSqlxLoadError> {
    let limits = loader.limits();
    let rows = load_item_rare_table_rows_sqlx(pool, loader).await?;
    build_item_attr_section_with_limits(ItemAttrTableKind::Rare, &rows, limits)
        .map_err(ItemAttrSqlxLoadError::Section)
}

/// Compatibility spelling for the rare section loader.
///
/// # Errors
///
/// Returns the same typed errors as
/// [`load_item_rare_table_section_sqlx`].
pub async fn load_item_rare_section_sqlx(
    pool: &ConnectionPool,
    loader: &ItemRareLoader,
) -> Result<BootSection, ItemAttrSqlxLoadError> {
    load_item_rare_table_section_sqlx(pool, loader).await
}

/// Compatibility spelling without the `SQLx` suffix for the rare section.
///
/// # Errors
///
/// Returns the same typed errors as
/// [`load_item_rare_table_section_sqlx`].
pub async fn load_item_rare_section(
    pool: &ConnectionPool,
    loader: &ItemRareLoader,
) -> Result<BootSection, ItemAttrSqlxLoadError> {
    load_item_rare_table_section_sqlx(pool, loader).await
}

/// Acquire and build the normal section with the explicitly named legacy
/// conversion policy.
///
/// The acquisition boundary is identical to the strict loader. The pure
/// legacy builder, rather than this module, applies numeric-prefix/cast and
/// `strlcpy` compatibility behavior.
///
/// # Errors
///
/// Returns the same typed acquisition errors and wraps legacy pure-policy
/// failures in [`ItemAttrSqlxLoadError::Section`].
pub async fn load_item_attr_table_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &ItemAttrLoader,
) -> Result<BootSection, ItemAttrSqlxLoadError> {
    let limits = loader.limits();
    let rows = load_item_attr_table_rows_sqlx(pool, loader).await?;
    build_item_attr_section_legacy_with_limits(ItemAttrTableKind::Normal, &rows, limits)
        .map_err(ItemAttrSqlxLoadError::Section)
}

/// Compatibility spelling for the normal legacy section loader.
///
/// # Errors
///
/// Returns the same typed errors as
/// [`load_item_attr_table_section_legacy_sqlx`].
pub async fn load_item_attr_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &ItemAttrLoader,
) -> Result<BootSection, ItemAttrSqlxLoadError> {
    load_item_attr_table_section_legacy_sqlx(pool, loader).await
}

/// Compatibility spelling without the `SQLx` suffix for the normal legacy
/// section.
///
/// # Errors
///
/// Returns the same typed errors as
/// [`load_item_attr_table_section_legacy_sqlx`].
pub async fn load_item_attr_section_legacy(
    pool: &ConnectionPool,
    loader: &ItemAttrLoader,
) -> Result<BootSection, ItemAttrSqlxLoadError> {
    load_item_attr_table_section_legacy_sqlx(pool, loader).await
}

/// Acquire and build the rare section with the explicitly named legacy
/// conversion policy.
///
/// # Errors
///
/// Returns the same typed acquisition errors and wraps legacy pure-policy
/// failures in [`ItemAttrSqlxLoadError::Section`].
pub async fn load_item_rare_table_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &ItemRareLoader,
) -> Result<BootSection, ItemAttrSqlxLoadError> {
    let limits = loader.limits();
    let rows = load_item_rare_table_rows_sqlx(pool, loader).await?;
    build_item_attr_section_legacy_with_limits(ItemAttrTableKind::Rare, &rows, limits)
        .map_err(ItemAttrSqlxLoadError::Section)
}

/// Compatibility spelling for the rare legacy section loader.
///
/// # Errors
///
/// Returns the same typed errors as
/// [`load_item_rare_table_section_legacy_sqlx`].
pub async fn load_item_rare_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &ItemRareLoader,
) -> Result<BootSection, ItemAttrSqlxLoadError> {
    load_item_rare_table_section_legacy_sqlx(pool, loader).await
}

/// Compatibility spelling without the `SQLx` suffix for the rare legacy section.
///
/// # Errors
///
/// Returns the same typed errors as
/// [`load_item_rare_table_section_legacy_sqlx`].
pub async fn load_item_rare_section_legacy(
    pool: &ConnectionPool,
    loader: &ItemRareLoader,
) -> Result<BootSection, ItemAttrSqlxLoadError> {
    load_item_rare_table_section_legacy_sqlx(pool, loader).await
}

/// Acquire rows for either checked query through one generic normal/rare
/// boundary.
///
/// The enum variant selects the exact kind and statement. No profile is chosen
/// and no row policy is applied here.
///
/// # Errors
///
/// Returns the same typed acquisition errors as the concrete loaders.
pub async fn load_item_attr_rows_for_kind_sqlx(
    pool: &ConnectionPool,
    query: ItemAttrSqlxQuery<'_>,
    limits: ItemAttrSectionLimits,
) -> Result<Vec<ItemAttrTableQueryRow>, ItemAttrSqlxLoadError> {
    acquire_rows(pool, query.kind(), query.as_str(), limits).await
}

/// Build a strict section for either checked query through one generic
/// normal/rare boundary.
///
/// # Errors
///
/// Returns the same typed acquisition and pure-section errors as the concrete
/// loaders.
pub async fn load_item_attr_section_for_kind_sqlx(
    pool: &ConnectionPool,
    query: ItemAttrSqlxQuery<'_>,
    limits: ItemAttrSectionLimits,
) -> Result<BootSection, ItemAttrSqlxLoadError> {
    let kind = query.kind();
    let rows = load_item_attr_rows_for_kind_sqlx(pool, query, limits).await?;
    build_item_attr_section_with_limits(kind, &rows, limits).map_err(ItemAttrSqlxLoadError::Section)
}

/// Build a section for either checked query with the explicitly named legacy
/// policy.
///
/// # Errors
///
/// Returns the same typed acquisition and pure-section errors as the concrete
/// loaders.
pub async fn load_item_attr_section_for_kind_legacy_sqlx(
    pool: &ConnectionPool,
    query: ItemAttrSqlxQuery<'_>,
    limits: ItemAttrSectionLimits,
) -> Result<BootSection, ItemAttrSqlxLoadError> {
    let kind = query.kind();
    let rows = load_item_attr_rows_for_kind_sqlx(pool, query, limits).await?;
    build_item_attr_section_legacy_with_limits(kind, &rows, limits)
        .map_err(ItemAttrSqlxLoadError::Section)
}

/// Validate a configured source-row cap before it reaches the pool.
///
/// The cap must fit the legacy `u16` section count. This check is separate from
/// the pure section limits so an invalid cap cannot cause a query or a
/// potentially large bounded acquisition.
///
/// # Errors
///
/// Returns [`ItemAttrSqlxLoadError::InvalidSourceLimit`] when `maximum` is
/// greater than the representable source-row count.
pub fn validate_source_limit(maximum: usize) -> Result<(), ItemAttrSqlxLoadError> {
    if maximum > ITEM_ATTR_TABLE_MAX_RECORDS {
        Err(ItemAttrSqlxLoadError::InvalidSourceLimit {
            maximum,
            limit: ITEM_ATTR_TABLE_MAX_RECORDS,
        })
    } else {
        Ok(())
    }
}

/// Compatibility spelling for [`validate_source_limit`].
///
/// # Errors
///
/// Returns [`ItemAttrSqlxLoadError::InvalidSourceLimit`] when `maximum` is
/// greater than the representable source-row count.
pub fn validate_item_attr_source_limit(maximum: usize) -> Result<(), ItemAttrSqlxLoadError> {
    validate_source_limit(maximum)
}

/// Check that a bounded result is non-empty before it is accumulated.
///
/// The legacy item-attribute loaders reject zero source rows. Keeping this
/// check separate makes the empty-result boundary testable without a live
/// database and prevents an empty source from being mistaken for a successful
/// section.
///
/// # Errors
///
/// Returns [`ItemAttrSqlxLoadError::EmptyResult`] when `rows` is empty.
pub fn check_item_attr_nonempty(
    rows: &[ItemAttrTableQueryRow],
) -> Result<(), ItemAttrSqlxLoadError> {
    if rows.is_empty() {
        Err(ItemAttrSqlxLoadError::EmptyResult)
    } else {
        Ok(())
    }
}

/// Convert one already-decoded SQL cell into the lossless item-attribute cell
/// boundary without UTF-8 conversion or normalization.
///
/// # Examples
///
/// ```
/// use db_server::item_attr_sqlx::byte_cell_to_item_attr_value;
/// use db_server::item_attr::ItemAttrQueryValue;
///
/// let value = byte_cell_to_item_attr_value(Some(vec![0xff, 0, b'7']));
/// assert!(matches!(value, ItemAttrQueryValue::Bytes(bytes) if bytes == [0xff, 0, b'7']));
/// ```
#[must_use]
pub fn byte_cell_to_item_attr_value(value: Option<Vec<u8>>) -> ItemAttrQueryValue {
    value.map_or(ItemAttrQueryValue::Null, ItemAttrQueryValue::Bytes)
}

/// Compatibility spelling emphasizing that the rare table uses the same cell
/// boundary.
#[must_use]
pub fn byte_cell_to_item_rare_value(value: Option<Vec<u8>>) -> ItemAttrQueryValue {
    byte_cell_to_item_attr_value(value)
}

/// Compatibility spelling for callers that call the value a raw cell.
#[must_use]
pub fn raw_cell_to_item_attr_value(value: Option<Vec<u8>>) -> ItemAttrQueryValue {
    byte_cell_to_item_attr_value(value)
}

/// Check the exact eighteen-column normal result shape.
///
/// # Errors
///
/// Returns [`ItemAttrSqlxLoadError::RowShape`] when `actual` differs from the
/// source-fixed normal width.
pub fn check_item_attr_row_shape(row: usize, actual: usize) -> Result<(), ItemAttrSqlxLoadError> {
    check_item_attr_row_shape_for_kind(ItemAttrTableKind::Normal, row, actual)
}

/// Check the exact sixteen-column rare result shape.
///
/// # Errors
///
/// Returns [`ItemAttrSqlxLoadError::RowShape`] when `actual` differs from the
/// source-fixed rare width.
pub fn check_item_rare_row_shape(row: usize, actual: usize) -> Result<(), ItemAttrSqlxLoadError> {
    check_item_attr_row_shape_for_kind(ItemAttrTableKind::Rare, row, actual)
}

/// Check a row shape against an explicitly selected table kind.
///
/// # Errors
///
/// Returns [`ItemAttrSqlxLoadError::RowShape`] when the actual width does not
/// equal the selected source-fixed width.
pub fn check_item_attr_row_shape_for_kind(
    kind: ItemAttrTableKind,
    row: usize,
    actual: usize,
) -> Result<(), ItemAttrSqlxLoadError> {
    let expected = match kind {
        ItemAttrTableKind::Normal => ITEM_ATTR_TABLE_QUERY_COLUMNS,
        ItemAttrTableKind::Rare => ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS,
    };
    if actual == expected {
        Ok(())
    } else {
        Err(ItemAttrSqlxLoadError::RowShape {
            row,
            expected,
            actual,
        })
    }
}

async fn acquire_rows(
    pool: &ConnectionPool,
    kind: ItemAttrTableKind,
    query: &str,
    limits: ItemAttrSectionLimits,
) -> Result<Vec<ItemAttrTableQueryRow>, ItemAttrSqlxLoadError> {
    validate_source_limit(limits.max_records)?;

    // The query argument here is obtained only from a checked postfix query
    // type by the public wrappers. Keep this one pool call before any adapter
    // row/cell accumulation; query_up_to returns None for an extra row.
    let bounded = pool
        .query_up_to(query, limits.max_records)
        .await
        .map_err(ItemAttrSqlxLoadError::Database)?;
    let Some(rows) = bounded else {
        return Err(ItemAttrSqlxLoadError::SourceLimitExceeded {
            maximum: limits.max_records,
        });
    };
    if rows.is_empty() {
        return Err(ItemAttrSqlxLoadError::EmptyResult);
    }

    let mut source_rows = Vec::new();
    source_rows.try_reserve_exact(rows.len()).map_err(|_| {
        ItemAttrSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        }
    })?;

    for (row_index, row) in rows.into_iter().enumerate() {
        let actual = row.columns().len();
        check_item_attr_row_shape_for_kind(kind, row_index, actual)?;

        let expected = kind.column_count();
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(expected)
            .map_err(|_| ItemAttrSqlxLoadError::AllocationFailed {
                requested: expected,
            })?;
        for column in 0..expected {
            // `query_up_to` uses `sqlx::raw_sql`, so MySQL returns
            // its source-compatible raw/text projection. The unchecked type
            // read bypasses only `SQLx`'s numeric metadata compatibility check;
            // positional bounds, NULL state, and raw bytes remain explicit.
            let value = row
                .try_get_unchecked::<Option<Vec<u8>>, usize>(column)
                .map_err(|source| ItemAttrSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                })?;
            cells.push(byte_cell_to_item_attr_value(value));
        }
        source_rows.push(ItemAttrTableQueryRow::from_typed_columns(cells));
    }

    Ok(source_rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item_attr::{ItemAttrSectionLimits, ITEM_ATTR_TABLE_WIRE_SIZE};
    use crate::postfix::TablePostfix;

    fn normal_loader() -> ItemAttrLoader {
        ItemAttrLoader::new(
            &TablePostfix::parse("_test").unwrap(),
            ItemAttrSectionLimits::new(7),
        )
        .unwrap()
    }

    fn rare_loader() -> ItemRareLoader {
        ItemRareLoader::new(
            &TablePostfix::parse("_test").unwrap(),
            ItemAttrSectionLimits::new(7),
        )
        .unwrap()
    }

    fn normal_row() -> ItemAttrTableQueryRow {
        let mut cells = vec![byte_cell_to_item_attr_value(Some(b"A".to_vec()))];
        cells.extend((0..17).map(|_| byte_cell_to_item_attr_value(Some(b"1".to_vec()))));
        ItemAttrTableQueryRow::new(cells)
    }

    fn rare_row() -> ItemAttrTableQueryRow {
        let mut cells = vec![byte_cell_to_item_attr_value(Some(b"R".to_vec()))];
        cells.extend((0..15).map(|_| byte_cell_to_item_attr_value(Some(b"1".to_vec()))));
        ItemAttrTableQueryRow::new(cells)
    }

    #[test]
    fn checked_normal_and_rare_queries_keep_exact_statements() {
        let normal = normal_loader();
        let rare = rare_loader();
        assert_eq!(
            normal.query().as_str(),
            "SELECT apply, apply+0, prob, lv1, lv2, lv3, lv4, lv5, weapon, body, wrist, foots, neck, head, shield, ear, talisman, glove FROM item_attr_test ORDER BY apply"
        );
        assert_eq!(
            rare.query().as_str(),
            "SELECT apply, apply+0, prob, lv1, lv2, lv3, lv4, lv5, weapon, body, wrist, foots, neck, head, shield, ear FROM item_attr_rare_test ORDER BY apply"
        );
        assert_eq!(normal.query().as_str().matches(',').count() + 1, 18);
        assert_eq!(rare.query().as_str().matches(',').count() + 1, 16);
    }

    #[test]
    fn raw_cells_preserve_null_and_non_utf8_bytes() {
        assert!(matches!(
            byte_cell_to_item_attr_value(None),
            ItemAttrQueryValue::Null
        ));
        let source = vec![0xff, 0, b'7', 0x80];
        assert_eq!(
            byte_cell_to_item_attr_value(Some(source.clone())),
            ItemAttrQueryValue::Bytes(source.clone())
        );
        assert_eq!(
            raw_cell_to_item_attr_value(Some(source.clone())),
            ItemAttrQueryValue::Bytes(source.clone())
        );
    }

    #[test]
    fn shape_and_cap_checks_are_kind_specific() {
        assert!(check_item_attr_row_shape(0, 18).is_ok());
        assert!(check_item_rare_row_shape(0, 16).is_ok());
        assert!(matches!(
            check_item_attr_row_shape(3, 16),
            Err(ItemAttrSqlxLoadError::RowShape {
                row: 3,
                expected: 18,
                actual: 16
            })
        ));
        assert!(matches!(
            check_item_rare_row_shape(3, 18),
            Err(ItemAttrSqlxLoadError::RowShape {
                row: 3,
                expected: 16,
                actual: 18
            })
        ));
        assert!(validate_source_limit(0).is_ok());
        assert!(validate_source_limit(ITEM_ATTR_TABLE_MAX_RECORDS).is_ok());
        assert!(matches!(
            validate_source_limit(ITEM_ATTR_TABLE_MAX_RECORDS + 1),
            Err(ItemAttrSqlxLoadError::InvalidSourceLimit { .. })
        ));
    }

    #[test]
    fn empty_result_and_pure_errors_remain_distinct() {
        assert!(matches!(
            check_item_attr_nonempty(&[]),
            Err(ItemAttrSqlxLoadError::EmptyResult)
        ));
        assert!(check_item_attr_nonempty(&[normal_row()]).is_ok());
        let error = ItemAttrSqlxLoadError::Section(ItemAttrSectionError::TooManyRecords {
            count: 2,
            maximum: 1,
        });
        assert!(error.to_string().contains("section construction failed"));
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn order_and_duplicates_reach_the_pure_builder_unchanged() {
        let first = normal_row();
        let section = crate::item_attr::build_item_attr_section_with_limits(
            ItemAttrTableKind::Normal,
            &[first.clone(), first.clone()],
            ItemAttrSectionLimits::new(2),
        )
        .unwrap();
        assert_eq!(section.record_size as usize, ITEM_ATTR_TABLE_WIRE_SIZE);
        assert_eq!(section.count, 2);
        assert_eq!(section.data.len(), ITEM_ATTR_TABLE_WIRE_SIZE * 2);
    }

    #[test]
    fn rare_shape_reaches_the_shared_record_builder() {
        let section = crate::item_attr::build_item_attr_section_with_limits(
            ItemAttrTableKind::Rare,
            &[rare_row()],
            ItemAttrSectionLimits::new(1),
        )
        .unwrap();
        assert_eq!(section.kind, protocol::db_boot::BootSectionKind::ItemRare);
        assert_eq!(section.data.len(), ITEM_ATTR_TABLE_WIRE_SIZE);
    }
}
