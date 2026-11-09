//! The bundle shop: `CG_MYSHOP` opens the seller's own shop window, and
//! `/close_shop` closes it (`CInputMain::MyShop`, `G/input_main.cpp:3255-3314`,
//! `CHARACTER::OpenMyShop` and `CloseMyShop`, `G/char.cpp:805-975` and `:978-1007`).
//!
//! The world keeps each seller's sign and window slots, as legacy's `CShopManager`
//! keeps `m_map_pkShopByPC`. A slot names the held item's id and the asking price;
//! the item itself stays in the seller's inventory, as `SHOP_ITEM::pkItem` points
//! at the live item. Buying from the window is the next slice's; this one builds
//! the window, shows the sign, and closes.
//!
//! # The order
//!
//! The input gates run first: at the yang cap the line refuses, stunned or dead is
//! silent, and a trade, an open safebox, a browsed shop, an open cube or an open
//! aura window refuses with `[LS;1014]`. Then `CanHandleItem` refuses with
//! `[LS;841]` (an observer, an open own shop, a smith's window, a cube, a dragon
//! soul window, a sash, an aura or a changelook window); a warp cannot run a
//! command, so its arm is vacuous. A running quest and an empty build are silent.
//! A total at the yang cap refuses with the Romanian line, an empty sign is
//! silent, and a banned word refuses with `[LS;812]`. Each table is then checked:
//! a repeated cell is silent, an ungiveable or unsellable-in-a-shop item refuses
//! with `[LS;813]`, a worn one with `[LS;1060]`, and a window-locked one with
//! `[LS;1133]`. Without a silk bundle the build spends one paper bundle, and
//! without either it is silent. Then the sign goes around, the window stands, the
//! polymorph affect goes, a summoned horse is dismissed (or else a mount is
//! unlaid), and the seller shows as the bundle mob, 30000.
//!
//! A re-open with a shop standing refuses with `[LS;841]`: `CanHandleItem` sees
//! the open shop before `OpenMyShop` reaches its close-and-return, so the toggle
//! is dead code in legacy and stays dead here. The close runs on `/close_shop`,
//! on death and stun, and on leaving the world: the sign clears around and the
//! seller shows as its job again.
//!
//! The silk bundle's price list is a DB-server packet (`HEADER_GD_MYSHOP_PRICELIST_UPDATE`)
//! with no client record; the market-price slice reads the shops themselves.
//!
//! Everything here runs on the game thread only (ADR-0002).

use std::collections::BTreeSet;

use common::item_slots::EWindows;
use common::vid::Vid;
use protocol::cg_myshop::{CgMyShopTable, CG_MYSHOP_SIGN_FIELD_SIZE};
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_fields::{GcShopSign, GC_SHOP_SIGN_FIELD_SIZE};
use world::character::{count_specify_item, race_to_job, remove_specify_item, Character, Points};
use world::item::{ItemId, ITEM_ANTIFLAG_GIVE, ITEM_ANTIFLAG_MYSHOP};

use super::view::EntityKey;
use super::GameState;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

/// `SHOP_HOST_ITEM_MAX_NUM` (`C/length.h:44`): the window's slots.
pub const MYSHOP_WINDOW_SLOTS: usize = 40;

/// The shop grid's width: `CGrid(5, 9)` (`G/shop.cpp:31`).
const MYSHOP_GRID_WIDTH: usize = 5;

/// The shop grid's height.
const MYSHOP_GRID_HEIGHT: usize = 9;

/// `30000`: the bundle mob the seller shows as (`char.cpp:974`).
pub const MYSHOP_POLYMORPH_VNUM: u32 = 30000;

/// The silk bundle: kept, and its prices listed (`char.cpp:918-943`).
pub const MYSHOP_SILK_BUNDLE: u32 = 71049;

/// The paper bundle: spent to open (`char.cpp:944-945`).
pub const MYSHOP_PAPER_BUNDLE: u32 = 50200;

/// `GOLD_MAX_MAX` (`C/length.h:100`): the yang cap the open refuses at.
pub const MYSHOP_GOLD_CAP: u64 = 1_200_000_000_000_000_000;

/// One window slot: the held item's id and the asking price in yang.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MyShopSlot {
    /// The held item's id.
    pub item: ItemId,
    /// The asking price in yang.
    pub price: u64,
}

/// A seller's open bundle shop: the sign and the window slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyShop {
    /// The sign as shown, NUL-padded.
    pub sign: [u8; GC_SHOP_SIGN_FIELD_SIZE],
    /// The window slots.
    pub slots: [Option<MyShopSlot>; MYSHOP_WINDOW_SLOTS],
}

impl GameState {
    /// Whether the character under `vid` sells from an open bundle shop.
    pub(super) fn has_my_shop(&self, vid: Vid) -> bool {
        self.my_shops.contains_key(&vid)
    }

    /// `CInputMain::MyShop` and `CHARACTER::OpenMyShop` for the character under
    /// `vid`: the gates in legacy order, then the build. The spend's records and
    /// the sign go out immediately, in legacy order (spend, sign, polymorph), as
    /// `set_polymorph` emits immediately too; a relayed sign would reach a
    /// witness after the polymorph's view. The spend's rows store as a move's.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn myshop(
        &mut self,
        vid: Vid,
        sign: [u8; CG_MYSHOP_SIGN_FIELD_SIZE],
        tables: &[CgMyShopTable],
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
            world::character::MoveDone {
                kind: world::character::MoveKind::Moved,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        );
        if self.gold_of(vid) >= MYSHOP_GOLD_CAP {
            // `LC_TEXT` answers the text itself (`chat_line::format_line`).
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You reached the yang limit.",
                &[],
            );
            return Ok(moved);
        }
        if self.is_stunned(vid.raw()) || self.is_dead(vid.raw()) {
            return Ok(moved);
        }
        if self.trade_partner(vid).is_some()
            || self.safebox_open(vid)
            || self.browsing.contains_key(&vid)
            || self.cubes.contains_key(&vid)
            || self.aura_windows.contains_key(&vid)
        {
            self.tell(vid, actor, CHAT_TYPE_INFO, b"[LS;1014]", &[]);
            return Ok(moved);
        }
        if self.is_observing(vid)
            || self.has_my_shop(vid)
            || self.under_refine(vid)
            || self.cubes.contains_key(&vid)
            || self.ds_windows.contains(&vid)
            || self.sash_windows.contains_key(&vid)
            || self.aura_windows.contains_key(&vid)
            || self.changelook_windows.contains(&vid)
        {
            self.tell(vid, actor, CHAT_TYPE_INFO, b"[LS;841]", &[]);
            return Ok(moved);
        }
        if self.quest_running(vid) || tables.is_empty() {
            return Ok(moved);
        }
        let total: u64 = tables.iter().map(|table| table.price).sum();
        if total.saturating_add(self.gold_of(vid)) >= MYSHOP_GOLD_CAP {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Nu poti avea mai mult yang de atat.",
                &[],
            );
            return Ok(moved);
        }
        let mut shown = [0u8; GC_SHOP_SIGN_FIELD_SIZE];
        shown.copy_from_slice(&sign);
        let end = shown
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(shown.len());
        if end == 0 {
            return Ok(moved);
        }
        if self.name_rules.holds_word(&shown[..end]) {
            self.tell(vid, actor, CHAT_TYPE_INFO, b"[LS;812]", &[]);
            return Ok(moved);
        }
        let Some(slots) = self.myshop_slots(vid, tables, actor) else {
            return Ok(moved);
        };
        if !self.myshop_bundle(vid, actor, &mut moved) {
            return Ok(moved);
        }
        self.myshop_stand(vid, shown, &slots);
        Ok(moved)
    }

    /// The window stands, the sign goes around, the polymorph affect goes, a
    /// summoned horse is dismissed (or else a mount is unlaid), and the seller
    /// shows as the bundle mob.
    fn myshop_stand(
        &mut self,
        vid: Vid,
        shown: [u8; GC_SHOP_SIGN_FIELD_SIZE],
        slots: &[Option<MyShopSlot>; MYSHOP_WINDOW_SLOTS],
    ) {
        self.my_shops.insert(
            vid,
            MyShop {
                sign: shown,
                slots: *slots,
            },
        );
        let mut record = Vec::new();
        GcShopSign::new(vid.raw(), shown).encode_into(&mut record);
        self.packet_around(EntityKey::Character(vid.raw()), &record, None);
        if self
            .characters
            .find_by_vid(vid)
            .is_ok_and(|character| character.points().is_some_and(Points::is_polymorphed))
        {
            let _gone = self.remove_pc_affect(vid, crate::loading_phase::AFFECT_POLYMORPH);
        }
        if self.horse_is_summoned(vid) {
            self.horse_unsummon(vid, true);
        } else if self.characters.find_by_vid(vid).is_ok_and(|character| {
            character
                .points()
                .is_some_and(|points| points.mount_vnum() != 0)
        }) {
            self.unmount(vid);
        }
        self.set_polymorph(vid, MYSHOP_POLYMORPH_VNUM, true);
    }

    /// The window slots the tables stand on, or `None` after telling the refusal:
    /// a repeated cell is silent, an ungiveable one answers `[LS;813]`, a worn one
    /// `[LS;1060]`, and a window-locked one `[LS;1133]`. A table past the window,
    /// on a taken slot, or naming no held item is skipped, as `SetShopItems`
    /// skips it.
    fn myshop_slots(
        &mut self,
        vid: Vid,
        tables: &[CgMyShopTable],
        actor: Mover,
    ) -> Option<[Option<MyShopSlot>; MYSHOP_WINDOW_SLOTS]> {
        let mut seen = BTreeSet::new();
        let mut slots: [Option<MyShopSlot>; MYSHOP_WINDOW_SLOTS] = [None; MYSHOP_WINDOW_SLOTS];
        // `bItemCount > SHOP_HOST_ITEM_MAX_NUM` stands an empty window (`:92-93`).
        if tables.len() > MYSHOP_WINDOW_SLOTS {
            return Some(slots);
        }
        let mut placed: Vec<(usize, i32)> = Vec::with_capacity(tables.len());
        for table in tables {
            let key = (table.pos.window_type, table.pos.cell);
            if !seen.insert(key) {
                return None;
            }
            let Ok(character) = self.characters.find_by_vid(vid) else {
                return None;
            };
            let Some(item) = character.items().item_at(table.pos) else {
                continue;
            };
            let Some(proto) = self.protos.get(item.vnum) else {
                continue;
            };
            if proto.anti_flags & (ITEM_ANTIFLAG_GIVE | ITEM_ANTIFLAG_MYSHOP) != 0 {
                self.tell(vid, actor, CHAT_TYPE_INFO, b"[LS;813]", &[]);
                return None;
            }
            if table.pos.window_type == EWindows::Equipment as u8 {
                self.tell(vid, actor, CHAT_TYPE_INFO, b"[LS;1060]", &[]);
                return None;
            }
            if self.sash_item_at(vid, table.pos)
                || self.changelook_item_at(vid, table.pos)
                || self.aura_item_at(vid, table.pos)
            {
                self.tell(vid, actor, CHAT_TYPE_INFO, b"[LS;1133]", &[]);
                return None;
            }
            let at = usize::from(table.display_pos);
            if at >= MYSHOP_WINDOW_SLOTS || slots[at].is_some() {
                continue;
            }
            // A display slot past the window vector is a heap write past
            // `m_itemVector` in legacy (the grid is 45 cells, the vector 40), a
            // Defect; the bound above refuses it. A size the grid cannot hold at
            // the slot is skipped, as `SetShopItems` skips it (`:139-153`).
            if !Self::myshop_cell_free(&placed, at, proto.size) {
                continue;
            }
            placed.push((at, proto.size));
            slots[at] = Some(MyShopSlot {
                item: item.id,
                price: table.price,
            });
        }
        Some(slots)
    }

    /// Whether a table of `size` stands on display slot `at` over `placed`, as
    /// `CGrid::IsEmpty(at, 1, size)` answers it (`libgame/grid.cpp`): the column
    /// stays inside the 5-by-9 grid and every cell is free.
    fn myshop_cell_free(placed: &[(usize, i32)], at: usize, size: i32) -> bool {
        if size < 1 || at >= MYSHOP_WINDOW_SLOTS {
            return false;
        }
        // The row the anchor stands on plus the size stays inside the grid.
        if at / MYSHOP_GRID_WIDTH + usize::try_from(size).unwrap_or(usize::MAX) > MYSHOP_GRID_HEIGHT
        {
            return false;
        }
        let cells = |anchor: usize, tall: i32| {
            (0..tall).map(move |row| anchor + usize::try_from(row).unwrap_or(0) * MYSHOP_GRID_WIDTH)
        };
        for cell in cells(at, size) {
            for &(other, tall) in placed {
                if cells(other, tall).any(|taken| taken == cell) {
                    return false;
                }
            }
        }
        true
    }

    /// The bundle the build spends: with a silk bundle nothing, else one paper
    /// bundle, else silent. Answers whether the build runs.
    fn myshop_bundle(&mut self, vid: Vid, actor: Mover, moved: &mut MovedItems) -> bool {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return false;
        };
        let usable = common::item_slots::usable_inventory_cells(character.inven_point());
        if count_specify_item(character.items(), usable, MYSHOP_SILK_BUNDLE) > 0 {
            return true;
        }
        if count_specify_item(character.items(), usable, MYSHOP_PAPER_BUNDLE) == 0 {
            return false;
        }
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return false;
        };
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let spent =
            remove_specify_item(items, slots, &self.protos, usable, (MYSHOP_PAPER_BUNDLE, 1));
        let pid = character.player_id();
        let packed = MovedItems::new(pid, vid.raw(), spent, actor, &self.protos, &self.locale);
        for record in packed.records {
            let _sent = self.write_to_client(vid, record);
        }
        moved.changes.extend(packed.changes);
        moved.quickslots = packed.quickslots;
        true
    }

    /// `do_close_shop` (`G/cmd_general.cpp:2004-2012`) for the caller under `vid`:
    /// an observer stays silent, and so does a caller with no own shop; else the
    /// window drops, the sign clears around, and the seller shows as its job again.
    pub(super) fn close_shop_line(&mut self, vid: Vid) {
        if self.is_observing(vid) {
            return;
        }
        self.close_my_shop(vid);
    }

    /// `CHARACTER::CloseMyShop` for the seller under `vid`: the window drops, the
    /// sign clears around, and the seller shows as its job again.
    pub(super) fn close_my_shop(&mut self, vid: Vid) {
        let Some(_shop) = self.my_shops.remove(&vid) else {
            return;
        };
        let mut record = Vec::new();
        GcShopSign::new(vid.raw(), [0u8; GC_SHOP_SIGN_FIELD_SIZE]).encode_into(&mut record);
        self.packet_around(EntityKey::Character(vid.raw()), &record, None);
        let job = self.characters.find_by_vid(vid).map_or(0, |character| {
            character
                .points()
                .map_or(0, |points| race_to_job(points.race()))
        });
        self.set_polymorph(vid, u32::from(job), true);
    }

    /// Whether the character under `vid` has a summoned horse standing.
    fn horse_is_summoned(&self, vid: Vid) -> bool {
        self.characters.find_by_vid(vid).is_ok_and(|character| {
            character
                .points()
                .is_some_and(|points| points.horse().summoned())
        })
    }

    /// The yang the character under `vid` holds.
    fn gold_of(&self, vid: Vid) -> u64 {
        self.characters.find_by_vid(vid).map_or(0, Character::gold)
    }
}

#[cfg(test)]
mod tests {
    use common::item_slots::EWindows;
    use common::vid::Vid;
    use protocol::item_pos::ItemPos;
    use world::item::Item;

    use super::super::GameState;
    use super::{MYSHOP_PAPER_BUNDLE, MYSHOP_POLYMORPH_VNUM, MYSHOP_SILK_BUNDLE};
    use crate::game_state::fixtures::{a_world, enter_seeing_with, TestClock, PLACE};
    use crate::item_move::Mover;

    const SELLER: Vid = Vid::new(11);
    const PID: u32 = 11;

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

    fn inventory(cell: u16) -> ItemPos {
        ItemPos::new(EWindows::Inventory as u8, cell)
    }

    fn held(id: u32, vnum: u32, count: u16) -> Item {
        let mut item = Item::new(id, vnum);
        item.count = count;
        item.set_size(1).unwrap();
        item
    }

    /// A world where player 11 holds a sword (vnum 19) in cell 0 and `bundle`
    /// of `bundle_vnum` in cell 1.
    fn a_seller(
        bundle_vnum: u32,
        bundle: u16,
    ) -> (GameState, tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let sword = held(5, 19, 1);
        let paper = held(6, bundle_vnum, bundle);
        let (inbox, _) = enter_seeing_with(
            &mut world,
            PLACE,
            11,
            (5, 5),
            10_000,
            &[(inventory(0), sword), (inventory(1), paper)],
        );
        (world, inbox)
    }

    fn sign_of(text: &[u8]) -> [u8; 33] {
        let mut sign = [0u8; 33];
        let end = text.len().min(32);
        sign[..end].copy_from_slice(&text[..end]);
        sign
    }

    fn table(cell: u16, price: u64, display: u8) -> protocol::cg_myshop::CgMyShopTable {
        protocol::cg_myshop::CgMyShopTable {
            vnum: 19,
            count: 1,
            pos: inventory(cell),
            price,
            display_pos: display,
            tail: [0u8; protocol::cg_myshop::CG_MYSHOP_TABLE_TAIL_SIZE],
        }
    }

    fn drained(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Ok(record) = inbox.try_recv() {
            out.push(record);
        }
        out
    }

    /// The build stands the window, spends the paper bundle, and in legacy order
    /// (spend, sign, polymorph) tells the seller, signs around, and morphs the
    /// seller into the bundle mob.
    #[test]
    fn an_open_stands_the_window_spends_the_bundle_and_signs() {
        let (mut state, mut inbox) = a_seller(MYSHOP_PAPER_BUNDLE, 1);
        drained(&mut inbox);
        let moved = state
            .myshop(SELLER, sign_of(b"Swords"), &[table(0, 500, 3)], mover())
            .unwrap();
        assert!(state.has_my_shop(SELLER));
        let shop = &state.my_shops[&SELLER];
        assert_eq!(&shop.sign[..7], b"Swords\x00");
        assert_eq!(
            shop.slots[3],
            Some(super::MyShopSlot {
                item: 5,
                price: 500
            })
        );
        {
            let character = state.characters.find_by_vid(SELLER).unwrap();
            assert!(character.items().item(6).is_none(), "the bundle is spent");
            assert_eq!(moved.changes.len(), 1, "the spend's destroy stores");
            let points = character.points().unwrap();
            assert_eq!(points.polymorph_race(), MYSHOP_POLYMORPH_VNUM);
        }
        let heard = drained(&mut inbox);
        assert!(!heard.is_empty());
        assert_eq!(
            heard[0][0],
            protocol::gc_inventory::HEADER_GC_ITEM_SET2.value(),
            "the spend first: {heard:02x?}"
        );
        assert_eq!(
            heard[1][0],
            protocol::gc_fields::HEADER_GC_SHOP_SIGN,
            "then the sign: {heard:02x?}"
        );
        assert_eq!(&heard[1][5..11], b"Swords");
        assert!(
            heard.len() > 2,
            "the polymorph's view follows: {heard:02x?}"
        );
        let _ = PID;
    }

    /// A re-open with a shop standing refuses: `CanHandleItem` sees the open
    /// shop before the close-and-return, so the toggle is dead.
    #[test]
    fn a_reopen_refuses_and_keeps_the_old_window() {
        let (mut state, mut inbox) = a_seller(MYSHOP_PAPER_BUNDLE, 2);
        drained(&mut inbox);
        state
            .myshop(SELLER, sign_of(b"Swords"), &[table(0, 500, 3)], mover())
            .unwrap();
        drained(&mut inbox);
        let moved = state
            .myshop(SELLER, sign_of(b"Axes"), &[table(0, 700, 4)], mover())
            .unwrap();
        assert!(moved.records.is_empty());
        assert!(moved.around.is_empty());
        assert_eq!(&state.my_shops[&SELLER].sign[..7], b"Swords\x00");
        assert!(state.my_shops[&SELLER].slots[4].is_none());
        let told = drained(&mut inbox);
        assert_eq!(told.len(), 1, "the [LS;841] refusal chats");
        assert_eq!(told[0][0], protocol::gc_inventory::HEADER_GC_CHAT.value());
    }

    /// The silk bundle opens without spending, and without one paper bundle the
    /// build is silent.
    #[test]
    fn silk_keeps_and_no_bundle_is_silent() {
        let (mut state, mut inbox) = a_seller(MYSHOP_SILK_BUNDLE, 1);
        drained(&mut inbox);
        state
            .myshop(SELLER, sign_of(b"Silk"), &[table(0, 500, 0)], mover())
            .unwrap();
        assert!(state.has_my_shop(SELLER));
        let character = state.characters.find_by_vid(SELLER).unwrap();
        assert!(character.items().item(6).is_some(), "silk is kept");

        let (mut state, mut inbox) = a_seller(MYSHOP_PAPER_BUNDLE, 0);
        // A zero-count row is no bundle: hold a potion instead.
        {
            let character = state.characters.find_by_vid_mut(SELLER).unwrap();
            let (items, _, _) = character.items_points_and_quickslots_mut();
            let _ = items.release(6);
            let mut potion = held(7, 27003, 1);
            potion.count = 1;
            items.set(inventory(1), &potion).unwrap();
        }
        drained(&mut inbox);
        let moved = state
            .myshop(SELLER, sign_of(b"Poor"), &[table(0, 500, 0)], mover())
            .unwrap();
        assert!(!state.has_my_shop(SELLER));
        assert!(moved.records.is_empty() && moved.around.is_empty());
        assert!(drained(&mut inbox).is_empty(), "no bundle is silent");
    }

    /// A display slot past the window is refused, where legacy writes past
    /// `m_itemVector` (a Defect, STATUS): the grid is 45 cells, the vector 40.
    #[test]
    fn a_display_slot_past_the_window_is_refused() {
        let (mut state, mut inbox) = a_seller(MYSHOP_PAPER_BUNDLE, 1);
        drained(&mut inbox);
        let moved = state
            .myshop(SELLER, sign_of(b"Past"), &[table(0, 500, 40)], mover())
            .unwrap();
        assert!(state.has_my_shop(SELLER), "the window still stands");
        assert!(
            state.my_shops[&SELLER].slots.iter().all(Option::is_none),
            "but the table is skipped"
        );
        assert!(moved.records.is_empty() && moved.around.is_empty());
        assert!(drained(&mut inbox)
            .iter()
            .any(|record| record[0] == protocol::gc_fields::HEADER_GC_SHOP_SIGN));
    }

    /// A repeated cell stands no window and tells nothing.
    #[test]
    fn a_repeated_cell_is_silent() {
        let (mut state, mut inbox) = a_seller(MYSHOP_PAPER_BUNDLE, 1);
        drained(&mut inbox);
        let moved = state
            .myshop(
                SELLER,
                sign_of(b"Dup"),
                &[table(0, 500, 0), table(0, 600, 1)],
                mover(),
            )
            .unwrap();
        assert!(!state.has_my_shop(SELLER));
        assert!(moved.records.is_empty() && moved.around.is_empty());
        assert!(drained(&mut inbox).is_empty());
    }

    /// The close drops the window, clears the sign around, and shows the job again.
    #[test]
    fn a_close_drops_the_window_and_shows_the_job() {
        let (mut state, mut inbox) = a_seller(MYSHOP_PAPER_BUNDLE, 1);
        drained(&mut inbox);
        state
            .myshop(SELLER, sign_of(b"Swords"), &[table(0, 500, 3)], mover())
            .unwrap();
        drained(&mut inbox);
        state.close_shop_line(SELLER);
        assert!(!state.has_my_shop(SELLER));
        let character = state.characters.find_by_vid(SELLER).unwrap();
        let points = character.points().unwrap();
        assert!(!points.is_polymorphed());
        let cleared = drained(&mut inbox);
        assert!(
            cleared.iter().any(
                |record| record[0] == protocol::gc_fields::HEADER_GC_SHOP_SIGN && record[5] == 0
            ),
            "the cleared sign reaches the seller: {cleared:02x?}"
        );
    }
}
