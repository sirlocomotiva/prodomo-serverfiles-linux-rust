//! Characterizes process-level connection acceptance during shutdown.

use prodomo::ServerState;

#[test]
fn server_state_stops_accepting_when_shutdown_is_initiated() {
    // Given: a newly created server that accepts connections.
    let state = ServerState::new();
    assert!(!state.is_shutting_down());
    assert!(state.should_accept_connections());

    // When: shutdown is initiated.
    state.initiate_shutdown();

    // Then: shutdown is visible and connection acceptance stops.
    assert!(state.is_shutting_down());
    assert!(!state.should_accept_connections());
}
