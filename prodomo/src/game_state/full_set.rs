//! The GM's kit lines: `/item_full_set`, `/attr_full_set` and `/full_set`
//! (`ACMD(do_item_full_set)`, `do_attr_full_set`, `do_full_set`,
//! `G/cmd_gm.cpp:4760-4902`, `:4904-5019`, `:4723-4732`).
//!
//! `item_full_set` strips the six first wear cells and the shield (`UnequipItem`),
//! then makes eight fresh pieces for the caller's job and puts each on where
//! `FindEquipCell` says (`CreateItem` + `EquipTo`), in the order the legacy arms
//! run them, skipping what has no prototype or cannot go on. `attr_full_set`
//! clears and forces five attributes on each of the eight worn pieces, in the
//! order the legacy arms read them (head, weapon, shield, body, foots, wrist,
//! neck, ear), each forced pair updating the client's cell (`UpdatePacket`) and
//! saving the row (`Save`). `full_set` runs the skill-master line first (ledger
//! 342), then the two halves above in order.
//!
//! The kit tables are the legacy arms' vnums: the sura's 11699/13049/15189/189/
//! 12529/14109/17209/16209, the warrior's 11299/13049/15189/3159/12249/14109/
//! 17109/16109, the shaman's 11899/13049/15189/7159/12669/14109/17209/16209 and
//! the assassin's 11499/13049/15189/1139/12389/14109/17189/16189. The attribute
//! tables are the legacy arms' `(APPLY_*, value)` pairs. A job past the shaman's
//! runs neither arm, as the legacy switches' missing `default` does.

use common::vid::Vid;
use world::character::{wear_full_set, ItemChange, MoveDone, MoveKind};
use world::item::Item;

use super::GameState;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

/// The eight kit pieces per job, in the order `do_item_full_set` makes them
/// (`G/cmd_gm.cpp:4777-4901`). The rows run `JOB_WARRIOR` (0) to `JOB_SHAMAN` (3)
/// (`C/length.h:329-332`).
const KIT: [[u32; 8]; 4] = [
    // `JOB_WARRIOR`.
    [11299, 13049, 15189, 3159, 12249, 14109, 17109, 16109],
    // `JOB_ASSASSIN`.
    [11499, 13049, 15189, 1139, 12389, 14109, 17189, 16189],
    // `JOB_SURA`.
    [11699, 13049, 15189, 189, 12529, 14109, 17209, 16209],
    // `JOB_SHAMAN`.
    [11899, 13049, 15189, 7159, 12669, 14109, 17209, 16209],
];

/// The wear cells `do_attr_full_set` reads, in the order it reads them
/// (`G/cmd_gm.cpp:4914-5009`): head, weapon, shield, body, foots, wrist, neck,
/// ear.
const FORCED_CELLS: [common::enums::EWearPositions; 8] = [
    common::enums::EWearPositions::Head,
    common::enums::EWearPositions::Weapon,
    common::enums::EWearPositions::Shield,
    common::enums::EWearPositions::Body,
    common::enums::EWearPositions::Foots,
    common::enums::EWearPositions::Wrist,
    common::enums::EWearPositions::Neck,
    common::enums::EWearPositions::Ear,
];

/// The five forced `(APPLY_*, value)` pairs per cell, in `FORCED_CELLS` order
/// (`G/cmd_gm.cpp:4919-5005`).
const FORCED: [[(u8, i16); 5]; 8] = [
    // Head: attack speed, HP/SP regen, dodge, steal SP.
    [(7, 8), (10, 30), (11, 30), (28, 15), (24, 10)],
    // Weapon: cast speed, critical, penetrate, devil, strength.
    [(9, 20), (15, 10), (16, 10), (22, 20), (5, 12)],
    // Shield: constitution, block, reflect, stun and slow immunity.
    [(3, 12), (27, 15), (39, 10), (48, 1), (49, 1)],
    // Body: HP, cast speed, steal HP, reflect, attack grade.
    [(1, 2000), (9, 20), (23, 10), (39, 10), (53, 50)],
    // Foots: HP/SP, move speed, attack speed, critical.
    [(1, 2000), (2, 80), (8, 8), (7, 8), (15, 10)],
    // Wrist: HP/SP, penetrate, steal HP, mana burn.
    [(1, 2000), (2, 80), (16, 10), (23, 10), (25, 10)],
    // Neck: HP/SP, critical, penetrate, steal SP.
    [(1, 2000), (2, 80), (15, 10), (16, 10), (24, 10)],
    // Ear: move speed, mana burn, poison reduce, devil, undead.
    [(8, 20), (25, 10), (41, 5), (22, 20), (21, 20)],
];

impl GameState {
    /// Run `/item_full_set` for the character online under `vid`: the strips and
    /// the kit for its job, or nothing when nobody is online there.
    ///
    /// # Errors
    ///
    /// As [`GameState::move_item`].
    pub fn item_full_set(&mut self, vid: Vid, actor: Mover) -> Result<MovedItems, MoveItemRefused> {
        let Some(job) = self.bodies.get(&vid).map(|body| body.card.job) else {
            return Err(MoveItemRefused::NoSuchCharacter { vid });
        };
        let Some(kit) = KIT.get(usize::from(job)) else {
            return self.empty_full_set(vid, actor, MoveKind::FullSet);
        };
        self.run_item_step(vid, actor, |items, mut ids, rules, gear, protos| {
            let Some(gear) = gear else {
                return Err(world::character::MoveRefused::NotPorted(
                    world::character::Unported::Equipment,
                ));
            };
            let done = wear_full_set(items, gear, rules.usable_cells, kit, |vnum| {
                let proto = protos.get(vnum)?;
                let id = ids.as_mut()?.allocate().ok()?;
                let mut item = Item::new(id, vnum);
                let _ = item.set_size(u8::try_from(proto.size).unwrap_or(0));
                Some(item)
            });
            Ok(done)
        })
    }

    /// Run `/attr_full_set` for the character online under `vid`: the five forced
    /// pairs on each worn piece, or nothing when nobody is online there.
    ///
    /// # Errors
    ///
    /// As [`GameState::move_item`].
    pub fn attr_full_set(&mut self, vid: Vid, actor: Mover) -> Result<MovedItems, MoveItemRefused> {
        self.run_item_step(vid, actor, |items, _ids, _rules, _gear, _protos| {
            let mut records = Vec::new();
            let mut changes = Vec::new();
            for (cell, forced) in FORCED_CELLS.iter().zip(FORCED.iter()) {
                let pos = world::character::wear_pos(*cell as u16);
                let Some(worn) = items.item_at(pos).map(|item| item.id) else {
                    continue;
                };
                // `ClearAttribute` writes the five slots without sending or
                // saving; each `SetForceAttribute` then sends and saves.
                for index in 0..5 {
                    let _ = items.set_attribute(
                        worn,
                        index,
                        protocol::gc_item_window::ItemAttribute::new(0, 0),
                    );
                }
                for (index, (b_type, s_value)) in forced.iter().enumerate() {
                    let done = world::character::force_worn_attribute(
                        items,
                        worn,
                        (index, *b_type, *s_value),
                    );
                    records.extend(done.records);
                    changes.extend(done.changes);
                }
            }
            // Every forced pair updated the stored attributes; the store takes
            // the last write per item, which is the full forced row.
            let mut seen = std::collections::BTreeSet::new();
            let mut merged: Vec<ItemChange> = Vec::new();
            for change in changes.into_iter().rev() {
                if let ItemChange::Attributes { id, .. } = change {
                    if seen.insert(id) {
                        merged.push(change);
                    }
                } else {
                    merged.push(change);
                }
            }
            merged.reverse();
            Ok(MoveDone {
                kind: MoveKind::Attributed,
                records,
                changes: merged,
            })
        })
    }

    /// Run `/full_set` for the character online under `vid`: the skill-master line
    /// first, then the kit and its attributes, with every record in legacy's order.
    /// Answers the master's planted skills for the store with the moved items.
    ///
    /// # Errors
    ///
    /// As [`GameState::move_item`].
    pub fn full_set(
        &mut self,
        vid: Vid,
        actor: Mover,
    ) -> Result<(Vec<super::SetSkillLanded>, MovedItems), MoveItemRefused> {
        let planted = self.run_all_skill_master(vid);
        let mut kit = self.item_full_set(vid, actor)?;
        let forced = self.attr_full_set(vid, actor)?;
        kit.records.extend(forced.records);
        kit.changes.extend(forced.changes);
        kit.around.extend(forced.around);
        if forced.points.is_some() {
            kit.points = forced.points;
        }
        if forced.quickslots.is_some() {
            kit.quickslots = forced.quickslots;
        }
        Ok((planted, kit))
    }

    /// The answer when a kit line runs for a job with no kit: no records, no rows.
    fn empty_full_set(
        &mut self,
        vid: Vid,
        actor: Mover,
        kind: MoveKind,
    ) -> Result<MovedItems, MoveItemRefused> {
        self.run_item_step(vid, actor, |_items, _ids, _rules, _gear, _protos| {
            Ok(MoveDone {
                kind,
                records: Vec::new(),
                changes: Vec::new(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::fixtures::{a_world, enter, TestClock};
    use common::enums::EWearPositions;
    use common::item_slots::{EWindows, INVENTORY_MAX_NUM};
    use protocol::gc_item_window::{HEADER_GC_ITEM_SET, HEADER_GC_ITEM_UPDATE};
    use protocol::item_pos::ItemPos;
    use world::character::MoveKind as Kind;

    use db::items::RowChange;

    const VID: u32 = 0x6001;

    const fn mover() -> Mover {
        Mover {
            recently_fought: false,
            empire: 1,
            language: 1,
            pk_mode: crate::loading_phase::PK_MODE_PEACE,
            affect_flags: [0; 2],
            dead: false,
            killer_mode: false,
            alignment: 0,
            view_guild: (0, 0),
            premium: (0, 0),
        }
    }

    fn kitted(job: u8) -> GameState {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _ids = world
            .install_item_ids(world::item::ItemIdRange::new(1, 1_000_000, 1).unwrap())
            .unwrap();
        let _inbox = enter(&mut world, VID, (100, 100), 12_000);
        world.bodies.get_mut(&Vid::new(VID)).unwrap().card.job = job;
        world
    }

    fn worn(state: &GameState, wear: EWearPositions) -> Option<(u32, u32)> {
        let character = state.characters.find_by_vid(Vid::new(VID)).unwrap();
        let item = character.items().item_at(ItemPos::new(
            EWindows::Inventory as u8,
            INVENTORY_MAX_NUM + wear as u16,
        ))?;
        Some((item.id, item.vnum))
    }

    #[test]
    fn the_warrior_s_kit_lands_on_its_cells() {
        let mut world = kitted(0);
        let moved = world.item_full_set(Vid::new(VID), mover()).unwrap();
        assert_eq!(moved.kind, Kind::FullSet);
        // Body, shield, foots, weapon, head, wrist, ear, neck. The shield's file
        // cell says `WEAR_UNIQUE`, which the reader swaps to `WEARABLE_SHIELD`,
        // so it lands on the shield cell.
        assert_eq!(
            worn(&world, EWearPositions::Body).map(|(_, v)| v),
            Some(11299)
        );
        assert_eq!(
            worn(&world, EWearPositions::Weapon).map(|(_, v)| v),
            Some(3159)
        );
        assert_eq!(
            worn(&world, EWearPositions::Head).map(|(_, v)| v),
            Some(12249)
        );
        assert_eq!(
            worn(&world, EWearPositions::Shield).map(|(_, v)| v),
            Some(13049)
        );
        // Eight pieces made: eight `Created` rows, each stored under `EQUIPMENT`.
        let created = moved
            .changes
            .iter()
            .filter_map(|change| match change {
                RowChange::Created(row) => Some(row),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(created.len(), 8);
        assert!(created
            .iter()
            .all(|row| row.window_type == EWindows::Equipment as u8));
        assert!(moved
            .records
            .iter()
            .any(|record| record[0] == HEADER_GC_ITEM_SET));
    }

    #[test]
    fn each_job_s_kit_lands_its_eight() {
        for (job, body) in [(1, 11499), (2, 11699), (3, 11899)] {
            let mut world = kitted(job);
            let moved = world.item_full_set(Vid::new(VID), mover()).unwrap();
            assert_eq!(
                worn(&world, EWearPositions::Body).map(|(_, v)| v),
                Some(body)
            );
            let created = moved
                .changes
                .iter()
                .filter(|change| matches!(change, RowChange::Created(_)))
                .count();
            assert_eq!(created, 8, "job {job} makes eight pieces");
        }
    }

    #[test]
    fn the_forcing_pairs_land_on_each_worn_piece() {
        let mut world = kitted(0);
        let _kit = world.item_full_set(Vid::new(VID), mover()).unwrap();
        let moved = world.attr_full_set(Vid::new(VID), mover()).unwrap();
        assert_eq!(moved.kind, Kind::Attributed);
        // Eight pieces forced: one `Attributes` row each, forty updates sent.
        let forced = moved
            .changes
            .iter()
            .filter(|change| matches!(change, RowChange::Attributes { .. }))
            .count();
        assert_eq!(forced, 8);
        let updates = moved
            .records
            .iter()
            .filter(|record| record[0] == HEADER_GC_ITEM_UPDATE)
            .count();
        assert_eq!(updates, 40);
        // The head's five pairs.
        let character = world.characters.find_by_vid(Vid::new(VID)).unwrap();
        let head = character
            .items()
            .item_at(ItemPos::new(
                EWindows::Inventory as u8,
                INVENTORY_MAX_NUM + EWearPositions::Head as u16,
            ))
            .unwrap();
        let pairs: Vec<(u8, i16)> = head.attributes[..5]
            .iter()
            .map(|attr| (attr.b_type, attr.s_value))
            .collect();
        assert_eq!(pairs, vec![(7, 8), (10, 30), (11, 30), (28, 15), (24, 10)]);
        assert!(head.attributes[5..].iter().all(|attr| attr.b_type == 0));
    }

    #[test]
    fn the_full_one_masters_kits_and_forces_in_order() {
        let mut world = kitted(2);
        let (planted, moved) = world.full_set(Vid::new(VID), mover()).unwrap();
        assert!(!planted.is_empty());
        assert!(planted.iter().all(|set| set.level == 40));
        assert_eq!(
            worn(&world, EWearPositions::Body).map(|(_, v)| v),
            Some(11699)
        );
        let updates = moved
            .records
            .iter()
            .filter(|record| record[0] == HEADER_GC_ITEM_UPDATE)
            .count();
        assert_eq!(updates, 40);
        // The kit's sets lead the forcing's updates.
        let first_set = moved
            .records
            .iter()
            .position(|record| record[0] == HEADER_GC_ITEM_SET)
            .unwrap();
        let first_update = moved
            .records
            .iter()
            .position(|record| record[0] == HEADER_GC_ITEM_UPDATE)
            .unwrap();
        assert!(first_set < first_update);
    }

    #[test]
    fn a_job_past_the_shaman_s_runs_nothing() {
        let mut world = kitted(9);
        let moved = world.item_full_set(Vid::new(VID), mover()).unwrap();
        assert!(moved.records.is_empty());
        assert!(moved.changes.is_empty());
    }
}
