//! Which scratch databases belong to a test process that still runs.
//!
//! A test drops its database when it ends, by passing or by panicking. A test process that
//! is killed runs no `Drop`, so its databases stay on the server; fifteen were found on the
//! test machine by the review of ledger 679. Each test process therefore takes a number, its
//! *owner token*, and holds a session advisory lock on it for as long as it lives; the
//! server frees the lock the moment the process is gone, however it went. Every scratch
//! database carries its owner's token in its name. Before a process takes its own token it
//! tries each token it finds in a database name: a lock it can take has no owner left, so
//! those databases are dropped.
//!
//! An advisory lock belongs to the database its session is connected to, so every test run
//! against one server must use the same `DATABASE_URL`, as `AGENTS.md` gives it.
//!
//! `prodomo/tests/support/mod.rs` compiles this file too (`#[path]`), so the rule has one
//! copy.

use std::hash::{BuildHasher as _, RandomState};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, OnceLock};

use db::sqlx::{self, Connection as _, PgConnection};

/// The first key of every owner lock, so that the tokens share no key space with another
/// user of advisory locks: `PROD` in ASCII.
const LOCK_CLASS: i32 = 0x5052_4f44;

/// A name for a new scratch database: `<prefix>_<owner token>_<n>`, of letters, digits and
/// underscores only, so it needs no quoting. `prefix` is one of the three [`sweep`] knows.
///
/// # Panics
///
/// Panics when the server at `admin_url` cannot be reached.
pub fn database_name(prefix: &str, admin_url: &str) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!(
        "{prefix}_{:08x}_{}",
        owner_token(admin_url),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// This process's owner token. The first call drops the databases of dead owners, takes the
/// token, and leaves a thread holding the connection that owns its lock.
fn owner_token(admin_url: &str) -> i32 {
    static TOKEN: OnceLock<i32> = OnceLock::new();
    *TOKEN.get_or_init(|| {
        let (report, reported) = mpsc::channel();
        let admin_url = admin_url.to_owned();
        // A thread and a runtime of its own: the caller may be a synchronous test, or an
        // async one whose runtime ends with the test, and the lock must outlive both.
        std::thread::Builder::new()
            .name("scratch-owner".to_owned())
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("a runtime should start")
                    .block_on(async {
                        let mut admin = PgConnection::connect(&admin_url)
                            .await
                            .expect("DATABASE_URL should accept a connection");
                        sweep(&mut admin).await;
                        let token = take_token(&mut admin).await;
                        report
                            .send(token)
                            .expect("the first caller should be waiting for its token");
                        // Never ends: `admin`, and with it the lock, lasts as long as the
                        // process does.
                        std::future::pending::<()>().await;
                    });
            })
            .expect("the owner thread should start");
        reported
            .recv()
            .expect("the owner thread should report its token")
    })
}

/// Lock a token no live process holds, and return it.
async fn take_token(admin: &mut PgConnection) -> i32 {
    loop {
        // Each `RandomState` is keyed afresh, so each turn tries another number.
        let token = i32::try_from(RandomState::new().hash_one(0_u8) >> 33)
            .expect("31 bits should fit an i32");
        if try_lock(admin, token).await {
            return token;
        }
    }
}

/// Take the owner lock of `token` if no session holds it.
async fn try_lock(admin: &mut PgConnection, token: i32) -> bool {
    sqlx::query_scalar("SELECT pg_try_advisory_lock($1, $2)")
        .bind(LOCK_CLASS)
        .bind(token)
        .fetch_one(admin)
        .await
        .expect("the server should answer for an advisory lock")
}

/// Drop every scratch database whose owner no longer holds its lock.
///
/// A failure is reported and passed over: another role's database cannot be dropped by this
/// one, and that must not fail a test that has nothing to do with it.
async fn sweep(admin: &mut PgConnection) {
    // The pattern admits exactly the names `database_name` makes, which is what lets a name
    // go into a statement unquoted and its third field be read as a token.
    let names: Vec<String> = match sqlx::query_scalar(
        "SELECT datname::text FROM pg_database \
         WHERE datname ~ '^prodomo_(test|proc|async)_[0-9a-f]{8}_[0-9]+$' ORDER BY datname",
    )
    .fetch_all(&mut *admin)
    .await
    {
        Ok(names) => names,
        Err(error) => {
            eprintln!("could not list the scratch databases: {error}");
            return;
        }
    };
    for name in names {
        let Some(token) = name
            .split('_')
            .nth(2)
            .and_then(|token| i32::from_str_radix(token, 16).ok())
        else {
            continue;
        };
        if !try_lock(admin, token).await {
            continue;
        }
        let dropped = sqlx::query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
            .execute(&mut *admin)
            .await;
        if let Err(error) = dropped {
            eprintln!("could not drop the leaked scratch database {name}: {error}");
        }
        let unlocked = sqlx::query("SELECT pg_advisory_unlock($1, $2)")
            .bind(LOCK_CLASS)
            .bind(token)
            .execute(&mut *admin)
            .await;
        if let Err(error) = unlocked {
            eprintln!("could not release the owner lock of {name}: {error}");
        }
    }
}
