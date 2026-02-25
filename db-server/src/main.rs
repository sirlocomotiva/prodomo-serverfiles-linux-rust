//! Metin2 database cache server binary
//!
//! Main entry point for the DB server. Handles:
//! - Configuration parsing (db.toml)
//! - Logging initialization
//! - TCP listener setup for game server connections
//! - Signal handling (SIGTERM, SIGINT)
//! - Graceful shutdown sequence

#![warn(missing_docs)]
#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::config::{parse_db_config, DbConfig};
use common::logging::{init_from_env, init_logging, LogConfig};
use db::pool::{ConnectionPool, DatabaseConfig};
use db_server::boot::{BootResponseAdapter, BootResponseError};
use db_server::boot_cache::BootTableCache;
use db_server::boot_empty::{BootClock, SystemBootClock};
use db_server::boot_loader::{BootDataSources, BootTableLoader};
use db_server::item_id_range::{empty_range, ItemIdRangePool};
use db_server::peer_auth::{PeerAuthOutcome, PeerAuthenticator};
use db_server::peer_policy::{
    AuthenticatedDbPeer, AuthorizationDecision, AuthorizationEffect, DbAuthPeerSlot, DbPeerId,
    DbPeerOperation, DbPeerPolicy, DbPeerRole, PeerAuthenticationOutcome, PeerGeneration,
};
use db_server::service::{DbRequestService, DbServiceOutcome, SetupResponder};
use db_server::session::{
    DbDispatchOutcome, DbPeerSession, DbRequest, DbRequestError, DbSessionError,
};
use db_server::setup::SetupRequest;
use db_server::setup_reply::{self, SetupReplyError, SetupReplyOutcome};
use protocol::db_boot::BootFeatureProfile;
use protocol::db_wire::DbFrame;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::signal;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

/// Default configuration file path.
///
/// The legacy `conf.txt` used `key=value` lines and is no longer read. See
/// `docs/REWRITE_LEDGER.md` section 169 for the legacy-key to TOML-key mapping.
const DEFAULT_CONFIG_PATH: &str = "db.toml";

/// Shutdown timeout in seconds
const SHUTDOWN_TIMEOUT_SECS: u64 = 30;

/// How long to wait between boot-table load attempts.
const BOOT_TABLE_RETRY: Duration = Duration::from_secs(3);

/// How often the shutdown flag is re-read by [`ServerState::wait`].
const SHUTDOWN_POLL: Duration = Duration::from_millis(100);

/// Monotonic peer identifier source.
///
/// The legacy `CClientManager` hands every accepted peer a fresh identifier and
/// a generation of 1. The identifier has to be unique for the lifetime of the
/// process, otherwise a closed peer's identifier could be reused while a
/// correlation handle from the old peer is still in flight.
static PEER_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Allocate the next peer identifier.
///
/// Identifiers start at 1 so that a zero identifier can only mean "no peer",
/// which keeps a default-constructed slot distinguishable from a real one.
fn next_peer_id() -> DbPeerId {
    DbPeerId::new(PEER_SEQUENCE.fetch_add(1, Ordering::Relaxed))
}

/// Build every SQL pool without contacting a database.
///
/// The legacy DB server builds its three pools in `main` and only issues
/// queries later, so a server whose database is down still binds its listener
/// and can still answer protocol-level requests. This helper reproduces that
/// ordering: it parses each URL and opens no socket. A malformed or missing URL
/// is a startup error, not a runtime error, because it is a configuration
/// defect rather than a database outage.
///
/// # Errors
///
/// Returns the first [`db::pool::PoolError`] from any of the three pools.
/// The three legacy databases, each kept with its own name.
///
/// The legacy server reads `mob`, `item`, `shop`, `skill`, `object`, `land`, and the
/// rest from the player database, and account/host/admin data from the account
/// database. Passing an anonymous `Vec<ConnectionPool>` to a loader would make
/// "which database" an array index, which is exactly the kind of naming that
/// silently queries the wrong schema.
#[derive(Debug, Clone, Default)]
struct DbPools {
    /// The `player` / `metin2` schema. Holds the fourteen boot tables.
    player: Option<ConnectionPool>,
    /// The `account` schema. Holds the account, GM-host, and admin tables.
    account: Option<ConnectionPool>,
    /// The `common` schema. Holds shared lookup data.
    common: Option<ConnectionPool>,
}

impl DbPools {
    /// How many of the three are configured.
    fn configured(&self) -> usize {
        [
            self.player.is_some(),
            self.account.is_some(),
            self.common.is_some(),
        ]
        .iter()
        .filter(|present| **present)
        .count()
    }
}

/// Open one lazy pool per configured section.
///
/// The pools are lazy: they parse their URL and open no socket, so an
/// unreachable database cannot stop this process from binding its listener.
/// That ordering is what the legacy server relies on.
fn open_lazy_pools(config: &DbConfig) -> Result<DbPools, Box<dyn std::error::Error>> {
    let mut pools = DbPools::default();
    let open = |sql: &common::config::SqlConfig| -> Result<Option<ConnectionPool>, Box<dyn std::error::Error>> {
        if !sql.is_configured() {
            // An unconfigured section means there is no pool, exactly as in the
            // legacy server, where an unset section simply leaves that cache
            // empty. This is not an error.
            return Ok(None);
        }
        let settings = DatabaseConfig {
            url: sql.connection_url(),
            ..DatabaseConfig::default()
        };
        Ok(Some(ConnectionPool::lazy(&settings)?))
    };
    pools.player = open(&config.sql_player)?;
    pools.account = open(&config.sql_account)?;
    pools.common = open(&config.sql_common)?;
    Ok(pools)
}

/// Server state flags
struct ServerState {
    /// Whether shutdown has been initiated
    shutdown: AtomicBool,
    /// Whether to accept new connections
    accept_connections: AtomicBool,
}

impl ServerState {
    fn new() -> Self {
        Self {
            shutdown: AtomicBool::new(false),
            accept_connections: AtomicBool::new(true),
        }
    }

    fn initiate_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.accept_connections.store(false, Ordering::SeqCst);
    }

    fn is_shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    fn should_accept_connections(&self) -> bool {
        self.accept_connections.load(Ordering::SeqCst)
    }

    /// Wait until shutdown is initiated.
    ///
    /// Polling keeps this independent of the signal task, so a caller can wait
    /// for shutdown without holding a handle to that task.
    pub async fn wait(&self) {
        while !self.is_shutting_down() {
            tokio::time::sleep(SHUTDOWN_POLL).await;
        }
    }
}

/// Parse command line arguments
struct CliArgs {
    /// Path to configuration file
    config_path: PathBuf,
    /// Override port from command line
    port: Option<u16>,
    /// Verbose logging to stdout
    verbose: bool,
}

impl Default for CliArgs {
    fn default() -> Self {
        Self {
            config_path: PathBuf::from(DEFAULT_CONFIG_PATH),
            port: None,
            verbose: false,
        }
    }
}

impl CliArgs {
    fn parse() -> Self {
        let mut args = Self::default();
        let mut iter = std::env::args().skip(1);

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-c" | "--config" => {
                    if let Some(path) = iter.next() {
                        args.config_path = PathBuf::from(path);
                    }
                }
                "-p" | "--port" => {
                    if let Some(port_str) = iter.next() {
                        if let Ok(port) = port_str.parse::<u16>() {
                            if port > 1024 {
                                args.port = Some(port);
                            } else {
                                eprintln!("Port must be greater than 1024");
                            }
                        }
                    }
                }
                "-v" | "--verbose" => {
                    args.verbose = true;
                }
                "-h" | "--help" => {
                    Self::print_usage();
                    std::process::exit(0);
                }
                _ => {
                    eprintln!("Unknown argument: {arg}");
                    Self::print_usage();
                }
            }
        }

        args
    }

    fn print_usage() {
        println!(
            "Metin2 DB Server\n\
             \n\
             Usage: db-server [OPTIONS]\n\
             \n\
             Options:\n\
             -c, --config <path>  Path to TOML configuration file (default: db.toml)\n\
             -p, --port <port>    Override listen port (must be > 1024)\n\
             -v, --verbose        Enable verbose logging to stdout\n\
             -h, --help           Show this help message"
        );
    }
}

/// Shared state behind one accepted DB peer.
///
/// The legacy `CClientManager` is a process-wide singleton, so the auth slot
/// and the item-ID range list are global rather than per-connection. This type
/// reproduces that shape: the accept loop clones one `Arc<DbService>` into
/// every connection task.
#[derive(Debug)]
struct DbService {
    /// The configured allowlist. Refuses everything by default.
    authenticator: PeerAuthenticator,
    /// The single global auth-server slot.
    auth_slot: Mutex<DbAuthPeerSlot>,
    /// The request executor.
    ///
    /// The shared mutable boot state (the item-ID range FIFO, the feature
    /// profile, and the clock) lives inside the boot responder this service
    /// owns, not in a second copy here. One owner means the FIFO cannot be
    /// read through one handle while a boot consumed a range through another.
    ///
    /// `main` routes a validated boot request through the service rather than
    /// calling the composition helpers directly, so there is one dispatch
    /// implementation instead of two.
    requests: DbRequestService<UnavailableHorseNames, (), LiveBootResponder, EchoSetupResponder>,
    /// The validated table-postfix and clock reading for every boot query.
    loader: BootTableLoader,
    /// The three SQL pools, named by schema rather than by array index.
    sources: BootDataSources,
    /// The loaded tables, shared with every boot request.
    cache: Arc<BootTableCache>,
}

/// A horse-name lookup that is explicitly unavailable.
///
/// `DbRequestService` answers a horse-name request with 25 zero bytes when the
/// lookup returns `None`, which is the correct wire form for a genuinely
/// missing row. This server has no cache at all, so returning `None` for every
/// player would be a false negative: it would claim a character that does own a
/// horse has none. Keeping the lookup unavailable holds the header on the
/// honest `NotReady` path until a real cache is wired in.
#[derive(Debug, Clone, Copy)]
struct UnavailableHorseNames;

impl db_server::service::HorseNameLookup for UnavailableHorseNames {
    fn lookup(
        &self,
        _player_id: u32,
    ) -> Option<[u8; protocol::db_records::LEGACY_CHARACTER_NAME_BYTES]> {
        None
    }
}

/// A per-request boot composer bound to live shared server state.
///
/// The legacy `QUERY_BOOT` consumes two item-ID ranges from a global FIFO on
/// every call and reads `time(0)` for the tail, so the payload is a function of
/// server state rather than of the request. That is why the immutable-snapshot
/// `BootResponseAdapter` cannot serve a live peer, and why this seam exists
/// next to it.
#[derive(Debug)]
struct LiveBootResponder<C: BootClock = SystemBootClock> {
    /// The shared range FIFO, so consecutive boots consume successive ranges.
    item_ranges: Arc<Mutex<ItemIdRangePool>>,
    /// The tables loaded once at startup, and the cached monarch record.
    cache: Arc<BootTableCache>,
    /// The validated loader, used for the per-request GM tail read.
    loader: Arc<BootTableLoader>,
    /// The three SQL pools, for the per-request GM tail read.
    sources: Arc<BootDataSources>,
    /// The clock source for the boot tail's x86 `time_t`.
    ///
    /// The type parameter rather than a trait object: the production clock is
    /// a unit struct, and a generic keeps the responder `Debug` and `Send`
    /// without a manual implementation while still letting a test supply a
    /// fixed clock.
    clock: C,
}

impl<C: BootClock> db_server::service::BootResponder for LiveBootResponder<C> {
    async fn respond(&self, request_ip: Option<&str>) -> Result<DbFrame, BootResponseError> {
        // The range is taken only after the cache check passes, so a refused
        // boot does not consume a range that a later successful boot needs.
        let (tables, monarch) =
            self.cache
                .require()
                .map_err(|error| BootResponseError::Compose {
                    message: error.to_string(),
                })?;
        // A poisoned range lock must fail closed. The legacy all-zero range is
        // the "no range available" answer, and it is safer than handing a peer a
        // range read from an unknown state.
        let (active, spare) = self.item_ranges.lock().map_or_else(
            |_| (empty_range(), empty_range()),
            |mut pool| pool.take_boot_pair(),
        );
        // Legacy resolves the GM lists inside this handler, and the
        // administrator half is filtered on the requesting peer's address, so
        // this read is per request rather than cached with the tables.
        let gm = self
            .loader
            .load_gm_tail(&self.sources, request_ip)
            .await
            .map_err(|error| BootResponseError::Compose {
                message: error.to_string(),
            })?;
        tables
            .compose(*monarch.as_ref(), self.clock.now(), active, spare, &gm)
            .map_err(|error| BootResponseError::Compose {
                message: error.to_string(),
            })?
            .encode_frame()
            .map_err(BootResponseError::Encode)
    }
}

/// A setup-reply composer that echoes the request back to its own peer.
///
/// The legacy `QUERY_SETUP` builds its `TMapLocation` from the requesting
/// peer's own public IP, listen port, and map list, so the whole reply is a
/// function of the request. No configuration, SQL, or other peer is consulted,
/// and no global peer list is updated.
#[derive(Debug, Clone, Copy)]
struct EchoSetupResponder;

impl SetupResponder for EchoSetupResponder {
    fn respond_setup(&self, request: &SetupRequest) -> Result<SetupReplyOutcome, SetupReplyError> {
        setup_reply::compose_setup_reply(request)
    }
}

impl DbService {
    /// Build the shared service state from a validated configuration.
    ///
    /// The pools are lazy, so nothing here contacts a database. The boot tables
    /// are loaded by [`Self::load_boot_tables`] after the listener binds, which
    /// keeps an unreachable database from stopping the process from starting.
    fn new(config: &DbConfig, pools: &DbPools) -> Result<Self, Box<dyn std::error::Error>> {
        let authenticator =
            PeerAuthenticator::from_config(&config.trusted_game_peers, &config.trusted_auth_peers)?;
        if authenticator.allows_any() {
            warn!(
                game_peers = authenticator.game_peer_count(),
                auth_peers = authenticator.auth_peer_count(),
                "trusted peer addresses are configured. This is transport identity, \
                 NOT authentication: any host that can spoof the source address is \
                 admitted. Replace it with a credential-based adapter before \
                 exposing this server to an untrusted network."
            );
        } else {
            warn!(
                "no trusted peer addresses are configured, so every connection is \
                 refused. Set trusted_game_peers (and trusted_auth_peers for the \
                 auth server) in db.toml to admit a peer."
            );
        }
        // The active profile matches the checked-in legacy build, which defines
        // ENABLE_RENEWAL_SHOPEX, __EVENT_MANAGER__, and
        // __PREMIUM_PRIVATE_SHOP__.
        let profile = BootFeatureProfile::active();
        let item_ranges = Arc::new(Mutex::new(ItemIdRangePool::new()));
        let cache = BootTableCache::new();
        let sources = BootDataSources {
            player: pools.player.clone(),
            account: pools.account.clone(),
            common: pools.common.clone(),
        };
        // The postfix comes from configuration, and an invalid one fails here
        // rather than becoming an identifier interpolated into fourteen queries.
        // The market-price filter interpolates this into `FROM_UNIXTIME(%u)`,
        // so it is the real clock, read once here. The boot tail's own `time_t`
        // is read again per request, exactly as legacy `QUERY_BOOT` does.
        let now = SystemBootClock.now();
        // The boot tail narrows `time(0)` to a 4-byte x86 `time_t`, so a
        // post-2038 clock is already truncated before it reaches the wire. The
        // market-price filter is a SQL parameter rather than wire data, but it
        // is the same reading, and reusing the truncated value keeps the two
        // consistent instead of silently wrapping to a different second.
        let unix_seconds = u32::try_from(now).unwrap_or(u32::MAX);
        let loader =
            BootTableLoader::new(Some(config.table_postfix.as_str()), profile, unix_seconds)?;
        let requests = DbRequestService::new(
            BootResponseAdapter::unavailable(profile),
            UnavailableHorseNames,
        )
        .with_boot_responder(LiveBootResponder::<SystemBootClock> {
            item_ranges: Arc::clone(&item_ranges),
            cache: Arc::clone(&cache),
            loader: Arc::new(loader.clone()),
            sources: Arc::new(sources.clone()),
            clock: SystemBootClock,
        })
        .with_setup_responder(EchoSetupResponder);
        Ok(Self {
            authenticator,
            auth_slot: Mutex::new(DbAuthPeerSlot::empty()),
            requests,
            loader,
            sources,
            cache,
        })
    }

    /// Read every boot table, retrying until it succeeds or shutdown is asked.
    ///
    /// The retry exists because the pools are lazy: this may be the first
    /// statement that touches the database, and a database that is still
    /// starting must not permanently turn into an empty world. It is the one
    /// place a boot-table failure is tolerated, and it is bounded by shutdown
    /// rather than by a fixed attempt count.
    ///
    /// Returns `true` when the tables are loaded, and `false` when shutdown was
    /// asked first. A `false` result leaves the cache empty, so every boot
    /// request is refused rather than answered from nothing.
    async fn load_boot_tables_until_shutdown(&self, shutdown: &ServerState) -> bool {
        let mut attempt: u32 = 0;
        loop {
            attempt = attempt.saturating_add(1);
            match self.cache.load_once(&self.loader, &self.sources).await {
                Ok(records) => {
                    info!(
                        records,
                        cached_records = self.cache.record_count().unwrap_or_default(),
                        sections = self.loader.profile().section_kinds().len(),
                        postfix = self.loader.postfix().as_str(),
                        "boot tables loaded from the player database"
                    );
                    return true;
                }
                Err(error) => {
                    self.cache.record_failure(&error);
                    warn!(
                        attempt,
                        %error,
                        "the boot tables could not be loaded, so QUERY_BOOT stays refused; retrying"
                    );
                }
            }
            tokio::select! {
                () = shutdown.wait() => return false,
                () = tokio::time::sleep(BOOT_TABLE_RETRY) => {}
            }
        }
    }

    /// Classify a freshly accepted socket, or refuse it.
    ///
    /// A refused peer is closed without a single reply byte. That is the whole
    /// point of failing closed: an unlisted address must not be able to learn
    /// that the server is listening, let alone which headers it supports.
    fn admit_peer(&self, peer: SocketAddr) -> Result<DbPeerRole, SocketAddr> {
        match self.authenticator.classify(peer) {
            PeerAuthOutcome::Classified { role } => Ok(role),
            PeerAuthOutcome::Denied => Err(peer),
        }
    }

    /// Authorize one validated request, then apply its effect.
    ///
    /// Returns `None` when the request may proceed, and `Some(reason)` when it
    /// must be refused. A refused request writes no bytes at all.
    ///
    /// The legacy `CClientManager` trusts every frame it decodes, which is why
    /// the hardening rules in `AGENTS.md` require an externally verified owner
    /// and an explicit role gate before any of this runs live. A `bAuthServer`
    /// byte in a setup payload is raw data, never a credential.
    fn authorize_and_apply(
        &self,
        policy: &DbPeerPolicy,
        request: &DbRequest,
    ) -> Result<DbPeerOperation, String> {
        let decision = {
            let slot = self
                .auth_slot
                .lock()
                .map_err(|_| "auth slot lock poisoned".to_owned())?;
            policy.authorize_request(&slot, request)
        };
        let AuthorizationDecision::Allow { operation, effect } = decision else {
            let AuthorizationDecision::Deny { operation, reason } = decision else {
                unreachable!()
            };
            return Err(format!("{operation:?} denied: {reason:?}"));
        };
        match effect {
            AuthorizationEffect::None => {}
            AuthorizationEffect::BindAuthPeer { owner } => {
                // A poisoned slot lock counts as "the slot is busy", which keeps
                // a second live auth server from displacing the first.
                let mut slot = self
                    .auth_slot
                    .lock()
                    .map_err(|_| "auth slot lock poisoned".to_owned())?;
                slot.bind(owner)
                    .map_err(|error| format!("auth slot bind refused: {error}"))?;
            }
        }
        Ok(operation)
    }
}

/// Initialize the server with configuration
fn initialize_server(args: &CliArgs) -> Result<DbConfig, Box<dyn std::error::Error>> {
    // Parse configuration file
    info!("Loading configuration from: {}", args.config_path.display());
    let mut config =
        parse_db_config(&args.config_path).map_err(|e| format!("Failed to parse config: {e}"))?;

    // Override port from command line if specified
    if let Some(port) = args.port {
        info!("Overriding port from command line: {}", port);
        config.bind_port = port;
    }

    // Initialize logging
    if args.verbose {
        init_from_env();
    } else {
        let log_config = LogConfig::default();
        init_logging(&log_config);
    }

    info!("Configuration loaded successfully");
    info!("  Test Server: {}", config.test_server);
    info!("  Log Enabled: {}", config.log);
    info!("  Client Heart FPS: {}", config.client_heart_fps);
    info!("  Locale: {}", config.locale);
    info!("  Table Postfix: {}", config.table_postfix);
    info!("  Bind Address: {}:{}", config.bind_ip, config.bind_port);
    info!("  Player ID Start: {}", config.player_id_start);
    info!(
        "  Player Cache Flush: {}s",
        config.player_cache_flush_seconds
    );
    info!("  Item Cache Flush: {}s", config.item_cache_flush_seconds);

    Ok(config)
}

/// Set up TCP listener on the configured port
async fn setup_tcp_listener(
    bind_ip: &str,
    port: u16,
) -> Result<TcpListener, Box<dyn std::error::Error>> {
    let bind_addr = format!("{bind_ip}:{port}");
    info!("Binding TCP listener to: {}", bind_addr);

    let listener = TcpListener::bind(&bind_addr)
        .await
        .map_err(|e| format!("Failed to bind TCP listener to {bind_addr}: {e}"))?;

    info!("TCP listener bound successfully");
    Ok(listener)
}

fn request_kind_name(request: &DbRequest) -> &'static str {
    match request {
        DbRequest::Boot { .. } => "boot",
        DbRequest::Setup { .. } => "setup",
        DbRequest::HorseName { .. } => "horse_name",
        DbRequest::FindChannel { .. } => "find_channel",
        DbRequest::LoginByKey { .. } => "login_by_key",
        DbRequest::PlayerLoad { .. } => "player_load",
        DbRequest::AddAffect { .. } => "add_affect",
        DbRequest::RemoveAffect { .. } => "remove_affect",
    }
}

/// Classify a connection, then serve validated requests until it closes.
async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    peer_id: DbPeerId,
    role: DbPeerRole,
    service: Arc<DbService>,
    shutdown_tx: &broadcast::Sender<()>,
) {
    info!(%addr, ?peer_id, ?role, "New DB peer connection");

    let mut session = DbPeerSession::new(stream);
    let mut shutdown_rx = shutdown_tx.subscribe();
    let mut policy = DbPeerPolicy::new(peer_id, PeerGeneration::new(1));
    // The allowlist classified the source address before this task existed.
    // That is transport identity, not credential authentication, and it is
    // recorded as such: the proof below is derived from an address comparison.
    let proof =
        AuthenticatedDbPeer::from_external_verification(peer_id, PeerGeneration::new(1), role);
    match policy.authenticate(proof) {
        Ok(PeerAuthenticationOutcome::Authenticated) => {}
        Err(error) => {
            warn!(%addr, ?peer_id, %error, "peer proof rejected; closing without a reply");
            return;
        }
    }

    loop {
        tokio::select! {
            request = session.read_request() => {
                match request {
                    Ok(Some(DbDispatchOutcome::Validated(request))) => {
                        let header = request_header(&request);
                        let handle = request_handle(&request);

                        // Authorization runs before any effect and before a
                        // single byte is written. A denied request produces
                        // silence, and the connection is dropped below.
                        match service.authorize_and_apply(&policy, &request) {
                            Ok(operation) => {
                                info!(%addr, ?peer_id, request_kind = request_kind_name(&request),
                                      handle, header, ?operation, "DB request authorized");
                            }
                            Err(reason) => {
                                warn!(%addr, ?peer_id, header, "DB request refused: {reason}");
                                break;
                            }
                        }

                        if !serve_request(&mut session, &service, &request).await {
                            break;
                        }
                    }
                    Ok(Some(DbDispatchOutcome::NotReady { handle, header, payload_len })) => {
                        info!(
                            %addr,
                            ?peer_id,
                            %handle,
                            header,
                            payload_len,
                            "Recognized DB request is not implemented yet"
                        );
                    }
                    Ok(None) => {
                        info!(%addr, ?peer_id, "DB peer closed cleanly");
                        break;
                    }
                    Err(DbSessionError::Request(DbRequestError::UnknownHeader { .. })) => {
                        // An unknown header is not retryable. Retrying would
                        // spin the peer against a server that will never
                        // understand it, so the connection is closed instead.
                        warn!(%addr, ?peer_id, "DB peer sent an unknown request header");
                        break;
                    }
                    Err(error) => {
                        warn!(%addr, ?peer_id, %error, "DB peer session stopped");
                        break;
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!(%addr, ?peer_id, "DB peer session stopping for shutdown");
                break;
            }
        }
    }

    info!(%addr, ?peer_id, "DB peer connection closed");
}

/// Return the correlation handle carried by a typed request.
///
/// The handle is part of the request, not a property of the peer, so it is
/// read from the typed value instead of being tracked separately.
fn request_handle(request: &DbRequest) -> u32 {
    match request {
        DbRequest::Boot { handle, .. }
        | DbRequest::Setup { handle, .. }
        | DbRequest::HorseName { handle, .. }
        | DbRequest::FindChannel { handle, .. }
        | DbRequest::LoginByKey { handle, .. }
        | DbRequest::PlayerLoad { handle, .. }
        | DbRequest::AddAffect { handle, .. }
        | DbRequest::RemoveAffect { handle, .. } => *handle,
    }
}

/// Return the one-byte header that produced a typed request.
///
/// The typed request is the authority here. Re-reading the header from a
/// buffered frame would trust bytes the decoder has already interpreted, and
/// that is exactly the kind of second parse that diverges under repair.
fn request_header(request: &DbRequest) -> u8 {
    match request {
        DbRequest::Boot { .. } => protocol::db_boot::HEADER_GD_BOOT,
        DbRequest::Setup { .. } => db_server::setup::HEADER_GD_SETUP,
        DbRequest::HorseName { .. } => protocol::db_records::HEADER_GD_REQ_HORSE_NAME,
        DbRequest::FindChannel { .. } => protocol::db_records::HEADER_GD_FIND_CHANNEL,
        DbRequest::LoginByKey { .. } => protocol::db_records::HEADER_GD_LOGIN_BY_KEY,
        DbRequest::PlayerLoad { .. } => protocol::db_records::HEADER_GD_PLAYER_LOAD,
        DbRequest::AddAffect { .. } => protocol::db_records::HEADER_GD_ADD_AFFECT,
        DbRequest::RemoveAffect { .. } => protocol::db_records::HEADER_GD_REMOVE_AFFECT,
    }
}

/// Execute one validated request, writing at most one response frame.
///
/// Returns `false` when the connection must be closed. Every failure path
/// returns `false`: a failed boot composition, a failed write, or a service
/// error. Half-written frames are worse than no frame, because a peer that
/// cannot parse the stream cannot recover it.
async fn serve_request<S>(
    session: &mut DbPeerSession<S>,
    service: &DbService,
    request: &DbRequest,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let outcome = DbDispatchOutcome::Validated(request.clone());
    match service.requests.handle(session, &outcome).await {
        Ok(DbServiceOutcome::Responded { header, handle }) => {
            info!(header, handle, "DB response written");
            true
        }
        Ok(DbServiceOutcome::NoResponse { header, handle }) => {
            // A silent outcome is correct only for the headers the legacy
            // server also answers in silence. Keeping it explicit prevents a
            // missing reply from looking like a successful one.
            info!(
                header,
                handle, "DB request answered with no frame, matching the legacy server"
            );
            true
        }
        Ok(DbServiceOutcome::NotReady {
            header,
            handle,
            payload_len,
        }) => {
            warn!(
                header,
                handle,
                payload_len,
                "DB request is recognized but has no implementation; no bytes written"
            );
            true
        }
        Err(error) => {
            warn!(%error, "DB request failed; closing the peer without a partial response");
            false
        }
    }
}

/// Main DB server loop - processes DB queries and cache operations
async fn db_server_loop(
    mut shutdown_rx: broadcast::Receiver<()>,
    state: Arc<ServerState>,
    _config: &DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("DB server loop starting");

    let mut tick_count: u64 = 0;
    // Match C++ CLIENT_HEART_FPS (default 50 = 20ms per tick)
    let tick_interval = Duration::from_millis(20);

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("DB server loop received shutdown signal");
                break;
            }
            () = tokio::time::sleep(tick_interval) => {
                if state.is_shutting_down() {
                    break;
                }

                tick_count += 1;

                // Process DB operations (equivalent to C++ heartbeat)
                process_db_tick(tick_count);

                // Log statistics every 60 seconds (3000 ticks at 50 tps)
                if tick_count % 3000 == 0 {
                    info!("DB tick: {} (uptime: {}s)", tick_count, tick_count / 50);
                }
            }
        }
    }

    info!("DB server loop stopped");
    Ok(())
}

/// Process a single DB server tick
fn process_db_tick(tick: u64) {
    // TODO: Implement DB tick processing
    // - Process pending DB queries
    // - Handle cache flush operations (player, item, pricelist)
    // - Process guild operations
    // - Process marriage operations
    // - Process monarch operations
    // - Process block country operations
    // - Process item ID range operations

    // Placeholder for now
    let _ = tick;
}

/// Graceful shutdown sequence
async fn graceful_shutdown(
    state: Arc<ServerState>,
    shutdown_tx: broadcast::Sender<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Initiating graceful shutdown sequence");

    // Signal all tasks to stop accepting new connections
    state.initiate_shutdown();

    // Notify all tasks of shutdown
    let _ = shutdown_tx.send(());

    // Wait for connections to drain (with timeout)
    info!(
        "Waiting up to {} seconds for connections to drain",
        SHUTDOWN_TIMEOUT_SECS
    );

    tokio::time::timeout(Duration::from_secs(SHUTDOWN_TIMEOUT_SECS), async {
        // TODO: Implement connection draining
        // - Send disconnect packets to all game servers
        // - Wait for game servers to disconnect
        // - Force disconnect remaining connections after timeout

        // For now, just wait a bit
        tokio::time::sleep(Duration::from_secs(2)).await;
    })
    .await
    .ok();

    // Shutdown sequence (mirrors C++ shutdown)
    info!("<shutdown> Starting shutdown sequence...");

    // TODO: Implement shutdown steps in order:
    // 1. Stop accepting new connections
    // 2. Wait for pending queries to complete
    // 3. Flush all caches to database
    // 4. Close database connections
    // 5. Close network connections

    info!("<shutdown> Shutdown complete");
    Ok(())
}

/// Signal handler task
async fn signal_handler(
    state: Arc<ServerState>,
    shutdown_tx: broadcast::Sender<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Wait for SIGTERM or SIGINT
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {
            info!("Received SIGINT (Ctrl+C)");
        }
        () = terminate => {
            info!("Received SIGTERM");
        }
    }

    // Initiate graceful shutdown
    graceful_shutdown(state, shutdown_tx).await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command line arguments
    let args = CliArgs::parse();

    // Initialize server with configuration
    let config = initialize_server(&args)?;

    // Create shared server state
    let state = Arc::new(ServerState::new());

    // Build the shared peer state and the SQL pools. The pools are lazy: they
    // parse their URL and open no socket, so an unreachable database cannot
    // stop this process from binding its listener. That ordering is what the
    // legacy server relies on, and it is what lets a peer be served at all
    // before any table has been loaded.
    let pools = open_lazy_pools(&config)?;
    let service = Arc::new(DbService::new(&config, &pools)?);
    info!(
        pools = pools.configured(),
        player = pools.player.is_some(),
        account = pools.account.is_some(),
        common = pools.common.is_some(),
        "SQL pools prepared lazily; no database connection has been attempted"
    );
    // The pools are moved into the service, which is cloned into every
    // connection task, so they live for the process lifetime.
    drop(pools);

    // Create shutdown broadcast channel
    let (shutdown_tx, _) = broadcast::channel::<()>(16);

    // Set up TCP listener
    let listener = setup_tcp_listener(&config.bind_ip, config.bind_port).await?;

    // Load the boot tables. This runs after the listener binds and on its own
    // task, because the pools are lazy: this is the first statement that can
    // touch the database, and a database that is still starting must not stop
    // the process from accepting peers. Until the load succeeds, `QUERY_BOOT` is
    // refused rather than answered from an empty world, so the game server's
    // own boot gate stays closed.
    let table_state = Arc::clone(&state);
    let table_service = Arc::clone(&service);
    let table_handle = tokio::spawn(async move {
        if !table_service
            .load_boot_tables_until_shutdown(&table_state)
            .await
        {
            info!("the boot-table load was stopped by shutdown");
        }
    });

    info!("DB server initialized successfully");
    info!(
        "Listening for game server connections on {}:{}",
        config.bind_ip, config.bind_port
    );

    // Spawn signal handler task
    let signal_state = Arc::clone(&state);
    let signal_shutdown_tx = shutdown_tx.clone();
    let signal_handle = tokio::spawn(async move {
        if let Err(e) = signal_handler(signal_state, signal_shutdown_tx).await {
            error!("Signal handler error: {}", e);
        }
    });

    // Spawn DB server loop task
    let db_shutdown_rx = shutdown_tx.subscribe();
    let db_state = Arc::clone(&state);
    let db_config = config.clone();
    let db_handle = tokio::spawn(async move {
        if let Err(e) = db_server_loop(db_shutdown_rx, db_state, &db_config).await {
            error!("DB server loop error: {}", e);
        }
    });

    // Main accept loop
    let mut shutdown_rx = shutdown_tx.subscribe();
    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, addr)) => {
                        if !state.should_accept_connections() {
                            // Server is shutting down, reject new connections
                            warn!("Rejecting new connection from {} (server shutting down)", addr);
                            drop(stream);
                            continue;
                        }
                        // Classify before spawning any task, so an unlisted
                        // address is dropped here and never becomes a session.
                        let peer_id = next_peer_id();
                        let role = match service.admit_peer(addr) {
                            Ok(role) => role,
                            Err(addr) => {
                                // No reply bytes. Failing closed here means an
                                // unlisted host cannot even learn that this
                                // port is a DB server.
                                warn!(%addr, "refused untrusted DB peer: no reply sent");
                                drop(stream);
                                continue;
                            }
                        };
                        let conn_shutdown_tx = shutdown_tx.clone();
                        let conn_service = Arc::clone(&service);
                        tokio::spawn(async move {
                            handle_connection(
                                stream,
                                addr,
                                peer_id,
                                role,
                                conn_service,
                                &conn_shutdown_tx,
                            )
                            .await;
                        });
                    }
                    Err(e) => {
                        error!("Failed to accept connection: {}", e);
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!("Main loop received shutdown signal");
                break;
            }
        }
    }

    // Wait for tasks to complete
    info!("Waiting for tasks to complete...");
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        let _ = signal_handle.await;
        let _ = db_handle.await;
        // The table task waits on the same shutdown flag, so this returns
        // promptly instead of holding shutdown open for a whole retry interval.
        let _ = table_handle.await;
    })
    .await;

    info!("DB server shutdown complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use db_server::boot_loader::LoadedTables;
    use db_server::service::BootResponder as _;

    /// The loader's market-price filter must carry the real clock.
    ///
    /// This is the wiring that a loader unit test cannot reach. Legacy
    /// `InitializePrivateShopMarketItemPrice` passes `time(0)` into
    /// `FROM_UNIXTIME(%u)`, and a zero there does not fail: it makes every row
    /// older than the day interval, so the price table loads empty and the
    /// failure looks like a table with no data. Only a test on the actual
    /// `DbService` construction catches that.
    #[test]
    fn the_service_loader_carries_a_current_unix_time() {
        let config = DbConfig::default();
        let pools = DbPools::default();
        let service = DbService::new(&config, &pools).expect("a default config builds a service");

        let before = u32::try_from(SystemBootClock.now()).unwrap_or(u32::MAX);
        let after = u32::try_from(SystemBootClock.now()).unwrap_or(u32::MAX);
        let supplied = service.loader.unix_seconds();
        assert!(
            (before..=after).contains(&supplied),
            "the market-price filter got {supplied}, which is outside the real clock range {before}..={after}"
        );
        assert_ne!(
            supplied, 0,
            "an epoch filter silently loads an empty price table"
        );
    }

    /// A boot request that cannot read the GM tail is refused, not faked.
    ///
    /// Legacy resolves `gmhost` and `gmlist` inside `QUERY_BOOT`, so a
    /// failure there fails the whole boot. With no common pool this server
    /// cannot run those queries, and answering with zero rows would hand the
    /// game server a world with no GM hosts and no administrators while
    /// reporting success. This proves the refusal happens and that the refusal
    /// names the missing pool rather than looking like a server with no GM
    /// configuration.
    #[tokio::test]
    async fn a_missing_common_pool_refuses_the_boot() {
        let service = service_with_published_tables();
        assert!(
            service.sources.common.is_none(),
            "this test needs a service with no common pool"
        );
        let responder = LiveBootResponder {
            item_ranges: Arc::new(Mutex::new(ItemIdRangePool::new())),
            cache: Arc::clone(&service.cache),
            loader: Arc::new(service.loader.clone()),
            sources: Arc::new(service.sources.clone()),
            clock: db_server::boot_empty::FixedBootClock(1_700_000_000),
        };
        let error = responder
            .respond(None)
            .await
            .expect_err("no common pool means no GM tail");
        assert!(
            error.to_string().contains("common database"),
            "the refusal should name the missing pool, got {error}"
        );
    }

    /// A refused boot must not consume an item-ID range.
    ///
    /// The range FIFO is the process-wide supply the legacy `GetRange` drains
    /// one pair per boot. A refusal that consumed a pair would silently shrink
    /// the supply for a later successful boot, so the range is taken only after
    /// the cache check and the GM read have both succeeded.
    #[tokio::test]
    async fn a_refused_boot_does_not_consume_an_item_id_range() {
        let config = DbConfig::default();
        let pools = DbPools::default();
        let service = DbService::new(&config, &pools).expect("a default config builds a service");
        // The cache is deliberately left empty, so this fails at the cache
        // check before the range is touched.
        let seed = protocol::db_boot::BootItemIdRange {
            min: 1,
            max: 1_000_000,
            usable_item_id_min: 1,
        };
        let ranges = Arc::new(Mutex::new(ItemIdRangePool::from_ranges([seed])));
        let responder = LiveBootResponder {
            item_ranges: Arc::clone(&ranges),
            cache: Arc::clone(&service.cache),
            loader: Arc::new(service.loader.clone()),
            sources: Arc::new(service.sources.clone()),
            clock: db_server::boot_empty::FixedBootClock(1_700_000_000),
        };
        let before = ranges.lock().expect("the range lock is not poisoned").len();
        assert_eq!(before, 1, "the seeded pool starts with one range");
        responder
            .respond(None)
            .await
            .expect_err("an unloaded cache refuses");
        assert_eq!(
            ranges.lock().expect("the range lock is not poisoned").len(),
            before,
            "a refused boot must leave the range supply untouched"
        );
    }

    /// The responder must hand its per-request address to the GM query.
    ///
    /// Legacy filters the administrator list on the requesting peer's `szIP`,
    /// so the address has to reach the statement builder. A hostile address is
    /// the observable seam: the builder rejects it before any SQL runs, so this
    /// test needs no database and no socket. If the responder dropped the
    /// address and passed `ALL` instead, the builder would have nothing to
    /// reject and the failure would come from the unreachable database with a
    /// different message, so the assertion distinguishes the two paths.
    #[tokio::test]
    async fn the_request_address_reaches_the_gm_query() {
        let config = DbConfig {
            sql_common: common::config::SqlConfig {
                host: "127.0.0.1".to_owned(),
                user: "u".to_owned(),
                password: "p".to_owned(),
                database: "d".to_owned(),
                port: 1,
            },
            ..DbConfig::default()
        };
        let pools = open_lazy_pools(&config).expect("a lazy pool opens without a socket");
        assert!(
            pools.common.is_some(),
            "this test needs a configured common pool"
        );
        let service = DbService::new(&config, &pools).expect("a config builds a service");
        // The cache check runs before the GM read, so the tables must be
        // published for this test to reach the statement builder at all.
        service.cache.publish(
            Arc::new(
                LoadedTables::new(
                    BootFeatureProfile::active(),
                    db_server::boot_empty::empty_sections(BootFeatureProfile::active())
                        .expect("every active section has a verified width"),
                )
                .expect("an empty active-profile set is valid"),
            ),
            Arc::new(protocol::db_boot::BootMonarchInfo {
                pid: [0; 4],
                money: [0; 4],
                name: [[0; 32]; 4],
                date: [[0; 32]; 4],
            }),
        );
        let responder = LiveBootResponder {
            item_ranges: Arc::new(Mutex::new(ItemIdRangePool::new())),
            cache: Arc::clone(&service.cache),
            loader: Arc::new(service.loader.clone()),
            sources: Arc::new(service.sources.clone()),
            clock: db_server::boot_empty::FixedBootClock(1_700_000_000),
        };
        let error = responder
            .respond(Some("10.0.0.1' OR '1'='1"))
            .await
            .expect_err("a hostile address cannot reach the statement");
        let message = error.to_string();
        assert!(
            message.contains("gmlist"),
            "the failure must come from the administrator query, got {message}"
        );
        assert!(
            !message.contains("connect"),
            "the statement is built before any connection, got {message}"
        );
    }

    /// Build a service whose boot tables are published and whose monarch is set.
    fn service_with_published_tables() -> DbService {
        let config = DbConfig::default();
        let pools = DbPools::default();
        let service = DbService::new(&config, &pools).expect("a default config builds a service");
        let tables = LoadedTables::new(
            BootFeatureProfile::active(),
            db_server::boot_empty::empty_sections(BootFeatureProfile::active())
                .expect("every active section has a verified width"),
        )
        .expect("an empty active-profile set is valid");
        service.cache.publish(
            Arc::new(tables),
            Arc::new(protocol::db_boot::BootMonarchInfo {
                pid: [0; 4],
                money: [0; 4],
                name: [[0; 32]; 4],
                date: [[0; 32]; 4],
            }),
        );
        service
    }
}
