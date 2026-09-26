//! The Operator commands: accounts, GM grants, and the two account currencies (ADR-0003).
//!
//! The Rewrite has no web shop or account site, so the Operator creates accounts and adjusts
//! balances with `prodomo account ...` and grants GM authority with `prodomo gm ...`. Each
//! command is checked in full, and its password read, before the store is opened: [`prepare`]
//! does the checking and [`Prepared::execute`] the writing.
//!
//! Legacy granted GM authority per character Name in `common.gmlist`, and only to a character
//! whose account matched (`gm_new_get_level`), so a grant names both the account and the Name.
//! The `mContactIP`, `mServerIP`, and `gmhost` columns are not carried over: there are no Cores to
//! pick between, and the host check they fed is off unless `gm_host_check` was set.
//!
//! A password is read from standard input. On a terminal it is asked for twice with echo off;
//! otherwise the first line is the password, so a script can pipe it in without it ever reaching
//! the command line or the process list.

use std::error::Error;
use std::fmt;
use std::io::{self, BufRead, IsTerminal, Write};
use std::os::fd::{AsFd, BorrowedFd};

use clap::Subcommand;
use common::gm::GmAuthority;
use db::accounts::{
    adjust_balance, create_account, grant_gm, list_gm_grants, revoke_gm, set_password,
    AccountError, Currency, Name, NewAccount,
};
use db::credentials::{CredentialError, DeleteCode, Login, NewPassword, PasswordDigest};
use db::store::Store;
use rustix::termios::{tcgetattr, tcsetattr, LocalModes, OptionalActions, Termios};

/// `prodomo account ...`.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum AccountCommand {
    /// Create an account. The password is read from standard input.
    Create {
        /// The login: 2 to 30 ASCII letters and digits, stored in lowercase.
        login: String,
        /// The 7 letters and digits a player types to delete a character. When omitted, a random
        /// 7-digit code is generated and printed.
        #[arg(long)]
        delete_code: Option<String>,
    },
    /// Replace an account's password with one read from standard input.
    Password {
        /// The login.
        login: String,
    },
    /// Add Coins, the item-shop currency, and print the new balance. A negative amount removes
    /// Coins.
    Coins {
        /// The login.
        login: String,
        /// The change.
        #[arg(allow_negative_numbers = true)]
        amount: i64,
    },
    /// Add Cash, the daily-gift currency, and print the new balance. A negative amount removes
    /// Cash.
    Cash {
        /// The login.
        login: String,
        /// The change.
        #[arg(allow_negative_numbers = true)]
        amount: i64,
    },
}

/// `prodomo gm ...`.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum GmCommand {
    /// Grant GM authority to one character Name of an account. Granting a Name the account already
    /// holds changes its authority.
    Grant {
        /// The login of the account the character must belong to.
        login: String,
        /// The character Name: 2 to 24 ASCII letters and digits. It need not exist yet.
        name: String,
        /// `low_wizard`, `wizard`, `high_wizard`, `god`, or `implementor`.
        #[arg(value_parser = parse_authority)]
        authority: GmAuthority,
    },
    /// Remove the grant of a character Name.
    Revoke {
        /// The character Name, in any case.
        name: String,
    },
    /// List every grant.
    List,
}

/// An Operator command, as the command line gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatorCommand {
    /// `prodomo account ...`.
    Account(AccountCommand),
    /// `prodomo gm ...`.
    Gm(GmCommand),
}

/// Parse a GM authority in the legacy spelling, in any case, with `-` accepted for `_`.
///
/// # Errors
///
/// Returns the list of accepted spellings.
pub fn parse_authority(raw: &str) -> Result<GmAuthority, String> {
    let spelling = raw.to_ascii_uppercase().replace('-', "_");
    GmAuthority::from_column(spelling.as_bytes()).ok_or_else(|| {
        let accepted: Vec<String> = GmAuthority::ALL
            .iter()
            .map(|authority| authority.column().to_ascii_lowercase())
            .collect();
        format!("expected one of {}", accepted.join(", "))
    })
}

/// A command whose arguments are checked and whose password is hashed, ready to run.
#[derive(Debug, Clone)]
pub enum Prepared {
    /// Create an account.
    CreateAccount {
        /// The account.
        account: NewAccount,
        /// Whether the delete code was generated, and so must be printed.
        generated_code: bool,
    },
    /// Replace a password.
    SetPassword {
        /// The login.
        login: Login,
        /// The new password.
        password: PasswordDigest,
    },
    /// Change a balance.
    AdjustBalance {
        /// The login.
        login: Login,
        /// The currency.
        currency: Currency,
        /// The change.
        delta: i64,
    },
    /// Grant GM authority.
    GrantGm {
        /// The login.
        login: Login,
        /// The character Name.
        name: Name,
        /// The authority.
        authority: GmAuthority,
    },
    /// Remove a GM grant.
    RevokeGm {
        /// The character Name.
        name: Name,
    },
    /// List every GM grant.
    ListGm,
}

/// Check a command's arguments and, for a command that sets a password, read and hash it.
///
/// `read_password` is called at most once, and only after every other argument is valid.
///
/// # Errors
///
/// Returns [`OperatorError::Invalid`] for a bad argument or password, or whatever
/// `read_password` returns.
pub fn prepare(
    command: &OperatorCommand,
    read_password: impl FnOnce() -> Result<NewPassword, OperatorError>,
) -> Result<Prepared, OperatorError> {
    let prepared = match command {
        OperatorCommand::Account(AccountCommand::Create { login, delete_code }) => {
            let login = Login::new(login)?;
            let (delete_code, generated_code) = match delete_code {
                Some(code) => (DeleteCode::new(code)?, false),
                None => (DeleteCode::random(), true),
            };
            let password = read_password()?.hash()?;
            Prepared::CreateAccount {
                account: NewAccount {
                    login,
                    password,
                    delete_code,
                },
                generated_code,
            }
        }
        OperatorCommand::Account(AccountCommand::Password { login }) => {
            let login = Login::new(login)?;
            Prepared::SetPassword {
                login,
                password: read_password()?.hash()?,
            }
        }
        OperatorCommand::Account(AccountCommand::Coins { login, amount }) => {
            Prepared::AdjustBalance {
                login: Login::new(login)?,
                currency: Currency::Coins,
                delta: *amount,
            }
        }
        OperatorCommand::Account(AccountCommand::Cash { login, amount }) => {
            Prepared::AdjustBalance {
                login: Login::new(login)?,
                currency: Currency::Cash,
                delta: *amount,
            }
        }
        OperatorCommand::Gm(GmCommand::Grant {
            login,
            name,
            authority,
        }) => Prepared::GrantGm {
            login: Login::new(login)?,
            name: Name::new(name)?,
            authority: *authority,
        },
        OperatorCommand::Gm(GmCommand::Revoke { name }) => Prepared::RevokeGm {
            name: Name::new(name)?,
        },
        OperatorCommand::Gm(GmCommand::List) => Prepared::ListGm,
    };
    Ok(prepared)
}

impl Prepared {
    /// Run the command and report what it did on `out`.
    ///
    /// # Errors
    ///
    /// Returns [`OperatorError::Invalid`] when the store refuses the change, or
    /// [`OperatorError::Output`].
    pub async fn execute(&self, store: &Store, out: &mut impl Write) -> Result<(), OperatorError> {
        match self {
            Self::CreateAccount {
                account,
                generated_code,
            } => {
                let id = create_account(store, account).await?;
                writeln!(out, "Created account {} with id {id}", account.login)?;
                if *generated_code {
                    writeln!(out, "Delete code: {}", account.delete_code.as_str())?;
                }
            }
            Self::SetPassword { login, password } => {
                set_password(store, login, password).await?;
                writeln!(out, "Changed the password of {login}")?;
            }
            Self::AdjustBalance {
                login,
                currency,
                delta,
            } => {
                let balance = adjust_balance(store, login, *currency, *delta).await?;
                writeln!(out, "{login} now has {balance} {currency}")?;
            }
            Self::GrantGm {
                login,
                name,
                authority,
            } => {
                grant_gm(store, login, name, *authority).await?;
                writeln!(
                    out,
                    "Granted {} to {name} on account {login}",
                    authority.column()
                )?;
            }
            Self::RevokeGm { name } => {
                revoke_gm(store, name).await?;
                writeln!(out, "Revoked the grant of {name}")?;
            }
            Self::ListGm => {
                let grants = list_gm_grants(store).await?;
                if grants.is_empty() {
                    writeln!(out, "No GM grants")?;
                }
                for grant in grants {
                    writeln!(
                        out,
                        "{:<24} {:<30} {}",
                        grant.name,
                        grant.login,
                        grant.authority.column()
                    )?;
                }
            }
        }
        Ok(())
    }
}

/// Why an Operator command failed.
#[derive(Debug)]
pub enum OperatorError {
    /// An argument or password was refused, or the store refused the change.
    Invalid(AccountError),
    /// Standard input held no password.
    NoPassword,
    /// The two passwords typed on the terminal differ.
    PasswordsDiffer,
    /// Reading the password or writing the report failed.
    Output(io::Error),
}

impl fmt::Display for OperatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(error) => error.fmt(f),
            Self::NoPassword => f.write_str("no password on standard input"),
            Self::PasswordsDiffer => f.write_str("the passwords differ"),
            Self::Output(error) => write!(f, "terminal I/O failed: {error}"),
        }
    }
}

impl Error for OperatorError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Invalid(error) => Some(error),
            Self::Output(error) => Some(error),
            Self::NoPassword | Self::PasswordsDiffer => None,
        }
    }
}

impl From<AccountError> for OperatorError {
    fn from(error: AccountError) -> Self {
        Self::Invalid(error)
    }
}

impl From<CredentialError> for OperatorError {
    fn from(error: CredentialError) -> Self {
        Self::Invalid(AccountError::Credential(error))
    }
}

impl From<io::Error> for OperatorError {
    fn from(error: io::Error) -> Self {
        Self::Output(error)
    }
}

/// Read a new password from standard input.
///
/// On a terminal the password is asked for twice on standard error with echo off. Otherwise the
/// first line is the password.
///
/// # Errors
///
/// Returns [`OperatorError::NoPassword`], [`OperatorError::PasswordsDiffer`],
/// [`OperatorError::Invalid`] for a password outside the rules, or [`OperatorError::Output`].
pub fn read_new_password() -> Result<NewPassword, OperatorError> {
    let stdin = io::stdin();
    let password = if stdin.is_terminal() {
        let first = prompt_hidden(&stdin, "Password: ")?;
        let second = prompt_hidden(&stdin, "Repeat the password: ")?;
        if first != second {
            return Err(OperatorError::PasswordsDiffer);
        }
        first
    } else {
        let mut line = Vec::new();
        stdin.lock().read_until(b'\n', &mut line)?;
        password_line(&line)?.to_vec()
    };
    Ok(NewPassword::new(&password)?)
}

/// The password on one line read from standard input, without its line ending.
///
/// # Errors
///
/// Returns [`OperatorError::NoPassword`] when standard input was empty.
pub fn password_line(line: &[u8]) -> Result<&[u8], OperatorError> {
    if line.is_empty() {
        return Err(OperatorError::NoPassword);
    }
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    Ok(line.strip_suffix(b"\r").unwrap_or(line))
}

fn prompt_hidden(stdin: &io::Stdin, prompt: &str) -> Result<Vec<u8>, OperatorError> {
    let mut stderr = io::stderr();
    write!(stderr, "{prompt}")?;
    stderr.flush()?;
    let mut line = Vec::new();
    let read = {
        let _echo_off = EchoOff::new(stdin.as_fd())?;
        stdin.lock().read_until(b'\n', &mut line)
    };
    // The Enter key was not echoed either.
    writeln!(stderr)?;
    read?;
    Ok(password_line(&line)?.to_vec())
}

/// Terminal echo, off until this is dropped.
struct EchoOff<'fd> {
    terminal: BorrowedFd<'fd>,
    saved: Termios,
}

impl<'fd> EchoOff<'fd> {
    fn new(terminal: BorrowedFd<'fd>) -> io::Result<Self> {
        let saved = tcgetattr(terminal)?;
        let mut quiet = saved.clone();
        quiet.local_modes.remove(LocalModes::ECHO);
        tcsetattr(terminal, OptionalActions::Now, &quiet)?;
        Ok(Self { terminal, saved })
    }
}

impl Drop for EchoOff<'_> {
    fn drop(&mut self) {
        // Nothing useful can be done if the terminal refuses; the shell's `reset` fixes it.
        let _ = tcsetattr(self.terminal, OptionalActions::Now, &self.saved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(password: &'static [u8]) -> impl FnOnce() -> Result<NewPassword, OperatorError> {
        move || Ok(NewPassword::new(password)?)
    }

    fn never() -> impl FnOnce() -> Result<NewPassword, OperatorError> {
        || panic!("the password must not be read")
    }

    fn create(login: &str, delete_code: Option<&str>) -> OperatorCommand {
        OperatorCommand::Account(AccountCommand::Create {
            login: login.to_owned(),
            delete_code: delete_code.map(str::to_owned),
        })
    }

    #[test]
    fn an_authority_is_accepted_in_the_legacy_spelling_in_any_case() {
        assert_eq!(parse_authority("god"), Ok(GmAuthority::God));
        assert_eq!(parse_authority("HIGH_WIZARD"), Ok(GmAuthority::HighWizard));
        assert_eq!(parse_authority("low-wizard"), Ok(GmAuthority::LowWizard));
        assert_eq!(
            parse_authority("player"),
            Err("expected one of low_wizard, wizard, high_wizard, god, implementor".to_owned())
        );
        assert!(parse_authority("gods").is_err());
    }

    #[test]
    fn a_password_line_loses_only_its_line_ending() {
        assert_eq!(password_line(b"pw\n").unwrap(), b"pw");
        assert_eq!(password_line(b"pw\r\n").unwrap(), b"pw");
        assert_eq!(password_line(b"pw").unwrap(), b"pw");
        assert_eq!(password_line(b" pw \n").unwrap(), b" pw ");
        assert_eq!(password_line(b"\n").unwrap(), b"");
        assert!(matches!(password_line(b""), Err(OperatorError::NoPassword)));
    }

    #[test]
    fn a_new_account_gets_a_lowercase_login_and_a_generated_code_when_none_is_given() {
        let Prepared::CreateAccount {
            account,
            generated_code,
        } = prepare(&create("Alice", None), fixed(b"s3cret")).unwrap()
        else {
            panic!("an account is created");
        };
        assert_eq!(account.login.as_str(), "alice");
        assert!(generated_code);
        assert!(account.password.verify(b"s3cret").unwrap());

        let Prepared::CreateAccount {
            account,
            generated_code,
        } = prepare(&create("alice", Some("AbC1234")), fixed(b"pw")).unwrap()
        else {
            panic!("an account is created");
        };
        assert_eq!(account.delete_code.as_str(), "AbC1234");
        assert!(!generated_code);
    }

    #[test]
    fn a_bad_argument_is_refused_before_the_password_is_read() {
        for command in [
            create("a", None),
            create("alice", Some("123456")),
            OperatorCommand::Account(AccountCommand::Password {
                login: "al ice".to_owned(),
            }),
        ] {
            assert!(
                matches!(prepare(&command, never()), Err(OperatorError::Invalid(_))),
                "{command:?}"
            );
        }
    }

    #[test]
    fn a_password_outside_the_rules_is_refused() {
        let read = || Ok(NewPassword::new(&[b'x'; 17])?);
        assert!(matches!(
            prepare(&create("alice", None), read),
            Err(OperatorError::Invalid(AccountError::Credential(
                CredentialError::PasswordLength(17)
            )))
        ));
    }

    #[test]
    fn a_balance_change_keeps_its_sign_and_currency() {
        let command = OperatorCommand::Account(AccountCommand::Cash {
            login: "Bob".to_owned(),
            amount: -5,
        });
        let Prepared::AdjustBalance {
            login,
            currency,
            delta,
        } = prepare(&command, never()).unwrap()
        else {
            panic!("a balance changes");
        };
        assert_eq!(
            (login.as_str(), currency, delta),
            ("bob", Currency::Cash, -5)
        );
    }

    #[test]
    fn a_gm_grant_keeps_the_name_as_typed() {
        let command = OperatorCommand::Gm(GmCommand::Grant {
            login: "Alice".to_owned(),
            name: "AdminX".to_owned(),
            authority: GmAuthority::God,
        });
        let Prepared::GrantGm {
            login,
            name,
            authority,
        } = prepare(&command, never()).unwrap()
        else {
            panic!("a grant is made");
        };
        assert_eq!(
            (login.as_str(), name.as_str(), authority),
            ("alice", "AdminX", GmAuthority::God)
        );
        let bad = OperatorCommand::Gm(GmCommand::Revoke {
            name: "A".to_owned(),
        });
        assert!(matches!(
            prepare(&bad, never()),
            Err(OperatorError::Invalid(AccountError::InvalidName(_)))
        ));
    }
}
