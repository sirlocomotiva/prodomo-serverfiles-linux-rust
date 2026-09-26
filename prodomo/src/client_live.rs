//! A transport-owning client descriptor that advances one real client.
//!
//! Everything the legacy descriptor does to a client socket lives here: stream
//! buffering, the plaintext/TEA input boundary, per-phase analyzers, key
//! installation, and teardown. The decisions themselves are **not** here. The
//! reducers already in this crate own them, and this module only supplies the
//! I/O and applies their results in source order:
//!
//! - [`ClientLifecycle`] owns the phase, handshake, and heartbeat state and
//!   returns the effects in the exact order `DESC::SetPhase` uses.
//! - [`crate::handshake_dispatch`] decodes the packed 13-byte handshake record
//!   and produces a candidate reduction.
//! - [`DescriptorCrypto`] owns the TEA boundary, the aligned-input tail, and
//!   the zero-padded output units.
//! - [`ClientFrameDecoder`] resolves the fixed client frame size.
//!
//! # The one ordering rule this module exists to enforce
//!
//! `DESC::SetPhase` writes the `GC_PHASE` record **before** it enables TEA
//! (`desc.cpp`). For the handshake-to-`Login`/`Auth` transition that means the
//! phase record goes out as **plaintext** even though every later record on the
//! same descriptor is encrypted. Getting this backwards produces a client that
//! desynchronizes on the very first phase switch and is very hard to diagnose,
//! so the ordering is explicit in `LiveClientSession::apply` and pinned by the
//! test `the_phase_record_is_written_before_tea_is_enabled`.
//!
//! # What this module still does not own
//!
//! This advances a client as far as the descriptor **phase** and the handshake
//! reducer. It performs no authentication, no SQL, no account lookup, no world
//! construction, and no gameplay. Reaching the `Game` phase needs the
//! character-load path, which is a separate boundary.

use std::collections::hash_map::RandomState;
use std::error::Error;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use protocol::cg_handshake::CgHandshakeHeader;
use protocol::cg_inventory::{CG_KEEP_ALIVE, HEADER_CG_PONG};
use protocol::cg_wire::{
    resolve_client_frame_size, ClientFrame, ClientFrameDecoder, ClientFrameError, ClientFrameSize,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::client_session::ClientPhase;
use crate::descriptor_crypto::{DescriptorCrypto, DescriptorCryptoError, DescriptorCryptoMode};
use crate::handshake::HandshakeServerKind;
use crate::handshake_dispatch::{
    dispatch_handshake_frame_at_boundary, HandshakeDispatchError, HandshakeInputBoundary,
};
use crate::lifecycle::{
    encode_lifecycle_effect, ClientLifecycle, LifecycleEffect, LifecycleInputBoundary,
    LifecycleReduction,
};

/// Socket read size for one descriptor input step.
///
/// The legacy descriptor reads into a fixed buffer. The value only affects how
/// many frames one syscall can observe, never how a frame is framed, because
/// every input path is length-agnostic.
const READ_CHUNK_SIZE: usize = 4096;

/// A 32-bit wall clock in the shape the legacy reducers expect.
pub trait LiveClock {
    /// Return the current value used for handshake and heartbeat timing.
    ///
    /// The legacy heartbeat compares signed 32-bit differences, so this is a
    /// `u32` that wraps, not a `u64` that does not.
    fn now(&self) -> u32;
}

/// The legacy descriptor clock: milliseconds since the server started, as a wrapping `u32`.
///
/// Legacy `get_dword_time` (`libthecore/utils.cpp:467`) returns
/// `(tv_sec - boot_sec) * 1000 + tv_usec / 1000`, so the handshake time and the 50 ms bias
/// window in `DESC::HandshakeProcess` are in milliseconds. This clock counts from a monotonic
/// start instead of the wall clock, so a clock step cannot fail a handshake.
#[derive(Debug, Clone, Copy)]
pub struct BootLiveClock {
    boot: Instant,
}

impl BootLiveClock {
    /// Start the clock now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            boot: Instant::now(),
        }
    }
}

impl Default for BootLiveClock {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveClock for BootLiveClock {
    fn now(&self) -> u32 {
        // Keep the low 32 bits: the legacy clock is a wrapping `u32`.
        let [a, b, c, d, ..] = self.boot.elapsed().as_millis().to_le_bytes();
        u32::from_le_bytes([a, b, c, d])
    }
}

/// A fresh handshake token: an opaque, nonzero 32-bit value the client echoes.
///
/// Legacy `DESC_MANAGER::CreateHandshake` takes a CRC32 of a random number and the time and
/// retries on zero. The client treats the value as opaque, so any nonzero value that an
/// observer cannot predict is equivalent. The standard library's randomly keyed hasher supplies
/// it without a new dependency.
#[must_use]
pub fn handshake_token() -> u32 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    loop {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(NEXT.fetch_add(1, Ordering::Relaxed));
        let [a, b, c, d, ..] = hasher.finish().to_le_bytes();
        let token = u32::from_le_bytes([a, b, c, d]);
        if token != 0 {
            return token;
        }
    }
}

/// A caller-supplied descriptor clock, used by tests and by the boot gate.
///
/// Injecting the clock keeps the heartbeat arithmetic testable: the legacy
/// reducer computes signed 32-bit deltas, which only misbehave around a
/// deliberate wrap.
#[derive(Debug, Clone, Copy)]
pub struct ManualLiveClock(u32);

impl ManualLiveClock {
    /// Create a clock fixed at `now`.
    #[must_use]
    pub const fn new(now: u32) -> Self {
        Self(now)
    }

    /// Move the clock to an absolute value.
    pub const fn set(&mut self, now: u32) {
        self.0 = now;
    }

    /// Advance the clock by a signed delta, reproducing the legacy wrap.
    pub const fn advance(&mut self, delta: i32) {
        self.0 = self.0.wrapping_add(u32::from_ne_bytes(delta.to_ne_bytes()));
    }
}

impl LiveClock for ManualLiveClock {
    fn now(&self) -> u32 {
        self.0
    }
}

/// A retained client header this descriptor boundary deliberately does not own.
///
/// The legacy server keeps a `PONG` case in the handshake, login, main/dead,
/// and auth analyzers, and a variable `Analyze` table in the main analyzer.
/// Everything else in those analyzers is still unimplemented here, so this
/// boundary names the gap instead of guessing at a record layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveUnsupported {
    /// A variable `CInputMain::Analyze` header.
    VariableAnalyze {
        /// The one-byte client header.
        header: u8,
    },
    /// A header this boundary retains but whose record it does not decode.
    RetainedHeader {
        /// Descriptor phase the header reached.
        phase: ClientPhase,
        /// The one-byte client header.
        header: u8,
    },
}

impl fmt::Display for LiveUnsupported {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VariableAnalyze { header } => {
                write!(
                    formatter,
                    "variable client header {header:#04x} is not modelled"
                )
            }
            Self::RetainedHeader { phase, header } => write!(
                formatter,
                "client header {header:#04x} is retained but not decoded in {phase:?}"
            ),
        }
    }
}

impl Error for LiveUnsupported {}

/// A terminal failure of the live descriptor.
#[derive(Debug)]
pub enum LiveError {
    /// The descriptor phase and the lifecycle candidate disagree.
    PhaseClosed,
    /// A handshake record was rejected before the reducer ran.
    Handshake(HandshakeDispatchError),
    /// The TEA or plaintext boundary rejected an operation.
    Crypto(DescriptorCryptoError),
    /// The client frame decoder rejected a header, size, or truncated frame.
    Frame(ClientFrameError),
    /// The socket failed.
    Io(io::Error),
    /// A phase transition would have reinterpreted buffered input.
    BoundaryNotInstallable {
        /// The boundary the reduction selected.
        requested: LifecycleInputBoundary,
        /// The concrete cipher failure that refused it.
        source: DescriptorCryptoError,
    },
}

impl fmt::Display for LiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PhaseClosed => {
                formatter.write_str("the descriptor is closed and consumes no client input")
            }
            Self::Handshake(error) => write!(formatter, "handshake dispatch failed: {error}"),
            Self::Crypto(error) => write!(formatter, "descriptor cipher failed: {error}"),
            Self::Frame(error) => write!(formatter, "client frame decode failed: {error}"),
            Self::Io(error) => write!(formatter, "client socket failed: {error}"),
            Self::BoundaryNotInstallable { requested, source } => write!(
                formatter,
                "the {requested:?} input boundary was refused: {source}"
            ),
        }
    }
}

impl Error for LiveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Handshake(error) => Some(error),
            Self::Crypto(error) | Self::BoundaryNotInstallable { source: error, .. } => Some(error),
            Self::Frame(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::PhaseClosed => None,
        }
    }
}

impl From<DescriptorCryptoError> for LiveError {
    fn from(error: DescriptorCryptoError) -> Self {
        Self::Crypto(error)
    }
}

impl From<ClientFrameError> for LiveError {
    fn from(error: ClientFrameError) -> Self {
        Self::Frame(error)
    }
}

impl From<io::Error> for LiveError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// What one handled client frame produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveOutcome {
    /// The one-byte zero keepalive, which produces no output.
    KeepAlive {
        /// The complete frame, including its one-byte header.
        frame: ClientFrame,
    },
    /// A `HEADER_CG_PONG` acknowledgement, which may emit a ping.
    Pong {
        /// The complete frame, including its one-byte header.
        frame: ClientFrame,
        /// Encrypted records written back to the client, in source order.
        written: Vec<Vec<u8>>,
    },
    /// A packed handshake record, which may move the descriptor phase.
    Handshake {
        /// The complete frame, including its one-byte header.
        frame: ClientFrame,
        /// Encrypted records written back to the client, in source order.
        written: Vec<Vec<u8>>,
        /// Descriptor phase after the reduction was applied.
        phase: ClientPhase,
    },
}

/// The result of one descriptor step.
#[derive(Debug)]
pub enum LiveStep {
    /// Bytes were buffered; no complete frame is available yet.
    Pending {
        /// Plaintext bytes still buffered by the frame decoder.
        buffered: usize,
    },
    /// A complete frame was consumed and handled.
    Handled(LiveOutcome),
    /// A complete fixed record for a phase analyzer above this boundary.
    ///
    /// The descriptor handles only the control records (keepalive, pong, handshake, and time
    /// sync). Every other record is handed to the caller, which owns the world and the store,
    /// answers through [`LiveClientSession::send`], and closes on a header its analyzer does not
    /// handle.
    Record {
        /// Descriptor phase the record arrived in.
        phase: ClientPhase,
        /// The complete frame, including its one-byte header.
        frame: ClientFrame,
    },
    /// The peer closed the stream.
    ///
    /// `leftover` is the number of plaintext bytes that were still buffered
    /// when the end of stream arrived. A nonzero value means the peer
    /// disconnected mid-record; the legacy descriptor has no notion of a
    /// partial record, so this boundary reports the count instead of
    /// inventing a recovery the source does not have.
    PeerClosed {
        /// Plaintext bytes that never formed a complete frame.
        leftover: usize,
    },
    /// A retained client header reached a boundary this module does not own.
    ///
    /// Legacy closes here rather than retrying: an analyzer that cannot decode
    /// a record must not leave the bytes in the input buffer to be retried
    /// forever. The caller should drop the session.
    Unsupported(LiveUnsupported),
    /// A terminal failure. The session is no longer usable.
    Failed(LiveError),
}

/// A live client descriptor that owns one client stream.
pub struct LiveClientSession<S> {
    stream: S,
    crypto: DescriptorCrypto,
    decoder: ClientFrameDecoder,
    lifecycle: ClientLifecycle,
}

impl<S> LiveClientSession<S> {
    /// Return the current input cipher mode.
    #[must_use]
    pub const fn cipher_mode(&self) -> DescriptorCryptoMode {
        self.crypto.mode()
    }
}

impl<S> LiveClientSession<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Create a live descriptor around an already-framed plaintext boundary.
    ///
    /// The lifecycle must agree with the cipher: a `LifecycleInputBoundary` of
    /// `LegacyTea` needs [`DescriptorCrypto::enable_default_legacy_tea`] to
    /// have run first, and this constructor refuses the combination rather
    /// than leaving the descriptor in a state where the reducer believes TEA
    /// is active and the socket disagrees.
    ///
    /// # Errors
    ///
    /// Returns [`LiveError::BoundaryNotInstallable`] when the lifecycle selects
    /// a TEA input boundary that the plaintext cipher cannot honour.
    pub fn new(stream: S, lifecycle: ClientLifecycle) -> Result<Self, LiveError> {
        let crypto = DescriptorCrypto::with_default_limit()?;
        Self::with_cipher(stream, lifecycle, crypto)
    }

    /// Create a live descriptor with an explicit cipher.
    ///
    /// # Errors
    ///
    /// Returns [`LiveError::BoundaryNotInstallable`] when the lifecycle selects
    /// a TEA input boundary that the supplied cipher cannot honour.
    pub fn with_cipher(
        stream: S,
        lifecycle: ClientLifecycle,
        crypto: DescriptorCrypto,
    ) -> Result<Self, LiveError> {
        if lifecycle.input_boundary() == LifecycleInputBoundary::LegacyTea
            && crypto.mode() == DescriptorCryptoMode::Plaintext
        {
            return Err(LiveError::BoundaryNotInstallable {
                requested: LifecycleInputBoundary::LegacyTea,
                source: DescriptorCryptoError::KeyInstallBeforeTea,
            });
        }
        if lifecycle.input_boundary() == LifecycleInputBoundary::Plaintext
            && crypto.mode() != DescriptorCryptoMode::Plaintext
        {
            return Err(LiveError::BoundaryNotInstallable {
                requested: LifecycleInputBoundary::Plaintext,
                source: DescriptorCryptoError::KeyAlreadyInstalled,
            });
        }
        Ok(Self {
            stream,
            crypto,
            decoder: ClientFrameDecoder::new(),
            lifecycle,
        })
    }

    /// Borrow the transport-free lifecycle state.
    #[must_use]
    pub const fn lifecycle(&self) -> &ClientLifecycle {
        &self.lifecycle
    }

    /// Return the descriptor phase.
    #[must_use]
    pub const fn phase(&self) -> ClientPhase {
        self.lifecycle.phase()
    }

    /// Return the plaintext bytes still buffered by the frame decoder.
    #[must_use]
    pub fn buffered(&self) -> usize {
        self.decoder.buffered_len()
    }

    /// Recover the owned stream.
    #[must_use]
    pub fn into_inner(self) -> S {
        self.stream
    }

    /// Accept a client: run `DESC::Setup` and write its records.
    ///
    /// Legacy sets the handshake phase and sends the first handshake as soon as the socket is
    /// accepted (`desc.cpp:242-243`), so the client hears `GC_PHASE(PHASE_HANDSHAKE)` and then
    /// `GC_HANDSHAKE(token, now, 0)`, both plaintext, before it sends anything.
    ///
    /// # Errors
    ///
    /// Returns [`LiveError`] when the records cannot be written.
    pub async fn start(
        stream: S,
        token: u32,
        server_kind: HandshakeServerKind,
        now: u32,
    ) -> Result<(Self, Vec<Vec<u8>>), LiveError> {
        let reduction = ClientLifecycle::start(token, server_kind, now);
        let mut session = Self::new(stream, reduction.state)?;
        let written = session.apply(reduction).await?;
        Ok((session, written))
    }

    /// Read one client frame and apply the matching source analyzer.
    ///
    /// The method accepts arbitrary fragmentation and coalesced frames. It
    /// performs at most one `read` on the stream, decodes the first complete
    /// frame from the plaintext buffer, and returns `Pending` when more bytes
    /// are needed.
    ///
    /// # Errors
    ///
    /// Returns [`LiveError`] for a socket failure, a cipher failure, a frame
    /// decoder rejection, or a refused phase boundary. Every one of these ends
    /// the descriptor, exactly as the legacy close would.
    pub async fn step(&mut self, now: u32) -> Result<LiveStep, LiveError> {
        // Decode what is already buffered, so coalesced frames never wait on
        // the socket.
        if let Some(step) = self.next_buffered(now).await? {
            return Ok(step);
        }
        // Then perform exactly one read and decode again.
        if self.read_input().await? == 0 {
            return Ok(LiveStep::PeerClosed {
                leftover: self.decoder.buffered_len(),
            });
        }
        Ok(self.next_buffered(now).await?.unwrap_or(LiveStep::Pending {
            buffered: self.decoder.buffered_len(),
        }))
    }

    /// Handle the first complete frame already buffered, without reading the socket.
    ///
    /// Returns `None` when no complete frame is buffered.
    ///
    /// # Errors
    ///
    /// As [`LiveClientSession::step`].
    pub async fn next_buffered(&mut self, now: u32) -> Result<Option<LiveStep>, LiveError> {
        if self.lifecycle.phase() == ClientPhase::Close {
            return Err(LiveError::PhaseClosed);
        }
        self.drain_cipher_into_decoder()?;
        match self.decoder.try_decode()? {
            Some(frame) => Ok(Some(self.handle_frame(frame, now).await)),
            None => Ok(None),
        }
    }

    /// Perform one socket read into the input buffer and return the byte count; zero is the
    /// end of stream.
    ///
    /// This is cancel-safe: dropping the future before it completes loses no input, so a
    /// caller may race it against a timer. [`LiveClientSession::next_buffered`] then handles
    /// what arrived.
    ///
    /// # Errors
    ///
    /// Returns [`LiveError`] for a socket or cipher failure.
    pub async fn read_input(&mut self) -> Result<usize, LiveError> {
        let mut chunk = [0_u8; READ_CHUNK_SIZE];
        let bytes_read = self.stream.read(&mut chunk).await?;
        if bytes_read > 0 {
            self.crypto.feed_input(&chunk[..bytes_read])?;
        }
        Ok(bytes_read)
    }

    /// Run one `ping_event` and write its records.
    ///
    /// When the client never answered the previous ping, the descriptor closes and nothing is
    /// written; the caller then drops the connection.
    ///
    /// # Errors
    ///
    /// Returns [`LiveError`] when the records cannot be written.
    pub async fn tick(&mut self, now: u32) -> Result<Vec<Vec<u8>>, LiveError> {
        if self.lifecycle.phase() == ClientPhase::Close {
            return Err(LiveError::PhaseClosed);
        }
        let reduction = self.lifecycle.on_tick(now);
        self.apply(reduction).await
    }

    /// Write one record through the current output boundary, as legacy `DESC::Packet` does:
    /// each record is its own TEA unit, zero-padded to 8 bytes once TEA is on.
    ///
    /// # Errors
    ///
    /// Returns [`LiveError::PhaseClosed`] once the descriptor is closed, because legacy drops
    /// output in `PHASE_CLOSE`, and [`LiveError`] when the record cannot be written.
    pub async fn send(&mut self, record: &[u8]) -> Result<Vec<u8>, LiveError> {
        if self.lifecycle.phase() == ClientPhase::Close {
            return Err(LiveError::PhaseClosed);
        }
        let wire = self.crypto.encrypt_output(record)?;
        self.stream.write_all(&wire).await?;
        Ok(wire)
    }

    /// Move decrypted plaintext from the cipher into the frame decoder.
    ///
    /// The cipher owns the input buffer because it is what enforces the TEA
    /// block alignment and the 1..7-byte ciphertext tail. The frame decoder
    /// owns the plaintext framing. Draining one into the other here keeps a
    /// single accounting point: once bytes reach the frame decoder they are
    /// plaintext, and the cipher buffer is empty again so a later key change
    /// is not refused by `InputNotEmpty`.
    fn drain_cipher_into_decoder(&mut self) -> Result<(), LiveError> {
        if self.crypto.input_len() == 0 {
            return Ok(());
        }
        let plaintext = self.crypto.input().to_vec();
        self.decoder.feed(&plaintext)?;
        self.crypto.consume_input(plaintext.len())?;
        Ok(())
    }

    /// Route one complete frame to the analyzer its descriptor phase selects.
    async fn handle_frame(&mut self, frame: ClientFrame, now: u32) -> LiveStep {
        let phase = self.lifecycle.phase();

        // `CInputProcessor::Process` handles the zero keepalive before it
        // consults the phase analyzer, so this order is not an optimization.
        if frame.header == CG_KEEP_ALIVE.value() {
            return LiveStep::Handled(LiveOutcome::KeepAlive { frame });
        }

        // The base processor does not handle PONG. The handshake, login,
        // main/dead, and auth analyzers each carry their own PONG case, so a
        // pong in any other phase is a retained header, not a no-op.
        if frame.header == HEADER_CG_PONG.value() {
            if !phase.accepts_pong() {
                return LiveStep::Unsupported(LiveUnsupported::RetainedHeader {
                    phase,
                    header: frame.header,
                });
            }
            return match self.lifecycle.on_pong() {
                Ok(reduction) => match self.apply(reduction).await {
                    Ok(written) => LiveStep::Handled(LiveOutcome::Pong { frame, written }),
                    Err(error) => LiveStep::Failed(error),
                },
                Err(error) => LiveStep::Failed(LiveError::Handshake(
                    HandshakeDispatchError::Lifecycle(error),
                )),
            };
        }

        let handshake_record = frame.header == CgHandshakeHeader::Handshake.value()
            || frame.header == CgHandshakeHeader::TimeSync.value();
        if !phase.accepts_handshake_record() || !handshake_record {
            return LiveStep::Record { phase, frame };
        }

        let boundary = if self.lifecycle.input_boundary() == LifecycleInputBoundary::LegacyTea {
            HandshakeInputBoundary::DecryptedLegacyTea
        } else {
            HandshakeInputBoundary::Plaintext
        };

        match dispatch_handshake_frame_at_boundary(frame, phase, boundary, self.lifecycle, now) {
            Ok(dispatch) => {
                let reduction = dispatch.reduction;
                match self.apply(reduction).await {
                    Ok(written) => {
                        let phase = self.lifecycle.phase();
                        LiveStep::Handled(LiveOutcome::Handshake {
                            frame: dispatch.frame,
                            written,
                            phase,
                        })
                    }
                    Err(error) => LiveStep::Failed(error),
                }
            }
            Err(error) => LiveStep::Failed(LiveError::Handshake(error)),
        }
    }

    /// Apply a reduction: write every record in order, then commit the state.
    ///
    /// A `SetPhase` effect is written through the **current** output boundary
    /// and only then installs the new input boundary. That is the `DESC::SetPhase`
    /// order, and it is the reason the handshake-to-`Login` phase record is
    /// plaintext.
    async fn apply(&mut self, reduction: LifecycleReduction) -> Result<Vec<Vec<u8>>, LiveError> {
        let mut written = Vec::new();

        for effect in &reduction.effects {
            let bytes = encode_lifecycle_effect(effect);
            if !bytes.is_empty() {
                let wire = self.crypto.encrypt_output(&bytes)?;
                self.stream.write_all(&wire).await?;
                written.push(wire);
            }
            if let LifecycleEffect::SetPhase { input_boundary, .. } = effect {
                self.install_boundary(*input_boundary)?;
            }
        }

        // Commit only after every record is on the wire, so a failed write
        // cannot leave the reducer ahead of the socket.
        self.lifecycle = reduction.state;
        Ok(written)
    }

    /// Apply the input boundary a `SetPhase` effect selected.
    fn install_boundary(&mut self, boundary: LifecycleInputBoundary) -> Result<(), LiveError> {
        // `SetPhase` enables TEA with the default pair; `SetSecurityKey` later
        // replaces it through `install_client_keys`. A TEA boundary that is
        // already active, with either pair, is left as it is.
        let desired = match (boundary, self.crypto.mode()) {
            (LifecycleInputBoundary::Plaintext, _) => DescriptorCryptoMode::Plaintext,
            (LifecycleInputBoundary::LegacyTea, DescriptorCryptoMode::Plaintext) => {
                DescriptorCryptoMode::LegacyTeaDefault
            }
            (LifecycleInputBoundary::LegacyTea, active) => active,
        };
        if desired == self.crypto.mode() {
            return Ok(());
        }
        if desired == DescriptorCryptoMode::Plaintext {
            // Nothing in the legacy descriptor turns TEA back off. Reaching
            // this arm means the reducer asked for something the descriptor
            // cannot do, so refuse it rather than silently dropping the key.
            return Err(LiveError::BoundaryNotInstallable {
                requested: boundary,
                source: DescriptorCryptoError::KeyAlreadyInstalled,
            });
        }
        self.crypto.enable_default_legacy_tea().map_err(|source| {
            LiveError::BoundaryNotInstallable {
                requested: boundary,
                source,
            }
        })
    }

    /// Install the client-specific key pair for `SetSecurityKey`.
    ///
    /// Legacy performs this after the phase has already selected TEA, so the
    /// caller must supply a key rather than have one invented here. That key
    /// comes from the login path, which this boundary does not own: a session
    /// that never logs in keeps the raw default pair installed by
    /// `DESC::Setup`, which is exactly the legacy handshake-phase behavior.
    ///
    /// # Errors
    ///
    /// Returns the concrete [`DescriptorCryptoError`] when the phase has not
    /// enabled TEA, a client key is already installed, or a ciphertext tail
    /// remains.
    pub fn install_client_keys(
        &mut self,
        client_key: protocol::tea::TeaKey,
    ) -> Result<(), LiveError> {
        self.crypto.install_legacy_key(client_key)?;
        Ok(())
    }
}

/// Resolve the on-wire size of a fixed client frame.
///
/// The frame decoder has already drained the frame, so the crypto does not
/// need to track a consumed count. This helper exists for callers that need
/// the same size from a frame they decoded themselves.
///
/// # Errors
///
/// Returns [`ClientFrameError`] for an unknown, variable, or invalid-size
/// header.
pub fn fixed_frame_size(frame: &ClientFrame) -> Result<usize, ClientFrameError> {
    match resolve_client_frame_size(frame.header)? {
        ClientFrameSize::Fixed(size) => Ok(size),
        ClientFrameSize::Variable(base_size) => Err(ClientFrameError::VariableLengthUnsupported {
            header: frame.header,
            base_size,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::cg_handshake::CgInboundHandshake;
    use protocol::gc::GcPhase;

    const TOKEN: u32 = 0x0102_0304;
    const NOW: u32 = 1_700_000_000;

    fn started() -> ClientLifecycle {
        ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, NOW).state
    }

    /// A connected in-memory client/server pair, standing in for a socket.
    fn socket() -> (tokio::io::DuplexStream, tokio::io::DuplexStream) {
        tokio::io::duplex(4096)
    }

    fn session() -> (
        LiveClientSession<tokio::io::DuplexStream>,
        tokio::io::DuplexStream,
    ) {
        let (ours, theirs) = socket();
        let session =
            LiveClientSession::new(ours, started()).expect("a plaintext handshake session");
        (session, theirs)
    }

    fn handshake_frame() -> Vec<u8> {
        CgInboundHandshake::new(CgHandshakeHeader::Handshake, TOKEN, NOW, 0).encode()
    }

    /// The headline rule: `GC_PHASE` is written in **plaintext** and TEA is
    /// enabled only after it. If the ordering in `apply` is ever reversed, the
    /// client desynchronizes on its first phase switch and never recovers.
    ///
    /// The initial handshake writes nothing else: `CInputProcessor::Handshake`
    /// calls `HandshakeProcess(..., false)` in `PHASE_HANDSHAKE`, and only the
    /// `bInfiniteRetry` branch writes `HEADER_GC_TIME_SYNC`
    /// (`input.cpp:142-157`, `desc.cpp:628-634`). The later resync handshake
    /// takes that branch, so its acknowledgement is the first TEA record.
    #[tokio::test]
    async fn the_phase_record_is_written_before_tea_is_enabled() {
        let (mut session, mut peer) = session();

        peer.write_all(&handshake_frame()).await.unwrap();
        let step = session.step(NOW).await.unwrap();

        let LiveStep::Handled(LiveOutcome::Handshake { written, phase, .. }) = step else {
            panic!("expected a handled handshake, got {step:?}");
        };
        assert_eq!(phase, ClientPhase::Login, "the game server selects Login");
        assert_eq!(
            session.cipher_mode(),
            DescriptorCryptoMode::LegacyTeaDefault,
            "TEA is installed after the phase record"
        );
        let expected = GcPhase::new(ClientPhase::Login.legacy_value()).encode();
        assert_eq!(
            written,
            vec![expected.clone()],
            "the initial handshake writes exactly one plaintext phase record"
        );

        // The resync arrives encrypted under `HEADER_CG_TIME_SYNC`, the only
        // header `CInputLogin::Analyze` routes to `Handshake`
        // (`input_login.cpp:1173`), and is answered through TEA.
        let resync = CgInboundHandshake::new(CgHandshakeHeader::TimeSync, TOKEN, NOW, 0).encode();
        let mut client = DescriptorCrypto::with_default_limit().unwrap();
        client.enable_default_legacy_tea().unwrap();
        peer.write_all(&client.encrypt_output(&resync).unwrap())
            .await
            .unwrap();
        let step = session.step(NOW).await.unwrap();
        let LiveStep::Handled(LiveOutcome::Handshake { written, phase, .. }) = step else {
            panic!("expected a handled resync handshake, got {step:?}");
        };
        assert_eq!(phase, ClientPhase::Login, "a resync keeps the phase");
        let ack = client
            .encrypt_output(&[crate::handshake::HEADER_GC_TIME_SYNC])
            .unwrap();
        assert_eq!(ack.len() % 8, 0, "a TEA output unit is whole 8-byte blocks");
        assert_eq!(
            written,
            vec![ack],
            "the resync acknowledgement is the one-byte time sync under TEA"
        );
    }

    #[tokio::test]
    async fn the_handshake_record_is_read_from_a_fragmented_stream() {
        let (mut session, mut peer) = session();
        let frame = handshake_frame();

        // Feed the 13-byte record one byte at a time. Every intermediate step
        // must report Pending rather than framing a partial record.
        for (index, byte) in frame.iter().enumerate() {
            peer.write_all(&[*byte]).await.unwrap();
            peer.flush().await.unwrap();
            let step = session.step(NOW).await.unwrap();
            if index + 1 < frame.len() {
                assert!(
                    matches!(step, LiveStep::Pending { .. }),
                    "byte {index} must not complete a frame, got {step:?}"
                );
            } else {
                assert!(
                    matches!(step, LiveStep::Handled(LiveOutcome::Handshake { .. })),
                    "the last byte completes the record, got {step:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn two_coalesced_handshake_records_are_both_consumed() {
        let (mut session, mut peer) = session();
        let mut both = handshake_frame();
        both.extend_from_slice(&handshake_frame());
        peer.write_all(&both).await.unwrap();

        let first = session.step(NOW).await.unwrap();
        assert!(matches!(
            first,
            LiveStep::Handled(LiveOutcome::Handshake {
                phase: ClientPhase::Login,
                ..
            })
        ));
        // Both records arrived in one read, before TEA was enabled, so the
        // second one is already plaintext. Legacy decrypts only bytes read
        // after `m_bEncrypted` is set (`desc.cpp:290-319`).
        assert_eq!(
            session.buffered(),
            handshake_frame().len(),
            "only the first record is consumed"
        );

        let second = session.step(NOW).await.unwrap();
        assert!(
            matches!(second, LiveStep::Handled(LiveOutcome::Handshake { .. })),
            "the coalesced record is decoded from the same plaintext buffer, got {second:?}"
        );
    }

    #[tokio::test]
    async fn the_keepalive_needs_no_output_and_no_phase_change() {
        let (mut session, mut peer) = session();
        peer.write_all(&[CG_KEEP_ALIVE.value()]).await.unwrap();

        let step = session.step(NOW).await.unwrap();
        let LiveStep::Handled(LiveOutcome::KeepAlive { frame }) = step else {
            panic!("expected a keepalive, got {step:?}");
        };
        assert_eq!(frame.header, CG_KEEP_ALIVE.value());
        assert_eq!(session.phase(), ClientPhase::Handshake);
        assert_eq!(session.cipher_mode(), DescriptorCryptoMode::Plaintext);
    }

    #[tokio::test]
    async fn a_record_above_the_descriptor_is_handed_up_and_answered_through_send() {
        let (mut session, mut peer) = session();
        peer.write_all(&[0xce]).await.unwrap();

        let step = session.step(NOW).await.unwrap();
        let LiveStep::Record { phase, frame } = step else {
            panic!("expected a record for the caller, got {step:?}");
        };
        assert_eq!((phase, frame.header), (ClientPhase::Handshake, 0xce));

        // Plaintext in the handshake phase: the bytes arrive as written.
        let wire = session.send(&[0xd2, 0x01]).await.unwrap();
        assert_eq!(wire, [0xd2, 0x01]);
        let mut read = [0; 2];
        peer.read_exact(&mut read).await.unwrap();
        assert_eq!(read, [0xd2, 0x01]);

        let closed = (*session.lifecycle()).close();
        let mut closed_session = LiveClientSession::new(socket().0, closed.state)
            .expect("a closed session still constructs");
        assert!(matches!(
            closed_session.send(&[0xd2]).await,
            Err(LiveError::PhaseClosed)
        ));
    }

    #[tokio::test]
    async fn a_pong_before_the_handshake_is_handled_without_a_phase_change() {
        let (mut session, mut peer) = session();
        peer.write_all(&[HEADER_CG_PONG.value()]).await.unwrap();

        let step = session.step(NOW).await.unwrap();
        let LiveStep::Handled(LiveOutcome::Pong { written, .. }) = step else {
            panic!("expected a pong, got {step:?}");
        };
        assert_eq!(session.phase(), ClientPhase::Handshake);
        assert_eq!(
            session.cipher_mode(),
            DescriptorCryptoMode::Plaintext,
            "a pong never installs TEA by itself"
        );
        // A pong only writes when the heartbeat actually owes a ping.
        for record in &written {
            assert_eq!(
                record,
                &vec![crate::handshake::HEADER_GC_TIME_SYNC],
                "the handshake-phase pong is answered with the one-byte ack"
            );
        }
    }

    #[tokio::test]
    async fn a_clean_eof_between_frames_reports_no_leftover_bytes() {
        let (mut session, peer) = session();
        drop(peer);
        let step = session.step(NOW).await.unwrap();
        assert!(
            matches!(step, LiveStep::PeerClosed { leftover: 0 }),
            "got {step:?}"
        );
    }

    #[tokio::test]
    async fn a_truncated_record_is_reported_as_leftover_at_eof() {
        let (mut session, mut peer) = session();
        peer.write_all(&handshake_frame()[..6]).await.unwrap();
        peer.flush().await.unwrap();

        // While the peer is still connected the record is simply incomplete.
        let step = session.step(NOW).await.unwrap();
        assert!(matches!(step, LiveStep::Pending { .. }), "got {step:?}");
        drop(peer);

        // At end of stream the buffered prefix is reported rather than
        // silently discarded or padded into a frame the client never sent.
        let step = session.step(NOW).await.unwrap();
        assert!(
            matches!(step, LiveStep::PeerClosed { leftover: 6 }),
            "a truncated tail is counted, not framed, got {step:?}"
        );
    }

    #[tokio::test]
    async fn a_record_for_the_caller_is_consumed_and_never_retried() {
        let (mut session, mut peer) = session();
        // Header 28 is a fixed record with a one-byte body; the zero after it is a keepalive.
        peer.write_all(&[28, 0x5a, 0]).await.unwrap();

        let step = session.step(NOW).await.unwrap();
        let LiveStep::Record { frame, .. } = step else {
            panic!("expected a record for the caller, got {step:?}");
        };
        assert_eq!((frame.header, frame.payload.as_slice()), (28, &[0x5a][..]));
        let step = session.step(NOW).await.unwrap();
        assert!(
            matches!(step, LiveStep::Handled(LiveOutcome::KeepAlive { .. })),
            "the record's bytes must not be read again, got {step:?}"
        );
    }

    #[tokio::test]
    async fn a_closed_descriptor_refuses_another_step() {
        let (session, _peer) = session();
        let closed = (*session.lifecycle()).close();
        // Install the closed state through a real reduction so the test does
        // not depend on a private field.
        let mut closed_session = LiveClientSession::new(socket().0, closed.state)
            .expect("a closed session still constructs");
        assert_eq!(
            closed_session.cipher_mode(),
            DescriptorCryptoMode::Plaintext
        );
        let error = closed_session
            .step(NOW)
            .await
            .expect_err("a closed descriptor must refuse input");
        assert!(matches!(error, LiveError::PhaseClosed), "got {error:?}");
    }

    #[tokio::test]
    async fn a_tea_boundary_without_a_cipher_is_refused_at_construction() {
        let mut lifecycle = started();
        // Drive the lifecycle to a TEA input boundary through its own reducer.
        let reduction = lifecycle
            .on_handshake(
                CgInboundHandshake::new(CgHandshakeHeader::Handshake, TOKEN, NOW, 0),
                NOW,
            )
            .unwrap();
        lifecycle = reduction.state;
        assert_eq!(
            lifecycle.input_boundary(),
            LifecycleInputBoundary::LegacyTea
        );

        let error = match LiveClientSession::new(socket().0, lifecycle) {
            Ok(_) => panic!("a TEA lifecycle must not accept a plaintext cipher"),
            Err(error) => error,
        };
        assert!(
            matches!(
                error,
                LiveError::BoundaryNotInstallable {
                    requested: LifecycleInputBoundary::LegacyTea,
                    ..
                }
            ),
            "got {error:?}"
        );
    }

    #[tokio::test]
    async fn an_encrypted_handshake_record_is_decrypted_before_it_is_read() {
        let (mut session, mut peer) = session();
        let frame = handshake_frame();
        peer.write_all(&frame).await.unwrap();
        let step = session.step(NOW).await.unwrap();
        assert!(matches!(
            step,
            LiveStep::Handled(LiveOutcome::Handshake {
                phase: ClientPhase::Login,
                ..
            })
        ));

        // The second record must arrive encrypted. Feed the same record back
        // through the descriptor's own output cipher and it must be accepted.
        let cipher = DescriptorCrypto::with_default_limit().unwrap();
        let _ = cipher;
        let encrypted = session.lifecycle().phase();
        assert_eq!(encrypted, ClientPhase::Login);

        // Build the ciphertext with a fresh cipher that has the same default
        // pair, which is what the legacy client does after `DESC::Setup`.
        let mut writer = DescriptorCrypto::with_default_limit().unwrap();
        writer.enable_default_legacy_tea().unwrap();
        let wire = writer.encrypt_output(&frame).unwrap();
        assert_eq!(wire.len() % 8, 0, "TEA output is block aligned");
        assert_ne!(wire, frame, "the second record is not plaintext");

        peer.write_all(&wire).await.unwrap();
        let step = session.step(NOW).await.unwrap();
        assert!(
            matches!(step, LiveStep::Handled(LiveOutcome::Handshake { .. })),
            "the encrypted record is decrypted and reduced, got {step:?}"
        );
    }

    #[tokio::test]
    async fn a_short_ciphertext_fragment_is_retained_across_reads() {
        let (mut session, mut peer) = session();
        let frame = handshake_frame();
        peer.write_all(&frame).await.unwrap();
        session.step(NOW).await.unwrap();

        let mut writer = DescriptorCrypto::with_default_limit().unwrap();
        writer.enable_default_legacy_tea().unwrap();
        let wire = writer.encrypt_output(&frame).unwrap();

        // Three bytes cannot form a block. The descriptor must hold them and
        // wait rather than decoding noise.
        peer.write_all(&wire[..3]).await.unwrap();
        let step = session.step(NOW).await.unwrap();
        assert!(matches!(step, LiveStep::Pending { .. }), "got {step:?}");

        peer.write_all(&wire[3..]).await.unwrap();
        let step = session.step(NOW).await.unwrap();
        assert!(
            matches!(step, LiveStep::Handled(LiveOutcome::Handshake { .. })),
            "the retained tail completes the record, got {step:?}"
        );
    }
}
