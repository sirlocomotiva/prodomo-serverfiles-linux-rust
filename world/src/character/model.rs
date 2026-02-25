use common::{vid::Vid, CharacterId};

use super::combat::{Damage, DamageOutcome, Vitality};
use super::state::{Activity, CharacterState, CoreState, Posture};

/// Runtime category used to distinguish players from non-player characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterKind {
    /// A character not backed by a player record.
    NonPlayer,
    /// A character backed by a player record.
    Player,
}

/// Bounded runtime core of a world character.
#[derive(Debug)]
pub struct Character {
    vid: Vid,
    player_id: CharacterId,
    name: String,
    kind: CharacterKind,
    state: CharacterState,
    posture: Posture,
    activity: Activity,
    vitality: Vitality,
    next_state_pulse: u64,
    destruction_requested: bool,
}

impl Character {
    /// Creates a non-player character with legacy runtime defaults.
    pub fn new(vid: Vid) -> Self {
        Self {
            vid,
            player_id: 0,
            name: String::new(),
            kind: CharacterKind::NonPlayer,
            state: CharacterState::new(),
            posture: Posture::Standing,
            activity: Activity::None,
            vitality: Vitality::new(0, 0),
            next_state_pulse: 0,
            destruction_requested: false,
        }
    }

    pub(crate) fn player(vid: Vid, player_id: CharacterId, name: &str) -> Self {
        Self {
            vid,
            player_id,
            name: name.to_owned(),
            kind: CharacterKind::Player,
            state: CharacterState::new(),
            posture: Posture::Standing,
            activity: Activity::None,
            vitality: Vitality::new(0, 0),
            next_state_pulse: 0,
            destruction_requested: false,
        }
    }

    /// Returns the character's virtual identifier.
    pub const fn vid(&self) -> Vid {
        self.vid
    }

    /// Returns the persistent player identifier, or zero for an unbound character.
    pub const fn player_id(&self) -> CharacterId {
        self.player_id
    }

    /// Returns the character name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns whether this runtime character is player-backed.
    pub const fn kind(&self) -> CharacterKind {
        self.kind
    }

    /// Returns the active core finite-state-machine state.
    pub const fn core_state(&self) -> CoreState {
        self.state.current()
    }

    /// Returns the state queued for the next eligible update.
    pub const fn pending_state(&self) -> Option<CoreState> {
        self.state.pending()
    }

    /// Returns the character's liveness posture.
    pub const fn posture(&self) -> Posture {
        self.posture
    }

    /// Returns the character's independent runtime activity.
    pub const fn activity(&self) -> Activity {
        self.activity
    }

    /// Changes the independent runtime activity.
    pub const fn set_activity(&mut self, activity: Activity) {
        self.activity = activity;
    }

    /// Returns current and maximum hit points.
    pub const fn vitality(&self) -> Vitality {
        self.vitality
    }

    /// Replaces current and maximum hit points without changing other Character state.
    pub const fn set_vitality(&mut self, vitality: Vitality) {
        self.vitality = vitality;
    }

    /// Applies damage and enters dead posture at the zero-hit-point boundary.
    pub const fn apply_damage(&mut self, damage: Damage) -> DamageOutcome {
        if matches!(self.posture, Posture::Dead) {
            return DamageOutcome::AlreadyDead;
        }
        let (vitality, outcome) = self.vitality.take(damage);
        self.vitality = vitality;
        if matches!(outcome, DamageOutcome::Killed { .. }) {
            self.posture = Posture::Dead;
        }
        outcome
    }

    /// Changes the liveness posture independently from the core FSM.
    pub const fn set_posture(&mut self, posture: Posture) {
        self.posture = posture;
    }

    /// Returns the delay used after an eligible state update.
    pub const fn state_duration(&self) -> u64 {
        self.state.duration()
    }

    /// Sets the pulse delay applied after the next eligible update.
    pub const fn set_state_duration(&mut self, duration: u64) {
        self.state.set_duration(duration);
    }

    /// Returns the absolute pulse at which the next state update is due.
    pub const fn next_state_pulse(&self) -> u64 {
        self.next_state_pulse
    }

    /// Queues a core state for the next eligible update.
    pub const fn request_state(&mut self, state: CoreState) {
        self.state.request(state);
    }

    /// Processes the core FSM when the supplied absolute pulse is due.
    pub fn update(&mut self, pulse: u64) -> bool {
        if pulse < self.next_state_pulse {
            return false;
        }

        match self.posture {
            Posture::Standing => {}
            Posture::Dead => return false,
        }

        self.state.apply_pending();
        self.next_state_pulse = pulse.saturating_add(self.state.duration());
        true
    }

    /// Requests removal after the manager's current update pass completes.
    pub const fn request_destruction(&mut self) {
        self.destruction_requested = true;
    }

    pub(crate) const fn destruction_requested(&self) -> bool {
        self.destruction_requested
    }
}
