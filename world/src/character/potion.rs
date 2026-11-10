//! Potions: the `USE_POTION_NODELAY` and `USE_POTION` arms of `UseItemEx`
//! (`G/char_item.cpp:5579-5755`), the `SetCount(GetCount() - 1)` both end with
//! (`CItem::SetCount`, `G/item.cpp:296-330`), and the hit and spell point part of the affect
//! event `USE_POTION` starts (`CHARACTER::UpdateAffect`, `G/char_affect.cpp:165-200`).
//!
//! A `USE_POTION_NODELAY` potion changes the pools at once. A `USE_POTION` potion adds to
//! `POINT_HP_RECOVERY` or `POINT_SP_RECOVERY`, and [`update_recovery`] moves at most 7% of the
//! maximum a second from there into the pool, which the caller runs once a second while
//! [`is_recovering`] holds.
//!
//! The last potion of a stack syncs its slots as `SetCount(0)` does (ledger 220): a
//! `USE_POTION` potion's slots follow the next stack of its vnum, and a `USE_POTION_NODELAY`
//! potion's are deleted.
//!
//! Arena admission and accepted-use counts run in the game state's item dispatch. Not ported:
//! the `PvP` potion limits, dungeon and guild war hooks, moon cake timer and continuous
//! recovery affects. The rest of the affect event runs in the game state.
//!
//! The automatic potions' item half is [`toggle_auto_potion`] (ledger 558); their gates,
//! rows and tick run in the game state's `auto_potion`.

use common::point_slot as point;
use gamedata::item_kind::{USE_POTION, USE_POTION_NODELAY};
use gamedata::item_proto::ItemProto;

use super::equip::Trail;
use super::item_move::{
    declined, done, ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord, MoveRefused, MoveRules,
    Unported,
};
use super::items::CharacterItems;
use super::points::{PointRecord, Points};
use super::quickslot::{chains_when_used_up, QuickslotSync, SyncTo};
use crate::item::ItemIds;
use crate::item::{gc_item_clear, Item};
use protocol::item_pos::ItemPos;

/// `SE_HPUP_RED` (`common/length.h`): the red effect a hit point potion shows.
pub const SE_HPUP_RED: u8 = 1;
/// `SE_SPUP_BLUE`: the blue effect a spell point potion shows.
pub const SE_SPUP_BLUE: u8 = 2;

/// Which automatic-potion branch the use takes (`G/char_item.cpp:5434-5504`): a fresh
/// toggle-on, the same item's toggle-off, or a swap that first stands the old item down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoBranch {
    /// No row of the type holds: the item toggles on.
    Fresh,
    /// The row's item is this one: it toggles off.
    ToggleOff,
    /// Another item holds the row: it stands down first, then this one toggles on.
    Swap {
        /// The standing-down item's id.
        old: u32,
    },
}

/// The item half of an automatic potion's use (`G/char_item.cpp:5408-5504`): the old
/// item, if any, is unlocked with its socket 0 cleared; a stack splits one single off
/// into the first free base cell (carrying socket 1 only, `CreateItem` fresh otherwise);
/// and the single toggles its socket 0 and lock on — or off, when it already holds the
/// row. Answers the step and the toggled item's id, which the affect names as its flag.
///
/// # Errors
///
/// [`MoveRefused::Empty`] at an empty cell, [`MoveRefused::NoItemIds`] with no allocator,
/// [`MoveRefused::IdsExhausted`] past the range, and [`MoveRefused::NoRoomInInventory`]
/// when a stack has nowhere to split (the use then hears `[LS;1130]`).
pub fn toggle_auto_potion(
    items: &mut CharacterItems,
    ids: Option<&mut ItemIds>,
    rules: &MoveRules,
    at: ItemPos,
    branch: AutoBranch,
) -> Result<(MoveDone, u32), MoveRefused> {
    let mut done = MoveDone {
        kind: MoveKind::Used,
        records: Vec::new(),
        changes: Vec::new(),
    };
    let item = items.item_at(at).cloned().ok_or(MoveRefused::Empty)?;
    if branch == AutoBranch::ToggleOff {
        stand_auto_down(items, item.id, &mut done);
        return Ok((done, item.id));
    }
    let id = if item.count > 1 {
        let cell = items
            .find_free_inventory_cell(rules.usable_cells, item.size)
            .ok_or(MoveRefused::NoRoomInInventory)?;
        let ids = ids.ok_or(MoveRefused::NoItemIds)?;
        let single = ids.allocate().map_err(|_| MoveRefused::IdsExhausted)?;
        let mut piece = Item::new(single, item.vnum);
        piece.flags = item.flags;
        piece.anti_flags = item.anti_flags;
        piece.size = item.size;
        piece.sockets[1] = item.sockets[1];
        let pos = ItemPos::new(at.window_type, cell);
        items.set(pos, &piece).map_err(MoveRefused::Storage)?;
        let left = item.count - 1;
        if let Err(reason) = items.set_count(item.id, u32::from(left)) {
            let _ = items.release(single);
            return Err(MoveRefused::Count(reason));
        }
        if let Some(source) = items.item(item.id) {
            done.records.push(MoveRecord::Item(ItemRecord::Update(
                source.gc_item_update(),
            )));
        }
        done.changes.push(ItemChange::Count {
            id: item.id,
            count: left,
        });
        if let Some(placed) = items.item(single) {
            done.records.push(MoveRecord::Item(ItemRecord::Set(
                placed.gc_item_set(pos, 0),
            )));
        }
        let placed = items.item(single).cloned().ok_or(MoveRefused::Count(
            super::items::CountRefused::NotHeld(single),
        ))?;
        done.changes.push(ItemChange::Created(placed));
        single
    } else {
        item.id
    };
    // The split precedes the old item's stand-down, as legacy splits before it finds the
    // row (`:5419-5440` against `:5442`): a roomless split leaves the old item lit. A
    // missing old item is silently fine (`NULL != old`, `:5468`).
    if let AutoBranch::Swap { old } = branch {
        stand_auto_down(items, old, &mut done);
    }
    let sockets = items.set_socket(id, 0, 1).map_err(|_| MoveRefused::Empty)?;
    let _ = items.set_locked(id, true);
    if let Some(item) = items.item(id) {
        done.records
            .push(MoveRecord::Item(ItemRecord::Update(item.gc_item_update())));
    }
    done.changes.push(ItemChange::Sockets { id, sockets });
    Ok((done, id))
}

/// Stand an automatic potion down: unlocked, its socket 0 cleared, the cell updated.
/// An item the character does not hold is silently skipped, as legacy's `NULL` old one
/// is (`G/char_item.cpp:5468-5473`).
fn stand_auto_down(items: &mut CharacterItems, id: u32, done: &mut MoveDone) {
    let Ok(sockets) = items.set_socket(id, 0, 0) else {
        return;
    };
    let _ = items.set_locked(id, false);
    if let Some(item) = items.item(id) {
        done.records
            .push(MoveRecord::Item(ItemRecord::Update(item.gc_item_update())));
    }
    done.changes.push(ItemChange::Sockets { id, sockets });
}

/// The share of the maximum one second of recovery may restore, in percent.
const RECOVERY_PER_SECOND_PCT: i64 = 7;

/// The cap `USE_POTION` puts on `100 + POINT_POTION_BONUS`.
const POTION_BONUS_CAP: i32 = 200;

/// The gold bars, whose arm is not ported (`G/char_item.cpp:3334-3369`).
const GOLD_BARS: core::ops::RangeInclusive<u32> = 80_003..=80_008;

/// The ability potions, whose arm answers every sub type but `USE_POTION_NODELAY` in its own
/// way (`G/char_item.cpp:3371-3521`).
const ABILITY_POTIONS: core::ops::RangeInclusive<u32> = 50_801..=50_820;

/// Whether `CG_ITEM_USE` on an item of this prototype runs [`use_potion`].
#[must_use]
pub(super) fn is_potion(proto: &ItemProto) -> bool {
    proto.item_type == gamedata::item_kind::ITEM_USE
        && (proto.sub_type == USE_POTION || proto.sub_type == USE_POTION_NODELAY)
}

/// The `ITEM_USE` arm of `UseItemEx` for a potion: its effect on `points`, then one of its
/// stack used up.
///
/// # Errors
///
/// [`MoveRefused::NotPorted`] for a gold bar and for an ability potion's other sub types.
/// [`MoveRefused::NothingToRecover`] when the potion changed nothing: a `USE_POTION_NODELAY`
/// potion whose pools are all full, or a `USE_POTION` one whose pool the recovery already
/// fills. A `USE_POTION` potion that recovers both pools and refuses only the second keeps
/// the first recovery and the potion, as legacy's does.
pub(super) fn use_potion(
    items: &mut CharacterItems,
    item: &Item,
    proto: &ItemProto,
    points: &mut Points,
) -> Result<MoveDone, MoveRefused> {
    if GOLD_BARS.contains(&item.vnum)
        || (ABILITY_POTIONS.contains(&item.vnum) && proto.sub_type != USE_POTION_NODELAY)
    {
        return Err(MoveRefused::NotPorted(Unported::UseVnum(item.vnum)));
    }
    let mut trail = Trail::default();
    let used = if proto.sub_type == USE_POTION_NODELAY {
        drink_at_once(proto, points, &mut trail)
    } else {
        if let Err(refused) = start_recovery(proto, points, &mut trail) {
            return declined(refused, trail);
        }
        true
    };
    if !used {
        return Err(MoveRefused::NothingToRecover);
    }
    use_one(items, item, chains_when_used_up(proto), &mut trail)?;
    Ok(done(MoveKind::Used, trail))
}

/// `USE_POTION_NODELAY`: each value whose pool is not full changes it at once. Answers whether
/// any did.
fn drink_at_once(proto: &ItemProto, points: &mut Points, trail: &mut Trail) -> bool {
    let bonus = 100 + points.get_point(point::POINT_POTION_BONUS);
    let [hp, sp, _, hp_pct, sp_pct, _] = proto.values;
    let mut used = false;
    for (value, scale, pool) in [
        (hp, bonus, Pool::Hp),
        (sp, bonus, Pool::Sp),
        (hp_pct, pool_max(points, Pool::Hp), Pool::Hp),
        (sp_pct, pool_max(points, Pool::Sp), Pool::Sp),
    ] {
        if value == 0 || pool_now(points, pool) >= pool_max(points, pool) {
            continue;
        }
        change(points, pool.kind(), percent_of(value, scale), trail);
        trail.records.push(MoveRecord::Effect(pool.effect()));
        used = true;
    }
    used
}

/// `USE_POTION`: the spell point value, then the hit point value, each added to its recovery
/// unless the recovery already fills the pool.
fn start_recovery(
    proto: &ItemProto,
    points: &mut Points,
    trail: &mut Trail,
) -> Result<(), MoveRefused> {
    let bonus = (100 + points.get_point(point::POINT_POTION_BONUS)).min(POTION_BONUS_CAP);
    for (value, pool) in [(proto.values[1], Pool::Sp), (proto.values[0], Pool::Hp)] {
        if value == 0 {
            continue;
        }
        let pending = points.get_point(pool.recovery());
        if i64::from(pending) + i64::from(pool_now(points, pool))
            >= i64::from(pool_max(points, pool))
        {
            return Err(MoveRefused::NothingToRecover);
        }
        change(points, pool.recovery(), percent_of(value, bonus), trail);
        trail.records.push(MoveRecord::Effect(pool.effect()));
    }
    Ok(())
}

/// `item->SetCount(item->GetCount() - 1)`: the last of a stack leaves the cell and the store,
/// and a larger stack shrinks in place.
///
/// The last one syncs its slots (`G/item.cpp:303-330`). When `chains`, it is cleared first and
/// its slots then follow the first item of its vnum; otherwise its slots are deleted before it
/// is cleared.
pub(super) fn use_one(
    items: &mut CharacterItems,
    item: &Item,
    chains: bool,
    trail: &mut Trail,
) -> Result<(), MoveRefused> {
    if item.count <= 1 {
        let pos = items.release(item.id).map_err(MoveRefused::Storage)?;
        let clear = MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos)));
        let from = item.pos.cell;
        if chains {
            trail.records.push(clear);
            let to = SyncTo::Chain(item.vnum);
            trail
                .records
                .push(MoveRecord::QuickslotSync(QuickslotSync { from, to }));
        } else {
            let to = SyncTo::Delete;
            trail
                .records
                .push(MoveRecord::QuickslotSync(QuickslotSync { from, to }));
            trail.records.push(clear);
        }
        trail.changes.push(ItemChange::Destroyed { id: item.id });
        return Ok(());
    }
    let count = item.count - 1;
    items
        .set_count(item.id, u32::from(count))
        .map_err(MoveRefused::Count)?;
    let kept = items.item(item.id).ok_or(MoveRefused::Empty)?;
    trail
        .records
        .push(MoveRecord::Item(ItemRecord::Update(kept.gc_item_update())));
    trail.changes.push(ItemChange::Count { id: item.id, count });
    Ok(())
}

/// Whether the character has recovery left for [`update_recovery`] to move.
#[must_use]
pub fn is_recovering(points: &Points) -> bool {
    points.get_point(point::POINT_HP_RECOVERY) != 0
        || points.get_point(point::POINT_SP_RECOVERY) != 0
}

/// One second of `UpdateAffect`'s recovery: for each pool with recovery left, a full pool
/// drops the recovery, and otherwise at most 7% of the maximum moves from the recovery into
/// the pool; then each pool's flat keep pays in whole (`G/char_affect.cpp:201-210`).
/// Answers the point records, in the order legacy writes them.
pub fn update_recovery(points: &mut Points) -> Vec<PointRecord> {
    let mut records = Vec::new();
    for pool in [Pool::Hp, Pool::Sp] {
        let pending = points.get_point(pool.recovery());
        if pending <= 0 {
            continue;
        }
        let max = pool_max(points, pool);
        if max <= pool_now(points, pool) {
            let cleared = points.point_change(pool.recovery(), -pending, false, false);
            records.extend(cleared.unwrap_or_default());
            continue;
        }
        let step = i64::from(pending).min(i64::from(max) * RECOVERY_PER_SECOND_PCT / 100);
        let step = i32::try_from(step).unwrap_or(pending);
        for (kind, amount) in [(pool.kind(), step), (pool.recovery(), -step)] {
            records.extend(
                points
                    .point_change(kind, amount, false, false)
                    .unwrap_or_default(),
            );
        }
    }
    // `POINT_HP/SP_RECOVER_CONTINUE` (`G/char_affect.cpp:201-210`): past the pools, the
    // flat keep pays into its pool whole, unclamped by any maximum but the pool's own.
    for pool in [Pool::Hp, Pool::Sp] {
        let keep = points.get_point(pool.keep());
        if keep > 0 {
            records.extend(
                points
                    .point_change(pool.kind(), keep, false, false)
                    .unwrap_or_default(),
            );
        }
    }
    records
}

/// `PointChange(kind, amount)`, with its records on the trail. A slot the Rewrite refuses
/// writes nothing, as legacy's unknown arm does; the pools and recoveries are all ported.
fn change(points: &mut Points, kind: usize, amount: i32, trail: &mut Trail) {
    let records = points
        .point_change(kind, amount, false, false)
        .unwrap_or_default();
    trail
        .records
        .extend(records.into_iter().map(MoveRecord::Point));
}

/// `value * scale / 100` in legacy's integer arithmetic, held in range.
fn percent_of(value: i32, scale: i32) -> i32 {
    let scaled = i64::from(value) * i64::from(scale) / 100;
    i32::try_from(scaled).unwrap_or(if scaled < 0 { i32::MIN } else { i32::MAX })
}

/// One of the two pools a potion fills.
#[derive(Debug, Clone, Copy)]
enum Pool {
    Hp,
    Sp,
}

impl Pool {
    const fn kind(self) -> usize {
        match self {
            Self::Hp => point::POINT_HP,
            Self::Sp => point::POINT_SP,
        }
    }

    const fn recovery(self) -> usize {
        match self {
            Self::Hp => point::POINT_HP_RECOVERY,
            Self::Sp => point::POINT_SP_RECOVERY,
        }
    }

    const fn keep(self) -> usize {
        match self {
            Self::Hp => point::POINT_HP_RECOVER_CONTINUE,
            Self::Sp => point::POINT_SP_RECOVER_CONTINUE,
        }
    }

    const fn effect(self) -> u8 {
        match self {
            Self::Hp => SE_HPUP_RED,
            Self::Sp => SE_SPUP_BLUE,
        }
    }
}

fn pool_now(points: &Points, pool: Pool) -> i32 {
    match pool {
        Pool::Hp => points.hp(),
        Pool::Sp => points.sp(),
    }
}

fn pool_max(points: &Points, pool: Pool) -> i32 {
    match pool {
        Pool::Hp => points.max_hp(),
        Pool::Sp => points.max_sp(),
    }
}

#[cfg(test)]
mod tests {
    use common::item_slots::EWindows;
    use gamedata::item_kind::ITEM_USE;
    use protocol::item_pos::ItemPos;

    use super::*;
    use crate::character::points::PointsRow;
    use crate::item::ItemIdRange;

    const AT: ItemPos = ItemPos {
        window_type: EWindows::Inventory as u8,
        cell: 3,
    };

    fn potion(vnum: u32, sub_type: i32, values: [i32; 6]) -> ItemProto {
        let mut proto = ItemProto::for_category_rule(vnum, ITEM_USE, sub_type);
        proto.values = values;
        proto
    }

    /// A level 10 warrior whose maxima are computed, hurt by `hurt` in both pools.
    fn warrior(hurt: i32) -> Points {
        let mut points = Points::load(&PointsRow {
            race: 0,
            level: 10,
            exp: 0,
            conqueror_level: 0,
            st: 6,
            ht: 4,
            dx: 3,
            iq: 3,
            sungma: [0; 4],
            hp: 10_000,
            sp: 10_000,
            stamina: 820,
            inven_point: 0,
            random_hp: 0,
            random_sp: 0,
            stat_point: 0,
            skill_point: 0,
            sub_skill_point: 0,
            level_step: 0,
            stat_reset_count: 0,
            map_index: 1,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
            playtime: 0,
            conqueror_exp: 0,
            gaya: 0,
            biologist: [0; 3],
            secured: (0, 999_999),
            premium: (0, 0),
            shop_unlocked: 0,
        });
        let _ = points.compute_points();
        for kind in [point::POINT_HP, point::POINT_SP] {
            points
                .point_change(kind, -hurt, false, false)
                .expect("a pool changes");
        }
        points
    }

    fn holding(count: u16, vnum: u32) -> (CharacterItems, Item) {
        let mut item = Item::new(7, vnum);
        item.count = count;
        let mut items = CharacterItems::new();
        items.set(AT, &item).expect("the fixture places");
        let placed = items.item_at(AT).cloned().expect("the potion is placed");
        (items, placed)
    }

    fn effects(done: &MoveDone) -> Vec<u8> {
        done.records
            .iter()
            .filter_map(|record| match record {
                MoveRecord::Effect(effect) => Some(*effect),
                _ => None,
            })
            .collect()
    }

    fn point_kinds(done: &MoveDone) -> Vec<(u8, i64)> {
        done.records
            .iter()
            .filter_map(|record| match record {
                MoveRecord::Point(record) => Some((record.kind, record.value)),
                _ => None,
            })
            .collect()
    }

    fn kind(slot: usize) -> u8 {
        u8::try_from(slot).expect("a point slot fits a byte")
    }

    #[test]
    fn an_instant_potion_fills_both_pools_shows_both_effects_and_uses_one_of_the_stack() {
        let mut points = warrior(300);
        let (hp, sp) = (points.hp(), points.sp());
        let (mut items, item) = holding(5, 27_101);
        let proto = potion(27_101, USE_POTION_NODELAY, [100, 50, 0, 0, 0, 0]);
        let done = use_potion(&mut items, &item, &proto, &mut points).expect("the potion works");
        assert_eq!(done.kind, MoveKind::Used);
        assert_eq!((points.hp(), points.sp()), (hp + 100, sp + 50));
        assert_eq!(effects(&done), [SE_HPUP_RED, SE_SPUP_BLUE]);
        assert_eq!(
            point_kinds(&done),
            [
                (kind(point::POINT_HP), i64::from(hp + 100)),
                (kind(point::POINT_SP), i64::from(sp + 50))
            ]
        );
        let kept = items.item_at(AT).expect("the stack is kept");
        assert_eq!(kept.count, 4);
        assert_eq!(
            done.records.last(),
            Some(&MoveRecord::Item(ItemRecord::Update(kept.gc_item_update())))
        );
        assert_eq!(done.changes, [ItemChange::Count { id: 7, count: 4 }]);
    }

    #[test]
    fn an_instant_potion_takes_the_potion_bonus_and_its_percentages_take_the_maximum() {
        let mut points = warrior(500);
        points
            .point_change(point::POINT_POTION_BONUS, 50, false, false)
            .expect("the bonus is ported");
        let hp = points.hp();
        let (mut items, item) = holding(2, 27_101);
        let proto = potion(27_101, USE_POTION_NODELAY, [100, 0, 0, 0, 0, 0]);
        let _ = use_potion(&mut items, &item, &proto, &mut points).expect("the potion works");
        assert_eq!(points.hp(), hp + 150);

        let (hp, sp) = (points.hp(), points.sp());
        let (hp_cap, sp_cap) = (points.max_hp(), points.max_sp());
        let proto = potion(27_101, USE_POTION_NODELAY, [0, 0, 0, 10, 20, 0]);
        let item = items.item_at(AT).cloned().expect("one is left");
        let done = use_potion(&mut items, &item, &proto, &mut points).expect("the potion works");
        assert_eq!(points.hp(), hp + hp_cap / 10);
        assert_eq!(points.sp(), (sp + sp_cap / 5).min(sp_cap));
        assert_eq!(effects(&done), [SE_HPUP_RED, SE_SPUP_BLUE]);
    }

    #[test]
    fn an_instant_potion_skips_a_full_pool_and_is_kept_when_both_are_full() {
        let mut points = warrior(0);
        let (mut items, item) = holding(3, 27_101);
        let proto = potion(27_101, USE_POTION_NODELAY, [100, 50, 0, 10, 10, 0]);
        let before = points.clone();
        assert_eq!(
            use_potion(&mut items, &item, &proto, &mut points),
            Err(MoveRefused::NothingToRecover)
        );
        assert_eq!(points, before);
        assert_eq!(items.item_at(AT).map(|kept| kept.count), Some(3));

        points
            .point_change(point::POINT_SP, -10, false, false)
            .expect("a pool changes");
        let done = use_potion(&mut items, &item, &proto, &mut points).expect("the potion works");
        assert_eq!(effects(&done), [SE_SPUP_BLUE]);
        assert_eq!(points.sp(), points.max_sp());
    }

    #[test]
    fn the_last_potion_of_a_stack_leaves_its_cell_and_the_store() {
        let mut points = warrior(300);
        let (mut items, item) = holding(1, 27_101);
        let proto = potion(27_101, USE_POTION_NODELAY, [100, 0, 0, 0, 0, 0]);
        let done = use_potion(&mut items, &item, &proto, &mut points).expect("the potion works");
        assert!(items.item_at(AT).is_none());
        assert_eq!(
            done.records.last(),
            Some(&MoveRecord::Item(ItemRecord::Set(gc_item_clear(AT))))
        );
        assert_eq!(done.changes, [ItemChange::Destroyed { id: 7 }]);
    }

    #[test]
    fn the_last_potion_deletes_its_slots_or_chains_them_by_its_sub_type() {
        let clear = MoveRecord::Item(ItemRecord::Set(gc_item_clear(AT)));
        // `USE_POTION_NODELAY` is neither sub type 0 nor 7: the slots go before the clear.
        let (mut items, item) = holding(1, 27_101);
        let proto = potion(27_101, USE_POTION_NODELAY, [100, 0, 0, 0, 0, 0]);
        let done = use_potion(&mut items, &item, &proto, &mut warrior(300)).expect("works");
        let delete = MoveRecord::QuickslotSync(QuickslotSync {
            from: 3,
            to: SyncTo::Delete,
        });
        assert_eq!(done.records[done.records.len() - 2..], [delete, clear]);
        // `USE_POTION`: the clear goes first, and the slots then follow the next stack.
        let (mut items, item) = holding(1, 27_001);
        let slow = potion(27_001, USE_POTION, [100, 0, 0, 0, 0, 0]);
        let done = use_potion(&mut items, &item, &slow, &mut warrior(300)).expect("works");
        let chain = MoveRecord::QuickslotSync(QuickslotSync {
            from: 3,
            to: SyncTo::Chain(27_001),
        });
        assert_eq!(done.records[done.records.len() - 2..], [clear, chain]);
        assert!(chains_when_used_up(&slow));
        // A stack that stays syncs nothing.
        let (mut items, item) = holding(2, 27_101);
        let proto = potion(27_101, USE_POTION_NODELAY, [100, 0, 0, 0, 0, 0]);
        let done = use_potion(&mut items, &item, &proto, &mut warrior(300)).expect("works");
        assert!(!done
            .records
            .iter()
            .any(|record| matches!(record, MoveRecord::QuickslotSync(_))));
    }

    #[test]
    fn a_slow_potion_adds_to_the_recovery_with_the_bonus_capped_at_double() {
        let mut points = warrior(600);
        // `PointChange` caps the bonus at 100, so `MIN(200, ...)` never binds on a bonus it
        // wrote: 150 is held at 100.
        points
            .point_change(point::POINT_POTION_BONUS, 150, false, false)
            .expect("the bonus is ported");
        assert_eq!(points.get_point(point::POINT_POTION_BONUS), 100);
        let (hp, sp) = (points.hp(), points.sp());
        let (mut items, item) = holding(2, 27_001);
        let proto = potion(27_001, USE_POTION, [100, 40, 0, 0, 0, 0]);
        let done = use_potion(&mut items, &item, &proto, &mut points).expect("the potion works");
        assert_eq!((points.hp(), points.sp()), (hp, sp));
        assert_eq!(points.get_point(point::POINT_HP_RECOVERY), 200);
        assert_eq!(points.get_point(point::POINT_SP_RECOVERY), 80);
        assert_eq!(effects(&done), [SE_SPUP_BLUE, SE_HPUP_RED]);
        assert_eq!(
            point_kinds(&done),
            [
                (kind(point::POINT_SP_RECOVERY), 80),
                (kind(point::POINT_HP_RECOVERY), 200)
            ]
        );
        assert_eq!(done.changes, [ItemChange::Count { id: 7, count: 1 }]);
        assert!(is_recovering(&points));
    }

    #[test]
    fn a_slow_potion_whose_recovery_would_fill_the_pool_is_kept() {
        let mut points = warrior(100);
        let (mut items, item) = holding(2, 27_001);
        let proto = potion(27_001, USE_POTION, [100, 0, 0, 0, 0, 0]);
        let _ = use_potion(&mut items, &item, &proto, &mut points).expect("the first works");
        let item = items.item_at(AT).cloned().expect("one is left");
        let before = points.clone();
        assert_eq!(
            use_potion(&mut items, &item, &proto, &mut points),
            Err(MoveRefused::NothingToRecover)
        );
        assert_eq!(points, before);
        assert_eq!(items.item_at(AT).map(|kept| kept.count), Some(1));
    }

    #[test]
    fn a_slow_potion_refused_on_hit_points_keeps_its_spell_point_recovery_and_the_potion() {
        let mut points = warrior(0);
        points
            .point_change(point::POINT_SP, -100, false, false)
            .expect("a pool changes");
        let (mut items, item) = holding(2, 27_001);
        let proto = potion(27_001, USE_POTION, [100, 40, 0, 0, 0, 0]);
        let done = use_potion(&mut items, &item, &proto, &mut points).expect("the first half");
        assert_eq!(done.kind, MoveKind::Declined);
        assert_eq!(points.get_point(point::POINT_SP_RECOVERY), 40);
        assert_eq!(points.get_point(point::POINT_HP_RECOVERY), 0);
        assert_eq!(effects(&done), [SE_SPUP_BLUE]);
        assert!(done.changes.is_empty());
        assert_eq!(items.item_at(AT).map(|kept| kept.count), Some(2));
    }

    #[test]
    fn the_gold_bars_and_the_ability_potions_other_arms_are_not_ported() {
        let mut points = warrior(300);
        for (vnum, sub_type) in [
            (80_003, USE_POTION_NODELAY),
            (80_008, USE_POTION),
            (50_801, USE_POTION),
            (50_820, USE_POTION),
        ] {
            let (mut items, item) = holding(2, vnum);
            let proto = potion(vnum, sub_type, [100, 0, 0, 0, 0, 0]);
            assert_eq!(
                use_potion(&mut items, &item, &proto, &mut points),
                Err(MoveRefused::NotPorted(Unported::UseVnum(vnum)))
            );
        }
        for vnum in [50_800, 50_801, 50_820, 50_821, 80_002, 80_009] {
            let (mut items, item) = holding(2, vnum);
            let proto = potion(vnum, USE_POTION_NODELAY, [10, 0, 0, 0, 0, 0]);
            assert!(use_potion(&mut items, &item, &proto, &mut points).is_ok());
        }
    }

    #[test]
    fn a_second_of_recovery_moves_at_most_seven_percent_and_a_full_pool_drops_the_rest() {
        let mut points = warrior(0);
        let max_hp = points.max_hp();
        let step = max_hp * 7 / 100;
        points
            .point_change(point::POINT_HP, -(step * 2), false, false)
            .expect("a pool changes");
        points
            .point_change(point::POINT_HP_RECOVERY, step * 4, false, false)
            .expect("the recovery is ported");
        let hp = points.hp();
        let records = update_recovery(&mut points);
        assert_eq!(points.hp(), hp + step);
        assert_eq!(points.get_point(point::POINT_HP_RECOVERY), step * 3);
        let kinds: Vec<u8> = records.iter().map(|record| record.kind).collect();
        assert_eq!(
            kinds,
            [kind(point::POINT_HP), kind(point::POINT_HP_RECOVERY)]
        );
        let _ = update_recovery(&mut points);
        assert_eq!(points.hp(), max_hp);
        assert_eq!(points.get_point(point::POINT_HP_RECOVERY), step * 2);
        // A full pool drops the whole of what is left, not one second's share of it.
        let records = update_recovery(&mut points);
        assert_eq!(points.get_point(point::POINT_HP_RECOVERY), 0);
        assert_eq!(records.len(), 1);
        assert!(!is_recovering(&points));
        assert!(update_recovery(&mut points).is_empty());
    }

    #[test]
    fn a_small_recovery_moves_whole_and_the_spell_points_follow_the_hit_points() {
        let mut points = warrior(100);
        for slot in [point::POINT_HP_RECOVERY, point::POINT_SP_RECOVERY] {
            points
                .point_change(slot, 5, false, false)
                .expect("the recovery is ported");
        }
        let (hp, sp) = (points.hp(), points.sp());
        let records = update_recovery(&mut points);
        assert_eq!((points.hp(), points.sp()), (hp + 5, sp + 5));
        let kinds: Vec<u8> = records.iter().map(|record| record.kind).collect();
        assert_eq!(
            kinds,
            [
                kind(point::POINT_HP),
                kind(point::POINT_HP_RECOVERY),
                kind(point::POINT_SP),
                kind(point::POINT_SP_RECOVERY)
            ]
        );
        assert!(!is_recovering(&points));
    }
    const RULES: MoveRules = MoveRules {
        count_limit: 200,
        usable_cells: 90,
        belt_grade: None,
        questing: false,
        secured: false,
    };

    fn ids() -> ItemIds {
        ItemIds::new(ItemIdRange::new(1, 100, 8).expect("the test range holds"))
    }

    fn changes(done: &MoveDone) -> Vec<ItemChange> {
        done.changes.clone()
    }

    #[test]
    fn a_fresh_single_toggles_on_with_its_socket_lit_and_locked() {
        let (mut items, _) = holding(1, 72_723);
        let (done, id) =
            toggle_auto_potion(&mut items, Some(&mut ids()), &RULES, AT, AutoBranch::Fresh)
                .expect("toggles on");
        assert_eq!(id, 7);
        let item = items.item(7).expect("still held");
        assert_eq!(item.sockets[0], 1);
        assert!(item.locked);
        assert_eq!(
            changes(&done),
            vec![ItemChange::Sockets {
                id: 7,
                sockets: item.sockets,
            }]
        );
    }

    #[test]
    fn a_fresh_stack_splits_one_single_off_and_toggles_the_single_on() {
        let (mut items, _) = holding(3, 72_727);
        items.set_socket(7, 1, 900).expect("the socket sticks");
        let (done, id) =
            toggle_auto_potion(&mut items, Some(&mut ids()), &RULES, AT, AutoBranch::Fresh)
                .expect("splits and toggles");
        assert_eq!(id, 8);
        // The `Created` answers the split-time snapshot (socket 0 still dark); the
        // toggle's `Sockets` follows it.
        assert!(matches!(
            &changes(&done)[..],
            [
                ItemChange::Count { id: 7, count: 2 },
                ItemChange::Created(piece),
                ItemChange::Sockets { id: 8, .. },
            ] if piece.id == 8 && piece.count == 1 && piece.sockets == [0, 900, 0, 0, 0, 0]
        ));
        let left = items.item(7).expect("the stack stays");
        assert_eq!(left.count, 2);
        assert_eq!(left.sockets[0], 0);
        let single = items.item(8).expect("the single places");
        assert_eq!(single.count, 1);
        assert_eq!(single.sockets[0], 1);
        assert_eq!(single.sockets[1], 900);
        assert!(single.locked);
    }

    #[test]
    fn the_same_item_toggles_off_unlocked_with_its_socket_dark() {
        let (mut items, _) = holding(1, 72_723);
        items.set_socket(7, 0, 1).expect("the socket sticks");
        let _ = items.set_locked(7, true);
        let (done, id) = toggle_auto_potion(
            &mut items,
            Some(&mut ids()),
            &RULES,
            AT,
            AutoBranch::ToggleOff,
        )
        .expect("toggles off");
        assert_eq!(id, 7);
        let item = items.item(7).expect("still held");
        assert_eq!(item.sockets[0], 0);
        assert!(!item.locked);
        assert_eq!(
            changes(&done),
            vec![ItemChange::Sockets {
                id: 7,
                sockets: item.sockets,
            }]
        );
    }

    #[test]
    fn a_swap_stands_the_old_item_down_before_the_new_one_toggles_on() {
        let mut items = CharacterItems::new();
        let mut old = Item::new(7, 72_723);
        old.sockets[0] = 1;
        items.set(AT, &old).expect("the old places");
        let _ = items.set_locked(7, true);
        let other = ItemPos {
            window_type: EWindows::Inventory as u8,
            cell: 4,
        };
        items
            .set(other, &Item::new(9, 72_724))
            .expect("the new places");
        let (done, id) = toggle_auto_potion(
            &mut items,
            Some(&mut ids()),
            &RULES,
            other,
            AutoBranch::Swap { old: 7 },
        )
        .expect("swaps");
        assert_eq!(id, 9);
        let stood = items.item(7).expect("the old stays");
        assert_eq!(stood.sockets[0], 0);
        assert!(!stood.locked);
        let toggled = items.item(9).expect("the new stays");
        assert_eq!(toggled.sockets[0], 1);
        assert!(toggled.locked);
        assert_eq!(changes(&done).len(), 2);
    }

    #[test]
    fn a_swap_whose_old_item_is_gone_still_toggles_the_new_one_on() {
        let (mut items, _) = holding(1, 72_724);
        let (done, id) = toggle_auto_potion(
            &mut items,
            Some(&mut ids()),
            &RULES,
            AT,
            AutoBranch::Swap { old: 4242 },
        )
        .expect("the missing old is silently fine");
        assert_eq!(id, 7);
        assert_eq!(items.item(7).unwrap().sockets[0], 1);
        assert_eq!(changes(&done).len(), 1);
    }

    #[test]
    fn a_stack_with_nowhere_to_split_is_refused_with_no_room() {
        let (mut items, _) = holding(3, 72_723);
        let cramped = MoveRules {
            usable_cells: 0,
            ..RULES
        };
        let refused = toggle_auto_potion(
            &mut items,
            Some(&mut ids()),
            &cramped,
            AT,
            AutoBranch::Fresh,
        );
        assert_eq!(refused.unwrap_err(), MoveRefused::NoRoomInInventory);
        assert_eq!(items.item(7).unwrap().count, 3);
    }
    #[test]
    fn a_swap_with_nowhere_to_split_leaves_the_old_item_lit() {
        let mut items = CharacterItems::new();
        let mut old = Item::new(7, 72_723);
        old.sockets[0] = 1;
        items.set(AT, &old).expect("the old places");
        let _ = items.set_locked(7, true);
        let other = ItemPos {
            window_type: EWindows::Inventory as u8,
            cell: 4,
        };
        let mut stack = Item::new(9, 72_724);
        stack.count = 3;
        items.set(other, &stack).expect("the stack places");
        let cramped = MoveRules {
            usable_cells: 0,
            ..RULES
        };
        let refused = toggle_auto_potion(
            &mut items,
            Some(&mut ids()),
            &cramped,
            other,
            AutoBranch::Swap { old: 7 },
        );
        assert_eq!(refused.unwrap_err(), MoveRefused::NoRoomInInventory);
        let stood = items.item(7).expect("the old stays");
        assert_eq!(stood.sockets[0], 1, "the split fails before the stand-down");
        assert!(stood.locked);
        assert_eq!(items.item(9).unwrap().count, 3);
    }
}
