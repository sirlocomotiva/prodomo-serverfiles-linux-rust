use common::item_slots::INVENTORY_MAX_EXTENDED;
use common::{vid::Vid, CharacterId};

use super::combat::{Damage, DamageOutcome, Vitality};
use super::items::CharacterItems;
use super::points::Points;
use super::quickslot::Quickslots;
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
    items: CharacterItems,
    envanter: u16,
    points: Option<Points>,
    quickslots: Quickslots,
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
            items: CharacterItems::new(),
            envanter: 0,
            points: None,
            quickslots: Quickslots::default(),
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
            items: CharacterItems::new(),
            envanter: 0,
            points: None,
            quickslots: Quickslots::default(),
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

    /// This character's four item windows.
    ///
    /// `pItems`, `pDSItems`, `pAttr67AddItem` and `pSwitchbotItems` (`char.h:458-478`)
    /// are members of `CHARACTER`, not of a side table, so they live here too. The
    /// store holds item **ids**; the items themselves belong to whoever owns the
    /// id, which is the character, so a character with no item table could not be
    /// given anything.
    pub const fn items(&self) -> &CharacterItems {
        &self.items
    }

    /// This character's item windows, mutably.
    pub fn items_mut(&mut self) -> &mut CharacterItems {
        &mut self.items
    }

    /// The player's points (`m_points` and `m_pointsInstant`), or `None` for a character that
    /// was admitted without them.
    pub const fn points(&self) -> Option<&Points> {
        self.points.as_ref()
    }

    /// Sets the player's points.
    pub fn set_points(&mut self, points: Option<Points>) {
        self.points = points;
    }

    /// The item windows and the points together, which wearing an item changes both of.
    pub fn items_and_points_mut(&mut self) -> (&mut CharacterItems, Option<&mut Points>) {
        (&mut self.items, self.points.as_mut())
    }

    /// The character's quickslots (`m_quickslot`).
    pub const fn quickslots(&self) -> &Quickslots {
        &self.quickslots
    }

    /// Sets the character's quickslots, the way the load's `SetQuickslot` calls leave them.
    pub fn set_quickslots(&mut self, quickslots: Quickslots) {
        self.quickslots = quickslots;
    }

    /// The item windows, read, and the quickslots, which a client's item slot is checked
    /// against.
    pub fn items_and_quickslots_mut(&mut self) -> (&CharacterItems, &mut Quickslots) {
        (&self.items, &mut self.quickslots)
    }

    /// Returns the character's `Inven_Point`, the `m_points.envanter` that
    /// `char.h:1284` exposes as `Inven_Point()`.
    ///
    /// This is not the inventory size. The base inventory is always
    /// [`common::item_slots::INVENTORY_MAX_NUM`] cells in the array, and this stat decides how many
    /// of them are usable, through
    /// [`common::item_slots::usable_inventory_cells`]. A fresh character
    /// has 0, which buys the legacy default 90 cells, so a character that has
    /// never spent the stat is not locked out of its own first page.
    pub const fn inven_point(&self) -> u16 {
        self.envanter
    }

    /// Sets the character's `Inven_Point`.
    ///
    /// The value is clamped at [`INVENTORY_MAX_EXTENDED`], the largest stat the
    /// base inventory can express. Legacy copies `m_points.envanter` straight out
    /// of the stored blob and never clamps, and
    /// [`common::item_slots::usable_inventory_cells`] is a
    /// bare sum, so a hand-edited row can name a cell past the end of the base
    /// inventory and into the equipment window. Clamping on write is Divergence
    /// 202.1: the alternative is reproducing a cell index the client cannot
    /// draw.
    pub fn set_inven_point(&mut self, value: u16) {
        self.envanter = value.min(INVENTORY_MAX_EXTENDED);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::grant::{grant, GrantRefused};
    use crate::item::Item;
    use common::vid::Vid;

    /// A one-cell item, which is the smallest footprint a grant will place.
    fn one_cell(id: u32) -> Item {
        let mut item = Item::new(id, 30_000);
        item.set_size(1).expect("a size of at least one");
        item
    }

    #[test]
    fn a_new_character_holds_an_empty_item_table() {
        let character = Character::new(Vid::new(1));
        assert!(
            character.items().is_empty(),
            "a fresh character owns no items"
        );
    }

    #[test]
    fn a_live_character_can_be_given_an_item_and_keeps_it() {
        // This is the seam the grant unit was missing: `CharacterItems` existed, but
        // nothing owned one, so nothing could be given to anything.
        let mut character = Character::new(Vid::new(1));
        let mut item = one_cell(1);
        let placed =
            grant(character.items_mut(), &mut item, &[], 0).expect("an empty inventory has room");
        assert_eq!(placed.pos.cell, 0);
        assert_eq!(
            character.items().len(),
            1,
            "the character keeps what it was given"
        );
    }

    #[test]
    fn two_grants_to_one_character_take_two_cells() {
        let mut character = Character::new(Vid::new(1));
        let mut first = one_cell(1);
        let mut second = one_cell(2);
        let a = grant(character.items_mut(), &mut first, &[], 0).expect("cell 0 is free");
        let b = grant(character.items_mut(), &mut second, &[], 0).expect("cell 1 is free");
        assert_eq!((a.pos.cell, b.pos.cell), (0, 1));
        assert_eq!(character.items().len(), 2);
    }

    #[test]
    fn a_refused_grant_leaves_the_character_as_it_was() {
        let mut character = Character::new(Vid::new(1));
        for cell in 0..180 {
            character
                .items_mut()
                .set(
                    protocol::item_pos::ItemPos {
                        window_type: common::item_slots::EWindows::Inventory as u8,
                        cell,
                    },
                    &one_cell(1_000 + u32::from(cell)),
                )
                .expect("a free base cell takes an item");
        }
        let mut item = one_cell(9_999);
        let refused = grant(character.items_mut(), &mut item, &[], 0)
            .expect_err("180 one-cell items fill the base inventory");
        assert_eq!(refused, GrantRefused::NoRoom { size: 1 });
        assert_eq!(
            character.items().len(),
            180,
            "a refused grant does not add a phantom item"
        );
    }
}
