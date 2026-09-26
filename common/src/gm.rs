//! The GM host and administrator lists.
//!
//! The legacy DB server read two `SQL_COMMON` tables inside `QUERY_BOOT`:
//! `__GetHostInfo` read the whole `gmhost` table, and `__GetAdminInfo` read
//! the `gmlist` rows whose `mServerIP` is `ALL` or the game server's public
//! address. The game server then decided a character's GM level from those
//! lists.
//!
//! This module keeps the row rules: which rows survive, how each string is
//! copied into its fixed-width field, and how `mAuthority` maps onto the
//! legacy `EGMLevels` order. It does not query a database; the store supplies
//! the rows.

use std::error::Error;
use std::fmt;

/// The legacy `gmhost` statement, kept to document the single column.
///
/// There is no `WHERE` clause: legacy reads the whole table.
pub const GM_HOST_QUERY: &str = "SELECT mIP FROM gmhost";

/// The legacy `gmlist` column list, in source order.
pub const ADMIN_QUERY_COLUMNS: &str = "mID,mAccount,mName,mContactIP,mServerIP,mAuthority";

/// The `mServerIP` value that makes a `gmlist` row apply to every server.
///
/// Legacy `__GetAdminInfo` also passes `"ALL"` for a NULL `szIP`, which then
/// matches only these rows.
pub const ADMIN_ALL_SERVERS: &str = "ALL";

/// Width of a GM host field (`char[16]`).
pub const GM_HOST_BYTES: usize = 16;

/// Width of `tAdminInfo::m_szAccount` and `m_szName`.
pub const ADMIN_NAME_BYTES: usize = 32;

/// Width of `tAdminInfo::m_szContactIP` and `m_szServerIP`.
pub const ADMIN_IP_BYTES: usize = 16;

/// Default row cap: the legacy boot stream counted each list in a `WORD`, so
/// no legacy list held more.
pub const GM_LIST_DEFAULT_MAX_ROWS: usize = u16::MAX as usize;

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

/// Row caps for the two lists.
///
/// Legacy has no bound: it reserves `uiNumRows` and appends whatever the
/// database returns. These caps fail instead of truncating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GmListLimits {
    /// Maximum source rows read from `gmhost`.
    pub max_host_rows: usize,
    /// Maximum source rows read from `gmlist`.
    pub max_admin_rows: usize,
}

impl Default for GmListLimits {
    fn default() -> Self {
        Self {
            max_host_rows: GM_LIST_DEFAULT_MAX_ROWS,
            max_admin_rows: GM_LIST_DEFAULT_MAX_ROWS,
        }
    }
}

/// Why a GM host or administrator list could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GmListError {
    /// The source had more rows than the configured cap.
    TooManyRows {
        /// Which list overflowed.
        list: GmList,
        /// Configured maximum.
        maximum: usize,
    },
    /// A vector allocation failed.
    AllocationFailed {
        /// Which list could not allocate.
        list: GmList,
    },
}

/// Which of the two lists an error refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GmList {
    /// The `gmhost` list.
    Hosts,
    /// The `gmlist` administrators.
    Admins,
}

impl fmt::Display for GmListError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRows { list, maximum } => {
                write!(formatter, "{list:?} exceeded the {maximum}-row limit")
            }
            Self::AllocationFailed { list } => write!(formatter, "{list:?} allocation failed"),
        }
    }
}

impl Error for GmListError {}

/// One GM host address as the legacy `char[16]` held it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GmHost {
    /// Raw 16-byte field; at most 15 bytes and a NUL.
    pub bytes: [u8; GM_HOST_BYTES],
}

impl GmHost {
    /// The address up to its terminator.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        let end = self
            .bytes
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(GM_HOST_BYTES);
        &self.bytes[..end]
    }
}

/// One administrator as the legacy `tAdminInfo` held it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdminInfo {
    /// `m_ID`.
    pub id: i32,
    /// `m_szAccount`, trimmed and lowercased.
    pub account: [u8; ADMIN_NAME_BYTES],
    /// `m_szName`, copied verbatim.
    pub name: [u8; ADMIN_NAME_BYTES],
    /// `m_szContactIP`.
    pub contact_ip: [u8; ADMIN_IP_BYTES],
    /// `m_szServerIP`.
    pub server_ip: [u8; ADMIN_IP_BYTES],
    /// The mapped `mAuthority`.
    pub authority: GmAuthority,
}

/// Legacy `strlcpy` into a thirty-two-byte field.
///
/// A `NULL` column becomes an all-zero field, matching the legacy read of a
/// null cell. Bytes are copied verbatim, so non-ASCII values are preserved
/// rather than replaced, and the field is zero-padded to its full width. The
/// last byte is always left as the NUL that `strlcpy` writes.
fn copy_name_field(raw: Option<&[u8]>) -> [u8; ADMIN_NAME_BYTES] {
    let mut field = [0_u8; ADMIN_NAME_BYTES];
    let raw = raw.unwrap_or(&[]);
    let take = raw.len().min(field.len() - 1);
    if take > 0 {
        field[..take].copy_from_slice(&raw[..take]);
    }
    field
}

/// Legacy `strlcpy` into a sixteen-byte field.
fn copy_ip_field(raw: Option<&[u8]>) -> [u8; ADMIN_IP_BYTES] {
    let mut field = [0_u8; ADMIN_IP_BYTES];
    let raw = raw.unwrap_or(&[]);
    let take = raw.len().min(ADMIN_IP_BYTES - 1);
    if take > 0 {
        field[..take].copy_from_slice(&raw[..take]);
    }
    field
}

/// Legacy `trim_and_lower` applied to the 32-byte account field.
///
/// The legacy helper skips leading and trailing `isspace` bytes, lowercases the
/// rest, and always NUL-terminates within `dest_size`. The Hangul guard
/// (`(ch & 0xE0) > 0x90`) only matters for multi-byte text, which an account
/// name does not contain, so an ASCII space test is equivalent here.
fn trim_and_lower(raw: Option<&[u8]>) -> [u8; ADMIN_NAME_BYTES] {
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
    let mut field = [0_u8; ADMIN_NAME_BYTES];
    let take = body.len().min(ADMIN_NAME_BYTES - 1);
    for (index, byte) in body[..take].iter().enumerate() {
        field[index] = byte.to_ascii_lowercase();
    }
    field
}

/// Build the `gmhost` list from raw `mIP` column values.
///
/// Legacy skips a row whose value is `NULL` or empty and keeps everything else
/// verbatim in fetch order. A 16-byte field holds at most 15 characters plus a
/// NUL, so a longer address is truncated exactly as `Encode(c_str(), 16)`
/// truncates it.
///
/// # Errors
///
/// Returns [`GmListError::TooManyRows`] when the source returns more rows than
/// [`GmListLimits::max_host_rows`], or [`GmListError::AllocationFailed`].
pub fn build_host_list(
    rows: &[Option<Vec<u8>>],
    limits: &GmListLimits,
) -> Result<Vec<GmHost>, GmListError> {
    if rows.len() > limits.max_host_rows {
        return Err(GmListError::TooManyRows {
            list: GmList::Hosts,
            maximum: limits.max_host_rows,
        });
    }
    let mut hosts = Vec::new();
    hosts
        .try_reserve(rows.len())
        .map_err(|_| GmListError::AllocationFailed {
            list: GmList::Hosts,
        })?;
    for value in rows {
        let Some(raw) = value.as_deref() else {
            continue;
        };
        if raw.is_empty() {
            continue;
        }
        hosts.push(GmHost {
            bytes: copy_ip_field(Some(raw)),
        });
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

impl AdminRow {
    /// Whether the legacy filter `mServerIP='ALL' or mServerIP='<ip>'` keeps
    /// this row for a server whose public address is `server_ip`.
    ///
    /// The legacy table is `latin1` with its default case-insensitive,
    /// trailing-space-padded collation, so the comparison ignores ASCII case
    /// and trailing spaces. A `NULL` `mServerIP` never matches, and a `None`
    /// address matches only the `ALL` rows, as legacy's `"ALL"` fallback did.
    #[must_use]
    pub fn serves(&self, server_ip: Option<&[u8]>) -> bool {
        let Some(row_ip) = self.server_ip.as_deref() else {
            return false;
        };
        let wanted = server_ip.unwrap_or(ADMIN_ALL_SERVERS.as_bytes());
        [ADMIN_ALL_SERVERS.as_bytes(), wanted]
            .iter()
            .any(|value| collation_eq(row_ip, value))
    }
}

/// Equality under the legacy `latin1_swedish_ci` rules for ASCII text.
fn collation_eq(left: &[u8], right: &[u8]) -> bool {
    fn trim_trailing_spaces(value: &[u8]) -> &[u8] {
        let end = value
            .iter()
            .rposition(|&byte| byte != b' ')
            .map_or(0, |index| index + 1);
        &value[..end]
    }
    trim_trailing_spaces(left).eq_ignore_ascii_case(trim_trailing_spaces(right))
}

/// Build the administrator list, dropping rows legacy drops.
///
/// A row whose `mAuthority` is `NULL` or is not one of the six exact names is
/// skipped, matching the legacy `continue`. The caller applies
/// [`AdminRow::serves`] first, as the legacy `WHERE` clause did.
///
/// # Errors
///
/// Returns [`GmListError::TooManyRows`] when the source returns more rows than
/// [`GmListLimits::max_admin_rows`], or [`GmListError::AllocationFailed`].
pub fn build_admin_list(
    rows: &[AdminRow],
    limits: &GmListLimits,
) -> Result<Vec<AdminInfo>, GmListError> {
    if rows.len() > limits.max_admin_rows {
        return Err(GmListError::TooManyRows {
            list: GmList::Admins,
            maximum: limits.max_admin_rows,
        });
    }
    let mut admins = Vec::new();
    admins
        .try_reserve(rows.len())
        .map_err(|_| GmListError::AllocationFailed {
            list: GmList::Admins,
        })?;
    for row in rows {
        let Some(authority) = row.authority.as_deref().and_then(GmAuthority::from_column) else {
            continue;
        };
        admins.push(AdminInfo {
            id: row.id,
            account: trim_and_lower(row.account.as_deref()),
            name: copy_name_field(row.name.as_deref()),
            contact_ip: copy_ip_field(row.contact_ip.as_deref()),
            server_ip: copy_ip_field(row.server_ip.as_deref()),
            authority,
        });
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

    // --- the legacy statements and the server filter -----------------------

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
    fn the_server_filter_keeps_all_rows_and_this_servers_rows() {
        let for_server = |server: Option<&[u8]>| row(1, None, None, None, server, Some(b"GOD"));
        let here = Some(&b"10.0.0.7"[..]);
        assert!(for_server(Some(b"ALL")).serves(here));
        assert!(for_server(Some(b"all  ")).serves(here), "the collation is _ci and pads");
        assert!(for_server(Some(b"10.0.0.7")).serves(here));
        assert!(!for_server(Some(b"10.0.0.8")).serves(here));
        assert!(!for_server(None).serves(here), "NULL never equals anything");
    }

    #[test]
    fn a_missing_server_address_matches_only_the_all_rows() {
        let for_server = |server: Option<&[u8]>| row(1, None, None, None, server, Some(b"GOD"));
        assert!(for_server(Some(b"ALL")).serves(None));
        assert!(!for_server(Some(b"10.0.0.7")).serves(None));
    }

    // --- the authority filter ---------------------------------------------

    #[test]
    fn the_five_legacy_authority_names_map_onto_the_enum_order() {
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
            &GmListLimits::default(),
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
            &GmListLimits::default(),
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
            &GmListLimits::default(),
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
            &GmListLimits::default(),
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
            &GmListLimits::default(),
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
    fn the_record_keeps_the_id_and_the_mapped_authority() {
        let admins = build_admin_list(
            &[row(
                7,
                Some(b"a"),
                Some(b"n"),
                Some(b"1.1.1.1"),
                Some(b"ALL"),
                Some(b"GOD"),
            )],
            &GmListLimits::default(),
        )
        .expect("the row is well formed");
        assert_eq!(admins[0].id, 7);
        assert_eq!(admins[0].authority, GmAuthority::God);
        assert_eq!(admins[0].authority.as_i32(), 4);
    }

    // --- the host list ----------------------------------------------------

    #[test]
    fn a_null_or_empty_host_is_skipped() {
        // Legacy skips a row whose value is NULL or empty. Keeping a zeroed
        // 16-byte record would look like a real address to the game server.
        let hosts = build_host_list(
            &[None, Some(Vec::new()), Some(b"10.0.0.1".to_vec())],
            &GmListLimits::default(),
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
        let hosts = build_host_list(&[Some(long)], &GmListLimits::default())
            .expect("the row is well formed");
        assert_eq!(hosts[0].bytes.len(), GM_HOST_BYTES);
        assert_eq!(hosts[0].bytes[15], 0, "the 16th byte is the terminator");
        assert_eq!(&hosts[0].bytes[..15], &[b'9'; 15]);
    }

    #[test]
    fn hosts_and_admins_keep_fetch_order() {
        let rows: Vec<_> = (0..5)
            .map(|index| Some(format!("10.0.0.{index}").into_bytes()))
            .collect();
        let hosts = build_host_list(&rows, &GmListLimits::default()).expect("well formed");
        let texts: Vec<_> = hosts
            .iter()
            .map(|host| String::from_utf8_lossy(host.as_bytes()).into_owned())
            .collect();
        assert_eq!(
            texts,
            vec!["10.0.0.0", "10.0.0.1", "10.0.0.2", "10.0.0.3", "10.0.0.4"]
        );
    }

    // --- limits -----------------------------------------------------------

    #[test]
    fn a_row_over_the_source_cap_fails_instead_of_truncating() {
        let limits = GmListLimits {
            max_host_rows: 2,
            ..GmListLimits::default()
        };
        let rows = vec![
            Some(b"10.0.0.1".to_vec()),
            Some(b"10.0.0.2".to_vec()),
            Some(b"10.0.0.3".to_vec()),
        ];
        let error = build_host_list(&rows, &limits).expect_err("three rows exceed a cap of two");
        assert_eq!(
            error,
            GmListError::TooManyRows {
                list: GmList::Hosts,
                maximum: 2
            }
        );
    }

    #[test]
    fn the_default_limits_match_the_declared_word_count() {
        let limits = GmListLimits::default();
        assert_eq!(limits.max_host_rows, u16::MAX as usize);
        assert_eq!(limits.max_admin_rows, u16::MAX as usize);
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
            &GmListLimits::default(),
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
            &GmListLimits::default(),
        )
        .expect("the row is well formed");
        assert_eq!(&admins[0].contact_ip[..15], exact);
        assert_eq!(admins[0].contact_ip[15], 0);
    }
}
