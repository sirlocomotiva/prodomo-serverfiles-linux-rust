#![allow(dead_code)]

//! A PostgreSQL database for one test, for the tests that drive the real binary.
//!
//! `db/tests/accounts.rs` has the async twin of this helper; the process tests are synchronous.
//!
//! Every test binary that says `mod support;` compiles this file for itself and uses a
//! different part of it, so a helper unused by one binary is a warning rather than a
//! signal. `#![allow(dead_code)]` is the honest response: the items are used, just not
//! everywhere.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use db::sqlx::postgres::{PgColumn, PgRow};
use db::sqlx::{self, Column as _, Connection, PgConnection, Row as _};

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
        execute(&admin_url, &format!("CREATE DATABASE {name}"))
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

    /// Create an empty database from inside an async test, or `None` when
    /// `DATABASE_URL` is unset.
    ///
    /// [`Self::create`] is synchronous because the process tests are synchronous, and
    /// it starts a runtime with `block_on`. Doing that inside a `#[tokio::test]` panics,
    /// so the async tests call this instead. The database name comes from the same
    /// counter, so the two paths cannot collide.
    pub async fn create_async() -> Option<Self> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let admin_url = database_url()?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock should follow the Unix epoch")
            .as_nanos();
        let name = format!(
            "prodomo_async_{}_{}_{nanos}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(
                &mut PgConnection::connect(&admin_url)
                    .await
                    .expect("the admin connection"),
            )
            .await
            .expect("the test role should be allowed to create databases");
        let url = with_database(&admin_url, &name);
        Some(Self {
            admin_url,
            name,
            url,
        })
    }

    /// A migrated [`db::store::Store`] for this database.
    ///
    /// The process tests above do not need this because the binary under test migrates
    /// itself. A test that queries rows directly has nothing else to migrate for it, and
    /// a missing table would otherwise be reported as a confusing constraint error rather
    /// than as "the test never created the schema".
    pub async fn store(&self) -> db::store::Store {
        let store = db::store::Store::lazy(&db::store::StoreConfig {
            url: self.url.clone(),
            max_connections: db::store::StoreConfig::DEFAULT_MAX_CONNECTIONS,
        })
        .expect("a well-formed store configuration");
        store.migrate().await.expect("the migrations apply");
        store
    }
}

impl Drop for ScratchDatabase {
    /// Drop the database on a thread of its own.
    ///
    /// `Drop` cannot block the current thread when that thread is already driving a
    /// runtime, and an async test is exactly that. Blocking it panics, and a panic in a
    /// destructor while another panic is unwinding aborts the process, which loses the
    /// real failure in the noise. Handing the work to a detached thread and joining it
    /// works in both cases, because the new thread is not running the test's runtime.
    fn drop(&mut self) {
        let admin_url = self.admin_url.clone();
        let name = self.name.clone();
        let thread_name = name.clone();
        let reported = name.clone();
        let drop_statement = format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)");
        let worker = std::thread::Builder::new()
            .name(format!("drop-{thread_name}"))
            .spawn(move || {
                if execute(&admin_url, &drop_statement).is_err() {
                    eprintln!("could not drop scratch database {thread_name}");
                }
            });
        match worker {
            Ok(worker) => {
                let _ = worker.join();
            }
            Err(error) => eprintln!("could not start the drop thread for {reported}: {error}"),
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

/// Run one statement on its own connection and runtime, such as an `UPDATE` a scenario needs.
pub fn execute(url: &str, statement: &str) -> Result<(), sqlx::Error> {
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

/// A transaction that holds the row locks its statement took, on a thread of its own, until it
/// is released or dropped.
pub struct HeldLock {
    /// Dropped to end the transaction.
    release: Option<tokio::sync::oneshot::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl HeldLock {
    /// Commit the transaction, which frees its locks, and wait until it has.
    ///
    /// # Panics
    ///
    /// Panics when the transaction could not commit.
    pub fn release(mut self) {
        self.release = None;
        if let Some(worker) = self.worker.take() {
            assert!(worker.join().is_ok(), "the held transaction should commit");
        }
    }
}

impl Drop for HeldLock {
    /// End a transaction never released, as on a failed assertion, so that its locks and its
    /// connection do not outlive the test. A failure here is not reported: the test has already
    /// failed.
    fn drop(&mut self) {
        self.release = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Begin a transaction on its own connection, run `statement` in it, such as a
/// `SELECT ... FOR UPDATE`, and hold the locks it took until the answer is released. Also
/// answers how many rows the statement returned, so a scenario can check that it locked the
/// row it meant to.
///
/// # Panics
///
/// Panics when the store cannot be reached or the statement does not run.
pub fn hold_lock(url: &str, statement: &str) -> (HeldLock, usize) {
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let (count_tx, count_rx) = std::sync::mpsc::channel();
    let url = url.to_owned();
    let statement = statement.to_owned();
    let worker = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime should start")
            .block_on(async {
                let mut connection = PgConnection::connect(&url)
                    .await
                    .expect("the scenario's store should accept a connection");
                let mut transaction = connection.begin().await.expect("a transaction begins");
                let locked = sqlx::query(&statement)
                    .fetch_all(&mut *transaction)
                    .await
                    .expect("the statement should run");
                let _ = count_tx.send(locked.len());
                // A release or a drop of the handle ends the wait alike.
                let _ = released.await;
                transaction.commit().await.expect("the transaction commits");
                let _ = connection.close().await;
            });
    });
    let count = count_rx.recv().expect("the statement should have run");
    let held = HeldLock {
        release: Some(release),
        worker: Some(worker),
    };
    (held, count)
}

/// The integer columns of a `SELECT`, `width` per row.
///
/// The scenarios call this to read a fact back out of the store that only the store knows,
/// so the number is compared as a number. A scenario that rendered the row to text and
/// compared strings would pass on a `count(*)` that came back as `"1"` and on an `INT8`
/// that came back as `"0000000001"`, and the point of reading the column is not to re-parse
/// it.
///
/// This is **not** the shape a production query takes. `db` binds every value as a
/// parameter and a formatted value into SQL is a Defect; here the statement is written by
/// the scenario, in the scenario, out of literals the scenario owns, because the thing
/// under test is the server's answer rather than the query.
///
/// # Panics
///
/// Panics when the store cannot be reached or the statement does not run. A scenario that
/// gets that far has a scratch database it is the only user of, so a failure is a bug in
/// the scenario or the server, not a condition to skip past.
pub fn rows(url: &str, statement: &str, width: usize) -> Vec<Vec<i64>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime should start")
        .block_on(async {
            let mut connection = PgConnection::connect(url)
                .await
                .expect("the scenario's store should accept a connection");
            let result = sqlx::query(statement)
                .fetch_all(&mut connection)
                .await
                .expect("the query should run");
            let _ = connection.close().await;
            let mut out = Vec::with_capacity(result.len());
            for row in &result {
                let mut cells = Vec::with_capacity(width);
                for index in 0..width {
                    cells.push(scalar(row, index));
                }
                out.push(cells);
            }
            out
        })
}

/// One cell of one row, as an `i64`.
///
/// PostgreSQL's `INT2`, `INT4` and `INT8` are all read as one Rust integer, because the
/// only thing a scenario does with a number it read back is compare it, and a `count(*)`
/// arriving as `INT8` is a fact about PostgreSQL rather than about the store. `INT4` is
/// the only column a scenario can currently store (`item.id`, `item.vnum`,
/// `item.count`, `item.pos`, `player.id`), so the cast is right for every row a scenario
/// can produce today and a wider column would be caught by the panic below rather than
/// silently read as a smaller number.
///
/// # Panics
///
/// Panics when the column is not an integer, which is a mistake in the statement rather
/// than a missing value: a scenario reading text wants [`rows`] and a `::text` column.
fn scalar(row: &PgRow, index: usize) -> i64 {
    let column: &PgColumn = &row.columns()[index];
    // `PgTypeInfo::name` is crate-private, but its `Display` is the same string.
    match column.type_info().to_string().as_str() {
        "INT2" => i64::from(row.get::<i16, _>(index)),
        "INT4" => i64::from(row.get::<i32, _>(index)),
        "INT8" => row.get::<i64, _>(index),
        other => panic!(
            "a scenario should read a number back, and {other} is not an integer this \
             helper knows how to read"
        ),
    }
}

/// `url` with its database path replaced by `name`, keeping any query string.
pub fn with_database(url: &str, name: &str) -> String {
    let (scheme, rest) = url
        .split_once("://")
        .expect("DATABASE_URL should have a scheme");
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(authority_end);
    let query = tail.find('?').map_or("", |start| &tail[start..]);
    format!("{scheme}://{authority}/{name}{query}")
}
