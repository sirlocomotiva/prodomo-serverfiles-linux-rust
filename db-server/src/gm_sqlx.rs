//! `SQLx` acquisition adapter for the boot tail's GM lists.
//!
//! The pure row, filter, and packing rules live in [`crate::gm`]. This module
//! supplies only the real database call: it runs the exact legacy statements
//! against the common pool, preserves fetch order, and hands the raw column
//! bytes to the pure builder. It does not choose a schema, create tables, or
//! turn a query failure into an empty list.

use std::error::Error;
use std::fmt;

use db::sqlx::Row;
use db::{ConnectionPool, DbError};
use protocol::db_boot::{BootAdminInfo, BootGmHost};

use crate::gm::{
    build_admin_list, build_host_list, AdminQuery, AdminRow, GmSectionError, GmSectionLimits,
    GM_HOST_QUERY,
};

/// A failure while acquiring or bounding a SQLx-backed GM list.
#[derive(Debug)]
pub enum GmSqlxLoadError {
    /// The pool could not execute the fixed query after its retry policy.
    Database(DbError),
    /// The source contained more rows than the configured cap.
    SourceLimitExceeded {
        /// Which list overflowed.
        maximum: usize,
    },
    /// A returned row did not have the column shape the query promises.
    RowShape {
        /// Which list the row belongs to.
        list: GmTailList,
        /// Zero-based row index in the result stream.
        row: usize,
        /// Required column count.
        expected: usize,
        /// Actual column count.
        actual: usize,
    },
    /// A returned column could not be decoded.
    ColumnDecode {
        /// Which list the column belongs to.
        list: GmTailList,
        /// Zero-based row index in the result stream.
        row: usize,
        /// The column that failed.
        column: &'static str,
        /// `SQLx` decode error.
        source: db::sqlx::Error,
    },
    /// The `mID` column was not a four-byte signed integer.
    AdminIdDecode {
        /// Zero-based row index in the result stream.
        row: usize,
        /// `SQLx` decode error.
        source: db::sqlx::Error,
    },
    /// The returned rows violate the pure packing policy.
    Section(GmSectionError),
}

/// Which of the two tail lists an adapter error refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GmTailList {
    /// The `gmhost` list.
    Hosts,
    /// The `gmlist` administrators.
    Admins,
}

impl fmt::Display for GmSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => write!(formatter, "GM list database query failed: {source}"),
            Self::SourceLimitExceeded { maximum } => {
                write!(formatter, "GM list source exceeded the {maximum}-row limit")
            }
            Self::RowShape {
                list,
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "{list:?} row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                list,
                row,
                column,
                source,
            } => write!(
                formatter,
                "{list:?} row {row} column {column} decode failed: {source}"
            ),
            Self::AdminIdDecode { row, source } => {
                write!(formatter, "admin row {row} id decode failed: {source}")
            }
            Self::Section(source) => write!(formatter, "GM section construction failed: {source}"),
        }
    }
}

impl Error for GmSqlxLoadError {
    #[allow(clippy::match_same_arms)]
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            // The four arms share a body but not a source type, so they
            // cannot be one or-pattern. Merging them would need a boxed
            // `dyn Error`, which would erase the concrete type that
            // `source()` promises to return.
            Self::Database(source) => Some(source),
            Self::ColumnDecode { source, .. } => Some(source),
            Self::AdminIdDecode { source, .. } => Some(source),
            Self::Section(source) => Some(source),
            Self::SourceLimitExceeded { .. } | Self::RowShape { .. } => None,
        }
    }
}

impl From<GmSectionError> for GmSqlxLoadError {
    fn from(source: GmSectionError) -> Self {
        Self::Section(source)
    }
}

/// Query `gmhost` and build the packed host list.
///
/// The statement is the fixed [`GM_HOST_QUERY`]. A `NULL` or empty `mIP` is
/// skipped, exactly as legacy skips it, rather than becoming a zeroed host that
/// would look like a real address to the game server.
///
/// # Errors
///
/// Returns [`GmSqlxLoadError::Database`] for pool/query failures,
/// [`GmSqlxLoadError::SourceLimitExceeded`] when the bounded stream sees more
/// rows than allowed, [`GmSqlxLoadError::RowShape`] or
/// [`GmSqlxLoadError::ColumnDecode`] for malformed SQL results, and
/// [`GmSqlxLoadError::Section`] for pure record/byte/allocation limits.
pub async fn load_gm_hosts_sqlx(
    common: &ConnectionPool,
    limits: GmSectionLimits,
) -> Result<Vec<BootGmHost>, GmSqlxLoadError> {
    let rows = common
        .query_up_to(GM_HOST_QUERY, limits.max_host_rows)
        .await
        .map_err(GmSqlxLoadError::Database)?;
    let Some(rows) = rows else {
        return Err(GmSqlxLoadError::SourceLimitExceeded {
            maximum: limits.max_host_rows,
        });
    };

    let mut values = Vec::new();
    values.try_reserve(rows.len()).map_err(|_| {
        GmSqlxLoadError::Section(GmSectionError::AllocationFailed {
            list: crate::gm::GmList::Hosts,
        })
    })?;
    for (row_index, row) in rows.into_iter().enumerate() {
        check_shape(GmTailList::Hosts, row_index, row.columns().len(), 1)?;
        let host = row.try_get::<Option<Vec<u8>>, _>("mIP").map_err(|source| {
            GmSqlxLoadError::ColumnDecode {
                list: GmTailList::Hosts,
                row: row_index,
                column: "mIP",
                source,
            }
        })?;
        values.push(host);
    }

    build_host_list(&values, &limits).map_err(GmSqlxLoadError::Section)
}

/// Query `gmlist` for one requesting address and build the packed list.
///
/// The `WHERE` clause is built by [`AdminQuery`], which validates the address
/// before interpolating it. The `mAuthority` string filter is applied by the
/// pure builder so an unrecognized value drops the row here exactly as it does
/// in legacy.
///
/// # Errors
///
/// Returns [`GmSqlxLoadError::Database`] for pool/query failures,
/// [`GmSqlxLoadError::SourceLimitExceeded`] when the bounded stream sees more
/// rows than allowed, [`GmSqlxLoadError::RowShape`],
/// [`GmSqlxLoadError::ColumnDecode`], or [`GmSqlxLoadError::AdminIdDecode`] for
/// malformed SQL results, and [`GmSqlxLoadError::Section`] for pure
/// record/byte/allocation limits.
pub async fn load_admins_sqlx(
    common: &ConnectionPool,
    query: &AdminQuery,
    limits: GmSectionLimits,
) -> Result<Vec<BootAdminInfo>, GmSqlxLoadError> {
    let rows = common
        .query_up_to(query.as_str(), limits.max_admin_rows)
        .await
        .map_err(GmSqlxLoadError::Database)?;
    let Some(rows) = rows else {
        return Err(GmSqlxLoadError::SourceLimitExceeded {
            maximum: limits.max_admin_rows,
        });
    };

    let mut parsed = Vec::new();
    parsed.try_reserve(rows.len()).map_err(|_| {
        GmSqlxLoadError::Section(GmSectionError::AllocationFailed {
            list: crate::gm::GmList::Admins,
        })
    })?;
    for (row_index, row) in rows.into_iter().enumerate() {
        check_shape(GmTailList::Admins, row_index, row.columns().len(), 6)?;
        let id = row
            .try_get::<Option<i32>, _>("mID")
            .map_err(|source| GmSqlxLoadError::AdminIdDecode {
                row: row_index,
                source,
            })?
            .unwrap_or_default();
        let read = |column: &'static str| -> Result<Option<Vec<u8>>, GmSqlxLoadError> {
            row.try_get::<Option<Vec<u8>>, _>(column).map_err(|source| {
                GmSqlxLoadError::ColumnDecode {
                    list: GmTailList::Admins,
                    row: row_index,
                    column,
                    source,
                }
            })
        };
        parsed.push(AdminRow {
            id,
            account: read("mAccount")?,
            name: read("mName")?,
            contact_ip: read("mContactIP")?,
            server_ip: read("mServerIP")?,
            authority: read("mAuthority")?,
        });
    }

    build_admin_list(&parsed, &limits).map_err(GmSqlxLoadError::Section)
}

fn check_shape(
    list: GmTailList,
    row: usize,
    actual: usize,
    expected: usize,
) -> Result<(), GmSqlxLoadError> {
    if actual == expected {
        Ok(())
    } else {
        Err(GmSqlxLoadError::RowShape {
            list,
            row,
            expected,
            actual,
        })
    }
}
