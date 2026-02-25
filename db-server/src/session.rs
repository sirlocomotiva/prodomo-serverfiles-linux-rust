//! Isolated DB peer session boundary.
//!
//! This module joins [`net::db_transport`] to request-level validation. It
//! does not open sockets by itself, query tables, construct a boot response,
//! or claim that any legacy request is ready for execution. Boot, horse-name,
//! channel-lookup, login-by-key, primary player-load, add-affect, and
//! remove-affect requests have typed fixed-width validation, plus an explicit
//! active-x86 setup decoder;
//! execution remains the caller's responsibility.

use std::error::Error;
use std::fmt;

use net::db_transport::{DbFrameTransport, DbTransportError};
use protocol::db_boot::{DbBootError, DbBootRequest, HEADER_GD_BOOT};
use protocol::db_records::{
    AddAffectRequest, ChannelChangeRequest, DbRecordError, HorseNameRequest, LoginByKeyRequest,
    PlayerLoadRequest, RemoveAffectRequest, HEADER_GD_ADD_AFFECT, HEADER_GD_FIND_CHANNEL,
    HEADER_GD_LOGIN_BY_KEY, HEADER_GD_PLAYER_LOAD, HEADER_GD_REMOVE_AFFECT,
    HEADER_GD_REQ_HORSE_NAME,
};
use protocol::db_wire::DbFrame;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::setup::{
    decode_setup_request, SetupDecodeError, SetupDecodeLimits, SetupFeatureProfile, SetupRequest,
    HEADER_GD_SETUP,
};

/// A typed DB request that passed its available wire validation.
///
/// The boot, setup, horse-name, channel-lookup, login-by-key, player-load,
/// add-affect, and remove-affect requests currently have source-verified request codecs in this
/// boundary. Other recognized request headers are returned as
/// [`DbDispatchOutcome::NotReady`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbRequest {
    /// A validated 24-byte `TPacketGDBoot` request.
    Boot {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Decoded legacy boot request.
        request: DbBootRequest,
    },
    /// A validated active-x86 `HEADER_GD_SETUP` request.
    Setup {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Strictly decoded setup base and declared login records.
        request: SetupRequest,
    },
    /// A validated horse-name lookup request.
    HorseName {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Decoded four-byte player-ID request.
        request: HorseNameRequest,
    },
    /// A validated channel-lookup request.
    FindChannel {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Decoded packed map/channel request.
        request: ChannelChangeRequest,
    },
    /// A validated key-based login request.
    LoginByKey {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Decoded packed login selector and client keys.
        request: LoginByKeyRequest,
    },
    /// A validated primary player-load request.
    PlayerLoad {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Decoded packed account/player selector.
        request: PlayerLoadRequest,
    },
    /// A validated add-affect request.
    AddAffect {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Decoded packed player ID and affect element.
        request: AddAffectRequest,
    },
    /// A validated remove-affect request.
    RemoveAffect {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Decoded packed player/type/apply-on selector.
        request: RemoveAffectRequest,
    },
}

/// The result of dispatching one complete DB peer frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbDispatchOutcome {
    /// The frame maps to a typed request and can be handed to a later service.
    Validated(DbRequest),
    /// The header is routed by the active legacy DB server, but this Rust
    /// boundary has no typed codec or executable service for it yet.
    ///
    /// No response is implied or constructed by this variant.
    NotReady {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Recognized legacy request header.
        header: u8,
        /// Number of payload bytes received.
        payload_len: usize,
    },
}

/// A request-level validation or dispatch error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbRequestError {
    /// The header is not handled by the active legacy DB request router.
    ///
    /// The complete frame has already been consumed. A caller may log this
    /// error and continue reading, matching the legacy router's default arm.
    UnknownHeader {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Unrecognized header value.
        header: u8,
    },
    /// A boot request payload did not match the source-verified fixed width.
    InvalidBootPayload {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Error returned by the boot request decoder.
        source: DbBootError,
    },
    /// A setup payload did not match the selected active-x86 setup profile.
    InvalidSetupPayload {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Error returned by the setup request decoder.
        source: SetupDecodeError,
    },
    /// A horse-name request payload did not match the source-verified width.
    InvalidHorseNamePayload {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Error returned by the horse-name request decoder.
        source: DbRecordError,
    },
    /// A key-based login request payload did not match the source-verified width.
    InvalidLoginByKeyPayload {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Error returned by the login-by-key request decoder.
        source: DbRecordError,
    },
    /// A channel-lookup request payload did not match the source-verified width.
    InvalidChannelRequestPayload {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Error returned by the channel-request decoder.
        source: DbRecordError,
    },
    /// A player-load request payload did not match the source-verified width.
    InvalidPlayerLoadPayload {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Error returned by the player-load request decoder.
        source: DbRecordError,
    },
    /// An add-affect request payload did not match the source-verified width.
    InvalidAddAffectPayload {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Error returned by the add-affect request decoder.
        source: DbRecordError,
    },
    /// A remove-affect request payload did not match the source-verified width.
    InvalidRemoveAffectPayload {
        /// Correlation handle copied from the DB peer frame.
        handle: u32,
        /// Error returned by the remove-affect request decoder.
        source: DbRecordError,
    },
}

impl fmt::Display for DbRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownHeader { handle, header } => write!(
                formatter,
                "unknown DB request header {header} for handle {handle}"
            ),
            Self::InvalidBootPayload { handle, source } => {
                write!(
                    formatter,
                    "invalid DB boot request for handle {handle}: {source}"
                )
            }
            Self::InvalidSetupPayload { handle, source } => {
                write!(
                    formatter,
                    "invalid DB setup request for handle {handle}: {source}"
                )
            }
            Self::InvalidHorseNamePayload { handle, source } => {
                write!(
                    formatter,
                    "invalid DB horse-name request for handle {handle}: {source}"
                )
            }
            Self::InvalidLoginByKeyPayload { handle, source } => {
                write!(
                    formatter,
                    "invalid DB login-by-key request for handle {handle}: {source}"
                )
            }
            Self::InvalidChannelRequestPayload { handle, source } => {
                write!(
                    formatter,
                    "invalid DB channel request for handle {handle}: {source}"
                )
            }
            Self::InvalidPlayerLoadPayload { handle, source } => {
                write!(
                    formatter,
                    "invalid DB player-load request for handle {handle}: {source}"
                )
            }
            Self::InvalidAddAffectPayload { handle, source } => {
                write!(
                    formatter,
                    "invalid DB add-affect request for handle {handle}: {source}"
                )
            }
            Self::InvalidRemoveAffectPayload { handle, source } => {
                write!(
                    formatter,
                    "invalid DB remove-affect request for handle {handle}: {source}"
                )
            }
        }
    }
}

#[allow(clippy::match_same_arms)]
impl Error for DbRequestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::UnknownHeader { .. } => None,
            Self::InvalidBootPayload { source, .. } => Some(source),
            Self::InvalidSetupPayload { source, .. } => Some(source),
            Self::InvalidHorseNamePayload { source, .. } => Some(source),
            Self::InvalidLoginByKeyPayload { source, .. } => Some(source),
            Self::InvalidChannelRequestPayload { source, .. } => Some(source),
            Self::InvalidPlayerLoadPayload { source, .. } => Some(source),
            Self::InvalidAddAffectPayload { source, .. } => Some(source),
            Self::InvalidRemoveAffectPayload { source, .. } => Some(source),
        }
    }
}

/// An error raised while reading or dispatching a DB peer request.
#[derive(Debug)]
pub enum DbSessionError {
    /// Reading or decoding the peer frame failed.
    Transport(DbTransportError),
    /// The complete frame failed request-level validation.
    Request(DbRequestError),
}

impl fmt::Display for DbSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => error.fmt(formatter),
            Self::Request(error) => error.fmt(formatter),
        }
    }
}

impl Error for DbSessionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::Request(error) => Some(error),
        }
    }
}

impl From<DbTransportError> for DbSessionError {
    fn from(error: DbTransportError) -> Self {
        Self::Transport(error)
    }
}

impl From<DbRequestError> for DbSessionError {
    fn from(error: DbRequestError) -> Self {
        Self::Request(error)
    }
}

/// Validate and dispatch one already framed DB peer request using the
/// repository's active-x86 setup profile and default decoder limits.
///
/// The setup profile and limits are selected by this named compatibility
/// boundary; they are not inferred from the setup payload. Callers that need
/// a different allocation policy should use
/// [`dispatch_db_frame_with_setup_validation`].
///
/// # Errors
///
/// Returns the same typed request and unknown-header errors as
/// [`dispatch_db_frame_with_setup_validation`], using the active-x86 profile
/// and default setup limits.
pub fn dispatch_db_frame(frame: DbFrame) -> Result<DbDispatchOutcome, DbRequestError> {
    dispatch_db_frame_with_setup_validation(
        frame,
        SetupFeatureProfile::ActiveX86,
        SetupDecodeLimits::default(),
    )
}

/// Validate and dispatch one framed request with explicit setup validation
/// settings.
///
/// The boot branch uses [`DbBootRequest::decode`], so the payload must be
/// exactly [`BOOT_REQUEST_WIRE_SIZE`](protocol::db_boot::BOOT_REQUEST_WIRE_SIZE)
/// bytes. The setup branch uses [`decode_setup_request`] and requires exactly
/// 154 bytes plus 100 bytes for each declared active-x86 login record. The
/// horse-name branch uses [`HorseNameRequest::decode`] and requires exactly
/// four payload bytes. The channel branch uses
/// [`ChannelChangeRequest::decode`] and requires exactly eight payload bytes.
/// The player-load branch uses [`PlayerLoadRequest::decode`] and requires
/// exactly nine payload bytes. The login-by-key branch uses
/// [`LoginByKeyRequest::decode`] and requires exactly 67 payload bytes. The
/// add-affect branch uses [`AddAffectRequest::decode`] and requires exactly 25
/// payload bytes. The remove-affect branch uses [`RemoveAffectRequest::decode`]
/// and requires exactly nine payload bytes. These exact checks intentionally harden
/// boundaries where the legacy C++ router casts or trusts the packet length.
///
/// Headers in the current active legacy router without a typed request codec
/// are accepted as [`DbDispatchOutcome::NotReady`]. A header outside that
/// router is rejected as unknown.
///
/// # Errors
///
/// Returns [`DbRequestError::UnknownHeader`] for an unhandled header,
/// [`DbRequestError::InvalidBootPayload`] for a malformed boot request,
/// [`DbRequestError::InvalidSetupPayload`] for a malformed setup request,
/// [`DbRequestError::InvalidHorseNamePayload`] for a malformed horse-name
/// request, [`DbRequestError::InvalidChannelRequestPayload`] for a malformed
/// channel request, [`DbRequestError::InvalidPlayerLoadPayload`] for a
/// malformed player-load request, [`DbRequestError::InvalidLoginByKeyPayload`]
/// for a malformed login-by-key request,
/// [`DbRequestError::InvalidAddAffectPayload`] for a malformed add-affect
/// request, and [`DbRequestError::InvalidRemoveAffectPayload`] for a malformed
/// remove-affect request.
pub fn dispatch_db_frame_with_setup_validation(
    frame: DbFrame,
    setup_profile: SetupFeatureProfile,
    setup_limits: SetupDecodeLimits,
) -> Result<DbDispatchOutcome, DbRequestError> {
    let DbFrame {
        header,
        handle,
        payload,
    } = frame;

    if header == HEADER_GD_BOOT {
        return DbBootRequest::decode(&payload)
            .map(|request| DbDispatchOutcome::Validated(DbRequest::Boot { handle, request }))
            .map_err(|source| DbRequestError::InvalidBootPayload { handle, source });
    }

    if header == HEADER_GD_SETUP {
        return decode_setup_request(&payload, setup_profile, setup_limits)
            .map(|request| DbDispatchOutcome::Validated(DbRequest::Setup { handle, request }))
            .map_err(|source| DbRequestError::InvalidSetupPayload { handle, source });
    }

    if header == HEADER_GD_REQ_HORSE_NAME {
        return HorseNameRequest::decode(&payload)
            .map(|request| DbDispatchOutcome::Validated(DbRequest::HorseName { handle, request }))
            .map_err(|source| DbRequestError::InvalidHorseNamePayload { handle, source });
    }

    if header == HEADER_GD_LOGIN_BY_KEY {
        return LoginByKeyRequest::decode(&payload)
            .map(|request| DbDispatchOutcome::Validated(DbRequest::LoginByKey { handle, request }))
            .map_err(|source| DbRequestError::InvalidLoginByKeyPayload { handle, source });
    }

    if header == HEADER_GD_FIND_CHANNEL {
        return ChannelChangeRequest::decode(&payload)
            .map(|request| DbDispatchOutcome::Validated(DbRequest::FindChannel { handle, request }))
            .map_err(|source| DbRequestError::InvalidChannelRequestPayload { handle, source });
    }

    if header == HEADER_GD_PLAYER_LOAD {
        return PlayerLoadRequest::decode(&payload)
            .map(|request| DbDispatchOutcome::Validated(DbRequest::PlayerLoad { handle, request }))
            .map_err(|source| DbRequestError::InvalidPlayerLoadPayload { handle, source });
    }

    if header == HEADER_GD_ADD_AFFECT {
        return AddAffectRequest::decode(&payload)
            .map(|request| DbDispatchOutcome::Validated(DbRequest::AddAffect { handle, request }))
            .map_err(|source| DbRequestError::InvalidAddAffectPayload { handle, source });
    }

    if header == HEADER_GD_REMOVE_AFFECT {
        return RemoveAffectRequest::decode(&payload)
            .map(|request| {
                DbDispatchOutcome::Validated(DbRequest::RemoveAffect { handle, request })
            })
            .map_err(|source| DbRequestError::InvalidRemoveAffectPayload { handle, source });
    }

    if is_legacy_routable_header(header) {
        return Ok(DbDispatchOutcome::NotReady {
            handle,
            header,
            payload_len: payload.len(),
        });
    }

    Err(DbRequestError::UnknownHeader { handle, header })
}

/// A DB peer stream with request-level dispatch.
///
/// This wrapper does not choose a response policy. A later service may write a
/// caller-supplied frame through [`Self::write_response`], but this module does
/// not synthesize a boot response. Its default 16 MiB frame cap is independent
/// of the setup decoder's caller-selected record limit; direct framed dispatch
/// has no transport cap and still enforces the setup decoder limit.
#[derive(Debug)]
pub struct DbPeerSession<S> {
    transport: DbFrameTransport<S>,
}

impl<S> DbPeerSession<S> {
    /// Wrap a stream with the default incoming payload limit.
    pub fn new(stream: S) -> Self {
        Self {
            transport: DbFrameTransport::new(stream),
        }
    }

    /// Wrap a stream with an explicit incoming payload limit.
    pub fn with_max_payload_size(stream: S, max_payload_size: usize) -> Self {
        Self {
            transport: DbFrameTransport::with_max_payload_size(stream, max_payload_size),
        }
    }

    /// Return the maximum accepted incoming payload size.
    pub fn max_payload_size(&self) -> usize {
        self.transport.max_payload_size()
    }

    /// Return the number of bytes retained by the incremental frame decoder.
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
}

impl<S> DbPeerSession<S>
where
    S: AsyncRead + Unpin,
{
    /// Read and dispatch the next complete DB peer request.
    ///
    /// TCP fragmentation and multiple complete frames in one read are handled
    /// by [`net::db_transport::DbFrameTransport`]. `Ok(None)` means the peer
    /// closed cleanly between frames.
    ///
    /// This method does not write any response. In particular, a validated
    /// boot request is not a loaded-table or boot-ready signal.
    ///
    /// # Errors
    ///
    /// Returns [`DbSessionError`] for transport, frame, or request validation
    /// failures.
    pub async fn read_request(&mut self) -> Result<Option<DbDispatchOutcome>, DbSessionError> {
        self.read_request_with_setup_validation(
            SetupFeatureProfile::ActiveX86,
            SetupDecodeLimits::default(),
        )
        .await
    }

    /// Read and dispatch the next complete request with explicit setup
    /// validation settings.
    ///
    /// This is the transport-aware form of
    /// [`dispatch_db_frame_with_setup_validation`]. It still performs no SQL,
    /// state mutation, authentication, response construction, or write.
    ///
    /// # Errors
    ///
    /// Returns [`DbSessionError`] for transport, frame, or request validation
    /// failures.
    pub async fn read_request_with_setup_validation(
        &mut self,
        setup_profile: SetupFeatureProfile,
        setup_limits: SetupDecodeLimits,
    ) -> Result<Option<DbDispatchOutcome>, DbSessionError> {
        self.transport
            .read_frame()
            .await?
            .map(|frame| {
                dispatch_db_frame_with_setup_validation(frame, setup_profile, setup_limits)
            })
            .transpose()
            .map_err(DbSessionError::Request)
    }
}

// Values routed by `CClientManager::ProcessPackets` in the active x86 build
// (`server/server/db/ClientManager.cpp` and `server/server/common/tables.h`).
// Gaps are intentional: the legacy router logs those values as unknown.
const LEGACY_ROUTABLE_HEADERS: &[u8] = &[
    2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14, 15, 16, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 30, 31,
    32, 33, 34, 35, 36, 37, 38, 39, 40, 42, 43, 44, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57,
    60, 61, 62, 70, 71, 72, 73, 74, 75, 76, 100, 101, 107, 108, 109, 110, 115, 116, 117, 118, 119,
    120, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 137, 138, 139, 140,
    141, 142, 145, 146, 147, 150, 151, 152, 153, 154, 155, 156, 157, 199, 200, 255,
];

fn is_legacy_routable_header(header: u8) -> bool {
    LEGACY_ROUTABLE_HEADERS.contains(&header)
}

impl<S> DbPeerSession<S>
where
    S: AsyncWrite + Unpin,
{
    /// Encode and write one complete DB response frame.
    ///
    /// The caller owns the response policy. This method does not synthesize a
    /// boot response, load tables, or infer a feature profile.
    ///
    /// # Errors
    ///
    /// Returns a transport or frame-encoding error if the response cannot be
    /// written.
    pub async fn write_response(&mut self, frame: &DbFrame) -> Result<(), DbSessionError> {
        self.transport
            .write_frame(frame)
            .await
            .map_err(DbSessionError::Transport)
    }

    /// Flush the underlying stream after a response write.
    ///
    /// # Errors
    ///
    /// Returns the underlying stream error.
    pub async fn flush(&mut self) -> Result<(), DbSessionError> {
        self.transport
            .flush()
            .await
            .map_err(DbSessionError::Transport)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use net::db_transport::DbFrameTransport;
    use protocol::db_boot::BOOT_REQUEST_WIRE_SIZE;
    use tokio::io::{duplex, AsyncWriteExt};

    use crate::setup::{LOGIN_ON_SETUP_WIRE_SIZE, SETUP_BASE_WIRE_SIZE};

    fn boot_request() -> DbBootRequest {
        let mut ip = [0_u8; 16];
        ip[..9].copy_from_slice(b"127.0.0.1");
        DbBootRequest::new([10, 20], ip)
    }

    fn boot_frame(handle: u32) -> DbFrame {
        DbFrame::new(HEADER_GD_BOOT, handle, boot_request().encode().to_vec())
    }

    fn horse_frame(handle: u32, player_id: u32) -> DbFrame {
        DbFrame::new(
            HEADER_GD_REQ_HORSE_NAME,
            handle,
            HorseNameRequest::new(player_id).encode(),
        )
    }

    fn channel_frame(handle: u32, map_index: i32, channel: i32) -> DbFrame {
        DbFrame::new(
            HEADER_GD_FIND_CHANNEL,
            handle,
            ChannelChangeRequest::new(map_index, channel).encode(),
        )
    }

    fn player_frame(handle: u32) -> DbFrame {
        DbFrame::new(
            HEADER_GD_PLAYER_LOAD,
            handle,
            PlayerLoadRequest {
                account_id: 11,
                player_id: 22,
                account_index: 2,
            }
            .encode(),
        )
    }

    fn add_affect_frame(handle: u32) -> DbFrame {
        DbFrame::new(
            HEADER_GD_ADD_AFFECT,
            handle,
            AddAffectRequest::new(
                0x0102_0304,
                protocol::db_records::AffectElementRecord::new(
                    0xaabb_ccdd,
                    0x7f,
                    -123,
                    0x0102_0304,
                    -456,
                    789,
                ),
            )
            .encode(),
        )
    }

    fn remove_affect_frame(handle: u32) -> DbFrame {
        DbFrame::new(
            HEADER_GD_REMOVE_AFFECT,
            handle,
            RemoveAffectRequest::new(0x0102_0304, 0xaabb_ccdd, 0x7f).encode(),
        )
    }

    fn login_frame(handle: u32) -> DbFrame {
        let mut login = [0_u8; 31];
        login[..5].copy_from_slice(b"alice");
        let mut ip = [0_u8; 16];
        ip[..9].copy_from_slice(b"127.0.0.1");
        DbFrame::new(
            HEADER_GD_LOGIN_BY_KEY,
            handle,
            LoginByKeyRequest {
                login,
                login_key: 0x1020_3040,
                client_key: [1, 2, 3, 4],
                ip,
            }
            .encode(),
        )
    }

    fn setup_frame(handle: u32, login_count: u32, auth_server: u8) -> DbFrame {
        let mut payload =
            vec![0_u8; SETUP_BASE_WIRE_SIZE + login_count as usize * LOGIN_ON_SETUP_WIRE_SIZE];
        payload[149..153].copy_from_slice(&login_count.to_le_bytes());
        payload[153] = auth_server;
        DbFrame::new(HEADER_GD_SETUP, handle, payload)
    }

    #[tokio::test]
    async fn reads_a_byte_fragmented_boot_request() {
        let expected_request = boot_request();
        let wire = boot_frame(0x1234_5678).encode().unwrap();
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            for byte in wire {
                writer.write_all(&[byte]).await.unwrap();
            }
        });

        let mut session = DbPeerSession::new(reader);
        assert_eq!(
            session.read_request().await.unwrap(),
            Some(DbDispatchOutcome::Validated(DbRequest::Boot {
                handle: 0x1234_5678,
                request: expected_request,
            }))
        );
        feed.await.unwrap();
    }

    #[tokio::test]
    async fn reads_a_byte_fragmented_setup_request() {
        let frame = setup_frame(0x1020_3040, 1, 0);
        let wire = frame.encode().unwrap();
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            for byte in wire {
                writer.write_all(&[byte]).await.unwrap();
            }
        });

        let mut session = DbPeerSession::new(reader);
        let outcome = session.read_request().await.unwrap();
        let Some(DbDispatchOutcome::Validated(DbRequest::Setup { handle, request })) = outcome
        else {
            panic!("expected a validated setup request, got {outcome:?}");
        };
        assert_eq!(handle, 0x1020_3040);
        assert_eq!(request.base().login_count, 1);
        assert_eq!(request.logins().len(), 1);
        assert_eq!(session.buffered_len(), 0);
        feed.await.unwrap();
    }

    #[tokio::test]
    async fn consumes_a_complete_frame_before_rejecting_malformed_setup() {
        let wire = DbFrame::new(HEADER_GD_SETUP, 33, vec![0_u8; SETUP_BASE_WIRE_SIZE - 1])
            .encode()
            .unwrap();
        let (mut writer, reader) = duplex(wire.len());
        writer.write_all(&wire).await.unwrap();
        drop(writer);

        let mut session = DbPeerSession::new(reader);
        let error = session.read_request().await.unwrap_err();
        assert!(matches!(
            error,
            DbSessionError::Request(DbRequestError::InvalidSetupPayload {
                handle: 33,
                source: SetupDecodeError::InvalidWidth { .. }
            })
        ));
        assert_eq!(session.buffered_len(), 0);
    }

    #[tokio::test]
    async fn reads_coalesced_setup_and_not_ready_requests() {
        let setup = setup_frame(17, 0, 1);
        let mut wire = setup.encode().unwrap();
        wire.extend_from_slice(&DbFrame::new(42, 18, vec![5, 6, 7]).encode().unwrap());
        let (mut writer, reader) = duplex(wire.len());
        writer.write_all(&wire).await.unwrap();
        drop(writer);

        let mut session = DbPeerSession::new(reader);
        let first = session.read_request().await.unwrap();
        assert!(matches!(
            first,
            Some(DbDispatchOutcome::Validated(DbRequest::Setup {
                handle: 17,
                ref request
            })) if request.base().auth_server == 1 && request.logins().is_empty()
        ));
        assert_eq!(
            session.read_request().await.unwrap(),
            Some(DbDispatchOutcome::NotReady {
                handle: 18,
                header: 42,
                payload_len: 3,
            })
        );
        assert_eq!(session.read_request().await.unwrap(), None);
    }

    #[test]
    fn validates_setup_width_count_and_auth_invariants() {
        let valid = dispatch_db_frame(setup_frame(11, 1, 0)).unwrap();
        let DbDispatchOutcome::Validated(DbRequest::Setup { handle, request }) = valid else {
            panic!("expected a validated setup request");
        };
        assert_eq!(handle, 11);
        assert_eq!(request.base().login_count, 1);
        assert_eq!(request.logins().len(), 1);

        let short = DbFrame::new(HEADER_GD_SETUP, 12, {
            let mut payload = vec![0_u8; SETUP_BASE_WIRE_SIZE];
            payload[149..153].copy_from_slice(&1_u32.to_le_bytes());
            payload
        });
        assert_eq!(
            dispatch_db_frame(short),
            Err(DbRequestError::InvalidSetupPayload {
                handle: 12,
                source: SetupDecodeError::InvalidWidth {
                    expected: SETUP_BASE_WIRE_SIZE + LOGIN_ON_SETUP_WIRE_SIZE,
                    actual: SETUP_BASE_WIRE_SIZE,
                },
            })
        );

        let long = DbFrame::new(HEADER_GD_SETUP, 13, {
            let mut payload = vec![0_u8; SETUP_BASE_WIRE_SIZE + 1];
            payload[153] = 1;
            payload
        });
        assert_eq!(
            dispatch_db_frame(long),
            Err(DbRequestError::InvalidSetupPayload {
                handle: 13,
                source: SetupDecodeError::TrailingBytes {
                    expected: SETUP_BASE_WIRE_SIZE,
                    actual: SETUP_BASE_WIRE_SIZE + 1,
                },
            })
        );

        let auth_with_login = setup_frame(14, 1, 1);
        assert_eq!(
            dispatch_db_frame(auth_with_login),
            Err(DbRequestError::InvalidSetupPayload {
                handle: 14,
                source: SetupDecodeError::AuthServerMustBeBaseOnly { login_count: 1 },
            })
        );

        assert_eq!(
            dispatch_db_frame_with_setup_validation(
                setup_frame(15, 1, 0),
                SetupFeatureProfile::ActiveX86,
                SetupDecodeLimits::new(0),
            ),
            Err(DbRequestError::InvalidSetupPayload {
                handle: 15,
                source: SetupDecodeError::LoginCountLimitExceeded {
                    login_count: 1,
                    max_login_records: 0,
                },
            })
        );
    }

    #[tokio::test]
    async fn reads_coalesced_validated_and_not_ready_requests() {
        let mut wire = boot_frame(7).encode().unwrap();
        wire.extend_from_slice(&DbFrame::new(2, 8, vec![1, 2, 3]).encode().unwrap());
        let (mut writer, reader) = duplex(wire.len());
        writer.write_all(&wire).await.unwrap();
        drop(writer);

        let mut session = DbPeerSession::new(reader);
        assert_eq!(
            session.read_request().await.unwrap(),
            Some(DbDispatchOutcome::Validated(DbRequest::Boot {
                handle: 7,
                request: boot_request(),
            }))
        );
        assert_eq!(
            session.read_request().await.unwrap(),
            Some(DbDispatchOutcome::NotReady {
                handle: 8,
                header: 2,
                payload_len: 3,
            })
        );
        assert_eq!(session.read_request().await.unwrap(), None);
    }

    #[test]
    fn validates_the_exact_boot_request_width() {
        let valid = dispatch_db_frame(boot_frame(11)).unwrap();
        assert!(matches!(
            valid,
            DbDispatchOutcome::Validated(DbRequest::Boot { handle: 11, .. })
        ));

        let short = DbFrame::new(HEADER_GD_BOOT, 12, vec![0; BOOT_REQUEST_WIRE_SIZE - 1]);
        assert_eq!(
            dispatch_db_frame(short),
            Err(DbRequestError::InvalidBootPayload {
                handle: 12,
                source: DbBootError::InvalidFixedRecordSize {
                    field: "boot_request",
                    expected: BOOT_REQUEST_WIRE_SIZE,
                    actual: BOOT_REQUEST_WIRE_SIZE - 1,
                },
            })
        );

        let long = DbFrame::new(HEADER_GD_BOOT, 13, vec![0; BOOT_REQUEST_WIRE_SIZE + 1]);
        assert_eq!(
            dispatch_db_frame(long),
            Err(DbRequestError::InvalidBootPayload {
                handle: 13,
                source: DbBootError::InvalidFixedRecordSize {
                    field: "boot_request",
                    expected: BOOT_REQUEST_WIRE_SIZE,
                    actual: BOOT_REQUEST_WIRE_SIZE + 1,
                },
            })
        );
    }

    #[tokio::test]
    async fn reads_a_byte_fragmented_horse_name_request() {
        let wire = horse_frame(0x89ab_cdef, 0x1020_3040).encode().unwrap();
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            for byte in wire {
                writer.write_all(&[byte]).await.unwrap();
            }
        });

        let mut session = DbPeerSession::new(reader);
        assert_eq!(
            session.read_request().await.unwrap(),
            Some(DbDispatchOutcome::Validated(DbRequest::HorseName {
                handle: 0x89ab_cdef,
                request: HorseNameRequest::new(0x1020_3040),
            }))
        );
        feed.await.unwrap();
    }

    #[tokio::test]
    async fn reads_a_byte_fragmented_channel_request() {
        let wire = channel_frame(0x1234_5678, 0x0102_0304, 2).encode().unwrap();
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            for byte in wire {
                writer.write_all(&[byte]).await.unwrap();
            }
        });

        let mut session = DbPeerSession::new(reader);
        assert_eq!(
            session.read_request().await.unwrap(),
            Some(DbDispatchOutcome::Validated(DbRequest::FindChannel {
                handle: 0x1234_5678,
                request: ChannelChangeRequest::new(0x0102_0304, 2),
            }))
        );
        feed.await.unwrap();
    }

    #[test]
    fn validates_the_exact_horse_name_request_width() {
        let valid = dispatch_db_frame(horse_frame(11, 0x0102_0304)).unwrap();
        assert_eq!(
            valid,
            DbDispatchOutcome::Validated(DbRequest::HorseName {
                handle: 11,
                request: HorseNameRequest::new(0x0102_0304),
            })
        );

        let short = DbFrame::new(HEADER_GD_REQ_HORSE_NAME, 12, vec![0; 3]);
        assert_eq!(
            dispatch_db_frame(short),
            Err(DbRequestError::InvalidHorseNamePayload {
                handle: 12,
                source: DbRecordError::Truncated {
                    record: "horse-name request",
                    needed: 4,
                    available: 3,
                },
            })
        );

        let long = DbFrame::new(HEADER_GD_REQ_HORSE_NAME, 13, vec![0; 5]);
        assert_eq!(
            dispatch_db_frame(long),
            Err(DbRequestError::InvalidHorseNamePayload {
                handle: 13,
                source: DbRecordError::LengthMismatch {
                    record: "horse-name request",
                    expected: 4,
                    actual: 5,
                },
            })
        );
    }

    #[test]
    fn validates_the_exact_channel_request_width() {
        let valid = dispatch_db_frame(channel_frame(11, 0x0102_0304, 2)).unwrap();
        assert_eq!(
            valid,
            DbDispatchOutcome::Validated(DbRequest::FindChannel {
                handle: 11,
                request: ChannelChangeRequest::new(0x0102_0304, 2),
            })
        );

        let short = DbFrame::new(HEADER_GD_FIND_CHANNEL, 12, vec![0; 7]);
        assert_eq!(
            dispatch_db_frame(short),
            Err(DbRequestError::InvalidChannelRequestPayload {
                handle: 12,
                source: DbRecordError::Truncated {
                    record: "TPacketChangeChannel",
                    needed: 8,
                    available: 7,
                },
            })
        );

        let long = DbFrame::new(HEADER_GD_FIND_CHANNEL, 13, vec![0; 9]);
        assert_eq!(
            dispatch_db_frame(long),
            Err(DbRequestError::InvalidChannelRequestPayload {
                handle: 13,
                source: DbRecordError::LengthMismatch {
                    record: "TPacketChangeChannel",
                    expected: 8,
                    actual: 9,
                },
            })
        );
    }

    #[tokio::test]
    async fn reads_a_byte_fragmented_login_by_key_request() {
        let wire = login_frame(0x1234_5678).encode().unwrap();
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            for byte in wire {
                writer.write_all(&[byte]).await.unwrap();
            }
        });

        let mut session = DbPeerSession::new(reader);
        let request = LoginByKeyRequest::decode(&login_frame(0x1234_5678).payload).unwrap();
        assert_eq!(
            session.read_request().await.unwrap(),
            Some(DbDispatchOutcome::Validated(DbRequest::LoginByKey {
                handle: 0x1234_5678,
                request,
            }))
        );
        feed.await.unwrap();
    }

    #[test]
    fn validates_the_exact_login_by_key_request_width() {
        let valid = dispatch_db_frame(login_frame(0)).unwrap();
        assert!(matches!(
            valid,
            DbDispatchOutcome::Validated(DbRequest::LoginByKey { handle: 0, .. })
        ));

        let short = DbFrame::new(HEADER_GD_LOGIN_BY_KEY, 12, vec![0; 66]);
        assert_eq!(
            dispatch_db_frame(short),
            Err(DbRequestError::InvalidLoginByKeyPayload {
                handle: 12,
                source: DbRecordError::Truncated {
                    record: "TPacketGDLoginByKey",
                    needed: 67,
                    available: 66,
                },
            })
        );

        let long = DbFrame::new(HEADER_GD_LOGIN_BY_KEY, 13, vec![0; 68]);
        assert_eq!(
            dispatch_db_frame(long),
            Err(DbRequestError::InvalidLoginByKeyPayload {
                handle: 13,
                source: DbRecordError::LengthMismatch {
                    record: "TPacketGDLoginByKey",
                    expected: 67,
                    actual: 68,
                },
            })
        );
    }

    #[tokio::test]
    async fn reads_a_byte_fragmented_player_load_request() {
        let wire = player_frame(0xa1b2_c3d4).encode().unwrap();
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            for byte in wire {
                writer.write_all(&[byte]).await.unwrap();
            }
        });

        let mut session = DbPeerSession::new(reader);
        assert_eq!(
            session.read_request().await.unwrap(),
            Some(DbDispatchOutcome::Validated(DbRequest::PlayerLoad {
                handle: 0xa1b2_c3d4,
                request: PlayerLoadRequest {
                    account_id: 11,
                    player_id: 22,
                    account_index: 2,
                },
            }))
        );
        feed.await.unwrap();
    }

    #[test]
    fn validates_the_exact_player_load_request_width() {
        let valid = dispatch_db_frame(player_frame(0)).unwrap();
        assert_eq!(
            valid,
            DbDispatchOutcome::Validated(DbRequest::PlayerLoad {
                handle: 0,
                request: PlayerLoadRequest {
                    account_id: 11,
                    player_id: 22,
                    account_index: 2,
                },
            })
        );

        let short = DbFrame::new(HEADER_GD_PLAYER_LOAD, 12, vec![0; 8]);
        assert_eq!(
            dispatch_db_frame(short),
            Err(DbRequestError::InvalidPlayerLoadPayload {
                handle: 12,
                source: DbRecordError::Truncated {
                    record: "TPlayerLoadPacket",
                    needed: 9,
                    available: 8,
                },
            })
        );

        let long = DbFrame::new(HEADER_GD_PLAYER_LOAD, 13, vec![0; 10]);
        assert_eq!(
            dispatch_db_frame(long),
            Err(DbRequestError::InvalidPlayerLoadPayload {
                handle: 13,
                source: DbRecordError::LengthMismatch {
                    record: "TPlayerLoadPacket",
                    expected: 9,
                    actual: 10,
                },
            })
        );
    }

    #[test]
    fn validates_the_exact_add_affect_request_and_preserves_handle() {
        let handle = 0xfeed_beef;
        let expected = AddAffectRequest::new(
            0x0102_0304,
            protocol::db_records::AffectElementRecord::new(
                0xaabb_ccdd,
                0x7f,
                -123,
                0x0102_0304,
                -456,
                789,
            ),
        );
        let frame = add_affect_frame(handle);
        assert_eq!(frame.payload.len(), AddAffectRequest::WIRE_SIZE);
        assert_eq!(
            dispatch_db_frame(frame),
            Ok(DbDispatchOutcome::Validated(DbRequest::AddAffect {
                handle,
                request: expected,
            }))
        );
    }

    #[test]
    fn rejects_every_short_add_affect_payload_and_trailing_bytes() {
        let complete = add_affect_frame(0).payload;
        for available in 0..AddAffectRequest::WIRE_SIZE {
            let handle = u32::try_from(available).unwrap();
            let error = dispatch_db_frame(DbFrame::new(
                HEADER_GD_ADD_AFFECT,
                handle,
                complete[..available].to_vec(),
            ));
            assert_eq!(
                error,
                Err(DbRequestError::InvalidAddAffectPayload {
                    handle,
                    source: DbRecordError::Truncated {
                        record: "TPacketGDAddAffect",
                        needed: AddAffectRequest::WIRE_SIZE,
                        available,
                    },
                })
            );
        }

        let mut long = complete;
        long.push(0xaa);
        assert_eq!(
            dispatch_db_frame(DbFrame::new(HEADER_GD_ADD_AFFECT, 99, long)),
            Err(DbRequestError::InvalidAddAffectPayload {
                handle: 99,
                source: DbRecordError::LengthMismatch {
                    record: "TPacketGDAddAffect",
                    expected: AddAffectRequest::WIRE_SIZE,
                    actual: AddAffectRequest::WIRE_SIZE + 1,
                },
            })
        );
    }

    #[tokio::test]
    async fn reads_byte_fragmented_add_affect_frames_without_consuming_the_next_frame() {
        let mut wire = add_affect_frame(0x1020_3040).encode().unwrap();
        wire.extend_from_slice(&add_affect_frame(0x5060_7080).encode().unwrap());
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            for byte in wire {
                writer.write_all(&[byte]).await.unwrap();
            }
        });

        let mut session = DbPeerSession::new(reader);
        assert_eq!(
            session.read_request().await.unwrap(),
            Some(DbDispatchOutcome::Validated(DbRequest::AddAffect {
                handle: 0x1020_3040,
                request: AddAffectRequest::new(
                    0x0102_0304,
                    protocol::db_records::AffectElementRecord::new(
                        0xaabb_ccdd,
                        0x7f,
                        -123,
                        0x0102_0304,
                        -456,
                        789,
                    ),
                ),
            }))
        );
        assert_eq!(
            session.read_request().await.unwrap(),
            Some(DbDispatchOutcome::Validated(DbRequest::AddAffect {
                handle: 0x5060_7080,
                request: AddAffectRequest::new(
                    0x0102_0304,
                    protocol::db_records::AffectElementRecord::new(
                        0xaabb_ccdd,
                        0x7f,
                        -123,
                        0x0102_0304,
                        -456,
                        789,
                    ),
                ),
            }))
        );
        assert_eq!(session.read_request().await.unwrap(), None);
        feed.await.unwrap();
    }

    #[test]
    fn validates_the_exact_remove_affect_request_and_preserves_handle() {
        let handle = 0xfeed_beef;
        let expected = RemoveAffectRequest::new(0x0102_0304, 0xaabb_ccdd, 0x7f);
        let frame = remove_affect_frame(handle);
        assert_eq!(
            frame.payload,
            vec![0x04, 0x03, 0x02, 0x01, 0xdd, 0xcc, 0xbb, 0xaa, 0x7f]
        );
        assert_eq!(
            dispatch_db_frame(frame),
            Ok(DbDispatchOutcome::Validated(DbRequest::RemoveAffect {
                handle,
                request: expected,
            }))
        );
    }

    #[test]
    fn rejects_every_short_remove_affect_payload_and_trailing_bytes() {
        let complete = RemoveAffectRequest::new(1, 2, 3).encode();
        for available in 0..RemoveAffectRequest::WIRE_SIZE {
            let handle = u32::try_from(available).unwrap();
            let error = dispatch_db_frame(DbFrame::new(
                HEADER_GD_REMOVE_AFFECT,
                handle,
                complete[..available].to_vec(),
            ));
            assert!(matches!(
                error,
                Err(DbRequestError::InvalidRemoveAffectPayload {
                    handle: error_handle,
                    source: DbRecordError::Truncated {
                        record: "TPacketGDRemoveAffect",
                        needed: 9,
                        available: _
                    }
                }) if error_handle == handle
            ));
        }

        let mut long = complete;
        long.push(0xaa);
        assert_eq!(
            dispatch_db_frame(DbFrame::new(HEADER_GD_REMOVE_AFFECT, 99, long)),
            Err(DbRequestError::InvalidRemoveAffectPayload {
                handle: 99,
                source: DbRecordError::LengthMismatch {
                    record: "TPacketGDRemoveAffect",
                    expected: 9,
                    actual: 10,
                },
            })
        );
    }

    #[tokio::test]
    async fn reads_a_byte_fragmented_remove_affect_request() {
        let wire = remove_affect_frame(0x1020_3040).encode().unwrap();
        let (mut writer, reader) = duplex(1);
        let feed = tokio::spawn(async move {
            for byte in wire {
                writer.write_all(&[byte]).await.unwrap();
            }
        });

        let mut session = DbPeerSession::new(reader);
        assert_eq!(
            session.read_request().await.unwrap(),
            Some(DbDispatchOutcome::Validated(DbRequest::RemoveAffect {
                handle: 0x1020_3040,
                request: RemoveAffectRequest::new(0x0102_0304, 0xaabb_ccdd, 0x7f),
            }))
        );
        feed.await.unwrap();
    }

    #[test]
    fn reports_a_recognized_unimplemented_request_as_not_ready() {
        assert_eq!(
            dispatch_db_frame(DbFrame::new(42, 99, vec![5, 6, 7])),
            Ok(DbDispatchOutcome::NotReady {
                handle: 99,
                header: 42,
                payload_len: 3,
            })
        );
    }

    #[test]
    fn rejects_a_header_outside_the_legacy_router() {
        assert_eq!(
            dispatch_db_frame(DbFrame::new(0xab, 0xfeed_beef, vec![1, 2, 3])),
            Err(DbRequestError::UnknownHeader {
                handle: 0xfeed_beef,
                header: 0xab,
            })
        );
    }

    #[tokio::test]
    async fn writes_a_complete_response_frame_without_choosing_one() {
        let response = DbFrame::new(42, 7, vec![1, 2, 3, 4]);
        let expected = response.clone();
        let (client, server) = duplex(64);
        let mut session = DbPeerSession::new(server);
        let writer = tokio::spawn(async move {
            session.write_response(&response).await.unwrap();
            session.flush().await.unwrap();
        });

        let mut client = DbFrameTransport::new(client);
        assert_eq!(client.read_frame().await.unwrap(), Some(expected));
        writer.await.unwrap();
    }
}
