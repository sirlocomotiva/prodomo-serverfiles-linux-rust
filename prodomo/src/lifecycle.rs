//! Transport-free client lifecycle state adapter.
//!
//! This module composes the source-verified handshake and heartbeat reducers
//! into one descriptor-shaped state boundary. It preserves the legacy order
//! of `SetPhase`, handshake, ping, PONG, and close effects, and records the
//! handshake timestamp changed by the heartbeat `SendHandshake` call.
//! It also models the outer `Login`/`Select`/`Loading`/`Game`/`Dead` phase
//! family without changing the handshake reducer's internal phase.
//! It does not open a socket, decrypt TEA, install a key, or execute gameplay.
//! A transport adapter must still apply the effects and enforce both the
//! output and input boundaries returned by each `SetPhase` effect.

use std::error::Error;
use std::fmt;

use protocol::cg_handshake::CgInboundHandshake;
use protocol::gc::{GcHandshake, GcPhase, GcPing};

use crate::client_session::ClientPhase;
use crate::handshake::{
    reduce as reduce_handshake, HandshakeEffect, HandshakeInputBoundary, HandshakePhase,
    HandshakeServerKind, HandshakeState,
};
use crate::heartbeat::{HeartbeatEffect, HeartbeatState};

/// Input boundary selected by the legacy descriptor phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleInputBoundary {
    /// The descriptor reads plaintext before the handshake completes.
    Plaintext,
    /// The descriptor would use its legacy TEA path after the handshake.
    ///
    /// This is a state boundary only. The module does not implement TEA.
    LegacyTea,
}

/// A post-handshake client phase that can be installed by the transport-free
/// lifecycle adapter.
///
/// The source descriptor uses the numeric values 3 through 6 for these phases
/// (`Select`, `Loading`, `Game`, and `Dead`). The adapter deliberately does
/// not include gameplay or character state; it only models the descriptor
/// phase and its input boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostHandshakePhase {
    /// Character selection phase.
    Select,
    /// Character loading phase.
    Loading,
    /// In-game phase.
    Game,
    /// Dead-character phase.
    Dead,
}

impl PostHandshakePhase {
    /// Return the corresponding legacy descriptor phase.
    #[must_use]
    pub const fn client_phase(self) -> ClientPhase {
        match self {
            Self::Select => ClientPhase::Select,
            Self::Loading => ClientPhase::Loading,
            Self::Game => ClientPhase::Game,
            Self::Dead => ClientPhase::Dead,
        }
    }

    /// Return the source `EPhase` numeric value.
    #[must_use]
    pub const fn legacy_value(self) -> u8 {
        self.client_phase().legacy_value()
    }
}

/// One source-ordered effect produced by the lifecycle adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEffect {
    /// Apply the descriptor phase and select its input boundary.
    ///
    /// `DESC::SetPhase` writes the `GC_PHASE` record before it enables the
    /// new phase's TEA boundary. A caller must therefore use
    /// `output_boundary` for the phase record and only then apply
    /// `input_boundary` to the next input read.
    SetPhase {
        /// Descriptor phase to install.
        phase: ClientPhase,
        /// Boundary used to write the phase record before the phase switch.
        output_boundary: LifecycleInputBoundary,
        /// Boundary the caller must enforce before reading subsequent input.
        input_boundary: LifecycleInputBoundary,
    },
    /// Send a full legacy GC handshake record.
    SendHandshake {
        /// Opaque descriptor token.
        token: u32,
        /// Descriptor clock value.
        time: u32,
        /// Signed correction value.
        delta: i32,
    },
    /// Send the one-byte GC time-sync acknowledgement.
    SendTimeSyncAck,
    /// Send the one-byte GC ping record.
    SendPing,
    /// Apply the descriptor PONG flag.
    SetPong {
        /// New acknowledgement value.
        acknowledged: bool,
    },
    /// Apply the terminal descriptor close transition.
    ///
    /// Close is control-only in the legacy packet path and projects no wire
    /// bytes.
    Close,
}

/// A failure when an externally supplied event cannot be handled in the
/// current descriptor phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleError {
    /// The descriptor is already closed and consumes no input.
    Closed,
    /// PONG is not handled by the selected internal descriptor phase.
    PongUnsupported {
        /// Current descriptor phase.
        phase: ClientPhase,
    },
    /// The handshake record cannot be handled by the current phase.
    HandshakeUnsupported {
        /// Current descriptor phase.
        phase: ClientPhase,
    },
    /// A post-handshake phase transition was requested from a phase that
    /// cannot perform it. The state is left unchanged.
    PhaseTransitionRejected {
        /// Current descriptor phase.
        from: ClientPhase,
        /// Requested descriptor phase.
        to: ClientPhase,
    },
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("client lifecycle is closed"),
            Self::PongUnsupported { phase } => {
                write!(formatter, "PONG is not handled in client phase {phase:?}")
            }
            Self::HandshakeUnsupported { phase } => write!(
                formatter,
                "handshake is not handled in client phase {phase:?}"
            ),
            Self::PhaseTransitionRejected { from, to } => write!(
                formatter,
                "phase transition from {from:?} to {to:?} is rejected"
            ),
        }
    }
}

impl Error for LifecycleError {}

/// Pure descriptor-shaped state assembled from the source-backed reducers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientLifecycle {
    phase: ClientPhase,
    handshake: HandshakeState,
    heartbeat: HeartbeatState,
}

impl ClientLifecycle {
    /// Start the source `DESC::Setup` sequence.
    ///
    /// The returned effects are ordered as `SetPhase(PHASE_HANDSHAKE)` then
    /// `SendHandshake`. PONG starts acknowledged and the input boundary is
    /// plaintext.
    #[must_use]
    pub fn start(
        expected_token: u32,
        server_kind: HandshakeServerKind,
        now: u32,
    ) -> LifecycleReduction {
        let handshake = HandshakeState::start(expected_token, server_kind, now);
        let state = Self {
            phase: ClientPhase::Handshake,
            handshake: handshake.state,
            heartbeat: HeartbeatState::new(),
        };
        LifecycleReduction {
            state,
            // `DESC::Setup` begins with the plaintext descriptor boundary.
            // The handshake phase record is written before any later TEA
            // boundary is installed.
            effects: handshake
                .effects
                .iter()
                .map(|effect| map_handshake_effect(effect, LifecycleInputBoundary::Plaintext))
                .collect(),
        }
    }

    /// Return the current descriptor phase.
    #[must_use]
    pub const fn phase(self) -> ClientPhase {
        self.phase
    }

    /// Return the input boundary implied by the current phase.
    #[must_use]
    pub const fn input_boundary(self) -> LifecycleInputBoundary {
        if self.phase.legacy_tea_encrypted() {
            LifecycleInputBoundary::LegacyTea
        } else {
            LifecycleInputBoundary::Plaintext
        }
    }

    /// Return whether input is currently behind the legacy TEA boundary.
    #[must_use]
    pub fn tea_required(self) -> bool {
        matches!(self.input_boundary(), LifecycleInputBoundary::LegacyTea)
    }

    /// Borrow the pure handshake state.
    #[must_use]
    pub const fn handshake(self) -> HandshakeState {
        self.handshake
    }

    /// Borrow the pure heartbeat state.
    #[must_use]
    pub const fn heartbeat(self) -> HeartbeatState {
        self.heartbeat
    }

    /// Set the source admin-mode heartbeat bypass.
    pub fn set_admin_mode(&mut self, enabled: bool) {
        self.heartbeat.set_admin_mode(enabled);
    }

    /// Apply one already decoded handshake or time-sync record.
    ///
    /// The reducer preserves source retry, timing, token, and phase behavior.
    /// The initial handshake changes the outer phase to `Login` or `Auth`.
    /// Once the outer phase has advanced to `Select`, `Loading`, `Game`, or
    /// `Dead`, the handshake reducer remains in its internal `Login` phase;
    /// a time-sync result must not regress that outer descriptor phase.
    ///
    /// # Errors
    ///
    /// Returns [`LifecycleError::Closed`] after the close transition and
    /// [`LifecycleError::HandshakeUnsupported`] when the internal handshake
    /// reducer cannot process a record in the current state.
    pub fn on_handshake(
        self,
        packet: CgInboundHandshake,
        now: u32,
    ) -> Result<LifecycleReduction, LifecycleError> {
        if self.phase == ClientPhase::Close {
            return Err(LifecycleError::Closed);
        }
        if !matches!(
            self.handshake.phase(),
            HandshakePhase::Handshake | HandshakePhase::Login | HandshakePhase::Auth
        ) {
            return Err(LifecycleError::HandshakeUnsupported { phase: self.phase });
        }

        // Keep the old boundary beside the reduction. It is the boundary that
        // `DESC::SetPhase` uses to write GC_PHASE before switching processors.
        let output_boundary = self.input_boundary();
        let reduction = reduce_handshake(self.handshake, packet, now);
        let phase = if reduction.state.phase() == HandshakePhase::Close {
            ClientPhase::Close
        } else if self.phase == ClientPhase::Handshake {
            // Only the initial handshake reducer result selects the first
            // post-handshake phase. Later post-handshake analyzers reuse the
            // internal Login phase, so preserve the outer phase instead.
            client_phase(reduction.state.phase())
        } else {
            self.phase
        };
        let effects = reduction
            .effects
            .iter()
            .map(|effect| map_handshake_effect(effect, output_boundary))
            .collect();
        Ok(LifecycleReduction {
            state: Self {
                phase,
                handshake: reduction.state,
                heartbeat: self.heartbeat,
            },
            effects,
        })
    }

    /// Apply an explicit post-handshake descriptor phase transition.
    ///
    /// The source calls `SetPhase` when login succeeds, a player is loaded,
    /// the client enters the game, or a character dies. The pure adapter
    /// accepts a target only while the outer descriptor is already in the
    /// post-handshake family (`Login`, `Select`, `Loading`, `Game`, or
    /// `Dead`). It never changes the internal handshake reducer phase.
    ///
    /// `DESC::SetPhase` writes the phase record through the current boundary
    /// before installing the target input processor and TEA state. Therefore
    /// a later transition carries `LegacyTea` as both `output_boundary` and
    /// `input_boundary`.
    ///
    /// # Errors
    ///
    /// Returns [`LifecycleError::Closed`] for a closed lifecycle and
    /// [`LifecycleError::PhaseTransitionRejected`] without mutating the state
    /// for handshake, auth, or internal descriptor phases.
    pub fn transition_to(
        self,
        target: PostHandshakePhase,
    ) -> Result<LifecycleReduction, LifecycleError> {
        if self.phase == ClientPhase::Close {
            return Err(LifecycleError::Closed);
        }

        let target_phase = target.client_phase();
        if !is_post_handshake_phase(self.phase) {
            return Err(LifecycleError::PhaseTransitionRejected {
                from: self.phase,
                to: target_phase,
            });
        }

        let output_boundary = self.input_boundary();
        let input_boundary = if target_phase.legacy_tea_encrypted() {
            LifecycleInputBoundary::LegacyTea
        } else {
            LifecycleInputBoundary::Plaintext
        };
        Ok(LifecycleReduction {
            state: Self {
                phase: target_phase,
                handshake: self.handshake,
                heartbeat: self.heartbeat,
            },
            effects: vec![LifecycleEffect::SetPhase {
                phase: target_phase,
                output_boundary,
                input_boundary,
            }],
        })
    }

    /// Apply a PONG acknowledgement.
    ///
    /// PONG updates descriptor state only. It does not generate a response.
    ///
    /// # Errors
    ///
    /// Returns [`LifecycleError::Closed`] after close and
    /// [`LifecycleError::PongUnsupported`] for DB/P2P/connecting phases.
    pub fn on_pong(self) -> Result<LifecycleReduction, LifecycleError> {
        if self.phase == ClientPhase::Close {
            return Err(LifecycleError::Closed);
        }
        if !self.phase.accepts_pong() {
            return Err(LifecycleError::PongUnsupported { phase: self.phase });
        }
        let reduction = self.heartbeat.on_pong();
        Ok(LifecycleReduction {
            state: Self {
                heartbeat: reduction.state,
                ..self
            },
            effects: reduction
                .effects
                .into_iter()
                .map(map_heartbeat_effect)
                .collect(),
        })
    }

    /// Apply one caller-timed heartbeat tick.
    ///
    /// The successful path preserves the source order: ping, clear PONG, then
    /// zero-delta handshake. The clear-PONG path closes the descriptor and
    /// emits no later handshake. A heartbeat handshake updates the pure
    /// handshake sent timestamp and `handshaking` flag, just like
    /// `DESC::SendHandshake`.
    #[must_use]
    pub fn on_tick(self, now: u32) -> LifecycleReduction {
        if self.phase == ClientPhase::Close || self.heartbeat.is_closed() {
            return no_op(self);
        }

        let reduction = self.heartbeat.on_tick(self.handshake.expected_token(), now);
        let mut state = Self {
            heartbeat: reduction.state,
            ..self
        };
        if reduction.state.is_closed() {
            state.phase = ClientPhase::Close;
            state.handshake = self.handshake.close();
        } else if reduction
            .effects
            .iter()
            .any(|effect| matches!(effect, HeartbeatEffect::SendHandshake { .. }))
        {
            state.handshake = self.handshake.record_sent_handshake(now);
        }
        LifecycleReduction {
            state,
            effects: reduction
                .effects
                .into_iter()
                .map(map_heartbeat_effect)
                .collect(),
        }
    }

    /// Apply the source `SetPhase(PHASE_CLOSE)` transition explicitly.
    #[must_use]
    pub fn close(self) -> LifecycleReduction {
        if self.phase == ClientPhase::Close {
            return no_op(self);
        }
        LifecycleReduction {
            state: Self {
                phase: ClientPhase::Close,
                handshake: self.handshake.close(),
                heartbeat: self.heartbeat,
            },
            effects: vec![LifecycleEffect::Close],
        }
    }

    /// `DESC::SetPhase(PHASE_CLOSE)`, as opposed to [`ClientLifecycle::close`].
    ///
    /// The two are not the same event and they are not interchangeable. `close` is a socket
    /// teardown. `SetPhase(PHASE_CLOSE)` assigns the phase and installs the close input
    /// processor, and it is **also** byte-free: `SetPhase` sets `m_iPhase` before it calls
    /// `Packet`, and `Packet` returns immediately for `PHASE_CLOSE`
    /// (`G/desc.cpp:495-500` and `:397-401`). Use `close_phase` for a refusal legacy reports
    /// with `SetPhase(PHASE_CLOSE)`, so the phase change is recorded where it happened.
    ///
    /// The input boundary does not change. The `PHASE_CLOSE` arm of the `switch` assigns
    /// `m_pInputProcessor` and never touches `m_bEncrypted` (`G/desc.cpp:504-507` against
    /// `:520-537`), so input decryption keeps whatever it was doing. The close processor is
    /// `CInputClose`, whose `Analyze` returns `m_iBufferLeft` (`G/input.h:35-41`) and therefore
    /// consumes nothing, which is how the descriptor learns to close.
    ///
    /// Nothing is written, so the two boundaries are the same value and the caller cannot tell
    /// them apart. They are kept separate so the reduction still says what it did.
    ///
    /// Idempotent: closing an already-closed lifecycle produces no effects.
    #[must_use]
    pub fn close_phase(self) -> LifecycleReduction {
        if self.phase == ClientPhase::Close {
            return no_op(self);
        }
        let boundary = self.input_boundary();
        LifecycleReduction {
            state: Self {
                phase: ClientPhase::Close,
                handshake: self.handshake.close(),
                heartbeat: self.heartbeat,
            },
            effects: vec![LifecycleEffect::SetPhase {
                phase: ClientPhase::Close,
                output_boundary: boundary,
                input_boundary: boundary,
            }],
        }
    }
}

/// State and source-ordered effects returned by one lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleReduction {
    /// State after the event.
    pub state: ClientLifecycle,
    /// Effects in the exact order required by the legacy descriptor.
    pub effects: Vec<LifecycleEffect>,
}

fn no_op(state: ClientLifecycle) -> LifecycleReduction {
    LifecycleReduction {
        state,
        effects: Vec::new(),
    }
}

fn is_post_handshake_phase(phase: ClientPhase) -> bool {
    matches!(
        phase,
        ClientPhase::Login
            | ClientPhase::Select
            | ClientPhase::Loading
            | ClientPhase::Game
            | ClientPhase::Dead
    )
}

fn client_phase(phase: HandshakePhase) -> ClientPhase {
    match phase {
        HandshakePhase::Handshake => ClientPhase::Handshake,
        HandshakePhase::Login => ClientPhase::Login,
        HandshakePhase::Auth => ClientPhase::Auth,
        HandshakePhase::Close => ClientPhase::Close,
    }
}

fn input_boundary(boundary: HandshakeInputBoundary) -> LifecycleInputBoundary {
    match boundary {
        HandshakeInputBoundary::Plaintext => LifecycleInputBoundary::Plaintext,
        HandshakeInputBoundary::LegacyTea => LifecycleInputBoundary::LegacyTea,
    }
}

fn map_handshake_effect(
    effect: &HandshakeEffect,
    output_boundary: LifecycleInputBoundary,
) -> LifecycleEffect {
    match effect {
        HandshakeEffect::SetPhase {
            phase,
            input_boundary: boundary,
        } => LifecycleEffect::SetPhase {
            phase: client_phase(*phase),
            output_boundary,
            input_boundary: input_boundary(*boundary),
        },
        HandshakeEffect::SendHandshake { token, time, delta } => LifecycleEffect::SendHandshake {
            token: *token,
            time: *time,
            delta: *delta,
        },
        HandshakeEffect::SendTimeSyncAck => LifecycleEffect::SendTimeSyncAck,
        // `HandshakeEffect::Close` is legacy's `SetPhase(PHASE_CLOSE)`, not a socket teardown,
        // so it is a phase change; the teardown is [`LifecycleEffect::Close`], which
        // [`ClientLifecycle::close`] emits. The phase change is silent, because `SetPhase`
        // assigns `m_iPhase` before it calls `Packet` and `Packet` returns at once for
        // `PHASE_CLOSE` (`G/desc.cpp:495-500` against `:397-401`). It leaves the input
        // boundary as it found it, because the `PHASE_CLOSE` arm assigns `m_pInputProcessor`
        // and never touches `m_bEncrypted` (`G/desc.cpp:504-507`).
        HandshakeEffect::Close => LifecycleEffect::SetPhase {
            phase: ClientPhase::Close,
            output_boundary,
            input_boundary: output_boundary,
        },
    }
}

fn map_heartbeat_effect(effect: HeartbeatEffect) -> LifecycleEffect {
    match effect {
        HeartbeatEffect::SendPing => LifecycleEffect::SendPing,
        HeartbeatEffect::SetPong { acknowledged } => LifecycleEffect::SetPong { acknowledged },
        HeartbeatEffect::SendHandshake { token, time, delta } => {
            LifecycleEffect::SendHandshake { token, time, delta }
        }
        HeartbeatEffect::Close => LifecycleEffect::Close,
    }
}

/// Project one lifecycle effect into its raw legacy wire record.
///
/// State-only effects and a bare [`LifecycleEffect::Close`] produce no bytes. A
/// [`LifecycleEffect::SetPhase`] produces its `GC_PHASE` record for every phase **except**
/// `PHASE_CLOSE`.
///
/// ```text
/// void DESC::SetPhase(int _phase)
/// {
///     m_iPhase = _phase;                      // assigned before the write
///     TPacketGCPhase pack;
///     pack.header = HEADER_GC_PHASE;
///     pack.phase = _phase;
///     Packet(&pack, sizeof(TPacketGCPhase));  // and the write path guards on that field
///     switch (m_iPhase)
///     {
///         case PHASE_CLOSE:
///             m_pInputProcessor = &m_inputClose;   // m_bEncrypted is not touched
///             break;
/// ```
///
/// The `Packet` side of the pair is what decides it:
///
/// ```text
/// void DESC::Packet(const void* c_pvData, int iSize)   // G/desc.cpp:397
/// {
///     assert(iSize > 0);
///     if (m_iPhase == PHASE_CLOSE)
///         return;
/// ```
///
/// So the record is built and then dropped, and a close is byte-free. That is the whole reason:
/// a closed descriptor must answer nothing, including anything still in flight. The two
/// effects remain distinct because `SetPhase(PHASE_CLOSE)` and a teardown are different events
/// that happen to agree on the wire, and a refusal has to be recorded as having gone through
/// `SetPhase`.
/// The function does not merge records or apply a socket/encryption boundary.
#[must_use]
pub fn encode_lifecycle_effect(effect: &LifecycleEffect) -> Vec<u8> {
    match effect {
        // `SetPhase` assigns `m_iPhase` on its first line and only then calls `Packet`, and
        // `Packet` returns at once for `PHASE_CLOSE` (`G/desc.cpp:495-500` and `:397-401`). So
        // the close record is built and then dropped, and the phase close is byte-free. This
        // is not an omission in the adapter: a descriptor is write-silent from the moment its
        // phase becomes `PHASE_CLOSE`.
        LifecycleEffect::SetPhase { phase, .. } if *phase == ClientPhase::Close => Vec::new(),
        LifecycleEffect::SetPhase { phase, .. } => GcPhase::new(phase.legacy_value()).encode(),
        LifecycleEffect::SendHandshake { token, time, delta } => {
            GcHandshake::new(*token, *time, *delta).encode()
        }
        LifecycleEffect::SendTimeSyncAck => vec![crate::handshake::HEADER_GC_TIME_SYNC],
        LifecycleEffect::SendPing => GcPing.encode(),
        LifecycleEffect::SetPong { .. } | LifecycleEffect::Close => Vec::new(),
    }
}

/// Project effects in order, dropping state-only and close effects.
#[must_use]
pub fn encode_lifecycle_effects(effects: &[LifecycleEffect]) -> Vec<Vec<u8>> {
    effects
        .iter()
        .map(encode_lifecycle_effect)
        .filter(|frame| !frame.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Legacy's `SetPhase(PHASE_CLOSE)`, which is a phase change with **no** record: it assigns
    /// `m_iPhase` and then calls `Packet`, which returns at once for `PHASE_CLOSE`
    /// (`G/desc.cpp:495-500` and `:397-401`).
    fn close_phase(boundary: LifecycleInputBoundary) -> LifecycleEffect {
        LifecycleEffect::SetPhase {
            phase: ClientPhase::Close,
            output_boundary: boundary,
            input_boundary: boundary,
        }
    }

    /// The boundary a close transition leaves installed.
    fn input_boundary_from(effects: &[LifecycleEffect]) -> LifecycleInputBoundary {
        match effects {
            [LifecycleEffect::SetPhase { input_boundary, .. }] => *input_boundary,
            other => panic!("expected one SetPhase effect, got {other:?}"),
        }
    }
    use crate::handshake::HANDSHAKE_RETRY_LIMIT;
    use protocol::cg_handshake::CgHandshakeHeader;

    const TOKEN: u32 = 0x1234_5678;

    fn packet(header: CgHandshakeHeader, time: u32, delta: i32) -> CgInboundHandshake {
        CgInboundHandshake::new(header, TOKEN, time, delta)
    }

    #[test]
    fn setup_preserves_phase_then_handshake_and_initial_pong_state() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        assert_eq!(start.state.phase(), ClientPhase::Handshake);
        assert_eq!(
            start.state.input_boundary(),
            LifecycleInputBoundary::Plaintext
        );
        assert!(!start.state.tea_required());
        assert!(start.state.heartbeat().pong_acknowledged());
        assert_eq!(
            start.effects,
            vec![
                LifecycleEffect::SetPhase {
                    phase: ClientPhase::Handshake,
                    output_boundary: LifecycleInputBoundary::Plaintext,
                    input_boundary: LifecycleInputBoundary::Plaintext,
                },
                LifecycleEffect::SendHandshake {
                    token: TOKEN,
                    time: 100,
                    delta: 0,
                },
            ]
        );
        assert_eq!(
            encode_lifecycle_effects(&start.effects),
            vec![
                vec![0xfd, 1],
                vec![0xff, 0x78, 0x56, 0x34, 0x12, 0x64, 0, 0, 0, 0, 0, 0, 0,],
            ]
        );
    }

    #[test]
    fn accepted_handshake_moves_to_legacy_tea_login_in_source_order() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        let accepted = start
            .state
            .on_handshake(packet(CgHandshakeHeader::Handshake, 90, 5), 100)
            .unwrap();
        assert_eq!(accepted.state.phase(), ClientPhase::Login);
        assert_eq!(
            accepted.state.input_boundary(),
            LifecycleInputBoundary::LegacyTea
        );
        assert!(accepted.state.tea_required());
        assert_eq!(
            accepted.effects,
            vec![LifecycleEffect::SetPhase {
                phase: ClientPhase::Login,
                output_boundary: LifecycleInputBoundary::Plaintext,
                input_boundary: LifecycleInputBoundary::LegacyTea,
            }]
        );
        assert_eq!(
            encode_lifecycle_effects(&accepted.effects),
            vec![vec![0xfd, 2]]
        );
    }

    #[test]
    fn auth_server_selects_auth_phase() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Auth, 100);
        let accepted = start
            .state
            .on_handshake(packet(CgHandshakeHeader::Handshake, 100, 0), 100)
            .unwrap();
        assert_eq!(accepted.state.phase(), ClientPhase::Auth);
        assert!(accepted.state.tea_required());
        assert_eq!(
            encode_lifecycle_effects(&accepted.effects),
            vec![vec![0xfd, 10]]
        );
    }

    #[test]
    fn post_handshake_targets_emit_phase_with_old_and_new_tea_boundaries() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        let login = start
            .state
            .on_handshake(packet(CgHandshakeHeader::Handshake, 90, 5), 100)
            .unwrap()
            .state;

        let targets = [
            (PostHandshakePhase::Select, ClientPhase::Select, 3),
            (PostHandshakePhase::Loading, ClientPhase::Loading, 4),
            (PostHandshakePhase::Game, ClientPhase::Game, 5),
            (PostHandshakePhase::Dead, ClientPhase::Dead, 6),
        ];
        for (target, expected_phase, legacy_value) in targets {
            let result = login.transition_to(target).unwrap();
            assert_eq!(result.state.phase(), expected_phase);
            assert_eq!(target.legacy_value(), legacy_value);
            assert_eq!(
                result.effects,
                vec![LifecycleEffect::SetPhase {
                    phase: expected_phase,
                    // The old encrypted boundary writes GC_PHASE first.
                    output_boundary: LifecycleInputBoundary::LegacyTea,
                    // The target processor is then selected behind TEA.
                    input_boundary: LifecycleInputBoundary::LegacyTea,
                }]
            );
            assert_eq!(
                encode_lifecycle_effects(&result.effects),
                vec![vec![0xfd, legacy_value]]
            );
        }
    }

    #[test]
    fn post_handshake_time_sync_preserves_each_outer_phase_and_sends_only_ack() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        let login = start
            .state
            .on_handshake(packet(CgHandshakeHeader::Handshake, 100, 0), 100)
            .unwrap()
            .state;

        for target in [
            PostHandshakePhase::Select,
            PostHandshakePhase::Loading,
            PostHandshakePhase::Game,
            PostHandshakePhase::Dead,
        ] {
            let phase_state = login.transition_to(target).unwrap().state;
            let result = phase_state
                .on_handshake(packet(CgHandshakeHeader::TimeSync, 2_000, 0), 2_010)
                .unwrap();
            assert_eq!(result.state.phase(), target.client_phase());
            assert_eq!(result.state.handshake().phase(), HandshakePhase::Login);
            assert_eq!(result.effects, vec![LifecycleEffect::SendTimeSyncAck]);
            assert_eq!(encode_lifecycle_effects(&result.effects), vec![vec![0xfc]]);
        }
    }

    #[test]
    fn post_handshake_wrong_handshake_header_is_consumed_without_closing() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        let login = start
            .state
            .on_handshake(packet(CgHandshakeHeader::Handshake, 100, 0), 100)
            .unwrap()
            .state;

        for target in [
            PostHandshakePhase::Select,
            PostHandshakePhase::Loading,
            PostHandshakePhase::Game,
            PostHandshakePhase::Dead,
        ] {
            let phase_state = login.transition_to(target).unwrap().state;
            let result = phase_state
                .on_handshake(packet(CgHandshakeHeader::Handshake, 100, 0), 100)
                .unwrap();
            assert_eq!(result.state, phase_state);
            assert!(result.effects.is_empty());
        }
    }

    #[test]
    fn a_post_handshake_wrong_token_closes_through_the_close_phase() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        let login = start
            .state
            .on_handshake(packet(CgHandshakeHeader::Handshake, 100, 0), 100)
            .unwrap()
            .state;

        for target in [
            PostHandshakePhase::Select,
            PostHandshakePhase::Loading,
            PostHandshakePhase::Game,
            PostHandshakePhase::Dead,
        ] {
            let phase_state = login.transition_to(target).unwrap().state;
            let boundary = phase_state.input_boundary();
            let mut bad = packet(CgHandshakeHeader::TimeSync, 2_000, 0);
            bad.token = TOKEN ^ 1;
            let result = phase_state.on_handshake(bad, 2_010).unwrap();
            assert_eq!(result.state.phase(), ClientPhase::Close);
            let effects = result.effects;
            assert_eq!(
                effects,
                vec![close_phase(boundary)],
                "the close keeps the boundary {target:?} had in force"
            );
            assert!(
                encode_lifecycle_effects(&effects).is_empty(),
                "the descriptor is write-silent from the moment its phase becomes PHASE_CLOSE, \
                 so the {target:?} close carries no record"
            );
            assert_eq!(
                input_boundary_from(&effects),
                boundary,
                "SetPhase(PHASE_CLOSE) installs a processor and never touches m_bEncrypted"
            );
        }
    }

    #[test]
    fn post_handshake_transition_rejects_closed_and_pre_handshake_phases() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        let before = start.state;
        assert_eq!(
            before.transition_to(PostHandshakePhase::Select),
            Err(LifecycleError::PhaseTransitionRejected {
                from: ClientPhase::Handshake,
                to: ClientPhase::Select,
            })
        );
        assert_eq!(before, start.state);

        let login = start
            .state
            .on_handshake(packet(CgHandshakeHeader::Handshake, 100, 0), 100)
            .unwrap()
            .state;
        let closed = login.close();
        assert_eq!(
            closed.state.transition_to(PostHandshakePhase::Select),
            Err(LifecycleError::Closed)
        );

        let auth_start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Auth, 100);
        let auth = auth_start
            .state
            .on_handshake(packet(CgHandshakeHeader::Handshake, 100, 0), 100)
            .unwrap()
            .state;
        assert_eq!(
            auth.transition_to(PostHandshakePhase::Select),
            Err(LifecycleError::PhaseTransitionRejected {
                from: ClientPhase::Auth,
                to: ClientPhase::Select,
            })
        );
    }

    #[test]
    fn post_handshake_pong_remains_state_only_in_each_later_phase() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        let login = start
            .state
            .on_handshake(packet(CgHandshakeHeader::Handshake, 100, 0), 100)
            .unwrap()
            .state;

        for target in [
            PostHandshakePhase::Select,
            PostHandshakePhase::Loading,
            PostHandshakePhase::Game,
            PostHandshakePhase::Dead,
        ] {
            let phase_state = login.transition_to(target).unwrap().state;
            // Clear the initially acknowledged PONG flag with one source-order
            // heartbeat tick, then exercise the PONG acknowledgement path.
            let pong = phase_state.on_tick(200).state.on_pong().unwrap();
            assert_eq!(pong.state.phase(), target.client_phase());
            assert!(pong.state.heartbeat().pong_acknowledged());
            assert_eq!(
                pong.effects,
                vec![LifecycleEffect::SetPong { acknowledged: true }]
            );
            assert!(encode_lifecycle_effects(&pong.effects).is_empty());
        }
    }

    #[test]
    fn heartbeat_updates_handshake_timestamp_and_pong_without_wire_response() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        let tick = start.state.on_tick(200);
        assert_eq!(
            tick.effects,
            vec![
                LifecycleEffect::SendPing,
                LifecycleEffect::SetPong {
                    acknowledged: false
                },
                LifecycleEffect::SendHandshake {
                    token: TOKEN,
                    time: 200,
                    delta: 0,
                },
            ]
        );
        assert_eq!(tick.state.handshake().handshake_sent_time(), 200);
        assert!(tick.state.handshake().is_handshaking());
        assert_eq!(
            encode_lifecycle_effects(&tick.effects),
            vec![
                vec![44],
                vec![0xff, 0x78, 0x56, 0x34, 0x12, 0xc8, 0, 0, 0, 0, 0, 0, 0,],
            ]
        );

        let pong = tick.state.on_pong().unwrap();
        assert_eq!(
            pong.effects,
            vec![LifecycleEffect::SetPong { acknowledged: true }]
        );
        assert!(pong.state.heartbeat().pong_acknowledged());
        assert!(encode_lifecycle_effects(&pong.effects).is_empty());

        let close = pong.state.on_tick(300).state.on_tick(400);
        assert_eq!(close.effects, vec![LifecycleEffect::Close]);
        assert!(close.state.phase() == ClientPhase::Close);
        assert!(close.state.heartbeat().is_closed());
        assert!(encode_lifecycle_effects(&close.effects).is_empty());
    }

    /// A phase close and a teardown are different events that agree on the wire: neither
    /// sends anything, because `SetPhase(PHASE_CLOSE)` is dropped by `Packet`. They are kept
    /// apart so a refusal can be recorded as having gone through `SetPhase`.
    #[test]
    fn a_phase_close_and_a_teardown_both_end_silently() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        let login = start
            .state
            .on_handshake(packet(CgHandshakeHeader::Handshake, 100, 0), 100)
            .unwrap()
            .state;

        let teardown = login.close();
        assert_eq!(teardown.state.phase(), ClientPhase::Close);
        assert_eq!(teardown.effects, vec![LifecycleEffect::Close]);
        assert!(
            encode_lifecycle_effects(&teardown.effects).is_empty(),
            "DESC::Destroy writes nothing"
        );

        let phased = login.close_phase();
        assert_eq!(phased.state.phase(), ClientPhase::Close);
        assert_eq!(
            phased.effects,
            vec![close_phase(LifecycleInputBoundary::LegacyTea)]
        );
        assert!(
            encode_lifecycle_effects(&phased.effects).is_empty(),
            "SetPhase builds the GC_PHASE(0) record and Packet drops it, because m_iPhase is \
             already PHASE_CLOSE by then (G/desc.cpp:495-500 against :397-401)"
        );
        assert_eq!(
            input_boundary_from(&phased.effects),
            LifecycleInputBoundary::LegacyTea,
            "the PHASE_CLOSE arm assigns m_pInputProcessor and leaves m_bEncrypted alone"
        );
    }

    /// A close keeps the input boundary the phase had. `SetPhase(PHASE_CLOSE)` assigns
    /// `m_pInputProcessor` and never touches `m_bEncrypted` (`G/desc.cpp:504-507` against
    /// `:520-537`), so a close behind TEA is still a TEA read.
    #[test]
    fn a_close_keeps_the_boundary_the_phase_had() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        let login = start
            .state
            .on_handshake(packet(CgHandshakeHeader::Handshake, 100, 0), 100)
            .unwrap()
            .state;
        for target in [
            PostHandshakePhase::Select,
            PostHandshakePhase::Loading,
            PostHandshakePhase::Game,
            PostHandshakePhase::Dead,
        ] {
            let state = login.transition_to(target).unwrap().state;
            let expected = state.input_boundary();
            let boundary = expected;
            let effects = state.close_phase().effects;
            assert_eq!(effects, vec![close_phase(boundary)], "{target:?}");
        }
    }

    /// Closing twice writes the record once. Legacy's `SetPhase` is unconditional, but a
    /// descriptor that has already stopped reading cannot receive a second one, and the
    /// reducer must not go behind the socket.
    #[test]
    fn a_second_close_phase_writes_nothing() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        let once = start.state.close_phase();
        let twice = once.state.close_phase();
        assert_eq!(twice.state.phase(), ClientPhase::Close);
        assert!(twice.effects.is_empty());
        assert!(encode_lifecycle_effects(&twice.effects).is_empty());
    }

    #[test]
    fn close_is_terminal_and_blocks_input_but_ticks_are_inert() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        let closed = start.state.close();
        assert_eq!(closed.state.phase(), ClientPhase::Close);
        assert_eq!(closed.effects, vec![LifecycleEffect::Close]);
        assert!(encode_lifecycle_effects(&closed.effects).is_empty());
        assert_eq!(
            closed
                .state
                .on_handshake(packet(CgHandshakeHeader::Handshake, 100, 0), 100),
            Err(LifecycleError::Closed)
        );
        assert_eq!(closed.state.on_pong(), Err(LifecycleError::Closed));
        assert!(closed.state.on_tick(200).effects.is_empty());
    }

    #[test]
    fn an_invalid_handshake_closes_through_the_close_phase() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        let mut bad = packet(CgHandshakeHeader::Handshake, 100, 0);
        bad.token = TOKEN ^ 1;
        let result = start.state.on_handshake(bad, 100).unwrap();
        assert_eq!(result.state.phase(), ClientPhase::Close);
        let effects = result.effects;
        let boundary = LifecycleInputBoundary::Plaintext;
        assert_eq!(
            effects,
            vec![close_phase(boundary)],
            "the phase changes, the socket is quiet"
        );
        assert!(
            encode_lifecycle_effects(&effects).is_empty(),
            "a bad token closes without a record"
        );
    }

    #[test]
    fn admin_mode_bypasses_tick_and_preserves_state() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        let mut state = start.state;
        state.set_admin_mode(true);
        let tick = state.on_tick(200);
        assert_eq!(tick.state, state);
        assert!(tick.effects.is_empty());
    }

    #[test]
    fn retry_limit_closes_after_source_number_of_resends() {
        let start = ClientLifecycle::start(TOKEN, HandshakeServerKind::Game, 100);
        let mut state = start.state;
        for _ in 0..=HANDSHAKE_RETRY_LIMIT {
            let result = state
                .on_handshake(packet(CgHandshakeHeader::Handshake, 0, 0), u32::MAX)
                .unwrap();
            state = result.state;
        }
        assert_eq!(state.phase(), ClientPhase::Close);
        assert!(state.handshake().handshake_retry() > HANDSHAKE_RETRY_LIMIT);
    }
}
