//! Explicit, caller-owned boot response composition.
//!
//! This adapter is a narrow boundary between a validated DB-peer boot
//! request and [`protocol::db_boot::DbBootPayload::encode_frame_with_limit`].
//! It does not query `MySQL`, load tables, choose a feature profile, or create a
//! production snapshot. A caller must inject an already validated payload and
//! an explicit profile. The normal DB binary can continue to report that it
//! is unavailable until a real loader supplies one.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use protocol::db_boot::{
    BootFeatureProfile, DbBootError, DbBootPayload, DEFAULT_MAX_DB_BOOT_PAYLOAD_SIZE,
};
use protocol::db_wire::DbFrame;

use crate::session::DbRequest;

/// A failure while composing a boot response frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootResponseError {
    /// No caller-supplied boot snapshot is available.
    Unavailable,
    /// The supplied snapshot could not be encoded as a valid boot frame.
    Encode(DbBootError),
    /// The request was not the legacy boot request.
    NotBootRequest,
    /// A per-request boot composer could not build its payload.
    ///
    /// This is distinct from [`Self::Encode`]: the payload was never produced,
    /// so there is nothing to encode. It happens when a table section width is
    /// unknown for the selected profile or a declared count overflows, and it
    /// is never used to report a load failure, because this server does not
    /// load tables yet.
    Compose {
        /// The composition failure, rendered for the log.
        message: String,
    },
}

impl fmt::Display for BootResponseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("no boot snapshot is available"),
            Self::Encode(source) => write!(formatter, "boot response encoding failed: {source}"),
            Self::NotBootRequest => formatter.write_str("request is not a DB boot request"),
            Self::Compose { message } => {
                write!(formatter, "boot response composition failed: {message}")
            }
        }
    }
}

impl Error for BootResponseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Unavailable | Self::NotBootRequest | Self::Compose { .. } => None,
            Self::Encode(source) => Some(source),
        }
    }
}

/// An immutable boot snapshot and its explicit wire profile.
///
/// The adapter is safe to share between connection tasks. It never exposes a
/// mutable snapshot and never chooses a profile from payload bytes. The
/// request handle is intentionally not copied into the response: the legacy
/// `QUERY_BOOT` response uses the unsolicited `HEADER_DG_BOOT`/zero-handle
/// frame, regardless of the incoming peer handle.
#[derive(Debug, Clone)]
pub struct BootResponseAdapter {
    profile: BootFeatureProfile,
    snapshot: Option<Arc<DbBootPayload>>,
    max_payload_size: usize,
}

impl BootResponseAdapter {
    /// Create an adapter for a caller-supplied snapshot and profile.
    #[must_use]
    pub fn new(snapshot: DbBootPayload, profile: BootFeatureProfile) -> Self {
        Self::with_max_payload_size(snapshot, profile, DEFAULT_MAX_DB_BOOT_PAYLOAD_SIZE)
    }

    /// Create an adapter with an explicit complete-payload allocation limit.
    #[must_use]
    pub fn with_max_payload_size(
        snapshot: DbBootPayload,
        profile: BootFeatureProfile,
        max_payload_size: usize,
    ) -> Self {
        Self {
            profile,
            snapshot: Some(Arc::new(snapshot)),
            max_payload_size,
        }
    }

    /// Create an adapter that reports that no snapshot has been loaded.
    ///
    /// This is useful for the production binary's current not-ready state; it
    /// does not create an empty or synthetic boot response.
    #[must_use]
    pub fn unavailable(profile: BootFeatureProfile) -> Self {
        Self {
            profile,
            snapshot: None,
            max_payload_size: DEFAULT_MAX_DB_BOOT_PAYLOAD_SIZE,
        }
    }

    /// Return the explicitly selected feature profile.
    #[must_use]
    pub const fn profile(&self) -> BootFeatureProfile {
        self.profile
    }

    /// Return the configured complete-payload allocation limit.
    #[must_use]
    pub const fn max_payload_size(&self) -> usize {
        self.max_payload_size
    }

    /// Return whether a caller-supplied snapshot is present.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.snapshot.is_some()
    }

    /// Compose a response for a validated boot request.
    ///
    /// The request payload is intentionally not inspected. Request-specific IP
    /// policy, SQL lookup, host/admin data, and table loading belong to the
    /// future data-source service. This method only turns the injected snapshot
    /// into the source-fixed zero-handle boot frame.
    ///
    /// # Errors
    ///
    /// Returns [`BootResponseError::Unavailable`] when no snapshot was
    /// injected, [`BootResponseError::NotBootRequest`] for another typed
    /// request (including validation-only add-affect and remove-affect requests), or
    /// [`BootResponseError::Encode`] when the payload/profile combination is
    /// invalid or exceeds the configured limit.
    pub fn response_for(&self, request: &DbRequest) -> Result<DbFrame, BootResponseError> {
        match request {
            DbRequest::Boot { .. } => self.response(),
            DbRequest::Setup { .. }
            | DbRequest::HorseName { .. }
            | DbRequest::FindChannel { .. }
            | DbRequest::LoginByKey { .. }
            | DbRequest::PlayerLoad { .. }
            | DbRequest::AddAffect { .. }
            | DbRequest::RemoveAffect { .. } => Err(BootResponseError::NotBootRequest),
        }
    }

    /// Compose a response without a request object.
    ///
    /// This is the same operation used by [`Self::response_for`] and is useful
    /// when a caller has already established that the request was a validated
    /// boot request.
    ///
    /// # Errors
    ///
    /// Returns [`BootResponseError::Unavailable`] or
    /// [`BootResponseError::Encode`].
    pub fn response(&self) -> Result<DbFrame, BootResponseError> {
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or(BootResponseError::Unavailable)?;
        snapshot
            .encode_frame_with_limit(self.profile, self.max_payload_size)
            .map_err(BootResponseError::Encode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use net::db_transport::DbFrameTransport;
    use protocol::db_boot::{
        parse_db_boot_payload, BootSectionKind, DbBootRequest, ADMIN_INFO_WIRE_SIZE,
        DB_BOOT_END_MARKER, DB_BOOT_VERSION, GM_HOST_WIRE_SIZE, HEADER_GD_BOOT,
        ITEM_ID_RANGE_WIRE_SIZE, MONARCH_CANDIDACY_WIRE_SIZE, MONARCH_INFO_WIRE_SIZE,
    };
    use protocol::db_records::{ITEM_TABLE_RECORD_WIRE_SIZE, MOB_TABLE_RECORD_WIRE_SIZE};
    use tokio::io::{duplex, AsyncWriteExt};

    fn c_array<const N: usize>(value: &str) -> [u8; N] {
        let mut bytes = [0; N];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        bytes
    }

    fn minimal_fixture() -> DbBootPayload {
        let profile = BootFeatureProfile::minimal();
        let mut body = vec![DB_BOOT_VERSION];
        for kind in profile.section_kinds() {
            let record_size = match kind {
                BootSectionKind::Mob => u16::try_from(MOB_TABLE_RECORD_WIRE_SIZE).unwrap(),
                BootSectionKind::Item => u16::try_from(ITEM_TABLE_RECORD_WIRE_SIZE).unwrap(),
                _ => 1,
            };
            body.extend_from_slice(&record_size.to_le_bytes());
            body.extend_from_slice(&0_u16.to_le_bytes());
        }
        body.extend_from_slice(&0_i32.to_le_bytes());
        body.extend_from_slice(
            &u16::try_from(ITEM_ID_RANGE_WIRE_SIZE)
                .unwrap()
                .to_le_bytes(),
        );
        body.extend_from_slice(&1_u16.to_le_bytes());
        body.extend_from_slice(&[0_u8; 24]);
        body.extend_from_slice(&u16::try_from(GM_HOST_WIRE_SIZE).unwrap().to_le_bytes());
        body.extend_from_slice(&0_u16.to_le_bytes());
        body.extend_from_slice(&u16::try_from(ADMIN_INFO_WIRE_SIZE).unwrap().to_le_bytes());
        body.extend_from_slice(&0_u16.to_le_bytes());
        body.extend_from_slice(&u16::try_from(MONARCH_INFO_WIRE_SIZE).unwrap().to_le_bytes());
        body.extend_from_slice(&1_u16.to_le_bytes());
        body.extend_from_slice(&[0_u8; MONARCH_INFO_WIRE_SIZE]);
        body.extend_from_slice(
            &u16::try_from(MONARCH_CANDIDACY_WIRE_SIZE)
                .unwrap()
                .to_le_bytes(),
        );
        body.extend_from_slice(&0_u16.to_le_bytes());
        body.extend_from_slice(&DB_BOOT_END_MARKER.to_le_bytes());

        let mut payload = Vec::with_capacity(body.len() + 4);
        payload.extend_from_slice(&u32::try_from(body.len() + 4).unwrap().to_le_bytes());
        payload.extend_from_slice(&body);
        parse_db_boot_payload(&payload, profile).unwrap()
    }

    fn boot_request() -> DbBootRequest {
        DbBootRequest::new([0x0102_0304, 0x0506_0708], c_array("127.0.0.1"))
    }

    #[test]
    fn ready_adapter_wraps_a_validated_boot_request_with_zero_handle() {
        let profile = BootFeatureProfile::minimal();
        let payload = minimal_fixture();
        let expected = payload.encode_frame(profile).unwrap();
        let adapter = BootResponseAdapter::new(payload, profile);
        let request = DbRequest::Boot {
            handle: 0xfeed_beef,
            request: boot_request(),
        };

        assert!(adapter.is_ready());
        assert_eq!(adapter.response_for(&request).unwrap(), expected);
        assert_eq!(expected.header, protocol::db_boot::HEADER_DG_BOOT);
        assert_eq!(expected.handle, protocol::db_boot::DB_BOOT_RESPONSE_HANDLE);
    }

    #[test]
    fn unavailable_and_oversized_adapters_do_not_fabricate_frames() {
        let profile = BootFeatureProfile::minimal();
        let request = DbRequest::Boot {
            handle: 9,
            request: boot_request(),
        };
        let unavailable = BootResponseAdapter::unavailable(profile);
        assert!(!unavailable.is_ready());
        assert_eq!(
            unavailable.response_for(&request),
            Err(BootResponseError::Unavailable)
        );

        let adapter = BootResponseAdapter::with_max_payload_size(minimal_fixture(), profile, 1);
        assert!(matches!(
            adapter.response_for(&request),
            Err(BootResponseError::Encode(
                DbBootError::EncodePayloadTooLarge { .. }
            ))
        ));
    }

    #[test]
    fn rejects_non_boot_typed_requests() {
        let adapter = BootResponseAdapter::unavailable(BootFeatureProfile::minimal());
        let horse = DbRequest::HorseName {
            handle: 1,
            request: protocol::db_records::HorseNameRequest::new(2),
        };
        assert_eq!(
            adapter.response_for(&horse),
            Err(BootResponseError::NotBootRequest)
        );

        let channel = DbRequest::FindChannel {
            handle: 2,
            request: protocol::db_records::ChannelChangeRequest::new(3, 4),
        };
        assert_eq!(
            adapter.response_for(&channel),
            Err(BootResponseError::NotBootRequest)
        );
    }

    #[tokio::test]
    async fn composes_a_fragmented_request_and_writes_a_validated_frame() {
        let profile = BootFeatureProfile::minimal();
        let payload = minimal_fixture();
        let request_wire = DbFrame::new(
            HEADER_GD_BOOT,
            0x1234_5678,
            boot_request().encode().to_vec(),
        )
        .encode()
        .unwrap();
        let (client, server) = duplex(4096);
        let (client_reader, mut client_writer) = tokio::io::split(client);
        let feed = tokio::spawn(async move {
            for byte in request_wire {
                client_writer.write_all(&[byte]).await.unwrap();
            }
        });

        let mut session = crate::session::DbPeerSession::new(server);
        let outcome = session.read_request().await.unwrap().unwrap();
        let request = match outcome {
            crate::session::DbDispatchOutcome::Validated(DbRequest::Boot {
                handle: _,
                request,
            }) => request,
            other => panic!("unexpected dispatch outcome: {other:?}"),
        };
        let adapter = BootResponseAdapter::new(payload, profile);
        let response = adapter
            .response_for(&DbRequest::Boot {
                handle: 0x1234_5678,
                request,
            })
            .unwrap();
        session.write_response(&response).await.unwrap();
        session.flush().await.unwrap();
        drop(session);
        feed.await.unwrap();

        let mut client = DbFrameTransport::new(client_reader);
        assert_eq!(client.read_frame().await.unwrap(), Some(response.clone()));
        assert_eq!(client.read_frame().await.unwrap(), None);
        let response_wire = response.encode().unwrap();
        assert_eq!(
            u32::from_le_bytes(response_wire[5..9].try_into().unwrap()) as usize,
            response.payload.len()
        );
        assert_eq!(
            u32::from_le_bytes(response.payload[..4].try_into().unwrap()) as usize,
            response.payload.len()
        );
    }
}
