//! The safebox password of an account (ADR-0005).
//!
//! Legacy keeps it in plain text in `safebox.password` and compares it in the DB process
//! (`db/ClientManager.cpp:706-766`, `:1078-1130`). The store keeps an argon2id hash of it
//! (ADR-0003), and an account without a row opens with [`DEFAULT_PASSWORD`], which is what
//! legacy answers for an account without one. The items of the safebox and the mall are
//! [`crate::items`] rows that the account holds.

use std::error::Error;
use std::fmt;

use common::tables::SAFEBOX_PASSWORD_MAX_LEN;

use crate::accounts::AccountId;
use crate::credentials::{hash_bytes, CredentialError, PasswordDigest};
use crate::store::Store;

/// The password of an account without a `safebox` row (`db/ClientManager.cpp:746-753`).
pub const DEFAULT_PASSWORD: &[u8] = b"000000";

/// A safebox password as the client typed it: 1 to [`SAFEBOX_PASSWORD_MAX_LEN`] bytes.
///
/// Legacy refuses an empty one and a longer one before it asks the DB process
/// (`ReqSafeboxLoad` and `do_safebox_change_password`, `[LS;526]`), and the column it
/// compares against is six bytes wide.
#[derive(Clone, PartialEq, Eq)]
pub struct SafeboxPassword(Vec<u8>);

impl SafeboxPassword {
    /// Check a typed password.
    ///
    /// # Errors
    ///
    /// Returns [`SafeboxError::PasswordLength`].
    pub fn new(raw: &[u8]) -> Result<Self, SafeboxError> {
        if raw.is_empty() || raw.len() > SAFEBOX_PASSWORD_MAX_LEN {
            return Err(SafeboxError::PasswordLength(raw.len()));
        }
        Ok(Self(raw.to_vec()))
    }

    /// Whether this is [`DEFAULT_PASSWORD`].
    fn is_default(&self) -> bool {
        self.0 == DEFAULT_PASSWORD
    }
}

impl fmt::Debug for SafeboxPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SafeboxPassword(***)")
    }
}

/// Why a safebox password operation failed.
#[derive(Debug)]
pub enum SafeboxError {
    /// The password is empty or longer than [`SAFEBOX_PASSWORD_MAX_LEN`] bytes.
    PasswordLength(usize),
    /// A hash could not be parsed or computed.
    Credential(CredentialError),
    /// No account has the ID.
    NoSuchAccount(AccountId),
    /// The server refused or failed a query.
    Database(sqlx::Error),
}

impl fmt::Display for SafeboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PasswordLength(length) => write!(
                f,
                "a safebox password is 1 to {SAFEBOX_PASSWORD_MAX_LEN} bytes, not {length}"
            ),
            Self::Credential(error) => error.fmt(f),
            Self::NoSuchAccount(id) => write!(f, "no account has id {id}"),
            Self::Database(error) => write!(f, "PostgreSQL error: {error}"),
        }
    }
}

impl Error for SafeboxError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Credential(error) => Some(error),
            Self::Database(error) => Some(error),
            Self::PasswordLength(_) | Self::NoSuchAccount(_) => None,
        }
    }
}

impl From<CredentialError> for SafeboxError {
    fn from(error: CredentialError) -> Self {
        Self::Credential(error)
    }
}

impl From<sqlx::Error> for SafeboxError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

/// Whether `candidate` opens the account's safebox.
///
/// Legacy compares exactly (`strcmp`, `db/ClientManager.cpp:760-766`), and an account without
/// a row opens only with [`DEFAULT_PASSWORD`]. The hash is checked off the async threads.
///
/// # Errors
///
/// Returns [`SafeboxError::NoSuchAccount`] for an ID no account column can hold,
/// [`SafeboxError::Credential`] for a stored hash that cannot be parsed, or
/// [`SafeboxError::Database`].
pub async fn verify_password(
    store: &Store,
    account: AccountId,
    candidate: &SafeboxPassword,
) -> Result<bool, SafeboxError> {
    let column = to_column(account)?;
    let stored: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM safebox WHERE account_id = $1")
            .bind(column)
            .fetch_optional(store.pool())
            .await?;
    match stored {
        None => Ok(candidate.is_default()),
        Some(phc) => verify(phc, candidate).await,
    }
}

/// Change the account's safebox password: `true` when `old` was its password and `new` now
/// is, `false` when `old` was wrong and nothing changed.
///
/// The row is read `FOR UPDATE` and written in the same transaction, so two changes cannot
/// both pass the check against one password. An account without a row has
/// [`DEFAULT_PASSWORD`], and a change creates its row; when another change created it first,
/// this one's `old` was checked against a password the account no longer has, so it
/// answers `false`. Two Divergences (ADR-0005): legacy answers 0 for an account without a
/// row, which only a size change creates, and it compares `old` without case (`strcasecmp`,
/// `db/ClientManager.cpp:1101`), which a hash cannot do.
///
/// # Errors
///
/// Returns [`SafeboxError::NoSuchAccount`] when no account has the ID,
/// [`SafeboxError::Credential`] when a hash cannot be parsed or computed, or
/// [`SafeboxError::Database`].
pub async fn change_password(
    store: &Store,
    account: AccountId,
    old: &SafeboxPassword,
    new: &SafeboxPassword,
) -> Result<bool, SafeboxError> {
    let column = to_column(account)?;
    let mut transaction = store.pool().begin().await?;
    let stored: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM safebox WHERE account_id = $1 FOR UPDATE")
            .bind(column)
            .fetch_optional(&mut *transaction)
            .await?;
    let matches = match &stored {
        None => old.is_default(),
        Some(phc) => verify(phc.clone(), old).await?,
    };
    if !matches {
        transaction.rollback().await?;
        return Ok(false);
    }
    let digest = hash(new).await?;
    let written = if stored.is_none() {
        sqlx::query(
            "INSERT INTO safebox (account_id, password_hash) VALUES ($1, $2) \
             ON CONFLICT (account_id) DO NOTHING",
        )
        .bind(column)
        .bind(digest.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(|error| classify(error, account))?
        .rows_affected()
    } else {
        sqlx::query("UPDATE safebox SET password_hash = $2 WHERE account_id = $1")
            .bind(column)
            .bind(digest.as_str())
            .execute(&mut *transaction)
            .await?
            .rows_affected()
    };
    transaction.commit().await?;
    Ok(written > 0)
}

/// The account ID as the `integer` column holds it.
fn to_column(account: AccountId) -> Result<i32, SafeboxError> {
    i32::try_from(account.get()).map_err(|_| SafeboxError::NoSuchAccount(account))
}

/// A foreign-key violation on the insert is an account that does not exist.
fn classify(error: sqlx::Error, account: AccountId) -> SafeboxError {
    if let sqlx::Error::Database(database) = &error {
        if database.code().as_deref() == Some("23503") {
            return SafeboxError::NoSuchAccount(account);
        }
    }
    SafeboxError::Database(error)
}

/// Check `candidate` against a stored PHC string on a blocking thread.
async fn verify(phc: String, candidate: &SafeboxPassword) -> Result<bool, SafeboxError> {
    let candidate = candidate.0.clone();
    let digest = PasswordDigest::from_stored(phc);
    tokio::task::spawn_blocking(move || digest.verify(&candidate))
        .await
        .map_err(|error| CredentialError::Hash(error.to_string()))?
        .map_err(SafeboxError::from)
}

/// Hash a new password on a blocking thread.
async fn hash(password: &SafeboxPassword) -> Result<PasswordDigest, SafeboxError> {
    let raw = password.0.clone();
    tokio::task::spawn_blocking(move || hash_bytes(&raw))
        .await
        .map_err(|error| CredentialError::Hash(error.to_string()))?
        .map_err(SafeboxError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_is_one_to_six_bytes() {
        assert!(matches!(
            SafeboxPassword::new(b""),
            Err(SafeboxError::PasswordLength(0))
        ));
        assert!(SafeboxPassword::new(b"a").is_ok());
        assert!(SafeboxPassword::new(b"abcdef").is_ok());
        assert!(matches!(
            SafeboxPassword::new(b"abcdefg"),
            Err(SafeboxError::PasswordLength(7))
        ));
    }

    #[test]
    fn only_the_six_zeroes_are_the_default() {
        let default = SafeboxPassword::new(b"000000").expect("six bytes");
        assert!(default.is_default());
        for other in [&b"00000"[..], &b"000001"[..], &b"00000 "[..], &b"0"[..]] {
            let password = SafeboxPassword::new(other).expect("a password");
            assert!(!password.is_default(), "{other:?}");
        }
    }

    #[test]
    fn a_password_never_reaches_a_log() {
        let password = SafeboxPassword::new(b"hunter").expect("six bytes");
        assert_eq!(format!("{password:?}"), "SafeboxPassword(***)");
    }

    #[test]
    fn a_hash_verifies_its_password_and_no_other() {
        let digest = hash_bytes(b"s3cret").expect("hash");
        assert!(digest.as_str().starts_with("$argon2id$"));
        assert!(digest.verify(b"s3cret").expect("verify"));
        assert!(!digest.verify(b"S3CRET").expect("verify"));
    }
}
