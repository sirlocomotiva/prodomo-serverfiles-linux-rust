//! Isolated client session boundary for fixed legacy game-server frames.
//!
//! This module joins [`net::client_transport::ClientFrameTransport`] to a
//! deliberately small request dispatcher. It validates the fixed one-byte
//! header frame, keeps the complete [`protocol::cg_wire::ClientFrame`], and
//! exposes only the handshake keepalive and pong acknowledgement that are
//! source-verified in `server/server/game/input.cpp` and `desc.cpp`.
//!
//! Variable `Analyze` packets, descriptor state, and TEA encryption are not
//! implemented here. They are returned as typed errors instead of being
//! guessed or silently treated as fixed packets. The module is therefore not
//! a live gameplay session.

use std::error::Error;
use std::fmt;

use net::client_transport::{ClientFrameTransport, ClientTransportError};
use protocol::cg_inventory::{CG_KEEP_ALIVE, HEADER_CG_PONG};
use protocol::cg_wire::{
    resolve_client_frame_size, ClientFrame, ClientFrameError, ClientFrameSize,
};
use tokio::io::AsyncRead;

/// The legacy descriptor phase relevant to the fixed-frame boundary.
///
/// The numeric values match the `EPhase` enum in the legacy `packet.h`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientPhase {
    /// The descriptor is closed and consumes no client input.
    Close = 0,
    /// The initial handshake phase.
    Handshake = 1,
    /// The login phase.
    Login = 2,
    /// Character selection phase.
    Select = 3,
    /// Character loading phase.
    Loading = 4,
    /// The in-game phase.
    Game = 5,
    /// The dead-character phase.
    Dead = 6,
    /// The client-connecting phase used by the legacy descriptor.
    ClientConnecting = 7,
    /// The DB-client phase used by the legacy descriptor.
    DbClient = 8,
    /// The peer-to-peer phase.
    P2P = 9,
    /// The authentication-server phase.
    Auth = 10,
}

impl ClientPhase {
    /// Return the legacy numeric phase value.
    #[must_use]
    pub const fn legacy_value(self) -> u8 {
        self as u8
    }

    /// Return whether the active legacy build turns on TEA for this phase.
    ///
    /// The legacy `DESC::SetPhase` enables TEA when entering login, select,
    /// loading, game, dead, and auth phases unless improved packet encryption
    /// is compiled. This boundary does not decrypt those phases; the value is
    /// used only to return a typed boundary error.
    #[must_use]
    pub const fn legacy_tea_encrypted(self) -> bool {
        matches!(
            self,
            Self::Login | Self::Select | Self::Loading | Self::Game | Self::Dead | Self::Auth
        )
    }

    /// Return whether the legacy game input analyzer has a PONG case.
    ///
    /// The base processor handles the zero keepalive before analyzer dispatch,
    /// but PONG is explicitly handled by the handshake, login, main/dead, and
    /// auth analyzers. Internal DB/P2P descriptors do not have that case.
    /// Return whether the legacy `InputHandshake` analyzer family accepts a
    /// packed handshake or time-sync record in this phase.
    ///
    /// `InputLogin` and `InputAuth` both derive from `InputHandshake` and reuse
    /// the same internal handshake reducer, so all three phases accept the
    /// record. The main/dead analyzer does not, and neither do the internal
    /// descriptor phases.
    #[must_use]
    pub const fn accepts_handshake_record(self) -> bool {
        matches!(self, Self::Handshake | Self::Login | Self::Auth)
    }

    /// Return whether the legacy analyzer for this phase carries a PONG case.
    #[must_use]
    pub const fn accepts_pong(self) -> bool {
        matches!(
            self,
            Self::Handshake
                | Self::Login
                | Self::Select
                | Self::Loading
                | Self::Game
                | Self::Dead
                | Self::Auth
        )
    }
}

impl Default for ClientPhase {
    fn default() -> Self {
        Self::Handshake
    }
}

/// A fixed client frame that crossed the available source-verified boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientDispatchOutcome {
    /// The one-byte zero keepalive consumed by `CInputProcessor::Process`.
    KeepAlive {
        /// The complete frame, including its one-byte header.
        frame: ClientFrame,
    },
    /// The one-byte `HEADER_CG_PONG` acknowledgement accepted by the
    /// source-backed handshake, login, main/dead, and auth analyzers.
    Pong {
        /// The complete frame, including its one-byte header.
        frame: ClientFrame,
    },
}

impl ClientDispatchOutcome {
    /// Borrow the original fixed frame.
    #[must_use]
    pub const fn frame(&self) -> &ClientFrame {
        match self {
            Self::KeepAlive { frame } | Self::Pong { frame } => frame,
        }
    }

    /// Consume the outcome and return its original fixed frame.
    #[must_use]
    pub fn into_frame(self) -> ClientFrame {
        match self {
            Self::KeepAlive { frame } | Self::Pong { frame } => frame,
        }
    }
}

/// A legacy client feature that is intentionally outside this fixed boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientUnsupportedError {
    /// A variable `CInputMain::Analyze` header needs subheader/string logic.
    VariableAnalyze {
        /// The one-byte client header.
        header: u8,
        /// The packet-info prefix size, including the header.
        base_size: usize,
    },
    /// A descriptor phase or descriptor callback would be required.
    DescriptorPhase {
        /// The current descriptor phase.
        phase: ClientPhase,
        /// The one-byte client header.
        header: u8,
    },
    /// The descriptor is closed, so legacy input is ignored before header
    /// lookup.
    PhaseClosed {
        /// The current descriptor phase.
        phase: ClientPhase,
    },
    /// TEA-encrypted bytes must be decrypted before fixed framing.
    ///
    /// This error is intentionally headerless. A ciphertext byte cannot be
    /// interpreted as a legacy plaintext header.
    TeaRequired {
        /// The current descriptor phase.
        phase: ClientPhase,
    },
}

impl ClientUnsupportedError {
    /// Return the plaintext header when one was observed and classified.
    #[must_use]
    pub const fn header(self) -> Option<u8> {
        match self {
            Self::VariableAnalyze { header, .. } | Self::DescriptorPhase { header, .. } => {
                Some(header)
            }
            Self::PhaseClosed { .. } | Self::TeaRequired { .. } => None,
        }
    }

    /// Return whether this is the conservative variable-`Analyze` error.
    #[must_use]
    pub const fn is_variable_analyze(self) -> bool {
        matches!(self, Self::VariableAnalyze { .. })
    }

    /// Return whether this is a descriptor-phase boundary error.
    #[must_use]
    pub const fn is_descriptor_phase(self) -> bool {
        matches!(self, Self::DescriptorPhase { .. })
    }

    /// Return whether this is a closed-phase boundary error.
    #[must_use]
    pub const fn is_phase_closed(self) -> bool {
        matches!(self, Self::PhaseClosed { .. })
    }

    /// Return whether this is a TEA boundary error.
    #[must_use]
    pub const fn is_tea_boundary(self) -> bool {
        matches!(self, Self::TeaRequired { .. })
    }
}

impl fmt::Display for ClientUnsupportedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VariableAnalyze { header, base_size } => write!(
                formatter,
                "client header 0x{header:02x} needs variable Analyze logic (prefix {base_size} bytes)"
            ),
            Self::DescriptorPhase { phase, header } => write!(
                formatter,
                "client header 0x{header:02x} requires descriptor phase {phase:?}"
            ),
            Self::PhaseClosed { phase } => {
                write!(formatter, "client descriptor is closed in phase {phase:?}")
            }
            Self::TeaRequired { phase } => {
                write!(formatter, "client input requires TEA decryption in phase {phase:?}")
            }
        }
    }
}

impl Error for ClientUnsupportedError {}

/// A request-level error raised after a complete fixed frame was decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientDispatchError {
    /// The fixed frame itself failed protocol validation.
    Protocol(ClientFrameError),
    /// The frame is valid wire input but needs an unimplemented legacy feature.
    Unsupported(ClientUnsupportedError),
}

impl ClientDispatchError {
    /// Return whether this error is an unknown-header protocol error.
    #[must_use]
    pub const fn is_unknown_header(&self) -> bool {
        matches!(self, Self::Protocol(ClientFrameError::UnknownHeader { .. }))
    }
}

impl fmt::Display for ClientDispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => error.fmt(formatter),
            Self::Unsupported(error) => error.fmt(formatter),
        }
    }
}

impl Error for ClientDispatchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Protocol(error) => Some(error),
            Self::Unsupported(error) => Some(error),
        }
    }
}

impl From<ClientFrameError> for ClientDispatchError {
    fn from(error: ClientFrameError) -> Self {
        Self::Protocol(error)
    }
}

impl From<ClientUnsupportedError> for ClientDispatchError {
    fn from(error: ClientUnsupportedError) -> Self {
        Self::Unsupported(error)
    }
}

/// An error raised while reading or dispatching one client frame.
#[derive(Debug)]
pub enum ClientSessionError {
    /// The underlying stream or transport failed.
    Transport(ClientTransportError),
    /// A complete frame or session boundary failed protocol/dispatch
    /// validation.
    Dispatch(ClientDispatchError),
}

impl ClientSessionError {
    /// Return whether this error identifies an unknown client header.
    #[must_use]
    pub const fn is_unknown_header(&self) -> bool {
        matches!(self, Self::Dispatch(error) if error.is_unknown_header())
    }
}

impl fmt::Display for ClientSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => error.fmt(formatter),
            Self::Dispatch(error) => error.fmt(formatter),
        }
    }
}

impl Error for ClientSessionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::Dispatch(error) => Some(error),
        }
    }
}

impl From<ClientTransportError> for ClientSessionError {
    fn from(error: ClientTransportError) -> Self {
        Self::Transport(error)
    }
}

impl From<ClientDispatchError> for ClientSessionError {
    fn from(error: ClientDispatchError) -> Self {
        Self::Dispatch(error)
    }
}

impl From<ClientUnsupportedError> for ClientSessionError {
    fn from(error: ClientUnsupportedError) -> Self {
        Self::Dispatch(ClientDispatchError::Unsupported(error))
    }
}

/// Validate and dispatch one already framed fixed client packet.
///
/// The zero keepalive and pong paths are the only handlers enabled by this
/// boundary. `Pong` corresponds to `CInputProcessor::Pong`, which sets the
/// descriptor's pong flag; no synthetic wire response is emitted here.
///
/// The two-argument form derives the legacy TEA state from `phase`. Use
/// [`dispatch_client_frame_at_boundary`] when a caller has an explicit phase
/// and encryption state.
///
/// # Errors
///
/// Returns a protocol error for an unknown or incorrectly sized frame, and a
/// typed unsupported error for variable `Analyze`, descriptor, or TEA input.
pub fn dispatch_client_frame(
    frame: ClientFrame,
    phase: ClientPhase,
) -> Result<ClientDispatchOutcome, ClientDispatchError> {
    dispatch_client_frame_at_boundary(frame, phase, phase.legacy_tea_encrypted())
}

/// Validate and dispatch one frame with an explicit descriptor encryption flag.
///
/// # Errors
///
/// Returns the same errors as [`dispatch_client_frame`].
pub fn dispatch_client_frame_at_boundary(
    frame: ClientFrame,
    phase: ClientPhase,
    tea_encrypted: bool,
) -> Result<ClientDispatchOutcome, ClientDispatchError> {
    // CInputProcessor::Process checks PHASE_CLOSE before looking at a header.
    // Keep that precedence even for a malformed, unknown, or variable frame
    // supplied directly to this pure dispatcher.
    if phase == ClientPhase::Close {
        return Err(ClientUnsupportedError::PhaseClosed { phase }.into());
    }

    // TEA operates on the byte stream before CInputProcessor::Process. No
    // ciphertext byte is a meaningful plaintext header, so report this before
    // resolving or length-checking the supplied frame.
    if tea_encrypted {
        return Err(ClientUnsupportedError::TeaRequired { phase }.into());
    }

    let size = match resolve_client_frame_size(frame.header)? {
        ClientFrameSize::Fixed(size) => size,
        ClientFrameSize::Variable(base_size) => {
            return Err(ClientUnsupportedError::VariableAnalyze {
                header: frame.header,
                base_size,
            }
            .into());
        }
    };
    let actual = frame.encoded_len()?;
    if actual != size {
        return Err(ClientFrameError::LengthMismatch {
            header: frame.header,
            expected: size,
            actual,
        }
        .into());
    }

    if frame.header == CG_KEEP_ALIVE.value() {
        return Ok(ClientDispatchOutcome::KeepAlive { frame });
    }
    if frame.header == HEADER_CG_PONG.value() {
        if phase.accepts_pong() {
            return Ok(ClientDispatchOutcome::Pong { frame });
        }
        return Err(ClientUnsupportedError::DescriptorPhase {
            phase,
            header: frame.header,
        }
        .into());
    }

    // Every other fixed frame reaches a descriptor callback, a phase-specific
    // analyzer, or a gameplay service that is not part of this boundary.
    Err(ClientUnsupportedError::DescriptorPhase {
        phase,
        header: frame.header,
    }
    .into())
}

/// A fixed-frame client session with a small, explicit phase state.
///
/// The transport owns stream I/O. This wrapper validates request-level
/// boundaries and records pong acknowledgements, but it does not create a
/// descriptor, decrypt TEA, or execute gameplay.
#[derive(Debug)]
pub struct ClientSession<S> {
    transport: ClientFrameTransport<S>,
    phase: ClientPhase,
    tea_encrypted: bool,
    pong_received: bool,
}

impl<S> ClientSession<S> {
    /// Wrap a stream in the initial plaintext handshake phase.
    #[must_use]
    pub fn new(stream: S) -> Self {
        Self {
            transport: ClientFrameTransport::new(stream),
            phase: ClientPhase::Handshake,
            tea_encrypted: false,
            pong_received: false,
        }
    }

    /// Wrap a stream with an explicit complete-frame limit.
    #[must_use]
    pub fn with_max_frame_size(stream: S, max_frame_size: usize) -> Self {
        Self {
            transport: ClientFrameTransport::with_max_frame_size(stream, max_frame_size),
            phase: ClientPhase::Handshake,
            tea_encrypted: false,
            pong_received: false,
        }
    }

    /// Wrap a stream in a selected descriptor phase.
    ///
    /// The encryption flag follows the active legacy `DESC::SetPhase`
    /// behavior for the selected phase. It still does not implement TEA.
    #[must_use]
    pub fn with_phase(stream: S, phase: ClientPhase) -> Self {
        Self {
            transport: ClientFrameTransport::new(stream),
            phase,
            tea_encrypted: phase.legacy_tea_encrypted(),
            pong_received: false,
        }
    }

    /// Wrap a stream with an explicit phase and TEA state.
    #[must_use]
    pub fn with_phase_and_tea(stream: S, phase: ClientPhase, tea_encrypted: bool) -> Self {
        Self {
            transport: ClientFrameTransport::new(stream),
            phase,
            tea_encrypted,
            pong_received: false,
        }
    }

    /// Return the current descriptor phase.
    #[must_use]
    pub const fn phase(&self) -> ClientPhase {
        self.phase
    }

    /// Change the phase and select the legacy TEA state for that phase.
    pub fn set_phase(&mut self, phase: ClientPhase) {
        self.phase = phase;
        self.tea_encrypted = phase.legacy_tea_encrypted();
    }

    /// Return whether this boundary is currently treating input as TEA bytes.
    #[must_use]
    pub const fn tea_encrypted(&self) -> bool {
        self.tea_encrypted
    }

    /// Set the explicit TEA boundary flag without claiming decryption support.
    pub fn set_tea_encrypted(&mut self, tea_encrypted: bool) {
        self.tea_encrypted = tea_encrypted;
    }

    /// Return whether a source-verified pong has been received.
    #[must_use]
    pub const fn pong_received(&self) -> bool {
        self.pong_received
    }

    /// Clear the recorded pong acknowledgement and return its previous value.
    pub fn take_pong(&mut self) -> bool {
        std::mem::take(&mut self.pong_received)
    }

    /// Return the maximum complete incoming frame size.
    #[must_use]
    pub const fn max_frame_size(&self) -> usize {
        self.transport.max_frame_size()
    }

    /// Return the number of bytes retained by the incremental decoder.
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.transport.buffered_len()
    }

    /// Borrow the underlying stream.
    pub const fn get_ref(&self) -> &S {
        self.transport.get_ref()
    }

    /// Mutably borrow the underlying stream.
    pub const fn get_mut(&mut self) -> &mut S {
        self.transport.get_mut()
    }

    /// Remove the session wrapper and return the underlying stream.
    pub fn into_inner(self) -> S {
        self.transport.into_inner()
    }

    /// Return a terminal boundary error before the plaintext decoder runs.
    fn boundary_error(&self) -> Option<ClientSessionError> {
        if self.phase == ClientPhase::Close {
            return Some(ClientUnsupportedError::PhaseClosed { phase: self.phase }.into());
        }
        if self.tea_encrypted {
            return Some(ClientUnsupportedError::TeaRequired { phase: self.phase }.into());
        }
        None
    }
}

impl<S> ClientSession<S>
where
    S: AsyncRead + Unpin,
{
    /// Read the next complete fixed frame without dispatching it.
    ///
    /// Fragmented and coalesced TCP input is handled by
    /// [`ClientFrameTransport`]. A clean EOF returns `Ok(None)`. Closed and TEA
    /// phases are rejected before any ciphertext byte is interpreted as a
    /// plaintext header. Variable `Analyze` errors are normalized to
    /// [`ClientDispatchError::Unsupported`].
    ///
    /// # Errors
    ///
    /// Returns transport/protocol errors or a typed session-boundary error.
    pub async fn read_frame(&mut self) -> Result<Option<ClientFrame>, ClientSessionError> {
        if let Some(error) = self.boundary_error() {
            return Err(error);
        }
        self.transport
            .read_frame()
            .await
            .map_err(map_transport_error)
    }

    /// Read and dispatch the next complete fixed frame.
    ///
    /// `Ok(None)` means clean EOF between frames. A pong outcome sets
    /// [`Self::pong_received`]. No response is written by this method.
    ///
    /// # Errors
    ///
    /// Returns transport/protocol errors or typed unsupported errors for
    /// variable `Analyze`, descriptor, and TEA boundaries.
    pub async fn read_dispatch(
        &mut self,
    ) -> Result<Option<ClientDispatchOutcome>, ClientSessionError> {
        match self.read_frame().await? {
            Some(frame) => {
                let outcome =
                    dispatch_client_frame_at_boundary(frame, self.phase, self.tea_encrypted)?;
                if matches!(outcome, ClientDispatchOutcome::Pong { .. }) {
                    self.pong_received = true;
                }
                Ok(Some(outcome))
            }
            None => Ok(None),
        }
    }

    /// Alias for [`Self::read_dispatch`].
    ///
    /// # Errors
    ///
    /// See [`Self::read_dispatch`].
    pub async fn read_packet(
        &mut self,
    ) -> Result<Option<ClientDispatchOutcome>, ClientSessionError> {
        self.read_dispatch().await
    }
}

fn map_transport_error(error: ClientTransportError) -> ClientSessionError {
    match error {
        ClientTransportError::Frame(ClientFrameError::VariableLengthUnsupported {
            header,
            base_size,
        }) => ClientSessionError::Dispatch(
            ClientUnsupportedError::VariableAnalyze { header, base_size }.into(),
        ),
        // Keep protocol failures in the same public category as direct
        // dispatch, regardless of whether the frame came from the stream or
        // an already-framed value. I/O failures remain transport errors.
        ClientTransportError::Frame(protocol) => {
            ClientSessionError::Dispatch(ClientDispatchError::Protocol(protocol))
        }
        io_error @ ClientTransportError::Io(_) => ClientSessionError::Transport(io_error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::cg_inventory::{HEADER_CG_CHAT, HEADER_CG_MOVE, HEADER_CG_TIME_SYNC};
    use tokio::io::{duplex, AsyncWriteExt};

    fn move_frame() -> ClientFrame {
        ClientFrame::new(HEADER_CG_MOVE.value(), vec![0x11; 15])
    }

    fn time_sync_frame() -> ClientFrame {
        ClientFrame::new(HEADER_CG_TIME_SYNC.value(), vec![0x22; 12])
    }

    fn pong_frame() -> ClientFrame {
        ClientFrame::new(HEADER_CG_PONG.value(), Vec::new())
    }

    #[tokio::test]
    async fn reads_byte_fragmented_fixed_frames_without_changing_sizes() {
        let first = move_frame();
        let second = time_sync_frame();
        let mut wire = first.encode().unwrap();
        wire.extend_from_slice(&second.encode().unwrap());
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            for byte in wire {
                writer.write_all(&[byte]).await.unwrap();
            }
        });

        let mut session = ClientSession::new(reader);
        let first_read = session.read_frame().await.unwrap().unwrap();
        let second_read = session.read_frame().await.unwrap().unwrap();
        assert_eq!(first_read, first);
        assert_eq!(second_read, second);
        assert_eq!(first_read.encoded_len().unwrap(), 16);
        assert_eq!(second_read.encoded_len().unwrap(), 13);
        assert_eq!(session.read_frame().await.unwrap(), None);
        feed.await.unwrap();
    }

    #[tokio::test]
    async fn leaves_coalesced_frames_buffered_for_successive_dispatches() {
        let first = pong_frame();
        let second = ClientFrame::new(0, Vec::new());
        let mut wire = first.encode().unwrap();
        wire.extend_from_slice(&second.encode().unwrap());
        let (mut writer, reader) = duplex(wire.len());
        writer.write_all(&wire).await.unwrap();
        drop(writer);

        let mut session = ClientSession::new(reader);
        assert_eq!(
            session.read_dispatch().await.unwrap(),
            Some(ClientDispatchOutcome::Pong { frame: first })
        );
        assert_eq!(
            session.read_dispatch().await.unwrap(),
            Some(ClientDispatchOutcome::KeepAlive { frame: second })
        );
        assert!(session.pong_received());
        assert_eq!(session.read_dispatch().await.unwrap(), None);
    }

    #[test]
    fn returns_the_source_verified_pong_ack_path() {
        let frame = pong_frame();
        assert_eq!(
            dispatch_client_frame(frame.clone(), ClientPhase::Handshake).unwrap(),
            ClientDispatchOutcome::Pong { frame }
        );
    }

    #[test]
    fn returns_typed_descriptor_and_tea_boundary_errors() {
        let frame = move_frame();
        assert_eq!(
            dispatch_client_frame_at_boundary(frame.clone(), ClientPhase::Handshake, false),
            Err(ClientDispatchError::Unsupported(
                ClientUnsupportedError::DescriptorPhase {
                    phase: ClientPhase::Handshake,
                    header: HEADER_CG_MOVE.value(),
                }
            ))
        );
        assert_eq!(
            dispatch_client_frame_at_boundary(frame, ClientPhase::Game, true),
            Err(ClientDispatchError::Unsupported(
                ClientUnsupportedError::TeaRequired {
                    phase: ClientPhase::Game,
                }
            ))
        );
    }

    /// A variable header whose declared size is below its own base prefix never reaches
    /// the analyzer. The live transport resolves the frame itself, so the failure is the
    /// typed `InvalidVariableSize` rather than the fixed-only
    /// `VariableLengthUnsupported` the transport used to report.
    #[tokio::test]
    async fn refuses_a_variable_declaration_below_its_base_prefix() {
        let (mut writer, reader) = duplex(4);
        let feed = tokio::spawn(async move {
            writer
                .write_all(&[HEADER_CG_CHAT.value(), 0, 0, 0])
                .await
                .unwrap();
        });
        let mut session = ClientSession::new(reader);
        match session.read_dispatch().await {
            Err(ClientSessionError::Dispatch(ClientDispatchError::Protocol(
                ClientFrameError::InvalidVariableSize {
                    header,
                    declared_size,
                    base_size,
                },
            ))) => {
                assert_eq!(header, HEADER_CG_CHAT.value());
                assert_eq!(declared_size, 0);
                assert_eq!(base_size, 4);
            }
            other => panic!("unexpected variable Analyze result: {other:?}"),
        }
        feed.await.unwrap();
    }

    #[tokio::test]
    async fn preserves_unknown_header_as_a_typed_protocol_error() {
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            writer.write_all(&[0x99]).await.unwrap();
        });
        let mut session = ClientSession::new(reader);
        assert!(matches!(
            session.read_dispatch().await,
            Err(ClientSessionError::Dispatch(ClientDispatchError::Protocol(
                ClientFrameError::UnknownHeader { header: 0x99 }
            )))
        ));
        feed.await.unwrap();
    }

    #[tokio::test]
    async fn reports_clean_eof_between_frames() {
        let (writer, reader) = duplex(1);
        drop(writer);
        let mut session = ClientSession::new(reader);
        assert_eq!(session.read_dispatch().await.unwrap(), None);
    }

    #[test]
    fn rejects_a_directly_constructed_unknown_frame() {
        assert_eq!(
            dispatch_client_frame(ClientFrame::new(0x99, [1, 2, 3]), ClientPhase::Handshake),
            Err(ClientDispatchError::Protocol(
                ClientFrameError::UnknownHeader { header: 0x99 }
            ))
        );
    }

    #[test]
    fn close_precedes_header_classification() {
        assert_eq!(
            dispatch_client_frame(ClientFrame::new(0x99, [1, 2, 3]), ClientPhase::Close),
            Err(ClientDispatchError::Unsupported(
                ClientUnsupportedError::PhaseClosed {
                    phase: ClientPhase::Close,
                }
            ))
        );
    }

    #[test]
    fn pong_is_limited_to_source_supported_client_phases() {
        let frame = pong_frame();
        assert!(matches!(
            dispatch_client_frame(frame.clone(), ClientPhase::P2P),
            Err(ClientDispatchError::Unsupported(
                ClientUnsupportedError::DescriptorPhase {
                    phase: ClientPhase::P2P,
                    ..
                }
            ))
        ));
        assert!(matches!(
            dispatch_client_frame(frame, ClientPhase::Handshake),
            Ok(ClientDispatchOutcome::Pong { .. })
        ));
    }

    #[tokio::test]
    async fn tea_is_reported_before_plaintext_decoder_reads_bytes() {
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            writer.write_all(&[0x99]).await.unwrap();
        });
        let mut session = ClientSession::with_phase_and_tea(reader, ClientPhase::Game, true);
        assert!(matches!(
            session.read_dispatch().await,
            Err(ClientSessionError::Dispatch(
                ClientDispatchError::Unsupported(ClientUnsupportedError::TeaRequired {
                    phase: ClientPhase::Game,
                })
            ))
        ));
        assert_eq!(session.buffered_len(), 0);
        feed.await.unwrap();
    }

    #[tokio::test]
    async fn close_is_reported_before_plaintext_decoder_reads_bytes() {
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            writer.write_all(&[0x99]).await.unwrap();
        });
        let mut session = ClientSession::with_phase(reader, ClientPhase::Close);
        assert!(matches!(
            session.read_dispatch().await,
            Err(ClientSessionError::Dispatch(
                ClientDispatchError::Unsupported(ClientUnsupportedError::PhaseClosed {
                    phase: ClientPhase::Close,
                })
            ))
        ));
        assert_eq!(session.buffered_len(), 0);
        feed.await.unwrap();
    }
}
