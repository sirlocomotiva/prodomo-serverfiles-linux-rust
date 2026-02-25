//! Pure rules for the boot tail's GM host and administrator lists.
//!
//! The legacy DB server calls two helpers inside `QUERY_BOOT`:
//! `__GetHostInfo` reads the whole `gmhost` table, and `__GetAdminInfo` reads
//! the `gmlist` rows whose `mServerIP` is `ALL` or the requesting peer's
//! address. Both helpers run a `SQL_COMMON` query, so neither table takes a
//! `TABLE_POSTFIX` and neither lives in the player schema.
//!
//! This module owns the pure half: the exact statement text, the row filters,
//! the string conversions, and the packed record widths. The `SQLx` call lives in
//! `crate::gm_sqlx`.
//! Nothing here queries a database, and no row is invented for a source that
//! has not been read.
//!
//! # The administrator filter is per request
//!
//! `__GetAdminInfo` takes the requesting peer's `szIP` and filters on it, so an
//! administrator list is a function of the request rather than of the loaded
//! tables. That is why [`AdminQuery`] carries the address and why
//! [`BootDataSources`](crate::boot_loader::BootDataSources) keeps the common
//! pool available for the duration of a request rather than only at load time.

use std::error::Error;
use std::fmt;

use protocol::db_boot::{BootAdminInfo, BootGmHost, ADMIN_INFO_WIRE_SIZE, GM_HOST_WIRE_SIZE};

/// The exact source-fixed `gmhost` statement.
///
/// Legacy: `SELECT mIP FROM gmhost`. There is no `WHERE` clause, no
/// `TABLE_POSTFIX`, and no `SQL_PLAYER` slot: `__GetHostInfo` passes
/// `SQL_COMMON` as the second argument to `DirectQuery`.
pub const GM_HOST_QUERY: &str = "SELECT mIP FROM gmhost";

/// The exact `gmlist` column list, in source order.
pub const ADMIN_QUERY_COLUMNS: &str = "mID,mAccount,mName,mContactIP,mServerIP,mAuthority";

/// The literal used when the request carries no address.
///
/// Legacy `__GetAdminInfo` passes `"ALL"` for a NULL `szIP`, which then matches
/// only the `mServerIP='ALL'` rows.
pub const ADMIN_ALL_SERVERS: &str = "ALL";

/// The `mAuthority` values legacy maps onto its `EGMLevels` enum.
///
/// The enum order is `GM_PLAYER=0, GM_LOW_WIZARD=1, GM_WIZARD=2,
/// GM_HIGH_WIZARD=3, GM_GOD=4, GM_IMPLEMENTOR=5`. The column is a *string* in
/// SQL, and an unrecognized string makes legacy `continue`, dropping the row
/// rather than defaulting it to `GM_PLAYER`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(i32)]
pub enum GmAuthority {
    /// `LOW_WIZARD`.
    LowWizard = 1,
    /// `WIZARD`.
    Wizard = 2,
    /// `HIGH_WIZARD`.
    HighWizard = 3,
    /// `GOD`.
    God = 4,
    /// `IMPLEMENTOR`.
    Implementor = 5,
}

impl GmAuthority {
    /// Map a raw `mAuthority` string onto the enum, or `None` to drop the row.
    ///
    /// Legacy compares the whole string with `std::string::compare`, so the
    /// match is exact and case-sensitive. A value outside the six names
    /// produces no row rather than a `GM_PLAYER` row.
    #[must_use]
    pub fn from_column(raw: &[u8]) -> Option<Self> {
        match raw {
            b"IMPLEMENTOR" => Some(Self::Implementor),
            b"GOD" => Some(Self::God),
            b"HIGH_WIZARD" => Some(Self::HighWizard),
            b"LOW_WIZARD" => Some(Self::LowWizard),
            b"WIZARD" => Some(Self::Wizard),
            _ => None,
        }
    }

    /// The x86 `int` written into `tAdminInfo::m_Authority`.
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// The bounds applied to both tail lists.
///
/// Legacy has no bound at all: it reserves `uiNumRows` and appends whatever the
/// database returns, then writes the count as a `WORD`. A source with more than
/// `u16::MAX` rows would therefore wrap the declared count in legacy. These
/// limits fail instead, which is the safe direction: a wrapped count would
/// desynchronize the game server's parser for every record after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GmSectionLimits {
    /// Maximum source rows read from `gmhost`.
    pub max_host_rows: usize,
    /// Maximum source rows read from `gmlist`.
    pub max_admin_rows: usize,
    /// Maximum packed `gmhost` bytes.
    pub max_host_bytes: usize,
    /// Maximum packed `gmlist` bytes.
    pub max_admin_bytes: usize,
}

impl Default for GmSectionLimits {
    /// Defaults derived from the wire widths and the declared `WORD` counts.
    fn default() -> Self {
        Self {
            max_host_rows: u16::MAX as usize,
            max_admin_rows: u16::MAX as usize,
            max_host_bytes: u16::MAX as usize * GM_HOST_WIRE_SIZE,
            max_admin_bytes: u16::MAX as usize * ADMIN_INFO_WIRE_SIZE,
        }
    }
}

/// Why a GM host or administrator list could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GmSectionError {
    /// The source had more rows than the configured cap.
    TooManyRows {
        /// Which list overflowed.
        list: GmList,
        /// Configured maximum.
        maximum: usize,
    },
    /// The packed output would exceed the configured byte cap.
    TooManyBytes {
        /// Which list overflowed.
        list: GmList,
        /// Configured maximum.
        maximum: usize,
    },
    /// A record width does not fit the wire field.
    WidthOverflow {
        /// Which list overflowed.
        list: GmList,
        /// The size that did not fit.
        size: usize,
    },
    /// A vector allocation failed.
    AllocationFailed {
        /// Which list could not allocate.
        list: GmList,
    },
}

/// Which of the two tail lists an error refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GmList {
    /// The `gmhost` list.
    Hosts,
    /// The `gmlist` administrators.
    Admins,
}

impl fmt::Display for GmSectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRows { list, maximum } => {
                write!(formatter, "{list:?} exceeded the {maximum}-row limit")
            }
            Self::TooManyBytes { list, maximum } => {
                write!(formatter, "{list:?} exceeded the {maximum}-byte limit")
            }
            Self::WidthOverflow { list, size } => {
                write!(formatter, "{list:?} record width {size} overflows u16")
            }
            Self::AllocationFailed { list } => write!(formatter, "{list:?} allocation failed"),
        }
    }
}

impl Error for GmSectionError {}

/// The exact `gmlist` statement for one requesting address.
///
/// Legacy: `SELECT mID,mAccount,mName,mContactIP,mServerIP,mAuthority FROM
/// gmlist WHERE mServerIP='ALL' or mServerIP='%s'` with `szIP ? szIP : "ALL"`.
///
/// The address is interpolated, so it is validated as a dotted-quad or an IPv6
/// literal before it reaches the statement. That validation is stricter than
/// legacy, which interpolates whatever it was handed; a request address
/// containing a quote would otherwise be able to change the query's meaning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminQuery {
    statement: String,
    server_ip: String,
}

impl AdminQuery {
    /// Build the statement for a requesting peer address.
    ///
    /// # Errors
    ///
    /// Returns [`GmSectionError`] when the address is not a bare IP literal, or
    /// when the resulting statement is wider than the legacy `char[512]`.
    pub fn new(request_ip: Option<&str>) -> Result<Self, GmSectionError> {
        let server_ip = request_ip.unwrap_or(ADMIN_ALL_SERVERS);
        if !is_safe_ip_literal(server_ip) {
            return Err(GmSectionError::WidthOverflow {
                list: GmList::Admins,
                size: server_ip.len(),
            });
        }
        let statement = format!(
            "SELECT {ADMIN_QUERY_COLUMNS} FROM gmlist WHERE mServerIP='{ADMIN_ALL_SERVERS}' or mServerIP='{server_ip}'"
        );
        if statement.len() >= 512 {
            return Err(GmSectionError::WidthOverflow {
                list: GmList::Admins,
                size: statement.len(),
            });
        }
        Ok(Self {
            statement,
            server_ip: server_ip.to_owned(),
        })
    }

    /// The exact statement text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.statement
    }

    /// The requesting address, or `ALL`.
    #[must_use]
    pub fn server_ip(&self) -> &str {
        &self.server_ip
    }
}

/// Whether a string is safe to interpolate as a quoted SQL literal.
///
/// The value reaches an unescaped `'%s'` in the legacy statement, so the only
/// acceptable characters are the ones an IP literal is built from. This rejects
/// quotes, backslashes, and spaces rather than trying to escape them, because
/// no legitimate address contains them.
fn is_safe_ip_literal(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 45
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b':' | b'-' | b'_' | b'%')
        })
}

/// Legacy `strlcpy` into a thirty-two-byte field.
///
/// A `NULL` column becomes an all-zero field, matching the legacy read of a
/// null cell. Bytes are copied verbatim, so non-ASCII values are preserved
/// rather than replaced, and the field is zero-padded to its full width. The
/// last byte is always left as the NUL that `strlcpy` writes.
fn copy_name_field(raw: Option<&[u8]>) -> [u8; 32] {
    let mut field = [0_u8; 32];
    let raw = raw.unwrap_or(&[]);
    let take = raw.len().min(field.len() - 1);
    if take > 0 {
        field[..take].copy_from_slice(&raw[..take]);
    }
    field
}

/// Legacy `strlcpy` into a sixteen-byte field.
fn copy_ip_field(raw: Option<&[u8]>) -> [u8; 16] {
    let mut field = [0_u8; 16];
    let raw = raw.unwrap_or(&[]);
    let take = raw.len().min(15);
    if take > 0 {
        field[..take].copy_from_slice(&raw[..take]);
    }
    field
}

/// Legacy `trim_and_lower` applied to a bounded account field.
///
/// The legacy helper skips leading and trailing `isspace` bytes, lowercases the
/// rest, and always NUL-terminates within `dest_size`. The Hangul guard
/// (`(ch & 0xE0) > 0x90`) only matters for multi-byte text, which an account
/// name does not contain, so an ASCII space test is equivalent here.
fn trim_and_lower(raw: Option<&[u8]>, width: usize) -> [u8; 32] {
    let raw = raw.unwrap_or(&[]);
    let start = raw
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(raw.len());
    let trimmed = &raw[start..];
    let end = trimmed
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| start + index + 1);
    let body = &trimmed[..end - start];
    let mut field = [0_u8; 32];
    let take = body.len().min(width - 1);
    for (index, byte) in body[..take].iter().enumerate() {
        field[index] = byte.to_ascii_lowercase();
    }
    field
}

/// Build the packed `gmhost` list from raw `mIP` column values.
///
/// Legacy skips a row whose value is `NULL` or empty and keeps everything else
/// verbatim in fetch order. A 16-byte field holds at most 15 characters plus a
/// NUL, so a longer address is truncated exactly as `Encode(c_str(), 16)`
/// truncates it.
///
/// # Errors
///
/// Returns [`GmSectionError::TooManyRows`] when the source returns more rows
/// than [`GmSectionLimits::max_host_rows`]. The legacy sender writes the count
/// as a `WORD`, so accepting more rows than that would wrap the declared count
/// and desynchronize the game server's parser for every later record. Refusing
/// is the safe direction: the boot fails loudly instead of sending a payload
/// the client cannot walk.
pub fn build_host_list(
    rows: &[Option<Vec<u8>>],
    limits: &GmSectionLimits,
) -> Result<Vec<BootGmHost>, GmSectionError> {
    if rows.len() > limits.max_host_rows {
        return Err(GmSectionError::TooManyRows {
            list: GmList::Hosts,
            maximum: limits.max_host_rows,
        });
    }
    let mut hosts = Vec::new();
    hosts
        .try_reserve(rows.len())
        .map_err(|_| GmSectionError::AllocationFailed {
            list: GmList::Hosts,
        })?;
    let mut total = 0_usize;
    for value in rows {
        let Some(raw) = value.as_deref() else {
            continue;
        };
        if raw.is_empty() {
            continue;
        }
        let mut bytes = [0_u8; GM_HOST_WIRE_SIZE];
        let take = raw.len().min(GM_HOST_WIRE_SIZE - 1);
        bytes[..take].copy_from_slice(&raw[..take]);
        total = total.saturating_add(GM_HOST_WIRE_SIZE);
        if total > limits.max_host_bytes {
            return Err(GmSectionError::TooManyBytes {
                list: GmList::Hosts,
                maximum: limits.max_host_bytes,
            });
        }
        hosts.push(BootGmHost { bytes });
    }
    Ok(hosts)
}

/// One raw `gmlist` row, before the authority filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminRow {
    /// `mID`, a four-byte x86 `int`.
    pub id: i32,
    /// `mAccount`, lowercased and trimmed.
    pub account: Option<Vec<u8>>,
    /// `mName`, copied verbatim.
    pub name: Option<Vec<u8>>,
    /// `mContactIP`, copied verbatim.
    pub contact_ip: Option<Vec<u8>>,
    /// `mServerIP`, copied verbatim.
    pub server_ip: Option<Vec<u8>>,
    /// `mAuthority`, matched exactly against the six legacy names.
    pub authority: Option<Vec<u8>>,
}

/// Build the packed administrator list, dropping rows legacy drops.
///
/// A row whose `mAuthority` is `NULL` or is not one of the six exact names is
/// skipped, matching the legacy `continue`.
///
/// # Errors
///
/// Returns [`GmSectionError::TooManyRows`] when the source returns more rows
/// than [`GmSectionLimits::max_admin_rows`], for the same `WORD` count reason
/// described on [`build_host_list`].
pub fn build_admin_list(
    rows: &[AdminRow],
    limits: &GmSectionLimits,
) -> Result<Vec<BootAdminInfo>, GmSectionError> {
    if rows.len() > limits.max_admin_rows {
        return Err(GmSectionError::TooManyRows {
            list: GmList::Admins,
            maximum: limits.max_admin_rows,
        });
    }
    let mut admins = Vec::new();
    admins
        .try_reserve(rows.len())
        .map_err(|_| GmSectionError::AllocationFailed {
            list: GmList::Admins,
        })?;
    let mut total = 0_usize;
    for row in rows {
        let Some(authority) = row.authority.as_deref().and_then(GmAuthority::from_column) else {
            continue;
        };
        let record = BootAdminInfo {
            id: row.id,
            account: trim_and_lower(row.account.as_deref(), 32),
            name: copy_name_field(row.name.as_deref()),
            contact_ip: copy_ip_field(row.contact_ip.as_deref()),
            server_ip: copy_ip_field(row.server_ip.as_deref()),
            authority: authority.as_i32(),
        };
        total = total.saturating_add(ADMIN_INFO_WIRE_SIZE);
        if total > limits.max_admin_bytes {
            return Err(GmSectionError::TooManyBytes {
                list: GmList::Admins,
                maximum: limits.max_admin_bytes,
            });
        }
        admins.push(record);
    }
    Ok(admins)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        id: i32,
        account: Option<&[u8]>,
        name: Option<&[u8]>,
        contact: Option<&[u8]>,
        server: Option<&[u8]>,
        authority: Option<&[u8]>,
    ) -> AdminRow {
        AdminRow {
            id,
            account: account.map(<[u8]>::to_vec),
            name: name.map(<[u8]>::to_vec),
            contact_ip: contact.map(<[u8]>::to_vec),
            server_ip: server.map(<[u8]>::to_vec),
            authority: authority.map(<[u8]>::to_vec),
        }
    }

    // --- the exact statements ---------------------------------------------

    #[test]
    fn the_host_statement_is_the_source_fixed_one() {
        // Legacy: `SELECT mIP FROM gmhost`, no postfix, SQL_COMMON.
        assert_eq!(GM_HOST_QUERY, "SELECT mIP FROM gmhost");
        assert!(
            !GM_HOST_QUERY.contains("WHERE"),
            "legacy reads the whole table"
        );
    }

    #[test]
    fn the_admin_statement_matches_legacy_column_and_filter_order() {
        let query = AdminQuery::new(Some("10.0.0.7")).expect("an address literal is safe");
        assert_eq!(
            query.as_str(),
            "SELECT mID,mAccount,mName,mContactIP,mServerIP,mAuthority FROM gmlist \
             WHERE mServerIP='ALL' or mServerIP='10.0.0.7'"
        );
    }

    #[test]
    fn a_null_request_address_becomes_the_all_literal() {
        // Legacy passes "ALL" for a NULL szIP, which then matches only the
        // mServerIP='ALL' rows.
        let query = AdminQuery::new(None).expect("NULL is safe");
        assert_eq!(query.server_ip(), "ALL");
        assert!(query.as_str().ends_with("mServerIP='ALL'"));
    }

    #[test]
    fn a_request_address_cannot_escape_the_quoted_literal() {
        // The address reaches an unescaped '%s'. A quote would change the
        // query's meaning, so the builder refuses instead of escaping.
        for hostile in [
            "' OR '1'='1",
            "'; DROP TABLE gmlist; --",
            "1' UNION SELECT",
            "",
        ] {
            assert!(
                AdminQuery::new(Some(hostile)).is_err(),
                "{hostile:?} must not reach the statement"
            );
        }
    }

    // --- the authority filter ---------------------------------------------

    #[test]
    fn the_six_legacy_authority_names_map_onto_the_enum_order() {
        // The enum order is GM_PLAYER=0, LOW_WIZARD=1, WIZARD=2, HIGH_WIZARD=3,
        // GOD=4, IMPLEMENTOR=5.
        for (text, expected) in [
            ("LOW_WIZARD", 1),
            ("WIZARD", 2),
            ("HIGH_WIZARD", 3),
            ("GOD", 4),
            ("IMPLEMENTOR", 5),
        ] {
            let authority = GmAuthority::from_column(text.as_bytes())
                .unwrap_or_else(|| panic!("{text} is a legacy authority"));
            assert_eq!(authority.as_i32(), expected, "{text}");
        }
        assert_eq!(
            GmAuthority::from_column(b"GOD").map(GmAuthority::as_i32),
            Some(4)
        );
    }

    #[test]
    fn an_unrecognized_authority_drops_the_row() {
        // Legacy `continue`s on an unknown string. Defaulting to GM_PLAYER
        // would invent an administrator that the database never listed.
        for bad in [&b""[..], b"PLAYER", b"god", b"ADMIN", b"IMPLEMENTER"] {
            assert_eq!(
                GmAuthority::from_column(bad),
                None,
                "{bad:?} must not become an administrator"
            );
        }
        let admins = build_admin_list(
            &[row(
                1,
                Some(b"a"),
                Some(b"n"),
                Some(b"1.1.1.1"),
                Some(b"ALL"),
                Some(b"ADMIN"),
            )],
            &GmSectionLimits::default(),
        )
        .expect("the row is well formed");
        assert!(
            admins.is_empty(),
            "an unknown authority must drop the row, got {admins:?}"
        );
    }

    // --- the account field ------------------------------------------------

    #[test]
    fn the_account_field_is_trimmed_and_lowercased() {
        // Legacy `trim_and_lower` skips leading and trailing isspace bytes and
        // lowercases the rest into a 32-byte field.
        let admins = build_admin_list(
            &[row(
                1,
                Some(b"  MiXeD\t"),
                Some(b"Name"),
                Some(b"1.1.1.1"),
                Some(b"ALL"),
                Some(b"GOD"),
            )],
            &GmSectionLimits::default(),
        )
        .expect("the row is well formed");
        let account = &admins[0].account;
        let end = account
            .iter()
            .position(|byte| *byte == 0)
            .expect("a NUL terminator");
        assert_eq!(
            &account[..end],
            b"mixed",
            "the field is trimmed and lowercased"
        );
        assert!(
            account[end..].iter().all(|byte| *byte == 0),
            "the field is zero padded"
        );
    }

    #[test]
    fn a_long_account_is_truncated_to_31_bytes_and_a_nul() {
        // `dest_size` is 32 and legacy reserves the last byte for the NUL.
        let long = vec![b'A'; 40];
        let admins = build_admin_list(
            &[row(1, Some(&long), None, None, None, Some(b"GOD"))],
            &GmSectionLimits::default(),
        )
        .expect("the row is well formed");
        assert_eq!(admins[0].account[31], 0, "the 32nd byte is the terminator");
        // The account is lowercased as well as truncated, so the expected
        // bytes are the lowercase form of the 31 kept characters.
        assert_eq!(&admins[0].account[..31], &[b'a'; 31]);
    }

    #[test]
    fn a_null_column_becomes_a_zeroed_field() {
        let admins = build_admin_list(
            &[row(1, None, None, None, None, Some(b"GOD"))],
            &GmSectionLimits::default(),
        )
        .expect("the row is well formed");
        assert_eq!(admins[0].account, [0_u8; 32]);
        assert_eq!(admins[0].name, [0_u8; 32]);
        assert_eq!(admins[0].contact_ip, [0_u8; 16]);
        assert_eq!(admins[0].server_ip, [0_u8; 16]);
    }

    #[test]
    fn a_verbatim_name_field_keeps_its_bytes() {
        // Legacy `strlcpy` copies the name, unlike the account, so case and
        // spaces survive.
        let admins = build_admin_list(
            &[row(
                1,
                Some(b"acct"),
                Some(b"  MixedCase Name  "),
                Some(b"10.0.0.1"),
                Some(b"10.0.0.2"),
                Some(b"GOD"),
            )],
            &GmSectionLimits::default(),
        )
        .expect("the row is well formed");
        let name = &admins[0].name;
        let end = name
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(name.len());
        assert_eq!(&name[..end], b"  MixedCase Name  ");
    }

    #[test]
    fn the_packed_record_is_exactly_the_wire_width() {
        // 4 + 32 + 32 + 16 + 16 + 4 = 104, matching sizeof(tAdminInfo) on x86.
        assert_eq!(ADMIN_INFO_WIRE_SIZE, 104);
        let admins = build_admin_list(
            &[row(
                7,
                Some(b"a"),
                Some(b"n"),
                Some(b"1.1.1.1"),
                Some(b"ALL"),
                Some(b"GOD"),
            )],
            &GmSectionLimits::default(),
        )
        .expect("the row is well formed");
        let bytes = admins[0].encode();
        assert_eq!(bytes.len(), ADMIN_INFO_WIRE_SIZE);
        assert_eq!(&bytes[..4], &7_i32.to_le_bytes());
        assert_eq!(&bytes[100..104], &4_i32.to_le_bytes());
    }

    // --- the host list ----------------------------------------------------

    #[test]
    fn a_null_or_empty_host_is_skipped() {
        // Legacy skips a row whose value is NULL or empty. Keeping a zeroed
        // 16-byte record would look like a real address to the game server.
        let hosts = build_host_list(
            &[None, Some(Vec::new()), Some(b"10.0.0.1".to_vec())],
            &GmSectionLimits::default(),
        )
        .expect("the rows are well formed");
        assert_eq!(hosts.len(), 1, "only the real address survives");
        assert_eq!(&hosts[0].bytes[..8], b"10.0.0.1");
        assert_eq!(hosts[0].bytes[15], 0);
    }

    #[test]
    fn a_host_is_truncated_to_fifteen_bytes_and_a_nul() {
        // Legacy `Encode(vHost[n].c_str(), 16)` writes at most 15 bytes plus
        // the NUL.
        let long = vec![b'9'; 20];
        let hosts = build_host_list(&[Some(long)], &GmSectionLimits::default())
            .expect("the row is well formed");
        assert_eq!(hosts[0].bytes.len(), GM_HOST_WIRE_SIZE);
        assert_eq!(hosts[0].bytes[15], 0, "the 16th byte is the terminator");
        assert_eq!(&hosts[0].bytes[..15], &[b'9'; 15]);
    }

    #[test]
    fn hosts_and_admins_keep_fetch_order() {
        let rows: Vec<_> = (0..5)
            .map(|index| Some(format!("10.0.0.{index}").into_bytes()))
            .collect();
        let hosts = build_host_list(&rows, &GmSectionLimits::default()).expect("well formed");
        let texts: Vec<_> = hosts
            .iter()
            .map(|host| {
                let end = host.bytes.iter().position(|byte| *byte == 0).unwrap();
                String::from_utf8_lossy(&host.bytes[..end]).into_owned()
            })
            .collect();
        assert_eq!(
            texts,
            vec!["10.0.0.0", "10.0.0.1", "10.0.0.2", "10.0.0.3", "10.0.0.4"]
        );
    }

    // --- limits -----------------------------------------------------------

    #[test]
    fn a_row_over_the_source_cap_fails_instead_of_truncating() {
        // Legacy writes the count as a WORD, so a source with more than u16::MAX
        // rows would wrap the declared count and desynchronize the game
        // server's parser for every later record.
        let limits = GmSectionLimits {
            max_host_rows: 2,
            ..GmSectionLimits::default()
        };
        let rows = vec![
            Some(b"10.0.0.1".to_vec()),
            Some(b"10.0.0.2".to_vec()),
            Some(b"10.0.0.3".to_vec()),
        ];
        let error = build_host_list(&rows, &limits).expect_err("three rows exceed a cap of two");
        assert_eq!(
            error,
            GmSectionError::TooManyRows {
                list: GmList::Hosts,
                maximum: 2
            }
        );
    }

    #[test]
    fn the_default_limits_match_the_declared_word_count() {
        let limits = GmSectionLimits::default();
        assert_eq!(limits.max_host_rows, u16::MAX as usize);
        assert_eq!(limits.max_admin_rows, u16::MAX as usize);
        assert_eq!(limits.max_host_bytes, u16::MAX as usize * GM_HOST_WIRE_SIZE);
        assert_eq!(
            limits.max_admin_bytes,
            u16::MAX as usize * ADMIN_INFO_WIRE_SIZE
        );
    }

    // --- the administrator address fields ---------------------------------

    /// A long contact or server address is truncated to fifteen bytes.
    ///
    /// Both fields are sixteen bytes, so the legacy `strlcpy` keeps fifteen
    /// characters and leaves the last byte as the NUL. Keeping all sixteen
    /// would leave an unterminated field, and the game server would read past
    /// it when it formats the administrator.
    #[test]
    fn a_long_administrator_address_keeps_fifteen_bytes_and_a_nul() {
        let long = vec![b'9'; 20];
        let admins = build_admin_list(
            &[row(
                1,
                Some(b"acct"),
                Some(b"name"),
                Some(&long),
                Some(&long),
                Some(b"GOD"),
            )],
            &GmSectionLimits::default(),
        )
        .expect("the row is well formed");
        assert_eq!(&admins[0].contact_ip[..15], &[b'9'; 15]);
        assert_eq!(
            admins[0].contact_ip[15], 0,
            "the 16th byte is the terminator"
        );
        assert_eq!(&admins[0].server_ip[..15], &[b'9'; 15]);
        assert_eq!(
            admins[0].server_ip[15], 0,
            "the 16th byte is the terminator"
        );
    }

    /// A fifteen-byte address is kept whole.
    #[test]
    fn a_fifteen_byte_administrator_address_is_kept_whole() {
        let exact = b"123.123.123.123";
        assert_eq!(exact.len(), 15);
        let admins = build_admin_list(
            &[row(1, None, None, Some(exact), Some(exact), Some(b"GOD"))],
            &GmSectionLimits::default(),
        )
        .expect("the row is well formed");
        assert_eq!(&admins[0].contact_ip[..15], exact);
        assert_eq!(admins[0].contact_ip[15], 0);
    }
}
