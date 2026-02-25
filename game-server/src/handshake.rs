//! Pure state reduction for the source-verified CG handshake boundary.
//!
//! This module models only the descriptor fields and decisions in
//! `server/server/game/input.cpp` and `desc.cpp`. It does not generate tokens,
//! read a clock, frame bytes, open sockets, enable TEA, or call a live
//! descriptor. Effects are returned in legacy order for a later adapter to
//! apply through the existing phase, Close, and TEA barriers. It models the
//! active legacy branch; the alternate improved key-agreement branch remains
//! unimplemented and is not inferred from these effects.

use protocol::cg_handshake::{CgHandshakeHeader, CgInboundHandshake};

/// Maximum number of retry sends allowed during the initial handshake.
///
/// `HANDSHAKE_RETRY_LIMIT` is 32 in `game/desc.h`. The legacy code closes on
/// the next rejected response after 32 sends have already been emitted.
pub const HANDSHAKE_RETRY_LIMIT: u32 = 32;

/// Inclusive timing-bias acceptance bound from `HandshakeProcess`.
pub const HANDSHAKE_BIAS_LIMIT: i32 = 50;

/// Header for the one-byte GC time-sync acknowledgement.
pub const HEADER_GC_TIME_SYNC: u8 = 0xfc;

/// Server role that selects the post-handshake phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeServerKind {
    /// The normal game server transitions to the login phase.
    Game,
    /// The authentication server transitions to the auth phase.
    Auth,
}

/// Descriptor phases reached by this bounded handshake reducer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakePhase {
    /// The plaintext initial handshake phase.
    Handshake,
    /// The normal game's TEA-protected login phase.
    Login,
    /// The auth server's TEA-protected authentication phase.
    Auth,
    /// The terminal closed phase.
    Close,
}

impl HandshakePhase {
    /// Return the numeric legacy `EPhase` value used in `TPacketGCPhase`.
    #[must_use]
    pub const fn legacy_value(self) -> u8 {
        match self {
            Self::Close => 0,
            Self::Handshake => 1,
            Self::Login => 2,
            Self::Auth => 10,
        }
    }
}

/// Input boundary that a later descriptor adapter must preserve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeInputBoundary {
    /// The legacy input processor reads plaintext in the handshake phase.
    Plaintext,
    /// Login and auth input is behind the legacy TEA boundary.
    LegacyTea,
}

/// A source-ordered effect returned by the pure reducer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeEffect {
    /// Apply `DESC::SetPhase` with the explicit input boundary.
    ///
    /// This module does not perform the transition. In particular, an adapter
    /// must not route post-handshake plaintext through a login/auth phase.
    SetPhase {
        /// Source descriptor phase.
        phase: HandshakePhase,
        /// Boundary that must be established by the descriptor adapter.
        input_boundary: HandshakeInputBoundary,
    },
    /// Send a full GC handshake with the current token, time, and correction.
    SendHandshake {
        /// Expected client token.
        token: u32,
        /// Current descriptor time.
        time: u32,
        /// Computed signed correction.
        delta: i32,
    },
    /// Send the one-byte GC `0xfc` time-sync acknowledgement.
    SendTimeSyncAck,
    /// Apply the complete legacy `DESC::SetPhase(PHASE_CLOSE)` operation.
    ///
    /// A live adapter must preserve the GC phase-frame barrier and then
    /// install the close input processor. This is a combined effect, not a
    /// socket close performed by this pure reducer.
    Close,
}

/// Pure handshake state mirrored from the active descriptor fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandshakeState {
    expected_token: u32,
    server_kind: HandshakeServerKind,
    phase: HandshakePhase,
    handshake_sent_time: u32,
    handshake_retry: u32,
    client_time: u32,
    handshaking: bool,
}

impl HandshakeState {
    /// Create the initial handshake state and its ordered setup effects.
    ///
    /// This mirrors `DESC::Setup`: set the plaintext handshake phase, then
    /// start the handshake with the caller-provided token and current time.
    #[must_use]
    pub fn start(
        expected_token: u32,
        server_kind: HandshakeServerKind,
        now: u32,
    ) -> HandshakeReduction {
        let state = Self {
            expected_token,
            server_kind,
            phase: HandshakePhase::Handshake,
            handshake_sent_time: now,
            handshake_retry: 0,
            client_time: 0,
            handshaking: true,
        };
        let effects = vec![
            HandshakeEffect::SetPhase {
                phase: HandshakePhase::Handshake,
                input_boundary: HandshakeInputBoundary::Plaintext,
            },
            HandshakeEffect::SendHandshake {
                token: expected_token,
                time: now,
                delta: 0,
            },
        ];
        HandshakeReduction { state, effects }
    }

    /// Return the caller-provided expected token.
    #[must_use]
    pub const fn expected_token(self) -> u32 {
        self.expected_token
    }

    /// Return the server role used for the initial phase transition.
    #[must_use]
    pub const fn server_kind(self) -> HandshakeServerKind {
        self.server_kind
    }

    /// Return the reduced descriptor phase.
    #[must_use]
    pub const fn phase(self) -> HandshakePhase {
        self.phase
    }

    /// Return the time associated with the most recent full handshake send.
    #[must_use]
    pub const fn handshake_sent_time(self) -> u32 {
        self.handshake_sent_time
    }

    /// Return the finite initial-handshake retry count.
    #[must_use]
    pub const fn handshake_retry(self) -> u32 {
        self.handshake_retry
    }

    /// Return the client time captured by the last accepted response.
    #[must_use]
    pub const fn client_time(self) -> u32 {
        self.client_time
    }

    /// Return whether the descriptor considers a handshake response pending.
    #[must_use]
    pub const fn is_handshaking(self) -> bool {
        self.handshaking
    }

    /// Record a legacy `SendHandshake` state transition.
    ///
    /// The heartbeat path calls `SendHandshake` without entering
    /// `HandshakeProcess`. This method preserves the two descriptor fields
    /// changed by that source method: the sent timestamp and handshaking flag.
    /// It does not create a wire frame or negotiate encryption.
    #[must_use]
    pub fn record_sent_handshake(mut self, now: u32) -> Self {
        self.handshake_sent_time = now;
        self.handshaking = true;
        self
    }

    /// Apply the source `SetPhase(PHASE_CLOSE)` state transition.
    ///
    /// The caller remains responsible for the control-only close effect and
    /// any input-processor or transport cleanup. No wire bytes are implied.
    #[must_use]
    pub fn close(mut self) -> Self {
        self.phase = HandshakePhase::Close;
        self
    }
}

/// Result of starting or reducing the pure handshake state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeReduction {
    /// State after the event.
    pub state: HandshakeState,
    /// Effects in the order required by the legacy source.
    pub effects: Vec<HandshakeEffect>,
}

/// Reduce one already decoded inbound handshake record.
///
/// Decoding and exact-length/header validation happen in
/// `protocol::cg_handshake`; malformed bytes never reach this function.
#[must_use]
pub fn reduce(state: HandshakeState, packet: CgInboundHandshake, now: u32) -> HandshakeReduction {
    match state.phase {
        HandshakePhase::Handshake => {
            if packet.header != CgHandshakeHeader::Handshake {
                return close(state);
            }
            if packet.token != state.expected_token {
                return close(state);
            }
            process_timing(state, packet, now, false)
        }
        HandshakePhase::Login => {
            if packet.header != CgHandshakeHeader::TimeSync {
                return unchanged(state);
            }
            if packet.token != state.expected_token {
                return close(state);
            }
            process_timing(state, packet, now, true)
        }
        // The auth analyzer explicitly ignores 0xff, and its default path
        // only logs 0xfc. Neither reaches common Handshake processing.
        HandshakePhase::Auth | HandshakePhase::Close => unchanged(state),
    }
}

fn process_timing(
    mut state: HandshakeState,
    packet: CgInboundHandshake,
    now: u32,
    infinite_retry: bool,
) -> HandshakeReduction {
    if packet.delta < 0 {
        return unchanged(state);
    }

    // C++ promotes the signed x86 long to DWORD for this addition. Copying
    // the little-endian bytes preserves the same 32-bit bit pattern without a
    // sign-changing Rust cast.
    let client_server_time = packet
        .time
        .wrapping_add(u32::from_le_bytes(packet.delta.to_le_bytes()));
    let bias = i32::from_le_bytes(now.wrapping_sub(client_server_time).to_le_bytes());
    if (0..=HANDSHAKE_BIAS_LIMIT).contains(&bias) {
        state.client_time = now;
        state.handshaking = false;
        if infinite_retry {
            return HandshakeReduction {
                state,
                effects: vec![HandshakeEffect::SendTimeSyncAck],
            };
        }

        let phase = match state.server_kind {
            HandshakeServerKind::Game => HandshakePhase::Login,
            HandshakeServerKind::Auth => HandshakePhase::Auth,
        };
        state.phase = phase;
        return HandshakeReduction {
            state,
            effects: vec![HandshakeEffect::SetPhase {
                phase,
                input_boundary: HandshakeInputBoundary::LegacyTea,
            }],
        };
    }

    let mut new_delta = signed_wrapped_difference(now, packet.time) / 2;
    if new_delta < 0 {
        new_delta = signed_wrapped_difference(now, state.handshake_sent_time) / 2;
    }

    if !infinite_retry {
        state.handshake_retry += 1;
        if state.handshake_retry > HANDSHAKE_RETRY_LIMIT {
            state.phase = HandshakePhase::Close;
            return HandshakeReduction {
                state,
                effects: vec![HandshakeEffect::Close],
            };
        }
    }

    state.handshake_sent_time = now;
    state.handshaking = true;
    HandshakeReduction {
        state,
        effects: vec![HandshakeEffect::SendHandshake {
            token: state.expected_token,
            time: now,
            delta: new_delta,
        }],
    }
}

fn signed_wrapped_difference(left: u32, right: u32) -> i32 {
    i32::from_le_bytes(left.wrapping_sub(right).to_le_bytes())
}

fn close(state: HandshakeState) -> HandshakeReduction {
    let mut state = state;
    state.phase = HandshakePhase::Close;
    // `Close` represents SetPhase(PHASE_CLOSE): the phase is assigned before
    // the legacy Packet call, which returns without writing. A later adapter
    // still has to perform the close input-processor transition.
    HandshakeReduction {
        state,
        effects: vec![HandshakeEffect::Close],
    }
}

fn unchanged(state: HandshakeState) -> HandshakeReduction {
    HandshakeReduction {
        state,
        effects: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: u32 = 0x1234_5678;

    fn packet(header: CgHandshakeHeader, time: u32, delta: i32) -> CgInboundHandshake {
        CgInboundHandshake::new(header, TOKEN, time, delta)
    }

    fn start(kind: HandshakeServerKind) -> HandshakeState {
        HandshakeState::start(TOKEN, kind, 1_000).state
    }

    #[test]
    fn initial_start_and_accepted_flow_preserve_effect_order() {
        let start = HandshakeState::start(TOKEN, HandshakeServerKind::Game, 1_000);
        assert_eq!(
            start.effects,
            vec![
                HandshakeEffect::SetPhase {
                    phase: HandshakePhase::Handshake,
                    input_boundary: HandshakeInputBoundary::Plaintext,
                },
                HandshakeEffect::SendHandshake {
                    token: TOKEN,
                    time: 1_000,
                    delta: 0,
                },
            ]
        );
        assert_eq!(start.state.phase(), HandshakePhase::Handshake);
        assert!(start.state.is_handshaking());

        let accepted = reduce(
            start.state,
            packet(CgHandshakeHeader::Handshake, 990, 5),
            1_000,
        );
        assert_eq!(accepted.state.phase(), HandshakePhase::Login);
        assert_eq!(accepted.state.client_time(), 1_000);
        assert!(!accepted.state.is_handshaking());
        assert_eq!(
            accepted.effects,
            vec![HandshakeEffect::SetPhase {
                phase: HandshakePhase::Login,
                input_boundary: HandshakeInputBoundary::LegacyTea,
            }]
        );
    }

    #[test]
    fn initial_success_selects_auth_instead_of_login() {
        let state = start(HandshakeServerKind::Auth);
        let accepted = reduce(state, packet(CgHandshakeHeader::Handshake, 1_000, 0), 1_000);
        assert_eq!(accepted.state.phase(), HandshakePhase::Auth);
        assert_eq!(
            accepted.effects,
            vec![HandshakeEffect::SetPhase {
                phase: HandshakePhase::Auth,
                input_boundary: HandshakeInputBoundary::LegacyTea,
            }]
        );
    }

    #[test]
    fn token_mismatch_closes_before_timing_or_retry_work() {
        let state = start(HandshakeServerKind::Game);
        let mut wrong = packet(CgHandshakeHeader::Handshake, 1_000, 0);
        wrong.token = TOKEN ^ 1;
        let result = reduce(state, wrong, 1_000);

        assert_eq!(result.state.phase(), HandshakePhase::Close);
        assert_eq!(result.state.handshake_retry(), 0);
        assert_eq!(result.state.client_time(), 0);
        assert_eq!(result.effects, vec![HandshakeEffect::Close]);
    }

    #[test]
    fn timing_bias_is_inclusive_and_signed_delta_failure_is_a_noop() {
        let exact_zero = reduce(
            start(HandshakeServerKind::Game),
            packet(CgHandshakeHeader::Handshake, 950, 50),
            1_000,
        );
        assert_eq!(exact_zero.state.phase(), HandshakePhase::Login);

        let exact_fifty = reduce(
            start(HandshakeServerKind::Game),
            packet(CgHandshakeHeader::Handshake, 900, 50),
            1_000,
        );
        assert_eq!(exact_fifty.state.phase(), HandshakePhase::Login);

        let negative = reduce(
            start(HandshakeServerKind::Game),
            packet(CgHandshakeHeader::Handshake, 1_000, -1),
            1_000,
        );
        assert_eq!(negative.state, start(HandshakeServerKind::Game));
        assert!(negative.effects.is_empty());
    }

    #[test]
    fn initial_rejection_sends_corrected_handshake_and_counts_retries() {
        let result = reduce(
            start(HandshakeServerKind::Game),
            packet(CgHandshakeHeader::Handshake, 800, 0),
            1_000,
        );
        assert_eq!(result.state.phase(), HandshakePhase::Handshake);
        assert_eq!(result.state.handshake_retry(), 1);
        assert_eq!(result.state.handshake_sent_time(), 1_000);
        assert!(result.state.is_handshaking());
        assert_eq!(
            result.effects,
            vec![HandshakeEffect::SendHandshake {
                token: TOKEN,
                time: 1_000,
                delta: 100,
            }]
        );

        let mut state = result.state;
        for expected_retry in 2..=HANDSHAKE_RETRY_LIMIT {
            let result = reduce(state, packet(CgHandshakeHeader::Handshake, 0, 0), u32::MAX);
            state = result.state;
            assert_eq!(state.handshake_retry(), expected_retry);
            assert_eq!(state.phase(), HandshakePhase::Handshake);
            assert_eq!(result.effects.len(), 1);
        }

        let closed = reduce(state, packet(CgHandshakeHeader::Handshake, 0, 0), u32::MAX);
        assert_eq!(closed.state.handshake_retry(), HANDSHAKE_RETRY_LIMIT + 1);
        assert_eq!(closed.state.phase(), HandshakePhase::Close);
        assert_eq!(closed.effects, vec![HandshakeEffect::Close]);
    }

    #[test]
    fn post_handshake_resync_acknowledges_success_and_retries_without_a_limit() {
        let login = reduce(
            start(HandshakeServerKind::Game),
            packet(CgHandshakeHeader::Handshake, 1_000, 0),
            1_000,
        )
        .state;

        let accepted = reduce(login, packet(CgHandshakeHeader::TimeSync, 2_000, 0), 2_010);
        assert_eq!(accepted.state.phase(), HandshakePhase::Login);
        assert_eq!(accepted.state.client_time(), 2_010);
        assert_eq!(accepted.effects, vec![HandshakeEffect::SendTimeSyncAck]);

        let mut state = login;
        for _ in 0..(HANDSHAKE_RETRY_LIMIT + 2) {
            let result = reduce(state, packet(CgHandshakeHeader::TimeSync, 0, 0), u32::MAX);
            state = result.state;
            assert_eq!(state.phase(), HandshakePhase::Login);
            assert_eq!(result.effects.len(), 1);
            assert!(matches!(
                result.effects[0],
                HandshakeEffect::SendHandshake { .. }
            ));
        }
        assert_eq!(state.handshake_retry(), 0);
    }

    #[test]
    fn wrapping_time_arithmetic_uses_32_bit_dword_semantics() {
        let state = start(HandshakeServerKind::Game);
        let response = packet(CgHandshakeHeader::Handshake, u32::MAX - 10, 20);
        let result = reduce(state, response, 10);

        assert_eq!(result.state.phase(), HandshakePhase::Login);
        assert_eq!(result.state.client_time(), 10);
    }

    #[test]
    fn phase_specific_headers_and_auth_ignore_match_legacy_dispatch() {
        let handshake = start(HandshakeServerKind::Game);
        let wrong_initial = reduce(
            handshake,
            packet(CgHandshakeHeader::TimeSync, 1_000, 0),
            1_000,
        );
        assert_eq!(wrong_initial.state.phase(), HandshakePhase::Close);
        assert_eq!(wrong_initial.effects, vec![HandshakeEffect::Close]);

        let login = reduce(
            handshake,
            packet(CgHandshakeHeader::Handshake, 1_000, 0),
            1_000,
        )
        .state;
        let ignored_login_header =
            reduce(login, packet(CgHandshakeHeader::Handshake, 1_000, 0), 1_000);
        assert_eq!(ignored_login_header.state, login);
        assert!(ignored_login_header.effects.is_empty());

        let auth = reduce(
            start(HandshakeServerKind::Auth),
            packet(CgHandshakeHeader::Handshake, 1_000, 0),
            1_000,
        )
        .state;
        for header in [CgHandshakeHeader::Handshake, CgHandshakeHeader::TimeSync] {
            let ignored = reduce(auth, packet(header, 1_000, 0), 1_000);
            assert_eq!(ignored.state, auth);
            assert!(ignored.effects.is_empty());
        }
    }
}
