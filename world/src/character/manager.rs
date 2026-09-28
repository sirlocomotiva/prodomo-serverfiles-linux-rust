use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

use common::{vid::Vid, CharacterId};

use super::{Character, CharacterKind};

/// Typed failures produced by character manager operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CharacterManagerError {
    /// A player with this persistent identifier is already indexed.
    DuplicatePlayerId(CharacterId),
    /// A player with this ASCII case-insensitive name is already indexed.
    DuplicatePlayerName(String),
    /// A character with this virtual identifier is already indexed.
    ///
    /// Separate from the other duplicates because a caller that supplies its own VID
    /// can hit it, and "that number is taken" is a different repair from "that name
    /// is taken": the first is two clients claiming one identity, the second is one
    /// account logging in twice.
    DuplicateVid(Vid),
    /// A character cannot be indexed under the null VID.
    ///
    /// Legacy's `VID::NULL` is what a record with no target carries, so indexing a
    /// character there would make a "broadcast to nobody" claim find someone.
    NullVid,
    /// No character exists for this virtual identifier.
    UnknownVid(Vid),
    /// No player exists for this persistent identifier.
    UnknownPlayerId(CharacterId),
    /// No player exists for this ASCII case-insensitive name.
    UnknownPlayerName(String),
    /// The numeric virtual identifier space has been exhausted.
    VidExhausted,
}

impl fmt::Display for CharacterManagerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicatePlayerId(player_id) => {
                write!(formatter, "player id {player_id} is already indexed")
            }
            Self::DuplicatePlayerName(name) => {
                write!(formatter, "player name {name:?} is already indexed")
            }
            Self::DuplicateVid(vid) => write!(formatter, "character {vid} is already indexed"),
            Self::NullVid => {
                formatter.write_str("a character cannot be indexed under the null VID")
            }
            Self::UnknownVid(vid) => write!(formatter, "character {vid} was not found"),
            Self::UnknownPlayerId(player_id) => {
                write!(formatter, "player id {player_id} was not found")
            }
            Self::UnknownPlayerName(name) => {
                write!(formatter, "player name {name:?} was not found")
            }
            Self::VidExhausted => formatter.write_str("character VID space is exhausted"),
        }
    }
}

impl Error for CharacterManagerError {}

/// Observable result of processing one caller-supplied pulse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateReport {
    updated: usize,
    destroyed: usize,
}

impl UpdateReport {
    /// Returns the number of characters whose state machine executed.
    pub const fn updated(self) -> usize {
        self.updated
    }

    /// Returns the number of deferred destructions flushed after the pass.
    pub const fn destroyed(self) -> usize {
        self.destroyed
    }
}

/// Owns runtime characters and their identity indexes.
#[derive(Debug)]
pub struct CharacterManager {
    next_vid: u64,
    characters: HashMap<Vid, Character>,
    by_player_id: HashMap<CharacterId, Vid>,
    by_player_name: HashMap<String, Vid>,
    active: HashSet<Vid>,
}

impl CharacterManager {
    /// Creates an empty manager whose first allocated VID is one.
    pub fn new() -> Self {
        Self {
            next_vid: 1,
            characters: HashMap::new(),
            by_player_id: HashMap::new(),
            by_player_name: HashMap::new(),
            active: HashSet::new(),
        }
    }

    /// Creates and indexes a player character.
    ///
    /// # Errors
    /// Returns a typed duplicate-key or VID-exhaustion error.
    pub fn create_player(
        &mut self,
        player_id: CharacterId,
        name: &str,
    ) -> Result<Vid, CharacterManagerError> {
        let vid = self.allocate_vid()?;
        // The counter is known to be free because it came from `allocate_vid` and this
        // manager is the only allocator, so a duplicate-VID refusal here would be a
        // bug in that invariant rather than a state a caller can hit.
        self.create_player_with_vid(player_id, name, vid)?;
        Ok(vid)
    }

    /// Creates and indexes a player character under a caller-supplied VID.
    ///
    /// This is the entry point a live client uses, and the reason it exists is that
    /// the caller already has a number on the wire. Legacy allocates the VID from a
    /// process counter (`CHARACTER_MANAGER::AllocVID`) and pairs it with a server-local
    /// CRC that never reaches the wire, so a live descriptor is free to keep using
    /// whatever identity it published at enter-game. Handing the world a second
    /// number for the same character would make every record it sends unaddressable.
    ///
    /// # Errors
    ///
    /// [`CharacterManagerError::DuplicateVid`] when that VID is taken,
    /// [`CharacterManagerError::DuplicatePlayerId`], or
    /// [`CharacterManagerError::DuplicatePlayerName`]. None of the three inserts
    /// anything, and a null VID is refused because it is what a record with no target
    /// carries.
    pub fn create_player_with_vid(
        &mut self,
        player_id: CharacterId,
        name: &str,
        vid: Vid,
    ) -> Result<(), CharacterManagerError> {
        if vid.is_null() {
            return Err(CharacterManagerError::NullVid);
        }
        if self.characters.contains_key(&vid) {
            return Err(CharacterManagerError::DuplicateVid(vid));
        }
        if self.by_player_id.contains_key(&player_id) {
            return Err(CharacterManagerError::DuplicatePlayerId(player_id));
        }

        let canonical_name = name.to_ascii_lowercase();
        if self.by_player_name.contains_key(&canonical_name) {
            return Err(CharacterManagerError::DuplicatePlayerName(name.to_owned()));
        }

        let character = Character::player(vid, player_id, name);
        let _previous_character = self.characters.insert(vid, character);
        let _previous_player = self.by_player_id.insert(player_id, vid);
        let _previous_name = self.by_player_name.insert(canonical_name, vid);
        self.active.insert(vid);
        Ok(())
    }

    /// Finds a character by virtual identifier.
    ///
    /// # Errors
    /// Returns `UnknownVid` when no character is indexed by the supplied VID.
    pub fn find_by_vid(&self, vid: Vid) -> Result<&Character, CharacterManagerError> {
        self.characters
            .get(&vid)
            .ok_or(CharacterManagerError::UnknownVid(vid))
    }

    /// Finds a character by virtual identifier for mutation.
    ///
    /// The write-side twin of [`CharacterManager::find_by_vid`]. A record a client sends
    /// names no character, so the descriptor's own VID is the only key a client-driven
    /// change can use.
    ///
    /// # Errors
    /// Returns `UnknownVid` when no character is indexed by the supplied VID.
    pub fn find_by_vid_mut(&mut self, vid: Vid) -> Result<&mut Character, CharacterManagerError> {
        self.characters
            .get_mut(&vid)
            .ok_or(CharacterManagerError::UnknownVid(vid))
    }

    /// Finds a player by persistent identifier.
    ///
    /// # Errors
    /// Returns `UnknownPlayerId` when no player is indexed by the supplied PID.
    pub fn find_by_pid(&self, player_id: CharacterId) -> Result<&Character, CharacterManagerError> {
        let vid = self
            .by_player_id
            .get(&player_id)
            .copied()
            .ok_or(CharacterManagerError::UnknownPlayerId(player_id))?;
        self.find_by_vid(vid)
    }

    /// Finds a player by ASCII case-insensitive name.
    ///
    /// # Errors
    /// Returns `UnknownPlayerName` when no player is indexed by the supplied name.
    pub fn find_player_by_name(&self, name: &str) -> Result<&Character, CharacterManagerError> {
        let canonical_name = name.to_ascii_lowercase();
        let vid = self
            .by_player_name
            .get(&canonical_name)
            .copied()
            .ok_or_else(|| CharacterManagerError::UnknownPlayerName(name.to_owned()))?;
        self.find_by_vid(vid)
    }

    /// Finds a player character by name for mutation.
    ///
    /// The write-side twin of [`CharacterManager::find_player_by_name`], and the reason the
    /// name lookup is not open-coded in the caller: the name index is keyed by the
    /// lower-cased name, so a caller that iterated the character map instead would
    /// match case-sensitively and disagree with the rule that names are unique
    /// **regardless** of case.
    ///
    /// Returns `None` for an unknown name and for a name whose character is no longer
    /// indexed, which is the same answer for both and needs no second question asked.
    pub fn find_player_mut(&mut self, name: &str) -> Option<&mut Character> {
        let vid = *self.by_player_name.get(&name.to_ascii_lowercase())?;
        self.characters.get_mut(&vid)
    }

    /// Destroys a character and removes all manager-owned memberships.
    ///
    /// # Errors
    /// Returns `UnknownVid` when the character was already absent.
    pub fn destroy(&mut self, vid: Vid) -> Result<(), CharacterManagerError> {
        let character = self
            .characters
            .remove(&vid)
            .ok_or(CharacterManagerError::UnknownVid(vid))?;

        match character.kind() {
            CharacterKind::NonPlayer => {}
            CharacterKind::Player => {
                let _removed_player = self.by_player_id.remove(&character.player_id());
                let canonical_name = character.name().to_ascii_lowercase();
                let _removed_name = self.by_player_name.remove(&canonical_name);
            }
        }
        self.active.remove(&vid);
        Ok(())
    }

    /// Returns the number of owned characters.
    pub fn len(&self) -> usize {
        self.characters.len()
    }

    /// Returns whether the manager owns no characters.
    pub fn is_empty(&self) -> bool {
        self.characters.is_empty()
    }

    /// Returns the number of characters in the active state-update set.
    pub fn active_len(&self) -> usize {
        self.active.len()
    }

    /// Processes one caller-supplied pulse over an active-membership snapshot.
    pub fn process_pulse(&mut self, pulse: u64) -> UpdateReport {
        self.process_pulse_with(pulse, |_| {})
    }

    /// Processes one pulse and exposes each snapshot member to a synchronous visitor.
    pub fn process_pulse_with(
        &mut self,
        pulse: u64,
        mut visit: impl FnMut(&mut Character),
    ) -> UpdateReport {
        let active = self.active.iter().copied().collect::<Vec<_>>();
        let mut pending_destruction = Vec::new();
        let mut updated = 0;

        for vid in active {
            if let Some(character) = self.characters.get_mut(&vid) {
                visit(character);
                if character.update(pulse) {
                    updated += 1;
                }
                if character.destruction_requested() {
                    pending_destruction.push(vid);
                }
            }
        }

        let mut destroyed = 0;
        for vid in pending_destruction {
            if self.destroy(vid).is_ok() {
                destroyed += 1;
            }
        }

        UpdateReport { updated, destroyed }
    }

    fn allocate_vid(&mut self) -> Result<Vid, CharacterManagerError> {
        let raw = u32::try_from(self.next_vid).map_err(|_| CharacterManagerError::VidExhausted)?;
        self.next_vid = self
            .next_vid
            .checked_add(1)
            .ok_or(CharacterManagerError::VidExhausted)?;
        Ok(Vid::new(raw))
    }
}

impl Default for CharacterManager {
    fn default() -> Self {
        Self::new()
    }
}
