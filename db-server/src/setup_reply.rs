//! Composition of the legacy database setup reply.
//!
//! `CClientManager::QUERY_SETUP` answers a game core's setup request with one
//! `HEADER_DG_MAP_LOCATIONS` frame, built from the requesting peer's own
//! public IP, listen port, and map list. The other seven frames the legacy
//! handler can emit (party, guild war, privileges, event flags, marriage, and
//! private shop) are all guarded by a non-empty container or a SQL row count,
//! so a DB with none of that state sends exactly one frame.
//!
//! # What is deliberately not modelled
//!
//! - The legacy reply length depends on a process-wide `m_peerList`, so it can
//!   contain one `TMapLocation` per connected game peer. That list is global
//!   mutable state and is not built here, so the reply carries only the
//!   requester's own location. A single-channel deployment is unaffected.
//! - The auth branch of the legacy handler returns before writing anything. An
//!   auth-role peer therefore receives zero bytes, which is reproduced here as
//!   [`SetupReplyOutcome::Silent`] rather than as an empty frame.
//! - The legacy handler also copies the peer's values into the DB-wide peer
//!   list. This module performs no such registration; the authorization
//!   decision owns the only global effect.

use std::fmt;

use protocol::db_map_locations::{
    MapLocation, MapLocationsError, MapLocationsReply, MAP_LOCATIONS_REPLY_HANDLE,
};
use protocol::db_wire::DbFrame;

use crate::setup::{SetupBase, SetupRequest};

/// A failure while composing the setup reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupReplyError {
    /// The request declares a map list or host that the record cannot hold.
    Record(MapLocationsError),
    /// No setup composer is installed.
    Unavailable,
}

impl From<MapLocationsError> for SetupReplyError {
    fn from(error: MapLocationsError) -> Self {
        Self::Record(error)
    }
}

impl fmt::Display for SetupReplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Record(error) => {
                write!(formatter, "setup reply record could not be built: {error}")
            }
            Self::Unavailable => formatter.write_str("no setup reply composer is available"),
        }
    }
}

impl std::error::Error for SetupReplyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Record(error) => Some(error),
            Self::Unavailable => None,
        }
    }
}

/// What the legacy handler would write for one validated setup request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupReplyOutcome {
    /// One `HEADER_DG_MAP_LOCATIONS` frame is sent to the requesting peer.
    Frame(DbFrame),
    /// The legacy auth branch writes no bytes at all.
    ///
    /// This is not an empty frame. Sending a zero-length `0xfe` frame would
    /// make a peer parse a count byte that does not exist, so the connection
    /// carries no reply and waits for the next request.
    Silent,
}

/// Compose the reply for one validated setup request.
///
/// The values come from the request itself: the legacy handler copies the
/// peer's public IP, listen port, and map array into the record it echoes
/// back. Nothing is read from SQL, configuration, or another peer.
///
/// The `bAuthServer` byte is read only to select the silent branch. It is raw
/// payload data and is never treated as a credential; peer authorization has
/// already happened and owns the auth-role decision.
///
/// # Errors
///
/// Returns [`SetupReplyError::Record`] when the requested map list cannot be
/// held by a 32-slot record.
pub fn compose_setup_reply(request: &SetupRequest) -> Result<SetupReplyOutcome, SetupReplyError> {
    let SetupBase {
        public_ip,
        listen_port,
        maps,
        auth_server,
        ..
    } = *request.base();

    if auth_server != 0 {
        return Ok(SetupReplyOutcome::Silent);
    }

    let leading = leading_map_count(&maps);
    let indices = maps[..leading].to_vec();
    let record = MapLocation::new(&indices, public_ip, listen_port)?;
    let reply = MapLocationsReply::new(vec![record])?;
    let payload = reply.encode()?;
    Ok(SetupReplyOutcome::Frame(DbFrame::new(
        protocol::db_map_locations::HEADER_DG_MAP_LOCATIONS,
        MAP_LOCATIONS_REPLY_HANDLE,
        payload,
    )))
}

/// Return the number of leading map indices before the first zero.
///
/// The legacy reply always writes all 32 slots, and the game reader stops at
/// the first zero. Trimming to the leading run and letting
/// [`MapLocation::new`] zero-pad produces the same 146 bytes without inventing
/// a list from whatever follows the terminator.
fn leading_map_count(maps: &[i32]) -> usize {
    maps.iter().take_while(|index| **index != 0).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::{
        decode_setup_request, SetupDecodeError, SetupDecodeLimits, SetupFeatureProfile,
    };

    /// Build a validated base-only setup request from raw 154-byte payload
    /// fields, so every test drives the same strict decoder the server uses.
    fn request(
        public_ip: [u8; 16],
        listen_port: u16,
        maps: [i32; 32],
        auth_server: u8,
    ) -> SetupRequest {
        let mut payload = vec![0_u8; 154];
        payload[..16].copy_from_slice(&public_ip);
        payload[17..19].copy_from_slice(&listen_port.to_le_bytes());
        for (slot, index) in maps.iter().enumerate() {
            let at = 21 + slot * 4;
            payload[at..at + 4].copy_from_slice(&index.to_le_bytes());
        }
        payload[149..153].copy_from_slice(&0_u32.to_le_bytes());
        payload[153] = auth_server;
        decode_setup_request(
            &payload,
            SetupFeatureProfile::ActiveX86,
            SetupDecodeLimits::default(),
        )
        .expect("fixture is a valid base-only setup")
    }

    fn maps(leading: &[i32]) -> [i32; 32] {
        let mut all = [0_i32; 32];
        all[..leading.len()].copy_from_slice(leading);
        all
    }

    fn host() -> [u8; 16] {
        let mut raw = [0_u8; 16];
        raw[..9].copy_from_slice(b"127.0.0.1");
        raw
    }

    fn decode(outcome: &SetupReplyOutcome) -> Vec<u8> {
        let SetupReplyOutcome::Frame(frame) = outcome else {
            panic!("expected a frame outcome");
        };
        let raw = frame.encode().expect("frame encodes");
        assert_eq!(raw[0], protocol::db_map_locations::HEADER_DG_MAP_LOCATIONS);
        assert_eq!(&raw[1..5], &0_u32.to_le_bytes());
        raw[9..].to_vec()
    }

    #[test]
    fn a_game_setup_gets_exactly_one_zero_handled_map_locations_frame() {
        let outcome = compose_setup_reply(&request(host(), 50080, maps(&[1, 2, 3]), 0))
            .expect("composition succeeds");
        let SetupReplyOutcome::Frame(frame) = &outcome else {
            panic!("a game setup must be answered with a frame");
        };
        let raw = frame.encode().expect("frame encodes");
        // 9 header bytes plus a one-byte count plus one 146-byte record.
        assert_eq!(raw.len(), 9 + 147);
        assert_eq!(frame.header, 0xfe);
        assert_eq!(frame.handle, 0);
    }

    #[test]
    fn the_payload_is_a_one_count_byte_and_one_146_byte_record() {
        let outcome = compose_setup_reply(&request(host(), 50080, maps(&[1, 2, 3]), 0))
            .expect("composition succeeds");
        let payload = decode(&outcome);
        assert_eq!(payload.len(), 147);
        assert_eq!(payload[0], 1);
        let record = MapLocation::decode(&payload[1..]).expect("one whole record");
        assert_eq!(record.host, host());
        assert_eq!(record.port, 50080);
        assert_eq!(&record.map_indices[..4], &[1, 2, 3, 0]);
    }

    #[test]
    fn the_reply_echoes_the_requests_own_public_ip_listen_port_and_maps() {
        // Nothing else is consulted: not SQL, not configuration, not another
        // peer. Two different requests must produce two different replies.
        let first = compose_setup_reply(&request(host(), 50080, maps(&[1, 2]), 0))
            .expect("composition succeeds");
        let mut other_host = [0_u8; 16];
        other_host[..9].copy_from_slice(b"10.0.0.42");
        let second = compose_setup_reply(&request(other_host, 50900, maps(&[7, 8, 9]), 0))
            .expect("composition succeeds");
        assert_ne!(decode(&first), decode(&second));
        let record = MapLocation::decode(&decode(&second)[1..]).expect("one whole record");
        assert_eq!(record.host, other_host);
        assert_eq!(record.port, 50900);
        assert_eq!(&record.map_indices[..4], &[7, 8, 9, 0]);
    }

    #[test]
    fn a_map_list_terminated_by_zero_is_trimmed_and_zero_padded_to_32() {
        let payload = decode(
            &compose_setup_reply(&request(host(), 1, maps(&[5]), 0)).expect("composition succeeds"),
        );
        let record = MapLocation::decode(&payload[1..]).expect("one whole record");
        assert_eq!(record.map_indices[0], 5);
        assert_eq!(&record.map_indices[1..], &[0_i32; 31]);
        assert_eq!(record.map_index_count(), 1);
    }

    #[test]
    fn a_non_zero_slot_after_the_terminator_is_not_mistaken_for_a_map() {
        // The game reader stops at the first zero, so slots past it are dead
        // bytes. The reply must not present them as a longer list.
        let mut dirty = maps(&[1, 2]);
        dirty[10] = 999;
        let payload = decode(
            &compose_setup_reply(&request(host(), 1, dirty, 0)).expect("composition succeeds"),
        );
        let record = MapLocation::decode(&payload[1..]).expect("one whole record");
        assert_eq!(record.map_index_count(), 2);
        assert_eq!(record.map_indices[10], 0);
    }

    #[test]
    fn all_32_map_slots_survive_when_every_one_is_nonzero() {
        let full: Vec<i32> = (1..=32).collect();
        let payload = decode(
            &compose_setup_reply(&request(host(), 1, maps(&full), 0))
                .expect("composition succeeds"),
        );
        let record = MapLocation::decode(&payload[1..]).expect("one whole record");
        assert_eq!(record.map_index_count(), 32);
        assert_eq!(&record.map_indices, &full[..]);
    }

    #[test]
    fn an_auth_setup_receives_no_frame_at_all() {
        // The legacy auth branch returns before any Encode call. A zero-length
        // 0xfe frame would make the peer read a count byte that is not there,
        // so the outcome is silence, not an empty frame.
        let outcome = compose_setup_reply(&request(host(), 50080, maps(&[1]), 1))
            .expect("composition succeeds");
        assert_eq!(outcome, SetupReplyOutcome::Silent);
    }

    #[test]
    fn any_nonzero_auth_byte_selects_the_silent_branch() {
        // The legacy test is `if (p->bAuthServer)`, so every nonzero value
        // routes the same way. The byte stays raw and uninterpreted beyond that.
        for value in [1_u8, 2, 0x7f, 0x80, 0xff] {
            assert_eq!(
                compose_setup_reply(&request(host(), 1, maps(&[1]), value))
                    .expect("composition succeeds"),
                SetupReplyOutcome::Silent,
                "auth byte {value} must be silent"
            );
        }
        assert_eq!(
            compose_setup_reply(&request(host(), 1, maps(&[1]), 0)).expect("composition succeeds"),
            compose_setup_reply(&request(host(), 1, maps(&[1]), 0)).expect("composition succeeds")
        );
        assert!(matches!(
            compose_setup_reply(&request(host(), 1, maps(&[1]), 0)).expect("composition succeeds"),
            SetupReplyOutcome::Frame(_)
        ));
    }

    #[test]
    fn a_request_carrying_login_records_still_gets_the_same_single_frame() {
        // The reply is the same whether or not login records are present: the
        // legacy handler builds it from the peer's own location either way.
        let mut payload = vec![0_u8; 154 + 100];
        payload[..16].copy_from_slice(&host());
        payload[17..19].copy_from_slice(&50080_u16.to_le_bytes());
        for (slot, index) in [1_i32, 2].iter().enumerate() {
            let at = 21 + slot * 4;
            payload[at..at + 4].copy_from_slice(&index.to_le_bytes());
        }
        payload[149..153].copy_from_slice(&1_u32.to_le_bytes());
        let with_login = decode_setup_request(
            &payload,
            SetupFeatureProfile::ActiveX86,
            SetupDecodeLimits::default(),
        )
        .expect("fixture is a valid one-record setup");
        assert_eq!(with_login.logins().len(), 1);
        let outcome = compose_setup_reply(&with_login).expect("composition succeeds");
        assert_eq!(decode(&outcome).len(), 147);
    }

    #[test]
    fn a_high_byte_public_ip_is_preserved_as_raw_storage_not_text() {
        // `szPublicIP` is a fixed 16-byte array. A peer that fills all 16
        // bytes makes the legacy `strlen` read past the field; this codec keeps
        // the bytes and never treats them as a C string.
        let raw_host: [u8; 16] = [
            0xFF, 0xFE, 0x00, 0x01, 0x80, 0x7F, 0xC3, 0x28, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        let payload = decode(
            &compose_setup_reply(&request(raw_host, 1, maps(&[1]), 0))
                .expect("composition succeeds"),
        );
        let record = MapLocation::decode(&payload[1..]).expect("one whole record");
        assert_eq!(record.host, raw_host);
    }

    #[test]
    fn an_empty_map_list_still_produces_one_whole_record() {
        let payload = decode(
            &compose_setup_reply(&request(host(), 1, maps(&[]), 0)).expect("composition succeeds"),
        );
        assert_eq!(payload.len(), 147);
        let record = MapLocation::decode(&payload[1..]).expect("one whole record");
        assert_eq!(record.map_index_count(), 0);
        assert_eq!(&record.map_indices, &[0_i32; 32]);
    }

    #[test]
    fn composition_is_deterministic_for_the_same_request() {
        // The legacy reply leaks uninitialized stack bytes into szHost, so it
        // is not reproducible. This one must be byte-for-byte stable.
        let first = compose_setup_reply(&request(host(), 50080, maps(&[1, 2]), 0))
            .expect("composition succeeds");
        let second = compose_setup_reply(&request(host(), 50080, maps(&[1, 2]), 0))
            .expect("composition succeeds");
        assert_eq!(decode(&first), decode(&second));
    }

    #[test]
    fn the_decoder_still_refuses_a_short_setup_payload() {
        // The reply composer trusts a validated request, so the boundary that
        // matters is the decoder. Re-check it here so the two cannot drift.
        assert!(matches!(
            decode_setup_request(
                &[0_u8; 10],
                SetupFeatureProfile::ActiveX86,
                SetupDecodeLimits::default(),
            ),
            Err(SetupDecodeError::InvalidWidth { .. })
        ));
    }
}
