//! The automatic potions: the `USE_AUTO` vnums' toggle, and the affect tick's top-up
//! (`AutoRecoveryItemProcess`, `G/char_item.cpp:9892-10020`, called from `UpdateAffect`,
//! `G/char_affect.cpp:213-214`).
//!
//! # What legacy does
//!
//! Using an automatic potion (72723-72730, 76004/76005, 76021/76022,
//! `G/char_item.cpp:5334-5510`) is refused on an arena map (`[LS;403]`), while fighting a
//! consent duel with the `no_potions_on_pvp` event flag up and the vnum not on the allowed
//! list (`[LS;1205]`), and within a second of the last automatic use (`WAIT_BUFF`, whose
//! locale key is missing everywhere, so the wire carries `@0949WAIT_BUFF`). Past the gates
//! a stack splits one single off into the first free base cell (nowhere to split is
//! `[LS;1130]`; the single carries socket 1 only), and the single toggles: with no row of
//! the type it lays `AddAffect(type, bonus, 4, id, INF, 0)` — the X potions also lay
//! `POINT_MAX_HP/SP_PCT` — locks, sets socket 0 and runs the process at once; the same
//! item toggles off (unlocked, socket 0 cleared, the row removed); another item's row is
//! stood down first, then the new one lays. The row's flag is the item's id, in whose low
//! bits the affect flags land exactly as legacy's `TAffectFlag::Set` puts them (an id past
//! `AFF_BITS_MAX` sets nothing on either side).
//!
//! Each affect tick runs the process for the HP row then the SP row: a dead, stunned, or
//! stun-affected PC, or one carrying a stun-flagged 5/20/107/50 skill row, is skipped; a
//! missing or socket-0-less item removes the row; on an arena map, or fighting a duel with
//! the flag up and the vnum disallowed, the item is unlocked, socket 0 cleared and the row
//! removed. Else the missing pool — maximum less pool less pending recovery — pays into
//! the recovery, with the `SE_AUTO_HPUP`/`SE_AUTO_SPUP` effect around (19/20).
//!
//! Everything here runs on the game thread only (ADR-0002).

use common::point_slot as point;
use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_vid::GcSpecialEffect;
use protocol::item_pos::ItemPos;
use world::character::{toggle_auto_potion, AutoBranch, MoveDone, MoveKind, MoveRefused};

use super::affect::Affect;
use super::GameState;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};
use crate::loading_phase::{
    AFFECT_AUTO_HP_RECOVERY, AFFECT_AUTO_SP_RECOVERY, AFFECT_STUN, AFF_STUN,
    INFINITE_AFFECT_DURATION,
};

/// The automatic hit-point potions (`G/unique_item.h:87-90`, `:103-104`): S, M, L, X and
/// the two reward-box singles. The X also lays the maximum bonus.
const AUTO_HP: [u32; 6] = [72723, 72724, 72725, 72726, 76021, 76022];
/// The automatic spell-point potions (`:92-95`, `:100-101`).
const AUTO_SP: [u32; 6] = [72727, 72728, 72729, 72730, 76004, 76005];
/// The X potions, which lay the maximum bonus with the row.
const AUTO_X: [u32; 2] = [72726, 72730];

/// `SE_AUTO_HPUP` (`common/length.h:871`): 19, counted from `SE_NONE`.
const SE_AUTO_HPUP: u8 = 19;
/// `SE_AUTO_SPUP` (`:872`): 20.
const SE_AUTO_SPUP: u8 = 20;

/// The stun skills whose stun-flagged row stills the process, in legacy's order
/// (`G/char_item.cpp:9910-9920`): 5, 20, 107 and 50 (`G/skill.h:57`, `:66`, `:120`, `:84`).
const STUN_SKILLS: [u32; 4] = [5, 20, 107, 50];

/// The vnums the duel-potion gate lets through (`IsAllowedPotionOnPVP`,
/// `G/arena.cpp:1105-1124`): the blue potions and the automatic blue potions.
const PVP_ALLOWED: [u32; 10] = [
    27004, 27005, 27006, 39040, 39041, 39042, 72727, 72728, 72729, 72730,
];

/// The use throttle's quest flag (`G/char_item.cpp:5365`).
const LAST_USE_FLAG: &[u8] = b"auto_recovery.last_use_time";

/// `[LS;403]`: the arena-map refusal.
const ARENA_REFUSAL: &[u8] = b"[LS;403]";
/// `[LS;1205]`: the fighting refusal.
const PVP_REFUSAL: &[u8] = b"[LS;1205]";
/// `[LS;1130]`: a stack with nowhere to split.
const NO_ROOM_REFUSAL: &[u8] = b"[LS;1130]";
/// `LC_TEXT("WAIT_BUFF")` with its second: the key is in no locale file (the main table
/// and all 12 country files were swept), so europe's locale lookup answers `@0949` plus
/// the key (`G/locale.cpp:244-255`), and the format carries no `%d`, so the second is
/// ignored.
const WAIT_BUFF: &[u8] = b"@0949WAIT_BUFF";

/// The row's apply value (`AddAffect(type, bonus, 4, ...)`).
const AUTO_APPLY_VALUE: i32 = 4;

impl GameState {
    /// The automatic-potion row the item at `at` of the character under `vid` toggles:
    /// its kind and whether it is an X, by vnum alone, as legacy's switch does.
    /// `None` for anything else.
    pub(super) fn auto_at(&self, vid: Vid, at: ItemPos) -> Option<(u32, bool)> {
        let vnum = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .map(|item| item.vnum)?;
        if AUTO_HP.contains(&vnum) {
            Some((AFFECT_AUTO_HP_RECOVERY, AUTO_X.contains(&vnum)))
        } else if AUTO_SP.contains(&vnum) {
            Some((AFFECT_AUTO_SP_RECOVERY, AUTO_X.contains(&vnum)))
        } else {
            None
        }
    }

    /// Whether the automatic-potion gates let `vnum` through for `vid`: not on an arena
    /// map, and not fighting a consent duel with the `no_potions_on_pvp` event flag up
    /// (`questlib_extra.lua` sets it at load) unless the vnum is allowed. Tells legacy's
    /// line on a refusal.
    fn auto_gates_open(&self, vid: Vid, vnum: u32, mover: Mover) -> bool {
        if self
            .place_of(vid.raw())
            .is_some_and(|(_, map)| self.is_arena_map(map))
        {
            self.tell(vid, mover, CHAT_TYPE_INFO, ARENA_REFUSAL, &[]);
            return false;
        }
        let no_potions = self
            .quests
            .as_ref()
            .is_some_and(|quests| quests.manager().event_flag(b"no_potions_on_pvp") != 0);
        let fighting = self
            .characters
            .find_by_vid(vid)
            .ok()
            .is_some_and(|character| self.pvp_is_fighting(character.player_id()));
        if no_potions && fighting && !PVP_ALLOWED.contains(&vnum) {
            self.tell(vid, mover, CHAT_TYPE_INFO, PVP_REFUSAL, &[]);
            return false;
        }
        true
    }

    /// The use throttle (`G/char_item.cpp:5363-5378`): a use within a second of the last
    /// hears the wait line; else the flag takes now. A character whose quest row never
    /// ran passes, as legacy's `NULL` PC does.
    fn auto_throttle_open(&mut self, vid: Vid, mover: Mover) -> bool {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return true;
        };
        let pid = character.player_id();
        let Some(quests) = self.quests.as_mut() else {
            return true;
        };
        let last = quests.manager().quest_flag(pid, LAST_USE_FLAG).unwrap_or(0);
        let now = i32::try_from(Self::global_time()).unwrap_or(i32::MAX);
        if now.wrapping_sub(last) < 1 {
            self.tell(vid, mover, CHAT_TYPE_INFO, WAIT_BUFF, &[]);
            return false;
        }
        quests.manager_mut().set_quest_flag(pid, LAST_USE_FLAG, now);
        true
    }

    /// An empty use: the refused shape, with the points and quickslots the step would
    /// have carried, for a use legacy's gates stop with the item kept.
    fn auto_nothing_used(&self, vid: Vid, actor: Mover) -> Result<MovedItems, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let mut moved = MovedItems::new(
            character.player_id(),
            vid.raw(),
            MoveDone {
                kind: MoveKind::Used,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        );
        moved.points = character.points().cloned();
        moved.quickslots = Some(character.quickslots().clone());
        Ok(moved)
    }

    /// `CG_ITEM_USE` of an automatic potion (`G/char_item.cpp:5334-5510`): the gates, the
    /// throttle, the item half (a stack splits, the single toggles), the row's toggle or
    /// swap, and the immediate process. Answers the rows for the store.
    pub(super) fn use_auto_item(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
        kind: u32,
        special: bool,
    ) -> Result<MovedItems, MoveItemRefused> {
        let vnum = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .map_or(0, |item| item.vnum);
        if !self.auto_gates_open(vid, vnum, actor) {
            return self.auto_nothing_used(vid, actor);
        }
        if !self.auto_throttle_open(vid, actor) {
            return self.auto_nothing_used(vid, actor);
        }
        let row = self
            .affect_events
            .get(&vid)
            .and_then(|event| event.affects.iter().find(|row| row.kind == kind).copied());
        let held = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .map(|item| item.id);
        let branch = match (row, held) {
            (Some(row), Some(held)) if row.flag == held => AutoBranch::ToggleOff,
            (Some(row), _) => AutoBranch::Swap { old: row.flag },
            _ => AutoBranch::Fresh,
        };
        let mut toggled = 0;
        let mut moved = self.run_item_step(vid, actor, |items, ids, rules, _, _| {
            toggle_auto_potion(items, ids, rules, at, branch).map(|(done, id)| {
                toggled = id;
                done
            })
        });
        if matches!(
            moved,
            Err(MoveItemRefused::Refused(MoveRefused::NoRoomInInventory))
        ) {
            self.tell(vid, actor, CHAT_TYPE_INFO, NO_ROOM_REFUSAL, &[]);
            moved = self.auto_nothing_used(vid, actor);
        }
        let mut moved = moved?;
        if branch == AutoBranch::ToggleOff {
            // The remove frame rides the outbox behind the answer's Update: a command's
            // direct frames always land after its answer (`finish_item_step` sends the
            // answer straight to the socket), so legacy's remove-first order is out of
            // reach here, as it is for the blend's frames.
            let _off = self.remove_pc_affect(vid, kind);
            return Ok(moved);
        }
        let apply_on = if special {
            if kind == AFFECT_AUTO_HP_RECOVERY {
                point::POINT_MAX_HP_PCT
            } else {
                point::POINT_MAX_SP_PCT
            }
        } else {
            point::POINT_NONE
        };
        let row = Affect {
            kind,
            apply_on: u8::try_from(apply_on).unwrap_or(0),
            value: AUTO_APPLY_VALUE,
            flag: toggled,
            remaining: INFINITE_AFFECT_DURATION,
            sp_cost: 0,
        };
        // `add_pc_affect` computes, flags and sends (`G/char_affect.cpp:668-760`); the
        // override retires the swapped row's frame first, as the `RemoveAffect` does.
        self.add_pc_affect(vid, row, true);
        // The gates just passed, so the immediate process cannot strip; a strip folds
        // into the step all the same, so every mutation answers its rows.
        if let Some(stripped) = self.auto_recovery_process(vid, kind) {
            let pid = moved.owner_id;
            let packed =
                MovedItems::new(pid, vid.raw(), stripped, actor, &self.protos, &self.locale);
            moved.records.extend(packed.records);
            moved.changes.extend(packed.changes);
        }
        self.start_affect_event(vid);
        // The process paid recovery past the step: the save snapshot follows it.
        if let Ok(character) = self.characters.find_by_vid(vid) {
            moved.points = character.points().cloned();
        }
        Ok(moved)
    }

    /// `AutoRecoveryItemProcess` of `kind` for `vid` (`G/char_item.cpp:9892-10020`): the
    /// dead, the stunned and the stun-marked are skipped; a missing or stood-down item
    /// removes the row; an arena map or a flagged duel strips it; else the missing pool
    /// pays into the recovery with the effect around. Answers the strip's step, if it
    /// stripped, which the caller sends and orders, as the fishing timeout does.
    fn auto_recovery_process(&mut self, vid: Vid, kind: u32) -> Option<world::character::MoveDone> {
        if kind != AFFECT_AUTO_HP_RECOVERY && kind != AFFECT_AUTO_SP_RECOVERY {
            return None;
        }
        if self.is_dead(vid.raw()) || self.is_stunned(vid.raw()) {
            return None;
        }
        let rows = self
            .affect_events
            .get(&vid)
            .map(|event| event.affects.clone())?;
        if rows.iter().any(|row| row.kind == AFFECT_STUN) {
            return None;
        }
        if rows
            .iter()
            .any(|row| STUN_SKILLS.contains(&row.kind) && row.flag == AFF_STUN)
        {
            return None;
        }
        let row = rows.iter().find(|row| row.kind == kind).copied()?;
        let item = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item(row.flag).cloned());
        let Some(item) = item else {
            let _gone = self.remove_pc_affect(vid, kind);
            return None;
        };
        if item.sockets.first().copied().unwrap_or(0) == 0 {
            let _gone = self.remove_pc_affect(vid, kind);
            return None;
        }
        let on_arena = self
            .place_of(vid.raw())
            .is_some_and(|(_, map)| self.is_arena_map(map));
        let no_potions = self
            .quests
            .as_ref()
            .is_some_and(|quests| quests.manager().event_flag(b"no_potions_on_pvp") != 0);
        let fighting = self
            .characters
            .find_by_vid(vid)
            .ok()
            .is_some_and(|character| self.pvp_is_fighting(character.player_id()));
        if on_arena || (no_potions && fighting && !PVP_ALLOWED.contains(&item.vnum)) {
            let Ok(character) = self.characters.find_by_vid_mut(vid) else {
                return None;
            };
            let _ = character.items_mut().set_locked(item.id, false);
            let stripped =
                world::character::set_item_socket(character.items_mut(), item.id, (0, 0));
            let _gone = self.remove_pc_affect(vid, kind);
            return Some(stripped);
        }
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return None;
        };
        let points = character.points()?;
        let (pending, missing) = if kind == AFFECT_AUTO_HP_RECOVERY {
            (
                point::POINT_HP_RECOVERY,
                points.max_hp() - (points.hp() + points.get_point(point::POINT_HP_RECOVERY)),
            )
        } else {
            (
                point::POINT_SP_RECOVERY,
                points.max_sp() - (points.sp() + points.get_point(point::POINT_SP_RECOVERY)),
            )
        };
        if missing > 0 {
            self.change_pc_point(vid, pending, missing, false);
            let effect = if kind == AFFECT_AUTO_HP_RECOVERY {
                SE_AUTO_HPUP
            } else {
                SE_AUTO_SPUP
            };
            self.send_auto_effect(vid, effect);
        }
        None
    }

    /// The affect tick's automatic top-up, HP then SP (`UpdateAffect`,
    /// `G/char_affect.cpp:213-214`): a strip sends its step and orders its rows, as the
    /// fishing timeout does.
    pub(super) fn run_auto_recovery(&mut self, vid: Vid) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        for kind in [AFFECT_AUTO_HP_RECOVERY, AFFECT_AUTO_SP_RECOVERY] {
            let Some(stripped) = self.auto_recovery_process(vid, kind) else {
                continue;
            };
            let changes = self.send_step(vid, pid, stripped, mover);
            if let Some((channel, map)) = self.place_of(vid.raw()) {
                let _ordered = self.order_the_item_rows(
                    vid,
                    channel,
                    map,
                    changes,
                    crate::item_move::StoreOrder::default(),
                );
            }
        }
    }

    /// `EffectPacket` (`G/char.cpp:7533-7542`) around `vid`.
    fn send_auto_effect(&self, vid: Vid, effect: u8) {
        let mut record = Vec::new();
        GcSpecialEffect {
            effect_type: effect,
            vid: vid.raw(),
        }
        .encode_into(&mut record);
        self.packet_around(super::view::EntityKey::Character(vid.raw()), &record, None);
    }

    /// Lock the automatic-potion item a restored row names (`LoadAffect`,
    /// `G/char_affect.cpp:600-608`).
    pub(super) fn lock_restored_auto_item(&mut self, vid: Vid, id: u32) {
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            let _ = character.items_mut().set_locked(id, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use common::item_slots::EWindows;
    use common::point_slot as point;
    use common::vid::Vid;
    use protocol::item_pos::ItemPos;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::item::{Item, ItemIdRange};

    use super::super::fixtures::{a_world, enter_seeing_with, TestClock, PLACE};
    use super::super::GameState;
    use super::{AFFECT_AUTO_HP_RECOVERY, AFFECT_AUTO_SP_RECOVERY};
    use crate::item_move::Mover;

    const ME: Vid = Vid::new(7);

    fn inventory(cell: u16) -> ItemPos {
        ItemPos::new(EWindows::Inventory as u8, cell)
    }

    fn auto(id: u32, vnum: u32, count: u16) -> Item {
        let mut item = Item::new(id, vnum);
        item.set_size(1).unwrap();
        item.count = count;
        item
    }

    const fn mover() -> Mover {
        Mover {
            recently_fought: false,
            empire: 1,
            language: 1,
            pk_mode: crate::loading_phase::PK_MODE_PEACE,
            affect_flags: [0; 2],
            dead: false,
            alignment: 0,
            view_guild: (0, 0),
            premium: (0, 0),
        }
    }

    /// A warrior online on [`PLACE`] holding `items`, hurt by `hurt` in both pools, its
    /// inbox drained.
    fn a_drinker(
        items: &[(ItemPos, Item)],
        hurt: (i32, i32),
    ) -> (TestClock, GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        world
            .install_item_ids(ItemIdRange::new(1, 1_000_000, 100).unwrap())
            .unwrap();
        let (mut inbox, _) = enter_seeing_with(&mut world, PLACE, 7, (3200, 3200), 820, items);
        world.change_pc_point(ME, point::POINT_HP, -hurt.0, false);
        world.change_pc_point(ME, point::POINT_SP, -hurt.1, false);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        (clock, world, inbox)
    }

    fn row_of(world: &GameState, kind: u32) -> Option<super::super::affect::Affect> {
        world
            .affect_events
            .get(&ME)
            .and_then(|event| event.affects.iter().find(|row| row.kind == kind).copied())
    }

    fn recovery_of(world: &GameState, slot: usize) -> i32 {
        world
            .characters
            .find_by_vid(ME)
            .unwrap()
            .points()
            .unwrap()
            .get_point(slot)
    }

    fn inbox_has(inbox: &mut UnboundedReceiver<Vec<u8>>, needle: &[u8]) -> bool {
        std::iter::from_fn(|| inbox.try_recv().ok())
            .any(|frame| frame.windows(needle.len()).any(|window| window == needle))
    }

    /// A use lays the row with the item's id, locks, lights socket 0 and pays the
    /// missing pool into the recovery with the effect around.
    #[test]
    fn a_use_lays_the_row_locks_and_pays_the_missing_pool() {
        let items = [(inventory(3), auto(7, 72_723, 1))];
        let (_clock, mut world, mut inbox) = a_drinker(&items, (500, 300));
        let (kind, special) = world.auto_at(ME, inventory(3)).expect("an auto potion");
        let moved = world.use_auto_item(ME, inventory(3), mover(), kind, special);
        assert!(moved.is_ok(), "{moved:?}");
        let row = row_of(&world, AFFECT_AUTO_HP_RECOVERY).expect("the row lays");
        assert_eq!(row.flag, 7);
        assert_eq!(row.value, super::AUTO_APPLY_VALUE);
        let item = world
            .characters
            .find_by_vid(ME)
            .unwrap()
            .items()
            .item(7)
            .cloned()
            .unwrap();
        assert_eq!(item.sockets[0], 1);
        assert!(item.locked);
        assert_eq!(recovery_of(&world, point::POINT_HP_RECOVERY), 500);
        assert!(
            inbox_has(&mut inbox, &[super::SE_AUTO_HPUP]),
            "the effect goes around"
        );
    }

    /// The same item's second use stands it down and removes the row.
    #[test]
    fn the_same_items_second_use_toggles_it_off() {
        let items = [(inventory(3), auto(7, 72_723, 1))];
        let (_clock, mut world, mut inbox) = a_drinker(&items, (500, 300));
        let (kind, special) = world.auto_at(ME, inventory(3)).expect("an auto potion");
        world
            .use_auto_item(ME, inventory(3), mover(), kind, special)
            .expect("toggles on");
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        let (kind, special) = world
            .auto_at(ME, inventory(3))
            .expect("still an auto potion");
        world
            .use_auto_item(ME, inventory(3), mover(), kind, special)
            .expect("toggles off");
        assert!(row_of(&world, AFFECT_AUTO_HP_RECOVERY).is_none());
        let item = world
            .characters
            .find_by_vid(ME)
            .unwrap()
            .items()
            .item(7)
            .cloned()
            .unwrap();
        assert_eq!(item.sockets[0], 0);
        assert!(!item.locked);
    }

    /// A stack splits one single off; the single toggles, the stack stays dark.
    #[test]
    fn a_stack_splits_one_single_off_and_the_single_toggles() {
        let items = [(inventory(3), auto(7, 72_727, 3))];
        let (_clock, mut world, _inbox) = a_drinker(&items, (500, 300));
        let (kind, special) = world.auto_at(ME, inventory(3)).expect("an auto potion");
        let moved = world.use_auto_item(ME, inventory(3), mover(), kind, special);
        assert!(moved.is_ok(), "{moved:?}");
        let row = row_of(&world, AFFECT_AUTO_SP_RECOVERY).expect("the row lays");
        assert_eq!(row.flag, 100);
        let held = world.characters.find_by_vid(ME).unwrap().items().clone();
        assert_eq!(held.item(7).unwrap().count, 2);
        assert_eq!(held.item(7).unwrap().sockets[0], 0);
        assert_eq!(held.item(100).unwrap().sockets[0], 1);
        assert_eq!(recovery_of(&world, point::POINT_SP_RECOVERY), 300);
    }

    /// The affect tick tops the recovery up again after the drain spends it.
    #[test]
    fn the_tick_tops_the_recovery_up() {
        let items = [(inventory(3), auto(7, 72_723, 1))];
        let (_clock, mut world, mut inbox) = a_drinker(&items, (500, 300));
        let (kind, special) = world.auto_at(ME, inventory(3)).expect("an auto potion");
        world
            .use_auto_item(ME, inventory(3), mover(), kind, special)
            .expect("toggles on");
        world.run_affect_events(25);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        let points = world
            .characters
            .find_by_vid(ME)
            .unwrap()
            .points()
            .unwrap()
            .clone();
        let missing = points.max_hp() - (points.hp() + points.get_point(point::POINT_HP_RECOVERY));
        assert_eq!(missing, 0, "the tick leaves nothing missing");
        assert!(row_of(&world, AFFECT_AUTO_HP_RECOVERY).is_some());
    }
    /// The X lays the maximum bonus with the row.
    #[test]
    fn the_x_lays_the_maximum_bonus_with_the_row() {
        let items = [(inventory(3), auto(7, 72_726, 1))];
        let (_clock, mut world, _inbox) = a_drinker(&items, (500, 300));
        let (kind, special) = world.auto_at(ME, inventory(3)).expect("an auto potion");
        assert!(special, "the X is special");
        world
            .use_auto_item(ME, inventory(3), mover(), kind, special)
            .expect("toggles on");
        let row = row_of(&world, AFFECT_AUTO_HP_RECOVERY).expect("the row lays");
        assert_eq!(
            row.apply_on,
            u8::try_from(point::POINT_MAX_HP_PCT).unwrap(),
            "the maximum bonus"
        );
    }

    /// A stack with nowhere to split hears the no-room line and keeps its count.
    #[test]
    fn a_stack_with_nowhere_to_split_hears_the_no_room_line() {
        let mut held: Vec<(ItemPos, Item)> = (0..89)
            .map(|cell| (inventory(cell), auto(1000 + u32::from(cell), 72_723, 1)))
            .collect();
        held.push((inventory(89), auto(7, 72_727, 3)));
        let (_clock, mut world, mut inbox) = a_drinker(&held, (500, 300));
        let (kind, special) = world.auto_at(ME, inventory(89)).expect("an auto potion");
        let moved = world.use_auto_item(ME, inventory(89), mover(), kind, special);
        assert!(
            moved.is_ok(),
            "the refusal is a told nothing-used: {moved:?}"
        );
        assert!(row_of(&world, AFFECT_AUTO_SP_RECOVERY).is_none());
        let stack = world
            .characters
            .find_by_vid(ME)
            .unwrap()
            .items()
            .item(7)
            .cloned()
            .unwrap();
        assert_eq!(stack.count, 3);
        assert!(inbox_has(&mut inbox, b"[LS;1130]"), "the no-room line");
    }

    /// The second item of the type stands the first down and takes the row.
    #[test]
    fn the_second_item_of_the_type_swaps_the_row_over() {
        let held = [
            (inventory(3), auto(7, 72_723, 1)),
            (inventory(4), auto(9, 72_724, 1)),
        ];
        let (_clock, mut world, _inbox) = a_drinker(&held, (500, 300));
        let (kind, special) = world.auto_at(ME, inventory(3)).expect("an auto potion");
        world
            .use_auto_item(ME, inventory(3), mover(), kind, special)
            .expect("toggles on");
        let (kind, special) = world.auto_at(ME, inventory(4)).expect("an auto potion");
        world
            .use_auto_item(ME, inventory(4), mover(), kind, special)
            .expect("swaps");
        let row = row_of(&world, AFFECT_AUTO_HP_RECOVERY).expect("the row stays");
        assert_eq!(row.flag, 9);
        let items = world.characters.find_by_vid(ME).unwrap().items().clone();
        assert_eq!(items.item(7).unwrap().sockets[0], 0);
        assert!(!items.item(7).unwrap().locked);
        assert_eq!(items.item(9).unwrap().sockets[0], 1);
        assert!(items.item(9).unwrap().locked);
    }

    /// On an arena map the use is refused with the arena line and lays no row.
    #[test]
    fn on_an_arena_map_the_use_hears_the_arena_line() {
        let items = [(inventory(3), auto(7, 72_723, 1))];
        let (_clock, mut world, mut inbox) = a_drinker(&items, (500, 300));
        world.load_arenas(&[quest::host::Arena {
            map_index: PLACE.1,
            start_a: (32, 32),
            start_b: (34, 32),
        }]);
        let (kind, special) = world.auto_at(ME, inventory(3)).expect("an auto potion");
        let moved = world.use_auto_item(ME, inventory(3), mover(), kind, special);
        assert!(
            moved.is_ok(),
            "the refusal is a told nothing-used: {moved:?}"
        );
        assert!(row_of(&world, AFFECT_AUTO_HP_RECOVERY).is_none());
        assert!(inbox_has(&mut inbox, b"[LS;403]"), "the arena line");
    }

    /// Warping onto an arena map with the row up strips it on the next process: the
    /// item stands down and the row goes.
    #[test]
    fn the_process_strips_a_row_carried_onto_an_arena_map() {
        let items = [(inventory(3), auto(7, 72_723, 1))];
        let (_clock, mut world, mut inbox) = a_drinker(&items, (500, 300));
        let (kind, special) = world.auto_at(ME, inventory(3)).expect("an auto potion");
        world
            .use_auto_item(ME, inventory(3), mover(), kind, special)
            .expect("toggles on");
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        world.load_arenas(&[quest::host::Arena {
            map_index: PLACE.1,
            start_a: (32, 32),
            start_b: (34, 32),
        }]);
        world.run_auto_recovery(ME);
        assert!(row_of(&world, AFFECT_AUTO_HP_RECOVERY).is_none());
        let item = world
            .characters
            .find_by_vid(ME)
            .unwrap()
            .items()
            .item(7)
            .cloned()
            .unwrap();
        assert_eq!(item.sockets[0], 0);
        assert!(!item.locked);
    }

    /// The duel gate's allowed list is legacy's nine cases plus the X (`IsAllowedPotionOnPVP`,
    /// `G/arena.cpp:1105-1124`).
    #[test]
    fn the_duel_gate_lets_the_blue_potions_through() {
        assert_eq!(
            super::PVP_ALLOWED,
            [27004, 27005, 27006, 39040, 39041, 39042, 72727, 72728, 72729, 72730]
        );
    }
    /// At full pools the use lays the row but pays nothing and shows no effect.
    #[test]
    fn at_full_pools_the_use_lays_the_row_but_pays_nothing() {
        let items = [(inventory(3), auto(7, 72_723, 1))];
        let (_clock, mut world, mut inbox) = a_drinker(&items, (0, 0));
        let (kind, special) = world.auto_at(ME, inventory(3)).expect("an auto potion");
        world
            .use_auto_item(ME, inventory(3), mover(), kind, special)
            .expect("toggles on");
        assert!(row_of(&world, AFFECT_AUTO_HP_RECOVERY).is_some());
        assert_eq!(recovery_of(&world, point::POINT_HP_RECOVERY), 0);
        let effects: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok())
            .filter(|frame| frame.first() == Some(&protocol::gc_vid::HEADER_GC_SEPCIAL_EFFECT))
            .collect();
        assert!(effects.is_empty(), "no effect at full pools");
    }

    /// The SP X lays the spell-point maximum bonus.
    #[test]
    fn the_sp_x_lays_the_sp_maximum_bonus() {
        let items = [(inventory(3), auto(7, 72_730, 1))];
        let (_clock, mut world, _inbox) = a_drinker(&items, (500, 300));
        let (kind, special) = world.auto_at(ME, inventory(3)).expect("an auto potion");
        assert_eq!(kind, AFFECT_AUTO_SP_RECOVERY);
        assert!(special, "the SP X is special");
        world
            .use_auto_item(ME, inventory(3), mover(), kind, special)
            .expect("toggles on");
        let row = row_of(&world, AFFECT_AUTO_SP_RECOVERY).expect("the row lays");
        assert_eq!(
            row.apply_on,
            u8::try_from(point::POINT_MAX_SP_PCT).unwrap(),
            "the SP maximum bonus"
        );
    }
    /// A strip of the last row ends the event that same tick, when nothing is owed.
    #[test]
    fn a_strip_of_the_last_row_ends_the_event_that_same_tick() {
        let items = [(inventory(3), auto(7, 72_723, 1))];
        let (_clock, mut world, mut inbox) = a_drinker(&items, (0, 0));
        let (kind, special) = world.auto_at(ME, inventory(3)).expect("an auto potion");
        world
            .use_auto_item(ME, inventory(3), mover(), kind, special)
            .expect("toggles on");
        assert!(world.affect_events.contains_key(&ME), "the event runs");
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        world.load_arenas(&[quest::host::Arena {
            map_index: PLACE.1,
            start_a: (32, 32),
            start_b: (34, 32),
        }]);
        world.run_affect_events(25);
        assert!(row_of(&world, AFFECT_AUTO_HP_RECOVERY).is_none());
        assert!(
            !world.affect_events.contains_key(&ME),
            "the stripped event ends with nothing owed"
        );
    }
}
