//! Accounts, GM grants, and the two account currencies.
//!
//! These are the writes the Operator makes from outside the game (ADR-0003), plus the one GM
//! lookup the game will make. Every value reaches SQL as a bound parameter.

use std::error::Error;
use std::fmt;

use common::gm::GmAuthority;
use sqlx::Row;

use crate::credentials::{CredentialError, DeleteCode, Login, PasswordDigest};
use crate::store::Store;

/// Shortest character Name (`check_name_alphabet`, `locale_service.cpp:324`).
pub const NAME_MIN_LEN: usize = 2;

/// Longest character Name (`CHARACTER_NAME_MAX_LEN`, `common/length.h:15`).
pub const NAME_MAX_LEN: usize = 24;

/// An account ID. It reaches the client and the game as a 32-bit value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId(u32);

impl AccountId {
    /// The ID.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    fn from_column(id: i32) -> Result<Self, AccountError> {
        u32::try_from(id)
            .map(Self)
            .map_err(|_| AccountError::Corrupt(format!("account id {id} is negative")))
    }

    fn to_column(self) -> Result<i32, AccountError> {
        i32::try_from(self.0).map_err(|_| AccountError::NoSuchAccountId(self))
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The shape of a character Name: 2 to 24 ASCII letters and digits.
///
/// Only the shape is checked. The banned-word and monster-name checks belong to character
/// creation, and a GM grant may name a character that does not exist yet.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Name(String);

impl Name {
    /// Check a Name.
    ///
    /// # Errors
    ///
    /// Returns [`AccountError::InvalidName`].
    pub fn new(raw: &str) -> Result<Self, AccountError> {
        let length_ok = (NAME_MIN_LEN..=NAME_MAX_LEN).contains(&raw.len());
        if !length_ok || !raw.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(AccountError::InvalidName(raw.to_owned()));
        }
        Ok(Self(raw.to_owned()))
    }

    /// The Name as it was given.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An account currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Currency {
    /// Coins: the item-shop currency (legacy `coins`, a `long long`).
    Coins,
    /// Cash: the daily-gift currency (legacy `cash`, sent to the client as a `DWORD`).
    Cash,
}

impl Currency {
    /// The largest balance the currency may hold.
    #[must_use]
    pub const fn max(self) -> i64 {
        match self {
            Self::Coins => i64::MAX,
            Self::Cash => u32::MAX as i64,
        }
    }

    const fn select_for_update(self) -> &'static str {
        match self {
            Self::Coins => "SELECT coins FROM account WHERE login = $1 FOR UPDATE",
            Self::Cash => "SELECT cash FROM account WHERE login = $1 FOR UPDATE",
        }
    }

    const fn update(self) -> &'static str {
        match self {
            Self::Coins => "UPDATE account SET coins = $2 WHERE login = $1",
            Self::Cash => "UPDATE account SET cash = $2 WHERE login = $1",
        }
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Coins => "coins",
            Self::Cash => "cash",
        })
    }
}

/// A new account, ready to store.
#[derive(Debug, Clone)]
pub struct NewAccount {
    /// The login.
    pub login: Login,
    /// The hashed password.
    pub password: PasswordDigest,
    /// The character delete code.
    pub delete_code: DeleteCode,
}

/// One GM grant, as [`list_gm_grants`] reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmGrant {
    /// The account the Name must belong to.
    pub login: String,
    /// The character Name.
    pub name: String,
    /// The authority.
    pub authority: GmAuthority,
}

/// Why an account operation failed.
#[derive(Debug)]
pub enum AccountError {
    /// A credential was refused.
    Credential(CredentialError),
    /// The Name is not 2 to 24 ASCII letters and digits.
    InvalidName(String),
    /// Another account already has the login.
    LoginTaken(Login),
    /// No account has the login.
    NoSuchAccount(Login),
    /// No account has the ID.
    NoSuchAccountId(AccountId),
    /// The Name is granted to a different account. Revoke it first.
    NameGrantedElsewhere {
        /// The Name.
        name: Name,
        /// The account that holds the grant.
        login: String,
    },
    /// The Name has no grant.
    NoSuchGrant(Name),
    /// The change would take a balance below zero or above [`Currency::max`].
    BalanceOutOfRange {
        /// The currency.
        currency: Currency,
        /// The balance before the change.
        balance: i64,
        /// The requested change.
        delta: i64,
    },
    /// A stored value breaks a rule the schema should have enforced.
    Corrupt(String),
    /// The server refused or failed a query.
    Database(sqlx::Error),
}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Credential(error) => error.fmt(f),
            Self::InvalidName(name) => write!(
                f,
                "{name:?} is not a Name: {NAME_MIN_LEN} to {NAME_MAX_LEN} ASCII letters and digits"
            ),
            Self::LoginTaken(login) => write!(f, "an account with login {login} already exists"),
            Self::NoSuchAccount(login) => write!(f, "no account has login {login}"),
            Self::NoSuchAccountId(id) => write!(f, "no account has id {id}"),
            Self::NameGrantedElsewhere { name, login } => write!(
                f,
                "{name} is already granted to account {login}; revoke that grant first"
            ),
            Self::NoSuchGrant(name) => write!(f, "{name} has no GM grant"),
            Self::BalanceOutOfRange {
                currency,
                balance,
                delta,
            } => write!(
                f,
                "{currency} would leave 0..={}: balance {balance}, change {delta}",
                currency.max()
            ),
            Self::Corrupt(detail) => write!(f, "stored data is invalid: {detail}"),
            Self::Database(error) => write!(f, "PostgreSQL error: {error}"),
        }
    }
}

impl Error for AccountError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Credential(error) => Some(error),
            Self::Database(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CredentialError> for AccountError {
    fn from(error: CredentialError) -> Self {
        Self::Credential(error)
    }
}

impl From<sqlx::Error> for AccountError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

/// Create an account with no GM grant and empty balances.
///
/// # Errors
///
/// Returns [`AccountError::LoginTaken`] or [`AccountError::Database`].
pub async fn create_account(
    store: &Store,
    account: &NewAccount,
) -> Result<AccountId, AccountError> {
    let id: Option<i32> = sqlx::query_scalar(
        "INSERT INTO account (login, password_hash, delete_code) VALUES ($1, $2, $3) \
         ON CONFLICT (login) DO NOTHING RETURNING id",
    )
    .bind(account.login.as_str())
    .bind(account.password.as_str())
    .bind(account.delete_code.as_str())
    .fetch_optional(store.pool())
    .await?;
    let id = id.ok_or_else(|| AccountError::LoginTaken(account.login.clone()))?;
    AccountId::from_column(id)
}

/// Replace an account's password.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchAccount`] or [`AccountError::Database`].
pub async fn set_password(
    store: &Store,
    login: &Login,
    password: &PasswordDigest,
) -> Result<(), AccountError> {
    let updated = sqlx::query("UPDATE account SET password_hash = $2 WHERE login = $1")
        .bind(login.as_str())
        .bind(password.as_str())
        .execute(store.pool())
        .await?
        .rows_affected();
    if updated == 0 {
        return Err(AccountError::NoSuchAccount(login.clone()));
    }
    Ok(())
}

/// The account ID and stored password of a login, for the auth path.
///
/// # Errors
///
/// Returns [`AccountError::Corrupt`] for a negative ID, or [`AccountError::Database`].
pub async fn find_credentials(
    store: &Store,
    login: &Login,
) -> Result<Option<(AccountId, PasswordDigest)>, AccountError> {
    let row = sqlx::query("SELECT id, password_hash FROM account WHERE login = $1")
        .bind(login.as_str())
        .fetch_optional(store.pool())
        .await?;
    row.map(|row| {
        let id = AccountId::from_column(row.try_get("id")?)?;
        let hash: String = row.try_get("password_hash")?;
        Ok((id, PasswordDigest::from_stored(hash)))
    })
    .transpose()
}

/// Add `delta` to a balance, which may be negative, and return the new balance.
///
/// The row is locked for the change, so concurrent changes add up.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchAccount`], [`AccountError::BalanceOutOfRange`] without changing
/// anything, or [`AccountError::Database`].
pub async fn adjust_balance(
    store: &Store,
    login: &Login,
    currency: Currency,
    delta: i64,
) -> Result<i64, AccountError> {
    let mut transaction = store.pool().begin().await?;
    let balance: i64 = sqlx::query_scalar(currency.select_for_update())
        .bind(login.as_str())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| AccountError::NoSuchAccount(login.clone()))?;
    let out_of_range = AccountError::BalanceOutOfRange {
        currency,
        balance,
        delta,
    };
    let updated = balance
        .checked_add(delta)
        .filter(|updated| (0..=currency.max()).contains(updated))
        .ok_or(out_of_range)?;
    sqlx::query(currency.update())
        .bind(login.as_str())
        .bind(updated)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(updated)
}

/// Grant `authority` to the character `name` of the account `login`.
///
/// Granting a Name the same account already holds changes its authority. A Name granted to
/// another account is refused, because legacy would then refuse the character anyway: it checks
/// that the grant's account is the character's (`gm_new_get_level`).
///
/// # Errors
///
/// Returns [`AccountError::NoSuchAccount`], [`AccountError::NameGrantedElsewhere`], or
/// [`AccountError::Database`].
pub async fn grant_gm(
    store: &Store,
    login: &Login,
    name: &Name,
    authority: GmAuthority,
) -> Result<(), AccountError> {
    let mut transaction = store.pool().begin().await?;
    let account_id: i32 = sqlx::query_scalar("SELECT id FROM account WHERE login = $1")
        .bind(login.as_str())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| AccountError::NoSuchAccount(login.clone()))?;
    let granted = sqlx::query(
        "INSERT INTO gm_grant (account_id, name, authority) VALUES ($1, $2, $3) \
         ON CONFLICT ((lower(name))) DO UPDATE \
         SET name = EXCLUDED.name, authority = EXCLUDED.authority, granted_at = now() \
         WHERE gm_grant.account_id = EXCLUDED.account_id",
    )
    .bind(account_id)
    .bind(name.as_str())
    .bind(authority.column())
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if granted == 0 {
        let holder: String = sqlx::query_scalar(
            "SELECT a.login FROM gm_grant g JOIN account a ON a.id = g.account_id \
             WHERE lower(g.name) = lower($1)",
        )
        .bind(name.as_str())
        .fetch_one(&mut *transaction)
        .await?;
        return Err(AccountError::NameGrantedElsewhere {
            name: name.clone(),
            login: holder,
        });
    }
    transaction.commit().await?;
    Ok(())
}

/// Remove the grant of `name`.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchGrant`] or [`AccountError::Database`].
pub async fn revoke_gm(store: &Store, name: &Name) -> Result<(), AccountError> {
    let removed = sqlx::query("DELETE FROM gm_grant WHERE lower(name) = lower($1)")
        .bind(name.as_str())
        .execute(store.pool())
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(AccountError::NoSuchGrant(name.clone()));
    }
    Ok(())
}

/// Every GM grant, ordered by Name regardless of case.
///
/// # Errors
///
/// Returns [`AccountError::Corrupt`] for an authority the schema should have refused, or
/// [`AccountError::Database`].
pub async fn list_gm_grants(store: &Store) -> Result<Vec<GmGrant>, AccountError> {
    let rows = sqlx::query(
        "SELECT a.login, g.name, g.authority FROM gm_grant g \
         JOIN account a ON a.id = g.account_id ORDER BY lower(g.name)",
    )
    .fetch_all(store.pool())
    .await?;
    rows.iter()
        .map(|row| {
            let authority: String = row.try_get("authority")?;
            Ok(GmGrant {
                login: row.try_get("login")?,
                name: row.try_get("name")?,
                authority: parse_authority(&authority)?,
            })
        })
        .collect()
}

/// The authority of the character `name` on account `account`, or `None` for a player.
///
/// This is legacy `gm_new_get_level` with its host check off, which is the legacy default
/// (`g_bGMHostCheck = false`, `config.cpp:45`): the grant must exist for the Name, and its
/// account must be the character's. The Name matches regardless of case, because Names are
/// unique regardless of case; legacy compared it exactly.
///
/// # Errors
///
/// Returns [`AccountError::Corrupt`] or [`AccountError::Database`].
pub async fn gm_authority(
    store: &Store,
    account: AccountId,
    name: &str,
) -> Result<Option<GmAuthority>, AccountError> {
    let authority: Option<String> = sqlx::query_scalar(
        "SELECT authority FROM gm_grant WHERE lower(name) = lower($1) AND account_id = $2",
    )
    .bind(name)
    .bind(account.to_column()?)
    .fetch_optional(store.pool())
    .await?;
    authority.as_deref().map(parse_authority).transpose()
}

fn parse_authority(column: &str) -> Result<GmAuthority, AccountError> {
    GmAuthority::from_column(column.as_bytes())
        .ok_or_else(|| AccountError::Corrupt(format!("unknown GM authority {column:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_two_to_twenty_four_letters_and_digits_in_any_case() {
        assert_eq!(Name::new("Admin").unwrap().as_str(), "Admin");
        assert!(Name::new(&"a".repeat(24)).is_ok());
        for bad in ["", "a", &"a".repeat(25), "Ad min", "Ad_min", "Admín"] {
            assert!(
                matches!(Name::new(bad), Err(AccountError::InvalidName(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn cash_is_bounded_by_the_client_dword_and_coins_by_the_column() {
        assert_eq!(Currency::Cash.max(), 4_294_967_295);
        assert_eq!(Currency::Coins.max(), i64::MAX);
    }

    #[test]
    fn a_balance_error_names_the_currency_and_the_range() {
        let error = AccountError::BalanceOutOfRange {
            currency: Currency::Cash,
            balance: 5,
            delta: -6,
        };
        assert_eq!(
            error.to_string(),
            "cash would leave 0..=4294967295: balance 5, change -6"
        );
    }

    #[test]
    fn an_account_id_above_the_identity_range_matches_no_row() {
        let id = AccountId(u32::MAX);
        assert!(matches!(
            id.to_column(),
            Err(AccountError::NoSuchAccountId(missing)) if missing == id
        ));
        assert_eq!(AccountId(7).to_column().unwrap(), 7);
        assert!(matches!(
            AccountId::from_column(-1),
            Err(AccountError::Corrupt(_))
        ));
    }
}
