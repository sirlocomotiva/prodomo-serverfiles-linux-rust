//! Character manager indexing and lifecycle behavior tests.

use common::vid::Vid;
use world::character::{Character, CharacterManager, CharacterManagerError};

#[test]
fn manager_indexes_by_vid_pid_and_ascii_case_insensitive_name() {
    // Given: one player created through a fresh manager.
    let mut manager = CharacterManager::new();
    let vid = manager.create_player(42, "Alice").unwrap();

    // When: each supported identity index is queried.
    let by_virtual_id = manager.find_by_vid(vid).map(Character::vid);
    let by_player_id = manager.find_by_pid(42).map(Character::vid);
    let by_name = manager.find_player_by_name("aLiCe").map(Character::vid);

    // Then: every index resolves consistently and duplicate keys are typed errors.
    assert_eq!(vid, Vid::new(1));
    assert_eq!(by_virtual_id, Ok(vid));
    assert_eq!(by_player_id, Ok(vid));
    assert_eq!(by_name, Ok(vid));
    assert_eq!(
        manager.create_player(42, "Other"),
        Err(CharacterManagerError::DuplicatePlayerId(42))
    );
    assert_eq!(
        manager.create_player(43, "ALICE"),
        Err(CharacterManagerError::DuplicatePlayerName(
            "ALICE".to_owned()
        ))
    );
    assert_eq!(
        manager.find_by_vid(Vid::new(999)).map(Character::vid),
        Err(CharacterManagerError::UnknownVid(Vid::new(999)))
    );
}

#[test]
fn manager_destruction_removes_every_index_and_state_membership() {
    // Given: a player present in every manager-owned index.
    let mut manager = CharacterManager::new();
    let vid = manager.create_player(42, "Alice").unwrap();
    assert_eq!(manager.len(), 1);
    assert_eq!(manager.active_len(), 1);

    // When: immediate destruction is requested twice.
    let first_destroy = manager.destroy(vid);
    let repeated_destroy = manager.destroy(vid);

    // Then: all memberships are gone and repetition has a typed result.
    assert_eq!(first_destroy, Ok(()));
    assert_eq!(
        repeated_destroy,
        Err(CharacterManagerError::UnknownVid(vid))
    );
    assert_eq!(manager.len(), 0);
    assert_eq!(manager.active_len(), 0);
    assert_eq!(
        manager.find_by_vid(vid).map(Character::vid),
        Err(CharacterManagerError::UnknownVid(vid))
    );
    assert_eq!(
        manager.find_by_pid(42).map(Character::vid),
        Err(CharacterManagerError::UnknownPlayerId(42))
    );
    assert_eq!(
        manager.find_player_by_name("ALICE").map(Character::vid),
        Err(CharacterManagerError::UnknownPlayerName("ALICE".to_owned()))
    );
}

#[test]
fn destruction_requested_during_iteration_is_deferred_until_pass_finishes() {
    // Given: two active players in one manager update snapshot.
    let mut manager = CharacterManager::new();
    let doomed = manager.create_player(10, "Doomed").unwrap();
    let survivor = manager.create_player(11, "Survivor").unwrap();
    let mut visited = Vec::new();

    // When: one character requests destruction twice during the pass.
    let report = manager.process_pulse_with(0, |character| {
        visited.push(character.vid());
        if character.vid() == doomed {
            character.request_destruction();
            character.request_destruction();
        }
    });

    // Then: the full snapshot updates before one deferred removal is flushed.
    visited.sort_unstable_by_key(|vid| vid.raw());
    assert_eq!(visited, vec![doomed, survivor]);
    assert_eq!(report.updated(), 2);
    assert_eq!(report.destroyed(), 1);
    assert_eq!(manager.len(), 1);
    assert_eq!(manager.active_len(), 1);
    assert_eq!(
        manager.find_by_vid(doomed).map(Character::vid),
        Err(CharacterManagerError::UnknownVid(doomed))
    );
    assert_eq!(manager.find_by_vid(survivor).unwrap().vid(), survivor);
}
