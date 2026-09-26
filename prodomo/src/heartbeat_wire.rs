//! Pure wire projection for heartbeat effects.
//!
//! State-only effects are intentionally omitted. In particular, the legacy
//! close path assigns `PHASE_CLOSE` before `Packet` is called, so it produces
//! no wire record. This module does not write sockets or manage timers.

use protocol::gc::{GcHandshake, GcPing};

use crate::heartbeat::HeartbeatEffect;

/// Project one heartbeat effect into an optional raw legacy wire record.
///
/// `None` means that the effect is descriptor state only. The caller remains
/// responsible for applying the state transition and transport boundary.
#[must_use]
pub fn encode_heartbeat_effect(effect: HeartbeatEffect) -> Option<Vec<u8>> {
    match effect {
        HeartbeatEffect::SendPing => Some(GcPing.encode()),
        HeartbeatEffect::SetPong { .. } | HeartbeatEffect::Close => None,
        HeartbeatEffect::SendHandshake { token, time, delta } => {
            Some(GcHandshake::new(token, time, delta).encode())
        }
    }
}

/// Project effects in order, dropping state-only effects.
#[must_use]
pub fn encode_heartbeat_effects(effects: &[HeartbeatEffect]) -> Vec<Vec<u8>> {
    effects
        .iter()
        .copied()
        .filter_map(encode_heartbeat_effect)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heartbeat::HeartbeatState;

    #[test]
    fn acknowledged_tick_projects_ping_then_handshake() {
        let reduction = HeartbeatState::new().on_tick(0x1234_5678, 100);
        assert_eq!(
            encode_heartbeat_effects(&reduction.effects),
            vec![
                vec![44],
                vec![0xff, 0x78, 0x56, 0x34, 0x12, 0x64, 0, 0, 0, 0, 0, 0, 0,],
            ]
        );
    }

    #[test]
    fn state_and_close_effects_have_no_wire_record() {
        let waiting = HeartbeatState::new().on_tick(7, 100).state;
        let close = waiting.on_tick(7, 200);
        assert_eq!(
            encode_heartbeat_effects(&close.effects),
            Vec::<Vec<u8>>::new()
        );
        assert!(encode_heartbeat_effect(HeartbeatEffect::SetPong { acknowledged: true }).is_none());
    }
}
