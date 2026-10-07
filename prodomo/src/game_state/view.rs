//! The view model: which entities each entity sees, kept as legacy's `CEntity` keeps it.
//!
//! Each map of a Channel is a grid of sectrees (`SECTREE_MAP`), each sectree holds the entities
//! standing in it, and each entity has a view (`m_map_view`): the entities whose insert records
//! it has been sent, with the age of the last recompute that found them. The four verbs
//! (`ViewInsert`, `ViewRemove`, `ViewCleanup`, `ViewReencode`, `G/entity_view.cpp:8-83`) and
//! `UpdateSectree` (`:122-236`) are ported exactly; each returns what it did as an [`Effect`]
//! list, which `view_encode` turns into records.
//!
//! Legacy walks its sectrees and views in an order hashed by pointer. The Rewrite walks the
//! sectrees around a point in legacy's `Build` order and everything else in [`EntityKey`] order,
//! every player by VID and then every NPC by VID (the V1 Divergence, docs/STATUS.md).
//!
//! Everything here runs on the game thread only (ADR-0002).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use gamedata::server_attr::SectreeGrid;

use crate::sync_position::distance_approx;

/// `VIEW_BONUS_RANGE` (`G/config.cpp:124`), added to `VIEW_RANGE` for the view's radius.
pub(super) const VIEW_BONUS_RANGE: i64 = 500;

/// One entity a sectree holds and a view names. The derived order (every player by VID, then
/// every NPC by VID) is the fixed order the Rewrite uses where legacy walks a set hashed by
/// pointer (V1).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) enum EntityKey {
    Character(u32),
    Npc(u32),
}

/// A sectree, by column and row from its map grid's first (`SectreeGrid::sectree_at`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct Tree {
    column: u32,
    row: u32,
}

/// Where an entity stands. `tree` is `None` only for a player that entered at a point no
/// sectree holds (V4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Spot {
    pub(super) tree: Option<Tree>,
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) z: i32,
}

/// `m_map_view` and `m_iViewAge` (`G/entity.h:62`).
#[derive(Debug, Default)]
struct View {
    age: u64,
    seen: BTreeMap<EntityKey, u64>,
}

/// `Insert { of, to }` is `of->EncodeInsertPacket(to)`; `Remove { of, to }` is
/// `of->EncodeRemovePacket(to)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Effect {
    Insert { of: EntityKey, to: EntityKey },
    Remove { of: EntityKey, to: EntityKey },
}

/// `SECTREE_MAP::Build`'s walk of the sectrees around one (`G/sectree_manager.cpp:79-120`): the
/// sectree itself, then the eight `neighbor_coord` steps of (column, row).
const BUILD_ORDER: [(i32, i32); 9] = [
    (0, 0),
    (-1, 0),
    (1, 0),
    (0, -1),
    (0, 1),
    (-1, 1),
    (1, -1),
    (-1, -1),
    (1, 1),
];

/// One map of one Channel: its sectrees, and every entity's spot and view.
#[derive(Debug)]
pub(super) struct MapIndex {
    grid: SectreeGrid,
    /// The entities each sectree holds. A sectree no entity stands in has no entry. Looked up by
    /// key, never iterated.
    trees: HashMap<Tree, BTreeSet<EntityKey>>,
    /// The single store of a position on the map. Looked up by key, never iterated.
    spots: HashMap<EntityKey, Spot>,
    /// Looked up by key, never iterated.
    views: HashMap<EntityKey, View>,
}

impl MapIndex {
    /// An empty map over `grid`.
    pub(super) fn new(grid: SectreeGrid) -> Self {
        Self {
            grid,
            trees: HashMap::new(),
            spots: HashMap::new(),
            views: HashMap::new(),
        }
    }

    /// A map with no sectree at all, for a map the world was given no grid for: every entrant
    /// stands outside every sectree (V4).
    pub(super) fn treeless() -> Self {
        Self::new(SectreeGrid {
            x: 0,
            y: 0,
            columns: 0,
            rows: 0,
        })
    }

    /// The sectree holding a point, or `None` where the map built none.
    pub(super) fn tree_at(&self, x: i32, y: i32) -> Option<Tree> {
        self.grid
            .sectree_at(x, y)
            .map(|(column, row)| Tree { column, row })
    }

    /// Where an entity stands.
    pub(super) fn spot(&self, me: EntityKey) -> Option<Spot> {
        self.spots.get(&me).copied()
    }

    /// The entities `me` sees, in key order, or `None` when `me` stands in no sectree, where
    /// legacy's `PacketAround` sends nothing at all (`G/entity.cpp:95`).
    pub(super) fn viewers(&self, me: EntityKey) -> Option<Vec<EntityKey>> {
        self.spots.get(&me)?.tree?;
        Some(
            self.views
                .get(&me)
                .map(|view| view.seen.keys().copied().collect())
                .unwrap_or_default(),
        )
    }

    /// Whether `me` sees `ent`.
    #[cfg(test)]
    pub(super) fn sees(&self, me: EntityKey, ent: EntityKey) -> bool {
        self.views
            .get(&me)
            .is_some_and(|view| view.seen.contains_key(&ent))
    }

    /// `SECTREE::InsertEntity` without its commented-out `UpdateSectree` (`G/sectree.cpp:137`).
    fn index(&mut self, me: EntityKey, tree: Tree) {
        self.trees.entry(tree).or_default().insert(me);
    }

    /// `SECTREE::RemoveEntity`.
    fn unindex(&mut self, me: EntityKey, tree: Tree) {
        if let Some(keys) = self.trees.get_mut(&tree) {
            keys.remove(&me);
            if keys.is_empty() {
                self.trees.remove(&tree);
            }
        }
    }

    /// `CEntity::ViewInsert` (`G/entity_view.cpp:47-67`). Observers are dormant.
    fn view_insert(
        &mut self,
        me: EntityKey,
        ent: EntityKey,
        recursive: bool,
        out: &mut Vec<Effect>,
    ) {
        if me == ent {
            return;
        }
        let view = self.views.entry(me).or_default();
        let age = view.age;
        if let Some(seen) = view.seen.get_mut(&ent) {
            // Found: the age is refreshed, with no record and no recursion (:53-57).
            *seen = age;
            return;
        }
        view.seen.insert(ent, age);
        out.push(Effect::Insert { of: ent, to: me });
        if recursive {
            self.view_insert(ent, me, false, out);
        }
    }

    /// `CEntity::ViewRemove` (`:69-83`).
    fn view_remove(
        &mut self,
        me: EntityKey,
        ent: EntityKey,
        recursive: bool,
        out: &mut Vec<Effect>,
    ) {
        let removed = self
            .views
            .get_mut(&me)
            .is_some_and(|view| view.seen.remove(&ent).is_some());
        if !removed {
            return;
        }
        out.push(Effect::Remove { of: ent, to: me });
        if recursive {
            self.view_remove(ent, me, false, out);
        }
    }

    /// `CEntity::ViewCleanup` (`:8-21`): every entity `me` sees loses `me`, and `me` is sent
    /// nothing.
    fn view_cleanup(&mut self, me: EntityKey, out: &mut Vec<Effect>) {
        let seen: Vec<EntityKey> = self
            .views
            .get(&me)
            .map(|view| view.seen.keys().copied().collect())
            .unwrap_or_default();
        for ent in seen {
            self.view_remove(ent, me, false, out);
        }
        if let Some(view) = self.views.get_mut(&me) {
            view.seen.clear();
        }
    }

    /// `CEntity::ViewReencode` (`:23-45`): `me`'s own removal and insert, then for each entity
    /// it sees, `me`'s removal and insert to it and its insert to `me`, with no removal before
    /// that last insert (quirk 3).
    fn view_reencode(&self, me: EntityKey, out: &mut Vec<Effect>) {
        out.push(Effect::Remove { of: me, to: me });
        out.push(Effect::Insert { of: me, to: me });
        if let Some(view) = self.views.get(&me) {
            for ent in view.seen.keys().copied() {
                out.push(Effect::Remove { of: me, to: ent });
                out.push(Effect::Insert { of: me, to: ent });
                out.push(Effect::Insert { of: ent, to: me });
            }
        }
    }

    /// The entities of the sectrees around `tree`, in `Build` order and key order inside each
    /// sectree. Legacy collects them before calling anything (`G/sectree.h:79-90`).
    fn around(&self, tree: Tree) -> Vec<EntityKey> {
        let mut keys = Vec::new();
        for (dc, dr) in BUILD_ORDER {
            let (Some(column), Some(row)) = (
                tree.column.checked_add_signed(dc),
                tree.row.checked_add_signed(dr),
            ) else {
                continue;
            };
            if column >= self.grid.columns || row >= self.grid.rows {
                continue;
            }
            if let Some(entities) = self.trees.get(&Tree { column, row }) {
                keys.extend(entities.iter().copied());
            }
        }
        keys
    }

    /// The entities of the sectrees around the one holding `me`, in [`MapIndex::around`] order.
    pub(super) fn around_of(&self, me: EntityKey) -> Vec<EntityKey> {
        self.spots
            .get(&me)
            .and_then(|spot| spot.tree)
            .map(|tree| self.around(tree))
            .unwrap_or_default()
    }

    /// `CEntity::UpdateSectree` (`G/entity_view.cpp:122-236`): insert every entity around within
    /// `radius`, both ways, then drop every entity the recompute did not find, both ways.
    pub(super) fn update_sectree(&mut self, me: EntityKey, radius: i64, out: &mut Vec<Effect>) {
        let Some(spot) = self.spots.get(&me).copied() else {
            return;
        };
        let Some(tree) = spot.tree else {
            // "null sectree" (:124-133).
            return;
        };
        let around = self.around(tree);
        let age = {
            let view = self.views.entry(me).or_default();
            view.age = view.age.wrapping_add(1);
            view.age
        };
        for ent in around {
            // CFuncViewInsert (:97-119); no object is ported, so every entity is range-tested.
            let Some(other) = self.spots.get(&ent).copied() else {
                continue;
            };
            let dx = other.x.saturating_sub(spot.x);
            let dy = other.y.saturating_sub(spot.y);
            if i64::from(distance_approx(dx, dy)) > radius {
                continue;
            }
            self.view_insert(me, ent, true, out);
            // :110-117 starts an NPC's state machine here for a player `me`; `sys.mob.ai`
            // fills the hook (docs/STATUS.md).
        }
        let stale: Vec<EntityKey> = self
            .views
            .get(&me)
            .map(|view| {
                view.seen
                    .iter()
                    .filter(|(_, seen)| **seen < age)
                    .map(|(ent, _)| *ent)
                    .collect()
            })
            .unwrap_or_default();
        for ent in stale {
            // :212-235: the removal to me, the erase, then the other side's removal.
            out.push(Effect::Remove { of: ent, to: me });
            if let Some(view) = self.views.get_mut(&me) {
                view.seen.remove(&ent);
            }
            self.view_remove(ent, me, false, out);
        }
    }

    /// `CHARACTER::Show` (`G/char.cpp:1847-1917`). Returns false, changing nothing, when no
    /// sectree holds the point (`:1849-1854`).
    pub(super) fn show(
        &mut self,
        me: EntityKey,
        (x, y, z): (i32, i32, i32),
        radius: i64,
        out: &mut Vec<Effect>,
    ) -> bool {
        let Some(tree) = self.tree_at(x, y) else {
            return false;
        };
        let old = self.spots.get(&me).and_then(|spot| spot.tree);
        let changed = old != Some(tree);
        if changed {
            if let Some(old) = old {
                self.unindex(me, old);
            }
            self.view_cleanup(me, out);
        }
        self.spots.insert(
            me,
            Spot {
                tree: Some(tree),
                x,
                y,
                z,
            },
        );
        if changed {
            out.push(Effect::Insert { of: me, to: me });
            self.index(me, tree);
            self.update_sectree(me, radius, out);
        } else {
            self.view_reencode(me, out);
        }
        true
    }

    /// The V4 entry: a player entering at a point no sectree holds stands there unindexed, with
    /// no view, and is sent only its own insert.
    pub(super) fn place_treeless(
        &mut self,
        me: EntityKey,
        (x, y, z): (i32, i32, i32),
        out: &mut Vec<Effect>,
    ) {
        self.spots.insert(
            me,
            Spot {
                tree: None,
                x,
                y,
                z,
            },
        );
        out.push(Effect::Insert { of: me, to: me });
    }

    /// `CHARACTER::Sync` (`G/char.cpp:3384-3458`): the spot changes, with z set to 0 even at the
    /// same point, and so does the sectree's membership, but nothing is recomputed. A target no
    /// sectree holds keeps the spot and returns false (`__FIX_KICK_HACK__`, `:3391-3410`). A
    /// treeless body moved into a sectree is indexed and fills its view (V4).
    pub(super) fn move_body(
        &mut self,
        me: EntityKey,
        x: i32,
        y: i32,
        radius: i64,
        out: &mut Vec<Effect>,
    ) -> bool {
        let Some(spot) = self.spots.get(&me).copied() else {
            return false;
        };
        let Some(tree) = self.tree_at(x, y) else {
            return false;
        };
        self.spots.insert(
            me,
            Spot {
                tree: Some(tree),
                x,
                y,
                z: 0,
            },
        );
        match spot.tree {
            Some(old) if old == tree => {}
            Some(old) => {
                self.unindex(me, old);
                self.index(me, tree);
            }
            None => {
                self.index(me, tree);
                self.update_sectree(me, radius, out);
            }
        }
        true
    }

    /// `CEntity::Destroy`'s `ViewCleanup` then `SECTREE::RemoveEntity` (`G/char.cpp:786-789`):
    /// every entity that sees `me` loses it, and `me` leaves the map.
    pub(super) fn remove(&mut self, me: EntityKey, out: &mut Vec<Effect>) {
        self.view_cleanup(me, out);
        if let Some(tree) = self.spots.get(&me).and_then(|spot| spot.tree) {
            self.unindex(me, tree);
        }
        self.spots.remove(&me);
        self.views.remove(&me);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use EntityKey::{Character as Pc, Npc};

    const RADIUS: i64 = 10_500;

    /// A 10 x 10 grid from the origin, so sectree (c, r) holds x in `6400c..6400c+6399`.
    fn grid() -> MapIndex {
        MapIndex::new(SectreeGrid {
            x: 0,
            y: 0,
            columns: 10,
            rows: 10,
        })
    }

    /// The centre of sectree (c, r).
    fn centre(column: i32, row: i32) -> (i32, i32, i32) {
        (column * 6400 + 3200, row * 6400 + 3200, 0)
    }

    fn place(map: &mut MapIndex, me: EntityKey, at: (i32, i32, i32)) -> Vec<Effect> {
        let mut out = Vec::new();
        assert!(map.show(me, at, RADIUS, &mut out));
        out
    }

    fn ins(of: EntityKey, to: EntityKey) -> Effect {
        Effect::Insert { of, to }
    }

    fn rem(of: EntityKey, to: EntityKey) -> Effect {
        Effect::Remove { of, to }
    }

    /// Index an entity at a point without any view work.
    fn stand(map: &mut MapIndex, me: EntityKey, (x, y): (i32, i32)) {
        let tree = map.tree_at(x, y).expect("a sectree");
        map.spots.insert(
            me,
            Spot {
                tree: Some(tree),
                x,
                y,
                z: 0,
            },
        );
        map.index(me, tree);
    }

    #[test]
    fn the_nine_sectrees_are_walked_in_legacy_build_order() {
        let mut map = grid();
        // One NPC per sectree around (5, 5), numbered by its step's place in `Build`.
        let steps = [
            (0, 0),
            (-1, 0),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, 1),
            (1, -1),
            (-1, -1),
            (1, 1),
        ];
        // Number them against key order, so key order alone cannot produce the walk.
        for (n, (dc, dr)) in steps.iter().enumerate() {
            let vid = 100 - u32::try_from(n).expect("small");
            stand(
                &mut map,
                Npc(vid),
                (centre(5 + dc, 5 + dr).0, centre(5 + dc, 5 + dr).1),
            );
        }
        let tree = map
            .tree_at(centre(5, 5).0, centre(5, 5).1)
            .expect("a sectree");
        let walked = map.around(tree);
        let expected: Vec<EntityKey> = (0..9).map(|n: u32| Npc(100 - n)).collect();
        assert_eq!(walked, expected);
        // A sectree two steps away is not around.
        stand(&mut map, Npc(1), (centre(7, 5).0, centre(7, 5).1));
        assert_eq!(map.around(tree), expected);
    }

    #[test]
    fn the_edge_and_corner_sectrees_skip_missing_neighbours() {
        let mut map = grid();
        for column in 0..10 {
            for row in 0..10 {
                let vid = u32::try_from(column * 10 + row).expect("small");
                stand(
                    &mut map,
                    Npc(vid),
                    (centre(column, row).0, centre(column, row).1),
                );
            }
        }
        let corner = map.tree_at(100, 100).expect("a sectree");
        // (0,0), then (1,0), (0,1), (1,1): the steps that leave the grid are skipped.
        assert_eq!(map.around(corner), vec![Npc(0), Npc(10), Npc(1), Npc(11)]);
        let far = map.tree_at(63_999, 63_999).expect("a sectree");
        // (9,9), then (8,9), (9,8), (8,8).
        assert_eq!(map.around(far), vec![Npc(99), Npc(89), Npc(98), Npc(88)]);
        let edge = map
            .tree_at(centre(0, 5).0, centre(0, 5).1)
            .expect("a sectree");
        // (0,5), (1,5), (0,4), (0,6), (1,4), (1,6).
        assert_eq!(
            map.around(edge),
            vec![Npc(5), Npc(15), Npc(4), Npc(6), Npc(14), Npc(16)]
        );
    }

    #[test]
    fn entities_of_one_sectree_are_walked_players_then_npcs_by_vid() {
        let mut map = grid();
        let at = (centre(2, 2).0, centre(2, 2).1);
        for key in [Npc(3), Pc(9), Npc(1), Pc(2), Pc(40)] {
            stand(&mut map, key, at);
        }
        let tree = map.tree_at(at.0, at.1).expect("a sectree");
        assert_eq!(map.around(tree), vec![Pc(2), Pc(9), Pc(40), Npc(1), Npc(3)]);
    }

    #[test]
    fn the_inserts_follow_the_build_order_of_the_sectrees_around() {
        // `UpdateSectree` walks the own sectree first and then the neighbours in
        // `BUILD_ORDER`, and each entity found is inserted both ways before the next.
        let mut map = grid();
        stand(&mut map, Npc(7), (centre(3, 3).0, centre(3, 3).1));
        stand(&mut map, Pc(5), (centre(3, 4).0, centre(3, 4).1));
        stand(&mut map, Pc(4), (centre(2, 3).0, centre(2, 3).1));
        let mut out = Vec::new();
        map.spots.insert(
            Pc(1),
            Spot {
                tree: map.tree_at(centre(3, 3).0, centre(3, 3).1),
                x: centre(3, 3).0,
                y: centre(3, 3).1,
                z: 0,
            },
        );
        map.index(
            Pc(1),
            map.tree_at(centre(3, 3).0, centre(3, 3).1)
                .expect("a sectree"),
        );
        map.update_sectree(Pc(1), RADIUS, &mut out);
        // (3,3) holds Pc(1) and Npc(7); (2,3) is the first step, then (3,4) is the fourth.
        assert_eq!(
            out,
            vec![
                ins(Npc(7), Pc(1)),
                ins(Pc(1), Npc(7)),
                ins(Pc(4), Pc(1)),
                ins(Pc(1), Pc(4)),
                ins(Pc(5), Pc(1)),
                ins(Pc(1), Pc(5)),
            ]
        );
    }

    #[test]
    fn view_insert_of_self_does_nothing() {
        let mut map = grid();
        let mut out = Vec::new();
        map.view_insert(Pc(1), Pc(1), true, &mut out);
        assert!(out.is_empty());
        assert!(!map.sees(Pc(1), Pc(1)));
    }

    #[test]
    fn view_insert_of_a_seen_entity_refreshes_its_age_without_a_record() {
        let mut map = grid();
        let mut out = Vec::new();
        map.view_insert(Pc(1), Pc(2), true, &mut out);
        out.clear();
        map.views.get_mut(&Pc(1)).expect("a view").age = 7;
        map.view_insert(Pc(1), Pc(2), true, &mut out);
        assert!(out.is_empty());
        assert_eq!(map.views[&Pc(1)].seen[&Pc(2)], 7);
        // No recursion either: the other side's age is untouched.
        assert_eq!(map.views[&Pc(2)].seen[&Pc(1)], 0);
    }

    #[test]
    fn view_insert_recursive_inserts_both_ways_in_legacy_order() {
        let mut map = grid();
        let mut out = Vec::new();
        map.view_insert(Pc(1), Npc(2), true, &mut out);
        assert_eq!(out, vec![ins(Npc(2), Pc(1)), ins(Pc(1), Npc(2))]);
        assert!(map.sees(Pc(1), Npc(2)) && map.sees(Npc(2), Pc(1)));
        let mut one_way = Vec::new();
        map.view_insert(Pc(3), Pc(4), false, &mut one_way);
        assert_eq!(one_way, vec![ins(Pc(4), Pc(3))]);
        assert!(!map.sees(Pc(4), Pc(3)));
    }

    #[test]
    fn view_remove_of_an_unseen_entity_does_nothing() {
        let mut map = grid();
        let mut out = Vec::new();
        map.view_remove(Pc(1), Pc(2), true, &mut out);
        assert!(out.is_empty());
        map.view_insert(Pc(1), Pc(2), true, &mut out);
        out.clear();
        map.view_remove(Pc(1), Pc(2), true, &mut out);
        assert_eq!(out, vec![rem(Pc(2), Pc(1)), rem(Pc(1), Pc(2))]);
        out.clear();
        map.view_remove(Pc(1), Pc(2), true, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn view_cleanup_tells_every_viewer_and_not_the_leaver() {
        let mut map = grid();
        let mut out = Vec::new();
        for other in [Npc(9), Pc(3), Pc(2)] {
            map.view_insert(Pc(1), other, true, &mut out);
        }
        out.clear();
        map.view_cleanup(Pc(1), &mut out);
        assert_eq!(
            out,
            vec![rem(Pc(1), Pc(2)), rem(Pc(1), Pc(3)), rem(Pc(1), Npc(9))]
        );
        for other in [Npc(9), Pc(3), Pc(2)] {
            assert!(!map.sees(Pc(1), other) && !map.sees(other, Pc(1)));
        }
    }

    #[test]
    fn view_reencode_sends_the_duplicate_pair_without_a_del() {
        let mut map = grid();
        let mut out = Vec::new();
        map.view_insert(Pc(1), Npc(5), true, &mut out);
        map.view_insert(Pc(1), Pc(2), true, &mut out);
        out.clear();
        map.view_reencode(Pc(1), &mut out);
        assert_eq!(
            out,
            vec![
                rem(Pc(1), Pc(1)),
                ins(Pc(1), Pc(1)),
                rem(Pc(1), Pc(2)),
                ins(Pc(1), Pc(2)),
                ins(Pc(2), Pc(1)),
                rem(Pc(1), Npc(5)),
                ins(Pc(1), Npc(5)),
                ins(Npc(5), Pc(1)),
            ]
        );
        // The views are unchanged.
        assert!(map.sees(Pc(1), Pc(2)) && map.sees(Pc(2), Pc(1)) && map.sees(Pc(1), Npc(5)));
    }

    #[test]
    fn update_sectree_uses_distance_approx_strictly_greater_than_the_radius() {
        // DISTANCE_APPROX(10927, 0) is 10500 and (10928, 0) is 10501.
        assert_eq!(distance_approx(10_927, 0), 10_500);
        assert_eq!(distance_approx(10_928, 0), 10_501);
        let mut map = grid();
        // At the start of column 5, so column 6 reaches 12799 to the east.
        let (x, y) = (32_010, centre(5, 5).1);
        stand(&mut map, Pc(2), (x + 10_927, y));
        stand(&mut map, Pc(3), (x + 10_928, y));
        let mut out = place(&mut map, Pc(1), (x, y, 0));
        assert_eq!(
            out,
            vec![ins(Pc(1), Pc(1)), ins(Pc(2), Pc(1)), ins(Pc(1), Pc(2))]
        );
        assert!(!map.sees(Pc(1), Pc(3)));
        out.clear();
        // The bound follows the radius it is given.
        map.update_sectree(Pc(1), 10_501, &mut out);
        assert_eq!(out, vec![ins(Pc(3), Pc(1)), ins(Pc(1), Pc(3))]);
    }

    #[test]
    fn update_sectree_prunes_every_entry_older_than_the_new_age_in_key_order() {
        let mut map = grid();
        stand(&mut map, Pc(2), (centre(5, 5).0 + 100, centre(5, 5).1));
        stand(&mut map, Npc(9), (centre(5, 5).0 - 100, centre(5, 5).1));
        stand(&mut map, Pc(3), (centre(5, 6).0, centre(5, 6).1));
        place(&mut map, Pc(1), centre(5, 5));
        assert!(map.sees(Pc(1), Pc(2)) && map.sees(Pc(1), Pc(3)) && map.sees(Pc(1), Npc(9)));
        // Pc(1) moves without a recompute to a sectree from which only Pc(3) is around.
        let mut out = Vec::new();
        assert!(map.move_body(Pc(1), centre(5, 7).0, centre(5, 7).1, RADIUS, &mut out));
        assert!(out.is_empty());
        map.update_sectree(Pc(1), RADIUS, &mut out);
        assert_eq!(
            out,
            vec![
                rem(Pc(2), Pc(1)),
                rem(Pc(1), Pc(2)),
                rem(Npc(9), Pc(1)),
                rem(Pc(1), Npc(9)),
            ]
        );
        // Pc(3) was found again: its age is the new one.
        assert_eq!(map.views[&Pc(1)].seen[&Pc(3)], map.views[&Pc(1)].age);
        // An entry of exactly the new age stays.
        out.clear();
        map.update_sectree(Pc(1), RADIUS, &mut out);
        assert!(out.is_empty());
        assert!(map.sees(Pc(1), Pc(3)));
    }

    #[test]
    fn a_standing_character_never_prunes() {
        let mut map = grid();
        place(&mut map, Pc(1), centre(5, 5));
        place(&mut map, Pc(2), (centre(5, 5).0 + 50, centre(5, 5).1, 0));
        // Pc(2) leaves by `Sync` alone, far away: neither recomputes, so each still sees the
        // other (quirk 1) until one of them recomputes.
        let mut out = Vec::new();
        assert!(map.move_body(Pc(2), centre(9, 9).0, centre(9, 9).1, RADIUS, &mut out));
        assert!(out.is_empty());
        assert!(map.sees(Pc(1), Pc(2)) && map.sees(Pc(2), Pc(1)));
        map.update_sectree(Pc(2), RADIUS, &mut out);
        assert_eq!(out, vec![rem(Pc(1), Pc(2)), rem(Pc(2), Pc(1))]);
    }

    #[test]
    fn show_to_a_point_no_sectree_holds_returns_false_and_changes_nothing() {
        let mut map = grid();
        place(&mut map, Pc(1), centre(1, 1));
        let mut out = Vec::new();
        assert!(!map.show(Pc(1), (64_000, 100, 0), RADIUS, &mut out));
        assert!(!map.show(Pc(1), (-1, 100, 0), RADIUS, &mut out));
        assert!(out.is_empty());
        let (x, y, _) = centre(1, 1);
        assert_eq!(
            map.spot(Pc(1)),
            Some(Spot {
                tree: map.tree_at(x, y),
                x,
                y,
                z: 0
            })
        );
        assert!(!map.show(Pc(2), (64_000, 100, 0), RADIUS, &mut out));
        assert!(map.spot(Pc(2)).is_none());
    }

    #[test]
    fn an_entrant_at_a_treeless_point_gets_its_own_pair_and_no_view() {
        let mut map = grid();
        place(&mut map, Pc(2), centre(9, 9));
        let mut out = Vec::new();
        map.place_treeless(Pc(1), (64_100, 63_900, 5), &mut out);
        assert_eq!(out, vec![ins(Pc(1), Pc(1))]);
        assert_eq!(map.viewers(Pc(1)), None);
        assert!(!map.sees(Pc(2), Pc(1)));
        // No sectree indexes it, so a recompute around (9, 9) does not find it.
        out.clear();
        map.update_sectree(Pc(2), RADIUS, &mut out);
        assert!(out.is_empty());
        map.update_sectree(Pc(1), RADIUS, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn a_treeless_body_moved_into_a_sectree_is_indexed_and_fills_its_view() {
        let mut map = grid();
        place(&mut map, Pc(2), centre(9, 9));
        let mut out = Vec::new();
        map.place_treeless(Pc(1), (64_100, 63_900, 5), &mut out);
        out.clear();
        // A step to another point no sectree holds keeps the spot.
        assert!(!map.move_body(Pc(1), 64_200, 63_900, RADIUS, &mut out));
        assert_eq!(map.spot(Pc(1)).map(|spot| spot.x), Some(64_100));
        assert!(map.move_body(Pc(1), 63_900, 63_900, RADIUS, &mut out));
        assert_eq!(out, vec![ins(Pc(2), Pc(1)), ins(Pc(1), Pc(2))]);
        assert_eq!(map.viewers(Pc(1)), Some(vec![Pc(2)]));
        assert_eq!(map.spot(Pc(1)).map(|spot| spot.z), Some(0));
    }

    /// A move into another sectree takes the entity out of the old one's membership
    /// (`SECTREE::InsertEntity`, `G/sectree.cpp:133-134`): an update whose neighbours reach the
    /// old sectree and not the new one does not find it, though it stands in range.
    #[test]
    fn a_body_moved_to_another_sectree_leaves_the_old_ones_membership() {
        // Control: standing in sectree (3, 3), 9600 from (19_190, 19_190) in (2, 2), it is found.
        let mut map = grid();
        place(&mut map, Pc(1), (25_590, 25_590, 0));
        let out = place(&mut map, Pc(2), (19_190, 19_190, 0));
        assert!(out.contains(&ins(Pc(1), Pc(2))), "{out:?}");
        // Moved 20 on into (4, 4), outside (2, 2)'s neighbours, it is not.
        let mut map = grid();
        place(&mut map, Pc(1), (25_590, 25_590, 0));
        let mut moved = Vec::new();
        assert!(map.move_body(Pc(1), 25_610, 25_610, RADIUS, &mut moved));
        assert!(moved.is_empty());
        let out = place(&mut map, Pc(2), (19_190, 19_190, 0));
        assert!(!out.contains(&ins(Pc(1), Pc(2))), "{out:?}");
        assert!(!map.sees(Pc(2), Pc(1)));
    }

    #[test]
    fn show_in_another_tree_cleans_up_inserts_itself_then_updates() {
        let mut map = grid();
        place(&mut map, Pc(2), centre(1, 1));
        place(&mut map, Pc(3), centre(8, 8));
        place(&mut map, Pc(1), (centre(1, 1).0 + 10, centre(1, 1).1, 0));
        let out = place(&mut map, Pc(1), (centre(8, 8).0 + 10, centre(8, 8).1, 7));
        assert_eq!(
            out,
            vec![
                rem(Pc(1), Pc(2)),
                ins(Pc(1), Pc(1)),
                ins(Pc(3), Pc(1)),
                ins(Pc(1), Pc(3)),
            ]
        );
        assert!(!map.sees(Pc(1), Pc(2)) && !map.sees(Pc(2), Pc(1)));
        let old = map
            .tree_at(centre(1, 1).0, centre(1, 1).1)
            .expect("a sectree");
        assert!(!map.trees[&old].contains(&Pc(1)));
        assert_eq!(map.spot(Pc(1)).map(|spot| spot.z), Some(7));
    }

    #[test]
    fn show_in_the_same_tree_reencodes_without_an_update() {
        let mut map = grid();
        place(&mut map, Pc(2), centre(1, 1));
        place(&mut map, Pc(1), (centre(1, 1).0 + 10, centre(1, 1).1, 0));
        // A third entity arrives without a recompute: the same-tree Show must not find it.
        stand(&mut map, Pc(3), (centre(1, 1).0 - 10, centre(1, 1).1));
        let out = place(&mut map, Pc(1), (centre(1, 1).0 + 20, centre(1, 1).1, 0));
        assert_eq!(
            out,
            vec![
                rem(Pc(1), Pc(1)),
                ins(Pc(1), Pc(1)),
                rem(Pc(1), Pc(2)),
                ins(Pc(1), Pc(2)),
                ins(Pc(2), Pc(1)),
            ]
        );
        assert!(!map.sees(Pc(1), Pc(3)));
        assert_eq!(
            map.spot(Pc(1)).map(|spot| spot.x),
            Some(centre(1, 1).0 + 20)
        );
    }

    #[test]
    fn views_stay_symmetric_over_a_random_walk() {
        let mut map = grid();
        // A fixed xorshift stream, so the walk is the same on every run.
        let mut state: u32 = 229;
        let mut roll = |n: u32| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            i32::try_from(state % n).expect("small")
        };
        let npcs = [Npc(1), Npc(2), Npc(3)];
        let pcs = [Pc(1), Pc(2), Pc(3), Pc(4), Pc(5)];
        for npc in npcs {
            place(&mut map, npc, (roll(64_000), roll(64_000), 0));
        }
        for pc in pcs {
            place(&mut map, pc, (roll(64_000), roll(64_000), 0));
        }
        let mut out = Vec::new();
        for pulse in 0_u32..2000 {
            for pc in pcs {
                let spot = map.spot(pc).expect("placed");
                let x = (spot.x + roll(801) - 400).clamp(0, 63_999);
                let y = (spot.y + roll(801) - 400).clamp(0, 63_999);
                if roll(50) == 0 {
                    place(&mut map, pc, (x, y, 0));
                } else {
                    map.move_body(pc, x, y, RADIUS, &mut out);
                }
                if pulse % 16 == 0 {
                    map.update_sectree(pc, RADIUS, &mut out);
                }
            }
            for me in pcs.iter().chain(npcs.iter()) {
                let seen = map
                    .views
                    .get(me)
                    .map(|v| v.seen.clone())
                    .unwrap_or_default();
                for other in seen.keys() {
                    assert!(map.sees(*other, *me), "{me:?} sees {other:?} alone");
                }
            }
        }
        // Every insert and removal went to an entity the other side holds a view for.
        assert!(out.iter().all(|effect| match effect {
            Effect::Insert { of, to } | Effect::Remove { of, to } => of != to,
        }));
    }

    #[test]
    fn the_radius_is_given_in_64_bits() {
        // A radius past i32::MAX still holds every distance on the map (D4).
        let mut map = grid();
        stand(&mut map, Pc(2), (63_000, 63_000));
        let mut out = Vec::new();
        assert!(map.show(Pc(1), (100, 100, 0), i64::from(i32::MAX) + 500, &mut out));
        // (63000, 63000) is not in the 3x3 of (0, 0), so the range alone does not insert it.
        assert_eq!(out, vec![ins(Pc(1), Pc(1))]);
        stand(&mut map, Pc(3), (12_000, 12_000));
        out.clear();
        map.update_sectree(Pc(1), i64::from(i32::MAX) + 500, &mut out);
        assert_eq!(out, vec![ins(Pc(3), Pc(1)), ins(Pc(1), Pc(3))]);
    }

    #[test]
    fn viewers_without_a_sectree_are_none_and_with_one_are_the_view_in_key_order() {
        let mut map = grid();
        place(&mut map, Npc(4), centre(1, 1));
        place(&mut map, Pc(9), centre(1, 2));
        place(&mut map, Pc(3), centre(2, 1));
        assert_eq!(map.viewers(Pc(1)), None);
        place(&mut map, Pc(1), centre(1, 1));
        assert_eq!(map.viewers(Pc(1)), Some(vec![Pc(3), Pc(9), Npc(4)]));
        let mut out = Vec::new();
        map.place_treeless(Pc(5), (70_000, 70_000, 0), &mut out);
        assert_eq!(map.viewers(Pc(5)), None);
    }

    #[test]
    fn remove_tells_every_viewer_and_leaves_the_sectree() {
        let mut map = grid();
        place(&mut map, Pc(2), centre(1, 1));
        place(&mut map, Npc(3), centre(1, 1));
        place(&mut map, Pc(1), centre(1, 1));
        let mut out = Vec::new();
        map.remove(Pc(1), &mut out);
        assert_eq!(out, vec![rem(Pc(1), Pc(2)), rem(Pc(1), Npc(3))]);
        assert!(map.spot(Pc(1)).is_none());
        let tree = map
            .tree_at(centre(1, 1).0, centre(1, 1).1)
            .expect("a sectree");
        assert!(!map.trees[&tree].contains(&Pc(1)));
        // A recompute no longer finds it.
        out.clear();
        map.update_sectree(Pc(2), RADIUS, &mut out);
        assert!(out.is_empty());
    }
}
