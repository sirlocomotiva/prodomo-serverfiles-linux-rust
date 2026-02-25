//! Transport-free game-to-DB client state machine, retry policy, and
//! boot-ready gate.
//!
//! This module is the game half of the vertical slice. It decides *what* the
//! game core sends to the DB peer and *when* it may accept a client, and it
//! does so without owning a socket, a timer, or a table cache.
//!
//! # Legacy evidence
//!
//! * `server/server/game/desc_client.cpp:126-226` is the only place that
//!   writes `HEADER_GD_BOOT` and `HEADER_GD_SETUP`. Both are sent from
//!   `CLIENT_DESC::SetPhase(PHASE_DBCLIENT)`, in that order, both with handle
//!   `0` (`desc_client.cpp:141` and `desc_client.cpp:223`).
//! * `server/server/game/desc_client.cpp:73-103` throttles reconnect attempts
//!   to one per three seconds of game time, and stamps the attempt time
//!   *before* the connect call, so a failed connect still consumes the window.
//! * `server/server/game/main.cpp:671-697` binds and listens on the client
//!   port, arms the read watch, and only then constructs the DB connector.
//! * `server/server/game/main.cpp:840-856` calls `TryConnect()` and discards
//!   its result, then calls `AcceptDesc()` unconditionally every pass.
//! * `server/server/game/desc_manager.cpp:150-198` accepts any descriptor with
//!   no boot check. The only rejections are `accept()` failure, the auth-mode
//!   ban-IP list, and the admin-IP user-count gate.
//! * `server/server/db/ClientManager.cpp:1395,1405,1431` answer `GD_SETUP`
//!   with `HEADER_DG_MAP_LOCATIONS` then `HEADER_DG_P2P`, all at handle 0.
//! * `server/server/game/input_db.cpp:1940-1942` dispatches `HEADER_DG_BOOT`
//!   (byte 43) to `CInputDB::Boot`, and `:2007-2012` dispatches
//!   `HEADER_DG_MAP_LOCATIONS` (byte 0xfe) and `HEADER_DG_P2P` (byte 0xff).
//!
//! # Deliberate divergences from legacy
//!
//! These are hardening decisions, not compatibility losses. Each is named so a
//! reviewer can check it against the legacy source above.
//!
//! 1. **Clients are refused until boot completes.** Legacy accepts, handshakes,
//!    and logs players in while the DB connector is still retrying, because no
//!    readiness flag exists anywhere in the tree. This gate is the one place the
//!    rewrite knowingly differs. See [`BootReadyGate`](crate::db_client::BootReadyGate).
//! 2. **`GD_BOOT` is resent on every reconnect.** Legacy guards the boot send
//!    with a function-local `static bool bSentBoot` (`desc_client.cpp:132`)
//!    that is never reset, so a DB restart leaves the game running on the
//!    original tables and never re-requests the item-ID range. A fresh boot per
//!    connection is what the wire sequence plainly describes.
//! 3. **An unknown DB header closes the connection.** Legacy logs it and
//!    consumes the frame (`input_db.cpp:2273-2280`). For a trusted-peer link the
//!    rewrite treats an unrecognised header as a protocol fault rather than
//!    silently continuing to parse.
//! 4. **The map-location record count is capped.** Legacy trusts the one-byte
//!    count and would read `255 * 146` bytes (`input_db.cpp:1270-1290`).
//! 5. **The boot payload length is validated.** Legacy reads the self-declared
//!    packet size at `input_db.cpp:463` and never compares it to anything; the
//!    frame length is dropped at `input_db.cpp:2273` and never reaches a handler.
//! 6. **The frame length is read as `u32`.** Legacy narrows it to `int`
//!    (`input_db.cpp:2255,2264`), so a length at or above `0x8000_0000` makes the
//!    framing loop run backwards and read out of bounds.
//!
//! This module performs no SQL, holds no table cache, and does not run any
//! gameplay. It emits the frames to write and reports the frames it received.

use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use protocol::db_boot::{
    parse_db_boot_frame, BootFeatureProfile, DbBootRequest, HEADER_DG_BOOT, HEADER_GD_BOOT,
};
use protocol::db_map_locations::{
    MapLocation, MapLocationsError, MapLocationsReply, MAP_LOCATIONS_REPLY_HANDLE,
};
use protocol::db_setup::{
    encode_setup_payload, LoginOnSetup, SetupBase, SetupEncodeError, HEADER_GD_SETUP,
};
use protocol::db_wire::{DbFrame, DbFrameError};

/// Minimum interval between two connect attempts, matching legacy.
///
/// `server/server/game/desc_client.cpp:73-103` compares against
/// `get_global_time()` plus three seconds. The game loop runs at 40 ms, so the
/// window is checked in whole seconds there; this module takes a `Duration` so
/// the caller keeps its own clock.
pub const DB_RECONNECT_INTERVAL: Duration = Duration::from_secs(3);

/// Handle written on every frame the game core sends to the DB peer.
///
/// `desc_client.cpp:141` and `desc_client.cpp:223` both pass handle `0`, and
/// `ClientManager.cpp:1395,1405,1431` answer at handle `0`. The bootstrap
/// path has no per-request correlation, so any non-zero handle is a fault.
pub const DB_BOOTSTRAP_HANDLE: u32 = 0;

/// Raw width of the 16-byte `szPublicIP` field in `TPacketGDSetup`.
pub const SETUP_PUBLIC_IP_BYTES: usize = 16;

/// Copy an IPv4 string into the 16-byte legacy `szPublicIP` field.
///
/// The field is a NUL-terminated `char[16]`, so the longest usable string is
/// fifteen characters plus the terminator. A longer string is refused rather
/// than silently truncated, because a truncated address points at a different
/// host.
///
/// # Errors
///
/// Returns [`DbClientError::PublicIpTooLong`] when the text cannot fit.
pub fn public_ip_field(text: &str) -> Result<[u8; SETUP_PUBLIC_IP_BYTES], DbClientError> {
    let bytes = text.as_bytes();
    if bytes.len() >= SETUP_PUBLIC_IP_BYTES {
        return Err(DbClientError::PublicIpTooLong {
            length: bytes.len(),
        });
    }
    let mut field = [0_u8; SETUP_PUBLIC_IP_BYTES];
    field[..bytes.len()].copy_from_slice(bytes);
    Ok(field)
}

/// Reject a frame whose payload cannot be expressed on the wire.
///
/// `DbFrame::new` is infallible, so a length overflow is only detectable when
/// the frame is encoded. Encoding here means the error is reported to the
/// caller as a typed failure instead of panicking or truncating in the socket
/// writer.
fn try_frame(frame: DbFrame) -> Result<DbFrame, DbClientError> {
    frame.encode().map_err(DbClientError::from)?;
    Ok(frame)
}

/// The connection phase of the game-to-DB link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbConnectionPhase {
    /// No connection has been attempted yet.
    Idle,
    /// A connect attempt is in flight.
    Connecting,
    /// The socket is open and `GD_BOOT` has been written; no boot reply yet.
    AwaitingBoot,
    /// A `DG_BOOT` reply was accepted. The client gate is open.
    Booted,
    /// The link is down. [`DbConnectionPhase::is_retryable`] says whether a
    /// reconnect is worth attempting.
    Failed,
}

impl DbConnectionPhase {
    /// Whether the client-acceptance gate is open in this phase.
    #[must_use]
    pub const fn accepts_clients(self) -> bool {
        matches!(self, Self::Booted)
    }

    /// Whether the phase has a live socket.
    #[must_use]
    pub const fn is_connected(self) -> bool {
        matches!(self, Self::AwaitingBoot | Self::Booted)
    }

    /// Whether a fresh connect attempt may be started from this phase.
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::Idle | Self::Failed)
    }
}

/// One ordered effect the caller must perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbClientEffect {
    /// Write this frame to the DB peer socket.
    ///
    /// Boot and setup are two separate frames in two separate writes, matching
    /// `desc_client.cpp:141` and `desc_client.cpp:223`. They are separate
    /// effects rather than one buffer so a partial write cannot interleave a
    /// later frame into the middle of the setup payload.
    SendFrame(DbFrame),
    /// A `DG_BOOT` reply passed validation. The client gate is now open.
    BootAccepted {
        /// Payload length the DB declared, cross-checked against the frame.
        declared_length: u32,
    },
    /// A `DG_MAP_LOCATIONS` reply passed validation.
    MapLocationsAccepted {
        /// Records, in the order the DB sent them.
        records: Vec<MapLocation>,
    },
    /// A `DG_P2P` announcement arrived. The payload is opaque here.
    ///
    /// The rewrite does not yet build a peer-to-peer map, so the payload is
    /// reported rather than interpreted. It is not discarded silently.
    P2PAnnounced {
        /// Opaque announcement payload.
        payload: Vec<u8>,
    },
}

/// An error raised by the game-side DB client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbClientError {
    /// A connect attempt was requested before the retry window elapsed.
    RetryWindowOpen {
        /// Time still to wait.
        remaining: Duration,
    },
    /// The configured public IP cannot be represented in `szPublicIP`.
    PublicIpTooLong {
        /// Length of the supplied text in bytes.
        length: usize,
    },
    /// The configured setup payload could not be encoded.
    SetupEncode(SetupEncodeError),
    /// A map-location reply could not be decoded.
    MapLocations(MapLocationsError),
    /// A frame could not be encoded for the peer.
    Frame(DbFrameError),
    /// A frame header arrived that this client does not implement.
    UnsupportedHeader {
        /// The received header byte.
        header: u8,
    },
    /// A bootstrap-path frame arrived with a handle other than zero.
    UnexpectedHandle {
        /// The received handle.
        handle: u32,
        /// The header whose handle was wrong.
        header: u8,
    },
    /// A `DG_BOOT` reply failed its own length or version check.
    InvalidBootReply {
        /// What specifically was wrong.
        detail: String,
    },
    /// The setup base field count exceeded the legacy record width.
    TooManyLoginRecords {
        /// Number of records the caller supplied.
        count: usize,
    },
    /// A live socket operation failed.
    ///
    /// The transport-free client never produces this variant. It exists so the
    /// live adapter can report the real I/O cause instead of flattening every
    /// failure into a retry hint and losing the reason.
    Io {
        /// What was being attempted.
        context: &'static str,
        /// The underlying error text.
        detail: String,
    },
}

impl fmt::Display for DbClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RetryWindowOpen { remaining } => write!(
                f,
                "the DB reconnect window is still open for another {remaining:?}"
            ),
            Self::PublicIpTooLong { length } => {
                write!(f, "a {length}-byte public IP does not fit szPublicIP[16]")
            }
            Self::SetupEncode(inner) => write!(f, "setup payload encoding failed: {inner}"),
            Self::MapLocations(inner) => {
                write!(f, "map-location reply decoding failed: {inner}")
            }
            Self::Frame(inner) => write!(f, "DB frame encoding failed: {inner}"),
            Self::UnsupportedHeader { header } => {
                write!(f, "unsupported DB header 0x{header:02x}")
            }
            Self::UnexpectedHandle { handle, header } => write!(
                f,
                "DB header 0x{header:02x} arrived with handle {handle}, not 0"
            ),
            Self::InvalidBootReply { detail } => write!(f, "invalid DG_BOOT reply: {detail}"),
            Self::TooManyLoginRecords { count } => {
                write!(
                    f,
                    "{count} login records exceed the 32-bit dwLoginCount field"
                )
            }
            Self::Io { context, detail } => write!(f, "{context} failed: {detail}"),
        }
    }
}

impl Error for DbClientError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::SetupEncode(inner) => Some(inner),
            Self::MapLocations(inner) => Some(inner),
            Self::Frame(inner) => Some(inner),
            _ => None,
        }
    }
}

impl From<SetupEncodeError> for DbClientError {
    fn from(value: SetupEncodeError) -> Self {
        Self::SetupEncode(value)
    }
}

impl From<MapLocationsError> for DbClientError {
    fn from(value: MapLocationsError) -> Self {
        Self::MapLocations(value)
    }
}

impl From<DbFrameError> for DbClientError {
    fn from(value: DbFrameError) -> Self {
        Self::Frame(value)
    }
}

/// Immutable settings for the game-to-DB link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbClientConfig {
    /// `wListenPort` reported to the DB in the setup base record.
    pub listen_port: u16,
    /// `wP2PPort` reported to the DB in the setup base record.
    pub p2p_port: u16,
    /// `bChannel` reported to the DB in the setup base record.
    pub channel: u8,
    /// `szPublicIP`, already packed into its 16-byte field.
    pub public_ip: [u8; SETUP_PUBLIC_IP_BYTES],
    /// `alMaps`, the allowed map indices, in configured order.
    pub map_allow: Vec<i32>,
    /// `bAuthServer`. An auth-mode setup is base only.
    pub auth_server: bool,
    /// `dwItemIDRange[0]` and `dwItemIDRange[1]`. Legacy sends both as zero.
    pub item_id_range: [u32; 2],
}

impl DbClientConfig {
    /// Build the base record this configuration describes.
    ///
    /// `login_count` is left at zero: [`GameDbClient`] derives the wire count
    /// from the record list it is actually given, so the two cannot disagree.
    #[must_use]
    pub fn base(&self) -> SetupBase {
        let mut maps = [0_i32; protocol::db_setup::SETUP_MAP_LIMIT];
        for (slot, index) in self.map_allow.iter().copied().enumerate() {
            // The 33rd configured index would alias `dwLoginCount` in the packed
            // record. Legacy has this bug (defect 7 in the M3 audit: the store
            // precedes the bound test in `config.cpp:274-287`). Dropping the
            // extra entry is the deliberate divergence; it is not a silent fix
            // of the legacy server.
            if slot >= maps.len() {
                break;
            }
            maps[slot] = index;
        }
        SetupBase {
            public_ip: self.public_ip,
            channel: self.channel,
            listen_port: self.listen_port,
            p2p_port: self.p2p_port,
            maps,
            login_count: 0,
            auth_server: u8::from(self.auth_server),
        }
    }

    /// Return the `GD_BOOT` request record this configuration describes.
    #[must_use]
    pub fn boot_request(&self) -> DbBootRequest {
        DbBootRequest::new(self.item_id_range, self.public_ip)
    }
}

/// The transport-free game-to-DB client.
///
/// The caller owns the socket. This type decides the frame sequence, the retry
/// window, and when a game client may be accepted. It holds no I/O, so every
/// branch is reachable from a unit test.
#[derive(Debug)]
pub struct GameDbClient {
    config: DbClientConfig,
    phase: DbConnectionPhase,
    last_attempt: Option<Duration>,
    /// The client's current retry-clock reading, set by [`GameDbClient::set_clock`].
    clock: Duration,
    /// Locations learned from the setup reply, in the order the DB sent them.
    locations: Vec<MapLocation>,
    /// The two item-ID ranges the boot reply carried, when it has been parsed.
    boot_item_id_ranges: Option<[u32; 6]>,
}

impl GameDbClient {
    /// Create an idle client. No connection has been attempted.
    #[must_use]
    pub const fn new(config: DbClientConfig) -> Self {
        Self {
            config,
            phase: DbConnectionPhase::Idle,
            last_attempt: None,
            clock: Duration::ZERO,
            locations: Vec::new(),
            boot_item_id_ranges: None,
        }
    }

    /// The current connection phase.
    #[must_use]
    pub const fn phase(&self) -> DbConnectionPhase {
        self.phase
    }

    /// The configuration this client was built with.
    #[must_use]
    pub const fn config(&self) -> &DbClientConfig {
        &self.config
    }

    /// The map locations learned from the last setup reply.
    #[must_use]
    pub fn locations(&self) -> &[MapLocation] {
        &self.locations
    }

    /// The six item-ID range words from the boot reply, once parsed.
    #[must_use]
    pub const fn boot_item_id_ranges(&self) -> Option<[u32; 6]> {
        self.boot_item_id_ranges
    }

    /// Whether a client socket may be accepted right now.
    #[must_use]
    pub const fn accepts_clients(&self) -> bool {
        self.phase.accepts_clients()
    }

    /// Decide whether a connect attempt may start at `now`.
    ///
    /// Legacy stamps the attempt time *before* connecting
    /// (`desc_client.cpp:73-103`), so a refused connection still consumes the
    /// window. This preserves that: the caller calls [`GameDbClient::begin_connect`]
    /// on every attempt, and the window is consumed whether the connect
    /// succeeds or fails.
    ///
    /// # Errors
    ///
    /// Returns [`DbClientError::RetryWindowOpen`] while the three-second window
    /// is still running, and refuses a connect from a phase that already has a
    /// live socket.
    pub fn connect_allowed(&self, now: Duration) -> Result<(), DbClientError> {
        if !self.phase.is_retryable() {
            return Err(DbClientError::RetryWindowOpen {
                remaining: Duration::ZERO,
            });
        }
        if let Some(last) = self.last_attempt {
            let elapsed = now.checked_sub(last).unwrap_or(Duration::ZERO);
            if elapsed < DB_RECONNECT_INTERVAL {
                return Err(DbClientError::RetryWindowOpen {
                    remaining: DB_RECONNECT_INTERVAL - elapsed,
                });
            }
        }
        Ok(())
    }

    /// Begin a connect attempt and return the frames to write once the socket
    /// opens.
    ///
    /// The two bootstrap frames are returned together, boot first, matching the
    /// append order at `desc_client.cpp:141` then `desc_client.cpp:223`. An
    /// auth-mode client sends the base-only setup and no boot request, because
    /// `desc_client.cpp:130` skips the boot block entirely.
    ///
    /// `logins` is the exact list the caller is reporting. The wire count is
    /// derived from its length, never from a caller-supplied field.
    ///
    /// # Errors
    ///
    /// Returns [`DbClientError::RetryWindowOpen`] if the window is still open,
    /// and the encode errors from the setup payload.
    pub fn begin_connect(
        &mut self,
        now: Duration,
        logins: &[LoginOnSetup],
    ) -> Result<Vec<DbClientEffect>, DbClientError> {
        self.connect_allowed(now)?;
        self.clock = now;
        self.on_connect_attempt(logins)
    }

    /// Record a connect attempt and produce the bootstrap frames.
    ///
    /// This is [`GameDbClient::begin_connect`] without the retry-window check.
    /// The live adapter needs the split because it must stamp the attempt
    /// *before* the socket call, so that a refused connection still consumes the
    /// window exactly as `desc_client.cpp:73-103` does. Calling
    /// `begin_connect` twice for one attempt would stamp two timestamps and
    /// double the effective backoff.
    ///
    /// # Errors
    ///
    /// Returns the encode errors from the setup payload, including
    /// [`DbClientError::AuthServerMustBeBaseOnly`](SetupEncodeError::AuthServerMustBeBaseOnly)
    /// for a non-empty record list in auth mode.
    pub fn on_connect_attempt(
        &mut self,
        logins: &[LoginOnSetup],
    ) -> Result<Vec<DbClientEffect>, DbClientError> {
        // Stamp before the attempt, exactly as legacy does.
        self.last_attempt = Some(self.clock);
        self.phase = DbConnectionPhase::Connecting;
        self.locations.clear();
        self.boot_item_id_ranges = None;

        let mut effects = Vec::new();
        if !self.config.auth_server {
            let request = self.config.boot_request();
            effects.push(DbClientEffect::SendFrame(try_frame(DbFrame::new(
                HEADER_GD_BOOT,
                DB_BOOTSTRAP_HANDLE,
                request.encode().to_vec(),
            ))?));
        }
        let payload = encode_setup_payload(&self.config.base(), logins)?;
        effects.push(DbClientEffect::SendFrame(try_frame(DbFrame::new(
            HEADER_GD_SETUP,
            DB_BOOTSTRAP_HANDLE,
            payload,
        ))?));
        Ok(effects)
    }

    /// Set the client's retry clock.
    ///
    /// [`GameDbClient::begin_connect`] sets the clock from its own `now`
    /// argument. The live adapter instead calls
    /// [`GameDbClient::connect_allowed`] and
    /// [`GameDbClient::on_connect_attempt`] as separate steps, so it records its
    /// monotonic reading here first. Without this, an attempt would be stamped
    /// at the zero the constructor used.
    pub const fn set_clock(&mut self, now: Duration) {
        self.clock = now;
    }

    /// The socket is open. Move to [`DbConnectionPhase::AwaitingBoot`].
    ///
    /// # Errors
    ///
    /// Returns [`DbClientError::RetryWindowOpen`] if the client is not in
    /// [`DbConnectionPhase::Connecting`], which means the caller skipped
    /// [`GameDbClient::begin_connect`].
    pub fn connected(&mut self) -> Result<(), DbClientError> {
        if self.phase != DbConnectionPhase::Connecting {
            return Err(DbClientError::RetryWindowOpen {
                remaining: Duration::ZERO,
            });
        }
        self.phase = DbConnectionPhase::AwaitingBoot;
        Ok(())
    }

    /// The socket closed. Return to [`DbConnectionPhase::Failed`].
    ///
    /// The next attempt is then subject to the same three-second window, so a
    /// flapping DB does not spin.
    pub fn disconnected(&mut self) {
        self.phase = DbConnectionPhase::Failed;
        self.locations.clear();
    }

    /// Handle one frame received from the DB peer.
    ///
    /// # Errors
    ///
    /// Returns [`DbClientError::UnsupportedHeader`] for a header this client
    /// does not implement, [`DbClientError::UnexpectedHandle`] for a
    /// bootstrap-path frame at a non-zero handle, and
    /// [`DbClientError::InvalidBootReply`] for a boot reply that fails its own
    /// length or version check. In every one of those cases the returned
    /// effects close the link, which is the divergence from the legacy
    /// log-and-continue behavior.
    pub fn on_frame(&mut self, frame: &DbFrame) -> Result<Vec<DbClientEffect>, DbClientError> {
        match frame.header {
            HEADER_DG_BOOT => self.on_boot(frame),
            protocol::db_map_locations::HEADER_DG_MAP_LOCATIONS => self.on_map_locations(frame),
            protocol::db_map_locations::HEADER_DG_P2P => Ok(vec![DbClientEffect::P2PAnnounced {
                payload: frame.payload.clone(),
            }]),
            other => {
                // Legacy logs an unknown header and consumes the frame
                // (input_db.cpp:2273-2280), then keeps parsing the rest of the
                // stream. On a trusted-peer link an unrecognized header means
                // the two sides disagree about the protocol, so the link is
                // closed instead.
                self.disconnected();
                Err(DbClientError::UnsupportedHeader { header: other })
            }
        }
    }

    fn on_boot(&mut self, frame: &DbFrame) -> Result<Vec<DbClientEffect>, DbClientError> {
        // `parse_db_boot_frame` cross-checks the header, the zero handle, the
        // self-declared length word, the version word, and the width word of
        // all fourteen table sections. Legacy does almost none of this: it
        // reads the declared length and only logs it (input_db.cpp:463,468),
        // never compares it, and walks the sections by raw pointer arithmetic
        // (divergence 5).
        let payload = match parse_db_boot_frame(frame, BootFeatureProfile::active()) {
            Ok(payload) => payload,
            Err(error) => {
                self.disconnected();
                return Err(DbClientError::InvalidBootReply {
                    detail: error.to_string(),
                });
            }
        };
        self.boot_item_id_ranges = Some([
            payload.item_id_ranges.active.min,
            payload.item_id_ranges.active.max,
            payload.item_id_ranges.active.usable_item_id_min,
            payload.item_id_ranges.spare.min,
            payload.item_id_ranges.spare.max,
            payload.item_id_ranges.spare.usable_item_id_min,
        ]);
        self.phase = DbConnectionPhase::Booted;
        Ok(vec![DbClientEffect::BootAccepted {
            declared_length: payload.packet_size,
        }])
    }

    fn on_map_locations(&mut self, frame: &DbFrame) -> Result<Vec<DbClientEffect>, DbClientError> {
        if frame.handle != MAP_LOCATIONS_REPLY_HANDLE {
            self.disconnected();
            return Err(DbClientError::UnexpectedHandle {
                handle: frame.handle,
                header: protocol::db_map_locations::HEADER_DG_MAP_LOCATIONS,
            });
        }
        let reply = MapLocationsReply::decode(&frame.payload).map_err(|error| {
            self.disconnected();
            DbClientError::MapLocations(error)
        })?;
        self.locations = reply.records().to_vec();
        let records = self.locations.clone();
        Ok(vec![DbClientEffect::MapLocationsAccepted { records }])
    }
}

/// Client-acceptance gate driven by the DB boot phase.
///
/// # Why this gate exists
///
/// There is no readiness flag anywhere in the legacy tree. `main.cpp:671` binds
/// the client port, `main.cpp:697` only then creates the DB connector, and
/// `main.cpp:840-856` calls `TryConnect()` discarding its result before calling
/// `AcceptDesc()` unconditionally. A search of the whole server tree for
/// `g_bBoot`, `BootComplete`, `BootFinish`, `bBooted`, `IsBoot`, `boot_ok`, and
/// `g_boot` returns nothing, while the positive control `g_bNoMoreClient`
/// returns three hits. So legacy really does accept, handshake, and log in
/// players while the DB link is still retrying.
///
/// That is a known legacy defect, not a behavior to preserve. This gate is the
/// single intentional divergence on the M3 path, and it is isolated here so a
/// reviewer can find and evaluate it in one place.
///
/// It is a counting gate rather than a plain flag because a refused connection
/// should be closed immediately, while a client that arrives exactly at the
/// moment boot completes should be served rather than dropped.
///
/// The state is atomic so the accept loop and the DB link task can share one
/// `Arc<BootReadyGate>` without a lock on the hot path.
#[derive(Debug, Default)]
pub struct BootReadyGate {
    booted: AtomicBool,
    refused: AtomicU64,
    admitted: AtomicU64,
}

impl BootReadyGate {
    /// A closed gate, as the client listener is at startup.
    #[must_use]
    pub const fn closed() -> Self {
        Self {
            booted: AtomicBool::new(false),
            refused: AtomicU64::new(0),
            admitted: AtomicU64::new(0),
        }
    }

    /// Whether the DB boot has completed.
    #[must_use]
    pub fn is_boot_ready(&self) -> bool {
        self.booted.load(Ordering::SeqCst)
    }

    /// Total connections refused because boot had not completed.
    #[must_use]
    pub fn refused(&self) -> u64 {
        self.refused.load(Ordering::SeqCst)
    }

    /// Total connections admitted.
    #[must_use]
    pub fn admitted(&self) -> u64 {
        self.admitted.load(Ordering::SeqCst)
    }

    /// Open the gate. Called once a `DG_BOOT` reply has been accepted.
    pub fn open(&self) {
        self.booted.store(true, Ordering::SeqCst);
    }

    /// Close the gate again. Called when the DB link drops.
    ///
    /// Legacy does not re-close anything: a DB restart after a successful boot
    /// leaves the game fully operational on stale tables. Re-closing is the
    /// other half of the same divergence, and it is what makes the gate honest
    /// about the state the process is actually in.
    pub fn close(&self) {
        self.booted.store(false, Ordering::SeqCst);
    }

    /// Decide whether one inbound client connection may be served.
    ///
    /// A refused connection should be closed by the caller, not queued: the
    /// legacy client has no way to learn that the game core is not ready, and
    /// holding the socket open would only make the failure look like a hang.
    pub fn admit(&self) -> bool {
        if self.is_boot_ready() {
            self.admitted.fetch_add(1, Ordering::SeqCst);
            true
        } else {
            self.refused.fetch_add(1, Ordering::SeqCst);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_boot::{
        encode_active_db_boot_payload, BootAdminSection, BootGmHostSection, BootItemIdRange,
        BootItemIdRanges, BootMonarchCandidacySection, BootMonarchInfo, BootSection,
        BootSectionKind, DbBootPayload, ADMIN_INFO_WIRE_SIZE, GM_HOST_WIRE_SIZE,
        ITEM_ID_RANGE_WIRE_SIZE, MONARCH_CANDIDACY_WIRE_SIZE,
    };
    use protocol::db_map_locations::{HEADER_DG_MAP_LOCATIONS, HEADER_DG_P2P};
    use protocol::db_records::{
        EVENT_TABLE_WIRE_SIZE, ITEM_ATTR_RECORD_WIRE_SIZE, ITEM_TABLE_RECORD_WIRE_SIZE,
        LAND_RECORD_WIRE_SIZE, MARKET_ITEM_PRICE_WIRE_SIZE, MOB_TABLE_RECORD_WIRE_SIZE,
        OBJECT_PROTO_RECORD_WIRE_SIZE, OBJECT_RECORD_WIRE_SIZE, REFINE_TABLE_WIRE_SIZE,
        SHOP_TABLE_RECORD_WIRE_SIZE, SKILL_TABLE_RECORD_WIRE_SIZE,
    };
    use protocol::db_setup::{LOGIN_ON_SETUP_WIRE_SIZE, SETUP_BASE_WIRE_SIZE};
    use protocol::db_wire::DB_PEER_HEADER_SIZE;

    /// The configuration a one-channel, non-auth game core reports.
    fn config() -> DbClientConfig {
        DbClientConfig {
            listen_port: 50080,
            p2p_port: 51000,
            channel: 1,
            public_ip: public_ip_field("10.0.0.1").expect("a short IP fits"),
            map_allow: vec![1, 2, 3],
            auth_server: false,
            item_id_range: [0, 0],
        }
    }

    fn connected_client() -> GameDbClient {
        let mut client = GameDbClient::new(config());
        client
            .begin_connect(Duration::ZERO, &[])
            .expect("the first attempt is allowed");
        client.connected().expect("the socket opened");
        client
    }

    /// The verified packed width of one empty table record, by section kind.
    ///
    /// This table is test scaffolding, not protocol knowledge: the widths are
    /// the same public constants the production SQL adapters validate loaded
    /// rows against, so a value that drifts here makes the boot reply malformed
    /// and the client below refuses it. That failure is the signal, not a
    /// silent pass.
    fn empty_record_width(kind: BootSectionKind) -> usize {
        match kind {
            BootSectionKind::Mob => MOB_TABLE_RECORD_WIRE_SIZE,
            BootSectionKind::Item => ITEM_TABLE_RECORD_WIRE_SIZE,
            BootSectionKind::Shop | BootSectionKind::RenewalShop => SHOP_TABLE_RECORD_WIRE_SIZE,
            BootSectionKind::Skill => SKILL_TABLE_RECORD_WIRE_SIZE,
            BootSectionKind::Refine => REFINE_TABLE_WIRE_SIZE,
            BootSectionKind::ItemAttr | BootSectionKind::ItemRare => ITEM_ATTR_RECORD_WIRE_SIZE,
            BootSectionKind::Banword => protocol::db_boot::BANWORD_WIRE_SIZE,
            BootSectionKind::Land => LAND_RECORD_WIRE_SIZE,
            BootSectionKind::ObjectProto => OBJECT_PROTO_RECORD_WIRE_SIZE,
            BootSectionKind::Object => OBJECT_RECORD_WIRE_SIZE,
            BootSectionKind::Event => EVENT_TABLE_WIRE_SIZE,
            BootSectionKind::PremiumMarketPrice => MARKET_ITEM_PRICE_WIRE_SIZE,
        }
    }

    /// Number of table sections in the active version-6 boot payload.
    const EMPTY_BOOT_SECTION_COUNT: usize = 14;

    /// Exact payload width of a version-6 boot reply carrying no table rows.
    ///
    /// This is the verified 415-byte empty boot: the four-byte length prefix,
    /// the version byte, one four-byte header per table section, the four-byte
    /// global time, the item-ID range header plus its two twelve-byte records,
    /// the GM-host and admin headers, the monarch header plus its 304-byte
    /// record, the candidacy header, and the two-byte `0xffff` terminator.
    const EMPTY_BOOT_PAYLOAD_SIZE: usize =
        4 + 1 + EMPTY_BOOT_SECTION_COUNT * 4 + 4 + 4 + 2 * 12 + 4 + 4 + 4 + 304 + 4 + 2;

    /// A well-formed version-6 boot reply with no table rows.
    fn empty_boot_payload() -> Vec<u8> {
        let profile = BootFeatureProfile::active();
        let payload = DbBootPayload {
            // The encoder rejects a declared size that disagrees with the bytes
            // it produces, so this must be the real empty-payload width: a
            // four-byte length prefix, one version byte, fourteen four-byte
            // section headers, and the fixed 276-byte tail.
            packet_size: u32::try_from(EMPTY_BOOT_PAYLOAD_SIZE).expect("415 fits u32"),
            version: protocol::db_boot::DB_BOOT_VERSION,
            sections: profile
                .section_kinds()
                .iter()
                .map(|kind| BootSection {
                    kind: *kind,
                    record_size: u16::try_from(empty_record_width(*kind))
                        .expect("a record width fits u16"),
                    count: 0,
                    data: Vec::new(),
                })
                .collect(),
            global_time: 0,
            item_id_ranges: BootItemIdRanges {
                record_size: u16::try_from(ITEM_ID_RANGE_WIRE_SIZE)
                    .expect("the item-range width fits u16"),
                declared_count: 1,
                active: BootItemIdRange {
                    min: 0,
                    max: 0,
                    usable_item_id_min: 0,
                },
                spare: BootItemIdRange {
                    min: 0,
                    max: 0,
                    usable_item_id_min: 0,
                },
            },
            gm_hosts: BootGmHostSection {
                record_size: u16::try_from(GM_HOST_WIRE_SIZE).expect("GM host width fits u16"),
                count: 0,
                hosts: Vec::new(),
            },
            admins: BootAdminSection {
                record_size: u16::try_from(ADMIN_INFO_WIRE_SIZE).expect("admin width fits u16"),
                count: 0,
                admins: Vec::new(),
            },
            monarch: BootMonarchInfo {
                pid: [0; 4],
                money: [0; 4],
                name: [[0; 32]; 4],
                date: [[0; 32]; 4],
            },
            monarch_candidacy: BootMonarchCandidacySection {
                record_size: u16::try_from(MONARCH_CANDIDACY_WIRE_SIZE)
                    .expect("candidacy width fits u16"),
                count: 0,
                candidates: Vec::new(),
            },
            end_marker: protocol::db_boot::DB_BOOT_END_MARKER,
        };
        let bytes = encode_active_db_boot_payload(&payload).expect("an empty boot payload encodes");
        assert_eq!(
            bytes.len(),
            EMPTY_BOOT_PAYLOAD_SIZE,
            "the empty boot payload width is the verified 415 bytes"
        );
        bytes
    }

    /// The boot reply frame a real DB server sends for an empty snapshot.
    fn empty_boot_frame() -> DbFrame {
        DbFrame::new(HEADER_DG_BOOT, DB_BOOTSTRAP_HANDLE, empty_boot_payload())
    }

    fn map_locations_frame(host: &str, port: u16) -> DbFrame {
        let reply =
            MapLocationsReply::single(&[1, 2, 3], public_ip_field(host).expect("short host"), port)
                .expect("a one-record reply encodes");
        DbFrame::new(
            HEADER_DG_MAP_LOCATIONS,
            MAP_LOCATIONS_REPLY_HANDLE,
            reply.encode().expect("the reply encodes"),
        )
    }

    fn sent_frames(effects: &[DbClientEffect]) -> Vec<DbFrame> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                DbClientEffect::SendFrame(frame) => Some(frame.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_first_connection_sends_boot_then_setup_at_handle_zero() {
        let mut client = GameDbClient::new(config());
        let effects = client
            .begin_connect(Duration::ZERO, &[])
            .expect("the first attempt is allowed");
        let frames = sent_frames(&effects);
        assert_eq!(frames.len(), 2, "boot and setup are two frames");
        assert_eq!(frames[0].header, HEADER_GD_BOOT);
        assert_eq!(frames[1].header, HEADER_GD_SETUP);
        for frame in &frames {
            assert_eq!(
                frame.handle, 0,
                "desc_client.cpp:141 and 223 both send handle 0"
            );
        }
        // desc_client.cpp:141 then 223 append boot before setup in one phase.
        assert_eq!(frames[0].payload.len(), 24, "TPacketGDBoot is 24 bytes");
        assert_eq!(frames[1].payload.len(), SETUP_BASE_WIRE_SIZE);
    }

    #[test]
    fn a_real_boot_reply_opens_the_client_gate() {
        let mut client = connected_client();
        assert!(
            !client.accepts_clients(),
            "the gate is closed while the boot reply is outstanding"
        );
        let effects = client
            .on_frame(&empty_boot_frame())
            .expect("a valid boot reply is accepted");
        assert_eq!(
            effects,
            vec![DbClientEffect::BootAccepted {
                declared_length: u32::try_from(frames_len(&empty_boot_frame()))
                    .expect("a small payload fits u32"),
            }]
        );
        assert_eq!(client.phase(), DbConnectionPhase::Booted);
        assert!(client.accepts_clients());
        assert!(
            client.phase().is_connected(),
            "a booted link is still connected"
        );
    }

    fn frames_len(frame: &DbFrame) -> usize {
        frame.payload.len()
    }

    #[test]
    fn the_boot_reply_keeps_both_item_id_ranges() {
        // The item-ID range is why the rewrite resends BOOT on reconnect:
        // after a DB restart the game needs the current range, and legacy's
        // function-local bSentBoot means it never asks again.
        let mut client = connected_client();
        let frame = empty_boot_frame();
        client
            .on_frame(&frame)
            .expect("a valid boot reply is accepted");
        let ranges = client
            .boot_item_id_ranges()
            .expect("the boot reply carried both ranges");
        assert_eq!(ranges.len(), 6, "two ranges of three words each");
    }

    #[test]
    fn a_boot_reply_whose_length_word_disagrees_is_refused() {
        // Legacy reads this word and only logs it (input_db.cpp:463,468).
        let mut client = connected_client();
        let mut frame = empty_boot_frame();
        frame.payload[0] = frame.payload[0].wrapping_add(1);
        let error = client
            .on_frame(&frame)
            .expect_err("a mismatched length word is refused");
        assert!(
            matches!(error, DbClientError::InvalidBootReply { .. }),
            "got {error:?}"
        );
        assert_eq!(
            client.phase(),
            DbConnectionPhase::Failed,
            "a refused reply closes the link"
        );
        assert!(!client.accepts_clients());
    }

    #[test]
    fn a_boot_reply_at_the_wrong_version_is_refused_and_the_link_closes() {
        // Legacy shuts down and then keeps parsing as version 6 because the
        // branch has no `return` (input_db.cpp:470-474).
        let mut client = connected_client();
        let mut frame = empty_boot_frame();
        frame.payload[4] = 5;
        let error = client
            .on_frame(&frame)
            .expect_err("a wrong version is refused");
        assert!(matches!(error, DbClientError::InvalidBootReply { .. }));
        assert_eq!(client.phase(), DbConnectionPhase::Failed);
    }

    #[test]
    fn a_truncated_boot_reply_is_refused() {
        let mut client = connected_client();
        let mut frame = empty_boot_frame();
        frame.payload.truncate(3);
        assert!(client.on_frame(&frame).is_err());
        assert_eq!(client.phase(), DbConnectionPhase::Failed);
    }

    #[test]
    fn a_boot_reply_at_a_non_zero_handle_is_refused() {
        // The bootstrap path has no per-request correlation. ClientManager.cpp
        // writes handle 0 for every bootstrap reply.
        let mut client = connected_client();
        let mut frame = empty_boot_frame();
        frame.handle = 7;
        assert!(client.on_frame(&frame).is_err());
        assert_eq!(client.phase(), DbConnectionPhase::Failed);
    }

    #[test]
    fn the_setup_reply_records_map_locations() {
        let mut client = connected_client();
        let frame = map_locations_frame("10.0.0.7", 51000);
        let effects = client
            .on_frame(&frame)
            .expect("a valid setup reply is accepted");
        match &effects[..] {
            [DbClientEffect::MapLocationsAccepted { records }] => {
                assert_eq!(records.len(), 1);
                assert_eq!(
                    records[0].port, 51000,
                    "the reply port is preserved verbatim"
                );
            }
            other => panic!("unexpected effects: {other:?}"),
        }
        assert_eq!(client.locations().len(), 1);
    }

    #[test]
    fn the_setup_reply_at_a_non_zero_handle_is_refused() {
        let mut client = connected_client();
        let mut frame = map_locations_frame("10.0.0.7", 51000);
        frame.handle = 3;
        let error = client.on_frame(&frame).expect_err("the handle is wrong");
        assert!(matches!(
            error,
            DbClientError::UnexpectedHandle { handle: 3, .. }
        ));
        assert_eq!(client.phase(), DbConnectionPhase::Failed);
    }

    #[test]
    fn a_p2p_announcement_is_reported_rather_than_discarded() {
        let mut client = connected_client();
        let frame = DbFrame::new(HEADER_DG_P2P, 0, vec![1, 2, 3]);
        let effects = client.on_frame(&frame).expect("P2P is a known header");
        assert_eq!(
            effects,
            vec![DbClientEffect::P2PAnnounced {
                payload: vec![1, 2, 3]
            }]
        );
        assert_eq!(
            client.phase(),
            DbConnectionPhase::AwaitingBoot,
            "an announcement does not boot the link"
        );
    }

    #[test]
    fn an_unknown_header_closes_the_link_instead_of_being_skipped() {
        // Legacy logs and consumes the frame (input_db.cpp:2273-2280) and
        // keeps parsing. That is divergence 3.
        let mut client = connected_client();
        let frame = DbFrame::new(0x7e, 0, vec![]);
        let error = client
            .on_frame(&frame)
            .expect_err("an unknown header is an error");
        assert_eq!(error, DbClientError::UnsupportedHeader { header: 0x7e });
        assert_eq!(client.phase(), DbConnectionPhase::Failed);
    }

    #[test]
    fn a_reconnect_resends_the_boot_request_which_legacy_never_does() {
        // desc_client.cpp:132 guards the boot send with a function-local
        // `static bool bSentBoot` that is never reset anywhere in the tree, so
        // after a DB restart legacy keeps serving the original tables. The
        // rewrite resends on every connection.
        let mut client = connected_client();
        client
            .on_frame(&empty_boot_frame())
            .expect("boot reply accepted");
        client.disconnected();
        let effects = client
            .begin_connect(Duration::from_secs(60), &[])
            .expect("the window has elapsed");
        let headers: Vec<u8> = sent_frames(&effects)
            .iter()
            .map(|frame| frame.header)
            .collect();
        assert_eq!(
            headers,
            vec![HEADER_GD_BOOT, HEADER_GD_SETUP],
            "both bootstrap frames are resent on a new connection"
        );
        assert_eq!(client.phase(), DbConnectionPhase::Connecting);
    }

    #[test]
    fn the_reconnect_window_is_three_seconds_and_a_failed_attempt_consumes_it() {
        // desc_client.cpp:73-103 stamps the attempt time before connecting, so
        // a refused connection still consumes the window.
        let mut client = GameDbClient::new(config());
        client
            .begin_connect(Duration::from_secs(10), &[])
            .expect("the first attempt is allowed");
        client.disconnected();
        let error = client
            .connect_allowed(Duration::from_secs(12))
            .expect_err("two seconds is inside the window");
        assert_eq!(
            error,
            DbClientError::RetryWindowOpen {
                remaining: Duration::from_secs(1)
            }
        );
        client
            .connect_allowed(Duration::from_secs(13))
            .expect("three seconds has elapsed");
    }

    #[test]
    fn a_connect_during_a_live_link_is_refused() {
        let client = connected_client();
        assert!(
            client.connect_allowed(Duration::from_secs(600)).is_err(),
            "a booted link already has its socket"
        );
    }

    #[test]
    fn an_auth_client_sends_a_base_only_setup_and_no_boot_request() {
        // desc_client.cpp:130 skips the boot block for an auth server and
        // desc_client.cpp:219-220 writes a bare setup with no login tail.
        let mut auth_config = config();
        auth_config.auth_server = true;
        let mut client = GameDbClient::new(auth_config);
        let effects = client
            .begin_connect(Duration::ZERO, &[])
            .expect("the first attempt is allowed");
        let frames = sent_frames(&effects);
        assert_eq!(frames.len(), 1, "an auth client sends no boot request");
        assert_eq!(frames[0].header, HEADER_GD_SETUP);
        assert_eq!(frames[0].payload.len(), SETUP_BASE_WIRE_SIZE);
        assert_eq!(
            frames[0].payload[SETUP_BASE_WIRE_SIZE - 1],
            1,
            "bAuthServer is 1 on the wire"
        );
    }

    #[test]
    fn an_auth_client_cannot_reports_login_records() {
        let mut auth_config = config();
        auth_config.auth_server = true;
        let mut client = GameDbClient::new(auth_config);
        let record = LoginOnSetup {
            id: 1,
            login: [b'a'; 31],
            social_id: [b'1'; 19],
            host: [b'1'; 16],
            login_key: 0,
            client_keys: [0; 4],
            language: 1,
            player_id: 0,
            player_handle: 0,
            has_private_shop_raw: 0,
        };
        let error = client
            .begin_connect(Duration::ZERO, &[record])
            .expect_err("auth mode must be base only");
        assert!(matches!(error, DbClientError::SetupEncode(_)));
    }

    #[test]
    fn reported_login_records_drive_the_wire_count() {
        // desc_client.cpp:167-215 counts connected descriptors with a nonzero
        // account id, writes that count, then appends exactly that many
        // records. The count is derived from the list here, so the two cannot
        // disagree.
        let mut client = GameDbClient::new(config());
        let record = LoginOnSetup {
            id: 7,
            login: [b'u'; 31],
            social_id: [b'2'; 19],
            host: [b'1'; 16],
            login_key: 42,
            client_keys: [1, 2, 3, 4],
            language: 1,
            player_id: 99,
            player_handle: 5,
            has_private_shop_raw: 1,
        };
        let effects = client
            .begin_connect(Duration::ZERO, &[record, record])
            .expect("a two-record setup encodes");
        let frames = sent_frames(&effects);
        let payload = &frames[1].payload;
        assert_eq!(
            payload.len(),
            SETUP_BASE_WIRE_SIZE + 2 * LOGIN_ON_SETUP_WIRE_SIZE
        );
        assert_eq!(
            u32::from_le_bytes(
                payload[SETUP_BASE_WIRE_SIZE - 5..SETUP_BASE_WIRE_SIZE - 1]
                    .try_into()
                    .expect("4 bytes")
            ),
            2,
            "dwLoginCount is the number of records actually sent"
        );
        assert!(record.has_private_shop(), "the fixture sets the flag");
    }

    #[test]
    fn the_map_allow_list_cannot_overflow_into_the_login_count() {
        // Legacy writes size+1 map values because the store precedes the bound
        // test (config.cpp:274-287, called from desc_client.cpp:158). A 33rd
        // value lands on dwLoginCount at offset 149 of the packed record.
        let mut wide = config();
        wide.map_allow = (0..40).collect();
        let base = wide.base();
        assert_eq!(base.maps[31], 31, "the 32nd configured index is kept");
        assert_eq!(base.login_count, 0, "the count is not aliased by a map");
    }

    #[test]
    fn a_long_public_ip_is_refused_rather_than_truncated() {
        assert_eq!(
            public_ip_field("123.123.123.1234567"),
            Err(DbClientError::PublicIpTooLong { length: 19 })
        );
        // A fifteen-character string plus the NUL is the longest that fits.
        assert!(public_ip_field("123456789012345").is_ok());
    }

    #[test]
    fn every_bootstrap_frame_survives_the_real_peer_framing() {
        let mut client = GameDbClient::new(config());
        let effects = client
            .begin_connect(Duration::ZERO, &[])
            .expect("composition succeeds");
        for effect in &effects {
            let DbClientEffect::SendFrame(frame) = effect else {
                panic!("expected a send effect, got {effect:?}");
            };
            let bytes = frame.encode().expect("the frame encodes");
            assert_eq!(bytes[0], frame.header);
            assert_eq!(bytes.len(), DB_PEER_HEADER_SIZE + frame.payload.len());
            assert_eq!(
                u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) as usize,
                frame.payload.len(),
                "the length field counts payload bytes only"
            );
        }
    }

    #[test]
    fn the_gate_is_closed_at_startup_and_opens_only_on_a_boot_reply() {
        let gate = BootReadyGate::closed();
        assert!(!gate.is_boot_ready());
        assert!(!gate.admit(), "clients are refused before boot");
        assert_eq!(gate.refused(), 1);
        assert_eq!(gate.admitted(), 0);
        gate.open();
        assert!(gate.is_boot_ready());
        assert!(gate.admit());
        assert_eq!(gate.admitted(), 1);
        assert_eq!(gate.refused(), 1);
    }

    #[test]
    fn the_gate_recloses_when_the_db_link_drops() {
        let gate = BootReadyGate::closed();
        gate.open();
        assert!(gate.admit());
        gate.close();
        assert!(
            !gate.admit(),
            "a dropped DB link stops accepting clients again"
        );
        assert_eq!(gate.refused(), 1);
        assert_eq!(gate.admitted(), 1);
    }

    #[test]
    fn the_client_and_the_gate_agree_across_a_full_connect_boot_drop_cycle() {
        let mut client = connected_client();
        let gate = BootReadyGate::closed();
        assert!(!client.accepts_clients());
        assert!(!gate.admit());
        client
            .on_frame(&empty_boot_frame())
            .expect("boot reply accepted");
        if client.accepts_clients() {
            gate.open();
        }
        assert!(gate.admit(), "the gate opens on the real boot reply");
        client.disconnected();
        gate.close();
        assert!(!client.accepts_clients());
        assert!(!gate.admit());
    }
}
