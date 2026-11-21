//! The scratch database every store test runs against.

mod lease;

use db::sqlx::{self, Connection, PgConnection};
use db::store::{Store, StoreConfig};

/// A database created for one test and dropped when the test ends, even on a panic. One
/// left by a test process that was killed is dropped by the next process ([`lease`]).
pub struct ScratchDatabase {
    admin_url: String,
    name: String,
    pub store: Store,
}

impl ScratchDatabase {
    /// Create and migrate a database, or `None` when `DATABASE_URL` is unset.
    pub async fn create() -> Option<Self> {
        let Ok(admin_url) = std::env::var("DATABASE_URL") else {
            eprintln!("skipped: DATABASE_URL is not set");
            return None;
        };
        let name = lease::database_name("prodomo_test", &admin_url);
        let mut admin = PgConnection::connect(&admin_url)
            .await
            .expect("DATABASE_URL should accept a connection");
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(&mut admin)
            .await
            .expect("the test role should be allowed to create databases");
        admin
            .close()
            .await
            .expect("the admin connection should close");
        let store = Store::connect(&StoreConfig::new(with_database(&admin_url, &name)))
            .await
            .expect("the scratch database should accept a connection");
        store.migrate().await.expect("the migrations should apply");
        Some(Self {
            admin_url,
            name,
            store,
        })
    }
}

impl Drop for ScratchDatabase {
    fn drop(&mut self) {
        let admin_url = self.admin_url.clone();
        let statement = format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name);
        // Drop cannot await, and the test's runtime may be the one running this, so the
        // statement runs on a thread with its own runtime.
        let dropped = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime should start")
                .block_on(async {
                    let mut admin = PgConnection::connect(&admin_url).await?;
                    sqlx::query(&statement).execute(&mut admin).await?;
                    admin.close().await
                })
        })
        .join();
        if !matches!(dropped, Ok(Ok(()))) {
            eprintln!("could not drop scratch database {}", self.name);
        }
    }
}

/// `url` with its database path replaced by `name`, keeping any query string.
fn with_database(url: &str, name: &str) -> String {
    let (scheme, rest) = url
        .split_once("://")
        .expect("DATABASE_URL should have a scheme");
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(authority_end);
    let query = tail.find('?').map_or("", |start| &tail[start..]);
    format!("{scheme}://{authority}/{name}{query}")
}

#[test]
fn a_database_url_keeps_its_credentials_host_and_options() {
    assert_eq!(
        with_database(
            "postgres://u:p@127.0.0.1:55432/prodomo?sslmode=disable",
            "t1"
        ),
        "postgres://u:p@127.0.0.1:55432/t1?sslmode=disable"
    );
    assert_eq!(with_database("postgres://u@db", "t2"), "postgres://u@db/t2");
    assert_eq!(
        with_database("postgresql://db/?x=1", "t3"),
        "postgresql://db/t3?x=1"
    );
}
