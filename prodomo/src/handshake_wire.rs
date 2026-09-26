//! Pure wire projection for handshake reducer effects.
//!
//! The legacy descriptor writes the same records represented by
//! `protocol::gc`: a 13-byte `TPacketGCHandshake`, a two-byte
//! `TPacketGCPhase`, or the one-byte `0xfc` time-sync acknowledgement. This
//! module only projects already-reduced effects into raw wire bytes and
//! preserves their order. It does not apply a phase processor, negotiate
//! TEA/key agreement, encrypt output, or write to a socket.

#![warn(missing_docs)]

use crate::handshake::{HandshakeEffect, HandshakePhase, HEADER_GC_TIME_SYNC};
use protocol::gc::{GcHandshake, GcPhase};

/// Encode one pure handshake effect as its raw legacy game-to-client record.
///
/// `SetPhase` produces a `TPacketGCPhase` record for a non-close phase. The
/// legacy close operation assigns `PHASE_CLOSE` before calling `Packet`, and
/// `Packet` returns immediately in that phase, so `Close` has no wire bytes.
/// The caller must still perform the close input-processor transition.
#[must_use]
pub fn encode_handshake_effect(effect: &HandshakeEffect) -> Vec<u8> {
    match effect {
        HandshakeEffect::SetPhase { phase, .. } if *phase == HandshakePhase::Close => Vec::new(),
        HandshakeEffect::SetPhase { phase, .. } => GcPhase::new(phase.legacy_value()).encode(),
        HandshakeEffect::SendHandshake { token, time, delta } => {
            GcHandshake::new(*token, *time, *delta).encode()
        }
        HandshakeEffect::SendTimeSyncAck => vec![HEADER_GC_TIME_SYNC],
        HandshakeEffect::Close => Vec::new(),
    }
}

/// Encode a source-ordered effect sequence without merging or reordering it.
///
/// The returned vector contains one independently framed record per effect.
/// A caller remains responsible for transport writes and the descriptor's
/// plaintext/TEA boundary.
#[must_use]
pub fn encode_handshake_effects(effects: &[HandshakeEffect]) -> Vec<Vec<u8>> {
    effects.iter().map(encode_handshake_effect).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handshake::{reduce, HandshakeInputBoundary, HandshakeServerKind, HandshakeState};
    use protocol::cg_handshake::{CgHandshakeHeader, CgInboundHandshake};

    #[test]
    fn startup_projects_phase_then_handshake_in_source_order() {
        let start = HandshakeState::start(0x1234_5678, HandshakeServerKind::Game, 100);
        let frames = encode_handshake_effects(&start.effects);

        assert_eq!(
            frames,
            vec![
                vec![0xfd, 1],
                vec![0xff, 0x78, 0x56, 0x34, 0x12, 0x64, 0, 0, 0, 0, 0, 0, 0,]
            ]
        );
    }

    #[test]
    fn accepted_game_handshake_projects_login_phase() {
        let start = HandshakeState::start(7, HandshakeServerKind::Game, 100);
        let packet = CgInboundHandshake::new(CgHandshakeHeader::Handshake, 7, 90, 5);
        let reduction = reduce(start.state, packet, 100);

        assert_eq!(
            reduction.effects,
            vec![HandshakeEffect::SetPhase {
                phase: HandshakePhase::Login,
                input_boundary: HandshakeInputBoundary::LegacyTea,
            }]
        );
        assert_eq!(
            encode_handshake_effects(&reduction.effects),
            vec![vec![0xfd, 2]]
        );
    }

    #[test]
    fn auth_phase_uses_the_legacy_numeric_phase_value() {
        let effect = HandshakeEffect::SetPhase {
            phase: HandshakePhase::Auth,
            input_boundary: HandshakeInputBoundary::LegacyTea,
        };
        assert_eq!(encode_handshake_effect(&effect), vec![0xfd, 10]);
    }

    #[test]
    fn resync_effect_is_exactly_the_one_byte_time_sync_header() {
        let effect = HandshakeEffect::SendTimeSyncAck;
        assert_eq!(encode_handshake_effect(&effect), vec![0xfc]);
    }

    #[test]
    fn close_effect_projects_no_wire_record() {
        assert!(encode_handshake_effect(&HandshakeEffect::Close).is_empty());
        assert!(encode_handshake_effect(&HandshakeEffect::SetPhase {
            phase: HandshakePhase::Close,
            input_boundary: HandshakeInputBoundary::Plaintext,
        })
        .is_empty());
    }

    #[test]
    fn invalid_handshake_and_retry_exhaustion_project_no_close_bytes() {
        let start = HandshakeState::start(7, HandshakeServerKind::Game, 100);
        let wrong = CgInboundHandshake::new(CgHandshakeHeader::Handshake, 8, 100, 0);
        let rejected = reduce(start.state, wrong, 100);
        let rejected_frames = encode_handshake_effects(&rejected.effects);
        assert_eq!(rejected_frames.len(), 1);
        assert!(rejected_frames[0].is_empty());

        let mut state = start.state;
        for _ in 0..=crate::handshake::HANDSHAKE_RETRY_LIMIT {
            let result = reduce(
                state,
                CgInboundHandshake::new(CgHandshakeHeader::Handshake, 7, 0, 0),
                u32::MAX,
            );
            state = result.state;
            if result
                .effects
                .iter()
                .any(|effect| *effect == HandshakeEffect::Close)
            {
                let frames = encode_handshake_effects(&result.effects);
                assert_eq!(frames.len(), 1);
                assert!(frames[0].is_empty());
                return;
            }
        }
        panic!("retry limit did not close");
    }
}
