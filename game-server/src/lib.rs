//! Game-server scheduling and asynchronous boundary modules.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};

/// Deterministic scheduling and dedicated game-loop thread ownership.
pub mod game_loop;

/// Typed commands, effects, and terminal acknowledgements for the game loop.
pub mod game_loop_messages;

/// Isolated fixed-frame client session and dispatch boundary.
pub mod client_live;
pub mod client_session;

/// Pure handshake state reduction and ordered descriptor effects.
pub mod handshake;

/// Pure raw-wire projection for handshake reducer effects.
pub mod handshake_wire;

/// Transport-free decoded handshake-frame dispatch to lifecycle state.
pub mod handshake_dispatch;

/// Pure heartbeat/PONG liveness reduction.
pub mod heartbeat;

/// Transport-free descriptor lifecycle adapter.
pub mod lifecycle;

/// Transport-free game-to-DB client state machine and boot-ready gate.
pub mod db_client;

/// Live Tokio socket adapter for the game-to-DB link.
pub mod db_client_live;

/// Source-derived legacy descriptor TEA key and buffering boundary.
pub mod descriptor_crypto;

/// Pure raw-wire projection for heartbeat effects.
pub mod heartbeat_wire;

/// Transport-free account and player state reducer.
pub mod account_player;

/// Transport-free account/player and lifecycle routing seam.
pub mod account_player_router;

/// Pure, injected policy for the legacy sync-position gameplay action.
pub mod sync_position;

/// Process-level shutdown and connection-acceptance flags.
pub struct ServerState {
    shutdown: AtomicBool,
    accept_connections: AtomicBool,
}

impl ServerState {
    /// Creates a running state that accepts connections.
    pub const fn new() -> Self {
        Self {
            shutdown: AtomicBool::new(false),
            accept_connections: AtomicBool::new(true),
        }
    }

    /// Atomically marks shutdown requested and stops connection acceptance.
    pub fn initiate_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.accept_connections.store(false, Ordering::SeqCst);
    }

    /// Reports whether shutdown has been initiated.
    pub fn is_shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// Reports whether new connections may be accepted.
    pub fn should_accept_connections(&self) -> bool {
        self.accept_connections.load(Ordering::SeqCst)
    }
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}
