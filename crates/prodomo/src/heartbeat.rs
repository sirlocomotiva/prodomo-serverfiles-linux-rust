//! Pure heartbeat and PONG liveness reduction.
//!
//! This module models the active `DESC::ping_event` path in
//! `server/server/game/desc.cpp` and the PONG input handlers in the legacy
//! analyzers. It receives a caller-supplied tick, token, and clock value; it
//! does not create a timer, open a socket, negotiate TEA, or claim a live
//! session. Effects are returned in source order for a later adapter.

/// One source-ordered heartbeat effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeartbeatEffect {
    /// Send the one-byte `HEADER_GC_PING` record.
    SendPing,
    /// Apply `DESC::SetPong` to the supplied acknowledgement value.
    SetPong {
        /// The value assigned to the descriptor's `m_bPong` field.
        acknowledged: bool,
    },
    /// Send the full GC handshake with a zero correction.
    SendHandshake {
        /// The descriptor's current handshake token.
        token: u32,
        /// The caller-provided descriptor clock value.
        time: u32,
        /// The source call always supplies zero here.
        delta: i32,
    },
    /// Apply the source close transition. This is control-only on the wire.
    Close,
}

/// Pure heartbeat state mirrored from the relevant descriptor fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeartbeatState {
    pong_acknowledged: bool,
    admin_mode: bool,
    closed: bool,
}

impl HeartbeatState {
    /// Create the source initial state: `m_bPong = true`, admin mode off.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pong_acknowledged: true,
            admin_mode: false,
            closed: false,
        }
    }

    /// Return whether the descriptor currently considers PONG acknowledged.
    #[must_use]
    pub const fn pong_acknowledged(self) -> bool {
        self.pong_acknowledged
    }

    /// Return whether the source admin-mode bypass is active.
    #[must_use]
    pub const fn admin_mode(self) -> bool {
        self.admin_mode
    }

    /// Return whether the descriptor has reached its terminal close state.
    #[must_use]
    pub const fn is_closed(self) -> bool {
        self.closed
    }

    /// Set the source admin-mode bypass used by `ping_event`.
    pub fn set_admin_mode(&mut self, enabled: bool) {
        self.admin_mode = enabled;
    }

    /// Apply a source PONG acknowledgement.
    ///
    /// The legacy handler calls `SetPong(true)`. A duplicate acknowledgement is
    /// represented as a no-op because it cannot change descriptor state.
    #[must_use]
    pub fn on_pong(self) -> HeartbeatReduction {
        if self.closed || self.pong_acknowledged {
            return unchanged(self);
        }
        HeartbeatReduction {
            state: Self {
                pong_acknowledged: true,
                ..self
            },
            effects: vec![HeartbeatEffect::SetPong { acknowledged: true }],
        }
    }

    /// Reduce one caller-timed heartbeat tick.
    ///
    /// On an acknowledged tick the source sends the ping, clears `m_bPong`,
    /// and then sends a zero-delta handshake in exactly that order. On a
    /// missing acknowledgement it applies the close transition and emits no
    /// subsequent ping or handshake. Admin mode returns before either check.
    #[must_use]
    pub fn on_tick(self, token: u32, now: u32) -> HeartbeatReduction {
        if self.closed || self.admin_mode {
            return unchanged(self);
        }
        if !self.pong_acknowledged {
            return HeartbeatReduction {
                state: Self {
                    closed: true,
                    ..self
                },
                effects: vec![HeartbeatEffect::Close],
            };
        }

        HeartbeatReduction {
            state: Self {
                pong_acknowledged: false,
                ..self
            },
            effects: vec![
                HeartbeatEffect::SendPing,
                HeartbeatEffect::SetPong {
                    acknowledged: false,
                },
                HeartbeatEffect::SendHandshake {
                    token,
                    time: now,
                    delta: 0,
                },
            ],
        }
    }
}

impl Default for HeartbeatState {
    fn default() -> Self {
        Self::new()
    }
}

/// State and effects returned by one heartbeat event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatReduction {
    /// State after the event.
    pub state: HeartbeatState,
    /// Effects in the exact order required by the legacy source.
    pub effects: Vec<HeartbeatEffect>,
}

fn unchanged(state: HeartbeatState) -> HeartbeatReduction {
    HeartbeatReduction {
        state,
        effects: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_matches_descriptor_default() {
        let state = HeartbeatState::new();
        assert!(state.pong_acknowledged());
        assert!(!state.admin_mode());
        assert!(!state.is_closed());
    }

    #[test]
    fn acknowledged_tick_sends_ping_clears_pong_then_handshake() {
        let result = HeartbeatState::new().on_tick(0x1234_5678, 100);
        assert_eq!(
            result.effects,
            vec![
                HeartbeatEffect::SendPing,
                HeartbeatEffect::SetPong {
                    acknowledged: false,
                },
                HeartbeatEffect::SendHandshake {
                    token: 0x1234_5678,
                    time: 100,
                    delta: 0,
                },
            ]
        );
        assert!(!result.state.pong_acknowledged());
    }

    #[test]
    fn missing_pong_closes_without_more_wire_effects() {
        let first = HeartbeatState::new().on_tick(7, 100);
        let second = first.state.on_tick(7, 200);
        assert_eq!(second.effects, vec![HeartbeatEffect::Close]);
        assert!(second.state.is_closed());
        assert!(second.state.on_tick(7, 300).effects.is_empty());
    }

    #[test]
    fn pong_rearms_and_duplicate_pong_is_idempotent() {
        let waiting = HeartbeatState::new().on_tick(7, 100).state;
        let rearmed = waiting.on_pong();
        assert_eq!(
            rearmed.effects,
            vec![HeartbeatEffect::SetPong { acknowledged: true }]
        );
        assert!(rearmed.state.pong_acknowledged());
        assert!(rearmed.state.on_pong().effects.is_empty());
    }

    #[test]
    fn admin_mode_bypasses_tick_and_does_not_change_pong() {
        let mut state = HeartbeatState::new();
        state.set_admin_mode(true);
        let result = state.on_tick(7, 100);
        assert_eq!(result.state, state);
        assert!(result.effects.is_empty());
    }
}
