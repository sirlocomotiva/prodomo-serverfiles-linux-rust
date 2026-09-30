//! The Prodomo server: auth, every Channel, and the Operator commands in one process (ADR-0002).

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};

/// Deterministic scheduling and dedicated game-loop thread ownership.
pub mod game_loop;

/// The characters, item ids, and item prototypes one game thread owns.
pub mod game_state;

/// Typed commands, effects, and terminal acknowledgements for the game loop.
pub mod game_loop_messages;

/// The auth login rules and the login-key registry.
pub mod auth_login;

/// The Channel login rules, the logon registry, and the character list.
pub mod channel_login;
pub mod chat;

/// `CHARACTER::ChatPacket`: a server line, in the descriptor's language, to one character.
pub mod chat_line;
/// The command interpreter (`interpret_command`) and its table.
pub mod command;

/// The Channel status list answered to `STATE_CHECKER`.
pub mod channel_status;
pub mod client_live;
pub mod client_registry;
pub mod movement;

/// Isolated fixed-frame client session and dispatch boundary.
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

/// Source-derived legacy descriptor TEA key and buffering boundary.
pub mod descriptor_crypto;

/// Pure raw-wire projection for heartbeat effects.
pub mod heartbeat_wire;

/// Transport-free account and player state reducer.
pub mod account_player;

/// Transport-free account/player and lifecycle routing seam.
pub mod account_player_router;

pub mod account_records;

pub mod ready_gate;

pub mod select_phase;

pub mod loading_phase;

pub mod listeners;

pub mod item_grant;

/// Placing a character's stored items at character select.
pub mod item_load;
/// `CG_ITEM_MOVE` around the world's move: prototype facts, row changes, and notices.
pub mod item_move;
pub mod item_persist;
pub mod operator;
pub mod operator_console;
/// Quickslots between the store, the world and the client.
pub mod quickslot;

/// When a character's row is written, as legacy's save cycle and disconnect decide it.
pub mod save;

/// Pure, injected policy for the legacy sync-position gameplay action.
pub mod sync_position;
pub mod warp;

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
