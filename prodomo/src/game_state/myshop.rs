//! The bundle shop: `CG_MYSHOP` opens the seller's own shop window, and
//! `/close_shop` closes it (`CInputMain::MyShop`, `G/input_main.cpp:3255-3314`,
//! `CHARACTER::OpenMyShop` and `CloseMyShop`, `G/char.cpp:805-975` and `:978-1007`).
//!
//! The world keeps each seller's sign and window slots, as legacy's `CShopManager`
//! keeps `m_map_pkShopByPC`. A slot names the held item's id and the asking price;
//! the item itself stays in the seller's inventory, as `SHOP_ITEM::pkItem` points
//! at the live item.
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
//! A click on a seller opens its window (`CHARACTER::OnClick`, `G/char.cpp:6219-6293`),
//! and a buy moves the item by id, charges the full price, pays the seller net of
//! the `personal_shop` tax, and updates every guest (`CShop::Buy`, `G/shop.cpp:610-808`).
//! Both sides settle in one transaction, as a trade's do.
//!
//! The silk bundle's price list is a DB-server packet (`HEADER_GD_MYSHOP_PRICELIST_UPDATE`)
//! with no client record; the market-price slice reads the shops themselves.
//!
//! # Divergences
//!
//! - **The seller at the yang cap.** `ChangeGold` refuses gold that would reach the
//!   cap after the buyer's has been taken, losing the tax difference; the Rewrite
//!   refuses the buy first, as the sale does.
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

/// What a click on a seller's window found.
pub(super) enum MyShopClick {
    /// No shop stands, or a quest marked the seller: silent.
    Silent,
    /// A trade-window gate failed: `[LS;841]`.
    Refused,
    /// The window opens: the slots boxed, as the window dwarfs the refusals.
    Open {
        /// The window slots.
        slots: Box<[Option<MyShopSlot>; MYSHOP_WINDOW_SLOTS]>,
    },
}

/// The settled figures of a bundle-shop buy: the item, the buyer, and the gold.
/// A buy that passed its refusal checks: the item, the figures, and the sides.
struct MyshopBuyReady {
    /// The sold item, as the seller holds it.
    item: world::item::Item,
    /// The asking price.
    price: u64,
    /// The `personal_shop` tax rate, clamped to 100.
    rate: i32,
    /// The tax taken from the price.
    tax: u64,
    /// The move's figures.
    deal: MyshopBuyDeal,
    /// The seller's store id.
    seller_pid: u32,
    /// The seller's gold after the gain.
    seller_gold: u64,
    /// The buyer's gold after the price.
    left: u64,
}

#[derive(Debug, Clone, Copy)]
struct MyshopBuyDeal {
    /// The sold item's id.
    item_id: world::item::ItemId,
    /// The buyer's store id.
    buyer_pid: u32,
    /// The buyer's cell.
    at: protocol::item_pos::ItemPos,
    /// The buyer's gold after the price.
    left: u64,
    /// The seller's gain after the tax.
    net: u64,
    /// The seller's gold after the gain.
    seller_gold: u64,
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
                || self.shaman_locked_at(vid, table.pos)
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

    /// `CHARACTER::CloseMyShop` for the seller under `vid`: the window drops, its
    /// guests are kicked, the sign clears around, and the seller shows as its job
    /// again.
    pub(super) fn close_my_shop(&mut self, vid: Vid) {
        let Some(_shop) = self.my_shops.remove(&vid) else {
            return;
        };
        self.kick_shop_guests(vid);
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

    /// The trade-window gates of a click on the seller under `target` by the
    /// character under `vid` (`G/char.cpp:6236-6260`).
    pub(super) fn myshop_click_gates(
        &self,
        vid: Vid,
        target: u32,
        clicker_pid: u32,
    ) -> MyShopClick {
        let seller = Vid::new(target);
        let Some(shop) = self.my_shops.get(&seller) else {
            return MyShopClick::Silent;
        };
        if self.quest_targets.at(clicker_pid, target).is_some() {
            return MyShopClick::Silent;
        }
        if vid.raw() == target {
            // The seller clicks its own window (`:6236-6242`).
            if self.trade_partner(vid).is_some()
                || self.safebox_open(vid)
                || self.browsing.contains_key(&vid)
                || self.cubes.contains_key(&vid)
                || self.aura_windows.contains_key(&vid)
            {
                return MyShopClick::Refused;
            }
        } else {
            // Another player clicks it (`:6243-6260`).
            if self.trade_partner(vid).is_some()
                || self.safebox_open(vid)
                || self.has_my_shop(vid)
                || self.browsing.contains_key(&vid)
                || self.cubes.contains_key(&vid)
                || self.aura_windows.contains_key(&vid)
            {
                return MyShopClick::Refused;
            }
            if self.trade_partner(seller).is_some()
                || self.safebox_open(seller)
                || self.cubes.contains_key(&seller)
                || self.aura_windows.contains_key(&seller)
            {
                return MyShopClick::Refused;
            }
        }
        MyShopClick::Open {
            slots: Box::new(shop.slots),
        }
    }

    /// The guests of the seller under `vid`, in VID order, as `~CShop` kicks
    /// them (`G/shop.cpp:37-57`).
    fn kick_shop_guests(&mut self, vid: Vid) {
        let raw = vid.raw();
        let mut guests: Vec<Vid> = self
            .browsing
            .iter()
            .filter(|(_, browsing)| {
                browsing.keeper == raw && browsing.kind == super::shop::ShopKeeper::Player
            })
            .map(|(guest, _)| *guest)
            .collect();
        guests.sort_by_key(|guest| guest.raw());
        for guest in guests {
            self.end_browsing(guest);
        }
    }

    /// The START window for the seller's shop: each standing slot rendered from
    /// the live item, as `CShop::AddGuest` renders it (`G/shop.cpp:836-900`).
    pub(super) fn myshop_window(
        &self,
        keeper: u32,
        slots: &[Option<MyShopSlot>; MYSHOP_WINDOW_SLOTS],
        seller: Vid,
    ) -> Vec<u8> {
        let held = self.characters.find_by_vid(seller).ok();
        let mut items = [protocol::gc_shop::GcShopItem::default(); MYSHOP_WINDOW_SLOTS];
        for (shown, slot) in items.iter_mut().zip(slots.iter()) {
            let Some(slot) = slot else { continue };
            let Some(item) = held.and_then(|held| held.items().item(slot.item)) else {
                continue;
            };
            let mut rendered =
                protocol::gc_shop::GcShopItem::new(item.vnum, slot.price, item.count);
            rendered.refine_element = item.refine_element;
            rendered.sockets = item.sockets;
            rendered.attrs = item.attributes;
            rendered.transmutation = item.transmutation;
            *shown = rendered;
        }
        protocol::gc_shop::GcShopStart::new(keeper, items).encode()
    }

    /// `CShop::Buy` for a player's shop (`G/shop.cpp:610-808`, the live arm:
    /// `OFFLINE_SHOP` is undefined, and the `:200-438` overload never compiles).
    /// The buyer pays the full price, the item moves to its first free cell by
    /// id, the seller gains the price net of the `personal_shop` tax, the sold
    /// slot keeps its price and id (as legacy keeps them when it NULLs `pkItem`),
    /// and every guest hears the update. Both sides settle in one transaction,
    /// as a trade's do.
    pub(super) fn buy_from_player(
        &mut self,
        vid: Vid,
        keeper: u32,
        slot: usize,
        actor: Mover,
    ) -> super::shop::ShopAnswer {
        use protocol::gc_shop::{GcShop, GcShopItem, GcShopUpdateItem, SHOP_SUBHEADER_GC_SOLD_OUT};
        let seller = Vid::new(keeper);
        let failed = |subheader| super::shop::ShopAnswer::Declined {
            reason: super::shop::ShopDeclined::Refused(world::character::ShopRefused::SoldOut),
            records: vec![GcShop::new(subheader).encode()],
        };
        let ready = match self.myshop_buy_ready(vid, seller, slot) {
            Ok(ready) => ready,
            Err(subheader) => return failed(subheader),
        };
        let MyshopBuyReady {
            item,
            price,
            rate,
            tax,
            deal,
            seller_pid,
            seller_gold,
            left,
        } = ready;
        let buyer_pid = deal.buyer_pid;
        if vid == seller {
            // The seller buys its own listing: legacy walks the same path with one
            // character on both sides, and the store refuses two sides of one owner.
            return failed(SHOP_SUBHEADER_GC_SOLD_OUT);
        }
        // The move: the seller's cell clears and the buyer's fills, by id.
        let Some((seller_done, buyer_done)) = self.myshop_buy_move(vid, seller, &item, deal) else {
            return failed(SHOP_SUBHEADER_GC_SOLD_OUT);
        };
        if rate > 0 {
            // `[LS;881;%d]` with the rate (`:769-770`): the text is formatted, so it
            // rides its own line before the settled records, not a static notice.
            let seller_mover = self.own_mover(seller).unwrap_or(actor);
            self.tell(
                seller,
                seller_mover,
                protocol::gc_chat::CHAT_TYPE_INFO,
                b"[LS;881;%d]",
                &[crate::chat_line::Arg::Int(i64::from(rate))],
            );
        }
        // The slot keeps its price and item id, as legacy keeps `price` and
        // `itemid` when it NULLs `pkItem` (`:776`): a rebuy finds the item gone
        // from the seller and answers sold out. The update reaches every guest
        // after the settled records, as `BroadcastUpdateItem` runs after the
        // move's item records (`:776-778`), and carries vnum 0 with the kept
        // price and count (`:947-982`). The buyer's and the seller's copies
        // ride their settled sides, so the store write orders them; the other
        // guests hear it at once.
        let update = GcShopUpdateItem::new(
            u8::try_from(slot).unwrap_or(u8::MAX),
            GcShopItem::new(0, price, item.count),
        )
        .encode();
        self.send_shop_update_to_others(keeper, vid.raw(), &update);
        // The seller's empire takes the tax (`:772`).
        let monarch = if tax > 0 {
            self.credit_monarch_tax(seller, tax)
        } else {
            None
        };
        let seller_mover = self.own_mover(seller).unwrap_or(actor);
        let mut seller_side =
            self.settle_shop_side(seller, seller_pid, seller_done, seller_gold, seller_mover);
        seller_side.records.push(update.clone());
        let mut buyer_side = self.settle_shop_side(vid, buyer_pid, buyer_done, left, actor);
        buyer_side.records.push(update);
        super::shop::ShopAnswer::Settled(Box::new(super::trade::TradeSettled {
            own: buyer_side,
            partner: seller_side,
            partner_outbox: self.outboxes.get(&seller).cloned(),
            monarch,
        }))
    }

    /// The buy's refusal checks (`G/shop.cpp:610-702`): the slot, the held item,
    /// the buyer's gold, the buyer's cell, the tax rate, and the seller's landing.
    /// Answers the ready deal, or the refusal subheader. The guest check is
    /// `trading`'s: a buyer with no window never reaches here. The dead check is
    /// the NPC arm's (`:200-202`); the PC arm has none, because the window closes
    /// at death.
    fn myshop_buy_ready(&self, vid: Vid, seller: Vid, slot: usize) -> Result<MyshopBuyReady, u8> {
        use protocol::gc_shop::{
            SHOP_SUBHEADER_GC_INVENTORY_FULL, SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY,
            SHOP_SUBHEADER_GC_SOLD_OUT,
        };
        let Some(shop) = self.my_shops.get(&seller) else {
            return Err(SHOP_SUBHEADER_GC_SOLD_OUT);
        };
        let Some(listed) = shop.slots[slot] else {
            return Err(SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY);
        };
        if listed.price == 0 {
            return Err(SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY);
        }
        // The item must still be the seller's, or the slot is sold out (`:637-655`),
        // before the buyer's gold is counted (`:657-662`).
        let price = listed.price;
        let item_id = listed.item;
        let held = self
            .characters
            .find_by_vid(seller)
            .ok()
            .and_then(|seller| seller.items().item(item_id).cloned());
        let Some(item) = held else {
            return Err(SHOP_SUBHEADER_GC_SOLD_OUT);
        };
        let buyer_gold = self.gold_of(vid);
        if buyer_gold < price {
            return Err(SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY);
        }
        // A dragon soul stone goes to the stones' inventory, anything else to the
        // first free base cell (`:686-700`).
        let Some(at) = self.myshop_buy_cell(vid, &item) else {
            return Err(SHOP_SUBHEADER_GC_INVENTORY_FULL);
        };
        // The tax is taken after the buyer pays, from the full price: the buyer is
        // charged `price`, the seller gains `price - tax` (`:704-723`).
        let left = buyer_gold - price;
        let rate = self
            .quests
            .as_ref()
            .map_or(0, |quests| quests.manager().event_flag(b"personal_shop"))
            .clamp(0, 100);
        let tax = price * u64::try_from(rate).unwrap_or(0) / 100;
        let net = price - tax;
        let seller_pid = self
            .characters
            .find_by_vid(seller)
            .map_or(0, Character::player_id);
        let buyer_pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, Character::player_id);
        // `ChangeGold` refuses gold that would reach the cap (`G/char.cpp:3827-3832`)
        // after the buyer's has been taken, losing the tax difference; the Rewrite
        // refuses first, as the sale does.
        let Some(seller_gold) = self
            .gold_of(seller)
            .checked_add(net)
            .filter(|held| *held < world::character::GOLD_MAX_MAX)
        else {
            return Err(SHOP_SUBHEADER_GC_SOLD_OUT);
        };
        Ok(MyshopBuyReady {
            item,
            price,
            rate,
            tax,
            deal: MyshopBuyDeal {
                item_id,
                buyer_pid,
                at,
                left,
                net,
                seller_gold,
            },
            seller_pid,
            seller_gold,
            left,
        })
    }

    /// The buy's move: the seller's cell clears and the buyer's fills, by id,
    /// and both sides' records pack. `None` when a storage step fails, which
    /// the caller answers as sold out.
    fn myshop_buy_move(
        &mut self,
        vid: Vid,
        seller: Vid,
        item: &world::item::Item,
        deal: MyshopBuyDeal,
    ) -> Option<(world::character::MoveDone, world::character::MoveDone)> {
        use world::character::{ItemRecord, MoveDone, MoveKind, MoveRecord, QuickslotSync, SyncTo};
        use world::item::gc_item_clear;
        let MyshopBuyDeal {
            item_id,
            buyer_pid,
            at,
            left,
            net,
            seller_gold,
        } = deal;
        let seller_pos = item.pos;
        let mut moved_item = item.clone();
        moved_item.pos = at;
        self.characters
            .find_by_vid_mut(seller)
            .ok()?
            .items_mut()
            .release(item_id)
            .ok()?;
        self.characters
            .find_by_vid_mut(vid)
            .ok()?
            .items_mut()
            .set(at, &moved_item)
            .ok()?;
        // The seller's quickslots forget the cell, as `SyncQuickslot` does (`:729`).
        let seller_done = MoveDone {
            kind: MoveKind::Sold,
            records: vec![
                MoveRecord::QuickslotSync(QuickslotSync {
                    from: seller_pos.cell,
                    to: SyncTo::Delete,
                }),
                MoveRecord::Item(ItemRecord::Set(gc_item_clear(seller_pos))),
                MoveRecord::Gold {
                    amount: i64::try_from(net).unwrap_or(i64::MAX),
                    value: seller_gold,
                },
            ],
            changes: vec![world::character::ItemChange::Given {
                id: item_id,
                to: buyer_pid,
                pos: at,
            }],
        };
        let buyer_done = MoveDone {
            kind: MoveKind::Bought,
            records: vec![
                MoveRecord::Gold {
                    amount: 0,
                    value: left,
                },
                MoveRecord::Item(ItemRecord::Set(moved_item.gc_item_set(at, 1))),
            ],
            changes: vec![],
        };
        Some((seller_done, buyer_done))
    }

    /// The buyer's cell for the shop item: a dragon soul stone goes to its box,
    fn myshop_buy_cell(
        &self,
        vid: Vid,
        item: &world::item::Item,
    ) -> Option<protocol::item_pos::ItemPos> {
        let buyer = self.characters.find_by_vid(vid).ok()?;
        let proto = self.protos.get(item.vnum);
        if proto.is_some_and(|proto| proto.item_type == gamedata::item_kind::ITEM_DS) {
            let qualified = self.ds_qualified(vid);
            let graded = proto.map_or(0, |proto| proto.sub_type);
            return world::character::dragon_soul_buy_cell(buyer.items(), item, graded, qualified);
        }
        let usable = common::item_slots::usable_inventory_cells(buyer.inven_point());
        buyer
            .items()
            .find_free_inventory_cell(usable, item.size)
            .map(|cell| protocol::item_pos::ItemPos::new(EWindows::Inventory as u8, cell))
    }

    /// One settled shop side: the gold lands, the slots sync, and the move packs.
    fn settle_shop_side(
        &mut self,
        vid: Vid,
        pid: u32,
        mut done: world::character::MoveDone,
        gold: u64,
        actor: Mover,
    ) -> crate::item_move::MovedItems {
        use crate::item_move::gold_delta;
        use world::character::sync_quickslots;
        let mut step = crate::item_move::MovedItems::new(
            pid,
            vid.raw(),
            world::character::MoveDone {
                kind: done.kind,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        );
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            let change = gold_delta(character.gold(), gold);
            character.set_gold(gold);
            let (items, slots) = character.items_and_quickslots_mut();
            sync_quickslots(&mut done, slots, items);
            let quickslots = slots.clone();
            step = crate::item_move::MovedItems::new(
                pid,
                vid.raw(),
                done,
                actor,
                &self.protos,
                &self.locale,
            );
            step.quickslots = Some(quickslots);
            step.gold = Some(change);
        }
        step
    }

    /// The update to every guest of the seller under `keeper` except the buyer and
    /// the seller, as `CShop::BroadcastUpdateItem` fans it (`G/shop.cpp:943-988`).
    /// Those two hear their copies after their settled records, which ride their
    /// settled sides.
    fn send_shop_update_to_others(&self, keeper: u32, buyer: u32, update: &[u8]) {
        let seller = keeper;
        let mut guests: Vec<Vid> = self
            .browsing
            .iter()
            .filter(|(guest, browsing)| {
                browsing.keeper == keeper
                    && browsing.kind == super::shop::ShopKeeper::Player
                    && guest.raw() != buyer
                    && guest.raw() != seller
            })
            .map(|(guest, _)| *guest)
            .collect();
        guests.sort_by_key(|guest| guest.raw());
        for guest in guests {
            let _sent = self.write_to_client(guest, update.to_vec());
        }
    }

    /// The monarch tax of a PC-shop buy: the seller's empire's treasury takes
    /// `tax`, capped at two billion (`G/monarch.cpp:118-132`). Answers the slot
    /// and the landed cell for the store, as `run_temp` does.
    fn credit_monarch_tax(&mut self, seller: Vid, tax: u64) -> Option<(u8, i64)> {
        let empire = self.bodies.get(&seller)?.card.empire;
        let at = usize::from(empire);
        let cell = *self.monarch_money.get(at)?;
        let landed = cell.saturating_add(i64::try_from(tax).unwrap_or(i64::MAX));
        if !(0..=2_000_000_000).contains(&landed) {
            return None;
        }
        self.monarch_money[at] = landed;
        Some((empire, landed))
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
    use crate::game_loop_messages::GroundPlace;
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
            killer_mode: false,
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

    /// A buyer and a seller on one world: the seller holds the sword and the
    /// bundle, the buyer holds the gold.
    fn a_market(
        buyer_gold: u64,
    ) -> (
        GameState,
        tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    ) {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let sword = held(5, 19, 1);
        let paper = held(6, MYSHOP_PAPER_BUNDLE, 1);
        let (seller_inbox, _) = enter_seeing_with(
            &mut world,
            PLACE,
            11,
            (5, 5),
            10_000,
            &[(inventory(0), sword), (inventory(1), paper)],
        );
        let (buyer_inbox, _) = enter_seeing_with(&mut world, PLACE, 12, (5, 6), 10_000, &[]);
        world
            .characters
            .find_by_vid_mut(Vid::new(12))
            .unwrap()
            .set_gold(buyer_gold);
        (world, seller_inbox, buyer_inbox)
    }

    fn with_quests(world: &mut GameState) {
        use gamedata::mob_locale_names::MobNamesByLanguage;
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager =
            quest::manager::Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        world.quests = Some(crate::game_state::quests::Quests::new(
            manager,
            std::sync::Arc::default(),
            MobNamesByLanguage::default(),
        ));
    }

    /// A buy moves the item by id, charges the full price, pays the seller
    /// whole without the flag, keeps the sold slot's price, and trails the
    /// update on both settled sides.
    #[test]
    fn a_buy_moves_the_item_and_charges_the_full_price() {
        use super::super::shop::{ShopAnswer, ShopStep};
        const BUYER: Vid = Vid::new(12);
        let (mut state, mut seller_inbox, mut buyer_inbox) = a_market(1_000);
        state
            .myshop(SELLER, sign_of(b"Swords"), &[table(0, 500, 3)], mover())
            .unwrap();
        let place = GroundPlace {
            channel: 1,
            map: 41,
            x: 5,
            y: 6,
        };
        let account = crate::game_loop_messages::QuestAccount::default();
        let opened = state.shop(
            BUYER,
            ShopStep::Click { target: 11 },
            place,
            mover(),
            &account,
        );
        assert!(matches!(opened, Ok(ShopAnswer::Sent(_))));
        drained(&mut seller_inbox);
        drained(&mut buyer_inbox);
        let bought = state
            .shop(BUYER, ShopStep::Buy { pos: 3 }, place, mover(), &account)
            .unwrap();
        let ShopAnswer::Settled(settled) = bought else {
            panic!("the buy settles: {bought:?}");
        };
        assert!(
            state.my_shops[&SELLER].slots[3].is_some(),
            "the sold slot keeps its price, as legacy keeps it"
        );
        assert_eq!(settled.monarch, None, "no flag is no tax");
        assert_eq!(
            (settled.own.owner_id, settled.own.gold),
            (12, Some(-500)),
            "the buyer pays the full price"
        );
        assert_eq!(
            (settled.partner.owner_id, settled.partner.gold),
            (11, Some(500)),
            "the seller gains it whole"
        );
        let buyer = state.characters.find_by_vid(BUYER).unwrap();
        assert_eq!(buyer.gold(), 500);
        assert_eq!(buyer.items().item(5).unwrap().pos, inventory(0));
        let seller = state.characters.find_by_vid(SELLER).unwrap();
        assert_eq!(seller.gold(), 500);
        assert!(seller.items().item(5).is_none());
        assert_eq!(
            settled.own.changes.len(),
            0,
            "the buyer side stores no rows"
        );
        assert_eq!(settled.partner.changes.len(), 1, "the give stores");
        // The buyer's and the seller's copies ride their settled sides, after the
        // move's records.
        for (side, records) in [
            ("buyer", &settled.own.records),
            ("seller", &settled.partner.records),
        ] {
            let update = records.last().expect("the update trails the move");
            assert_eq!(
                update[0],
                protocol::gc_inventory::HEADER_GC_SHOP.value(),
                "the {side} hears the slot clear: {records:02x?}"
            );
            assert_eq!(
                update[3],
                protocol::gc_shop::SHOP_SUBHEADER_GC_UPDATE_ITEM,
                "the {side} hears the slot clear: {records:02x?}"
            );
        }
        let update = drained(&mut buyer_inbox);
        assert!(update.is_empty(), "no guest hears it twice: {update:02x?}");
    }

    /// The `personal_shop` flag taxes the seller: the buyer still pays the full
    /// price, the seller gains the net, the treasury takes the tax, and the
    /// seller hears `[LS;881;%d]` with the rate.
    #[test]
    fn a_taxed_buy_pays_the_net_and_the_treasury() {
        use super::super::shop::{ShopAnswer, ShopStep};
        const BUYER: Vid = Vid::new(12);
        let (mut state, mut seller_inbox, mut buyer_inbox) = a_market(1_000);
        with_quests(&mut state);
        state.set_event_flag(b"personal_shop", 10);
        state
            .myshop(SELLER, sign_of(b"Swords"), &[table(0, 500, 3)], mover())
            .unwrap();
        let place = GroundPlace {
            channel: 1,
            map: 41,
            x: 5,
            y: 6,
        };
        let account = crate::game_loop_messages::QuestAccount::default();
        let opened = state.shop(
            BUYER,
            ShopStep::Click { target: 11 },
            place,
            mover(),
            &account,
        );
        assert!(matches!(opened, Ok(ShopAnswer::Sent(_))));
        drained(&mut seller_inbox);
        drained(&mut buyer_inbox);
        let bought = state
            .shop(BUYER, ShopStep::Buy { pos: 3 }, place, mover(), &account)
            .unwrap();
        let ShopAnswer::Settled(settled) = bought else {
            panic!("the buy settles: {bought:?}");
        };
        assert_eq!(
            settled.own.gold,
            Some(-500),
            "the buyer pays the full price"
        );
        assert_eq!(settled.partner.gold, Some(450), "the seller gains the net");
        assert_eq!(settled.monarch, Some((1, 50)), "the empire takes the tax");
        assert_eq!(state.characters.find_by_vid(BUYER).unwrap().gold(), 500);
        assert_eq!(state.characters.find_by_vid(SELLER).unwrap().gold(), 450);
        let told = drained(&mut seller_inbox);
        assert!(
            told.iter().any(|record| {
                record[0] == protocol::gc_inventory::HEADER_GC_CHAT.value()
                    && record
                        .windows(b"[LS;881;10]".len())
                        .any(|window| window == b"[LS;881;10]")
            }),
            "the seller hears the tax line: {told:02x?}"
        );
    }

    /// A poor buyer is told too little gold, and an empty slot too; a slot
    /// whose item left the seller answers sold out before the gold is counted.
    #[test]
    fn the_refusals_answer_their_subheaders() {
        use super::super::shop::{ShopAnswer, ShopStep};
        use protocol::gc_shop::{SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY, SHOP_SUBHEADER_GC_SOLD_OUT};
        const BUYER: Vid = Vid::new(12);
        let account = crate::game_loop_messages::QuestAccount::default();
        let place = GroundPlace {
            channel: 1,
            map: 41,
            x: 5,
            y: 6,
        };
        let (mut state, mut seller_inbox, mut buyer_inbox) = a_market(400);
        state
            .myshop(SELLER, sign_of(b"Swords"), &[table(0, 500, 3)], mover())
            .unwrap();
        let opened = state.shop(
            BUYER,
            ShopStep::Click { target: 11 },
            place,
            mover(),
            &account,
        );
        assert!(matches!(opened, Ok(ShopAnswer::Sent(_))));
        let poor = state
            .shop(BUYER, ShopStep::Buy { pos: 3 }, place, mover(), &account)
            .unwrap();
        assert!(matches!(
            poor,
            ShopAnswer::Declined { records, .. }
            if records.len() == 1 && records[0][3] == SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY
        ));
        // An empty slot costs 0, which is too little gold.
        let empty = state
            .shop(BUYER, ShopStep::Buy { pos: 4 }, place, mover(), &account)
            .unwrap();
        assert!(matches!(
            empty,
            ShopAnswer::Declined { records, .. }
            if records.len() == 1 && records[0][3] == SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY
        ));
        // The item left the seller: sold out before the gold is counted.
        {
            let character = state.characters.find_by_vid_mut(SELLER).unwrap();
            let _ = character.items_mut().release(5);
        }
        let gone = state
            .shop(BUYER, ShopStep::Buy { pos: 3 }, place, mover(), &account)
            .unwrap();
        assert!(matches!(
            gone,
            ShopAnswer::Declined { records, .. }
            if records.len() == 1 && records[0][3] == SHOP_SUBHEADER_GC_SOLD_OUT
        ));
        let _ = (&mut seller_inbox, &mut buyer_inbox);
    }
}
