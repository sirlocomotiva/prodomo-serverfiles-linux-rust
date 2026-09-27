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

/// A live client brings its own VID, which is the number its descriptor already
/// published at enter-game. The manager must index it under exactly that number
/// rather than allocating a second one, or every record the world writes is
/// addressed to an identity the client has never seen.
#[test]
fn a_caller_supplied_vid_is_indexed_rather_than_a_fresh_counter_value() {
    // Given: an empty manager, which has not issued its first counter value yet.
    let mut manager = CharacterManager::new();

    // When: a character is admitted under the store's own id.
    let admitted = manager.create_player_with_vid(4_242, "Alice", Vid::new(4_242));

    // Then: the VID is the one supplied, and no counter value was consumed, which
    // is what a later `create_player` proves by still starting at one.
    assert_eq!(admitted, Ok(()));
    assert_eq!(
        manager.find_by_vid(Vid::new(4_242)).map(Character::vid),
        Ok(Vid::new(4_242))
    );
    assert_eq!(manager.create_player(1, "Mob").unwrap(), Vid::new(1));
}

#[test]
fn a_caller_supplied_duplicate_vid_is_refused_and_indexes_nothing() {
    // Given: one character already holding VID 7.
    let mut manager = CharacterManager::new();
    manager
        .create_player_with_vid(7, "Alice", Vid::new(7))
        .unwrap();

    // When: a second character claims the same VID under a different name and id.
    let refused = manager.create_player_with_vid(8, "Bob", Vid::new(7));

    // Then: the refusal is its own typed variant, not a name or id duplicate, and
    // the first character is still the one under that number.
    assert_eq!(
        refused,
        Err(CharacterManagerError::DuplicateVid(Vid::new(7)))
    );
    assert_eq!(manager.len(), 1);
    assert_eq!(
        manager.find_by_vid(Vid::new(7)).map(Character::name),
        Ok("Alice")
    );
    assert_eq!(
        manager.find_player_by_name("Bob").map(Character::vid),
        Err(CharacterManagerError::UnknownPlayerName("Bob".to_owned()))
    );
    assert_eq!(
        manager.find_by_pid(8).map(Character::vid),
        Err(CharacterManagerError::UnknownPlayerId(8))
    );
}

#[test]
fn a_null_vid_is_refused_because_it_is_what_a_record_with_no_target_carries() {
    // `VID::NULL` is what legacy puts in a record that addresses nobody. Indexing a
    // character there would make a "to nobody" claim find someone.
    let mut manager = CharacterManager::new();
    assert_eq!(
        manager.create_player_with_vid(1, "Alice", Vid::NULL),
        Err(CharacterManagerError::NullVid)
    );
    assert_eq!(manager.len(), 0);
    assert_eq!(manager.active_len(), 0);
    // And it indexes nothing at all, so the same character can still be admitted
    // properly afterwards.
    assert_eq!(
        manager.create_player_with_vid(1, "Alice", Vid::new(1)),
        Ok(())
    );
}
