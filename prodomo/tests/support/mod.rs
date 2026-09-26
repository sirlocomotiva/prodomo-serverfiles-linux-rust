//! A PostgreSQL database for one test, for the tests that drive the real binary.
//!
//! `db/tests/accounts.rs` has the async twin of this helper; the process tests are synchronous.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use db::sqlx::{self, Connection, PgConnection};

/// A database created for one test and dropped when the test ends, even on a panic.
pub struct ScratchDatabase {
    admin_url: String,
    name: String,
    url: String,
}

impl ScratchDatabase {
    /// Create an empty database, or `None` when `DATABASE_URL` is unset.
    ///
    /// Nothing is migrated: the binary under test does that.
    pub fn create() -> Option<Self> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let admin_url = database_url()?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock should follow the Unix epoch")
            .subsec_nanos();
        // Letters, digits, and underscores only, so the name needs no quoting.
        let name = format!(
            "prodomo_proc_{}_{}_{nanos}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        run(&admin_url, &format!("CREATE DATABASE {name}"))
            .expect("the test role should be allowed to create databases");
        let url = with_database(&admin_url, &name);
        Some(Self {
            admin_url,
            name,
            url,
        })
    }

    /// The connection URL of the database.
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for ScratchDatabase {
    fn drop(&mut self) {
        let statement = format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name);
        if run(&self.admin_url, &statement).is_err() {
            eprintln!("could not drop scratch database {}", self.name);
        }
    }
}

/// `DATABASE_URL`, or `None` with a notice when it is unset.
pub fn database_url() -> Option<String> {
    let url = std::env::var("DATABASE_URL").ok();
    if url.is_none() {
        eprintln!("skipped: DATABASE_URL is not set");
    }
    url
}

/// Run one statement on its own connection and runtime.
fn run(url: &str, statement: &str) -> Result<(), sqlx::Error> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime should start")
        .block_on(async {
            let mut connection = PgConnection::connect(url).await?;
            sqlx::query(statement).execute(&mut connection).await?;
            connection.close().await
        })
}

/// `url` with its database path replaced by `name`, keeping any query string.
pub fn with_database(url: &str, name: &str) -> String {
    let (scheme, rest) = url.split_once("://").expect("DATABASE_URL should have a scheme");
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(authority_end);
    let query = tail.find('?').map_or("", |start| &tail[start..]);
    format!("{scheme}://{authority}/{name}{query}")
}
