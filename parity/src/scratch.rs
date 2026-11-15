//! The directories a test writes into, and the sweep that removes what a killed run left.
//!
//! A test that ends, by passing or by panicking, removes its own directory in `Drop`. A test
//! process that is killed (a timeout, a `kill`, a closed terminal) runs no `Drop`, so its
//! directories stay and the servers it started keep running without a parent; thousands of
//! each had gathered by ledger 683. Every scratch directory is therefore named after the
//! process that made it, `prodomo-test-<pid>-<kind>-<n>` in the system's temporary folder,
//! and the first directory a process asks for starts a sweep: a directory whose process is
//! gone is removed, after any process still working inside it is killed.
//!
//! The sweep asks `/proc` which processes live, so it does nothing on a system without one.
//! It assumes a temporary folder is not shared between PID namespaces.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Once;

use rustix::process::{kill_process, Pid, Signal};

/// The start of every scratch directory's name; the owning process's ID follows it.
const PREFIX: &str = "prodomo-test-";

/// A directory of this test's own, removed when it is dropped.
#[derive(Debug)]
pub struct ScratchDir {
    path: PathBuf,
    kept: bool,
}

impl ScratchDir {
    /// Make an empty directory; `kind` says in its name what it is for.
    ///
    /// # Panics
    ///
    /// Panics when the directory cannot be made.
    #[must_use]
    pub fn new(kind: &str) -> Self {
        let path = unique_path(kind);
        fs::create_dir_all(&path).expect("a scratch directory should be creatable");
        Self { path, kept: false }
    }

    /// Where the directory is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Leave the directory in place when this is dropped, for a look at what a test wrote.
    /// The next test process still sweeps it once this one has ended.
    pub fn keep(&mut self) {
        self.kept = true;
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        if !self.kept {
            drop(fs::remove_dir_all(&self.path));
        }
    }
}

/// A path no other test has, for a caller that makes and removes the directory itself. It is
/// named like a [`ScratchDir`], so the sweep removes it if the caller never gets to.
#[must_use]
pub fn unique_path(kind: &str) -> PathBuf {
    static SWEEP: Once = Once::new();
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let temp = std::env::temp_dir();
    // Before this process makes its first directory: anything carrying its own ID is then a
    // dead process's too, one whose ID the system has since handed out again.
    SWEEP.call_once(|| sweep(&temp, std::process::id()));
    temp.join(format!(
        "{PREFIX}{}-{kind}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// The process a scratch directory's name says made it, or `None` for any other name. A
/// working directory that was removed reads as `<name> (deleted)`, which names the same owner.
fn owner(name: &str) -> Option<u32> {
    name.strip_prefix(PREFIX)?.split('-').next()?.parse().ok()
}

/// Whether `path` is a scratch directory in `temp` left by a process that no longer runs.
fn is_stale(temp: &Path, path: &Path, own: u32) -> bool {
    path.parent() == Some(temp)
        && path
            .file_name()
            .and_then(|name| owner(&name.to_string_lossy()))
            .is_some_and(|pid| pid == own || !Path::new("/proc").join(pid.to_string()).exists())
}

/// Kill every process working in a stale scratch directory of `temp`, then remove those
/// directories. `own` is the ID of the process sweeping.
fn sweep(temp: &Path, own: u32) {
    let Ok(processes) = fs::read_dir("/proc") else {
        return;
    };
    // `/proc/<pid>/cwd` names the real folder, so compare against the real temporary folder.
    let real = temp.canonicalize().unwrap_or_else(|_| temp.to_owned());
    for process in processes.flatten() {
        let Some(pid) = process
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
            .and_then(Pid::from_raw)
        else {
            continue;
        };
        // Another user's process refuses the read; it is not one of ours. A process that
        // ended since the listing cannot be killed, which is as good.
        if fs::read_link(process.path().join("cwd")).is_ok_and(|cwd| is_stale(&real, &cwd, own)) {
            let _ = kill_process(pid, Signal::KILL);
        }
    }
    let Ok(entries) = fs::read_dir(temp) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_stale(temp, &path, own) {
            drop(fs::remove_dir_all(&path));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use super::{fs, owner, sweep, unique_path, ScratchDir, PREFIX};

    #[test]
    fn a_name_gives_its_owner_and_any_other_name_gives_none() {
        assert_eq!(owner("prodomo-test-4711-server-12"), Some(4711));
        assert_eq!(owner("prodomo-test-4711-server-12 (deleted)"), Some(4711));
        assert_eq!(owner("prodomo-test-x-server-0"), None);
        assert_eq!(owner("prodomo-parity-4711-0"), None);
        assert_eq!(owner("tmp4711"), None);
    }

    #[test]
    fn a_dropped_directory_is_removed_and_a_kept_one_stays() {
        let dir = ScratchDir::new("drop");
        let path = dir.path().to_owned();
        assert!(path.is_dir());
        drop(dir);
        assert!(!path.exists(), "the drop removes it");

        let mut kept = ScratchDir::new("keep");
        let path = kept.path().to_owned();
        kept.keep();
        drop(kept);
        assert!(path.is_dir(), "a kept directory outlives its guard");
        fs::remove_dir_all(&path).expect("the kept directory is removable");
    }

    /// The sweep on a folder of its own, so that it cannot touch another test's directories:
    /// a dead process's directory goes and the process working in it dies; a live process's
    /// directory and a directory with another name stay.
    #[test]
    fn a_sweep_kills_and_removes_what_a_dead_process_left_and_nothing_else() {
        let temp = ScratchDir::new("sweep");
        let temp = temp.path().canonicalize().expect("the folder exists");
        // A process that has ended: its ID names no process now.
        let mut ended = Command::new("true").spawn().expect("`true` should run");
        let dead = ended.id();
        ended.wait().expect("`true` should end");
        let stale = temp.join(format!("{PREFIX}{dead}-server-0"));
        let live = temp.join(format!("{PREFIX}1-server-0"));
        let other = temp.join(format!("other-{dead}"));
        for dir in [&stale, &live, &other] {
            fs::create_dir(dir).expect("the folder is creatable");
        }
        let orphan = |dir| {
            Command::new("sleep")
                .arg("600")
                .current_dir(dir)
                .stdout(Stdio::null())
                .spawn()
                .expect("`sleep` should run")
        };
        let mut in_stale = orphan(&stale);
        let mut in_live = orphan(&live);

        // The sweeper's own ID is one that owns nothing here.
        sweep(&temp, u32::MAX);

        assert!(!stale.exists(), "the dead process's directory is removed");
        assert!(live.is_dir(), "process 1 lives, so its directory stays");
        assert!(other.is_dir(), "a directory with another name stays");
        let deadline = Instant::now() + Duration::from_secs(10);
        while in_stale.try_wait().expect("a readable status").is_none() {
            assert!(Instant::now() < deadline, "the orphan should be killed");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            in_live.try_wait().expect("a readable status").is_none(),
            "a process in a live directory keeps running"
        );
        in_live.kill().expect("the other `sleep` is killable");
        in_live.wait().expect("the other `sleep` ends");
    }

    #[test]
    fn a_sweep_takes_a_directory_that_carries_the_sweepers_own_id() {
        let temp = ScratchDir::new("own");
        let reused = temp
            .path()
            .join(format!("{PREFIX}{}-server-0", std::process::id()));
        fs::create_dir(&reused).expect("the folder is creatable");
        sweep(temp.path(), std::process::id());
        assert!(!reused.exists());
    }

    #[test]
    fn two_paths_differ() {
        assert_ne!(unique_path("a"), unique_path("a"));
    }
}
