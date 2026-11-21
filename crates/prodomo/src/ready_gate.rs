//! Client-acceptance gate driven by server startup.
//!
//! # Why this gate exists
//!
//! There is no readiness flag anywhere in the legacy tree. `main.cpp:671` binds
//! the client port, `main.cpp:697` only then creates the DB connector, and
//! `main.cpp:840-856` calls `TryConnect()` discarding its result before calling
//! `AcceptDesc()` unconditionally. So legacy accepts, handshakes, and logs in
//! players while its tables are still missing. That is a Defect, not a Quirk.
//!
//! The Rewrite has no DB server to wait for (ADR-0002), but it has a store. What
//! a client must not reach is a world whose store is unreachable or whose Game
//! data is not loaded yet, so the gate opens once startup has finished (today:
//! once the store's schema is migrated) and closes again when shutdown begins.
//! This is a Divergence, and it is isolated here so a reviewer can find it in
//! one place.
//!
//! It is a counting gate rather than a plain flag because a refused connection
//! is closed immediately, and the counters make that visible in the logs.
//!
//! The state is atomic so the accept loop and the startup task can share one
//! `Arc<ReadyGate>` without a lock on the hot path.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Counting client-acceptance gate; see the module documentation.
#[derive(Debug, Default)]
pub struct ReadyGate {
    ready: AtomicBool,
    refused: AtomicU64,
    admitted: AtomicU64,
}

impl ReadyGate {
    /// A closed gate, as the client listener is at startup.
    #[must_use]
    pub const fn closed() -> Self {
        Self {
            ready: AtomicBool::new(false),
            refused: AtomicU64::new(0),
            admitted: AtomicU64::new(0),
        }
    }

    /// Whether startup has finished and clients may be served.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    /// Total connections refused because the server was not ready.
    #[must_use]
    pub fn refused(&self) -> u64 {
        self.refused.load(Ordering::SeqCst)
    }

    /// Total connections admitted.
    #[must_use]
    pub fn admitted(&self) -> u64 {
        self.admitted.load(Ordering::SeqCst)
    }

    /// Open the gate. Called once startup has finished.
    pub fn open(&self) {
        self.ready.store(true, Ordering::SeqCst);
    }

    /// Close the gate again. Called when shutdown begins.
    pub fn close(&self) {
        self.ready.store(false, Ordering::SeqCst);
    }

    /// Decide whether one inbound client connection may be served.
    ///
    /// A refused connection should be closed by the caller, not queued: the
    /// legacy client has no way to learn that the server is not ready, and
    /// holding the socket open would only make the failure look like a hang.
    pub fn admit(&self) -> bool {
        if self.is_ready() {
            self.admitted.fetch_add(1, Ordering::SeqCst);
            true
        } else {
            self.refused.fetch_add(1, Ordering::SeqCst);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_is_closed_at_startup_and_opens_only_when_ready() {
        let gate = ReadyGate::closed();
        assert!(!gate.is_ready());
        assert!(!gate.admit(), "clients are refused before startup ends");
        assert_eq!(gate.refused(), 1);
        assert_eq!(gate.admitted(), 0);
        gate.open();
        assert!(gate.is_ready());
        assert!(gate.admit());
        assert_eq!(gate.admitted(), 1);
        assert_eq!(gate.refused(), 1);
    }

    #[test]
    fn the_gate_recloses_for_shutdown() {
        let gate = ReadyGate::closed();
        gate.open();
        assert!(gate.admit());
        gate.close();
        assert!(!gate.admit(), "a closing server stops accepting clients");
        assert_eq!(gate.refused(), 1);
        assert_eq!(gate.admitted(), 1);
    }
}
