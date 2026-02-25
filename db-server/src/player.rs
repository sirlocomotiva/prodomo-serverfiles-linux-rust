//! SQL-free primary player-load response composition.
//!
//! The legacy `HEADER_GD_PLAYER_LOAD` path performs login-state checks, cache
//! work, and companion queries before it sends a primary result. This module
//! owns only the response-framing boundary for an already-resolved outcome.
//! It does not execute SQL, mutate login state, or claim a complete
//! player-load service.

use protocol::db_records::{
    PlayerResultRecord, HEADER_DG_PLAYER_LOAD_FAILED, HEADER_DG_PLAYER_LOAD_SUCCESS,
    PLAYER_RESULT_WIRE_SIZE,
};
use protocol::db_wire::DbFrame;

/// The result of a caller-owned primary player lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum PlayerLoadPrimaryOutcome {
    /// A source-verified player record is available.
    Found(PlayerResultRecord),
    /// A source-verified lookup found no primary player record.
    Missing,
}

/// A pure adapter for the source-defined primary player-load response.
///
/// The adapter accepts an already-resolved outcome. It never treats an absent
/// provider, SQL error, or unavailable service as [`PlayerLoadPrimaryOutcome::Missing`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PlayerLoadPrimaryResponseAdapter;

impl PlayerLoadPrimaryResponseAdapter {
    /// Construct the stateless response adapter.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Compose one primary response frame.
    ///
    /// `Found` writes header `35`, the original handle, and exactly the
    /// source-fixed 2,007-byte `TPlayerTable` payload. `Missing` writes header
    /// `36`, the original handle, and an empty payload. No supplementary
    /// `NEED_LOGIN_LOG`, item, quest, affect, or private-shop frame is emitted.
    #[must_use]
    pub fn response_for(&self, handle: u32, outcome: &PlayerLoadPrimaryOutcome) -> DbFrame {
        match outcome {
            PlayerLoadPrimaryOutcome::Found(record) => {
                let payload = record.encode();
                debug_assert_eq!(payload.len(), PLAYER_RESULT_WIRE_SIZE);
                DbFrame::new(HEADER_DG_PLAYER_LOAD_SUCCESS, handle, payload)
            }
            PlayerLoadPrimaryOutcome::Missing => {
                DbFrame::new(HEADER_DG_PLAYER_LOAD_FAILED, handle, Vec::new())
            }
        }
    }
}

/// C++-style semantic alias for the primary outcome.
pub type PlayerLoadOutcome = PlayerLoadPrimaryOutcome;

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_records::{decode_player_result, PlayerLoadRequest};

    fn request() -> PlayerLoadRequest {
        PlayerLoadRequest {
            account_id: 11,
            player_id: 22,
            account_index: 2,
        }
    }

    #[allow(clippy::field_reassign_with_default)]
    fn record() -> PlayerResultRecord {
        let mut value = PlayerResultRecord::default();
        value.id = 22;
        value.level = 7;
        value
    }

    #[test]
    fn found_outcome_writes_exact_success_record_and_preserves_handle() {
        let adapter = PlayerLoadPrimaryResponseAdapter::new();
        let frame = adapter.response_for(0xa1b2_c3d4, &PlayerLoadPrimaryOutcome::Found(record()));
        assert_eq!(frame.header, HEADER_DG_PLAYER_LOAD_SUCCESS);
        assert_eq!(frame.handle, 0xa1b2_c3d4);
        assert_eq!(frame.payload.len(), PLAYER_RESULT_WIRE_SIZE);
        assert_eq!(decode_player_result(&frame.payload).unwrap(), record());
    }

    #[test]
    fn missing_outcome_writes_empty_failure_and_preserves_handle() {
        let adapter = PlayerLoadPrimaryResponseAdapter::new();
        let frame = adapter.response_for(0, &PlayerLoadPrimaryOutcome::Missing);
        assert_eq!(frame.header, HEADER_DG_PLAYER_LOAD_FAILED);
        assert_eq!(frame.handle, 0);
        assert!(frame.payload.is_empty());
    }

    #[test]
    fn request_codec_fixture_remains_nine_bytes() {
        assert_eq!(request().encode().len(), PlayerLoadRequest::WIRE_SIZE);
        assert_eq!(PlayerLoadRequest::WIRE_SIZE, 9);
    }
}
