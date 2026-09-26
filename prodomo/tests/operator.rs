//! Drives `prodomo account ...` and `prodomo gm ...` as an Operator would, against a real store.
//!
//! Every test runs only when `DATABASE_URL` is set.

#![cfg(unix)]

mod support;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use common::gm::GmAuthority;
use db::accounts::{find_credentials, gm_authority};
use db::credentials::Login;
use db::store::{Store, StoreConfig};
use support::ScratchDatabase;
use tempfile::TempDir;

const PASSWORD: &str = "s3cret pass";

/// A configuration naming the scratch database, in its own directory.
struct Operator {
    _root: TempDir,
    config: PathBuf,
}

impl Operator {
    fn new(database: &ScratchDatabase) -> Self {
        let root = tempfile::tempdir().expect("a temporary directory should be creatable");
        let config = root.path().join("prodomo.toml");
        fs::write(
            &config,
            format!(
                "[store]\nurl = \"{}\"\n[auth]\nport = 0\n\
                 [[channel]]\nnumber = 1\nports = [0]\nmaps = [1]\n",
                database.url()
            ),
        )
        .expect("the config should be writable");
        Self {
            _root: root,
            config,
        }
    }

    /// Run `prodomo --config <config> <args>` with `stdin` piped in.
    fn run(&self, args: &[&str], stdin: &str) -> Output {
        run(&self.config, args, stdin)
    }

    /// Run a command that must succeed, and return its stdout.
    fn ok(&self, args: &[&str], stdin: &str) -> String {
        let output = self.run(args, stdin);
        let (stdout, stderr) = text(&output);
        assert!(
            output.status.success(),
            "{args:?} failed:\n{stdout}{stderr}"
        );
        assert!(!stdout.contains(PASSWORD) && !stderr.contains(PASSWORD));
        stdout
    }

    /// Run a command that must fail, and return its stderr.
    fn refused(&self, args: &[&str], stdin: &str) -> String {
        let output = self.run(args, stdin);
        let (stdout, stderr) = text(&output);
        assert!(
            !output.status.success(),
            "{args:?} should fail:\n{stdout}{stderr}"
        );
        assert!(
            stdout.is_empty(),
            "a failed command reports nothing on stdout:\n{stdout}"
        );
        stderr
    }
}

fn run(config: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_prodomo"))
        .arg("--config")
        .arg(config)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("prodomo should start");
    let mut input = child.stdin.take().expect("stdin is piped");
    // A command that fails before reading its password closes stdin early.
    drop(input.write_all(stdin.as_bytes()));
    drop(input);
    child.wait_with_output().expect("prodomo should finish")
}

fn text(output: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn inspect<T>(database: &ScratchDatabase, check: impl AsyncFnOnce(&Store) -> T) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime should start")
        .block_on(async {
            let store = Store::connect(&StoreConfig::new(database.url()))
                .await
                .expect("the scratch database should accept a connection");
            let result = check(&store).await;
            store.close().await;
            result
        })
}

#[test]
fn an_operator_creates_an_account_and_changes_its_password() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let operator = Operator::new(&database);

    let created = operator.ok(&["account", "create", "Alice"], &format!("{PASSWORD}\n"));
    let mut lines = created.lines();
    assert_eq!(lines.next(), Some("Created account alice with id 1"));
    let code = lines
        .next()
        .and_then(|line| line.strip_prefix("Delete code: "))
        .expect("a generated delete code is printed");
    assert!(
        code.len() == 7 && code.bytes().all(|byte| byte.is_ascii_digit()),
        "{code:?}"
    );

    let chosen = operator.ok(
        &["account", "create", "bob", "--delete-code", "AbC1234"],
        "pw\r\n",
    );
    assert_eq!(
        chosen, "Created account bob with id 2\n",
        "a chosen code is not printed"
    );

    let (alice, bob) = inspect(&database, async |store| {
        let alice = find_credentials(store, &Login::new("alice").unwrap())
            .await
            .unwrap();
        let bob = find_credentials(store, &Login::new("bob").unwrap())
            .await
            .unwrap();
        (alice.unwrap().1, bob.unwrap().1)
    });
    assert!(alice.verify(PASSWORD.as_bytes()).unwrap());
    assert!(
        bob.verify(b"pw").unwrap(),
        "a CRLF line ending is not part of the password"
    );

    let stderr = operator.refused(&["account", "create", "ALICE"], "other\n");
    assert!(
        stderr.contains("an account with login alice already exists"),
        "{stderr}"
    );
    let stderr = operator.refused(&["account", "create", "x"], "");
    assert!(
        stderr.contains("a login is 2 to 30 ASCII letters and digits"),
        "{stderr}"
    );
    let stderr = operator.refused(&["account", "create", "carol"], "");
    assert!(stderr.contains("no password on standard input"), "{stderr}");

    operator.ok(&["account", "password", "alice"], "n3w\n");
    let alice = inspect(&database, async |store| {
        find_credentials(store, &Login::new("alice").unwrap())
            .await
            .unwrap()
            .unwrap()
            .1
    });
    assert!(alice.verify(b"n3w").unwrap());
    assert!(!alice.verify(PASSWORD.as_bytes()).unwrap());
    let stderr = operator.refused(&["account", "password", "carol"], "pw\n");
    assert!(stderr.contains("no account has login carol"), "{stderr}");
}

#[test]
fn an_operator_adds_and_removes_coins_and_cash() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let operator = Operator::new(&database);
    operator.ok(&["account", "create", "alice"], "pw\n");

    assert_eq!(
        operator.ok(&["account", "coins", "alice", "250"], ""),
        "alice now has 250 coins\n"
    );
    assert_eq!(
        operator.ok(&["account", "coins", "Alice", "-50"], ""),
        "alice now has 200 coins\n"
    );
    let stderr = operator.refused(&["account", "coins", "alice", "-201"], "");
    assert!(stderr.contains("balance 200, change -201"), "{stderr}");
    assert_eq!(
        operator.ok(&["account", "coins", "alice", "0"], ""),
        "alice now has 200 coins\n"
    );

    assert_eq!(
        operator.ok(&["account", "cash", "alice", "7"], ""),
        "alice now has 7 cash\n"
    );
    let stderr = operator.refused(&["account", "cash", "alice", "4294967289"], "");
    assert!(
        stderr.contains("cash would leave 0..=4294967295"),
        "{stderr}"
    );
    let stderr = operator.refused(&["account", "cash", "carol", "1"], "");
    assert!(stderr.contains("no account has login carol"), "{stderr}");
}

#[test]
fn an_operator_grants_lists_and_revokes_gm_authority() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let operator = Operator::new(&database);
    operator.ok(&["account", "create", "alice"], "pw\n");
    operator.ok(&["account", "create", "bob"], "pw\n");

    assert_eq!(operator.ok(&["gm", "list"], ""), "No GM grants\n");
    assert_eq!(
        operator.ok(&["gm", "grant", "alice", "Admin", "god"], ""),
        "Granted GOD to Admin on account alice\n"
    );
    operator.ok(&["gm", "grant", "bob", "Builder", "LOW_WIZARD"], "");
    let listed = operator.ok(&["gm", "list"], "");
    let rows: Vec<Vec<&str>> = listed
        .lines()
        .map(|line| line.split_whitespace().collect())
        .collect();
    assert_eq!(
        rows,
        [["Admin", "alice", "GOD"], ["Builder", "bob", "LOW_WIZARD"]]
    );

    let stderr = operator.refused(&["gm", "grant", "bob", "admin", "wizard"], "");
    assert!(
        stderr.contains("already granted to account alice"),
        "{stderr}"
    );
    let stderr = operator.refused(&["gm", "grant", "alice", "Admin", "player"], "");
    assert!(stderr.contains("expected one of low_wizard"), "{stderr}");

    let authority = inspect(&database, async |store| {
        let login = Login::new("alice").unwrap();
        let (alice, _) = find_credentials(store, &login).await.unwrap().unwrap();
        gm_authority(store, alice, "ADMIN").await.unwrap()
    });
    assert_eq!(authority, Some(GmAuthority::God), "the game sees the grant");

    assert_eq!(
        operator.ok(&["gm", "revoke", "ADMIN"], ""),
        "Revoked the grant of ADMIN\n"
    );
    let stderr = operator.refused(&["gm", "revoke", "Admin"], "");
    assert!(stderr.contains("Admin has no GM grant"), "{stderr}");
}

#[test]
fn an_operator_command_names_the_store_it_cannot_use() {
    let Some(admin_url) = support::database_url() else {
        return;
    };
    let root = tempfile::tempdir().expect("a temporary directory should be creatable");
    let config = root.path().join("prodomo.toml");
    let missing = support::with_database(&admin_url, "prodomo_no_such_database");
    fs::write(
        &config,
        format!(
            "[store]\nurl = \"{missing}\"\n[auth]\nport = 0\n\
             [[channel]]\nnumber = 1\nports = [0]\nmaps = [1]\n"
        ),
    )
    .expect("the config should be writable");
    let output = run(&config, &["gm", "list"], "");
    let (_, stderr) = text(&output);
    assert!(!output.status.success());
    assert!(
        stderr.contains("Store unavailable") && stderr.contains("prodomo_no_such_database"),
        "{stderr}"
    );
    let password = admin_url
        .split_once("://")
        .and_then(|(_, rest)| rest.split_once('@'))
        .and_then(|(user_info, _)| user_info.split_once(':'))
        .map(|(_, password)| password);
    if let Some(password) = password {
        assert!(
            !stderr.contains(password),
            "the password is never printed: {stderr}"
        );
    }
}
