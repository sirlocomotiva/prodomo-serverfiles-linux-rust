//! Injected DB request execution boundary.
//!
//! This module composes request validation, the immutable boot response
//! adapter, a caller-owned horse-name lookup, an optional caller-owned channel
//! resolver, and deliberately non-executable setup/login/player-load,
//! add-affect, and remove-affect boundaries. It deliberately performs no SQL queries, opens no
//! database connections, authenticates peers, or loads boot tables. All lookup
//! seams are explicit test or integration boundaries.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::boot::{BootResponseAdapter, BootResponseError};
use crate::session::{DbDispatchOutcome, DbPeerSession, DbRequest, DbSessionError};
use crate::setup::SetupRequest;
use crate::setup_reply::{SetupReplyError, SetupReplyOutcome};
use protocol::db_records::{
    AddAffectRequest, ChannelChangeRequest, ChannelResultRecord, HorseNameRecord,
    LoginByKeyRequest, PlayerLoadRequest, RemoveAffectRequest, HEADER_DG_ACK_HORSE_NAME,
    HEADER_DG_CHANNEL_RESULT, HEADER_GD_ADD_AFFECT, HEADER_GD_FIND_CHANNEL, HEADER_GD_LOGIN_BY_KEY,
    HEADER_GD_PLAYER_LOAD, HEADER_GD_REMOVE_AFFECT, LEGACY_CHARACTER_NAME_BYTES,
};
use protocol::db_wire::DbFrame;

/// A caller-owned horse-name lookup used by [`DbRequestService`].
///
/// `None` means the legacy `SELECT` found no row. The service then emits the
/// source-defined zero-filled 25-byte name buffer. `Some` bytes are copied
/// exactly, which lets an integration decide how to canonicalize a C string
/// instead of reproducing uninitialized stack-tail bytes.
pub trait HorseNameLookup {
    /// Return the exact raw 25-byte name buffer for a player, if present.
    fn lookup(&self, player_id: u32) -> Option<[u8; LEGACY_CHARACTER_NAME_BYTES]>;
}

impl<F> HorseNameLookup for F
where
    F: Fn(u32) -> Option<[u8; LEGACY_CHARACTER_NAME_BYTES]>,
{
    fn lookup(&self, player_id: u32) -> Option<[u8; LEGACY_CHARACTER_NAME_BYTES]> {
        self(player_id)
    }
}

impl<T> HorseNameLookup for Arc<T>
where
    T: HorseNameLookup + ?Sized,
{
    fn lookup(&self, player_id: u32) -> Option<[u8; LEGACY_CHARACTER_NAME_BYTES]> {
        (**self).lookup(player_id)
    }
}

/// An in-memory channel endpoint returned by an injected resolver.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelEndpoint {
    /// The legacy `inet_addr` result stored in signed x86 `long` form.
    pub address: i32,
    /// The game-server listen port.
    pub port: u16,
}

impl ChannelEndpoint {
    /// Construct an endpoint from the legacy address and port values.
    pub const fn new(address: i32, port: u16) -> Self {
        Self { address, port }
    }
}

/// A caller-owned in-memory channel registry seam.
///
/// The production DB binary does not implement this trait. A test or future
/// peer-registry integration supplies the map/channel lookup; `None` means no
/// matching peer and maps to the source-defined all-zero result record.
pub trait ChannelResolver {
    /// Resolve a map and channel to a public address and listen port.
    fn resolve(&self, map_index: i32, channel: i32) -> Option<ChannelEndpoint>;
}

impl<F> ChannelResolver for F
where
    F: Fn(i32, i32) -> Option<ChannelEndpoint>,
{
    fn resolve(&self, map_index: i32, channel: i32) -> Option<ChannelEndpoint> {
        self(map_index, channel)
    }
}

impl<T> ChannelResolver for Arc<T>
where
    T: ChannelResolver + ?Sized,
{
    fn resolve(&self, map_index: i32, channel: i32) -> Option<ChannelEndpoint> {
        (**self).resolve(map_index, channel)
    }
}

impl ChannelResolver for () {
    fn resolve(&self, _map_index: i32, _channel: i32) -> Option<ChannelEndpoint> {
        None
    }
}

/// An error while composing a channel-result response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelResponseError {
    /// No channel registry was injected.
    Unavailable,
    /// The supplied typed request was not a channel lookup.
    NotChannelRequest,
}

impl fmt::Display for ChannelResponseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("no channel resolver is available"),
            Self::NotChannelRequest => formatter.write_str("request is not a channel request"),
        }
    }
}

impl Error for ChannelResponseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Unavailable | Self::NotChannelRequest => None,
        }
    }
}

/// A small response adapter around a caller-owned [`ChannelResolver`].
///
/// The adapter preserves the legacy distinction between an invalid zero-valued
/// request, which produces no frame, and a valid request with no matching peer,
/// which produces one zero-valued result frame. It never creates a peer
/// registry itself.
#[derive(Debug)]
pub struct ChannelResponseAdapter<R> {
    resolver: Option<R>,
}

impl<R> ChannelResponseAdapter<R> {
    /// Construct an adapter backed by the supplied resolver.
    pub fn new(resolver: R) -> Self {
        Self {
            resolver: Some(resolver),
        }
    }

    /// Construct an adapter that reports that no channel registry is available.
    pub fn unavailable() -> Self {
        Self { resolver: None }
    }

    /// Return whether a resolver was injected.
    pub fn is_ready(&self) -> bool {
        self.resolver.is_some()
    }
}

impl<R> ChannelResponseAdapter<R>
where
    R: ChannelResolver,
{
    /// Compose the response for one validated DB request.
    ///
    /// `Ok(None)` means the legacy handler intentionally sends no frame because
    /// `lMapIndex` or `iChannel` is zero. `Ok(Some(frame))` is the one response
    /// frame expected for a valid request, including an all-zero record when
    /// the resolver finds no matching peer.
    ///
    /// # Errors
    ///
    /// Returns [`ChannelResponseError::Unavailable`] when no resolver is
    /// injected, or [`ChannelResponseError::NotChannelRequest`] for another
    /// typed request.
    pub fn response_for(
        &self,
        request: &DbRequest,
    ) -> Result<Option<DbFrame>, ChannelResponseError> {
        match request {
            DbRequest::FindChannel { handle, request } => {
                if !request.is_valid() {
                    return Ok(None);
                }
                let resolver = self
                    .resolver
                    .as_ref()
                    .ok_or(ChannelResponseError::Unavailable)?;
                let record = resolver
                    .resolve(request.map_index, request.channel)
                    .map_or_else(ChannelResultRecord::missing, |endpoint| {
                        ChannelResultRecord::new(endpoint.address, endpoint.port)
                    });
                Ok(Some(DbFrame::new(
                    HEADER_DG_CHANNEL_RESULT,
                    *handle,
                    record.encode(),
                )))
            }
            DbRequest::Boot { .. }
            | DbRequest::Setup { .. }
            | DbRequest::HorseName { .. }
            | DbRequest::LoginByKey { .. }
            | DbRequest::PlayerLoad { .. }
            | DbRequest::AddAffect { .. }
            | DbRequest::RemoveAffect { .. } => Err(ChannelResponseError::NotChannelRequest),
        }
    }
}

/// A response decision for one already validated or recognized DB request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbServiceOutcome {
    /// The service wrote one response frame with the reported header/handle.
    Responded {
        /// Header written on the DB peer frame.
        header: u8,
        /// Handle written on the DB peer frame.
        handle: u32,
    },
    /// The request was validated but the legacy handler intentionally sends
    /// no response.
    NoResponse {
        /// Correlation handle copied from the input frame.
        handle: u32,
        /// Request header that produced no response.
        header: u8,
    },
    /// The request is recognized but has no injected service implementation.
    ///
    /// No bytes are written for this outcome.
    NotReady {
        /// Correlation handle copied from the input frame.
        handle: u32,
        /// Recognized legacy request header.
        header: u8,
        /// Input payload length.
        payload_len: usize,
    },
}

/// An error raised while executing a validated DB request.
#[derive(Debug)]
pub enum DbServiceError {
    /// The boot response could not be encoded by the injected adapter.
    BootResponse(BootResponseError),
    /// The injected channel adapter could not compose a response.
    ChannelResponse(ChannelResponseError),
    /// The setup reply could not be composed.
    SetupReply(SetupReplyError),
    /// Writing or flushing the selected response failed.
    Session(DbSessionError),
}

impl fmt::Display for DbServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BootResponse(error) => error.fmt(formatter),
            Self::ChannelResponse(error) => error.fmt(formatter),
            Self::SetupReply(error) => error.fmt(formatter),
            Self::Session(error) => error.fmt(formatter),
        }
    }
}

impl Error for DbServiceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::BootResponse(error) => Some(error),
            Self::ChannelResponse(error) => Some(error),
            Self::SetupReply(error) => Some(error),
            Self::Session(error) => Some(error),
        }
    }
}

/// A per-request boot composer for a live server.
///
/// [`BootResponseAdapter`] answers from one injected immutable snapshot. That
/// is the right shape for a test and for a server that loads its tables once
/// and serves an identical payload forever.
///
/// A live server cannot use that shape. The legacy `QUERY_BOOT` consumes two
/// item-ID ranges from a shared FIFO on **every** call, so consecutive boot
/// requests from two different peers must receive different payloads. The boot
/// response is therefore a function of server state, not of the request, and it
/// has to be recomposed per request.
///
/// Implementations must:
///
/// - consume exactly one item-ID range pair per call, the way `GetRange` does;
/// - emit the legacy zero handle and `HEADER_DG_BOOT`, whatever handle the
///   request carried; and
/// - never read a feature profile out of payload bytes.
///
/// The method is `async` because legacy resolves the GM host and
/// administrator lists *inside* the handler: `__GetHostInfo` and
/// `__GetAdminInfo` each run a `SQL_COMMON` query per request, and
/// `__GetAdminInfo` filters on the requesting peer's address. A responder that
/// cannot await would have to either cache those lists (which changes their
/// contents between requests) or leave them permanently empty.
pub trait BootResponder {
    /// Compose the complete boot response frame for one validated boot request.
    ///
    /// # Errors
    ///
    /// Returns the composition error when a section width is unknown, a count
    /// overflows, or the frame cannot be encoded.
    fn respond(
        &self,
        request_ip: Option<&str>,
    ) -> impl std::future::Future<Output = Result<DbFrame, BootResponseError>>;
}

impl<T> BootResponder for Arc<T>
where
    T: BootResponder + ?Sized,
{
    async fn respond(&self, request_ip: Option<&str>) -> Result<DbFrame, BootResponseError> {
        (**self).respond(request_ip).await
    }
}

/// A per-request setup-reply composer for a live server.
///
/// The immutable `BootResponseAdapter` pattern does not fit the setup reply:
/// the legacy answer is built from the requesting peer's own values, so it is a
/// function of the request rather than of a snapshot.
///
/// An implementation must return [`SetupReplyOutcome::Silent`] for an
/// auth-mode request, because the legacy auth branch writes zero bytes and a
/// zero-length frame would make a peer read a count byte that is not there.
pub trait SetupResponder {
    /// Compose the reply for one validated setup request.
    ///
    /// # Errors
    ///
    /// Returns the composition failure when the requested map list cannot be
    /// held by a 32-slot record.
    fn respond_setup(&self, request: &SetupRequest) -> Result<SetupReplyOutcome, SetupReplyError>;
}

impl<T> SetupResponder for Arc<T>
where
    T: SetupResponder + ?Sized,
{
    fn respond_setup(&self, request: &SetupRequest) -> Result<SetupReplyOutcome, SetupReplyError> {
        (**self).respond_setup(request)
    }
}

/// A setup responder that is explicitly absent.
impl SetupResponder for () {
    fn respond_setup(&self, _request: &SetupRequest) -> Result<SetupReplyOutcome, SetupReplyError> {
        Err(SetupReplyError::Unavailable)
    }
}

/// The default, no-responder marker.
///
/// `DbRequestService` defaults its responder type to `()` so existing callers
/// keep the immutable-snapshot behavior. `()` is never actually consulted:
/// `handle` only calls `respond` on a `Some` responder, and `with_boot_responder`
/// replaces it. This impl exists so the default type parameter satisfies the
/// trait bound, and it fails closed rather than fabricating a frame.
impl BootResponder for () {
    async fn respond(&self, _request_ip: Option<&str>) -> Result<DbFrame, BootResponseError> {
        Err(BootResponseError::Unavailable)
    }
}

/// A transport-facing executor for the currently typed DB requests.
///
/// `DbPeerSession` still owns framing and request validation. This service
/// accepts the resulting outcome, uses the injected boot adapter for boot
/// requests, the injected horse-name lookup for header 132, and an optional
/// channel adapter for header 135. Boot and horse-name responses use the
/// source's zero response handles; the channel result echoes the request handle.
/// A validated remove-affect request is classification-only and remains
/// [`DbServiceOutcome::NotReady`].
#[derive(Debug)]
pub struct DbRequestService<L, C = (), R = (), S = ()> {
    boot: BootResponseAdapter,
    /// The per-request boot seam. When present it replaces the immutable
    /// snapshot adapter, because a live server must consume item-ID ranges per
    /// request.
    boot_responder: Option<R>,
    /// The per-request setup composer. When absent, a validated setup request
    /// stays an honest `NotReady` outcome.
    setup_responder: Option<S>,
    horse_names: L,
    channels: Option<ChannelResponseAdapter<C>>,
}

impl<L> DbRequestService<L, ()> {
    /// Construct a service with caller-owned boot and horse-name providers.
    ///
    /// No channel resolver is installed by this constructor, so a valid
    /// channel request remains an honest `NotReady` outcome.
    pub fn new(boot: BootResponseAdapter, horse_names: L) -> Self {
        Self {
            boot,
            boot_responder: None,
            setup_responder: None,
            horse_names,
            channels: None,
        }
    }

    /// Construct a service with an injected channel resolver.
    pub fn with_channel_resolver<C>(
        boot: BootResponseAdapter,
        horse_names: L,
        channels: C,
    ) -> DbRequestService<L, C, (), ()> {
        DbRequestService {
            boot,
            boot_responder: None,
            setup_responder: None,
            horse_names,
            channels: Some(ChannelResponseAdapter::new(channels)),
        }
    }
}

impl<L, C, S> DbRequestService<L, C, (), S> {
    /// Install a per-request boot composer when a setup composer is already
    /// installed.
    ///
    /// This is the other half of [`Self::with_setup_responder`], so the two
    /// seams can be installed in either order. It is available only while no
    /// boot composer is installed, so a second call stays a compile error
    /// rather than a silent replacement of a live composer.
    #[must_use]
    pub fn with_boot_responder<R>(self, responder: R) -> DbRequestService<L, C, R, S> {
        DbRequestService {
            boot: self.boot,
            boot_responder: Some(responder),
            setup_responder: self.setup_responder,
            horse_names: self.horse_names,
            channels: self.channels,
        }
    }
}

impl<L, C, R> DbRequestService<L, C, R, ()> {
    /// Install a per-request setup-reply composer.
    ///
    /// Available on any service that has no setup composer yet, so it composes
    /// with [`Self::with_boot_responder`] in either order. A second call is a
    /// compile error rather than a silent replacement of a live composer.
    #[must_use]
    pub fn with_setup_responder<S>(self, responder: S) -> DbRequestService<L, C, R, S> {
        DbRequestService {
            boot: self.boot,
            boot_responder: self.boot_responder,
            setup_responder: Some(responder),
            horse_names: self.horse_names,
            channels: self.channels,
        }
    }
}

impl<L, C> DbRequestService<L, C, (), ()> {}

impl<L, C, R, S> DbRequestService<L, C, R, S> {
    /// Return whether a per-request boot composer is installed.
    pub const fn has_boot_responder(&self) -> bool {
        self.boot_responder.is_some()
    }

    /// Return whether a per-request setup composer is installed.
    pub const fn has_setup_responder(&self) -> bool {
        self.setup_responder.is_some()
    }

    /// Borrow the immutable boot response adapter.
    pub const fn boot_adapter(&self) -> &BootResponseAdapter {
        &self.boot
    }

    /// Borrow the optional channel response adapter.
    pub const fn channel_adapter(&self) -> Option<&ChannelResponseAdapter<C>> {
        self.channels.as_ref()
    }
}

impl<L, C, R, SR> DbRequestService<L, C, R, SR>
where
    L: HorseNameLookup,
    C: ChannelResolver,
    R: BootResponder,
    SR: SetupResponder,
{
    /// Execute one dispatch outcome and write at most one response frame.
    ///
    /// A validated boot request is sent to the existing boot adapter. A
    /// validated horse-name request is answered with one
    /// `HEADER_DG_ACK_HORSE_NAME` frame. A validated channel request is
    /// delegated to the optional channel adapter: zero selectors return
    /// `NoResponse`, a valid unresolved request returns one zero-valued
    /// `HEADER_DG_CHANNEL_RESULT` frame, and a match echoes the request
    /// handle. Validated setup, login-by-key, player-load, add-affect, and remove-affect
    /// requests remain `NotReady` with their exact decoded payload lengths:
    /// setup is not allowed to synthesize authentication or state, the primary
    /// response adapters are intentionally not connected until source-verified
    /// stateful lookups exist, and add/remove-affect persistence is not injected.
    /// An already classified `NotReady` outcome is returned unchanged without a
    /// write. Malformed
    /// requests never reach this method because they fail in
    /// [`DbPeerSession`].
    ///
    /// # Errors
    ///
    /// Returns [`DbServiceError::BootResponse`] when the injected boot adapter
    /// is unavailable or cannot encode its payload,
    /// [`DbServiceError::ChannelResponse`] when an injected channel adapter
    /// rejects a request, and [`DbServiceError::Session`] when the response
    /// cannot be written or flushed.
    pub async fn handle<S>(
        &self,
        session: &mut DbPeerSession<S>,
        outcome: &DbDispatchOutcome,
    ) -> Result<DbServiceOutcome, DbServiceError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        match outcome {
            DbDispatchOutcome::Validated(request) => match request {
                boot @ DbRequest::Boot { request, .. } => {
                    // A per-request composer wins when installed, because a
                    // live server must consume two item-ID ranges per call and
                    // filters the administrator list on the requesting address.
                    let request_ip = request.ip_text();
                    let frame = match &self.boot_responder {
                        Some(responder) => responder
                            .respond(request_ip.as_deref())
                            .await
                            .map_err(DbServiceError::BootResponse)?,
                        None => self
                            .boot
                            .response_for(boot)
                            .map_err(DbServiceError::BootResponse)?,
                    };
                    Self::write_frame(session, &frame).await
                }
                DbRequest::HorseName { request, .. } => {
                    let name = self
                        .horse_names
                        .lookup(request.player_id)
                        .unwrap_or([0_u8; LEGACY_CHARACTER_NAME_BYTES]);
                    let record = HorseNameRecord::new(request.player_id, name);
                    let frame = DbFrame::new(HEADER_DG_ACK_HORSE_NAME, 0, record.encode());
                    Self::write_frame(session, &frame).await
                }
                DbRequest::FindChannel {
                    handle,
                    request: channel_request,
                } => {
                    if !channel_request.is_valid() {
                        return Ok(DbServiceOutcome::NoResponse {
                            handle: *handle,
                            header: HEADER_GD_FIND_CHANNEL,
                        });
                    }
                    let Some(adapter) = self.channels.as_ref() else {
                        return Ok(DbServiceOutcome::NotReady {
                            handle: *handle,
                            header: HEADER_GD_FIND_CHANNEL,
                            payload_len: ChannelChangeRequest::WIRE_SIZE,
                        });
                    };
                    let frame = adapter
                        .response_for(request)
                        .map_err(DbServiceError::ChannelResponse)?
                        .ok_or(DbServiceError::ChannelResponse(
                            ChannelResponseError::Unavailable,
                        ))?;
                    Self::write_frame(session, &frame).await
                }
                DbRequest::LoginByKey { handle, .. } => Ok(DbServiceOutcome::NotReady {
                    handle: *handle,
                    header: HEADER_GD_LOGIN_BY_KEY,
                    payload_len: LoginByKeyRequest::WIRE_SIZE,
                }),
                DbRequest::Setup { handle, request } => {
                    self.respond_to_setup(session, *handle, request).await
                }
                DbRequest::PlayerLoad { handle, .. } => Ok(DbServiceOutcome::NotReady {
                    handle: *handle,
                    header: HEADER_GD_PLAYER_LOAD,
                    payload_len: PlayerLoadRequest::WIRE_SIZE,
                }),
                DbRequest::AddAffect { handle, .. } => Ok(DbServiceOutcome::NotReady {
                    handle: *handle,
                    header: HEADER_GD_ADD_AFFECT,
                    payload_len: AddAffectRequest::WIRE_SIZE,
                }),
                DbRequest::RemoveAffect { handle, .. } => Ok(DbServiceOutcome::NotReady {
                    handle: *handle,
                    header: HEADER_GD_REMOVE_AFFECT,
                    payload_len: RemoveAffectRequest::WIRE_SIZE,
                }),
            },
            DbDispatchOutcome::NotReady {
                handle,
                header,
                payload_len,
            } => Ok(DbServiceOutcome::NotReady {
                handle: *handle,
                header: *header,
                payload_len: *payload_len,
            }),
        }
    }

    /// Answer one validated setup request.
    ///
    /// Split out of [`Self::handle`] so the boot, channel, and setup paths can
    /// each be read on their own. The auth branch writes nothing, so it is
    /// reported as `NoResponse` rather than `Responded`: claiming a frame that
    /// does not exist would make a silent legacy path look like a reply.
    async fn respond_to_setup<S>(
        &self,
        session: &mut DbPeerSession<S>,
        handle: u32,
        request: &SetupRequest,
    ) -> Result<DbServiceOutcome, DbServiceError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let Some(responder) = self.setup_responder.as_ref() else {
            return Ok(DbServiceOutcome::NotReady {
                handle,
                header: crate::setup::HEADER_GD_SETUP,
                payload_len: request.wire_len(),
            });
        };
        match responder
            .respond_setup(request)
            .map_err(DbServiceError::SetupReply)?
        {
            SetupReplyOutcome::Frame(frame) => {
                let (header, frame_handle) = (frame.header, frame.handle);
                Self::write_frame(session, &frame).await?;
                Ok(DbServiceOutcome::Responded {
                    header,
                    handle: frame_handle,
                })
            }
            SetupReplyOutcome::Silent => Ok(DbServiceOutcome::NoResponse {
                handle,
                header: crate::setup::HEADER_GD_SETUP,
            }),
        }
    }

    async fn write_frame<S>(
        session: &mut DbPeerSession<S>,
        frame: &DbFrame,
    ) -> Result<DbServiceOutcome, DbServiceError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        session
            .write_response(frame)
            .await
            .map_err(DbServiceError::Session)?;
        session.flush().await.map_err(DbServiceError::Session)?;
        Ok(DbServiceOutcome::Responded {
            header: frame.header,
            handle: frame.handle,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use net::db_transport::DbFrameTransport;
    use protocol::db_boot::{
        parse_db_boot_payload, BootFeatureProfile, DbBootRequest, HEADER_DG_BOOT, HEADER_GD_BOOT,
    };
    use protocol::db_map_locations::{HEADER_DG_MAP_LOCATIONS, MAP_LOCATION_WIRE_SIZE};
    use protocol::db_records::{
        AffectElementRecord, ChannelChangeRequest, ChannelResultRecord, HorseNameRequest,
        HEADER_DG_CHANNEL_RESULT, HEADER_GD_FIND_CHANNEL, HEADER_GD_REQ_HORSE_NAME,
    };
    use tokio::io::{duplex, AsyncWriteExt};

    use crate::setup::{
        decode_setup_request, SetupDecodeLimits, SetupFeatureProfile, LOGIN_ON_SETUP_WIRE_SIZE,
        SETUP_BASE_WIRE_SIZE,
    };

    fn minimal_boot_payload() -> Vec<u8> {
        let profile = BootFeatureProfile::minimal();
        let mut body = vec![6_u8];
        for _ in profile.section_kinds() {
            body.extend_from_slice(&1_u16.to_le_bytes());
            body.extend_from_slice(&0_u16.to_le_bytes());
        }
        body.extend_from_slice(&0_i32.to_le_bytes());
        body.extend_from_slice(&12_u16.to_le_bytes());
        body.extend_from_slice(&1_u16.to_le_bytes());
        body.extend_from_slice(&[0_u8; 24]);
        body.extend_from_slice(&16_u16.to_le_bytes());
        body.extend_from_slice(&0_u16.to_le_bytes());
        body.extend_from_slice(&104_u16.to_le_bytes());
        body.extend_from_slice(&0_u16.to_le_bytes());
        body.extend_from_slice(&304_u16.to_le_bytes());
        body.extend_from_slice(&1_u16.to_le_bytes());
        body.extend_from_slice(&[0_u8; 304]);
        body.extend_from_slice(&68_u16.to_le_bytes());
        body.extend_from_slice(&0_u16.to_le_bytes());
        body.extend_from_slice(&0xffff_u16.to_le_bytes());

        let mut payload = Vec::with_capacity(body.len() + 4);
        payload.extend_from_slice(&u32::try_from(body.len() + 4).unwrap().to_le_bytes());
        payload.extend_from_slice(&body);
        payload
    }

    fn boot_request() -> DbBootRequest {
        let mut ip = [0_u8; 16];
        ip[..9].copy_from_slice(b"127.0.0.1");
        DbBootRequest::new([10, 20], ip)
    }

    struct FixtureLookup;

    impl HorseNameLookup for FixtureLookup {
        fn lookup(&self, player_id: u32) -> Option<[u8; LEGACY_CHARACTER_NAME_BYTES]> {
            if player_id == 0x1020_3040 {
                let mut name = [0_u8; LEGACY_CHARACTER_NAME_BYTES];
                name[..9].copy_from_slice(b"windhorse");
                name[9..17].copy_from_slice(&[0xa1, 0xb2, 0xc3, 0xd4, 0xe5, 0xf6, 0x07, 0x18]);
                Some(name)
            } else {
                None
            }
        }
    }

    struct FixtureChannels;

    impl ChannelResolver for FixtureChannels {
        fn resolve(&self, map_index: i32, channel: i32) -> Option<ChannelEndpoint> {
            (map_index == 1 && channel == 2).then(|| ChannelEndpoint::new(0x0100_007f, 0x1234))
        }
    }

    fn boot_outcome(handle: u32) -> DbDispatchOutcome {
        DbDispatchOutcome::Validated(DbRequest::Boot {
            handle,
            request: boot_request(),
        })
    }

    fn horse_outcome(handle: u32, player_id: u32) -> DbDispatchOutcome {
        DbDispatchOutcome::Validated(DbRequest::HorseName {
            handle,
            request: HorseNameRequest::new(player_id),
        })
    }

    fn channel_outcome(handle: u32, map_index: i32, channel: i32) -> DbDispatchOutcome {
        DbDispatchOutcome::Validated(DbRequest::FindChannel {
            handle,
            request: ChannelChangeRequest::new(map_index, channel),
        })
    }

    fn player_outcome(handle: u32) -> DbDispatchOutcome {
        DbDispatchOutcome::Validated(DbRequest::PlayerLoad {
            handle,
            request: PlayerLoadRequest {
                account_id: 11,
                player_id: 22,
                account_index: 2,
            },
        })
    }

    fn add_affect_outcome(handle: u32) -> DbDispatchOutcome {
        DbDispatchOutcome::Validated(DbRequest::AddAffect {
            handle,
            request: AddAffectRequest::new(1, AffectElementRecord::new(2, 3, 4, 5, 6, 7)),
        })
    }

    fn remove_affect_outcome(handle: u32) -> DbDispatchOutcome {
        DbDispatchOutcome::Validated(DbRequest::RemoveAffect {
            handle,
            request: RemoveAffectRequest::new(1, 2, 3),
        })
    }

    fn login_outcome(handle: u32) -> DbDispatchOutcome {
        let mut login = [0_u8; 31];
        login[..5].copy_from_slice(b"alice");
        DbDispatchOutcome::Validated(DbRequest::LoginByKey {
            handle,
            request: LoginByKeyRequest {
                login,
                login_key: 7,
                client_key: [1, 2, 3, 4],
                ip: [0; 16],
            },
        })
    }

    fn setup_outcome(handle: u32, login_count: u32) -> DbDispatchOutcome {
        let mut payload =
            vec![0_u8; SETUP_BASE_WIRE_SIZE + login_count as usize * LOGIN_ON_SETUP_WIRE_SIZE];
        payload[149..153].copy_from_slice(&login_count.to_le_bytes());
        let request = decode_setup_request(
            &payload,
            SetupFeatureProfile::ActiveX86,
            SetupDecodeLimits::new(login_count as usize),
        )
        .expect("setup fixture is valid");
        DbDispatchOutcome::Validated(DbRequest::Setup { handle, request })
    }

    #[test]
    fn channel_adapter_distinguishes_invalid_zero_no_match_and_match() {
        let adapter = ChannelResponseAdapter::new(FixtureChannels);
        let invalid = DbRequest::FindChannel {
            handle: 7,
            request: ChannelChangeRequest::new(0, 2),
        };
        assert_eq!(adapter.response_for(&invalid), Ok(None));

        let no_match = DbRequest::FindChannel {
            handle: 8,
            request: ChannelChangeRequest::new(9, 9),
        };
        let no_match_frame = adapter.response_for(&no_match).unwrap().unwrap();
        assert_eq!(no_match_frame.header, HEADER_DG_CHANNEL_RESULT);
        assert_eq!(no_match_frame.handle, 8);
        assert_eq!(
            no_match_frame.payload,
            ChannelResultRecord::missing().encode()
        );

        let matched = DbRequest::FindChannel {
            handle: 9,
            request: ChannelChangeRequest::new(1, 2),
        };
        let matched_frame = adapter.response_for(&matched).unwrap().unwrap();
        assert_eq!(matched_frame.header, HEADER_DG_CHANNEL_RESULT);
        assert_eq!(matched_frame.handle, 9);
        assert_eq!(
            matched_frame.payload,
            vec![0x7f, 0x00, 0x00, 0x01, 0x34, 0x12]
        );

        let unavailable: ChannelResponseAdapter<FixtureChannels> =
            ChannelResponseAdapter::unavailable();
        assert_eq!(
            unavailable.response_for(&matched),
            Err(ChannelResponseError::Unavailable)
        );
        assert_eq!(
            adapter.response_for(&DbRequest::HorseName {
                handle: 10,
                request: HorseNameRequest::new(1),
            }),
            Err(ChannelResponseError::NotChannelRequest)
        );
    }

    #[tokio::test]
    async fn login_by_key_service_remains_not_ready_and_writes_nothing() {
        let service = DbRequestService::new(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
        );
        let (client, server) = duplex(64);
        let mut session = DbPeerSession::new(server);
        assert_eq!(
            service
                .handle(&mut session, &login_outcome(0xfeed_beef))
                .await
                .unwrap(),
            DbServiceOutcome::NotReady {
                handle: 0xfeed_beef,
                header: HEADER_GD_LOGIN_BY_KEY,
                payload_len: LoginByKeyRequest::WIRE_SIZE,
            }
        );
        drop(session);

        let mut client = DbFrameTransport::new(client);
        assert_eq!(client.read_frame().await.unwrap(), None);
    }

    #[tokio::test]
    async fn setup_service_remains_not_ready_and_writes_nothing() {
        let service = DbRequestService::new(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
        );
        let (client, server) = duplex(64);
        let mut session = DbPeerSession::new(server);
        assert_eq!(
            service
                .handle(&mut session, &setup_outcome(0xfeed_beef, 1))
                .await
                .unwrap(),
            DbServiceOutcome::NotReady {
                handle: 0xfeed_beef,
                header: crate::setup::HEADER_GD_SETUP,
                payload_len: SETUP_BASE_WIRE_SIZE + LOGIN_ON_SETUP_WIRE_SIZE,
            }
        );
        drop(session);

        let mut client = DbFrameTransport::new(client);
        assert_eq!(client.read_frame().await.unwrap(), None);
    }

    /// A boot composer that returns a fixed sentinel frame.
    ///
    /// Presence is all this test observes, so the payload is irrelevant.
    struct FixtureBoot;

    impl BootResponder for FixtureBoot {
        async fn respond(&self, _request_ip: Option<&str>) -> Result<DbFrame, BootResponseError> {
            Ok(DbFrame::new(HEADER_DG_BOOT, 0, vec![0x5a]))
        }
    }

    /// A setup composer that always answers with one known frame.
    ///
    /// It exists so the dispatch path can be tested without the production
    /// composer, which would make a failure ambiguous between the seam and the
    /// composition rules.
    struct FixtureSetupFrame;

    impl SetupResponder for FixtureSetupFrame {
        fn respond_setup(
            &self,
            _request: &SetupRequest,
        ) -> Result<SetupReplyOutcome, SetupReplyError> {
            // The frame header is not payload: the payload is the count byte
            // followed by the one record.
            let mut payload = vec![1_u8];
            payload.extend_from_slice(&[0x5a_u8; MAP_LOCATION_WIRE_SIZE]);
            Ok(SetupReplyOutcome::Frame(DbFrame::new(
                HEADER_DG_MAP_LOCATIONS,
                0,
                payload,
            )))
        }
    }

    /// A setup composer that reproduces the silent auth branch.
    struct FixtureSetupSilent;

    impl SetupResponder for FixtureSetupSilent {
        fn respond_setup(
            &self,
            _request: &SetupRequest,
        ) -> Result<SetupReplyOutcome, SetupReplyError> {
            Ok(SetupReplyOutcome::Silent)
        }
    }

    #[test]
    fn the_installed_seams_report_presence_in_either_order() {
        let base = DbRequestService::new(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
        );
        assert!(!base.has_boot_responder());
        assert!(!base.has_setup_responder());

        let boot_then_setup = DbRequestService::new(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
        )
        .with_boot_responder(FixtureBoot)
        .with_setup_responder(FixtureSetupFrame);
        assert!(boot_then_setup.has_boot_responder());
        assert!(boot_then_setup.has_setup_responder());

        let setup_then_boot = base
            .with_setup_responder(FixtureSetupFrame)
            .with_boot_responder(FixtureBoot);
        assert!(setup_then_boot.has_boot_responder());
        assert!(setup_then_boot.has_setup_responder());
    }

    #[tokio::test]
    async fn an_installed_setup_composer_writes_one_frame_and_reports_its_own_handle() {
        let service = DbRequestService::new(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
        )
        .with_setup_responder(FixtureSetupFrame);
        let (client, server) = duplex(256);
        let mut session = DbPeerSession::new(server);
        assert_eq!(
            service
                .handle(&mut session, &setup_outcome(0xfeed_beef, 1))
                .await
                .unwrap(),
            DbServiceOutcome::Responded {
                header: HEADER_DG_MAP_LOCATIONS,
                // The legacy setup reply never echoes the incoming handle.
                handle: 0,
            }
        );
        drop(session);

        let mut client = DbFrameTransport::new(client);
        let frame = client
            .read_frame()
            .await
            .unwrap()
            .expect("one frame is written");
        assert_eq!(frame.header, HEADER_DG_MAP_LOCATIONS);
        assert_eq!(frame.handle, 0);
        assert_eq!(frame.payload[0], 1, "one own-location record is declared");
        assert_eq!(
            frame.payload.len(),
            1 + MAP_LOCATION_WIRE_SIZE,
            "the count byte and one 146-byte record are the whole payload"
        );
        assert!(client.read_frame().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_silent_setup_composer_reports_no_response_and_writes_nothing() {
        let service = DbRequestService::new(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
        )
        .with_setup_responder(FixtureSetupSilent);
        let (client, server) = duplex(64);
        let mut session = DbPeerSession::new(server);
        assert_eq!(
            service
                .handle(&mut session, &setup_outcome(0xfeed_beef, 1))
                .await
                .unwrap(),
            DbServiceOutcome::NoResponse {
                handle: 0xfeed_beef,
                header: crate::setup::HEADER_GD_SETUP,
            }
        );
        drop(session);

        let mut client = DbFrameTransport::new(client);
        assert_eq!(client.read_frame().await.unwrap(), None);
    }

    #[tokio::test]
    async fn player_load_service_remains_not_ready_and_writes_nothing() {
        let service = DbRequestService::new(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
        );
        let (client, server) = duplex(64);
        let mut session = DbPeerSession::new(server);
        assert_eq!(
            service
                .handle(&mut session, &player_outcome(0xfeed_beef))
                .await
                .unwrap(),
            DbServiceOutcome::NotReady {
                handle: 0xfeed_beef,
                header: HEADER_GD_PLAYER_LOAD,
                payload_len: PlayerLoadRequest::WIRE_SIZE,
            }
        );
        drop(session);

        let mut client = DbFrameTransport::new(client);
        assert_eq!(client.read_frame().await.unwrap(), None);
    }

    #[tokio::test]
    async fn add_affect_service_remains_not_ready_and_writes_nothing() {
        let service = DbRequestService::new(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
        );
        let (client, server) = duplex(64);
        let mut session = DbPeerSession::new(server);
        assert_eq!(
            service
                .handle(&mut session, &add_affect_outcome(0xfeed_beef))
                .await
                .unwrap(),
            DbServiceOutcome::NotReady {
                handle: 0xfeed_beef,
                header: HEADER_GD_ADD_AFFECT,
                payload_len: AddAffectRequest::WIRE_SIZE,
            }
        );
        drop(session);

        let mut client = DbFrameTransport::new(client);
        assert_eq!(client.read_frame().await.unwrap(), None);
    }

    #[tokio::test]
    async fn remove_affect_service_remains_not_ready_and_writes_nothing() {
        let service = DbRequestService::new(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
        );
        let (client, server) = duplex(64);
        let mut session = DbPeerSession::new(server);
        assert_eq!(
            service
                .handle(&mut session, &remove_affect_outcome(0xfeed_beef))
                .await
                .unwrap(),
            DbServiceOutcome::NotReady {
                handle: 0xfeed_beef,
                header: HEADER_GD_REMOVE_AFFECT,
                payload_len: RemoveAffectRequest::WIRE_SIZE,
            }
        );
        drop(session);

        let mut client = DbFrameTransport::new(client);
        assert_eq!(client.read_frame().await.unwrap(), None);
    }

    #[tokio::test]
    async fn channel_service_writes_an_echoed_result_after_fragmented_input() {
        let service = DbRequestService::with_channel_resolver(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
            FixtureChannels,
        );
        let request_wire = DbFrame::new(
            HEADER_GD_FIND_CHANNEL,
            0x1234_5678,
            ChannelChangeRequest::new(1, 2).encode(),
        )
        .encode()
        .unwrap();
        let (client, server) = duplex(128);
        let (client_reader, mut client_writer) = tokio::io::split(client);
        let feed = tokio::spawn(async move {
            for byte in request_wire {
                client_writer.write_all(&[byte]).await.unwrap();
            }
        });

        let mut session = DbPeerSession::new(server);
        let request = session.read_request().await.unwrap().unwrap();
        assert_eq!(
            service.handle(&mut session, &request).await.unwrap(),
            DbServiceOutcome::Responded {
                header: HEADER_DG_CHANNEL_RESULT,
                handle: 0x1234_5678,
            }
        );
        feed.await.unwrap();
        drop(session);

        let mut client = DbFrameTransport::new(client_reader);
        let response = client.read_frame().await.unwrap().unwrap();
        assert_eq!(response.header, HEADER_DG_CHANNEL_RESULT);
        assert_eq!(response.handle, 0x1234_5678);
        assert_eq!(response.payload, vec![0x7f, 0x00, 0x00, 0x01, 0x34, 0x12]);
        assert!(client.read_frame().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn channel_service_keeps_invalid_and_unavailable_requests_silent() {
        let service = DbRequestService::with_channel_resolver(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
            FixtureChannels,
        );
        let (client, server) = duplex(64);
        let (client_reader, client_writer) = tokio::io::split(client);
        let mut session = DbPeerSession::new(server);
        assert_eq!(
            service
                .handle(&mut session, &channel_outcome(11, 0, 2))
                .await
                .unwrap(),
            DbServiceOutcome::NoResponse {
                handle: 11,
                header: HEADER_GD_FIND_CHANNEL,
            }
        );
        drop(session);
        drop(client_writer);
        let mut client = DbFrameTransport::new(client_reader);
        assert!(client.read_frame().await.unwrap().is_none());

        let service = DbRequestService::new(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
        );
        let (client, server) = duplex(64);
        let (client_reader, client_writer) = tokio::io::split(client);
        let mut session = DbPeerSession::new(server);
        assert_eq!(
            service
                .handle(&mut session, &channel_outcome(12, 1, 2))
                .await
                .unwrap(),
            DbServiceOutcome::NotReady {
                handle: 12,
                header: HEADER_GD_FIND_CHANNEL,
                payload_len: 8,
            }
        );
        drop(session);
        drop(client_writer);
        let mut client = DbFrameTransport::new(client_reader);
        assert!(client.read_frame().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn ready_service_composes_boot_and_horse_responses_with_zero_handles() {
        let profile = BootFeatureProfile::minimal();
        let boot_payload = minimal_boot_payload();
        let service = DbRequestService::new(
            BootResponseAdapter::new(
                parse_db_boot_payload(&boot_payload, profile).unwrap(),
                profile,
            ),
            FixtureLookup,
        );

        let mut input = DbFrame::new(
            HEADER_GD_BOOT,
            0x1234_5678,
            boot_request().encode().to_vec(),
        )
        .encode()
        .unwrap();
        input.extend_from_slice(
            &DbFrame::new(
                HEADER_GD_REQ_HORSE_NAME,
                0x89ab_cdef,
                HorseNameRequest::new(0x1020_3040).encode(),
            )
            .encode()
            .unwrap(),
        );
        let (client, server) = duplex(4096);
        let (client_reader, mut client_writer) = tokio::io::split(client);
        let feed = tokio::spawn(async move {
            client_writer.write_all(&input).await.unwrap();
        });

        let mut session = DbPeerSession::new(server);
        let boot = session.read_request().await.unwrap().unwrap();
        assert_eq!(
            service.handle(&mut session, &boot).await.unwrap(),
            DbServiceOutcome::Responded {
                header: HEADER_DG_BOOT,
                handle: 0,
            }
        );
        let horse = session.read_request().await.unwrap().unwrap();
        assert_eq!(
            service.handle(&mut session, &horse).await.unwrap(),
            DbServiceOutcome::Responded {
                header: HEADER_DG_ACK_HORSE_NAME,
                handle: 0,
            }
        );
        feed.await.unwrap();
        drop(session);

        let mut client = DbFrameTransport::new(client_reader);
        let boot_frame = client.read_frame().await.unwrap().unwrap();
        assert_eq!(boot_frame.header, HEADER_DG_BOOT);
        assert_eq!(boot_frame.handle, 0);
        assert_eq!(boot_frame.payload, boot_payload);

        let horse_frame = client.read_frame().await.unwrap().unwrap();
        assert_eq!(horse_frame.header, HEADER_DG_ACK_HORSE_NAME);
        assert_eq!(horse_frame.handle, 0);
        let record = HorseNameRecord::decode(&horse_frame.payload).unwrap();
        assert_eq!(record.player_id, 0x1020_3040);
        assert_eq!(&record.name[..9], b"windhorse");
        assert_eq!(
            &record.name[9..17],
            &[0xa1, 0xb2, 0xc3, 0xd4, 0xe5, 0xf6, 0x07, 0x18]
        );
        assert!(client.read_frame().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn missing_horse_name_uses_the_legacy_zero_filled_record() {
        let service = DbRequestService::new(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
        );
        let (client, server) = duplex(64);
        let (client_reader, client_writer) = tokio::io::split(client);
        let mut session = DbPeerSession::new(server);

        assert_eq!(
            service
                .handle(&mut session, &horse_outcome(77, 99))
                .await
                .unwrap(),
            DbServiceOutcome::Responded {
                header: HEADER_DG_ACK_HORSE_NAME,
                handle: 0,
            }
        );
        drop(session);
        drop(client_writer);

        let mut client = DbFrameTransport::new(client_reader);
        let frame = client.read_frame().await.unwrap().unwrap();
        assert_eq!(frame.header, HEADER_DG_ACK_HORSE_NAME);
        assert_eq!(frame.handle, 0);
        assert_eq!(frame.payload, HorseNameRecord::missing(99).encode());
        assert!(client.read_frame().await.unwrap().is_none());
    }

    /// A boot composer that records the address the service handed it.
    ///
    /// Legacy filters the administrator list on the requesting peer's `szIP`
    /// inside the boot handler, so that address is per request. This seam makes
    /// the propagation observable without a database.
    #[derive(Default)]
    struct RecordingBoot {
        seen: std::sync::Mutex<Vec<Option<String>>>,
    }

    impl BootResponder for RecordingBoot {
        async fn respond(&self, request_ip: Option<&str>) -> Result<DbFrame, BootResponseError> {
            self.seen
                .lock()
                .expect("the recording lock is not poisoned")
                .push(request_ip.map(str::to_owned));
            Ok(DbFrame::new(HEADER_DG_BOOT, 0, vec![0x5a]))
        }
    }

    /// Build a boot outcome whose `szIP` holds the given NUL-terminated text.
    fn boot_outcome_from_ip(handle: u32, text: &[u8]) -> DbDispatchOutcome {
        let mut ip = [0_u8; 16];
        ip[..text.len()].copy_from_slice(text);
        DbDispatchOutcome::Validated(DbRequest::Boot {
            handle,
            request: DbBootRequest::new([10, 20], ip),
        })
    }

    /// The decoded `szIP` must reach the composer for each request.
    ///
    /// The administrator half of the boot is filtered on this address, so
    /// dropping it would silently serve every peer the same GM list. The two
    /// requests carry different addresses and must be recorded separately: one
    /// shared address stored on the service would pass a single-boot test.
    #[tokio::test]
    async fn each_boot_request_passes_its_own_address_to_the_composer() {
        let boot = std::sync::Arc::new(RecordingBoot::default());
        let service = DbRequestService::new(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
        )
        .with_boot_responder(std::sync::Arc::clone(&boot));

        for (handle, address) in [(1_u32, &b"10.0.0.1"[..]), (2, b"10.0.0.2")] {
            let (client, server) = duplex(64);
            let (_client_reader, client_writer) = tokio::io::split(client);
            let mut session = DbPeerSession::new(server);
            service
                .handle(&mut session, &boot_outcome_from_ip(handle, address))
                .await
                .expect("an installed composer answers");
            drop(session);
            drop(client_writer);
        }

        let seen = boot.seen.lock().expect("the lock is not poisoned").clone();
        assert_eq!(
            seen,
            vec![Some("10.0.0.1".to_owned()), Some("10.0.0.2".to_owned())],
            "each request must carry its own decoded address"
        );
    }

    /// A `szIP` with no NUL is not an address.
    ///
    /// Legacy interpolates this field into an unescaped `'%s'`, so a field that
    /// was never terminated is a malformed request. Passing the raw sixteen
    /// bytes through would both filter on garbage and let those bytes reach the
    /// statement, so the service must pass no address at all and let the
    /// composer use the `ALL` literal.
    #[tokio::test]
    async fn an_unterminated_address_passes_nothing() {
        let boot = std::sync::Arc::new(RecordingBoot::default());
        let service = DbRequestService::new(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
        )
        .with_boot_responder(std::sync::Arc::clone(&boot));

        // Sixteen non-NUL bytes: no terminator anywhere in the field.
        let outcome = boot_outcome_from_ip(3, &[b'x'; 16]);
        let (client, server) = duplex(64);
        let (_client_reader, client_writer) = tokio::io::split(client);
        let mut session = DbPeerSession::new(server);
        service
            .handle(&mut session, &outcome)
            .await
            .expect("an installed composer answers");
        drop(session);
        drop(client_writer);

        let seen = boot.seen.lock().expect("the lock is not poisoned").clone();
        assert_eq!(
            seen,
            vec![None],
            "an unterminated szIP must not become a sixteen-byte address"
        );
    }

    #[tokio::test]
    async fn unavailable_boot_service_writes_no_frame() {
        let service = DbRequestService::new(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
        );
        let (client, server) = duplex(64);
        let (client_reader, client_writer) = tokio::io::split(client);
        let mut session = DbPeerSession::new(server);

        let error = service
            .handle(&mut session, &boot_outcome(8))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            DbServiceError::BootResponse(BootResponseError::Unavailable)
        ));
        drop(session);
        drop(client_writer);

        let mut client = DbFrameTransport::new(client_reader);
        assert!(client.read_frame().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn not_ready_service_result_writes_no_frame() {
        let service = DbRequestService::new(
            BootResponseAdapter::unavailable(BootFeatureProfile::minimal()),
            FixtureLookup,
        );
        let (client, server) = duplex(64);
        let (client_reader, client_writer) = tokio::io::split(client);
        let mut session = DbPeerSession::new(server);
        let outcome = DbDispatchOutcome::NotReady {
            handle: 9,
            header: 42,
            payload_len: 3,
        };

        assert_eq!(
            service.handle(&mut session, &outcome).await.unwrap(),
            DbServiceOutcome::NotReady {
                handle: 9,
                header: 42,
                payload_len: 3,
            }
        );
        drop(session);
        drop(client_writer);

        let mut client = DbFrameTransport::new(client_reader);
        assert!(client.read_frame().await.unwrap().is_none());
    }
}
