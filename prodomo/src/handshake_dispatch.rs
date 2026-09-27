//! Transport-free dispatch from a decoded handshake frame to client lifecycle state.
//!
//! The legacy descriptor checks `PHASE_CLOSE` before it interprets a header.
//! TEA is a stream boundary before `CInputProcessor::Process`, so a raw
//! ciphertext frame must not be passed to this adapter. This module accepts a
//! complete `ClientFrame` after the caller has selected and, when necessary,
//! completed the legacy TEA boundary. It validates the source packet-info size,
//! decodes the exact packed 13-byte handshake/time-sync record, and returns a
//! candidate `LifecycleReduction` without committing state, installing keys,
//! writing a socket, or applying effects.

use std::error::Error;
use std::fmt;

use protocol::cg_handshake::{CgHandshakeError, CgInboundHandshake};
use protocol::cg_wire::{
    resolve_client_frame_size, ClientFrame, ClientFrameError, ClientFrameSize,
};

use crate::client_session::ClientPhase;
use crate::lifecycle::{ClientLifecycle, LifecycleError, LifecycleReduction};

/// The explicit input boundary asserted by the caller before dispatch.
///
/// `DecryptedLegacyTea` is an assertion that the caller has already removed
/// the source TEA layer. This module does not decrypt, install keys, or prove
/// that a socket performed that work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeInputBoundary {
    /// The frame is still in the initial plaintext handshake boundary.
    Plaintext,
    /// The caller has already decrypted the frame for a post-handshake phase.
    DecryptedLegacyTea,
}

impl HandshakeInputBoundary {
    /// Return whether this boundary represents an already-decrypted TEA frame.
    #[must_use]
    pub const fn is_decrypted_legacy_tea(self) -> bool {
        matches!(self, Self::DecryptedLegacyTea)
    }
}

/// A failure raised before or during decoded handshake dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeDispatchError {
    /// The descriptor is closed, so it ignores the frame before header lookup.
    ///
    /// If either supplied phase is `Close`, this terminal result takes
    /// precedence over a phase mismatch. Otherwise a disagreement is reported
    /// as `PhaseMismatch`.
    PhaseClosed {
        /// Phase supplied by the transport/session boundary.
        session_phase: ClientPhase,
        /// Phase held by the lifecycle candidate.
        lifecycle_phase: ClientPhase,
    },
    /// The transport boundary and lifecycle candidate disagree about phase.
    PhaseMismatch {
        /// Phase supplied by the transport/session boundary.
        session_phase: ClientPhase,
        /// Phase held by the lifecycle candidate.
        lifecycle_phase: ClientPhase,
    },
    /// The caller supplied the wrong decoded/plaintext boundary for the phase.
    InputBoundaryMismatch {
        /// Lifecycle phase whose boundary was checked.
        phase: ClientPhase,
        /// Boundary required by that phase.
        expected: HandshakeInputBoundary,
        /// Boundary asserted by the caller.
        actual: HandshakeInputBoundary,
    },
    /// The complete frame uses a header outside the two handshake records.
    ///
    /// This includes unknown headers, fixed non-handshake records, and headers
    /// requiring variable `Analyze` logic. The pure adapter returns an error;
    /// it does not reproduce a legacy analyzer close.
    UnsupportedHeader {
        /// Descriptor phase supplied by the caller.
        phase: ClientPhase,
        /// The one-byte client header.
        header: u8,
    },
    /// A handshake header failed packet-info size validation.
    Protocol(ClientFrameError),
    /// The packet-info frame had a valid outer size but an invalid borrowed
    /// handshake body/header after boundary resolution.
    Decode(CgHandshakeError),
    /// The lifecycle reducer rejected the decoded record in its internal state.
    Lifecycle(LifecycleError),
}

impl HandshakeDispatchError {
    /// Return whether this is a closed-phase boundary error.
    #[must_use]
    pub const fn is_phase_closed(&self) -> bool {
        matches!(self, Self::PhaseClosed { .. })
    }

    /// Return whether this is a phase-consistency error.
    #[must_use]
    pub const fn is_phase_mismatch(&self) -> bool {
        matches!(self, Self::PhaseMismatch { .. })
    }

    /// Return whether this is an explicit input-boundary error.
    #[must_use]
    pub const fn is_input_boundary_mismatch(&self) -> bool {
        matches!(self, Self::InputBoundaryMismatch { .. })
    }

    /// Return whether this error is a non-handshake header boundary.
    #[must_use]
    pub const fn is_unsupported_header(&self) -> bool {
        matches!(self, Self::UnsupportedHeader { .. })
    }
}

impl fmt::Display for HandshakeDispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PhaseClosed {
                session_phase,
                lifecycle_phase,
            } => write!(
                formatter,
                "handshake dispatch is closed (session {session_phase:?}, lifecycle {lifecycle_phase:?})"
            ),
            Self::PhaseMismatch {
                session_phase,
                lifecycle_phase,
            } => write!(
                formatter,
                "handshake phase mismatch: session {session_phase:?}, lifecycle {lifecycle_phase:?}"
            ),
            Self::InputBoundaryMismatch {
                phase,
                expected,
                actual,
            } => write!(
                formatter,
                "handshake input boundary mismatch in phase {phase:?}: expected {expected:?}, got {actual:?}"
            ),
            Self::UnsupportedHeader { phase, header } => write!(
                formatter,
                "client header 0x{header:02x} is not a handshake record in phase {phase:?}"
            ),
            Self::Protocol(error) => error.fmt(formatter),
            Self::Decode(error) => error.fmt(formatter),
            Self::Lifecycle(error) => error.fmt(formatter),
        }
    }
}

impl Error for HandshakeDispatchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Protocol(error) => Some(error),
            Self::Decode(error) => Some(error),
            Self::Lifecycle(error) => Some(error),
            Self::PhaseClosed { .. }
            | Self::PhaseMismatch { .. }
            | Self::InputBoundaryMismatch { .. }
            | Self::UnsupportedHeader { .. } => None,
        }
    }
}

/// A candidate lifecycle reduction and the exact source frame that caused it.
///
/// The caller owns application of `reduction.state` and `reduction.effects`.
/// This type is deliberately effect-based: it does not mutate a live
/// descriptor or send any bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeDispatchReduction {
    /// The complete frame, including its one-byte header.
    pub frame: ClientFrame,
    /// Candidate state and effects returned by the lifecycle reducer.
    pub reduction: LifecycleReduction,
}

/// Decode and reduce one already-framed handshake or time-sync record.
///
/// The boundary order follows the source descriptor:
/// 1. reject a closed phase before looking at the frame header;
/// 2. verify the caller's explicit phase and input boundary;
/// 3. reject every non-handshake header, then resolve the handshake
///    packet-info size;
/// 4. decode the exact packed record through the borrowed frame decoder; and
/// 5. invoke the existing lifecycle reducer.
///
/// The function never applies the returned phase or effects. In particular, a
/// `SetPhase` effect remains an intent to write the phase record and then apply
/// the next input boundary; this adapter does not perform either operation.
///
/// `input_boundary` must be [`HandshakeInputBoundary::Plaintext`] for the
/// initial handshake phase and [`HandshakeInputBoundary::DecryptedLegacyTea`]
/// for a phase whose legacy input boundary is TEA. The latter is a caller
/// assertion only; it is not a claim that this module implemented TEA or
/// verified a socket decryption. The caller must keep that evidence at the
/// transport boundary.
///
/// # Errors
///
/// Returns [`HandshakeDispatchError::PhaseClosed`] before inspecting a header
/// when either supplied phase is closed. A disagreement between two non-closed
/// phases is [`HandshakeDispatchError::PhaseMismatch`]. Every non-handshake
/// header, including an unknown or variable header, is
/// [`HandshakeDispatchError::UnsupportedHeader`]; the adapter does not turn
/// that error into a close. Handshake size, borrowed decode, and lifecycle
/// failures remain distinct.
pub fn dispatch_handshake_frame_at_boundary(
    frame: ClientFrame,
    session_phase: ClientPhase,
    input_boundary: HandshakeInputBoundary,
    lifecycle: ClientLifecycle,
    now: u32,
) -> Result<HandshakeDispatchReduction, HandshakeDispatchError> {
    let lifecycle_phase = lifecycle.phase();

    // CInputProcessor::Process checks PHASE_CLOSE before header lookup. Keep
    // that precedence even when the caller supplies a malformed frame.
    if session_phase == ClientPhase::Close || lifecycle_phase == ClientPhase::Close {
        return Err(HandshakeDispatchError::PhaseClosed {
            session_phase,
            lifecycle_phase,
        });
    }

    // The outer session and the pure lifecycle candidate must describe the
    // same descriptor phase. Do not silently choose one as authoritative.
    if session_phase != lifecycle_phase {
        return Err(HandshakeDispatchError::PhaseMismatch {
            session_phase,
            lifecycle_phase,
        });
    }

    let expected_boundary = expected_input_boundary(lifecycle_phase);
    if input_boundary != expected_boundary {
        return Err(HandshakeDispatchError::InputBoundaryMismatch {
            phase: lifecycle_phase,
            expected: expected_boundary,
            actual: input_boundary,
        });
    }

    // This adapter has one explicit contract: only 0xff and 0xfc are
    // handshake records. Reject every other complete header before consulting
    // packet-info, so variable and unknown non-handshake inputs cannot be
    // mistaken for a handshake or silently become a legacy close.
    if !matches!(frame.header, 0xff | 0xfc) {
        return Err(HandshakeDispatchError::UnsupportedHeader {
            phase: session_phase,
            header: frame.header,
        });
    }

    let size = resolve_client_frame_size(frame.header).map_err(HandshakeDispatchError::Protocol)?;
    match size {
        ClientFrameSize::Variable(base_size) => {
            return Err(HandshakeDispatchError::Protocol(
                ClientFrameError::VariableLengthUnsupported {
                    header: frame.header,
                    base_size,
                },
            ));
        }
        ClientFrameSize::Fixed(expected_size) => {
            let actual_size = frame
                .encoded_len()
                .map_err(HandshakeDispatchError::Protocol)?;
            if actual_size != expected_size {
                return Err(HandshakeDispatchError::Protocol(
                    ClientFrameError::LengthMismatch {
                        header: frame.header,
                        expected: expected_size,
                        actual: actual_size,
                    },
                ));
            }
        }
    }

    let packet =
        CgInboundHandshake::decode_frame(&frame).map_err(HandshakeDispatchError::Decode)?;
    let reduction = lifecycle
        .on_handshake(packet, now)
        .map_err(HandshakeDispatchError::Lifecycle)?;

    Ok(HandshakeDispatchReduction { frame, reduction })
}

/// Return the input boundary required by a descriptor phase.
fn expected_input_boundary(phase: ClientPhase) -> HandshakeInputBoundary {
    if phase.legacy_tea_encrypted() {
        HandshakeInputBoundary::DecryptedLegacyTea
    } else {
        HandshakeInputBoundary::Plaintext
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every handshake refusal in legacy is `d->SetPhase(PHASE_CLOSE)`
    /// (`G/input.cpp:138` for a bad token, `:265` for an unexpected header, `:218` for a
    /// guild-mark login it cannot serve, `:87` in `CInputProcessor::Handshake`), so the effect
    /// is a phase change and not a teardown. It is byte-free either way, because `SetPhase`
    /// assigns `m_iPhase` before it calls `Packet` and `Packet` returns at once for
    /// `PHASE_CLOSE` (`G/desc.cpp:495-500` against `:397-401`).
    fn set_close(boundary: LifecycleInputBoundary) -> LifecycleEffect {
        LifecycleEffect::SetPhase {
            phase: ClientPhase::Close,
            output_boundary: boundary,
            input_boundary: boundary,
        }
    }
    use crate::handshake::{HandshakeServerKind, HANDSHAKE_RETRY_LIMIT};
    use crate::lifecycle::{
        encode_lifecycle_effects, LifecycleEffect, LifecycleInputBoundary, PostHandshakePhase,
    };
    use protocol::cg_handshake::CgHandshakeHeader;

    const TOKEN: u32 = 0x1234_5678;

    fn frame_with_token(
        header: CgHandshakeHeader,
        token: u32,
        time: u32,
        delta: i32,
    ) -> ClientFrame {
        let bytes = CgInboundHandshake::new(header, token, time, delta).encode();
        ClientFrame::new(header.value(), &bytes[1..])
    }

    fn frame(header: CgHandshakeHeader, time: u32, delta: i32) -> ClientFrame {
        frame_with_token(header, TOKEN, time, delta)
    }

    fn initial(now: u32) -> ClientLifecycle {
        ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, now).state
    }

    fn initial_auth(now: u32) -> ClientLifecycle {
        ClientLifecycle::start(TOKEN, HandshakeServerKind::Auth, now).state
    }

    fn post_login(now: u32) -> ClientLifecycle {
        initial(now)
            .on_handshake(
                CgInboundHandshake::new(CgHandshakeHeader::Handshake, TOKEN, now, 0),
                now,
            )
            .unwrap()
            .state
    }

    fn dispatch(
        input: ClientFrame,
        phase: ClientPhase,
        boundary: HandshakeInputBoundary,
        lifecycle: ClientLifecycle,
        now: u32,
    ) -> Result<HandshakeDispatchReduction, HandshakeDispatchError> {
        dispatch_handshake_frame_at_boundary(input, phase, boundary, lifecycle, now)
    }

    #[test]
    fn initial_handshake_uses_plaintext_and_returns_candidate_effects() {
        let lifecycle = initial(100);
        let input = frame(CgHandshakeHeader::Handshake, 90, 5);
        let result = dispatch(
            input.clone(),
            ClientPhase::Handshake,
            HandshakeInputBoundary::Plaintext,
            lifecycle,
            100,
        )
        .unwrap();

        assert_eq!(result.frame, input);
        assert_eq!(lifecycle.phase(), ClientPhase::Handshake);
        assert_eq!(result.reduction.state.phase(), ClientPhase::Login);
        assert_eq!(
            result.reduction.effects,
            vec![LifecycleEffect::SetPhase {
                phase: ClientPhase::Login,
                output_boundary: LifecycleInputBoundary::Plaintext,
                input_boundary: LifecycleInputBoundary::LegacyTea,
            }]
        );
        assert_eq!(
            encode_lifecycle_effects(&result.reduction.effects),
            vec![vec![0xfd, 2]]
        );
    }

    #[test]
    fn auth_initial_handshake_projects_auth_phase() {
        let result = dispatch(
            frame(CgHandshakeHeader::Handshake, 100, 0),
            ClientPhase::Handshake,
            HandshakeInputBoundary::Plaintext,
            initial_auth(100),
            100,
        )
        .unwrap();
        assert_eq!(result.reduction.state.phase(), ClientPhase::Auth);
        assert_eq!(
            result.reduction.effects,
            vec![LifecycleEffect::SetPhase {
                phase: ClientPhase::Auth,
                output_boundary: LifecycleInputBoundary::Plaintext,
                input_boundary: LifecycleInputBoundary::LegacyTea,
            }]
        );
        assert_eq!(
            encode_lifecycle_effects(&result.reduction.effects),
            vec![vec![0xfd, 10]]
        );
    }

    #[test]
    fn initial_bad_token_closes_without_wire_bytes() {
        let result = dispatch(
            frame_with_token(CgHandshakeHeader::Handshake, TOKEN ^ 1, 100, 0),
            ClientPhase::Handshake,
            HandshakeInputBoundary::Plaintext,
            initial(100),
            100,
        )
        .unwrap();
        assert_eq!(result.reduction.state.phase(), ClientPhase::Close);
        let effects = result.reduction.effects;
        assert_eq!(effects, vec![set_close(LifecycleInputBoundary::Plaintext)]);
        assert!(
            encode_lifecycle_effects(&effects).is_empty(),
            "a bad token is silent"
        );
    }

    #[test]
    fn initial_negative_delta_is_consumed_without_state_change() {
        let lifecycle = initial(100);
        let result = dispatch(
            frame(CgHandshakeHeader::Handshake, 100, -1),
            ClientPhase::Handshake,
            HandshakeInputBoundary::Plaintext,
            lifecycle,
            100,
        )
        .unwrap();
        assert_eq!(result.reduction.state, lifecycle);
        assert!(result.reduction.effects.is_empty());
    }

    #[test]
    fn initial_timing_retry_and_retry_limit_close_are_preserved() {
        let first = dispatch(
            frame(CgHandshakeHeader::Handshake, 0, 0),
            ClientPhase::Handshake,
            HandshakeInputBoundary::Plaintext,
            initial(0),
            1_000,
        )
        .unwrap();
        assert_eq!(first.reduction.state.handshake().handshake_retry(), 1);
        assert_eq!(
            first.reduction.effects,
            vec![LifecycleEffect::SendHandshake {
                token: TOKEN,
                time: 1_000,
                delta: 500,
            }]
        );

        let mut state = first.reduction.state;
        for expected_retry in 2..=HANDSHAKE_RETRY_LIMIT {
            let result = dispatch(
                frame(CgHandshakeHeader::Handshake, 0, 0),
                ClientPhase::Handshake,
                HandshakeInputBoundary::Plaintext,
                state,
                u32::MAX,
            )
            .unwrap();
            assert_eq!(
                result.reduction.state.handshake().handshake_retry(),
                expected_retry
            );
            assert_eq!(result.reduction.state.phase(), ClientPhase::Handshake);
            assert_eq!(result.reduction.effects.len(), 1);
            state = result.reduction.state;
        }

        let closed = dispatch(
            frame(CgHandshakeHeader::Handshake, 0, 0),
            ClientPhase::Handshake,
            HandshakeInputBoundary::Plaintext,
            state,
            u32::MAX,
        )
        .unwrap();
        assert_eq!(closed.reduction.state.phase(), ClientPhase::Close);
        let effects = closed.reduction.effects;
        assert_eq!(effects, vec![set_close(LifecycleInputBoundary::Plaintext)]);
        assert!(
            encode_lifecycle_effects(&effects).is_empty(),
            "the retry limit closes silently, like every other handshake refusal"
        );
    }

    #[test]
    fn post_handshake_time_sync_ack_and_bad_token_are_distinct() {
        let lifecycle = post_login(100);
        let accepted = dispatch(
            frame(CgHandshakeHeader::TimeSync, 1_000, 0),
            ClientPhase::Login,
            HandshakeInputBoundary::DecryptedLegacyTea,
            lifecycle,
            1_010,
        )
        .unwrap();
        assert_eq!(accepted.reduction.state.phase(), ClientPhase::Login);
        assert_eq!(
            accepted.reduction.effects,
            vec![LifecycleEffect::SendTimeSyncAck]
        );
        assert_eq!(
            encode_lifecycle_effects(&accepted.reduction.effects),
            vec![vec![0xfc]]
        );

        let rejected = dispatch(
            frame_with_token(CgHandshakeHeader::TimeSync, TOKEN ^ 1, 1_000, 0),
            ClientPhase::Login,
            HandshakeInputBoundary::DecryptedLegacyTea,
            lifecycle,
            1_010,
        )
        .unwrap();
        assert_eq!(rejected.reduction.state.phase(), ClientPhase::Close);
        let effects = rejected.reduction.effects;
        assert_eq!(
            effects,
            vec![set_close(LifecycleInputBoundary::LegacyTea)],
            "the close keeps the TEA the descriptor had"
        );
        assert!(encode_lifecycle_effects(&effects).is_empty());
    }

    #[test]
    fn post_handshake_handshake_header_is_consumed_without_close_even_with_bad_token() {
        let lifecycle = post_login(100);
        let result = dispatch(
            frame_with_token(CgHandshakeHeader::Handshake, TOKEN ^ 1, 1_000, 0),
            ClientPhase::Login,
            HandshakeInputBoundary::DecryptedLegacyTea,
            lifecycle,
            1_010,
        )
        .unwrap();
        assert_eq!(result.reduction.state, lifecycle);
        assert!(result.reduction.effects.is_empty());
    }

    #[test]
    fn auth_phase_ignores_both_handshake_headers() {
        let auth = initial_auth(100)
            .on_handshake(
                CgInboundHandshake::new(CgHandshakeHeader::Handshake, TOKEN, 100, 0),
                100,
            )
            .unwrap()
            .state;
        for header in [CgHandshakeHeader::Handshake, CgHandshakeHeader::TimeSync] {
            let result = dispatch(
                frame(header, 1_000, 0),
                ClientPhase::Auth,
                HandshakeInputBoundary::DecryptedLegacyTea,
                auth,
                1_010,
            )
            .unwrap();
            assert_eq!(result.reduction.state, auth);
            assert!(result.reduction.effects.is_empty());
        }
    }

    #[test]
    fn later_game_phases_preserve_outer_phase_for_time_sync() {
        let login = post_login(100);
        for target in [
            PostHandshakePhase::Select,
            PostHandshakePhase::Loading,
            PostHandshakePhase::Game,
            PostHandshakePhase::Dead,
        ] {
            let state = login.transition_to(target).unwrap().state;
            let result = dispatch(
                frame(CgHandshakeHeader::TimeSync, 1_000, 0),
                target.client_phase(),
                HandshakeInputBoundary::DecryptedLegacyTea,
                state,
                1_010,
            )
            .unwrap();
            assert_eq!(result.reduction.state.phase(), target.client_phase());
            assert_eq!(
                result.reduction.effects,
                vec![LifecycleEffect::SendTimeSyncAck]
            );
        }
    }

    #[test]
    fn closed_phase_precedes_header_and_boundary_checks() {
        let lifecycle = initial(100).close().state;
        let malformed = ClientFrame::new(0x99, vec![1, 2]);
        assert_eq!(
            dispatch(
                malformed,
                ClientPhase::Handshake,
                HandshakeInputBoundary::Plaintext,
                lifecycle,
                100,
            ),
            Err(HandshakeDispatchError::PhaseClosed {
                session_phase: ClientPhase::Handshake,
                lifecycle_phase: ClientPhase::Close,
            })
        );
        assert_eq!(
            dispatch(
                frame(CgHandshakeHeader::Handshake, 100, 0),
                ClientPhase::Close,
                HandshakeInputBoundary::Plaintext,
                initial(100),
                100,
            ),
            Err(HandshakeDispatchError::PhaseClosed {
                session_phase: ClientPhase::Close,
                lifecycle_phase: ClientPhase::Handshake,
            })
        );
    }

    #[test]
    fn phase_and_boundary_mismatches_are_rejected_explicitly() {
        let lifecycle = initial(100);
        let input = frame(CgHandshakeHeader::Handshake, 100, 0);
        assert_eq!(
            dispatch(
                input.clone(),
                ClientPhase::Login,
                HandshakeInputBoundary::DecryptedLegacyTea,
                lifecycle,
                100,
            ),
            Err(HandshakeDispatchError::PhaseMismatch {
                session_phase: ClientPhase::Login,
                lifecycle_phase: ClientPhase::Handshake,
            })
        );
        assert_eq!(
            dispatch(
                input.clone(),
                ClientPhase::Handshake,
                HandshakeInputBoundary::DecryptedLegacyTea,
                lifecycle,
                100,
            ),
            Err(HandshakeDispatchError::InputBoundaryMismatch {
                phase: ClientPhase::Handshake,
                expected: HandshakeInputBoundary::Plaintext,
                actual: HandshakeInputBoundary::DecryptedLegacyTea,
            })
        );
        assert_eq!(
            dispatch(
                input,
                ClientPhase::Handshake,
                HandshakeInputBoundary::Plaintext,
                post_login(100),
                100,
            ),
            Err(HandshakeDispatchError::PhaseMismatch {
                session_phase: ClientPhase::Handshake,
                lifecycle_phase: ClientPhase::Login,
            })
        );
    }

    #[test]
    fn malformed_known_frame_sizes_are_rejected_before_decode() {
        for header in [CgHandshakeHeader::Handshake, CgHandshakeHeader::TimeSync] {
            for (payload_len, actual) in [(11, 12), (13, 14)] {
                assert_eq!(
                    dispatch(
                        ClientFrame::new(header.value(), vec![0; payload_len]),
                        ClientPhase::Handshake,
                        HandshakeInputBoundary::Plaintext,
                        initial(100),
                        100,
                    ),
                    Err(HandshakeDispatchError::Protocol(
                        ClientFrameError::LengthMismatch {
                            header: header.value(),
                            expected: 13,
                            actual,
                        }
                    ))
                );
            }
        }
    }

    #[test]
    fn all_non_handshake_headers_are_explicitly_unsupported() {
        let lifecycle = initial(100);
        for (header, payload_len) in [
            (0, 0),
            (0x01, 48),
            (0x03, 3),
            (0x70, 6),
            (0x99, 6),
            (0xfe, 0),
        ] {
            assert_eq!(
                dispatch(
                    ClientFrame::new(header, vec![0; payload_len]),
                    ClientPhase::Handshake,
                    HandshakeInputBoundary::Plaintext,
                    lifecycle,
                    100,
                ),
                Err(HandshakeDispatchError::UnsupportedHeader {
                    phase: ClientPhase::Handshake,
                    header,
                })
            );
        }
    }

    #[test]
    fn initial_time_sync_is_reduced_to_close_without_wire_effect() {
        let result = dispatch(
            frame(CgHandshakeHeader::TimeSync, 100, 0),
            ClientPhase::Handshake,
            HandshakeInputBoundary::Plaintext,
            initial(100),
            100,
        )
        .unwrap();
        assert_eq!(result.reduction.state.phase(), ClientPhase::Close);
        let effects = result.reduction.effects;
        assert_eq!(effects, vec![set_close(LifecycleInputBoundary::Plaintext)]);
        assert!(encode_lifecycle_effects(&effects).is_empty());
    }
}
