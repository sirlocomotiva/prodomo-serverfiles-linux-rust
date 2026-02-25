//! Characterization coverage for the original world crate surface.

use world::{
    character::{Character, CoreState},
    common, protocol,
};

#[test]
fn world_reexports_existing_common_and_protocol_facilities() {
    // Given: the current world crate's public dependency re-exports.
    let vid = common::vid::Vid::new(41);

    // When: consumers use both facilities through world.
    let raw_vid = vid.raw();
    let character_name_capacity = protocol::CHARACTER_NAME_MAX_LEN;

    // Then: the existing public surface remains usable.
    assert_eq!(raw_vid, 41);
    assert_eq!(character_name_capacity, 24);
}

#[test]
fn world_preserves_current_character_export() {
    // Given: the character API exported by the current world surface.
    let character = Character::new(common::vid::Vid::new(42));

    // When: a world consumer observes its initial runtime state.
    let state = (character.vid(), character.core_state());

    // Then: the public export and legacy-compatible defaults remain available.
    assert_eq!(state, (common::vid::Vid::new(42), CoreState::Initial));
}
