//! The changelook (Transmutation) window: the `CG_CL` request (`CInputMain::ChangeLook`,
//! `G/input_main.cpp:3431-3459`), which adds, removes, refines and closes the two
//! materials Theowahdan's chat stood up (`G/char.cpp:10504-10776`).
//!
//! The request runs only while no quest waits on the character (`GetPCForce(...)->IsRunning()`),
//! with no observer gate (`G/input_main.cpp:3857-3861` sits outside the guarded band). An unknown
//! sub-header is absorbed, as legacy's `default: break` does.
//!
//! The window is `changelook_windows`; its materials are item ids in `changelook_held`, so a
//! held item stays held when it moves cells, as legacy's `LPITEM` pointers survive the same
//! moves. A close (`ChangeLookWindow(false)`, `:10504-10535`) answers the close record with cost
//! 0 only when the window stands — on a standing window the send runs before the flag clears,
//! and a close of no window sends nothing (the client close passes `bRequest = false`, so the
//! not-open line chats there). Death closes the window with its record (`Dead`, `G/char_battle.cpp:1642-1644`);
//! the penalty's `ChangeLookWindow(false, true)` runs at `dead_event`, after `Dead` already closed it,
//! so it sends nothing (`:334-339`). A warp and a disconnect forget the window silently: no warp or
//! disconnect path closes it, and the client rebuilds its windows on the new view. The warp NPC judges
//! `CanHandleItem(false, true)` (`:7964`): the sash and the changelook refuse, the shop, refine, cube,
//! dragon soul, warp and aura arms are not ported.
//!
//! The add (`AddClMaterial`, `:10563-10672`) resolves a `pos` at or past 2 to the first free slot
//! only when it is exactly 255, then guards in legacy's `else if` order: the item must exist
//! (`GetItem` refuses a worn cell and anything at or past `INVENTORY_AND_EQUIP_SLOT_MAX`; a belt
//! cell refuses), a worn item chats the equipped line, anything but a weapon, body armour or
//! costume chats the cannot line, an arrow or mount spear chats it, a non-body armour chats it,
//! a costume that is no body, hair, weapon or sash chats it (`ENABLE_WEAPON_COSTUME_SYSTEM` is on),
//! a held material chats the locked line, and an already-transmuted item chats the transmuted line.
//! `LC_TEXT` of an unknown key is the key itself, so the chat keys go out raw. Slot 1 with slot 0
//! empty refuses silently, as does a filled slot; slot 1 of mismatched type, subtype, original
//! vnum or antiflags (the sex pair either way, the job flags only when slot 1 names a job slot 0
//! does not) chats the cannot-submit line. The landed add answers `ADD` with cost 0, the slot,
//! and the named position.
//!
//! The remove (`RemoveClMaterial`, `:10674-10708`) unlocks slot 1 alone while slot 0 clears both;
//! both answer `REMOVE` with a zeroed `{INVENTORY, 0}` position and cost 0, and a remove of an
//! empty slot sends nothing. The refine (`RefineClMaterials`, `:10710-10744`) sends nothing with
//! slot 0 empty, chats the submit line with slot 1 empty, and chats the gold line short of
//! 50,000,000. Else it spends the gold (`ChangeGold`, `ENABLE_REMOVE_LIMIT_GOLD`), destroys slot 1,
//! stamps slot 1's vnum into slot 0's transmutation (`SetTransmutation`, `G/item.cpp:352-356`: the
//! `GC_ITEM_UPDATE` and the save), clears the window, and answers `REFINE` with a zeroed position.
//! The money and item logs are `sys.log`'s, which owns no sink.
//!
//! The detachment scroll (`USE_DETACHMENT` with `VALUE0` 8 on a transmuted weapon, armour or
//! costume, `CleanTransmutation`, `:10746-10768`) rides the use-on-an-item path
//! (`world::character::clean_transmutation`), not the window.
//!
//! `G/` = `server/server/game/`, `C/` = `server/server/common/`.

use common::item_slots::INVENTORY_MAX_NUM;
use common::vid::Vid;
use gamedata::item_kind::{
    ARMOR_BODY, COSTUME_BODY, COSTUME_HAIR, COSTUME_SASH, COSTUME_WEAPON, ITEM_ANTIFLAG_ASSASSIN,
    ITEM_ANTIFLAG_FEMALE, ITEM_ANTIFLAG_MALE, ITEM_ANTIFLAG_SHAMAN, ITEM_ANTIFLAG_SURA,
    ITEM_ANTIFLAG_WARRIOR, ITEM_ARMOR, ITEM_COSTUME, ITEM_WEAPON, WEAPON_ARROW, WEAPON_MOUNT_SPEAR,
};
use protocol::gc_changelook::{
    GcChangeLook, CL_SUBHEADER_ADD, CL_SUBHEADER_CLOSE, CL_SUBHEADER_REFINE, CL_SUBHEADER_REMOVE,
    CL_TRANSMUTATION_PRICE, INVENTORY_WINDOW,
};
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::item_pos::ItemPos;
use world::character::{MoveDone, MoveKind, MoveRecord};
use world::item::ItemId;

use super::GameState;
use crate::chat_line::{chat_packet, Arg};
use crate::item_move::{gold_delta, MoveItemRefused, MovedItems, Mover};

/// The materials the changelook window of each character holds, slot by slot, keyed by its
/// VID: `m_pointsInstant.pClMaterials` (`G/char.h:2404`), as the held item ids.
pub(super) type ChangelookMaterials = [Option<ItemId>; 2];

/// `CL_SUBHEADER_CLOSE`: shut the window.
pub const CL_CLOSE: u8 = 1;
/// `CL_SUBHEADER_ADD`: land a material.
pub const CL_ADD: u8 = 2;
/// `CL_SUBHEADER_REMOVE`: lift a material.
pub const CL_REMOVE: u8 = 3;
/// `CL_SUBHEADER_REFINE`: run the transmutation.
pub const CL_REFINE: u8 = 4;

/// `CL_WINDOW_MAX_MATERIALS` (`C/length.h:178`): the window's two slots.
pub const CL_MATERIALS: usize = 2;
/// The `pos` the client sends to mean the first free slot (`AddClMaterial`).
pub const CL_ANY_SLOT: u8 = 255;

/// The equipped refusal (`AddClMaterial`, `G/char.cpp:10583-10587`).
const CL_EQUIPPED_NOTICE: &[u8] =
    b"[Transmutation] You cannot transmute an item while it is equipped";
/// The wrong-kind refusal (`:10588-10608`).
const CL_CANNOT_NOTICE: &[u8] = b"[Transmutation] This item cannot be transmuted";
/// The locked refusal (`:10609-10613`).
const CL_LOCKED_NOTICE: &[u8] = b"[Transmutation] You can't add locked items";
/// The already-transmuted refusal (`:10614-10618`).
const CL_ALREADY_NOTICE: &[u8] = b"[Transmutation] You can't add items which are transmuted yet";
/// The slot-1 mismatch refusal (`:10647-10651`).
const CL_SUBMIT_NOTICE: &[u8] = b"[Transmutation] You cannot submit this item";
/// The empty-slot-1 refine refusal (`RefineClMaterials`, `:10715-10719`).
const CL_MISSING_NOTICE: &[u8] = b"[Transmutation] Please submit the item you want to transmute";
/// The poor refine refusal (`:10721-10725`).
const CL_POOR_NOTICE: &[u8] = b"[Transmutation] You don't have enough Gold";
/// The client close of no window (`ChangeLookWindow(false)`, `:10512-10519`).
const CL_NOT_OPEN_NOTICE: &[u8] = b"[Transmutation] The window is not opened";

impl GameState {
    /// The materials of the changelook window of the character under `vid`, slot by slot.
    pub(super) fn changelook_materials(&self, vid: Vid) -> ChangelookMaterials {
        self.changelook_held
            .get(&vid)
            .copied()
            .unwrap_or([None, None])
    }

    /// Whether the item `id` is a material of any open changelook window: the `isLocked` the add
    /// runs. The sash and changelook share the lock bit; the toggle and soul locks are not ported,
    /// so the two windows are the only writers.
    pub(super) fn changelook_locked(&self, id: ItemId) -> bool {
        self.sash_locked(id)
            || self
                .changelook_held
                .values()
                .any(|held| held.contains(&Some(id)))
    }

    /// Whether the item at `at` of the character under `vid` is a material of its open
    /// changelook window: the `isLocked` check the destroy, drop, move, gift, shop, safebox and
    /// scroll-refine paths run.
    pub(super) fn changelook_item_at(&self, vid: Vid, at: ItemPos) -> bool {
        let held = self.changelook_materials(vid);
        if held == [None, None] {
            return false;
        }
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .is_some_and(|item| held.contains(&Some(item.id)))
    }

    /// Run one `CG_CL` for the character online under `vid` (`CInputMain::ChangeLook`,
    /// `G/input_main.cpp:3431-3459`), answered as a move is.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when no character is online under `vid`.
    pub fn changelook(
        &mut self,
        vid: Vid,
        sub_header: u8,
        (pos, at): (u8, ItemPos),
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        let mut moved = MovedItems::new(
            pid,
            vid.raw(),
            MoveDone {
                kind: MoveKind::Refined,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        );
        if self.quest_running(vid) {
            return Ok(moved);
        }
        match sub_header {
            CL_CLOSE => self.close_changelook(vid, actor, &mut moved),
            CL_ADD => self.add_changelook_material(vid, pos, at, actor, &mut moved),
            CL_REMOVE => self.remove_changelook_material(vid, pos, &mut moved),
            CL_REFINE => self.refine_changelook_materials(vid, actor, &mut moved),
            _ => {}
        }
        Ok(moved)
    }

    /// `ChangeLookWindow(false)` (`G/char.cpp:10504-10535`): without a window the not-open line
    /// chats (the client close passes `bRequest = false`); else both materials unlock and the
    /// close record answers with cost 0.
    fn close_changelook(&mut self, vid: Vid, actor: Mover, moved: &mut MovedItems) {
        if !self.changelook_windows.remove(&vid) {
            moved
                .records
                .push(self.cl_line(actor, CL_NOT_OPEN_NOTICE, &[]));
            return;
        }
        self.changelook_held.remove(&vid);
        let close = GcChangeLook::new(CL_SUBHEADER_CLOSE, 0, 0, ItemPos::new(INVENTORY_WINDOW, 0));
        moved.records.push(close.encode());
    }

    /// `AddClMaterial` (`G/char.cpp:10563-10672`): the slot resolution and the guard chain in
    /// legacy's order, then the held id and the `ADD` record.
    #[allow(clippy::too_many_lines)]
    fn add_changelook_material(
        &mut self,
        vid: Vid,
        pos: u8,
        at: ItemPos,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        if !self.changelook_windows.contains(&vid) {
            return;
        }
        let mut held = self.changelook_materials(vid);
        let Some(slot) = Self::cl_slot(&held, pos) else {
            return;
        };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let Some(item) = character.items().item_at(at).cloned() else {
            return;
        };
        if !world::character::is_valid_item_position(at)
            || item.pos.cell >= INVENTORY_MAX_NUM
            || world::character::is_belt_inventory_position(at)
        {
            return;
        }
        if world::character::is_equip_position(at) {
            moved
                .records
                .push(self.cl_line(actor, CL_EQUIPPED_NOTICE, &[]));
            return;
        }
        let Some(proto) = self.protos.get(item.vnum).cloned() else {
            return;
        };
        if !Self::cl_transmutable(&proto) {
            moved
                .records
                .push(self.cl_line(actor, CL_CANNOT_NOTICE, &[]));
            return;
        }
        if self.changelook_locked(item.id) {
            moved
                .records
                .push(self.cl_line(actor, CL_LOCKED_NOTICE, &[]));
            return;
        }
        if item.transmutation != 0 {
            moved
                .records
                .push(self.cl_line(actor, CL_ALREADY_NOTICE, &[]));
            return;
        }
        if slot == 1 && held[0].is_none() {
            return;
        }
        if held[slot].is_some() {
            return;
        }
        if slot == 1 && !self.cl_slot1_fits(vid, held[0], &item, &proto) {
            moved
                .records
                .push(self.cl_line(actor, CL_SUBMIT_NOTICE, &[]));
            return;
        }
        held[slot] = Some(item.id);
        self.changelook_held.insert(vid, held);
        let added = GcChangeLook::new(CL_SUBHEADER_ADD, 0, pos_slot(slot), at);
        moved.records.push(added.encode());
    }

    /// The request's slot: a slot below 2, or the first free slot when the client sent
    /// 255 (`AddClMaterial`, `G/char.cpp:10566-10575`). `None` for any other `pos`.
    fn cl_slot(held: &ChangelookMaterials, pos: u8) -> Option<usize> {
        if usize::from(pos) < CL_MATERIALS {
            return Some(usize::from(pos));
        }
        if pos != CL_ANY_SLOT {
            return None;
        }
        held.iter().position(Option::is_none)
    }

    /// Whether the proto is a weapon (but no arrow or mount spear), a body armour, or a body,
    /// hair, weapon or sash costume (`AddClMaterial`, `G/char.cpp:10588-10608`;
    /// `ENABLE_WEAPON_COSTUME_SYSTEM` is on).
    fn cl_transmutable(proto: &gamedata::item_proto::ItemProto) -> bool {
        match proto.item_type {
            ITEM_WEAPON => proto.sub_type != WEAPON_ARROW && proto.sub_type != WEAPON_MOUNT_SPEAR,
            ITEM_ARMOR => proto.sub_type == ARMOR_BODY,
            ITEM_COSTUME => {
                proto.sub_type == COSTUME_BODY
                    || proto.sub_type == COSTUME_HAIR
                    || proto.sub_type == COSTUME_WEAPON
                    || proto.sub_type == COSTUME_SASH
            }
            _ => false,
        }
    }

    /// Whether slot 1 takes `item`: the type, subtype and original vnum match slot 0's, the sex
    /// antiflags agree either way, and every job antiflag slot 1 names slot 0 names too
    /// (`AddClMaterial`, `G/char.cpp:10628-10653`). `GetOriginalVnum` is the vnum
    /// (`G/item.h:107`), and `GetAntiFlag` is the proto's (`G/item.h:78`).
    fn cl_slot1_fits(
        &self,
        vid: Vid,
        first: Option<ItemId>,
        item: &world::item::Item,
        proto: &gamedata::item_proto::ItemProto,
    ) -> bool {
        let Some(first) = first else {
            return true;
        };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return false;
        };
        let (Some(one), Some(one_proto)) = (
            character.items().item(first),
            character
                .items()
                .item(first)
                .and_then(|held| self.protos.get(held.vnum)),
        ) else {
            return false;
        };
        if one_proto.item_type != proto.item_type || one_proto.sub_type != proto.sub_type {
            return false;
        }
        if one.vnum == item.vnum {
            return false;
        }
        let (kept, named) = (one_proto.anti_flags, proto.anti_flags);
        if (kept & ITEM_ANTIFLAG_FEMALE != 0) != (named & ITEM_ANTIFLAG_FEMALE != 0) {
            return false;
        }
        if (kept & ITEM_ANTIFLAG_MALE != 0) != (named & ITEM_ANTIFLAG_MALE != 0) {
            return false;
        }
        for flag in [
            ITEM_ANTIFLAG_WARRIOR,
            ITEM_ANTIFLAG_ASSASSIN,
            ITEM_ANTIFLAG_SHAMAN,
            ITEM_ANTIFLAG_SURA,
        ] {
            if named & flag != 0 && kept & flag == 0 {
                return false;
            }
        }
        true
    }

    /// `RemoveClMaterial` (`G/char.cpp:10674-10708`): slot 1 unlocks alone while slot 0 clears
    /// both; both answer `REMOVE` with a zeroed position. An empty slot sends nothing.
    fn remove_changelook_material(&mut self, vid: Vid, pos: u8, moved: &mut MovedItems) {
        if usize::from(pos) >= CL_MATERIALS {
            return;
        }
        let mut held = self.changelook_materials(vid);
        if held[usize::from(pos)].is_none() {
            return;
        }
        if pos == 1 {
            held[1] = None;
        } else {
            held = [None, None];
        }
        self.changelook_held.insert(vid, held);
        let removed = GcChangeLook::new(
            CL_SUBHEADER_REMOVE,
            0,
            pos,
            ItemPos::new(INVENTORY_WINDOW, 0),
        );
        moved.records.push(removed.encode());
    }

    /// `RefineClMaterials` (`G/char.cpp:10710-10744`): the missing-material lines and the poor
    /// chat send no record; a success spends the gold, destroys slot 1, stamps slot 1's vnum
    /// into slot 0's transmutation, clears the window, and answers `REFINE`.
    fn refine_changelook_materials(&mut self, vid: Vid, actor: Mover, moved: &mut MovedItems) {
        let [Some(first), second] = self.changelook_materials(vid) else {
            return;
        };
        let Some(second) = second else {
            moved
                .records
                .push(self.cl_line(actor, CL_MISSING_NOTICE, &[]));
            return;
        };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        if character.gold() < u64::from(CL_TRANSMUTATION_PRICE) {
            moved.records.push(self.cl_line(actor, CL_POOR_NOTICE, &[]));
            return;
        }
        let (Some(one), Some(two)) = (
            character.items().item(first).cloned(),
            character.items().item(second).cloned(),
        ) else {
            return;
        };
        let look = two.vnum;
        let update = {
            let mut stamped = one.clone();
            stamped.transmutation = look;
            stamped.gc_item_update()
        };
        let pos1 = two.pos;
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let held = character.gold();
        let left = held - u64::from(CL_TRANSMUTATION_PRICE);
        character.set_gold(left);
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let _ = items.set_transmutation(first, look);
        let mut done = MoveDone {
            kind: MoveKind::Refined,
            records: Vec::new(),
            changes: Vec::new(),
        };
        done.records
            .push(MoveRecord::Item(world::character::ItemRecord::Update(
                update,
            )));
        done.changes
            .push(world::character::ItemChange::Transmuted { id: first, look });
        done.records
            .push(MoveRecord::QuickslotSync(world::character::QuickslotSync {
                from: pos1.cell,
                to: world::character::SyncTo::Delete,
            }));
        if items.release(second).is_ok() {
            done.records
                .push(MoveRecord::Item(world::character::ItemRecord::Set(
                    world::item::gc_item_clear(pos1),
                )));
            done.changes
                .push(world::character::ItemChange::Destroyed { id: second });
        }
        done.records.push(MoveRecord::Gold {
            amount: 0,
            value: left,
        });
        world::character::sync_quickslots(&mut done, slots, items);
        let quickslots = character.quickslots().clone();
        let points = character.points().cloned();
        let pid = moved.owner_id;
        let done = MovedItems::new(pid, vid.raw(), done, actor, &self.protos, &self.locale);
        moved.records.extend(done.records);
        moved.changes.extend(done.changes);
        moved.gold = Some(gold_delta(held, left));
        moved.points = points;
        moved.quickslots = Some(quickslots);
        self.changelook_windows.remove(&vid);
        self.changelook_held.remove(&vid);
        let refined =
            GcChangeLook::new(CL_SUBHEADER_REFINE, 0, 0, ItemPos::new(INVENTORY_WINDOW, 0));
        moved.records.push(refined.encode());
    }

    /// A changelook `LC_TEXT` key chat as an `INFO` line. The keys name no locale row, so the
    /// raw key goes out.
    fn cl_line(&self, actor: Mover, key: &[u8], args: &[Arg<'_>]) -> Vec<u8> {
        chat_packet(actor.recipient(&self.locale), CHAT_TYPE_INFO, key, args)
    }
}

/// The `ADD` record's slot: `bPos` after the 255 resolution. The slot is 0 or 1 by
/// construction, so the narrow cannot truncate.
const fn pos_slot(slot: usize) -> u8 {
    if slot == 0 {
        0
    } else {
        1
    }
}
