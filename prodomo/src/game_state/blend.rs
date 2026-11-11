//! Blend items (`ITEM_BLEND`, `G/blend_item.cpp`, `G/char_item.cpp:2231-2320`,
//! `:2321-2652`, `:7064-7118`).
//!
//! A listed blend vnum rolls its sockets at creation (see
//! [`world::character::drop_item`]); using one lays an affect. A plain vnum
//! lays `AFFECT_BLEND` on the socketed point and spends one, unless the row or
//! the free-experience row already holds (`[LS;99]`). An extended vnum (the
//! `IsExtendedBlend` list) toggles its marker affect instead: the first use
//! lays the marker with the socketed point and value and starts the item's
//! tick, the second takes the marker off and stops it, and a use while another
//! copy holds the marker hears `[LS;99]`.
//!
//! The tick (`blend_use_expire_event`, `G/item.cpp:2090-2115`) counts the
//! item's socket 2 down once a second and spends one at 0. It never starts for
//! a real-time item or a socket at or past 8640000.
//!
//! What is not ported: the item lock (`Lock(true)` while a blend ticks, which
//! gates moves and refines) — the Rewrite's items have no lock, so a ticking
//! blend can be moved, and the tick follows it by item id.

use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::item_pos::ItemPos;

use super::GameState;
use crate::chat_line::chat_packet;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};
use crate::loading_phase::{
    AFFECT_ATTACK_SPEED, AFFECT_BLEND, AFFECT_BLEND_POTION_1, AFFECT_BLEND_POTION_2,
    AFFECT_BLEND_POTION_3, AFFECT_BLEND_POTION_4, AFFECT_BLEND_POTION_5, AFFECT_BLEND_POTION_6,
    AFFECT_BLEND_POTION_7, AFFECT_CRITICAL, AFFECT_DRAGON_GOD_1, AFFECT_DRAGON_GOD_2,
    AFFECT_DRAGON_GOD_3, AFFECT_DRAGON_GOD_4, AFFECT_ENERGY, AFFECT_EXP_BONUS_EURO_FREE,
    AFFECT_MALL, AFFECT_MOVE_SPEED, AFFECT_PENETRATE, AFFECT_WIND_SHOES,
};
use crate::save::PASSES_PER_SEC;

/// The extended-blend vnums (`CItemVnumHelper::IsExtendedBlend`,
/// `C/VnumHelper.h:71-111`), each with its marker affect. Only the 20201-20217
/// range runs the extended arm; the 508xx cases in `UseExtendedBlendAffect` are
/// dead, because the gate never lets those vnums in.
const EXTENDED_BLENDS: [(u32, u32); 17] = [
    (20_209, AFFECT_BLEND_POTION_2),
    (20_210, AFFECT_BLEND_POTION_1),
    (20_211, AFFECT_BLEND_POTION_5),
    (20_212, AFFECT_BLEND_POTION_6),
    (20_213, AFFECT_BLEND_POTION_4),
    (20_214, AFFECT_BLEND_POTION_3),
    (20_215, AFFECT_BLEND_POTION_7),
    (20_216, AFFECT_ENERGY),
    (20_205, AFFECT_DRAGON_GOD_1),
    (20_206, AFFECT_DRAGON_GOD_2),
    (20_207, AFFECT_DRAGON_GOD_3),
    (20_208, AFFECT_DRAGON_GOD_4),
    (20_203, AFFECT_CRITICAL),
    (20_204, AFFECT_PENETRATE),
    (20_201, AFFECT_MOVE_SPEED),
    (20_202, AFFECT_ATTACK_SPEED),
    (20_217, AFFECT_WIND_SHOES),
];

/// The plain use's marker (`CHARACTER::SetBlendAffect`, `G/char_item.cpp:2231-2320`),
/// by vnum. The 50825 pair lays 3 here and 5 on the extended arm.
const PLAIN_MARKERS: [(u32, u32); 22] = [
    (50_821, AFFECT_BLEND_POTION_1),
    (20_210, AFFECT_BLEND_POTION_1),
    (50_822, AFFECT_BLEND_POTION_2),
    (20_209, AFFECT_BLEND_POTION_2),
    (50_823, AFFECT_BLEND_POTION_3),
    (20_214, AFFECT_BLEND_POTION_3),
    (50_824, AFFECT_BLEND_POTION_4),
    (20_213, AFFECT_BLEND_POTION_4),
    (50_825, AFFECT_BLEND_POTION_3),
    (20_211, AFFECT_BLEND_POTION_3),
    (50_826, AFFECT_BLEND_POTION_6),
    (20_212, AFFECT_BLEND_POTION_6),
    (20_215, AFFECT_BLEND_POTION_7),
    (51_002, AFFECT_ENERGY),
    (20_216, AFFECT_ENERGY),
    (20_205, AFFECT_DRAGON_GOD_1),
    (20_206, AFFECT_DRAGON_GOD_2),
    (20_207, AFFECT_DRAGON_GOD_3),
    (20_208, AFFECT_DRAGON_GOD_4),
    (20_203, AFFECT_CRITICAL),
    (20_204, AFFECT_PENETRATE),
    (20_202, AFFECT_ATTACK_SPEED),
];

/// The plain use's last three markers (`:2304-2313`).
const PLAIN_MARKERS_TAIL: [(u32, u32); 2] =
    [(20_201, AFFECT_MOVE_SPEED), (20_217, AFFECT_WIND_SHOES)];

/// A socket at or past this never ticks (`CItem::StartBlendExpireEvent`,
/// `G/item.cpp:2129-2130`).
const BLEND_TICK_SOCKET_MAX: i32 = 8_640_000;

/// One blend tick: the item id and the Pulse of its next fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BlendTick {
    /// The ticking item's id.
    pub(super) id: world::item::ItemId,
    /// The Pulse the next fire runs on.
    pub(super) due: u64,
}

impl GameState {
    /// Whether `vnum` is an extended blend vnum, with its marker affect.
    fn extended_blend(vnum: u32) -> Option<u32> {
        EXTENDED_BLENDS
            .iter()
            .find(|(listed, _)| *listed == vnum)
            .map(|(_, marker)| *marker)
    }

    /// Whether the character under `vid` holds an affect of `kind` on `point`
    /// (`FindAffect`, `G/char_affect.cpp:129-142`): point 0 matches any point.
    fn has_affect(&self, vid: Vid, kind: u32, point: u8) -> bool {
        self.affect_events.get(&vid).is_some_and(|event| {
            event
                .affects
                .iter()
                .any(|affect| affect.kind == kind && (point == 0 || affect.apply_on == point))
        })
    }

    /// `RemoveAffect(kind)` on the player under `vid`: every row of the kind
    /// goes, each uncomputed with its remove packet sent.
    fn remove_affects_of(&mut self, vid: Vid, kind: u32) {
        let removed = self
            .affect_events
            .get_mut(&vid)
            .map_or(Vec::new(), |event| {
                super::affect::take_affects_of_kind(&mut event.affects, kind)
            });
        for affect in &removed {
            let _stripped = self.compute_affect(vid, affect, false);
            let frame = super::affect::affect_remove_frame(affect);
            let _retired = self.write_to_client(vid, frame);
        }
    }

    /// Restart the ticking blends a loaded character holds (`OnAfterCreatedItem`,
    /// `G/item.cpp:2759-2767`): each stored socket 3 of 1 starts its tick again. The
    /// lock is not ported.
    pub(super) fn restart_loaded_blends(&mut self, vid: Vid) {
        let Some(character) = self.characters.find_by_vid(vid).ok() else {
            return;
        };
        let ticking: Vec<world::item::ItemId> = character
            .items()
            .occupied()
            .iter()
            .filter_map(|(pos, _)| character.items().item_at(*pos))
            .filter(|item| {
                item.sockets[3] == 1
                    && item.sockets[2] < BLEND_TICK_SOCKET_MAX
                    && self.protos.get(item.vnum).is_some_and(|proto| {
                        proto.item_type == gamedata::item_kind::ITEM_BLEND
                            && !proto
                                .limits
                                .iter()
                                .any(|limit| limit.kind == gamedata::item_kind::LIMIT_REAL_TIME)
                    })
            })
            .map(|item| item.id)
            .collect();
        if let Some(id) = ticking.first().copied() {
            let due = self.last_pulse.saturating_add(PASSES_PER_SEC.into());
            self.blend_ticks.insert(vid, BlendTick { id, due });
        }
    }

    /// Whether the item at `at` of the character under `vid` is a blend item.
    pub(super) fn blend_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .and_then(|item| self.protos.get(item.vnum))
            .is_some_and(|proto| proto.item_type == gamedata::item_kind::ITEM_BLEND)
    }

    /// Run one `CG_ITEM_USE` on the blend item at `at` (`case ITEM_BLEND`,
    /// `G/char_item.cpp:7064-7118`).
    pub(super) fn use_blend_item(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let (vnum, sockets, id) = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .map(|item| (item.vnum, item.sockets, item.id))
            .ok_or(MoveItemRefused::NoSuchCharacter { vid })?;
        if !self.blend_table.find(vnum) {
            return self.blend_nothing_used(vid, actor);
        }
        let point = u8::try_from(sockets[0])
            .ok()
            .and_then(world::character::point_type)
            .and_then(|slot| u8::try_from(slot).ok())
            .unwrap_or(0);
        let value = sockets[1];
        let duration = sockets[2];
        if let Some(marker) = Self::extended_blend(vnum) {
            return self.use_extended_blend(
                vid,
                actor,
                at,
                id,
                vnum,
                marker,
                point,
                value,
                duration,
                sockets[3] != 0,
            );
        }
        self.use_plain_blend(vid, actor, at, id, vnum, point, value, duration)
    }

    /// Answer a blend use that changed nothing as a step that was never run.
    fn blend_nothing_used(
        &mut self,
        vid: Vid,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        self.nothing_used(vid, actor)
    }

    /// The plain arm: past the two `[LS;99]` refusals, the battle-pass mission
    /// counts, the marker lands when the vnum has one, and the blend affect
    /// lands with one spent.
    #[allow(clippy::too_many_arguments)]
    fn use_plain_blend(
        &mut self,
        vid: Vid,
        actor: Mover,
        at: ItemPos,
        id: world::item::ItemId,
        vnum: u32,
        point: u8,
        value: i32,
        duration: i32,
    ) -> Result<MovedItems, MoveItemRefused> {
        if self.has_affect(vid, AFFECT_BLEND, point)
            || self.has_affect(
                vid,
                AFFECT_EXP_BONUS_EURO_FREE,
                u8::try_from(common::point_slot::POINT_RESIST_MAGIC).unwrap_or(u8::MAX),
            )
        {
            let line = chat_packet(
                actor.recipient(&self.locale),
                CHAT_TYPE_INFO,
                b"[LS;99]",
                &[],
            );
            let mut moved = self.blend_nothing_used(vid, actor)?;
            moved.records.insert(0, line);
            return Ok(moved);
        }
        if let Some(mover) = self.own_mover(vid) {
            self.register_pass_item(
                vid,
                mover,
                gamedata::battle_pass::MISSION_TYPE_USE_ITEM,
                1,
                vnum,
            );
        }
        if let Some(marker) = PLAIN_MARKERS
            .iter()
            .chain(PLAIN_MARKERS_TAIL.iter())
            .find(|(listed, _)| *listed == vnum)
            .map(|(_, marker)| *marker)
        {
            self.add_pc_affect(
                vid,
                super::affect::Affect {
                    kind: marker,
                    apply_on: 0,
                    value: 0,
                    flag: 0,
                    remaining: duration,
                    sp_cost: 0,
                },
                false,
            );
        }
        self.add_pc_affect(
            vid,
            super::affect::Affect {
                kind: AFFECT_BLEND,
                apply_on: point,
                value,
                flag: 0,
                remaining: duration,
                sp_cost: 0,
            },
            false,
        );
        self.run_item_step(vid, actor, |items, _ids, _rules, _gear, _protos| {
            world::character::spend_blend(items, at, id)
        })
    }

    /// The extended arm (`UseExtendedBlendAffect`, `G/char_item.cpp:2321-2652`):
    /// past the three `[LS;99]` refusals, a held marker toggles off when this
    /// copy ticks (`on`), else the marker lays with the socketed point and
    /// value and the item starts its tick; a use that lays nothing stops it.
    #[allow(clippy::too_many_arguments)]
    fn use_extended_blend(
        &mut self,
        vid: Vid,
        actor: Mover,
        at: ItemPos,
        id: world::item::ItemId,
        vnum: u32,
        marker: u32,
        point: u8,
        value: i32,
        duration: i32,
        on: bool,
    ) -> Result<MovedItems, MoveItemRefused> {
        let duration = if duration <= 0 {
            i32::try_from(gamedata::item_proto_value::INFINITE_AFFECT_DURATION).unwrap_or(i32::MAX)
        } else {
            duration
        };
        let nope = |state: &mut Self| {
            chat_packet(
                actor.recipient(&state.locale),
                CHAT_TYPE_INFO,
                b"[LS;99]",
                &[],
            )
        };
        if self.has_affect(vid, AFFECT_BLEND, point)
            || self.has_affect(vid, AFFECT_EXP_BONUS_EURO_FREE, point)
            || self.has_affect(vid, AFFECT_MALL, point)
            || (vnum == 20_202 && self.has_affect(vid, crate::loading_phase::AFFECT_ATT_SPEED, 0))
            || ((vnum == 20_201 || vnum == 20_217)
                && self.has_affect(vid, crate::loading_phase::AFFECT_MOV_SPEED, 0))
        {
            let line = nope(self);
            let mut moved = self.blend_nothing_used(vid, actor)?;
            moved.records.insert(0, line);
            return Ok(moved);
        }
        if self.has_affect(vid, marker, 0) {
            if !on {
                let line = nope(self);
                let mut moved = self.blend_nothing_used(vid, actor)?;
                moved.records.insert(0, line);
                return Ok(moved);
            }
            self.remove_affects_of(vid, marker);
            return self.set_blend_tick(vid, actor, at, id, false);
        }
        if let Some(mover) = self.own_mover(vid) {
            self.register_pass_item(
                vid,
                mover,
                gamedata::battle_pass::MISSION_TYPE_USE_ITEM,
                1,
                vnum,
            );
        }
        self.add_pc_affect(
            vid,
            super::affect::Affect {
                kind: marker,
                apply_on: point,
                value,
                flag: 0,
                remaining: duration,
                sp_cost: 0,
            },
            true,
        );
        self.set_blend_tick(vid, actor, at, id, true)
    }

    /// Start or stop the item's tick: socket 3 takes `on`, the client's cell
    /// updates, the row saves, and the tick map follows. A start past
    /// [`BLEND_TICK_SOCKET_MAX`] or on a real-time item lays no tick
    /// (`CItem::StartBlendExpireEvent`, `G/item.cpp:2116-2130`).
    fn set_blend_tick(
        &mut self,
        vid: Vid,
        actor: Mover,
        at: ItemPos,
        id: world::item::ItemId,
        on: bool,
    ) -> Result<MovedItems, MoveItemRefused> {
        let tickable = self.characters.find_by_vid(vid).ok().and_then(|character| {
            let item = character.items().item_at(at).filter(|item| item.id == id)?;
            let proto = self.protos.get(item.vnum)?;
            let real_time = proto
                .limits
                .iter()
                .any(|limit| limit.kind == gamedata::item_kind::LIMIT_REAL_TIME);
            Some(item.sockets[2] < BLEND_TICK_SOCKET_MAX && !real_time)
        });
        let moved = self.run_item_step(vid, actor, |items, _ids, _rules, _gear, _protos| {
            let Some(item) = items.item_at(at).filter(|item| item.id == id) else {
                return Err(world::character::MoveRefused::Empty);
            };
            Ok(world::character::set_item_socket(
                items,
                item.id,
                (3, i32::from(on)),
            ))
        })?;
        if on && tickable.unwrap_or(false) {
            let due = self.last_pulse.saturating_add(PASSES_PER_SEC.into());
            self.blend_ticks.insert(vid, BlendTick { id, due });
        } else {
            self.blend_ticks.remove(&vid);
        }
        Ok(moved)
    }

    /// One Pulse of blend ticks (`blend_use_expire_event`, `G/item.cpp:2090-2115`
    /// on `PASSES_PER_SEC(1)`): a due item counts its socket 2 down by the
    /// seconds since its last fire and updates its cell, or spends one at 0
    /// and stops. A gone character, a gone item, or another item in the id
    /// stops the tick.
    pub(super) fn run_blend_ticks(&mut self, pulse: u64) {
        let due: Vec<(Vid, world::item::ItemId)> = self
            .blend_ticks
            .iter()
            .filter(|(_, tick)| pulse >= tick.due)
            .map(|(vid, tick)| (*vid, tick.id))
            .collect();
        for (vid, id) in due {
            let Some(character) = self.characters.find_by_vid(vid).ok() else {
                self.blend_ticks.remove(&vid);
                continue;
            };
            let Some(item) = character.items().item(id) else {
                self.blend_ticks.remove(&vid);
                continue;
            };
            if item.sockets[3] == 0 {
                self.blend_ticks.remove(&vid);
                continue;
            }
            let Some(actor) = self.own_mover(vid) else {
                self.blend_ticks.remove(&vid);
                continue;
            };
            let left = item.sockets[2].saturating_sub(1);
            if left <= 0 {
                self.blend_ticks.remove(&vid);
                let at = item.pos;
                let spent =
                    self.run_item_step(vid, actor, |items, _ids, _rules, _gear, _protos| {
                        world::character::spend_blend(items, at, id)
                    });
                if let Ok(spent) = spent {
                    for record in spent.records {
                        let _sent = self.write_to_client(vid, record);
                    }
                }
                continue;
            }
            let ticked = self.run_item_step(vid, actor, |items, _ids, _rules, _gear, _protos| {
                Ok(world::character::set_item_socket(items, id, (2, left)))
            });
            if let Ok(ticked) = ticked {
                for record in ticked.records {
                    let _sent = self.write_to_client(vid, record);
                }
            }
            let due = pulse.saturating_add(PASSES_PER_SEC.into());
            self.blend_ticks.insert(vid, BlendTick { id, due });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use common::item_slots::EWindows;
    use common::vid::Vid;
    use gamedata::item_proto::ItemProtos;
    use protocol::item_pos::ItemPos;
    use world::item::{Item, ItemIdRange};

    use super::super::GameState;
    use super::{AFFECT_BLEND, EXTENDED_BLENDS, PLAIN_MARKERS, PLAIN_MARKERS_TAIL};
    use crate::client_registry::ClientOutbox;
    use crate::item_move::Mover;

    const BLENDER: Vid = Vid::new(9);

    fn inventory(cell: u16) -> ItemPos {
        ItemPos::new(EWindows::Inventory as u8, cell)
    }

    fn blend(id: u32, vnum: u32, count: u16, sockets: [i32; 6]) -> Item {
        let mut item = Item::new(id, vnum);
        item.set_size(1).unwrap();
        item.count = count;
        for (index, value) in sockets.iter().enumerate() {
            item.set_socket(index, *value).unwrap();
        }
        item
    }

    /// The owner's protos and blend table, and a blender online holding `items`.
    /// The blender also stands, so world steps read a mover from its body.
    fn a_blender(items: &[(ItemPos, Item)]) -> GameState {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy");
        let protos = ItemProtos::load(&root.join("gamedata/proto")).unwrap();
        let table = gamedata::blend::load(&root.join("gamedata/locale/europe")).unwrap();
        let mut state = GameState::new(protos).with_blend_table(Arc::new(table));
        let _ids = state
            .install_item_ids(ItemIdRange::new(1, 1_000_000, 1).unwrap())
            .unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        state
            .enter_world_with_items("", BLENDER, 9, "Blender", items, ClientOutbox::new(tx))
            .unwrap();
        let card = crate::loading_phase::PcCard {
            name: "Blender".to_owned(),
            job: 0,
            empire: 1,
            level: 10,
            conqueror_level: 0,
            language: 1,
            pk_mode: crate::loading_phase::PK_MODE_PROTECT,
            skill_group: 0,
            alignment: 0,
            voice: 0,
            premium: 0,
            premium_time: 0,
        };
        let place = crate::game_loop_messages::EnterPlace {
            channel: 1,
            map: 1,
            x: 3200,
            y: 3200,
            z: 0,
        };
        let _shown = state.place_body(BLENDER, place, card);
        state
    }

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

    fn sockets(state: &GameState, cell: u16) -> Option<[i32; 6]> {
        state
            .characters
            .find_by_vid(BLENDER)
            .unwrap()
            .items()
            .item_at(inventory(cell))
            .map(|item| item.sockets)
    }

    fn count(state: &GameState, cell: u16) -> Option<u16> {
        state
            .characters
            .find_by_vid(BLENDER)
            .unwrap()
            .items()
            .item_at(inventory(cell))
            .map(|item| item.count)
    }

    #[test]
    fn a_plain_blend_lays_its_affect_and_refuses_its_echo() {
        let point = u8::try_from(world::character::point_type(1).unwrap()).unwrap();
        let items = [(inventory(0), blend(1, 51_002, 2, [1, 10, 1800, 0, 0, 0]))];
        let mut state = a_blender(&items);
        let moved = state.use_blend_item(BLENDER, inventory(0), mover());
        assert!(moved.is_ok(), "{moved:?}");
        assert_eq!(count(&state, 0), Some(1));
        assert!(state.has_affect(BLENDER, AFFECT_BLEND, point));
        assert!(state.has_affect(BLENDER, crate::loading_phase::AFFECT_ENERGY, 0));
        let moved = state
            .use_blend_item(BLENDER, inventory(0), mover())
            .unwrap();
        assert_eq!(count(&state, 0), Some(1));
        assert!(
            moved.records.iter().any(|record| {
                record
                    .windows(b"[LS;99]".len())
                    .any(|window| window == b"[LS;99]")
            }),
            "the echo hears [LS;99]"
        );
    }

    #[test]
    fn an_extended_blend_toggles_and_its_tick_spends_it() {
        let point = u8::try_from(world::character::point_type(1).unwrap()).unwrap();
        let items = [(inventory(0), blend(1, 20_210, 1, [1, 10, 2, 0, 0, 0]))];
        let mut state = a_blender(&items);
        let marker = crate::loading_phase::AFFECT_BLEND_POTION_1;
        state
            .use_blend_item(BLENDER, inventory(0), mover())
            .unwrap();
        assert!(state.has_affect(BLENDER, marker, point));
        assert_eq!(sockets(&state, 0).unwrap()[3], 1);
        assert!(state.blend_ticks.contains_key(&BLENDER));
        state
            .use_blend_item(BLENDER, inventory(0), mover())
            .unwrap();
        assert!(!state.has_affect(BLENDER, marker, 0));
        assert_eq!(sockets(&state, 0).unwrap()[3], 0);
        assert!(!state.blend_ticks.contains_key(&BLENDER));
        state
            .use_blend_item(BLENDER, inventory(0), mover())
            .unwrap();
        let due = state.blend_ticks[&BLENDER].due;
        state.run_blend_ticks(due);
        assert_eq!(sockets(&state, 0).unwrap()[2], 1);
        assert!(state.blend_ticks.contains_key(&BLENDER));
        let due = state.blend_ticks[&BLENDER].due;
        state.run_blend_ticks(due);
        assert_eq!(count(&state, 0), None);
        assert!(!state.blend_ticks.contains_key(&BLENDER));
    }

    #[test]
    fn the_lists_cover_the_owners_table_whole() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy");
        let table = gamedata::blend::load(&root.join("gamedata/locale/europe")).unwrap();
        let plain: Vec<u32> = PLAIN_MARKERS
            .iter()
            .chain(PLAIN_MARKERS_TAIL.iter())
            .map(|(vnum, _)| *vnum)
            .collect();
        for vnum in table.rows.keys() {
            let extended = EXTENDED_BLENDS.iter().any(|(listed, _)| listed == vnum);
            assert!(plain.contains(vnum), "the table's {vnum} lays a marker");
            // The 508xx dews and 51002 run the plain arm; every 202xx vnum runs the
            // extended one.
            assert_eq!(
                extended,
                (20_201..=20_217).contains(vnum),
                "the table's {vnum}"
            );
        }
        assert_eq!(table.rows.len(), 24);
    }
}
