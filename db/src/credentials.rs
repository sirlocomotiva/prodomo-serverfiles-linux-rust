//! The account credentials: the login, the password, and the delete code.
//!
//! Each type holds only a value the legacy auth and delete paths could accept, so the store never
//! holds an account the Reference client cannot log in to. Passwords are hashed with argon2id
//! (ADR-0003); nothing else in the store sees a password.

use std::error::Error;
use std::fmt;

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;

/// Shortest login legacy accepts (`FN_IS_VALID_LOGIN_STRING`, `input_auth.cpp:21`).
pub const LOGIN_MIN_LEN: usize = 2;

/// Longest login the client can send (`LOGIN_MAX_LEN`, `common/length.h:11`).
pub const LOGIN_MAX_LEN: usize = 30;

/// Longest password the client can send (`PASSWD_MAX_LEN`, `common/length.h:12`).
///
/// Legacy copies the client's password with `strlcpy` into `char[PASSWD_MAX_LEN + 1]`, so a
/// longer one could never be typed in full.
pub const PASSWORD_MAX_LEN: usize = 16;

/// Length of a delete code.
///
/// Legacy compares the last seven bytes of `social_id` with the first seven of the client's
/// `private_code` and refuses an account whose `social_id` is shorter
/// (`ClientManagerPlayer.cpp:1363`), so a code is exactly seven bytes.
pub const DELETE_CODE_LEN: usize = 7;

/// Why a credential was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialError {
    /// The login is not 2 to 30 ASCII letters and digits.
    InvalidLogin,
    /// The password is empty or longer than [`PASSWORD_MAX_LEN`] bytes.
    PasswordLength(usize),
    /// The password holds a byte that is not printable ASCII.
    PasswordNotPrintable,
    /// The delete code is not seven ASCII letters and digits.
    InvalidDeleteCode,
    /// The stored hash could not be parsed or computed.
    Hash(String),
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLogin => write!(
                f,
                "a login is {LOGIN_MIN_LEN} to {LOGIN_MAX_LEN} ASCII letters and digits"
            ),
            Self::PasswordLength(length) => write!(
                f,
                "a password is 1 to {PASSWORD_MAX_LEN} bytes, not {length}"
            ),
            Self::PasswordNotPrintable => f.write_str(
                "a password may hold only printable ASCII, because the client may send other \
                 characters in a different code page",
            ),
            Self::InvalidDeleteCode => write!(
                f,
                "a delete code is exactly {DELETE_CODE_LEN} ASCII letters and digits"
            ),
            Self::Hash(error) => write!(f, "password hashing failed: {error}"),
        }
    }
}

impl Error for CredentialError {}

/// A valid login, lowercased.
///
/// Legacy lowercases the login the client sends before checking and looking it up
/// (`trim_and_lower`, `input_auth.cpp:81`), so only the lowercase form is ever stored.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Login(String);

impl Login {
    /// Check and lowercase a login.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::InvalidLogin`] unless `raw` is 2 to 30 ASCII letters and digits.
    pub fn new(raw: &str) -> Result<Self, CredentialError> {
        let length_ok = (LOGIN_MIN_LEN..=LOGIN_MAX_LEN).contains(&raw.len());
        if !length_ok || !raw.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(CredentialError::InvalidLogin);
        }
        Ok(Self(raw.to_ascii_lowercase()))
    }

    /// The stored form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A new password set by the Operator.
///
/// It is 1 to 16 bytes of printable ASCII. The byte limit is the client's. The ASCII rule is
/// the Operator's safeguard: the client sends what the player types in its own code page, so a
/// password typed as UTF-8 in a terminal could never match. `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct NewPassword(Vec<u8>);

impl NewPassword {
    /// Check a new password.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::PasswordLength`] or [`CredentialError::PasswordNotPrintable`].
    pub fn new(raw: &[u8]) -> Result<Self, CredentialError> {
        if raw.is_empty() || raw.len() > PASSWORD_MAX_LEN {
            return Err(CredentialError::PasswordLength(raw.len()));
        }
        if !raw.iter().all(|&byte| (b' '..=b'~').contains(&byte)) {
            return Err(CredentialError::PasswordNotPrintable);
        }
        Ok(Self(raw.to_vec()))
    }

    /// Hash the password with argon2id and a fresh random salt.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::Hash`] if hashing fails.
    pub fn hash(&self) -> Result<PasswordDigest, CredentialError> {
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(&self.0, &salt)
            .map_err(|error| CredentialError::Hash(error.to_string()))?;
        Ok(PasswordDigest(hash.to_string()))
    }
}

impl fmt::Debug for NewPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NewPassword(***)")
    }
}

/// An argon2id PHC string, as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordDigest(String);

impl PasswordDigest {
    /// Wrap a stored PHC string.
    #[must_use]
    pub fn from_stored(phc: String) -> Self {
        Self(phc)
    }

    /// The PHC string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `candidate` is the password, as the client sent it.
    ///
    /// `candidate` is the raw bytes before the first NUL, the way legacy's `strlcpy` read them. It
    /// is not held to the [`NewPassword`] rules: a wrong password is simply wrong.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::Hash`] if the stored string is not a valid PHC hash.
    pub fn verify(&self, candidate: &[u8]) -> Result<bool, CredentialError> {
        let parsed =
            PasswordHash::new(&self.0).map_err(|error| CredentialError::Hash(error.to_string()))?;
        match Argon2::default().verify_password(candidate, &parsed) {
            Ok(()) => Ok(true),
            Err(argon2::password_hash::Error::Password) => Ok(false),
            Err(error) => Err(CredentialError::Hash(error.to_string())),
        }
    }
}

/// A delete code: exactly seven ASCII letters and digits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteCode(String);

impl DeleteCode {
    /// Check a delete code.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::InvalidDeleteCode`].
    pub fn new(raw: &str) -> Result<Self, CredentialError> {
        if raw.len() != DELETE_CODE_LEN || !raw.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(CredentialError::InvalidDeleteCode);
        }
        Ok(Self(raw.to_owned()))
    }

    /// Seven random decimal digits from the operating system's generator.
    #[must_use]
    pub fn random() -> Self {
        let digits = (0..DELETE_CODE_LEN)
            .map(|_| random_digit(&mut OsRng))
            .collect();
        Self(digits)
    }

    /// The code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One uniform decimal digit, rejecting the values that would bias `% 10`.
fn random_digit(rng: &mut impl RngCore) -> char {
    const LIMIT: u32 = u32::MAX - u32::MAX % 10;
    loop {
        let value = rng.next_u32();
        if value < LIMIT {
            return char::from_digit(value % 10, 10).unwrap_or('0');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::password_hash::rand_core;

    #[test]
    fn a_login_is_lowercased_as_legacy_does_before_its_lookup() {
        assert_eq!(Login::new("Alice42").unwrap().as_str(), "alice42");
    }

    #[test]
    fn a_login_is_two_to_thirty_letters_and_digits() {
        assert!(Login::new("ab").is_ok());
        assert!(Login::new(&"a".repeat(30)).is_ok());
        for bad in [
            "",
            "a",
            &"a".repeat(31),
            "al ice",
            "al_ice",
            "alicé",
            "alice\0",
        ] {
            assert_eq!(
                Login::new(bad),
                Err(CredentialError::InvalidLogin),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_new_password_is_one_to_sixteen_printable_ascii_bytes() {
        assert!(NewPassword::new(b"x").is_ok());
        assert!(NewPassword::new(b"a b~!0123456789Z").is_ok());
        assert_eq!(
            NewPassword::new(b""),
            Err(CredentialError::PasswordLength(0))
        );
        assert_eq!(
            NewPassword::new(&[b'a'; 17]),
            Err(CredentialError::PasswordLength(17))
        );
        for bad in [
            &b"tab\there"[..],
            b"nul\0",
            "p\u{e4}ss".as_bytes(),
            b"del\x7f",
        ] {
            assert_eq!(
                NewPassword::new(bad),
                Err(CredentialError::PasswordNotPrintable),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn debug_never_shows_a_password() {
        let shown = format!("{:?}", NewPassword::new(b"hunter2").unwrap());
        assert!(!shown.contains("hunter2"), "got {shown}");
    }

    #[test]
    fn a_hash_is_argon2id_salted_and_verifies_only_the_same_bytes() {
        let password = NewPassword::new(b"Secret 1").unwrap();
        let first = password.hash().unwrap();
        let second = password.hash().unwrap();
        assert!(
            first.as_str().starts_with("$argon2id$v=19$"),
            "got {}",
            first.as_str()
        );
        assert_ne!(first, second, "every hash has its own salt");
        assert!(!first.as_str().contains("Secret"));

        assert_eq!(first.verify(b"Secret 1"), Ok(true));
        assert_eq!(second.verify(b"Secret 1"), Ok(true));
        assert_eq!(first.verify(b"secret 1"), Ok(false), "case matters");
        assert_eq!(first.verify(b"Secret 1 "), Ok(false));
        assert_eq!(first.verify(b""), Ok(false));
    }

    #[test]
    fn a_stored_hash_that_is_not_phc_is_an_error_not_a_mismatch() {
        let stored =
            PasswordDigest::from_stored("*6BB4837EB74329105EE4568DDA7DC67ED2CA2AD9".into());
        assert!(matches!(
            stored.verify(b"123456"),
            Err(CredentialError::Hash(_))
        ));
    }

    #[test]
    fn a_delete_code_is_exactly_seven_letters_and_digits() {
        assert_eq!(DeleteCode::new("1234567").unwrap().as_str(), "1234567");
        assert!(DeleteCode::new("abcDEF0").is_ok());
        for bad in ["", "123456", "12345678", "123 567", "123-567"] {
            assert_eq!(
                DeleteCode::new(bad),
                Err(CredentialError::InvalidDeleteCode),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_random_delete_code_is_seven_digits() {
        let code = DeleteCode::random();
        assert_eq!(code.as_str().len(), DELETE_CODE_LEN);
        assert!(code.as_str().bytes().all(|byte| byte.is_ascii_digit()));
        assert_eq!(DeleteCode::new(code.as_str()), Ok(code));
    }

    /// Returns the queued values in order.
    struct Scripted(Vec<u32>);

    impl RngCore for Scripted {
        fn next_u32(&mut self) -> u32 {
            self.0.remove(0)
        }
        fn next_u64(&mut self) -> u64 {
            u64::from(self.next_u32())
        }
        fn fill_bytes(&mut self, dest: &mut [u8]) {
            dest.fill(0);
        }
        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
            self.fill_bytes(dest);
            Ok(())
        }
    }

    #[test]
    fn a_digit_rejects_the_biased_top_of_the_range() {
        // u32::MAX - 5 is in the incomplete last block of ten, so it is drawn again.
        let mut rng = Scripted(vec![u32::MAX - 5, 1234]);
        assert_eq!(random_digit(&mut rng), '4');
        assert!(rng.0.is_empty());
        let mut rng = Scripted(vec![u32::MAX - u32::MAX % 10 - 1]);
        assert_eq!(random_digit(&mut rng), '9');
    }
}
