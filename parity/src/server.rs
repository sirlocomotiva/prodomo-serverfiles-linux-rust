//! A real `prodomo serve` process on ports chosen by the operating system.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long a scenario waits for the server to log a line.
pub const PROCESS_TIMEOUT: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

const LISTENING: &str = "Listening for ";
/// The line `prodomo serve` logs when its gate opens.
pub const ACCEPTING: &str = "Accepting clients";

/// One Channel of the configuration a scenario starts the server with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSpec {
    /// The Channel number.
    pub number: u8,
    /// How many listeners the Channel binds (each on a port the operating system picks).
    pub listeners: usize,
    /// The map indexes the Channel hosts.
    pub maps: Vec<u32>,
}

/// The topology every scenario uses unless it needs another: Channel 1 with two listeners and
/// the Shared Channel with one, like the legacy layout in miniature.
#[must_use]
pub fn default_channels() -> Vec<ChannelSpec> {
    vec![
        ChannelSpec {
            number: 1,
            listeners: 2,
            maps: vec![1, 3],
        },
        ChannelSpec {
            number: 99,
            listeners: 1,
            maps: vec![72],
        },
    ]
}

/// A running server. Dropping it kills the process and removes its temporary directory.
pub struct Server {
    child: Child,
    root: PathBuf,
    lines: Receiver<String>,
    console: Vec<String>,
}

impl Server {
    /// Start `binary` against the store at `store_url` with [`default_channels`] and wait until
    /// it accepts clients.
    ///
    /// # Panics
    ///
    /// Panics when the process cannot start, exits early, or does not open its gate within
    /// [`PROCESS_TIMEOUT`]; a scenario cannot continue without a server.
    #[must_use]
    pub fn start(binary: &Path, store_url: &str) -> Self {
        Self::start_with(binary, store_url, &default_channels())
    }

    /// [`Server::start`] with a chosen set of Channels.
    ///
    /// # Panics
    ///
    /// As [`Server::start`].
    #[must_use]
    pub fn start_with(binary: &Path, store_url: &str, channels: &[ChannelSpec]) -> Self {
        let root = unique_root();
        let log_dir = root.join("log");
        fs::create_dir_all(&log_dir).expect("the scenario directory should be creatable");
        let config = root.join("prodomo.toml");
        fs::write(&config, config_text(store_url, channels))
            .expect("the scenario configuration should be writable");
        let mut child = Command::new(binary)
            .arg("--config")
            .arg(&config)
            .arg("serve")
            .arg("--verbose")
            .current_dir(&root)
            .env("LOG_ANSI", "false")
            .env("LOG_DIR", &log_dir)
            .env("RUST_LOG", "info")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("prodomo should start");
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut server = Self {
            child,
            root,
            lines: forward_lines(stdout),
            console: Vec::new(),
        };
        server.wait_for(ACCEPTING);
        server
    }

    /// Collect the console until a line contains `needle`.
    ///
    /// # Panics
    ///
    /// Panics when the process exits first or [`PROCESS_TIMEOUT`] passes.
    pub fn wait_for(&mut self, needle: &str) {
        if self.logged(needle) {
            return;
        }
        let deadline = Instant::now() + PROCESS_TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().expect("status should be readable") {
                self.console.extend(self.lines.try_iter());
                panic!(
                    "prodomo exited before logging {needle:?}: {status}\n{}",
                    self.console()
                );
            }
            match self.lines.recv_timeout(POLL_INTERVAL) {
                Ok(line) => {
                    let found = line.contains(needle);
                    self.console.push(line);
                    if found {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("stdout closed:\n{}", self.console());
                }
            }
            assert!(
                Instant::now() < deadline,
                "prodomo did not log {needle:?}:\n{}",
                self.console()
            );
        }
    }

    /// Whether a line collected so far contains `needle`.
    #[must_use]
    pub fn logged(&self, needle: &str) -> bool {
        self.console.iter().any(|line| line.contains(needle))
    }

    /// The console collected so far, for a failure message.
    #[must_use]
    pub fn console(&self) -> String {
        self.console.join("\n")
    }

    /// The listener addresses by role (`auth`, `channel 1`, ...), in the order they were bound.
    ///
    /// # Panics
    ///
    /// Panics on a listener line this harness cannot parse.
    #[must_use]
    pub fn listeners(&self) -> BTreeMap<String, Vec<SocketAddr>> {
        let mut found: BTreeMap<String, Vec<SocketAddr>> = BTreeMap::new();
        for line in &self.console {
            let Some(start) = line.find(LISTENING) else {
                continue;
            };
            let (role, addr) = line[start + LISTENING.len()..]
                .split_once(" clients on ")
                .unwrap_or_else(|| panic!("unexpected listener line {line:?}"));
            let addr = addr
                .trim()
                .parse()
                .expect("the logged address should parse");
            found.entry(role.to_owned()).or_default().push(addr);
        }
        found
    }

    /// The auth listener.
    ///
    /// # Panics
    ///
    /// Panics when the console names no auth listener.
    #[must_use]
    pub fn auth(&self) -> SocketAddr {
        self.role("auth")[0]
    }

    /// The first listener of Channel `number`.
    ///
    /// # Panics
    ///
    /// Panics when the console names no listener for that Channel.
    #[must_use]
    pub fn channel(&self, number: u8) -> SocketAddr {
        self.role(&format!("channel {number}"))[0]
    }

    fn role(&self, role: &str) -> Vec<SocketAddr> {
        self.listeners()
            .remove(role)
            .unwrap_or_else(|| panic!("no {role} listener in:\n{}", self.console()))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        drop(self.child.kill());
        drop(self.child.wait());
        drop(fs::remove_dir_all(&self.root));
    }
}

fn unique_root() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock should follow the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "prodomo-parity-{}-{}-{nanos}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// The `prodomo.toml` for a scenario: every port 0, so the operating system picks free ones.
fn config_text(store_url: &str, channels: &[ChannelSpec]) -> String {
    let mut text =
        format!("bind_ip = \"127.0.0.1\"\n\n[store]\nurl = \"{store_url}\"\n\n[auth]\nport = 0\n");
    for channel in channels {
        let ports = vec!["0"; channel.listeners].join(", ");
        let maps: Vec<String> = channel.maps.iter().map(u32::to_string).collect();
        write!(
            text,
            "\n[[channel]]\nnumber = {}\nports = [{ports}]\nmaps = [{}]\n",
            channel.number,
            maps.join(", ")
        )
        .expect("writing to a String cannot fail");
    }
    text
}

fn forward_lines(stdout: ChildStdout) -> Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configuration_names_every_channel_with_port_zero() {
        let text = config_text("postgres://h/db", &default_channels());
        assert_eq!(
            text,
            "bind_ip = \"127.0.0.1\"\n\n[store]\nurl = \"postgres://h/db\"\n\n[auth]\nport = 0\n\
             \n[[channel]]\nnumber = 1\nports = [0, 0]\nmaps = [1, 3]\n\
             \n[[channel]]\nnumber = 99\nports = [0]\nmaps = [72]\n"
        );
    }
}
