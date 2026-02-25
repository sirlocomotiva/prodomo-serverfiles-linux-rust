//! Character construction behavior tests.

use common::vid::Vid;
use world::character::{Activity, Character, CharacterKind, CoreState, Posture};

#[test]
fn character_creation_matches_legacy_defaults() {
    // Given: a freshly allocated legacy-compatible runtime identity.
    let character = Character::new(Vid::new(7));

    // When: its bounded runtime defaults are observed.
    let identity = (
        character.vid(),
        character.player_id(),
        character.name(),
        character.kind(),
    );

    // Then: initialization mirrors the legacy core without gameplay subsystems.
    assert_eq!(identity, (Vid::new(7), 0, "", CharacterKind::NonPlayer));
    assert_eq!(character.core_state(), CoreState::Initial);
    assert_eq!(character.pending_state(), Some(CoreState::Idle));
    assert_eq!(character.posture(), Posture::Standing);
    assert_eq!(character.activity(), Activity::None);
    assert_eq!(character.state_duration(), 1);
    assert_eq!(character.next_state_pulse(), 0);
}
