/// Core runtime finite-state-machine states from the legacy character FSM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreState {
    /// Construction state before the first eligible update.
    Initial,
    /// Waiting without movement or combat work.
    Idle,
    /// Movement processing.
    Move,
    /// Battle processing.
    Battle,
}

/// Character liveness/posture, independent from the core FSM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Posture {
    /// Character can process eligible state updates.
    Standing,
    /// Character is dead and suppresses state execution.
    Dead,
}

/// Character activity, independent from the core FSM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// No separate activity is active.
    None,
    /// Fishing activity seam reserved for its gameplay system.
    Fishing,
    /// Mining activity seam reserved for its gameplay system.
    Mining,
}

#[derive(Debug)]
pub(crate) struct CharacterState {
    current: CoreState,
    pending: Option<CoreState>,
    duration: u64,
}

impl CharacterState {
    pub(crate) const fn new() -> Self {
        Self {
            current: CoreState::Initial,
            pending: Some(CoreState::Idle),
            duration: 1,
        }
    }

    pub(crate) const fn current(&self) -> CoreState {
        self.current
    }

    pub(crate) const fn pending(&self) -> Option<CoreState> {
        self.pending
    }

    pub(crate) const fn duration(&self) -> u64 {
        self.duration
    }

    pub(crate) const fn set_duration(&mut self, duration: u64) {
        self.duration = duration;
    }

    pub(crate) const fn request(&mut self, state: CoreState) {
        self.pending = Some(state);
    }

    pub(crate) fn apply_pending(&mut self) {
        if let Some(state) = self.pending.take() {
            self.current = state;
        }
    }
}
