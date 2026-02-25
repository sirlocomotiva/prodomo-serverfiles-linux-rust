//! Live Tokio adapter for the game-to-DB link.
//!
//! [`GameDbClient`](crate::db_client::GameDbClient) decides the frame sequence, the retry window, and client
//! acceptance without owning a socket. This module supplies the socket: one
//! task that connects, writes the two bootstrap frames, reads replies, and
//! reconnects.
//!
//! # Why the link is a task and not a polled descriptor
//!
//! Legacy polls the DB descriptor from the main loop
//! (`server/server/game/main.cpp:783` calls `db_clientdesc->Update()`), so the
//! DB socket competes with every client descriptor inside one thread. The Rust
//! process already runs the accept loop and the client handlers on Tokio
//! tasks, so the DB socket belongs there too. The observable behavior is the
//! same: one attempt per reconnect interval, never a tight retry loop.
//!
//! # What this task deliberately does not do
//!
//! It does not load tables, run SQL, resolve a login, or dispatch gameplay.
//! It validates the boot reply, records the map locations, and moves the
//! [`BootReadyGate`](crate::db_client::BootReadyGate). Each of those is a
//! decision the client state machine already made in transport-free tests, so
//! this file contains no protocol policy of its own.

use std::sync::Arc;
use std::time::Duration;

use crate::db_client::{
    BootReadyGate, DbClientConfig, DbClientEffect, DbClientError, GameDbClient,
};
use protocol::db_setup::LoginOnSetup;
use protocol::db_wire::{DbFrame, DbFrameDecoder, DEFAULT_MAX_DB_PAYLOAD_SIZE};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, watch};
use tracing::{debug, info, warn};

/// How long one `connect()` may take.
///
/// The legacy connect is a blocking `connect()` on the game loop thread, so a
/// black-holed DB stalls the whole process and the retry window stops meaning
/// anything. Bounding it here keeps one attempt inside one interval.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long one DB read may take once the link is up.
///
/// A trusted peer that stops mid-frame must not hold the link open: the
/// boot-ready gate stays closed for exactly as long as the boot reply is
/// outstanding, so an unbounded read would look like a permanently refused
/// client.
const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// Read buffer for the DB link.
///
/// One boot reply is 415 bytes and the framed prefix is nine, so this is ample
/// for the current traffic. The length that actually bounds memory is
/// [`DbFrameDecoder`]'s payload limit, not this buffer.
const READ_BUFFER: usize = 8 * 1024;

/// Configuration for the live DB link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbLinkConfig {
    /// The DB server host: an IPv4 literal or a resolvable name.
    ///
    /// A name, not a [`std::net::SocketAddr`], because the legacy
    /// `socket_connect` uses `gethostbyname` for any host that does not begin
    /// with a digit
    /// (`libthecore/socket.cpp:255-265`). Resolving here rather than at startup
    /// also means a temporarily unresolvable name retries, exactly as legacy
    /// does, instead of aborting the process.
    pub host: String,
    /// The DB server port.
    pub port: u16,
    /// The record values reported in the two bootstrap frames.
    pub client: DbClientConfig,
    /// The exact login records reported in `GD_SETUP`.
    ///
    /// Empty today: the process has no connected-peer registry, so it reports
    /// no other logins. Legacy reports the peers in `CClientManager`.
    pub logins: Vec<LoginOnSetup>,
    /// Largest DB payload the link will buffer.
    ///
    /// Explicit rather than defaulted because a link that trusts a remote
    /// length is the defect the decoder limit exists to bound.
    pub max_payload_size: usize,
}

impl DbLinkConfig {
    /// A config using the protocol's default payload limit and no reported peers.
    #[must_use]
    pub fn new(host: impl Into<String>, port: u16, client: DbClientConfig) -> Self {
        Self {
            host: host.into(),
            port,
            client,
            logins: Vec::new(),
            max_payload_size: DEFAULT_MAX_DB_PAYLOAD_SIZE,
        }
    }
}

/// Why the DB link loop stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbLinkExit {
    /// A shutdown broadcast was received.
    Shutdown,
}

/// The live game-to-DB link.
#[derive(Debug)]
pub struct DbLink {
    config: DbLinkConfig,
    client: GameDbClient,
    gate: Arc<BootReadyGate>,
    /// Observable link state for the accept loop and for tests.
    state_tx: watch::Sender<DbLinkState>,
}

/// Observable state of the DB link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DbLinkState {
    /// The link has been up and `DG_BOOT` has been accepted at least once.
    pub boot_ready: bool,
    /// Consecutive failed connection attempts since the last success.
    pub consecutive_failures: u32,
}

impl DbLink {
    /// Build a link. It has not connected yet.
    #[must_use]
    pub fn new(config: DbLinkConfig, gate: Arc<BootReadyGate>) -> Self {
        let (state_tx, _) = watch::channel(DbLinkState::default());
        let client = GameDbClient::new(config.client.clone());
        Self {
            config,
            client,
            gate,
            state_tx,
        }
    }

    /// Borrow the transport-free client, for tests and diagnostics.
    #[must_use]
    pub const fn client(&self) -> &GameDbClient {
        &self.client
    }

    /// Borrow the gate this link drives.
    #[must_use]
    pub fn gate(&self) -> &Arc<BootReadyGate> {
        &self.gate
    }

    /// Subscribe to the link's observable state.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<DbLinkState> {
        self.state_tx.subscribe()
    }

    fn publish(&self, boot_ready: bool, consecutive_failures: u32) {
        self.state_tx.send_replace(DbLinkState {
            boot_ready,
            consecutive_failures,
        });
    }

    /// Run the connect, retry, and serve loop until shutdown.
    ///
    /// The loop never gives up. A DB that is down at startup must not stop the
    /// game process from starting, and must not stop it from starting later.
    pub async fn run(mut self, mut shutdown: broadcast::Receiver<()>) -> DbLinkExit {
        // A monotonic source, so a system clock adjustment cannot make the
        // retry window appear already-open or already-elapsed.
        let started = tokio::time::Instant::now();
        let mut consecutive_failures = 0_u32;
        loop {
            let now = started.elapsed();
            self.client.set_clock(now);
            if let Err(error) = self.client.connect_allowed(now) {
                // Wait only the remainder of the window rather than a whole
                // interval, so a slow shutdown is not delayed by up to three
                // seconds of pointless sleep.
                let DbClientError::RetryWindowOpen { remaining } = error else {
                    unreachable!("connect_allowed returns only the window error");
                };
                if wait_or_shutdown(remaining, &mut shutdown).await {
                    return DbLinkExit::Shutdown;
                }
                continue;
            }

            // Close before the attempt, not after it fails. Otherwise a
            // reconnect would briefly serve clients on tables the DB has since
            // replaced.
            let outcome = self.attempt().await;
            match outcome {
                Ok(()) => {
                    consecutive_failures = 0;
                    warn!("DB link closed; reconnecting");
                }
                Err(error) => {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    warn!(%error, consecutive_failures, "DB link failed; reconnecting");
                }
            }
            self.client.disconnected();
            self.gate.close();
            self.publish(false, consecutive_failures);

            if shutdown.try_recv().is_ok() {
                return DbLinkExit::Shutdown;
            }
        }
    }

    /// One connection attempt.
    ///
    /// The caller's clock was already stamped by
    /// [`GameDbClient::set_clock`], so [`GameDbClient::on_connect_attempt`]
    /// records this attempt at the right instant. A refused connection still
    /// consumes the retry window exactly as `desc_client.cpp:73-103` does.
    async fn attempt(&mut self) -> Result<(), DbClientError> {
        let effects = self.client.on_connect_attempt(&self.config.logins)?;
        let mut stream = self.connect().await?;

        write_effects(&mut stream, &effects).await?;
        self.client.connected()?;
        info!(
            host = %self.config.host,
            port = self.config.port,
            "Connected to the DB server"
        );

        let (mut reader, mut writer) = stream.into_split();
        let mut decoder = DbFrameDecoder::with_max_payload_size(self.config.max_payload_size);
        let mut buffer = vec![0_u8; READ_BUFFER];
        loop {
            let read = tokio::time::timeout(READ_TIMEOUT, reader.read(&mut buffer))
                .await
                .map_err(|_| DbClientError::Io {
                    context: "reading from the DB server",
                    detail: format!("no complete frame arrived within {READ_TIMEOUT:?}"),
                })?
                .map_err(|error| DbClientError::Io {
                    context: "reading from the DB server",
                    detail: error.to_string(),
                })?;
            if read == 0 {
                return Ok(());
            }
            decoder
                .feed(&buffer[..read])
                .map_err(|error| DbClientError::Io {
                    context: "buffering a DB frame",
                    detail: error.to_string(),
                })?;
            loop {
                let Some(frame) = decoder.try_decode().map_err(|error| DbClientError::Io {
                    context: "decoding a DB frame",
                    detail: error.to_string(),
                })?
                else {
                    break;
                };
                let effects = self.client.on_frame(&frame)?;
                self.apply(&mut writer, effects).await?;
            }
        }
    }

    /// Resolve the host and connect, trying every resolved address.
    ///
    /// # Errors
    ///
    /// Returns [`DbClientError::Io`] if the name does not resolve, no resolved
    /// address answers, or the whole attempt exceeds [`CONNECT_TIMEOUT`].
    async fn connect(&self) -> Result<TcpStream, DbClientError> {
        let target = format!("{}:{}", self.config.host, self.config.port);
        let lookup = tokio::time::timeout(
            CONNECT_TIMEOUT,
            tokio::net::lookup_host((self.config.host.as_str(), self.config.port)),
        )
        .await;
        let resolved = match lookup {
            Err(_) => {
                return Err(DbClientError::Io {
                    context: "resolving the DB server",
                    detail: format!("{target} did not resolve within {CONNECT_TIMEOUT:?}"),
                })
            }
            Ok(Err(error)) => {
                return Err(DbClientError::Io {
                    context: "resolving the DB server",
                    detail: format!("{target}: {error}"),
                })
            }
            Ok(Ok(addresses)) => addresses.collect::<Vec<_>>(),
        };
        if resolved.is_empty() {
            return Err(DbClientError::Io {
                context: "resolving the DB server",
                detail: format!("{target} resolved to no addresses"),
            });
        }

        let mut last = String::new();
        for address in &resolved {
            match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address)).await {
                Ok(Ok(stream)) => {
                    let _ = stream.set_nodelay(true);
                    return Ok(stream);
                }
                Ok(Err(error)) => last = format!("{address}: {error}"),
                Err(_) => last = format!("{address}: no answer within {CONNECT_TIMEOUT:?}"),
            }
        }
        Err(DbClientError::Io {
            context: "connecting to the DB server",
            detail: last,
        })
    }

    /// Apply one batch of client decisions to the socket and the gate.
    async fn apply(
        &mut self,
        writer: &mut tokio::net::tcp::OwnedWriteHalf,
        effects: Vec<DbClientEffect>,
    ) -> Result<(), DbClientError> {
        for effect in effects {
            match effect {
                DbClientEffect::SendFrame(frame) => write_frame(writer, &frame).await?,
                DbClientEffect::BootAccepted { declared_length } => {
                    info!(
                        declared_length,
                        "DB boot reply accepted; clients are now served"
                    );
                    self.gate.open();
                    self.publish(true, 0);
                }
                DbClientEffect::MapLocationsAccepted { records } => {
                    info!(count = records.len(), "DB map locations recorded");
                }
                DbClientEffect::P2PAnnounced { payload } => {
                    debug!(
                        bytes = payload.len(),
                        "DB peer-to-peer announcement received"
                    );
                }
            }
        }
        Ok(())
    }
}

/// Write every frame effect in order, then flush once.
async fn write_effects(
    stream: &mut TcpStream,
    effects: &[DbClientEffect],
) -> Result<(), DbClientError> {
    for effect in effects {
        let DbClientEffect::SendFrame(frame) = effect else {
            continue;
        };
        write_frame(stream, frame).await?;
    }
    stream.flush().await.map_err(|error| DbClientError::Io {
        context: "flushing the DB bootstrap frames",
        detail: error.to_string(),
    })
}

/// Write one frame and flush it.
async fn write_frame<W>(writer: &mut W, frame: &DbFrame) -> Result<(), DbClientError>
where
    W: AsyncWriteExt + Unpin,
{
    let bytes = frame.encode()?;
    writer
        .write_all(&bytes)
        .await
        .map_err(|error| DbClientError::Io {
            context: "writing to the DB server",
            detail: error.to_string(),
        })?;
    writer.flush().await.map_err(|error| DbClientError::Io {
        context: "flushing to the DB server",
        detail: error.to_string(),
    })
}

/// Sleep for `delay`, returning early if shutdown arrives.
async fn wait_or_shutdown(delay: Duration, shutdown: &mut broadcast::Receiver<()>) -> bool {
    tokio::select! {
        () = tokio::time::sleep(delay) => false,
        _ = shutdown.recv() => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{DbLink, DbLinkConfig, DbLinkExit, DbLinkState};
    use crate::db_client::{public_ip_field, BootReadyGate, DbClientConfig};
    use std::net::Ipv4Addr;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::broadcast;

    /// A representative record set: base fields, three allowed maps, ordinary
    /// (non-auth) mode, and the legacy all-zero item-ID range.
    fn config() -> DbClientConfig {
        DbClientConfig {
            listen_port: 8400,
            p2p_port: 8401,
            channel: 1,
            public_ip: public_ip_field("127.0.0.1").expect("address fits the field"),
            map_allow: vec![1, 2, 3],
            auth_server: false,
            item_id_range: [0, 0],
        }
    }

    /// Loopback host plus a port. The host is a name, not a `SocketAddr`,
    /// because that is what the config supplies and what legacy resolves.
    fn loopback(port: u16) -> (String, u16) {
        (Ipv4Addr::LOCALHOST.to_string(), port)
    }

    /// A free loopback port, so parallel tests do not collide.
    fn free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        listener.local_addr().expect("local address").port()
    }

    #[tokio::test]
    async fn a_link_to_a_closed_port_never_opens_the_gate() {
        let port = free_port();
        let gate = Arc::new(BootReadyGate::closed());
        let (tx, rx) = broadcast::channel::<()>(1);
        let link = DbLink::new(
            DbLinkConfig::new(loopback(port).0, loopback(port).1, config()),
            Arc::clone(&gate),
        );
        let state = link.subscribe();
        let task = tokio::spawn(link.run(rx));

        // One full reconnect interval is enough for at least two attempts
        // against a port nothing is listening on.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!gate.is_boot_ready(), "gate opened without a DB reply");
        let snapshot = state.borrow();
        assert!(!snapshot.boot_ready);
        assert!(
            snapshot.consecutive_failures >= 1,
            "no failure was recorded: {snapshot:?}"
        );

        tx.send(()).expect("shutdown receiver alive");
        let exit = task.await.expect("DB link task joined");
        assert_eq!(exit, DbLinkExit::Shutdown);
    }

    #[tokio::test]
    async fn a_refused_connection_does_not_open_the_gate() {
        let port = free_port();
        let gate = Arc::new(BootReadyGate::closed());
        let (tx, rx) = broadcast::channel::<()>(1);
        let link = DbLink::new(
            DbLinkConfig::new(loopback(port).0, loopback(port).1, config()),
            Arc::clone(&gate),
        );
        let task = tokio::spawn(link.run(rx));
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!gate.is_boot_ready());
        tx.send(()).expect("shutdown receiver alive");
        let _ = task.await;
        assert!(!gate.is_boot_ready(), "the gate reopened on shutdown");
    }

    #[tokio::test]
    async fn shutdown_during_the_reconnect_window_stops_the_link() {
        let port = free_port();
        let gate = Arc::new(BootReadyGate::closed());
        let (tx, rx) = broadcast::channel::<()>(1);
        let link = DbLink::new(
            DbLinkConfig::new(loopback(port).0, loopback(port).1, config()),
            Arc::clone(&gate),
        );
        let task = tokio::spawn(link.run(rx));
        // The first attempt consumes the window, so the link is now sleeping
        // for up to three seconds. Shutdown must cut that short.
        tokio::time::sleep(Duration::from_millis(50)).await;
        tx.send(()).expect("receiver alive");
        let exit = tokio::time::timeout(Duration::from_millis(500), task)
            .await
            .expect("shutdown cut the reconnect sleep short")
            .expect("DB link task joined");
        assert_eq!(exit, DbLinkExit::Shutdown);
    }

    #[tokio::test]
    async fn a_new_link_starts_closed_with_an_empty_state() {
        let port = free_port();
        let gate = Arc::new(BootReadyGate::closed());
        let link = DbLink::new(
            DbLinkConfig::new(loopback(port).0, loopback(port).1, config()),
            gate,
        );
        assert!(!link.client().accepts_clients());
        assert_eq!(
            *link.subscribe().borrow(),
            DbLinkState {
                boot_ready: false,
                consecutive_failures: 0
            }
        );
    }

    #[tokio::test]
    async fn a_peer_that_accepts_then_closes_does_not_open_the_gate() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let port = listener.local_addr().expect("local address").port();
        // A peer that accepts and immediately hangs up sends no boot reply.
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            drop(stream);
        });

        let gate = Arc::new(BootReadyGate::closed());
        let (tx, rx) = broadcast::channel::<()>(1);
        let link = DbLink::new(
            DbLinkConfig::new(loopback(port).0, loopback(port).1, config()),
            Arc::clone(&gate),
        );
        let task = tokio::spawn(link.run(rx));

        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!gate.is_boot_ready(), "gate opened on a silent peer");
        let _ = peer.await;
        tx.send(()).expect("receiver alive");
        let _ = task.await;
    }
}
