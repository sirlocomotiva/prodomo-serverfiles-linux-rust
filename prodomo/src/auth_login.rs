//! The auth login: the rules `CInputAuth::Login` applies to `HEADER_CG_LOGIN3`, and the registry
//! of logins held by auth descriptors and of the login keys they were granted.
//!
//! Legacy splits the checks in two. `CInputAuth::Login` (`G/input_auth.cpp:67-180`) runs the
//! checks that need no database, then queries the account; `DBManager::AnalyzeReturnQuery`
//! (`G/db.cpp:246-460`, `QID_AUTH_LOGIN`) judges the row. The functions here keep that order, and
//! the failure each check sends, so that a client which fails two checks sees the one legacy
//! reports first.
//!
//! A granted login key stays valid until the same login authenticates again, as the legacy DB
//! server's login data does (`D/ClientManager.cpp`, `QUERY_AUTH_LOGIN`); the Channel's `LOGIN2`
//! looks it up there on every Channel change.

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use db::accounts::{AccountId, AuthAccount};
use db::credentials::Login;

/// Legacy `LOGIN_MAX_LEN`: the login bytes `trim_and_lower` keeps.
pub const LOGIN_MAX_LEN: usize = 30;

/// Legacy `PASSWD_MAX_LEN`: the password bytes `strlcpy` keeps.
pub const PASSWD_MAX_LEN: usize = 16;

/// Legacy `LOCALE_MAX_NUM` (`server/server/common/length.h:1214`): a language byte at or above it is
/// refused with `INVLANG`.
pub const LANGUAGE_COUNT: u8 = 12;

/// The block date legacy compiles in (`g_stBlockDate`, `G/config.cpp:112`).
pub const DEFAULT_BLOCK_DATE: &str = "30000705";

/// The status an account must have to log in.
pub const STATUS_OK: &str = "OK";

/// A reason `CInputAuth::Login` or `QID_AUTH_LOGIN` refuses a login. Each is sent to the client
/// as `HEADER_GC_LOGIN_FAILURE`, and the descriptor stays open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthRefusal {
    /// The login is malformed, or no account has it.
    NoId,
    /// The server takes no more clients.
    Shutdown,
    /// Another auth descriptor holds the login.
    Already,
    /// The password does not match.
    WrongPassword,
    /// The account may not log in yet.
    NotAvailable,
    /// The account status is not `OK`; the status itself is the failure.
    Status(String),
    /// The language byte is at or above [`LANGUAGE_COUNT`].
    InvalidLanguage,
    /// The language byte is zero.
    NoLanguage,
    /// The account was created on or after the block date.
    BlockedLogin,
}

impl AuthRefusal {
    /// The `szStatus` bytes legacy sends for this refusal.
    #[must_use]
    pub fn status(&self) -> &[u8] {
        match self {
            Self::NoId => b"NOID",
            Self::Shutdown => b"SHUTDOWN",
            Self::Already => b"ALREADY",
            Self::WrongPassword => b"WRONGPWD",
            Self::NotAvailable => b"NOTAVAIL",
            Self::Status(status) => status.as_bytes(),
            Self::InvalidLanguage => b"INVLANG",
            Self::NoLanguage => b"NOLANG",
            Self::BlockedLogin => b"BLKLOGIN",
        }
    }
}

/// C `isspace` in the C locale. `u8::is_ascii_whitespace` leaves out the vertical tab.
const fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t'..=b'\r')
}

/// The login legacy checks, from the raw `login` field.
///
/// This is `trim_and_lower` (`libthecore/utils.cpp:121-165`) followed by
/// `FN_IS_VALID_LOGIN_STRING` (`G/input_auth.cpp:14-54`): leading whitespace is skipped, at most
/// [`LOGIN_MAX_LEN`] bytes are kept, trailing whitespace is dropped, and the rest must be two or
/// more ASCII letters and digits. `None` is the `NOID` legacy sends before any other check.
///
/// Legacy reads the field up to a NUL it does not guarantee; the Rewrite stops at the end of the
/// field.
#[must_use]
pub fn login_from_field(field: &[u8; 31]) -> Option<Login> {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    let start = field[..end]
        .iter()
        .position(|&b| !is_c_space(b))
        .unwrap_or(end);
    let kept = &field[start..end.min(start + LOGIN_MAX_LEN)];
    let trimmed_end = kept
        .iter()
        .rposition(|&b| !is_c_space(b))
        .map_or(0, |i| i + 1);
    let login = std::str::from_utf8(&kept[..trimmed_end]).ok()?;
    Login::new(login).ok()
}

/// The password legacy checks: the bytes before the first NUL, at most [`PASSWD_MAX_LEN`]
/// (`strlcpy(passwd, pinfo->passwd, sizeof(passwd))`).
#[must_use]
pub fn password_candidate(field: &[u8; 17]) -> &[u8] {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    &field[..end.min(PASSWD_MAX_LEN)]
}

/// The first half of `QID_AUTH_LOGIN`: the row exists, the password matches, and the account is
/// available (`G/db.cpp:372-383`). `password_matches` is the argon2id verification of the
/// candidate, which the caller runs off the async runtime.
///
/// # Errors
///
/// Returns the refusal legacy sends first.
pub fn judge_credentials(account: &AuthAccount, password_matches: bool) -> Result<(), AuthRefusal> {
    if !password_matches {
        return Err(AuthRefusal::WrongPassword);
    }
    if account.unavailable {
        return Err(AuthRefusal::NotAvailable);
    }
    Ok(())
}

/// The second half of `QID_AUTH_LOGIN`, after the `ALREADY` check: the status, the language, and
/// the block date (`G/db.cpp:404-431`).
///
/// `block_date` is compared with `created_on` as `strncmp(..., 8) >= 0` does. Both are eight
/// ASCII digits in practice, so the byte order is the date order.
///
/// # Errors
///
/// Returns the refusal legacy sends first.
pub fn judge_account(
    account: &AuthAccount,
    language: u8,
    block_date: &str,
) -> Result<(), AuthRefusal> {
    if account.status != STATUS_OK {
        return Err(AuthRefusal::Status(account.status.clone()));
    }
    if language >= LANGUAGE_COUNT {
        return Err(AuthRefusal::InvalidLanguage);
    }
    if language == 0 {
        return Err(AuthRefusal::NoLanguage);
    }
    if first_eight(account.created_on.as_bytes()) >= first_eight(block_date.as_bytes()) {
        return Err(AuthRefusal::BlockedLogin);
    }
    Ok(())
}

/// The bytes `strncmp(a, b, 8)` compares.
fn first_eight(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.len().min(8)]
}

/// What a login key grants: the account, and what the Channel's `LOGIN2` must match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginGrant {
    /// The account that authenticated.
    pub account: AccountId,
    /// Its login.
    pub login: Login,
    /// The `adwClientKey` the client sent with `LOGIN3`.
    pub client_key: [u32; 4],
    /// The language the client chose.
    pub language: u8,
}

#[derive(Debug, Default)]
struct Registry {
    /// Logins held by a live auth descriptor.
    held: HashMap<Login, u64>,
    /// Granted login keys.
    grants: HashMap<u32, LoginGrant>,
    /// The key each login was last granted.
    key_of: HashMap<Login, u32>,
    next_claim: u64,
}

/// The logins held by auth descriptors and the login keys granted to them.
#[derive(Debug, Default)]
pub struct AuthRegistry {
    inner: Mutex<Registry>,
}

impl AuthRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> MutexGuard<'_, Registry> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether an auth descriptor holds `login` (legacy `DESC_MANAGER::FindByLoginName`).
    #[must_use]
    pub fn is_held(&self, login: &Login) -> bool {
        self.lock().held.contains_key(login)
    }

    /// Hold `login` for one descriptor until the claim is dropped, or refuse with `ALREADY` when
    /// another descriptor holds it (legacy `ConnectAccount`, checked again at `G/db.cpp:386`).
    ///
    /// # Errors
    ///
    /// Returns [`AuthRefusal::Already`].
    pub fn claim(self: &Arc<Self>, login: &Login) -> Result<AuthClaim, AuthRefusal> {
        let mut registry = self.lock();
        if registry.held.contains_key(login) {
            return Err(AuthRefusal::Already);
        }
        registry.next_claim += 1;
        let id = registry.next_claim;
        registry.held.insert(login.clone(), id);
        Ok(AuthClaim {
            registry: Arc::clone(self),
            login: login.clone(),
            id,
        })
    }

    /// Grant a new login key, replacing the key this login was granted before, and return it.
    /// Keys are 1 to `i32::MAX` and unique among live grants (legacy `CreateLoginKey`).
    pub fn grant(&self, grant: LoginGrant) -> u32 {
        let mut registry = self.lock();
        let key = loop {
            let key = login_key_candidate();
            if !registry.grants.contains_key(&key) {
                break key;
            }
        };
        if let Some(old) = registry.key_of.insert(grant.login.clone(), key) {
            registry.grants.remove(&old);
        }
        registry.grants.insert(key, grant);
        key
    }

    /// The grant behind a login key, if it is live.
    #[must_use]
    pub fn grant_for(&self, key: u32) -> Option<LoginGrant> {
        self.lock().grants.get(&key).cloned()
    }
}

/// A random login key in 1 to `i32::MAX`.
fn login_key_candidate() -> u32 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    loop {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(NEXT.fetch_add(1, Ordering::Relaxed));
        let [a, b, c, d, ..] = hasher.finish().to_le_bytes();
        let key = u32::from_le_bytes([a, b, c, d]) & 0x7fff_ffff;
        if key != 0 {
            return key;
        }
    }
}

/// One descriptor's hold on a login. Dropping it releases the login.
#[derive(Debug)]
pub struct AuthClaim {
    registry: Arc<AuthRegistry>,
    login: Login,
    id: u64,
}

impl AuthClaim {
    /// The held login.
    #[must_use]
    pub fn login(&self) -> &Login {
        &self.login
    }
}

impl Drop for AuthClaim {
    fn drop(&mut self) {
        let mut registry = self.registry.lock();
        if registry.held.get(&self.login) == Some(&self.id) {
            registry.held.remove(&self.login);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::credentials::NewPassword;

    fn field<const N: usize>(bytes: &[u8]) -> [u8; N] {
        let mut out = [0; N];
        out[..bytes.len()].copy_from_slice(bytes);
        out
    }

    fn login(raw: &str) -> Login {
        Login::new(raw).unwrap()
    }

    fn account(status: &str, unavailable: bool, created_on: &str) -> AuthAccount {
        AuthAccount {
            id: AccountId::new(7),
            password: NewPassword::new(b"pw").unwrap().hash().unwrap(),
            status: status.to_owned(),
            unavailable,
            created_on: created_on.to_owned(),
        }
    }

    #[test]
    fn the_login_is_trimmed_lowered_and_cut_as_trim_and_lower_does() {
        assert_eq!(login_from_field(&field(b"Alice")), Some(login("alice")));
        assert_eq!(
            login_from_field(&field(b" \t\x0bAlice\r\n")),
            Some(login("alice"))
        );
        // Thirty bytes are kept, so a space after them is cut away, not refused.
        let thirty = [b'a'; 30];
        let mut long = field::<31>(&thirty);
        long[30] = b'b';
        assert_eq!(login_from_field(&long), Some(login(&"a".repeat(30))));
        let mut spaced = [b' '; 31];
        spaced[1..31].copy_from_slice(&thirty);
        assert_eq!(login_from_field(&spaced), Some(login(&"a".repeat(30))));
        // Trimming happens after the cut: the 31st byte is dropped and the inner space stays.
        let mut inner = [b'a'; 31];
        inner[5] = b' ';
        assert_eq!(login_from_field(&inner), None);
    }

    #[test]
    fn a_malformed_login_is_noid() {
        for raw in [
            &b""[..],
            b"a",
            b" a ",
            b"al ice",
            b"al_ice",
            b"al\xe9ce",
            b"\xa1\xa1ab",
        ] {
            assert_eq!(login_from_field(&field(raw)), None, "{raw:?}");
        }
        assert_eq!(login_from_field(&field(b"ab")), Some(login("ab")));
        assert_eq!(login_from_field(&field(b"ab\0cd")), Some(login("ab")));
    }

    #[test]
    fn the_password_stops_at_the_nul_and_at_sixteen_bytes() {
        assert_eq!(password_candidate(&field(b"secret\0junk")), b"secret");
        assert_eq!(password_candidate(&[b'x'; 17]), [b'x'; 16]);
        assert_eq!(password_candidate(&field(b"")), b"");
    }

    #[test]
    fn credentials_are_judged_in_the_legacy_order() {
        let open = account("OK", false, "20260926");
        assert_eq!(judge_credentials(&open, true), Ok(()));
        assert_eq!(
            judge_credentials(&open, false),
            Err(AuthRefusal::WrongPassword)
        );
        let later = account("OK", true, "20260926");
        assert_eq!(
            judge_credentials(&later, false),
            Err(AuthRefusal::WrongPassword)
        );
        assert_eq!(
            judge_credentials(&later, true),
            Err(AuthRefusal::NotAvailable)
        );
    }

    #[test]
    fn the_account_is_judged_in_the_legacy_order() {
        let ok = account("OK", false, "20260926");
        assert_eq!(judge_account(&ok, 1, DEFAULT_BLOCK_DATE), Ok(()));
        assert_eq!(judge_account(&ok, 11, DEFAULT_BLOCK_DATE), Ok(()));
        assert_eq!(
            judge_account(&ok, 12, DEFAULT_BLOCK_DATE),
            Err(AuthRefusal::InvalidLanguage)
        );
        assert_eq!(
            judge_account(&ok, 255, DEFAULT_BLOCK_DATE),
            Err(AuthRefusal::InvalidLanguage)
        );
        assert_eq!(
            judge_account(&ok, 0, DEFAULT_BLOCK_DATE),
            Err(AuthRefusal::NoLanguage)
        );
        assert_eq!(
            judge_account(&ok, 1, "20260926"),
            Err(AuthRefusal::BlockedLogin)
        );
        assert_eq!(
            judge_account(&ok, 1, "20260927"),
            Ok(()),
            "a block date after the creation date lets the account in"
        );
        assert_eq!(
            judge_account(&ok, 1, "20260925"),
            Err(AuthRefusal::BlockedLogin)
        );
        let blocked = account("BLOCK", false, "20260926");
        assert_eq!(
            judge_account(&blocked, 0, "20000101"),
            Err(AuthRefusal::Status("BLOCK".to_owned()))
        );
        assert_eq!(AuthRefusal::Status("BLOCK".to_owned()).status(), b"BLOCK");
        assert_eq!(
            judge_account(&ok, 12, "20000101"),
            Err(AuthRefusal::InvalidLanguage),
            "the language is checked before the block date"
        );
    }

    #[test]
    fn each_refusal_sends_the_legacy_status() {
        let refusals = [
            AuthRefusal::NoId,
            AuthRefusal::Shutdown,
            AuthRefusal::Already,
            AuthRefusal::WrongPassword,
            AuthRefusal::NotAvailable,
            AuthRefusal::InvalidLanguage,
            AuthRefusal::NoLanguage,
            AuthRefusal::BlockedLogin,
        ];
        let statuses: Vec<&[u8]> = refusals.iter().map(AuthRefusal::status).collect();
        assert_eq!(
            statuses,
            [
                &b"NOID"[..],
                b"SHUTDOWN",
                b"ALREADY",
                b"WRONGPWD",
                b"NOTAVAIL",
                b"INVLANG",
                b"NOLANG",
                b"BLKLOGIN"
            ]
        );
    }

    #[test]
    fn a_login_is_held_by_one_descriptor_until_its_claim_drops() {
        let registry = AuthRegistry::new();
        let alice = login("alice");
        let claim = registry.claim(&alice).unwrap();
        assert_eq!(claim.login(), &alice);
        assert!(registry.is_held(&alice));
        assert_eq!(registry.claim(&alice).unwrap_err(), AuthRefusal::Already);
        let bob = registry.claim(&login("bob")).unwrap();
        drop(claim);
        assert!(!registry.is_held(&alice));
        assert!(registry.is_held(&login("bob")));
        let again = registry.claim(&alice).unwrap();
        drop(bob);
        assert!(registry.is_held(&alice));
        drop(again);
        assert!(!registry.is_held(&alice));
    }

    #[test]
    fn a_new_grant_replaces_the_logins_old_key() {
        let registry = AuthRegistry::new();
        let grant = |name: &str, language| LoginGrant {
            account: AccountId::new(7),
            login: login(name),
            client_key: [0x0102_0304, 0x0506_0708, 0x090a_0b0c, 0x0d0e_0f10],
            language,
        };
        let first = registry.grant(grant("alice", 1));
        let other = registry.grant(grant("bob", 2));
        assert!((1..=0x7fff_ffff).contains(&first));
        assert_eq!(registry.grant_for(first), Some(grant("alice", 1)));
        let second = registry.grant(grant("alice", 3));
        assert_ne!(first, second);
        assert_eq!(registry.grant_for(first), None);
        assert_eq!(registry.grant_for(second), Some(grant("alice", 3)));
        assert_eq!(registry.grant_for(other), Some(grant("bob", 2)));
    }

    #[test]
    fn login_keys_are_positive_ints() {
        for _ in 0..10_000 {
            let key = login_key_candidate();
            assert!(key != 0 && key <= 0x7fff_ffff);
        }
    }
}
