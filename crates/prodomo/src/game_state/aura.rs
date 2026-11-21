//! The aura refine window: the `CG_AURA` request (`CInputMain::Aura`,
//! `G/input_main.cpp:5294-5340`), which checks in, checks out, accepts and cancels
//! the two materials the aura quest's opener stood up (`G/char_aura.cpp:14-1071`).
//!
//! The request runs with no observer gate (`G/input_main.cpp:3865-3868` sits outside
//! the guarded band) and no waiting-quest guard (unlike the sash, changelook and
//! cube, `CInputMain::Aura` checks no `IsRunning`). An unknown sub-header falls
//! through the switch and consumes the prefix alone, as legacy's missing `default:`
//! does.
//!
//! The window is `aura_windows`; its materials are item ids in `aura_held`, so a
//! held item stays held when it moves cells, as legacy's `TItemPos` slots do not —
//! a move buries the held item there. The Rewrite keeps the id, as the sash does,
//! and refuses a move of or onto a held item rather than lose it (a Divergence,
//! recorded in `docs/STATUS.md`).
//!
//! A cancel (`AuraRefineWindowClose`, `:64-98`) answers the close record only when
//! the window stands, and clears both slots. Death closes the window with its record
//! (`Dead`, `G/char_battle.cpp:1629-1631`); a disconnect closes it too
//! (`CHARACTER::Disconnect`, `G/char.cpp:1809-1810`), while a warp keeps it, and the
//! re-entered character finds its window and its locks waiting.
//!
//! The check-in (`AuraRefineWindowCheckIn`, `:239-691`) guards in legacy's order:
//! the aura slot must be a refine cell below the result, the window must stand, the
//! refine must be allowed (whose `CanHandleItem` always fails while the window
//! stands, so the opener's presence carries the check), and the named window must
//! match the standing one. Only the absorb window is ported: a growth or evolve
//! check-in runs the shared guards and then nothing, as those `switch` arms wait
//! for their systems.
//!
//! The absorb guards are the position (`SItemPos::IsValidItemPosition`), the worn
//! cell, the held item, the offered item (silent), the locked item, the main's aura
//! costume, the main's empty drain socket, and the sub's shield, bracelet, necklace
//! or earring (`:282-344`). A filled slot refuses silently; else the item locks and
//! the `SET_ITEM` record answers, with the result preview when both slots fill.
//! `ENABLE_SEALBIND_SYSTEM` is undefined, so the bound checks do not compile.
//!
//! The check-out (`AuraRefineWindowCheckOut`, `:693-774`) answers `CLEAR_SLOT` for
//! one slot, or `CLEAR_ALL` when the main leaves with the sub filled. The accept
//! (`AuraRefineWindowAccept`, `:776-871`) needs both slots, the held pair, no
//! offered pair and no worn pair; then the sub's attributes move onto the main
//! (`CopyAttributeTo`), the sub's vnum stamps the main's drain socket, the main's
//! update goes out, the sub is destroyed, and `CLEAR_ALL` answers. The chat is
//! `[LS;1481]`. The money and item logs are `sys.log`'s, which owns no sink.
//!
//! `G/` = `server/server/game/`, `C/` = `server/server/common/`.

use common::vid::Vid;
use gamedata::item_kind::{
    ARMOR_EAR, ARMOR_NECK, ARMOR_SHIELD, ARMOR_WRIST, COSTUME_AURA, ITEM_ARMOR, ITEM_COSTUME,
};
use protocol::cg_aura::{
    AURA_SUBHEADER_CG_REFINE_ACCEPT, AURA_SUBHEADER_CG_REFINE_CANCEL,
    AURA_SUBHEADER_CG_REFINE_CHECKIN, AURA_SUBHEADER_CG_REFINE_CHECKOUT,
};
use protocol::gc_aura::{
    GcAuraClearAll, GcAuraClearSlot, GcAuraClose, GcAuraSetItem, WINDOW_ABSORB,
};
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::item_pos::ItemPos;
use world::character::{is_equip_position, is_valid_item_position, ItemChange, ItemRecord};
use world::character::{MoveDone, MoveKind, MoveRecord};
use world::item::ItemId;

use super::GameState;
use crate::chat_line::{chat_packet, Arg};
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

/// The materials the aura window of each character holds, slot by slot, keyed by its
/// VID: `m_pAuraRefineWindowItemSlot` (`G/char.h:2469`), as the held item ids
/// (absent when the window stands empty or no window stands).
pub(super) type AuraMaterials = [Option<ItemId>; 2];

/// `AURA_SLOT_MAIN` (`C/length.h:1098`).
pub const AURA_SLOT_MAIN: u8 = 0;
/// `AURA_SLOT_SUB` (`C/length.h:1099`).
pub const AURA_SLOT_SUB: u8 = 1;
/// `AURA_SLOT_RESULT` (`C/length.h:1100`): the check-in refuses this cell and past it.
pub const AURA_SLOT_RESULT: u8 = 2;

/// `EWindows::AURA_REFINE` (`C/item_slots.rs:120`), the only window the aura slot takes.
pub const AURA_REFINE_WINDOW: u8 = 7;

/// `ITEM_SOCKET_AURA_DRAIN_ITEM_VNUM` (`C/item_length.h:473`): socket 0.
pub const AURA_DRAIN_SOCKET: usize = 0;

/// The no-window check-in line (`G/char_aura.cpp:253`).
const AURA_NO_WINDOW_NOTICE: &[u8] =
    b"<Aura> You need to open the specific aura refine window at first.";
/// The shut-windows check-in line (`:269`).
const AURA_SHUT_WINDOWS_NOTICE: &[u8] = b"<Aura> Close every window to continue the process.";
/// The wrong-window line (`:276`, `:736`, `:812`).
const AURA_WRONG_WINDOW_NOTICE: &[u8] =
    b"<Aura> The aura refine process has encountered an error and cannot be continued.";
/// The wrong-window check-out and accept line (`:736`, `:812`).
const AURA_OTHER_WINDOW_NOTICE: &[u8] =
    b"<Aura> You cannot do this while there is another aura window is opened.";
/// The absorb position line (`:286`).
const AURA_ABSORB_POSITION_NOTICE: &[u8] = b"<Aura> You cannot absorb this item.";
/// The absorb worn line (`:292`).
const AURA_ABSORB_WORN_NOTICE: &[u8] = b"<Aura> You cannot attach equipped items for absorption.";
/// The missing-item line (`:298`).
const AURA_MISSING_NOTICE: &[u8] = b"<Aura> An undefined error has occurred.";
/// The absorb locked line (`:313`).
const AURA_ABSORB_LOCKED_NOTICE: &[u8] =
    b"<Aura> This item is used, cannot be attached for absorption.";
/// The no-window check-out and accept line (`:706`, `:786`).
const AURA_NO_WINDOW_TAIL_NOTICE: &[u8] =
    b"<Aura> You cannot do this while there is no aura window opened.";
/// The missing-pair accept line (`:818`).
const AURA_ATTACH_NOTICE: &[u8] = b"<Aura> Attach the items in the window.";
/// The worn-pair accept line (`:835`).
const AURA_WORN_PAIR_NOTICE: &[u8] = b"<Aura> You cannot use equipped items.";
/// The absorb success (`:846`).
const AURA_ABSORB_SUCCESS: &[u8] = b"[LS;1481]";

impl GameState {
    /// The standing window of the character under `vid`: its type and its opener's VID.
    /// `None` when no window stands.
    pub(super) fn aura_window(&self, vid: Vid) -> Option<(u8, u32)> {
        self.aura_windows.get(&vid).copied()
    }

    /// The materials of the aura window of the character under `vid`, slot by slot.
    pub(super) fn aura_materials(&self, vid: Vid) -> AuraMaterials {
        self.aura_held.get(&vid).copied().unwrap_or([None, None])
    }

    /// Whether the item `id` is a material of any open aura window: `CItem::isLocked`
    /// as the aura sets it. The sash, changelook and aura share the lock bit; the
    /// toggle and soul locks are not ported, so the three windows are the only writers.
    pub(super) fn aura_locked(&self, id: ItemId) -> bool {
        self.sash_locked(id)
            || self
                .changelook_held
                .values()
                .any(|held| held.contains(&Some(id)))
            || self.aura_held.values().any(|held| held.contains(&Some(id)))
    }

    /// Whether the item at `at` of the character under `vid` is a material of its open
    /// aura window: the `isLocked` check the destroy, drop, move, gift, shop, safebox and
    /// scroll-refine paths run.
    pub(super) fn aura_item_at(&self, vid: Vid, at: ItemPos) -> bool {
        let held = self.aura_materials(vid);
        if held == [None, None] {
            return false;
        }
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .is_some_and(|item| held.contains(&Some(item.id)))
    }

    /// An aura `LC_TEXT` line as an `INFO` chat. The keys name no locale row, so the
    /// raw key goes out.
    fn aura_line(&self, actor: Mover, key: &[u8]) -> Vec<u8> {
        chat_packet(
            actor.recipient(&self.locale),
            CHAT_TYPE_INFO,
            key,
            &[] as &[Arg<'_>],
        )
    }

    /// Run one `CG_AURA` for the character online under `vid` (`CInputMain::Aura`,
    /// `G/input_main.cpp:5294-5340`): check in, check out, accept, or cancel. An
    /// unknown sub-header is absorbed, as legacy's missing `default:` does.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn aura(
        &mut self,
        vid: Vid,
        sub_header: u8,
        (item_cell, aura_cell, window): (ItemPos, ItemPos, u8),
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
        match sub_header {
            AURA_SUBHEADER_CG_REFINE_CHECKIN => {
                self.checkin_aura_material(vid, item_cell, aura_cell, window, actor, &mut moved);
            }
            AURA_SUBHEADER_CG_REFINE_CHECKOUT => {
                self.checkout_aura_material(vid, aura_cell, window, actor, &mut moved);
            }
            AURA_SUBHEADER_CG_REFINE_ACCEPT => {
                self.accept_aura_materials(vid, window, actor, &mut moved);
            }
            AURA_SUBHEADER_CG_REFINE_CANCEL => self.close_aura(vid, &mut moved),
            _ => {}
        }
        Ok(moved)
    }

    /// `AuraRefineWindowClose` (`G/char_aura.cpp:64-98`): without a window nothing is
    /// sent; else both materials unlock and the close record answers.
    fn close_aura(&mut self, vid: Vid, moved: &mut MovedItems) {
        if self.aura_windows.remove(&vid).is_none() {
            return;
        }
        self.aura_held.remove(&vid);
        moved.records.push(GcAuraClose.encode().to_vec());
    }

    /// Whether the aura window of the character under `vid` may refine
    /// (`IsAuraRefineWindowCanRefine`, `G/char_aura.cpp:100-118`): the `CanHandleItem`
    /// there always fails while the window stands (the aura's own arm refuses it), so
    /// only the standing window and the opener within 1000 count.
    fn aura_may_refine(&self, vid: Vid) -> bool {
        let Some((_, opener)) = self.aura_window(vid) else {
            return false;
        };
        let (Some(place), Some(kept)) = (self.place_of(vid.raw()), self.kept_place(vid)) else {
            return false;
        };
        self.npcs
            .get(&(place.0, place.1))
            .and_then(|npcs| npcs.npcs.iter().find(|npc| npc.vid == opener))
            .is_some_and(|npc| {
                crate::sync_position::distance_approx(npc.x - kept.1, npc.y - kept.2) < 1000
            })
    }

    /// `AuraRefineWindowCheckIn` (`G/char_aura.cpp:239-419`): the shared guards, then
    /// the absorb arm. A growth or evolve check-in runs the shared guards and then
    /// nothing, as those arms wait for their systems.
    fn checkin_aura_material(
        &mut self,
        vid: Vid,
        item_cell: ItemPos,
        aura_cell: ItemPos,
        window: u8,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        if aura_cell.window_type != AURA_REFINE_WINDOW
            || aura_cell.cell >= u16::from(AURA_SLOT_RESULT)
        {
            return;
        }
        let Some((standing, _)) = self.aura_window(vid) else {
            moved
                .records
                .push(self.aura_line(actor, AURA_NO_WINDOW_NOTICE));
            return;
        };
        if !self.aura_may_refine(vid) {
            if self.aura_window(vid).is_some() {
                return;
            }
            moved
                .records
                .push(self.aura_line(actor, AURA_SHUT_WINDOWS_NOTICE));
            return;
        }
        if standing != window {
            moved
                .records
                .push(self.aura_line(actor, AURA_WRONG_WINDOW_NOTICE));
            return;
        }
        if window != WINDOW_ABSORB {
            return;
        }
        self.checkin_aura_absorb(vid, item_cell, aura_cell, actor, moved);
    }

    /// The absorb arm of `AuraRefineWindowCheckIn` (`G/char_aura.cpp:282-419`): the
    /// guard chain in legacy's order, then the held id and the `SET_ITEM` record,
    /// with the result preview when both slots fill.
    #[allow(clippy::too_many_lines)]
    fn checkin_aura_absorb(
        &mut self,
        vid: Vid,
        item_cell: ItemPos,
        aura_cell: ItemPos,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        if !is_valid_item_position(item_cell) {
            moved
                .records
                .push(self.aura_line(actor, AURA_ABSORB_POSITION_NOTICE));
            return;
        }
        if is_equip_position(item_cell) {
            moved
                .records
                .push(self.aura_line(actor, AURA_ABSORB_WORN_NOTICE));
            return;
        }
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let Some(item) = character.items().item_at(item_cell).cloned() else {
            moved
                .records
                .push(self.aura_line(actor, AURA_MISSING_NOTICE));
            return;
        };
        if character.items().is_exchanging(item.id) {
            return;
        }
        if self.aura_locked(item.id) || self.shaman_locked(vid, item.id) {
            moved
                .records
                .push(self.aura_line(actor, AURA_ABSORB_LOCKED_NOTICE));
            return;
        }
        let Some(proto) = self.protos.get(item.vnum).cloned() else {
            moved
                .records
                .push(self.aura_line(actor, AURA_MISSING_NOTICE));
            return;
        };
        let slot = aura_cell.cell;
        if slot == u16::from(AURA_SLOT_MAIN)
            && !(proto.item_type == ITEM_COSTUME && proto.sub_type == COSTUME_AURA)
        {
            moved.records.push(self.aura_line(actor, b"[LS;1472]"));
            return;
        }
        if slot == u16::from(AURA_SLOT_MAIN) && item.sockets[AURA_DRAIN_SOCKET] != 0 {
            moved.records.push(self.aura_line(actor, b"[LS;1469]"));
            return;
        }
        let absorbable = proto.item_type == ITEM_ARMOR
            && (proto.sub_type == ARMOR_SHIELD
                || proto.sub_type == ARMOR_WRIST
                || proto.sub_type == ARMOR_NECK
                || proto.sub_type == ARMOR_EAR);
        if slot == u16::from(AURA_SLOT_SUB) && !absorbable {
            moved.records.push(self.aura_line(actor, b"[LS;1473]"));
            return;
        }
        let mut held = self.aura_materials(vid);
        if held[usize::from(slot)].is_some() {
            return;
        }
        held[usize::from(slot)] = Some(item.id);
        self.aura_held.insert(vid, held);
        moved.records.push(
            GcAuraSetItem {
                cell: item.pos,
                aura_cell,
                vnum: item.vnum,
                count: item.count,
                refine_element: item.refine_element,
                transmutation: item.transmutation,
                flags: item.flags,
                anti_flags: item.anti_flags,
                sockets: item.sockets,
                attributes: item.attributes,
            }
            .encode(),
        );
        self.preview_aura_absorb(vid, moved);
    }

    /// The absorb result preview (`G/char_aura.cpp:374-407`): when both slots hold
    /// a live, valid, unoffered, unworn pair, a second `SET_ITEM` copies the main
    /// with the sub's attributes and the sub's vnum in the drain socket, addressed
    /// to the reserved window and the result slot. The main-main double check is
    /// legacy's own (`m_pAuraRefineWindowItemSlot[AURA_SLOT_MAIN]` twice); the
    /// Rewrite checks both slots, as the sub read two lines down assumes.
    fn preview_aura_absorb(&self, vid: Vid, moved: &mut MovedItems) {
        let [Some(first), Some(second)] = self.aura_materials(vid) else {
            return;
        };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let (Some(one), Some(two)) = (
            character.items().item(first),
            character.items().item(second),
        ) else {
            return;
        };
        if !is_valid_item_position(one.pos)
            || !is_valid_item_position(two.pos)
            || character.items().is_exchanging(first)
            || character.items().is_exchanging(second)
            || is_equip_position(one.pos)
            || is_equip_position(two.pos)
        {
            return;
        }
        let mut sockets = one.sockets;
        sockets[AURA_DRAIN_SOCKET] = i32::try_from(two.vnum).unwrap_or(i32::MAX);
        moved.records.push(
            GcAuraSetItem {
                cell: ItemPos::new(common::item_slots::EWindows::ReservedWindow as u8, 0),
                aura_cell: ItemPos::new(AURA_REFINE_WINDOW, u16::from(AURA_SLOT_RESULT)),
                vnum: one.vnum,
                count: one.count,
                refine_element: one.refine_element,
                transmutation: one.transmutation,
                flags: one.flags,
                anti_flags: one.anti_flags,
                sockets,
                attributes: two.attributes,
            }
            .encode(),
        );
    }

    /// `AuraRefineWindowCheckOut` (`G/char_aura.cpp:693-774`): the shared guards, then
    /// `CLEAR_SLOT` for one slot, or `CLEAR_ALL` when the main leaves with the sub
    /// filled.
    fn checkout_aura_material(
        &mut self,
        vid: Vid,
        aura_cell: ItemPos,
        window: u8,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        if aura_cell.window_type != AURA_REFINE_WINDOW
            || aura_cell.cell >= u16::from(AURA_SLOT_RESULT)
        {
            return;
        }
        let Some((standing, _)) = self.aura_window(vid) else {
            moved
                .records
                .push(self.aura_line(actor, AURA_NO_WINDOW_TAIL_NOTICE));
            return;
        };
        if !self.aura_may_refine(vid) {
            if self.aura_window(vid).is_some() {
                return;
            }
            moved
                .records
                .push(self.aura_line(actor, AURA_SHUT_WINDOWS_NOTICE));
            return;
        }
        if standing != window {
            moved
                .records
                .push(self.aura_line(actor, AURA_OTHER_WINDOW_NOTICE));
            return;
        }
        let slot = aura_cell.cell;
        let mut held = self.aura_materials(vid);
        if held[usize::from(slot)].is_none() {
            return;
        }
        if slot == u16::from(AURA_SLOT_MAIN) && held[usize::from(AURA_SLOT_SUB)].is_some() {
            held = [None, None];
            self.aura_held.insert(vid, held);
            moved.records.push(GcAuraClearAll.encode().to_vec());
            return;
        }
        held[usize::from(slot)] = None;
        self.aura_held.insert(vid, held);
        let slot = u8::try_from(slot).unwrap_or(u8::MAX);
        moved
            .records
            .push(GcAuraClearSlot { slot }.encode().to_vec());
    }

    /// The absorb arm of `AuraRefineWindowAccept` (`G/char_aura.cpp:776-871`): the
    /// shared guards, then the sub's attributes onto the main, the sub's vnum into
    /// the main's drain socket, the main's update, the sub destroyed, and `CLEAR_ALL`.
    /// A growth or evolve accept runs the shared guards and then nothing.
    fn accept_aura_materials(
        &mut self,
        vid: Vid,
        window: u8,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        let Some((standing, _)) = self.aura_window(vid) else {
            moved
                .records
                .push(self.aura_line(actor, AURA_NO_WINDOW_TAIL_NOTICE));
            return;
        };
        if !self.aura_may_refine(vid) {
            if self.aura_window(vid).is_some() {
                return;
            }
            moved
                .records
                .push(self.aura_line(actor, AURA_SHUT_WINDOWS_NOTICE));
            return;
        }
        if standing != window {
            moved
                .records
                .push(self.aura_line(actor, AURA_OTHER_WINDOW_NOTICE));
            return;
        }
        if window != WINDOW_ABSORB {
            return;
        }
        let [Some(first), Some(second)] = self.aura_materials(vid) else {
            moved
                .records
                .push(self.aura_line(actor, AURA_ATTACH_NOTICE));
            return;
        };
        let Some((one, two)) = self.aura_absorb_pair(vid, first, second) else {
            return;
        };
        if is_equip_position(one.pos) || is_equip_position(two.pos) {
            moved
                .records
                .push(self.aura_line(actor, AURA_WORN_PAIR_NOTICE));
            return;
        }
        self.absorb_aura_pair(vid, (first, second), one, &two, actor, moved);
    }

    /// The absorb pair of `AuraRefineWindowAccept` (`:827-833`): both materials live,
    /// valid and unoffered. `None` otherwise, silent as legacy is.
    fn aura_absorb_pair(
        &self,
        vid: Vid,
        first: ItemId,
        second: ItemId,
    ) -> Option<(world::item::Item, world::item::Item)> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let (one, two) = (
            character.items().item(first).cloned()?,
            character.items().item(second).cloned()?,
        );
        if !is_valid_item_position(one.pos) || !is_valid_item_position(two.pos) {
            return None;
        }
        if character.items().is_exchanging(first) || character.items().is_exchanging(second) {
            return None;
        }
        Some((one, two))
    }

    /// The absorb write of `AuraRefineWindowAccept` (`:840-871`): the chat, the
    /// attributes and the drain stamp onto the main, the main's update, the sub
    /// destroyed, and `CLEAR_ALL`.
    fn absorb_aura_pair(
        &mut self,
        vid: Vid,
        (first, second): (ItemId, ItemId),
        one: world::item::Item,
        two: &world::item::Item,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        moved
            .records
            .push(self.aura_line(actor, AURA_ABSORB_SUCCESS));
        let (attributes, sockets) = (two.attributes, {
            let mut sockets = one.sockets;
            sockets[AURA_DRAIN_SOCKET] = i32::try_from(two.vnum).unwrap_or(i32::MAX);
            sockets
        });
        let mut updated = one;
        updated.attributes = attributes;
        updated.sockets = sockets;
        let update = updated.gc_item_update();
        let pos1 = two.pos;
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        for (index, attribute) in attributes.iter().enumerate() {
            let _ = items.set_attribute(first, index, *attribute);
        }
        let absorbed = i32::try_from(two.vnum).unwrap_or(i32::MAX);
        let _ = items.set_socket(first, AURA_DRAIN_SOCKET, absorbed);
        let mut done = MoveDone {
            kind: MoveKind::Refined,
            records: Vec::new(),
            changes: Vec::new(),
        };
        done.records
            .push(MoveRecord::Item(ItemRecord::Update(update)));
        done.changes.push(ItemChange::Attributes {
            id: first,
            attributes,
        });
        done.changes
            .push(ItemChange::Sockets { id: first, sockets });
        done.records
            .push(MoveRecord::QuickslotSync(world::character::QuickslotSync {
                from: pos1.cell,
                to: world::character::SyncTo::Delete,
            }));
        if items.release(second).is_ok() {
            done.records.push(MoveRecord::Item(ItemRecord::Set(
                world::item::gc_item_clear(pos1),
            )));
            done.changes.push(ItemChange::Destroyed { id: second });
        }
        world::character::sync_quickslots(&mut done, slots, items);
        let quickslots = character.quickslots().clone();
        let points = character.points().cloned();
        let pid = moved.owner_id;
        let done = MovedItems::new(pid, vid.raw(), done, actor, &self.protos, &self.locale);
        moved.records.extend(done.records);
        moved.changes.extend(done.changes);
        moved.points = points;
        moved.quickslots = Some(quickslots);
        self.aura_held.insert(vid, [None, None]);
        moved.records.push(GcAuraClearAll.encode().to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::fixtures::{a_world, enter_seeing_with, TestClock, PLACE};
    use common::item_slots::EWindows;

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

    /// A world with an aura costume (49011) at cell 0 and a shield (13000) at cell 1,
    /// the absorb window standing on an NPC 700 east.
    fn a_window() -> GameState {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let region = crate::game_state::fixtures::region();
        world.stand_npcs(
            PLACE.0,
            &region,
            world::npc::MapNpcs {
                npcs: vec![world::npc::Npc {
                    vid: 7001,
                    vnum: 60003,
                    race: 60003,
                    x: 3900,
                    y: 3200,
                    hp: 100,
                    max_hp: 100,
                    ..world::npc::Npc::default()
                }],
                positions: Vec::new(),
                regens: Vec::new(),
            },
        );
        let mut costume = world::item::Item::new(7, 49_011);
        costume.size = 1;
        let mut shield = world::item::Item::new(8, 13_000);
        shield.size = 1;
        shield.attributes[0] = protocol::gc_item_window::ItemAttribute::new(7, 26);
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let sub = ItemPos::new(EWindows::Inventory as u8, 1);
        let (_inbox, _) = enter_seeing_with(
            &mut world,
            PLACE,
            1,
            (3200, 3200),
            10_000,
            &[(at, costume), (sub, shield)],
        );
        world
            .aura_windows
            .insert(Vid::new(1), (WINDOW_ABSORB, 7001));
        world
    }

    fn cells(cell: u16, slot: u16) -> (ItemPos, ItemPos, u8) {
        (
            ItemPos::new(EWindows::Inventory as u8, cell),
            ItemPos::new(AURA_REFINE_WINDOW, slot),
            WINDOW_ABSORB,
        )
    }

    /// No window: the check-in, check-out and accept chat their no-window lines, the
    /// cancel stays silent, and an unknown sub-header changes nothing.
    #[test]
    fn without_a_window_only_the_no_window_lines_answer() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let (_inbox, _) = enter_seeing_with(&mut world, PLACE, 1, (3200, 3200), 10_000, &[]);
        let _ = at;
        let moved = world
            .aura(
                Vid::new(1),
                AURA_SUBHEADER_CG_REFINE_CHECKIN,
                cells(0, 0),
                mover(),
            )
            .expect("the check-in runs");
        assert_eq!(moved.records.len(), 1, "the no-window check-in chats");
        let moved = world
            .aura(
                Vid::new(1),
                AURA_SUBHEADER_CG_REFINE_CHECKOUT,
                cells(0, 0),
                mover(),
            )
            .expect("the check-out runs");
        assert_eq!(moved.records.len(), 1, "the no-window check-out chats");
        let moved = world
            .aura(
                Vid::new(1),
                AURA_SUBHEADER_CG_REFINE_ACCEPT,
                cells(0, 0),
                mover(),
            )
            .expect("the accept runs");
        assert_eq!(moved.records.len(), 1, "the no-window accept chats");
        let moved = world
            .aura(
                Vid::new(1),
                AURA_SUBHEADER_CG_REFINE_CANCEL,
                cells(0, 0),
                mover(),
            )
            .expect("the cancel runs");
        assert!(moved.records.is_empty(), "the cancel stays silent");
        let moved = world
            .aura(Vid::new(1), 9, cells(0, 0), mover())
            .expect("the unknown runs");
        assert!(moved.records.is_empty(), "the unknown is absorbed");
    }

    /// The check-out clears one slot, and the main's with the sub filled clears all;
    /// the cancel answers the close; a growth window runs its guards and then nothing.
    #[test]
    fn the_check_out_the_cancel_and_the_growth_guard() {
        let mut world = a_window();
        world
            .aura(
                Vid::new(1),
                AURA_SUBHEADER_CG_REFINE_CHECKIN,
                cells(0, 0),
                mover(),
            )
            .expect("the main lands");
        world
            .aura(
                Vid::new(1),
                AURA_SUBHEADER_CG_REFINE_CHECKIN,
                cells(1, 1),
                mover(),
            )
            .expect("the sub lands");
        let moved = world
            .aura(
                Vid::new(1),
                AURA_SUBHEADER_CG_REFINE_CHECKOUT,
                cells(0, 1),
                mover(),
            )
            .expect("the check-out runs");
        assert_eq!(
            moved.records,
            vec![GcAuraClearSlot { slot: 1 }.encode().to_vec()],
            "one slot clears"
        );
        world
            .aura(
                Vid::new(1),
                AURA_SUBHEADER_CG_REFINE_CHECKIN,
                cells(1, 1),
                mover(),
            )
            .expect("the sub lands again");
        let moved = world
            .aura(
                Vid::new(1),
                AURA_SUBHEADER_CG_REFINE_CHECKOUT,
                cells(0, 0),
                mover(),
            )
            .expect("the main leaves");
        assert_eq!(
            moved.records,
            vec![GcAuraClearAll.encode().to_vec()],
            "the main with the sub filled clears all"
        );
        assert_eq!(world.aura_materials(Vid::new(1)), [None, None]);
        let moved = world
            .aura(
                Vid::new(1),
                AURA_SUBHEADER_CG_REFINE_CANCEL,
                cells(0, 0),
                mover(),
            )
            .expect("the cancel runs");
        assert_eq!(
            moved.records,
            vec![GcAuraClose.encode().to_vec()],
            "the cancel answers the close"
        );
        assert!(world.aura_window(Vid::new(1)).is_none());
        world.aura_windows.insert(Vid::new(1), (1, 7001));
        let moved = world
            .aura(
                Vid::new(1),
                AURA_SUBHEADER_CG_REFINE_CHECKIN,
                (
                    ItemPos::new(EWindows::Inventory as u8, 0),
                    ItemPos::new(AURA_REFINE_WINDOW, 0),
                    1,
                ),
                mover(),
            )
            .expect("the growth check-in runs");
        assert!(
            moved.records.is_empty(),
            "growth runs its guards, then nothing"
        );
        assert_eq!(world.aura_materials(Vid::new(1)), [None, None]);
    }

    /// The lock and the window gates: a held item refuses the destroy, the move and
    /// the gift window, and the open window refuses a destroy of anything.
    #[test]
    fn the_held_and_the_open_refuse_their_steps() {
        use world::character::MoveRequest;
        let mut world = a_window();
        world
            .aura(
                Vid::new(1),
                AURA_SUBHEADER_CG_REFINE_CHECKIN,
                cells(0, 0),
                mover(),
            )
            .expect("the main lands");
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        assert!(world.aura_item_at(Vid::new(1), at));
        assert!(world.destroy_item(Vid::new(1), at, mover()).is_err());
        let to = ItemPos::new(EWindows::Inventory as u8, 5);
        assert!(world
            .move_item(
                Vid::new(1),
                MoveRequest {
                    from: at,
                    to,
                    count: 1
                },
                mover()
            )
            .is_err());
        assert!(world.give_blocked(Vid::new(1)));
    }
}
