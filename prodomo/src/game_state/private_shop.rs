//! The personal shop: `CG_PRIVATE_SHOP` opens the owner's panel and builds the
//! stand (`CInputMain::PrivateShopPanelOpen` and `PrivateShopBuild`,
//! `G/input_main.cpp:4222-4236` and `:4136-4185`, `CHARACTER::BuildPrivateShop`,
//! `G/char.cpp:11989-12127`, and `CPrivateShopManager::BuildPrivateShop`,
//! `G/private_shop_manager.cpp:80-184`).
//!
//! The world keeps each owner's panel flag and built shop, as legacy's
//! `m_bIsEditingPrivateShop` and `CPrivateShopManager::m_map_privateShop` do. A
//! ware names the held item's id, its shop slot and its price; the item itself
//! leaves the owner's window for the shop's table at build, as `RemoveItemCache`
//! and `PutPrivateShopItemCache` move it (`D/ClientManagerPrivateShop.cpp:1184-1192`).
//!
//! # The order
//!
//! The input gates run first: a pending logout is canceled with its line, and
//! stunned or dead is silent. Then the trade windows refuse with
//! `You cannot open a personal shop while another window is open.` (a trade, an
//! open safebox, an open own bundle shop, a browsed shop, an open cube, or a
//! smith's window), and a build within ten seconds of the last refuses with
//! `Please wait a few moments before building your personal shop again.` Then
//! `CHARACTER::BuildPrivateShop` runs: an observer, an open own panel or a
//! smith's window refuses with `You cannot use a personal shop now.` (`CanHandleItem`
//! answers no panel, no cube, no dragon soul, no sash, no aura and no changelook
//! window here, because the six have their own windows and the seventh is the
//! panel itself); an open shop, a dungeon, and a map outside the empire's market
//! refuse with their lines; a trade is canceled; a running quest is silent; an
//! empty build logs and refuses; a banned word refuses with
//! `Please choose another title name for your personal shop.`; and a title under
//! three bytes refuses with `The entered name is too short.` Each table is then
//! checked: a repeated cell logs and refuses, a missing item or proto logs and
//! refuses, a bundle refuses with `You cannot put Bundles items in a personal shop
//! when building it.`, an item-shop item with `You cannot sell Item-Shop items in a
//! personal shop.`, a worn one with `You cannot sell equipped items in a personal shop.`,
//! a locked one with `You cannot sell locked items in a personal shop.`, and a
//! negative price logs and refuses.
//!
//! The stand's polymorph clamps to the bundle row: under 30000 or over 30008 it
//! is 30000. A ware past the two pages, on a taken slot, or overlapping another
//! ware is refused, as `Initialize` refuses it (`G/private_shop.cpp:150-164`).
//!
//! The build stores as one transaction — the shop row, the wares leaving `item`,
//! the wares entering `private_shop_item` — and then the owner hears `LOAD` and
//! the viewers around the stand hear `ADD_ENTITY`, in that order, as
//! `SetPrivateShopTable` and `Show` send them (`G/char.cpp:12160-12178` and
//! `G/private_shop.cpp:58-92`).
//!
//! A browse opens the window: the logout cancel, the trade windows, the shop found
//! by its stand's VID, the owner's own click opening its panel, the same window
//! ignored, and the old shop left behind (`CInputMain::PrivateShopStart` and
//! `CPrivateShop::AddShopViewer`, `G/input_main.cpp:4271-4322` and
//! `G/private_shop.cpp:739-789`). An `END` stops the browse (`StopShopping`,
//! `G/private_shop_manager.cpp:393-402`).
//!
//! A buy settles at once, as the four legacy hops do in one breath: the input gates
//! (the logout cancel, the one-second wait, the stun, the viewed shop, the ware's
//! presence), the DB checks (the modify state, the ware, the price, the gold), and
//! the game transaction (the space, the gold again, the placement, the price to the
//! stash), and the DB completion (the sale to the owner, the stash, the ware gone).
//! Legacy forwards the game copy's price and re-checks it against the DB copy; with
//! one world the two are one, so the price check compares the shop's own price with
//! itself and always holds. The search and market updates are later slices.
//!
//! The close, the withdraw and the modify flip are the owner's own steps. The close
//! refuses on another map or from more than 3000 away, moves the wares back to the
//! owner's window biggest-first, cleans the viewers, deletes the rows, and answers
//! `CLOSE` with the closed line. The withdraw moves the stash to the gold under the
//! gold cap and answers the bare `WITHDRAW`. The modify flips the state between
//! open and modify and answers `STATE_UPDATE`.

use common::item_slots::{usable_inventory_cells, EWindows};
use common::vid::Vid;
use protocol::cg_private_shop::{
    CgPrivateShopItem, CgPrivateShopSearch, CG_PRIVATE_SHOP_TITLE_FIELD_SIZE,
};
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_private_shop::{
    GcPrivateShop, GcPrivateShopAddEntity, GcPrivateShopBalance, GcPrivateShopItemData,
    GcPrivateShopLoad, GcPrivateShopMove, GcPrivateShopPriceChange, GcPrivateShopRemove,
    GcPrivateShopSale, GcPrivateShopSearchData, GcPrivateShopSearchResult,
    GcPrivateShopSearchUpdate, GcPrivateShopSetItem, GcPrivateShopStart, GcPrivateShopState,
    GcPrivateShopTitle, GcPrivateShopTitleChange, GcPrivateShopUnlocked,
    GC_PRIVATE_SHOP_SEARCH_DATA_WIRE_SIZE, PRIVATE_SHOP_OWNER_FIELD_SIZE, PRIVATE_SHOP_STATE_OPEN,
    SUBHEADER_GC_PRIVATE_SHOP_ADD_ITEM, SUBHEADER_GC_PRIVATE_SHOP_CLOSE,
    SUBHEADER_GC_PRIVATE_SHOP_CLOSE_PANEL, SUBHEADER_GC_PRIVATE_SHOP_END,
    SUBHEADER_GC_PRIVATE_SHOP_OPEN_PANEL, SUBHEADER_GC_PRIVATE_SHOP_REMOVE_ITEM,
    SUBHEADER_GC_PRIVATE_SHOP_REMOVE_MY_ITEM, SUBHEADER_GC_PRIVATE_SHOP_SEARCH_OPEN_LOOK_MODE,
    SUBHEADER_GC_PRIVATE_SHOP_SEARCH_OPEN_TRADE_MODE, SUBHEADER_GC_PRIVATE_SHOP_SET_ITEM,
    SUBHEADER_GC_PRIVATE_SHOP_WITHDRAW,
};
use protocol::gc_whisper::{GcWhisper, WHISPER_TYPE_SYSTEM};
use world::item::{ItemId, ITEM_ANTIFLAG_GIVE, ITEM_ANTIFLAG_MYSHOP};

use super::GameState;
use crate::chat_line::{chat_packet, Arg};
use crate::item_move::{gold_delta, MoveItemRefused, MovedItems, Mover};
use crate::loading_phase::INSTANCE_MAP_BASE;

/// The shop grid's width: `PRIVATE_SHOP_WIDTH` (`C/length.h:133`).
pub const PRIVATE_SHOP_GRID_WIDTH: usize = 8;

/// A page's slots: `PRIVATE_SHOP_PAGE_ITEM_MAX_NUM` (`C/length.h:135`).
pub const PRIVATE_SHOP_PAGE_SLOTS: usize = 64;

/// The window's slots: `PRIVATE_SHOP_HOST_ITEM_MAX_NUM` (`C/length.h:136`).
pub const PRIVATE_SHOP_WINDOW_SLOTS: usize = 128;

/// `30000`: the bundle mob a bare stand shows as (`G/char.cpp:12118-12120`).
pub const PRIVATE_SHOP_DEFAULT_VNUM: u32 = 30000;

/// The last bundle-row vnum a stand may wear.
pub const PRIVATE_SHOP_MAX_VNUM: u32 = 30008;

/// `TITLE_MIN_LEN` (`C/length.h:147`): the shortest title that builds.
pub const PRIVATE_SHOP_TITLE_MIN_LEN: usize = 3;

/// The paper bundle, which a build never lists (`G/char.cpp:12065-12069`).
pub const PRIVATE_SHOP_PAPER_BUNDLE: u32 = 50200;

/// The silk bundle, which a build never lists either.
pub const PRIVATE_SHOP_SILK_BUNDLE: u32 = 71221;

/// The ten seconds between two builds, in Pulses (`PASSES_PER_SEC(10)`).
pub const PRIVATE_SHOP_BUILD_WAIT_PULSES: u64 = 250;

/// The one second between two buys, in Pulses (`PASSES_PER_SEC(1)`).
pub const PRIVATE_SHOP_BUY_WAIT_PULSES: u64 = 25;

/// The ten seconds between two closes or two withdraws, in Pulses.
pub const PRIVATE_SHOP_CLOSE_WAIT_PULSES: u64 = 250;

/// The one second between two state flips, in Pulses.
pub const PRIVATE_SHOP_STATE_WAIT_PULSES: u64 = 25;

/// The two seconds between two searches, in Pulses (`PASSES_PER_SEC(2)`).
pub const PRIVATE_SHOP_SEARCH_WAIT_PULSES: u64 = 50;

/// `MODE_LOOKING` (1): the search window shows wares (`C/length.h:153`).
pub const PRIVATE_SHOP_SEARCH_LOOKING: u8 = 1;

/// `MODE_TRADING` (2): the search window buys wares.
pub const PRIVATE_SHOP_SEARCH_TRADING: u8 = 2;

/// The looking glass, which opens the search to look (`G/char_item.cpp:4391-4394`).
pub const PRIVATE_SHOP_LOOKING_GLASS: u32 = 60004;

/// The trading glass, which opens the search to trade (`:4396-4399`).
pub const PRIVATE_SHOP_TRADING_GLASS: u32 = 60005;

/// `SAFE_RECV_BUFSIZE` (8192): the rows stop when the next would overflow it
/// (`G/private_shop_manager.cpp:943`).
pub const PRIVATE_SHOP_SEARCH_SAFE_BYTES: usize = 8192;

/// `STATE_REMOVED` (1): the update names a gone ware (`C/length.h:168`).
pub const PRIVATE_SHOP_SEARCH_REMOVED: u8 = 1;

/// `STATE_MODIFY` (3): the shop takes no customers.
pub const PRIVATE_SHOP_STATE_MODIFY: u8 = 3;

/// The 3000 units past which a close refuses (`ClosePrivateShop`,
/// `G/private_shop_manager.cpp:278-284`).
pub const PRIVATE_SHOP_CLOSE_DISTANCE: i32 = 3000;

/// The key that unlocks a shop's slots: `PRIVATE_SHOP_SLOT_UNLOCK_ITEM`
/// (`C/length.h:138`).
pub const PRIVATE_SHOP_SLOT_UNLOCK_VNUM: u32 = 72357;

/// The locked slots: `PRIVATE_SHOP_LOCKED_SLOT_MAX_NUM` (`C/length.h:137`), the
/// window's second half.
pub const PRIVATE_SHOP_LOCKED_SLOT_MAX_NUM: u16 = 64;

/// One ware of a built shop: the instance, its slot and its price. The instance rides
/// along because the window's rows show the sockets and attrs (`AddShopViewer` reads
/// them off the shop's items, `G/private_shop.cpp:775-786`), and the buy hands the same
/// instance to the buyer's window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateShopWare {
    /// The instance, as it stood in the owner's window.
    pub item: world::item::Item,
    /// The asking price in yang.
    pub gold: i64,
    /// The asking price in cheques.
    pub cheque: u32,
    /// The checkin time the window shows.
    pub checkin: i64,
}

/// A character's built personal shop: the stand and its wares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateShop {
    /// The title as shown, NUL-padded.
    pub title: [u8; CG_PRIVATE_SHOP_TITLE_FIELD_SIZE],
    /// The stand's polymorph vnum, clamped to the bundle row.
    pub vnum: u32,
    /// The title's type byte.
    pub title_type: u8,
    /// The page count.
    pub page_count: u8,
    /// The wares by shop slot.
    pub wares: [Option<PrivateShopWare>; PRIVATE_SHOP_WINDOW_SLOTS],
    /// The stash in yang (`TPrivateShop::llGold`).
    pub gold: i64,
    /// The shop's state: open (2) or modify (3).
    pub state: u8,
    /// The unlocked count of the window's locked second half.
    pub unlocked: u16,
    /// Where the stand stands: the map and the spot.
    pub at: (i32, i32, i32),
    /// The Channel the stand is on: the owner's at the build, which never moves.
    pub channel: u8,
}

/// A build's decoration: the title, the stand, and the pages.
#[derive(Debug, Clone, Copy)]
pub struct PrivateBuild {
    /// The raw title field.
    pub title: [u8; CG_PRIVATE_SHOP_TITLE_FIELD_SIZE],
    /// The polymorph vnum the shop stands as.
    pub poly_vnum: u32,
    /// The title's type byte.
    pub title_type: u8,
    /// The page count.
    pub page_count: u8,
}

/// What a panel step did: the records the client is sent, and nothing stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateShopPanel {
    /// The records the client is sent.
    pub records: Vec<Vec<u8>>,
}

/// What a buy did: the buyer's move, and the ware's id, the shop's owner, and the
/// price, for the store to write in one transaction.
#[derive(Debug, Clone)]
pub struct PrivateShopBought {
    /// The buyer's move: the gold it holds and the ware's set.
    pub bought: MovedItems,
    /// The ware's item id, leaving `private_shop_item` for `item`.
    pub ware: ItemId,
    /// The shop owner's player id.
    pub owner: u32,
    /// The price in yang, joining the shop's stash.
    pub price: i64,
}

/// What a withdraw did: the owner's move, and the shop's owner, for the store to
/// empty the stash in one transaction. `destroy` names an empty shop the withdraw
/// destroys with it.
#[derive(Debug, Clone)]
pub struct PrivateShopWithdrawn {
    /// The owner's move: `WITHDRAW` first, then the gold.
    pub withdrawn: MovedItems,
    /// The shop owner's player id.
    pub owner: u32,
    /// Whether the shop held no wares and is destroyed with the withdraw.
    pub destroy: bool,
}

/// What a price change did: the shop's owner, the ware's slot and its new price,
/// for the store. The owner and the viewers around the stand hear the price.
#[derive(Debug, Clone, Copy)]
pub struct PrivateShopPriceChanged {
    /// The shop owner's player id.
    pub owner: u32,
    /// The ware's shop slot.
    pub pos: u16,
    /// The new price in yang.
    pub gold: i64,
    /// The new price in cheques.
    pub cheque: u32,
}

/// What a move did: the shop's owner and the ware's old and new slots, for the
/// store. The owner and the viewers around the stand hear the move.
#[derive(Debug, Clone, Copy)]
pub struct PrivateShopMoved {
    /// The shop owner's player id.
    pub owner: u32,
    /// The ware's old shop slot.
    pub pos: u16,
    /// The ware's new shop slot.
    pub change_pos: u16,
}

/// What a checkin did: the shop's owner and the ware's row, for the store to
/// write in one transaction. The owner hears `SET_ITEM`, the viewers `ADD_ITEM`.
#[derive(Debug, Clone)]
pub struct PrivateShopCheckedIn {
    /// The shop owner's player id.
    pub owner: u32,
    /// The ware's row, leaving `item` for `private_shop_item`.
    pub ware: db::private_shop::ShopItemRow,
}

/// What a checkout did: the shop's owner, the ware's id, and where it lands, for
/// the store to write in one transaction. The owner's move carries the set; the
/// viewers around the stand hear the slot's clear.
#[derive(Debug, Clone)]
pub struct PrivateShopCheckedOut {
    /// The owner's move: the clear first, then the ware's set.
    pub moved: MovedItems,
    /// The shop owner's player id.
    pub owner: u32,
    /// The ware's item id, leaving `private_shop_item` for `item`.
    pub ware: ItemId,
    /// The window it lands in.
    pub window_type: u8,
    /// The cell it lands in.
    pub pos: u16,
    /// Whether no wares are left and the shop is destroyed with the checkout.
    pub destroy: bool,
}

/// What a rename did: the shop's owner and the new title, for the store. The
/// owner hears `TITLE_CHANGE`, the viewers around the stand `TITLE`.
#[derive(Debug, Clone, Copy)]
pub struct PrivateShopRenamed {
    /// The shop owner's player id.
    pub owner: u32,
    /// The raw 33-byte title field.
    pub title: [u8; CG_PRIVATE_SHOP_TITLE_FIELD_SIZE],
}

/// What an unlock did: the owner's move and the unlocked count, for the store.
/// The viewers around the stand hear the unlocked count; a character with no
/// stand hears the increased line.
#[derive(Debug, Clone)]
pub struct PrivateShopUnlocked {
    /// The owner's move: the key's spend and the point's records.
    pub moved: MovedItems,
    /// The character's player id.
    pub owner: u32,
    /// The unlocked count, for the shop row or the player row.
    pub slots: u16,
    /// Whether a stand stands and the shop row takes the count.
    pub standing: bool,
}

/// What a warp request did: the panel's close and the teleport line, in that
/// order. The countdown itself fires on the timed slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateShopWarp {
    /// The records the client is sent.
    pub records: Vec<Vec<u8>>,
}

/// What a modify did: the owner's flip, for the store.
#[derive(Debug, Clone, Copy)]
pub struct PrivateShopFlipped {
    /// The shop owner's player id.
    pub owner: u32,
    /// The shop's new state.
    pub state: u8,
}

/// What a close did: the owner's move, and the shop's owner and wares, for the
/// store to write in one transaction.
#[derive(Debug, Clone)]
pub struct PrivateShopClosed {
    /// The owner's move: `CLOSE` first, then the wares' sets.
    pub closed: MovedItems,
    /// The shop owner's player id.
    pub owner: u32,
    /// The wares' item ids, leaving `private_shop_item` for `item`.
    pub wares: Vec<ItemId>,
}

/// What a build did: the stored shop and wares, and the records for the owner
/// and the viewers around the stand.
#[derive(Debug, Clone)]
pub struct PrivateShopBuilt {
    /// The shop row the store writes.
    pub shop: db::private_shop::ShopRow,
    /// The wares the store writes.
    pub wares: Vec<db::private_shop::ShopItemRow>,
    /// The owner's `LOAD` record.
    pub load: Vec<u8>,
    /// The stand's `ADD_ENTITY`, for the viewers around it.
    pub entity: Vec<u8>,
}

impl GameState {
    /// Whether the character under `vid` owns a built personal shop.
    pub(super) fn has_private_shop(&self, vid: Vid) -> bool {
        self.private_shops.contains_key(&vid)
    }

    /// Whether the character under `vid` edits its panel.
    fn edits_private_shop(&self, vid: Vid) -> bool {
        self.private_panels.contains(&vid)
    }

    /// Whether the character under `vid` browses the shop search.
    pub(super) fn searches_private_shop(&self, vid: Vid) -> bool {
        self.private_searching.contains_key(&vid)
    }

    /// The glass at `at` of the character under `vid`, or `None` when the step is no
    /// search open: no item, or neither glass (`G/char_item.cpp:4391-4399`).
    pub(super) fn search_glass_at(&self, vid: Vid, at: protocol::item_pos::ItemPos) -> Option<u32> {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .map(|item| item.vnum)
            .filter(|vnum| {
                *vnum == PRIVATE_SHOP_LOOKING_GLASS || *vnum == PRIVATE_SHOP_TRADING_GLASS
            })
    }

    /// `CHARACTER::OpenShopSearch` (`G/char.cpp:12669-12689`): the trade windows
    /// refuse with their line, else the mode sets and the bare open answers. The
    /// glass is kept, as the `USE_SPECIAL` arm consumes nothing.
    pub(super) fn open_shop_search(
        &mut self,
        vid: Vid,
        vnum: u32,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        if !self.private_trade_windows_clear(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot trade while another window is open.",
                &[],
            );
            return self.nothing_used(vid, actor);
        }
        let mode = if vnum == PRIVATE_SHOP_LOOKING_GLASS {
            PRIVATE_SHOP_SEARCH_LOOKING
        } else {
            PRIVATE_SHOP_SEARCH_TRADING
        };
        let subheader = if mode == PRIVATE_SHOP_SEARCH_LOOKING {
            SUBHEADER_GC_PRIVATE_SHOP_SEARCH_OPEN_LOOK_MODE
        } else {
            SUBHEADER_GC_PRIVATE_SHOP_SEARCH_OPEN_TRADE_MODE
        };
        self.private_searching.insert(vid, mode);
        let mut moved = self.nothing_used(vid, actor)?;
        moved.records.push(GcPrivateShop::new(subheader).encode());
        Ok(moved)
    }

    /// `CInputMain::PrivateShopPanelOpen` for the character under `vid`
    /// (`G/input_main.cpp:4222-4236`): the trade windows refuse, an open panel
    /// is silent, and otherwise the panel opens.
    pub fn private_panel_open(&mut self, vid: Vid, actor: Mover) -> PrivateShopPanel {
        if !self.private_trade_windows_clear(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot open a personal shop while another window is open.",
                &[],
            );
            return PrivateShopPanel {
                records: Vec::new(),
            };
        }
        if self.edits_private_shop(vid) {
            return PrivateShopPanel {
                records: Vec::new(),
            };
        }
        // A viewed shop drops its viewer first, as the close drops it
        // (`CHARACTER::OpenPrivateShopPanel`, `G/char.cpp:12182-12195`).
        if let Some(viewed) = self.private_viewing.remove(&vid) {
            if let Some(shop) = self.private_viewers.get_mut(&viewed) {
                shop.retain(|viewer| *viewer != vid);
            }
        }
        self.private_panels.insert(vid);
        PrivateShopPanel {
            records: vec![GcPrivateShop::new(SUBHEADER_GC_PRIVATE_SHOP_OPEN_PANEL).encode()],
        }
    }

    /// `CInputMain::PrivateShopPanelClose` for the character under `vid`
    /// (`G/input_main.cpp:4238-4254`): stunned or dead is silent, and otherwise
    /// the panel closes. The modify request it may send is a later slice.
    pub fn private_panel_close(&mut self, vid: Vid) -> PrivateShopPanel {
        if self.is_stunned(vid.raw()) || self.is_dead(vid.raw()) {
            return PrivateShopPanel {
                records: Vec::new(),
            };
        }
        if !self.edits_private_shop(vid) {
            return PrivateShopPanel {
                records: Vec::new(),
            };
        }
        self.private_panels.remove(&vid);
        PrivateShopPanel {
            records: vec![GcPrivateShop::new(SUBHEADER_GC_PRIVATE_SHOP_CLOSE_PANEL).encode()],
        }
    }

    /// `CInputMain::PrivateShopBuild` and `CHARACTER::BuildPrivateShop` for the
    /// character under `vid`: the gates in legacy order, then the build. The
    /// shop stands in the world at once, as `CPrivateShopManager::BuildPrivateShop`
    /// stands it before the store answers; the records go out after the store
    /// takes the rows, which is why the build returns them instead of sending
    /// them.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn private_build(
        &mut self,
        vid: Vid,
        build: PrivateBuild,
        tables: &[CgPrivateShopItem],
        actor: Mover,
    ) -> Result<Result<PrivateShopBuilt, MovedItems>, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        let Some((channel, map)) = self.private_gates(vid, tables, actor) else {
            return Ok(Err(self.private_refused(vid, pid, actor)));
        };
        let Some(wares) = self.private_title_and_wares(vid, build.title, tables, actor) else {
            return Ok(Err(self.private_refused(vid, pid, actor)));
        };
        let vnum = if (PRIVATE_SHOP_DEFAULT_VNUM..=PRIVATE_SHOP_MAX_VNUM).contains(&build.poly_vnum)
        {
            build.poly_vnum
        } else {
            PRIVATE_SHOP_DEFAULT_VNUM
        };
        Ok(Ok(
            self.private_stand(vid, build, vnum, &wares, channel, map)
        ))
    }

    /// The build's gates in legacy order, answering the stand's place: the logout
    /// cancel, the stun, the ten-second wait, the trade windows, the handle check,
    /// the open shop, the dungeon, the map, the trade cancel, the running quest,
    /// and the empty build. `None` after telling the refusal.
    fn private_gates(
        &mut self,
        vid: Vid,
        tables: &[CgPrivateShopItem],
        actor: Mover,
    ) -> Option<(u8, i32)> {
        if self.timed_events.remove(&vid).is_some() {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Your logout has been canceled.",
                &[],
            );
            return None;
        }
        if self.is_stunned(vid.raw()) || self.is_dead(vid.raw()) {
            return None;
        }
        let last = self.private_build_at.get(&vid).copied().unwrap_or(0);
        if last != 0 && self.last_pulse.saturating_sub(last) < PRIVATE_SHOP_BUILD_WAIT_PULSES {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Please wait a few moments before building your personal shop again.",
                &[],
            );
            return None;
        }
        if !self.private_trade_windows_clear(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot open a personal shop while another window is open.",
                &[],
            );
            return None;
        }
        if self.is_observing(vid) || self.edits_private_shop(vid) || self.under_refine(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot use a personal shop now.",
                &[],
            );
            return None;
        }
        if self.has_private_shop(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You have already opened your personal shop.",
                &[],
            );
            return None;
        }
        let place = self.place_of(vid.raw())?;
        if place.1 >= INSTANCE_MAP_BASE {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot open a personal shop in a dungeon.",
                &[],
            );
            return None;
        }
        if !self.private_map_allowed(vid, place.1) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot open a personal shop in this map.",
                &[],
            );
            return None;
        }
        self.cancel_trade(vid);
        if self.quest_running(vid) || tables.is_empty() {
            return None;
        }
        Some(place)
    }

    /// The title and the wares the tables stand on, or `None` after telling the
    /// refusal: a banned word refuses with its line, a short title with its, and
    /// the wares with theirs.
    fn private_title_and_wares(
        &mut self,
        vid: Vid,
        title: [u8; CG_PRIVATE_SHOP_TITLE_FIELD_SIZE],
        tables: &[CgPrivateShopItem],
        actor: Mover,
    ) -> Option<Vec<(u16, world::item::Item, i64, u32)>> {
        let end = title
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(title.len());
        if self.name_rules.holds_word(&title[..end]) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Please choose another title name for your personal shop.",
                &[],
            );
            return None;
        }
        if end < PRIVATE_SHOP_TITLE_MIN_LEN {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"The entered name is too short.",
                &[],
            );
            return None;
        }
        self.private_wares(vid, tables, actor)
    }

    /// An empty answer for a refused build: the chats went out immediately.
    fn private_refused(&self, vid: Vid, pid: u32, actor: Mover) -> MovedItems {
        MovedItems::new(
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
        )
    }

    /// The trade-window gates of a personal-shop step (`CheckTradeWindows`,
    /// `G/private_shop_util.cpp:66-82`): a trade, an open safebox, an open own
    /// bundle shop, a browsed shop, an open cube, or a smith's window refuses.
    fn private_trade_windows_clear(&self, vid: Vid) -> bool {
        self.trade_partner(vid).is_none()
            && !self.safebox_open(vid)
            && !self.has_my_shop(vid)
            && !self.browsing.contains_key(&vid)
            && !self.cubes.contains_key(&vid)
            && !self.under_refine(vid)
    }

    /// Whether the empire's market is the map (`isShopAllowed`, `G/char.cpp:11975-11987`):
    /// empire 1 builds on map 1, empire 2 on map 21, empire 3 on map 41, and any
    /// other empire anywhere.
    fn private_map_allowed(&self, vid: Vid, map: i32) -> bool {
        let empire = self.bodies.get(&vid).map_or(0, |body| body.card.empire);
        match empire {
            1 => map == 1,
            2 => map == 21,
            3 => map == 41,
            _ => true,
        }
    }

    /// The wares the tables stand on, or `None` after telling the refusal: a
    /// repeated cell, a missing item or proto, and a negative price are silent
    /// (legacy logs them), a bundle refuses with its line, an item-shop item
    /// with its, a worn one with its, and a locked one with its. A ware past
    /// the pages, on a taken slot, or overlapping another is silent, as
    /// `Initialize` refuses it.
    fn private_wares(
        &mut self,
        vid: Vid,
        tables: &[CgPrivateShopItem],
        actor: Mover,
    ) -> Option<Vec<(u16, world::item::Item, i64, u32)>> {
        use std::collections::BTreeSet;
        let mut seen = BTreeSet::new();
        let mut wares = Vec::with_capacity(tables.len());
        for table in tables {
            let key = (table.pos.window_type, table.pos.cell);
            if !seen.insert(key) {
                return None;
            }
            let Ok(character) = self.characters.find_by_vid(vid) else {
                return None;
            };
            let item = character.items().item_at(table.pos)?;
            let proto = self.protos.get(item.vnum)?;
            if item.vnum == PRIVATE_SHOP_PAPER_BUNDLE || item.vnum == PRIVATE_SHOP_SILK_BUNDLE {
                self.tell(
                    vid,
                    actor,
                    CHAT_TYPE_INFO,
                    b"You cannot put Bundles items in a personal shop when building it.",
                    &[],
                );
                return None;
            }
            if proto.anti_flags & (ITEM_ANTIFLAG_GIVE | ITEM_ANTIFLAG_MYSHOP) != 0 {
                self.tell(
                    vid,
                    actor,
                    CHAT_TYPE_INFO,
                    b"You cannot sell Item-Shop items in a personal shop.",
                    &[],
                );
                return None;
            }
            if table.pos.window_type == EWindows::Equipment as u8 {
                self.tell(
                    vid,
                    actor,
                    CHAT_TYPE_INFO,
                    b"You cannot sell equipped items in a personal shop.",
                    &[],
                );
                return None;
            }
            if self.sash_item_at(vid, table.pos)
                || self.changelook_item_at(vid, table.pos)
                || self.aura_item_at(vid, table.pos)
            {
                self.tell(
                    vid,
                    actor,
                    CHAT_TYPE_INFO,
                    b"You cannot sell locked items in a personal shop.",
                    &[],
                );
                return None;
            }
            if table.gold < 0 {
                return None;
            }
            wares.push((table.display_pos, item.clone(), table.gold, table.cheque));
        }
        // The grid's answer, as `Initialize` grids it (`G/private_shop.cpp:150-164`):
        // each page is an 8-by-8 grid, each ware stands one wide and its size tall,
        // and an overlap refuses the build.
        let mut placed: Vec<(usize, i32)> = Vec::with_capacity(wares.len());
        for (slot, item, _, _) in &wares {
            let at = usize::from(*slot);
            let size = self.protos.get(item.vnum).map_or(0, |proto| proto.size);
            if at >= PRIVATE_SHOP_WINDOW_SLOTS
                || placed.iter().any(|&(other, _)| other == at)
                || !Self::private_cell_free(&placed, at, size)
            {
                return None;
            }
            placed.push((at, size));
        }
        Some(wares)
    }

    /// Whether a ware of `size` stands on shop slot `at` over `placed`, as
    /// `CGrid::IsEmpty(at, 1, size)` answers it: the column stays inside the
    /// page's 8-by-8 grid and every cell is free.
    fn private_cell_free(placed: &[(usize, i32)], at: usize, size: i32) -> bool {
        if size < 1 || at >= PRIVATE_SHOP_WINDOW_SLOTS {
            return false;
        }
        let page_at = at % PRIVATE_SHOP_PAGE_SLOTS;
        if page_at / PRIVATE_SHOP_GRID_WIDTH + usize::try_from(size).unwrap_or(usize::MAX)
            > PRIVATE_SHOP_GRID_WIDTH
        {
            return false;
        }
        let cells = |anchor: usize, tall: i32| {
            (0..tall).map(move |row| {
                anchor + usize::try_from(row).unwrap_or(0) * PRIVATE_SHOP_GRID_WIDTH
            })
        };
        for cell in cells(at, size) {
            // A ware never crosses its page: the anchor's page holds every cell.
            if cell / PRIVATE_SHOP_PAGE_SLOTS != at / PRIVATE_SHOP_PAGE_SLOTS {
                return false;
            }
            for &(other, tall) in placed {
                if cells(other, tall).any(|taken| taken == cell) {
                    return false;
                }
            }
        }
        true
    }

    /// Stand the shop: the wares leave the owner's window, the shop stands in
    /// the world, the build stamps its Pulse, and the `LOAD` and `ADD_ENTITY`
    /// records pack for the store to order after its rows.
    fn private_stand(
        &mut self,
        vid: Vid,
        stood: PrivateBuild,
        vnum: u32,
        wares: &[(u16, world::item::Item, i64, u32)],
        channel: u8,
        map: i32,
    ) -> PrivateShopBuilt {
        let spot = self
            .spot_of(vid.raw())
            .map_or((0, 0), |spot| (spot.x, spot.y));
        let name = self
            .characters
            .find_by_vid(vid)
            .map_or(String::new(), |character| character.name().to_owned());
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        let now = Self::private_now();
        let (slots, stored) = self.private_take_wares(vid, pid, wares, now);
        self.private_shops.insert(
            vid,
            PrivateShop {
                title: stood.title,
                vnum,
                title_type: stood.title_type,
                page_count: stood.page_count,
                wares: slots,
                gold: 0,
                state: PRIVATE_SHOP_STATE_OPEN,
                unlocked: 0,
                at: (map, spot.0, spot.1),
                channel,
            },
        );
        self.private_panels.insert(vid);
        self.private_build_at.insert(vid, self.last_pulse);
        let owner = private_owner_field(&name);
        let shop = db::private_shop::ShopRow {
            owner_id: pid,
            title: stood.title.to_vec(),
            owner_name: owner.to_vec(),
            state: PRIVATE_SHOP_STATE_OPEN,
            vnum,
            title_type: stood.title_type,
            map_index: map,
            x: spot.0,
            y: spot.1,
            channel,
            gold: 0,
            cheque: 0,
            page_count: stood.page_count,
            premium_time: now,
            unlocked_slots: 0,
        };
        let load = GcPrivateShopLoad {
            title: stood.title,
            gold: 0,
            cheque: 0,
            x: spot.0,
            y: spot.1,
            channel,
            state: PRIVATE_SHOP_STATE_OPEN,
            page_count: stood.page_count,
        }
        .encode();
        let entity = GcPrivateShopAddEntity {
            x: spot.0,
            y: spot.1,
            z: 0,
            vid: Self::private_vid(vid),
            vnum,
            owner,
            title_type: stood.title_type,
            title: stood.title,
        }
        .encode();
        PrivateShopBuilt {
            shop,
            wares: stored,
            load,
            entity,
        }
    }

    /// Take the wares out of the owner's window, answering the shop's slots and
    /// the store's rows.
    fn private_take_wares(
        &mut self,
        vid: Vid,
        pid: u32,
        wares: &[(u16, world::item::Item, i64, u32)],
        now: i64,
    ) -> (
        [Option<PrivateShopWare>; PRIVATE_SHOP_WINDOW_SLOTS],
        Vec<db::private_shop::ShopItemRow>,
    ) {
        let mut slots: [Option<PrivateShopWare>; PRIVATE_SHOP_WINDOW_SLOTS] =
            std::array::from_fn(|_| None);
        let mut stored = Vec::with_capacity(wares.len());
        for (slot, item, gold, cheque) in wares {
            slots[usize::from(*slot)] = Some(PrivateShopWare {
                item: item.clone(),
                gold: *gold,
                cheque: *cheque,
                checkin: now,
            });
            stored.push(db::private_shop::ShopItemRow {
                item: db_item_row(pid, item),
                pos: *slot,
                gold: *gold,
                cheque: *cheque,
                checkin: now,
            });
            // The ware leaves the owner's window, as `RemoveFromCharacter` takes it
            // (`D/ClientManagerPrivateShop.cpp:1184-1192`).
            if let Ok(character) = self.characters.find_by_vid_mut(vid) {
                let _taken = character.items_mut().release(item.id);
            }
        }
        (slots, stored)
    }

    /// The stand's VID: the owner's, as the viewers address the stand where the
    /// owner stood. Legacy allocates a fresh VID per shop
    /// (`CPrivateShopManager::AllocVID`); the entity table is a later slice, and
    /// the owner's VID is unique on the map already.
    fn private_vid(vid: Vid) -> u32 {
        vid.raw()
    }

    /// `CInputMain::PrivateShopStart` for the character under `vid` naming the stand
    /// `target` (`G/input_main.cpp:4271-4322`): the logout cancel, the trade windows,
    /// the shop found by its stand's VID, the owner's own click opening its panel,
    /// the same window ignored, and the old shop left behind. Answers the viewer's
    /// `START` and the closed panel's `CLOSE_PANEL`, if any.
    pub fn private_start(&mut self, vid: Vid, target: u32, actor: Mover) -> PrivateShopPanel {
        if self.timed_events.remove(&vid).is_some() {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Your logout has been canceled.",
                &[],
            );
            return PrivateShopPanel {
                records: Vec::new(),
            };
        }
        if !self.private_trade_windows_clear(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot view a personal shop while having other trading windows open.",
                &[],
            );
            return PrivateShopPanel {
                records: Vec::new(),
            };
        }
        let Some(owner) = self.private_owner_of_stand(target) else {
            return PrivateShopPanel {
                records: Vec::new(),
            };
        };
        if owner == vid {
            if self.edits_private_shop(vid) {
                return PrivateShopPanel {
                    records: Vec::new(),
                };
            }
            return self.private_panel_open(vid, actor);
        }
        if self.private_viewing.get(&vid) == Some(&owner) {
            return PrivateShopPanel {
                records: Vec::new(),
            };
        }
        let mut records = Vec::new();
        if self.edits_private_shop(vid) {
            records.extend(self.private_panel_close(vid).records);
        }
        if let Some(viewed) = self.private_viewing.remove(&vid) {
            if let Some(viewers) = self.private_viewers.get_mut(&viewed) {
                viewers.retain(|viewer| *viewer != vid);
            }
        }
        records.push(self.private_window(owner));
        self.private_viewing.insert(vid, owner);
        self.private_viewers.entry(owner).or_default().push(vid);
        PrivateShopPanel { records }
    }

    /// `CInputMain::PrivateShopEnd` for the character under `vid`
    /// (`G/input_main.cpp:4324-4330`): the browse stops, as a departure stops it.
    /// Stamps the shop time, as `StopShopping` does
    /// (`G/private_shop_manager.cpp:393-402`).
    pub fn private_end(&mut self, vid: Vid) -> PrivateShopPanel {
        PrivateShopPanel {
            records: self.private_stop_viewing(vid),
        }
    }

    /// Stop whatever shop `vid` views, answering the `END`. `SetMyShopTime` stamps
    /// the portal guard, as `StopShopping` stamps it.
    fn private_stop_viewing(&mut self, vid: Vid) -> Vec<Vec<u8>> {
        let Some(viewed) = self.private_viewing.remove(&vid) else {
            return Vec::new();
        };
        if let Some(viewers) = self.private_viewers.get_mut(&viewed) {
            viewers.retain(|viewer| *viewer != vid);
        }
        self.set_shop_time(vid);
        vec![GcPrivateShop::new(SUBHEADER_GC_PRIVATE_SHOP_END).encode()]
    }

    /// `CInputMain::PrivateShopSearchClose` for the character under `vid`
    /// (`G/input_main.cpp:5132-5138`): the window closes, with no answer.
    pub fn private_search_close(&mut self, vid: Vid) {
        let _closed = self.private_searching.remove(&vid);
    }

    /// `CInputMain::PrivateShopSearch` for the character under `vid`
    /// (`G/input_main.cpp:5140-5176`): stunned or dead is silent, a search within two
    /// seconds of the last refuses with the wait line, and the matching wares answer
    /// as `SEARCH_RESULT`. Nothing matches and nothing is sent, as legacy's empty
    /// answer sends nothing (`G/private_shop_manager.cpp:1080-1093`).
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn private_search(
        &mut self,
        vid: Vid,
        search: &CgPrivateShopSearch,
        actor: Mover,
    ) -> Result<PrivateShopPanel, MoveItemRefused> {
        self.characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        if self.is_stunned(vid.raw()) || self.is_dead(vid.raw()) {
            return Ok(PrivateShopPanel {
                records: Vec::new(),
            });
        }
        let last = self.private_search_at.get(&vid).copied().unwrap_or(0);
        if last != 0 && self.last_pulse.saturating_sub(last) < PRIVATE_SHOP_SEARCH_WAIT_PULSES {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Please wait a moment before searching other personal shops again.",
                &[],
            );
            return Ok(PrivateShopPanel {
                records: Vec::new(),
            });
        }
        self.private_search_at.insert(vid, self.last_pulse);
        let rows = self.private_search_rows(search);
        if rows.is_empty() {
            return Ok(PrivateShopPanel {
                records: Vec::new(),
            });
        }
        Ok(PrivateShopPanel {
            records: vec![GcPrivateShopSearchResult { rows }.encode()],
        })
    }

    /// `CPrivateShopManager::SearchItem` (`G/private_shop_manager.cpp:940-1094`): the
    /// matching wares of every standing shop, in owner-VID and slot order. The rows
    /// stop when the next would overflow the safe bytes, as legacy's break does.
    /// Legacy fans the search out to every core over P2P and merges the answers; one
    /// process holds every Channel (ADR-0002), so the scan already sees them all.
    fn private_search_rows(&self, search: &CgPrivateShopSearch) -> Vec<GcPrivateShopSearchData> {
        let mut owners: Vec<Vid> = self.private_shops.keys().copied().collect();
        owners.sort_by_key(|owner| owner.raw());
        let mut rows = Vec::new();
        let mut bytes = 0;
        if search.vnum != 0 {
            let Some(table) = self.protos.get(search.vnum) else {
                return rows;
            };
            let (item_type, item_subtype) = (table.item_type, table.sub_type);
            for owner in owners {
                for (slot, ware) in self.private_shop_wares(owner) {
                    if bytes + GC_PRIVATE_SHOP_SEARCH_DATA_WIRE_SIZE
                        > PRIVATE_SHOP_SEARCH_SAFE_BYTES
                    {
                        return rows;
                    }
                    let Some(row) = self.private_search_row(
                        search,
                        owner,
                        slot,
                        &ware,
                        item_type,
                        item_subtype,
                    ) else {
                        continue;
                    };
                    bytes += GC_PRIVATE_SHOP_SEARCH_DATA_WIRE_SIZE;
                    rows.push(row);
                }
            }
            return rows;
        }
        if search.owner.iter().any(|byte| *byte != 0) {
            let Some(owner) = self.private_shop_of_owner(&search.owner) else {
                return rows;
            };
            for (slot, ware) in self.private_shop_wares(owner) {
                if bytes + GC_PRIVATE_SHOP_SEARCH_DATA_WIRE_SIZE > PRIVATE_SHOP_SEARCH_SAFE_BYTES {
                    return rows;
                }
                let Some(row) = self.private_search_row(
                    search,
                    owner,
                    slot,
                    &ware,
                    search.item_type,
                    search.item_subtype,
                ) else {
                    continue;
                };
                bytes += GC_PRIVATE_SHOP_SEARCH_DATA_WIRE_SIZE;
                rows.push(row);
            }
            return rows;
        }
        if search.item_type >= 0 {
            for owner in owners {
                for (slot, ware) in self.private_shop_wares(owner) {
                    if bytes + GC_PRIVATE_SHOP_SEARCH_DATA_WIRE_SIZE
                        > PRIVATE_SHOP_SEARCH_SAFE_BYTES
                    {
                        return rows;
                    }
                    let Some(row) = self.private_search_row(
                        search,
                        owner,
                        slot,
                        &ware,
                        search.item_type,
                        search.item_subtype,
                    ) else {
                        continue;
                    };
                    bytes += GC_PRIVATE_SHOP_SEARCH_DATA_WIRE_SIZE;
                    rows.push(row);
                }
            }
        }
        rows
    }

    /// The wares of `owner`'s shop by slot, cloned for the scan.
    fn private_shop_wares(&self, owner: Vid) -> Vec<(u16, PrivateShopWare)> {
        self.private_shops.get(&owner).map_or(Vec::new(), |shop| {
            shop.wares
                .iter()
                .enumerate()
                .filter_map(|(slot, ware)| {
                    ware.clone()
                        .map(|ware| (u16::try_from(slot).unwrap_or(u16::MAX), ware))
                })
                .collect()
        })
    }

    /// The shop whose owner's name the search field names, as
    /// `GetPrivateShopByOwnerName` finds it (`G/private_shop_manager.cpp:54-65`):
    /// the field's bytes up to the first NUL. Later shops win on a repeated name,
    /// as the map walk passes the earlier one.
    fn private_shop_of_owner(&self, field: &[u8; 25]) -> Option<Vid> {
        let end = field
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(field.len());
        let mut owners: Vec<Vid> = self.private_shops.keys().copied().collect();
        owners.sort_by_key(|owner| owner.raw());
        let mut found = None;
        for owner in owners {
            let name = self
                .characters
                .find_by_vid(owner)
                .map_or(String::new(), |character| character.name().to_owned());
            if name.as_bytes() == &field[..end] {
                found = Some(owner);
            }
        }
        found
    }

    /// One ware as the search shows it, or `None` when the filter refuses it
    /// (`FilterItem` and `CopyItemData`, `G/private_shop_manager.cpp:789-938` and
    /// `G/private_shop_util.cpp:223-246`). The type and subtype are the vnum's when
    /// the search names a vnum, and the filter's else, as legacy overwrites them.
    #[allow(clippy::too_many_arguments)]
    fn private_search_row(
        &self,
        search: &CgPrivateShopSearch,
        owner: Vid,
        slot: u16,
        ware: &PrivateShopWare,
        item_type: i32,
        item_subtype: i32,
    ) -> Option<GcPrivateShopSearchData> {
        let item = &ware.item;
        let table = self.protos.get(item.vnum)?;
        if item_type >= 0 && table.item_type != item_type {
            return None;
        }
        if item_subtype >= 0 && table.sub_type != item_subtype {
            return None;
        }
        if search.use_filter && !Self::private_search_filter(search, item, table, item_type) {
            return None;
        }
        let (shop, state) = self
            .private_shops
            .get(&owner)
            .and_then(|shop| {
                self.characters
                    .find_by_vid(owner)
                    .ok()
                    .map(|character| (character.player_id(), shop.state))
            })
            .unwrap_or((0, PRIVATE_SHOP_STATE_OPEN));
        let name = self
            .characters
            .find_by_vid(owner)
            .map_or(String::new(), |character| character.name().to_owned());
        Some(GcPrivateShopSearchData {
            shop,
            owner: private_owner_field(&name),
            vnum: item.vnum,
            gold: ware.gold,
            cheque: ware.cheque,
            count: u32::from(item.count),
            pos: slot,
            sockets: item.sockets,
            attrs: item.attributes.map(|attr| (attr.b_type, attr.s_value)),
            checkin: i32::try_from(ware.checkin).unwrap_or(0),
            state,
        })
    }

    /// `FilterItem` under the flag (`G/private_shop_manager.cpp:806-937`): the vnum,
    /// the refine band on weapons and armours, the alchemy band on dragon souls, the
    /// job and gender, the stack, the level band, and the attributes on weapons and
    /// armours. The sash absorption is read and never judged, as legacy reads it.
    fn private_search_filter(
        search: &CgPrivateShopSearch,
        item: &world::item::Item,
        table: &gamedata::item_proto::ItemProto,
        item_type: i32,
    ) -> bool {
        if search.vnum != 0 && item.vnum != search.vnum {
            return false;
        }
        if item_type == gamedata::item_kind::ITEM_WEAPON
            || item_type == gamedata::item_kind::ITEM_ARMOR
        {
            let refine = item.vnum % 10;
            if search.min_refine > 0 && refine < u32::from(search.min_refine) {
                return false;
            }
            if search.max_refine > 0 && refine > u32::from(search.max_refine) {
                return false;
            }
        }
        if table.item_type == gamedata::item_kind::ITEM_DS {
            let grade = (item.vnum / 1000) % 10;
            let strength = (item.vnum / 10) % 10;
            if u32::from(search.alchemy_clarity) > grade {
                return false;
            }
            if u32::from(search.alchemy_level) > strength {
                return false;
            }
        }
        if !private_search_job_ok(search.job, table.anti_flags) {
            return false;
        }
        if !private_search_gender_ok(search.gender, table.anti_flags) {
            return false;
        }
        if search.min_stack > 0 && item.count < search.min_stack {
            return false;
        }
        if search.max_stack > 0 && item.count > search.max_stack {
            return false;
        }
        let level = private_search_level(table);
        if search.min_level > 0 && level < search.min_level {
            return false;
        }
        if search.max_level > 0 && level > search.max_level {
            return false;
        }
        if item_type != gamedata::item_kind::ITEM_WEAPON
            && item_type != gamedata::item_kind::ITEM_ARMOR
        {
            return true;
        }
        for asked in search.attrs {
            if asked.0 == 0 {
                continue;
            }
            let mut found = false;
            for attr in item.attributes {
                if attr.b_type == asked.0 {
                    found = attr.s_value >= asked.1;
                    break;
                }
            }
            if !found {
                return false;
            }
        }
        true
    }

    /// `CPrivateShopManager::SendShopSearchUpdate`
    /// (`G/private_shop_manager.cpp:713-749`): every searching character hears the
    /// changed shop, its slot and its state. Legacy fans the update out to every core
    /// over P2P; one process holds every Channel (ADR-0002), so the walk already
    /// reaches them all.
    fn private_search_update(&self, shop: u32, pos: i32, state: u8) {
        let record = GcPrivateShopSearchUpdate { shop, pos, state }.encode();
        let searching: Vec<Vid> = self.private_searching.keys().copied().collect();
        for searcher in searching {
            let _sent = self.write_to_client(searcher, record.clone());
        }
    }

    /// `CInputMain::PrivateShopBuy` for the character under `vid` naming shop slot
    /// `pos` (`G/input_main.cpp:4332-4395`), settled at once with the DB checks
    /// (`D/ClientManagerPrivateShop.cpp:1520-1650`), the game transaction
    /// (`G/private_shop_manager.cpp:517-655`) and the DB completion (`:2102-2186`).
    ///
    /// The gates in legacy order: the logout cancel, the one-second wait, the stun,
    /// the viewed shop, and the ware's presence. Then the settlement: no space
    /// refuses with `You don't have enough space in your inventory.`, no gold with
    /// `You don't have enough Yang.`, the ware moves to the buyer's window, the gold
    /// moves to the stash, the viewers hear `REMOVE_ITEM`, the owner hears
    /// `REMOVE_MY_ITEM`, `SET_SALE`, the sold whisper and `BALANCE_UPDATE`, and the
    /// buyer hears the bought line. Answers the buyer's `PrivateShopBought` for the
    /// store, or the refusal's chats as a move.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn private_buy(
        &mut self,
        vid: Vid,
        pos: u16,
        actor: Mover,
    ) -> Result<Result<PrivateShopBought, MovedItems>, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        if self.timed_events.remove(&vid).is_some() {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Your logout has been canceled.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        let last = self.private_buy_at.get(&vid).copied().unwrap_or(0);
        if last != 0 && self.last_pulse.saturating_sub(last) < PRIVATE_SHOP_BUY_WAIT_PULSES {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Please wait a moment before buying from a personal shop again.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        if self.is_stunned(vid.raw()) || self.is_dead(vid.raw()) {
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        let Some(owner) = self.private_viewing.get(&vid).copied() else {
            return Ok(Err(self.private_refused(vid, pid, actor)));
        };
        let at = usize::from(pos);
        let Some(ware) = self
            .private_shops
            .get(&owner)
            .and_then(|shop| shop.wares.get(at).and_then(Clone::clone))
        else {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot buy an item from your own personal shop.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        };
        self.private_buy_at.insert(vid, self.last_pulse);
        Ok(self.private_settle(vid, pid, owner, pos, &ware, actor))
    }

    /// Settle the buy of `ware` at `pos` of `owner`'s shop by the buyer under `vid`.
    fn private_settle(
        &mut self,
        vid: Vid,
        pid: u32,
        owner: Vid,
        pos: u16,
        ware: &PrivateShopWare,
        actor: Mover,
    ) -> Result<PrivateShopBought, MovedItems> {
        let buyer = self
            .characters
            .find_by_vid(vid)
            .expect("the buyer is online");
        let gold = buyer.gold();
        let Some(at) = self.private_buy_cell(vid, &ware.item) else {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You don't have enough space in your inventory.",
                &[],
            );
            return Err(self.private_refused(vid, pid, actor));
        };
        let price = u64::try_from(ware.gold).unwrap_or(u64::MAX);
        if gold < price {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You don't have enough Yang.",
                &[],
            );
            return Err(self.private_refused(vid, pid, actor));
        }
        Ok(self.private_close_sale(vid, pid, owner, pos, ware, at, gold, price, actor))
    }

    /// The buyer's cell for the ware: a dragon soul stone goes to its box, anything
    /// else to the base inventory (`GetEmptyInventory`, `G/private_shop_util.cpp:92-124`).
    fn private_buy_cell(
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
        let usable = usable_inventory_cells(buyer.inven_point());
        buyer
            .items()
            .find_free_inventory_cell(usable, item.size)
            .map(|cell| protocol::item_pos::ItemPos::new(EWindows::Inventory as u8, cell))
    }

    /// Move the ware to the buyer's window, clear the shop's slot, and tell the
    /// viewers, the owner and the buyer, in legacy order.
    #[allow(clippy::too_many_arguments)]
    fn private_close_sale(
        &mut self,
        vid: Vid,
        pid: u32,
        owner: Vid,
        pos: u16,
        ware: &PrivateShopWare,
        at: protocol::item_pos::ItemPos,
        gold: u64,
        price: u64,
        actor: Mover,
    ) -> PrivateShopBought {
        use world::character::{ItemChange, ItemRecord, MoveRecord};
        // The ware moves to the buyer's window, with its price and checkin cleared,
        // as `ItemTransaction` clears them (`:588-593`).
        let mut item = ware.item.clone();
        item.pos = at;
        let left = gold - price;
        let placed_id = item.id;
        {
            let buyer = self
                .characters
                .find_by_vid_mut(vid)
                .expect("the buyer is online");
            let _placed = buyer.items_mut().set(at, &item);
            buyer.set_gold(left);
        }
        // The ware leaves the shop, the stash takes the price, and the viewers and
        // the owner hear the slot clear (`RemoveItem` and `RemovePrivateShopItem`,
        // `G/private_shop.cpp:524-551` and `G/char.cpp:12257-12280`, and
        // `UpdateBalance`, `D/ClientManagerPrivateShop.cpp:2144`).
        if let Some(shop) = self.private_shops.get_mut(&owner) {
            shop.wares[usize::from(pos)] = None;
            shop.gold += ware.gold;
        }
        let viewers = self
            .private_viewers
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        let remove = GcPrivateShopRemove {
            subheader: SUBHEADER_GC_PRIVATE_SHOP_REMOVE_ITEM,
            pos,
        }
        .encode();
        for viewer in &viewers {
            let _sent = self.write_to_client(*viewer, remove.clone());
        }
        let owner_remove = GcPrivateShopRemove {
            subheader: SUBHEADER_GC_PRIVATE_SHOP_REMOVE_MY_ITEM,
            pos,
        }
        .encode();
        let _sent = self.write_to_client(owner, owner_remove);
        // The searchers hear the ware go, as `RemoveItem` tells them
        // (`G/private_shop.cpp:556`).
        let owner_pid = self
            .characters
            .find_by_vid(owner)
            .map_or(0, world::character::Character::player_id);
        self.private_search_update(owner_pid, i32::from(pos), PRIVATE_SHOP_SEARCH_REMOVED);
        self.private_tell_sale(owner, vid, pos, ware, &item);
        // The buyer hears the bought line, as `ItemTransaction` chats it (`:640-643`).
        if let Some(proto) = self.protos.get(item.vnum) {
            let name = String::from_utf8_lossy(&proto.locale_name).into_owned();
            if item.count > 1 {
                self.tell(
                    vid,
                    actor,
                    CHAT_TYPE_INFO,
                    b"You have bought x%d %s.",
                    &[
                        crate::chat_line::Arg::Int(i64::from(item.count)),
                        crate::chat_line::Arg::Text(name.as_bytes()),
                    ],
                );
            } else {
                self.tell(
                    vid,
                    actor,
                    CHAT_TYPE_INFO,
                    b"You have bought %s.",
                    &[crate::chat_line::Arg::Text(name.as_bytes())],
                );
            }
        }
        let mut done = world::character::MoveDone {
            kind: world::character::MoveKind::Bought,
            records: Vec::new(),
            changes: Vec::new(),
        };
        done.records.push(MoveRecord::Gold {
            amount: 0,
            value: left,
        });
        done.records
            .push(MoveRecord::Item(ItemRecord::Set(item.gc_item_set(at, 1))));
        done.changes.push(ItemChange::Created(item));
        let mut bought = MovedItems::new(pid, vid.raw(), done, actor, &self.protos, &self.locale);
        bought.gold = Some(gold_delta(gold, left));
        PrivateShopBought {
            bought,
            ware: placed_id,
            owner: owner_pid,
            price: ware.gold,
        }
    }

    /// Tell the owner the sale: `SET_SALE`, the sold whisper, and `BALANCE_UPDATE`
    /// (`SaleUpdate`, `G/char.cpp:12474-12536`).
    fn private_tell_sale(
        &self,
        owner: Vid,
        vid: Vid,
        pos: u16,
        ware: &PrivateShopWare,
        item: &world::item::Item,
    ) {
        let now = Self::private_now();
        let customer = self
            .characters
            .find_by_vid(vid)
            .map_or(String::new(), |buyer| buyer.name().to_owned());
        let sale = GcPrivateShopSale {
            customer: private_owner_field(&customer),
            time: i32::try_from(now).unwrap_or(0),
            item: GcPrivateShopItemData {
                vnum: item.vnum,
                gold: ware.gold,
                cheque: ware.cheque,
                checkin: i32::try_from(ware.checkin).unwrap_or(0),
                count: u32::from(item.count),
                pos,
                sockets: item.sockets,
                attrs: item.attributes.map(|attr| (attr.b_type, attr.s_value)),
            },
        }
        .encode();
        let _sent = self.write_to_client(owner, sale);
        // `You have sold x%d %s for %s Yangs.`: the prototype's locale name stands
        // for the item name table this build does not load, and the price groups by
        // threes with dots (`format_number`, `G/private_shop_util.h:12-24`).
        if let Some(proto) = self.protos.get(item.vnum) {
            let mut text = format!(
                "You have sold x{} {} for {} Yangs.",
                item.count,
                String::from_utf8_lossy(&proto.locale_name),
                private_grouped(ware.gold)
            )
            .into_bytes();
            text.push(0);
            let mut name_from = [0u8; PRIVATE_SHOP_OWNER_FIELD_SIZE];
            name_from[..12].copy_from_slice(b"Private Shop");
            let whisper = GcWhisper {
                whisper_type: WHISPER_TYPE_SYSTEM,
                name_from,
                can_format: true,
                text,
            };
            if let Ok(line) = whisper.encode() {
                let _sent = self.write_to_client(owner, line);
            }
        }
        let balance = GcPrivateShopBalance {
            gold: ware.gold,
            cheque: ware.cheque,
        }
        .encode();
        let _sent = self.write_to_client(owner, balance);
    }

    /// `CInputMain::PrivateShopClose` for the character under `vid`
    /// (`G/input_main.cpp:4189-4220`), settled at once with the DB close and the game
    /// close (`D/ClientManagerPrivateShop.cpp:1205-1233` and
    /// `G/private_shop_manager.cpp:261-310`): the logout cancel, the stun, the trade
    /// windows, the owner with its panel open, the ten-second wait, the empty stash,
    /// the same map, and the 3000 range. Then the wares move back to the owner's
    /// window biggest-first, the viewers are cleaned, and the owner hears `CLOSE`
    /// with the closed line. Answers the close for the store, or the refusal's chats
    /// as a move.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn private_close(
        &mut self,
        vid: Vid,
        actor: Mover,
    ) -> Result<Result<PrivateShopClosed, MovedItems>, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        if self.timed_events.remove(&vid).is_some() {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Your logout has been canceled.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        if self.is_stunned(vid.raw()) || self.is_dead(vid.raw()) {
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        if !self.private_trade_windows_clear(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot close a personal shop while another window is open.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        if !self.has_private_shop(vid) || !self.edits_private_shop(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You do not have an open personal shop.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        let last = self.private_close_at.get(&vid).copied().unwrap_or(0);
        if last != 0 && self.last_pulse.saturating_sub(last) < PRIVATE_SHOP_CLOSE_WAIT_PULSES {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Please wait a few moments before closing your personal shop again.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        let stash = self.private_shops.get(&vid).map_or(0, |shop| shop.gold);
        if stash != 0 {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You must withdraw your earnings before you can close your personal shop.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        if !self.private_stand_near(vid, actor) {
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        self.private_close_at.insert(vid, self.last_pulse);
        Ok(self.private_close_shop(vid, pid, actor))
    }

    /// Whether the owner stands by its stand: the same map and within 3000, telling
    /// the refusal (`ClosePrivateShop`, `G/private_shop_manager.cpp:272-284`).
    fn private_stand_near(&mut self, vid: Vid, actor: Mover) -> bool {
        let Some(shop) = self.private_shops.get(&vid) else {
            return false;
        };
        let (map, x, y) = shop.at;
        let Some(place) = self.place_of(vid.raw()) else {
            return false;
        };
        if place.1 != map {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot close a personal shop from a different location.",
                &[],
            );
            return false;
        }
        let here = self
            .spot_of(vid.raw())
            .map_or((0, 0), |spot| (spot.x, spot.y));
        if crate::sync_position::distance_approx(x - here.0, y - here.1)
            > PRIVATE_SHOP_CLOSE_DISTANCE
        {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You are too far away from your personal shop.",
                &[],
            );
            return false;
        }
        true
    }

    /// Close the shop: the wares move back to the owner's window, the viewers are
    /// cleaned, and the owner hears `CLOSE` with the closed line.
    fn private_close_shop(
        &mut self,
        vid: Vid,
        pid: u32,
        actor: Mover,
    ) -> Result<PrivateShopClosed, MovedItems> {
        use world::character::{ItemChange, ItemRecord, MoveRecord};
        let Some(shop) = self.private_shops.get(&vid) else {
            return Err(self.private_refused(vid, pid, actor));
        };
        // Biggest first, as `TransferItems` sorts them (`G/private_shop.cpp:218-222`).
        let mut wares: Vec<(u16, PrivateShopWare)> = shop
            .wares
            .iter()
            .enumerate()
            .filter_map(|(slot, ware)| {
                ware.as_ref()
                    .map(|ware| (u16::try_from(slot).unwrap_or(u16::MAX), ware.clone()))
            })
            .collect();
        wares.sort_by_key(|(_, ware)| core::cmp::Reverse(ware.item.size));
        let mut done = world::character::MoveDone {
            kind: world::character::MoveKind::Moved,
            records: Vec::new(),
            changes: Vec::new(),
        };
        let mut placed = Vec::with_capacity(wares.len());
        for (slot, ware) in &wares {
            let Some(at) = self.private_buy_cell(vid, &ware.item) else {
                self.tell(
                    vid,
                    actor,
                    CHAT_TYPE_INFO,
                    b"You don't have enough space in your inventory.",
                    &[],
                );
                return Err(self.private_refused(vid, pid, actor));
            };
            let mut item = ware.item.clone();
            item.pos = at;
            if self
                .characters
                .find_by_vid_mut(vid)
                .expect("the owner is online")
                .items_mut()
                .set(at, &item)
                .is_err()
            {
                self.tell(
                    vid,
                    actor,
                    CHAT_TYPE_INFO,
                    b"You don't have enough space in your inventory.",
                    &[],
                );
                return Err(self.private_refused(vid, pid, actor));
            }
            done.records
                .push(MoveRecord::Item(ItemRecord::Set(item.gc_item_set(at, 0))));
            done.changes.push(ItemChange::Created(item.clone()));
            placed.push((*slot, item));
        }
        // The shop is gone: the viewers hear `END`, the panel closes, and the owner
        // hears `CLOSE` with the closed line (`ClosePrivateShop`,
        // `G/char.cpp:12135-12158`).
        let viewers = self.private_viewers.remove(&vid).unwrap_or_default();
        let end = GcPrivateShop::new(SUBHEADER_GC_PRIVATE_SHOP_END).encode();
        for viewer in viewers {
            self.private_viewing.remove(&viewer);
            let _sent = self.write_to_client(viewer, end.clone());
        }
        self.private_shops.remove(&vid);
        self.private_panels.remove(&vid);
        let mut records = vec![GcPrivateShop::new(SUBHEADER_GC_PRIVATE_SHOP_CLOSE).encode()];
        let mut closed = MovedItems::new(pid, vid.raw(), done, actor, &self.protos, &self.locale);
        records.append(&mut closed.records);
        closed.records = records;
        self.tell(
            vid,
            actor,
            CHAT_TYPE_INFO,
            b"Your personal shop has been closed.",
            &[],
        );
        Ok(PrivateShopClosed {
            closed,
            owner: pid,
            wares: placed.iter().map(|(_, item)| item.id).collect(),
        })
    }

    /// `CInputMain::PrivateShopWithdraw` for the character under `vid`
    /// (`G/input_main.cpp:4398-4450`), settled at once with the DB request and the
    /// game completion (`D/ClientManagerPrivateShop.cpp:1289-1320` and
    /// `G/char.cpp:12598-12671`): the logout cancel, the stun, the owner with its
    /// panel open, the ten-second wait, the gold cap, and the stash. The stash moves
    /// to the gold, the owner hears the withdrawn line and the bare `WITHDRAW`, and
    /// an empty shop is destroyed with it. Answers the withdraw for the store, or
    /// the refusal's chats as a move.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn private_withdraw(
        &mut self,
        vid: Vid,
        actor: Mover,
    ) -> Result<Result<PrivateShopWithdrawn, MovedItems>, MoveItemRefused> {
        use world::character::MoveRecord;
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        let gold = character.gold();
        let Some(stash) = self.private_withdraw_gates(vid, pid, gold, actor) else {
            return Ok(Err(self.private_refused(vid, pid, actor)));
        };
        self.private_close_at.insert(vid, self.last_pulse);
        let held = gold + u64::try_from(stash).unwrap_or(0);
        if let Ok(owner) = self.characters.find_by_vid_mut(vid) {
            owner.set_gold(held);
        }
        if let Some(shop) = self.private_shops.get_mut(&vid) {
            shop.gold = 0;
        }
        self.tell(
            vid,
            actor,
            CHAT_TYPE_INFO,
            b"You have withdrawn %lld Yang from your personal shop earnings.",
            &[crate::chat_line::Arg::Int(stash)],
        );
        let mut done = world::character::MoveDone {
            kind: world::character::MoveKind::Moved,
            records: Vec::new(),
            changes: Vec::new(),
        };
        done.records.push(MoveRecord::Gold {
            amount: stash,
            value: held,
        });
        let mut withdrawn =
            MovedItems::new(pid, vid.raw(), done, actor, &self.protos, &self.locale);
        withdrawn.gold = Some(gold_delta(gold, held));
        // The bare `WITHDRAW` goes out with the gold.
        let mut records = vec![GcPrivateShop::new(SUBHEADER_GC_PRIVATE_SHOP_WITHDRAW).encode()];
        records.append(&mut withdrawn.records);
        withdrawn.records = records;
        // No wares left: the shop is destroyed with the withdraw
        // (`PrivateShopWithdraw`, `D/ClientManagerPrivateShop.cpp:1435-1456`).
        let empty = self
            .private_shops
            .get(&vid)
            .is_some_and(|shop| shop.wares.iter().all(Option::is_none));
        Ok(Ok(PrivateShopWithdrawn {
            withdrawn,
            owner: pid,
            destroy: empty,
        }))
    }

    /// The withdraw's gates in legacy order, answering the stash: the logout cancel,
    /// the stun, the owner with its panel open, the ten-second wait, the gold cap with
    /// the input line, the stash, and the cap again with the completion's line.
    /// `None` after telling the refusal.
    fn private_withdraw_gates(
        &mut self,
        vid: Vid,
        pid: u32,
        gold: u64,
        actor: Mover,
    ) -> Option<i64> {
        if self.timed_events.remove(&vid).is_some() {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Your logout has been canceled.",
                &[],
            );
            return None;
        }
        if self.is_stunned(vid.raw()) || self.is_dead(vid.raw()) {
            return None;
        }
        if !self.has_private_shop(vid) || !self.edits_private_shop(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You do not have an open personal shop.",
                &[],
            );
            return None;
        }
        let last = self.private_close_at.get(&vid).copied().unwrap_or(0);
        if last != 0 && self.last_pulse.saturating_sub(last) < PRIVATE_SHOP_CLOSE_WAIT_PULSES {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Please wait a few moments before withdrawing your personal shop again.",
                &[],
            );
            return None;
        }
        let stash = self.private_shops.get(&vid).map_or(0, |shop| shop.gold);
        if gold.saturating_add(u64::try_from(stash).unwrap_or(u64::MAX))
            >= common::constants::GOLD_MAX_MAX
        {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot exchange as you would exceed the maximum amount of Yang.",
                &[],
            );
            return None;
        }
        if stash == 0 {
            return None;
        }
        // The completion re-checks the cap with its own line (`WithdrawPrivateShop`,
        // `G/char.cpp:12606-12613`); with one world the two checks read the same gold,
        // so this arm never fires, but it is the completion's own words.
        if gold.saturating_add(u64::try_from(stash).unwrap_or(u64::MAX))
            >= common::constants::GOLD_MAX_MAX
        {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot collect the money, because you would then have more than 2 billion Yang.",
                &[],
            );
            return None;
        }
        let _ = pid;
        Some(stash)
    }

    /// `CInputMain::PrivateShopModify` for the character under `vid`
    /// (`G/input_main.cpp:4452-4480`), settled at once with the DB flip and the state
    /// update (`D/ClientManagerPrivateShop.cpp:1457-1515` and
    /// `G/char.cpp:12577-12596`): the stun, the owner with its panel open, and the
    /// one-second wait. The state flips between open and modify, and the owner hears
    /// `STATE_UPDATE` as its own. Answers the flip for the store.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn private_modify(
        &mut self,
        vid: Vid,
        actor: Mover,
    ) -> Result<Result<PrivateShopFlipped, MovedItems>, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        if self.is_stunned(vid.raw()) || self.is_dead(vid.raw()) {
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        if !self.has_private_shop(vid) || !self.edits_private_shop(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You do not have an open private shop.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        let last = self.private_state_at.get(&vid).copied().unwrap_or(0);
        if last != 0 && self.last_pulse.saturating_sub(last) < PRIVATE_SHOP_STATE_WAIT_PULSES {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Please wait a moment before changing state of your personal shop again.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        self.private_state_at.insert(vid, self.last_pulse);
        let state = self
            .private_shops
            .get(&vid)
            .map_or(PRIVATE_SHOP_STATE_OPEN, |shop| {
                if shop.state == PRIVATE_SHOP_STATE_OPEN {
                    PRIVATE_SHOP_STATE_MODIFY
                } else {
                    PRIVATE_SHOP_STATE_OPEN
                }
            });
        if let Some(shop) = self.private_shops.get_mut(&vid) {
            shop.state = state;
        }
        let update = GcPrivateShopState { state, own: true }.encode();
        let _sent = self.write_to_client(vid, update);
        // The searchers hear the whole shop change, as `SetState` tells them
        // (`G/private_shop.cpp:696-698`).
        self.private_search_update(pid, -1, state);
        Ok(Ok(PrivateShopFlipped { owner: pid, state }))
    }

    /// The owner whose stand carries `target`, or `None` when no stand does.
    fn private_owner_of_stand(&self, target: u32) -> Option<Vid> {
        self.private_shops
            .keys()
            .find(|owner| Self::private_vid(**owner) == target)
            .copied()
    }

    /// The window of `owner`'s shop as a `START` record, with the wares' rows in
    /// slot order and empty rows elsewhere (`CPrivateShop::AddShopViewer`,
    /// `G/private_shop.cpp:739-789`).
    fn private_window(&self, owner: Vid) -> Vec<u8> {
        let shop = self.private_shops.get(&owner).expect("the shop stands");
        let mut items = [GcPrivateShopItemData::empty(); PRIVATE_SHOP_WINDOW_SLOTS];
        for (slot, ware) in shop.wares.iter().enumerate() {
            let Some(ware) = ware else { continue };
            let item = &ware.item;
            items[slot] = GcPrivateShopItemData {
                vnum: item.vnum,
                gold: ware.gold,
                cheque: ware.cheque,
                checkin: i32::try_from(ware.checkin).unwrap_or(0),
                count: u32::from(item.count),
                pos: u16::try_from(slot).unwrap_or(u16::MAX),
                sockets: item.sockets,
                attrs: item.attributes.map(|attr| (attr.b_type, attr.s_value)),
            };
        }
        GcPrivateShopStart {
            title: shop.title,
            state: shop.state,
            page_count: shop.page_count,
            unlocked_slots: 0,
            vid: Self::private_vid(owner),
            items,
        }
        .encode()
    }

    /// The total in yang a price change or checkin must stay under
    /// (`GetPrivateShopTotalGold`, `G/char.cpp:12222-12230`): the stash plus every
    /// ware's price. The store's stash and the world's read the same row, so the
    /// world answers it.
    fn private_total_gold(&self, who: Vid) -> i64 {
        self.private_shops.get(&who).map_or(0, |shop| {
            shop.wares
                .iter()
                .flatten()
                .fold(shop.gold, |total, ware| total.saturating_add(ware.gold))
        })
    }

    /// `CanHandleItem` (`G/char_item.cpp:213-247`) for a move, checkin or checkout:
    /// an open own bundle shop, a refine, an open cube, or this build's open sash,
    /// changelook or aura window refuses silently, as legacy returns without a line.
    /// The observer, dragon-soul refine and warp arms have no state in this build
    /// (the Rewrite warps synchronously), so they never refuse.
    fn private_can_handle_item(&self, vid: Vid) -> bool {
        !(self.sash_windows.contains_key(&vid)
            || self.changelook_windows.contains(&vid)
            || self.aura_windows.contains_key(&vid)
            || self.under_refine(vid)
            || self.has_my_shop(vid)
            || self.cubes.contains_key(&vid))
    }

    /// The shared gates of the four edits in legacy order: the logout cancel, the
    /// trade windows with the modify line, the owner with its panel open, the
    /// modify state, and the one-second wait. `None` after telling the refusal, or
    /// for the silent stun. The caller's own gates (the stun or `CanHandleItem`,
    /// the cap and the ware) sit before or after, as legacy orders them.
    fn private_edit_gates(&mut self, vid: Vid, actor: Mover, pid: u32) -> Option<()> {
        if self.timed_events.remove(&vid).is_some() {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Your logout has been canceled.",
                &[],
            );
            return None;
        }
        if !self.private_trade_windows_clear(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot modify a personal shop while having other trading windows open.",
                &[],
            );
            return None;
        }
        if !self.has_private_shop(vid) || !self.edits_private_shop(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You do not have an open personal shop.",
                &[],
            );
            return None;
        }
        if self
            .private_shops
            .get(&vid)
            .is_none_or(|shop| shop.state != PRIVATE_SHOP_STATE_MODIFY)
        {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot manage personal shop's content while it is not in a modifying state.",
                &[],
            );
            return None;
        }
        let last = self.private_state_at.get(&vid).copied().unwrap_or(0);
        if last != 0 && self.last_pulse.saturating_sub(last) < PRIVATE_SHOP_STATE_WAIT_PULSES {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Please wait a moment before editing your personal shop's content again.",
                &[],
            );
            return None;
        }
        let _ = pid;
        Some(())
    }

    /// Tell the viewers around the owner's stand one record.
    fn private_tell_viewers(&mut self, owner: Vid, record: &[u8]) {
        let viewers = self
            .private_viewers
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        for viewer in viewers {
            let _sent = self.write_to_client(viewer, record.to_vec());
        }
    }

    /// `CInputMain::PrivateShopItemPriceChange` for the character under `vid`
    /// (`G/input_main.cpp:4481-4569`), settled at once with the DB price and the
    /// two syncs (`D/ClientManagerPrivateShop.cpp:1653-1715` and the game price,
    /// `G/char.cpp:12317-12346` with `G/private_shop.cpp:597-631`): the stun, the
    /// edit's gates, the ware, the cap, and the negative price's silent drop. The
    /// owner and the viewers around the stand hear the price. Answers the change
    /// for the store, or `None` after telling the refusal.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn private_price_change(
        &mut self,
        vid: Vid,
        actor: Mover,
        pos: u16,
        gold: i64,
        cheque: u32,
    ) -> Result<Option<PrivateShopPriceChanged>, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        let held = character.gold();
        if self.is_stunned(vid.raw()) || self.is_dead(vid.raw()) {
            return Ok(None);
        }
        if self.private_edit_gates(vid, actor, pid).is_none() {
            return Ok(None);
        }
        let Some(ware) = self
            .private_shops
            .get(&vid)
            .and_then(|shop| shop.wares.get(usize::from(pos)))
            .and_then(|ware| ware.as_ref())
        else {
            return Ok(None);
        };
        let old = ware.gold;
        // The cap reads the stash, every ware's price, and the held gold, with the
        // old price out and the new one in (`:4524-4528`).
        let total = self
            .private_total_gold(vid)
            .saturating_sub(old)
            .saturating_add(gold)
            .saturating_add(i64::try_from(held).unwrap_or(i64::MAX));
        if total >= i64::try_from(common::constants::GOLD_MAX_MAX).unwrap_or(i64::MAX) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"The items you put up for sale must not exceed the permitted total value.",
                &[],
            );
            return Ok(None);
        }
        if gold < 0 {
            return Ok(None);
        }
        if let Some(shop) = self.private_shops.get_mut(&vid) {
            if let Some(ware) = shop
                .wares
                .get_mut(usize::from(pos))
                .and_then(|ware| ware.as_mut())
            {
                ware.gold = gold;
                ware.cheque = cheque;
            }
        }
        self.private_state_at.insert(vid, self.last_pulse);
        let record = GcPrivateShopPriceChange { pos, gold, cheque }.encode();
        let _sent = self.write_to_client(vid, record.clone());
        self.private_tell_viewers(vid, &record);
        Ok(Some(PrivateShopPriceChanged {
            owner: pid,
            pos,
            gold,
            cheque,
        }))
    }

    /// `CInputMain::PrivateShopItemMove` for the character under `vid`
    /// (`G/input_main.cpp:4570-4649`), settled at once with the DB move and the two
    /// syncs (`D/ClientManagerPrivateShop.cpp:1716-1778` and the game move,
    /// `G/char.cpp:12347-12373` with `G/private_shop.cpp:634-671`): the stun,
    /// `CanHandleItem`, the edit's gates, the ware, and the grid's answer. A move
    /// past the window or onto a taken cell tells the cannot-move line, as the DB's
    /// `nullptr` chats it (`G/input_db.cpp:2734-2740`). Otherwise the owner and the
    /// viewers around the stand hear the move. Answers the move for the store, or
    /// `None` after telling the refusal.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn private_move(
        &mut self,
        vid: Vid,
        actor: Mover,
        pos: u16,
        change_pos: u16,
    ) -> Result<Option<PrivateShopMoved>, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        if self.is_stunned(vid.raw()) || self.is_dead(vid.raw()) {
            return Ok(None);
        }
        if !self.private_can_handle_item(vid) {
            return Ok(None);
        }
        if self.private_edit_gates(vid, actor, pid).is_none() {
            return Ok(None);
        }
        let at = usize::from(pos);
        let to = usize::from(change_pos);
        let Some(ware) = self
            .private_shops
            .get(&vid)
            .and_then(|shop| shop.wares.get(at))
            .and_then(Clone::clone)
        else {
            return Ok(None);
        };
        let size = self
            .protos
            .get(ware.item.vnum)
            .map_or(0, |proto| proto.size);
        // The DB lifts the ware out of its old cell before trying the new one
        // (`D/PrivateShop.cpp:377-385`); the cells the others hold grid the answer.
        let placed: Vec<(usize, i32)> = self
            .private_shops
            .get(&vid)
            .map(|shop| {
                shop.wares
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, ware)| {
                        let ware = ware.as_ref()?;
                        (slot != at).then(|| {
                            (
                                slot,
                                self.protos
                                    .get(ware.item.vnum)
                                    .map_or(0, |proto| proto.size),
                            )
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        if to >= PRIVATE_SHOP_WINDOW_SLOTS || !Self::private_cell_free(&placed, to, size) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"@@(input_db)tradus:You cannot move the item to that position.",
                &[],
            );
            return Ok(None);
        }
        if let Some(shop) = self.private_shops.get_mut(&vid) {
            shop.wares[at] = None;
            shop.wares[to] = Some(ware);
        }
        self.private_state_at.insert(vid, self.last_pulse);
        let record = GcPrivateShopMove { pos, change_pos }.encode();
        let _sent = self.write_to_client(vid, record.clone());
        self.private_tell_viewers(vid, &record);
        Ok(Some(PrivateShopMoved {
            owner: pid,
            pos,
            change_pos,
        }))
    }

    /// `CInputMain::PrivateShopItemCheckin` for the character under `vid`
    /// (`G/input_main.cpp:4651-4779`), settled at once with the DB placement and
    /// the two syncs (`D/ClientManagerPrivateShop.cpp:1780-1886` with the game
    /// checkin, `G/private_shop_manager.cpp:405-447`, and the shop-side checkin,
    /// `G/private_shop.cpp:559-595`): the stun, `CanHandleItem`, the edit's gates,
    /// the cap, the negative price's silent drop, the held item, its owner, its
    /// proto, the item-shop, worn and locked refusals, and the grid's answer. The
    /// picked slot stands when it is free, else the first free one across the two
    /// pages, else the no-space line. The ware leaves the window, the owner hears
    /// `SET_ITEM` and the viewers around the stand `ADD_ITEM`. Answers the checkin
    /// for the store, or `None` after telling the refusal.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    ///
    /// # Panics
    ///
    /// When the shop the gates proved standing is gone; the caller runs the gates
    /// first, so the shop is there.
    #[allow(clippy::too_many_lines)]
    pub fn private_checkin(
        &mut self,
        vid: Vid,
        actor: Mover,
        src: protocol::item_pos::ItemPos,
        gold: i64,
        cheque: u32,
        dst_pos: i32,
    ) -> Result<Option<PrivateShopCheckedIn>, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        let held = character.gold();
        if self.is_stunned(vid.raw()) || self.is_dead(vid.raw()) {
            return Ok(None);
        }
        if !self.private_can_handle_item(vid) {
            return Ok(None);
        }
        if self.private_edit_gates(vid, actor, pid).is_none() {
            return Ok(None);
        }
        let total = self
            .private_total_gold(vid)
            .saturating_add(i64::try_from(held).unwrap_or(i64::MAX))
            .saturating_add(gold);
        if total >= i64::try_from(common::constants::GOLD_MAX_MAX).unwrap_or(i64::MAX) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"The items you put up for sale must not exceed the permitted total value.",
                &[],
            );
            return Ok(None);
        }
        if gold < 0 {
            return Ok(None);
        }
        let Some(item) = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(src).cloned())
        else {
            return Ok(None);
        };
        let proto = self.protos.get(item.vnum);
        let Some(proto) = proto else {
            return Ok(None);
        };
        if proto.anti_flags & (ITEM_ANTIFLAG_GIVE | ITEM_ANTIFLAG_MYSHOP) != 0 {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot sell Item-Shop items in a personal shop.",
                &[],
            );
            return Ok(None);
        }
        if src.window_type == EWindows::Equipment as u8 {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot sell equipped items in a personal shop.",
                &[],
            );
            return Ok(None);
        }
        if self.sash_item_at(vid, src)
            || self.changelook_item_at(vid, src)
            || self.aura_item_at(vid, src)
        {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot sell locked items in a personal shop.",
                &[],
            );
            return Ok(None);
        }
        let shop = self.private_shops.get(&vid).expect("the shop stands");
        let placed: Vec<(usize, i32)> = shop
            .wares
            .iter()
            .enumerate()
            .filter_map(|(slot, ware)| {
                let ware = ware.as_ref()?;
                Some((
                    slot,
                    self.protos
                        .get(ware.item.vnum)
                        .map_or(0, |proto| proto.size),
                ))
            })
            .collect();
        // The picked slot stands when it names a free one, else the first free one
        // across the two pages (`D/ClientManagerPrivateShop.cpp:1841-1876`).
        let pick = usize::try_from(dst_pos).unwrap_or(usize::MAX);
        let mut found = (pick < PRIVATE_SHOP_WINDOW_SLOTS
            && Self::private_cell_free(&placed, pick, proto.size))
        .then_some(pick);
        if found.is_none() {
            found = (0..PRIVATE_SHOP_WINDOW_SLOTS)
                .find(|&slot| Self::private_cell_free(&placed, slot, proto.size));
        }
        let Some(at) = found else {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"No available space in your personal shop.",
                &[],
            );
            return Ok(None);
        };
        let pos = u16::try_from(at).unwrap_or(u16::MAX);
        let now = Self::private_now();
        let ware = PrivateShopWare {
            item: item.clone(),
            gold,
            cheque,
            checkin: now,
        };
        if let Some(shop) = self.private_shops.get_mut(&vid) {
            shop.wares[at] = Some(ware.clone());
        }
        // The ware leaves the owner's window, as the game checkin removes it
        // (`G/private_shop_manager.cpp:444-446`).
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            let _taken = character.items_mut().release(item.id);
        }
        self.private_state_at.insert(vid, self.last_pulse);
        let row = GcPrivateShopItemData {
            vnum: item.vnum,
            gold,
            cheque,
            checkin: i32::try_from(now).unwrap_or(0),
            count: u32::from(item.count),
            pos,
            sockets: item.sockets,
            attrs: item.attributes.map(|attr| (attr.b_type, attr.s_value)),
        };
        let set = GcPrivateShopSetItem {
            subheader: SUBHEADER_GC_PRIVATE_SHOP_SET_ITEM,
            item: row,
        }
        .encode();
        let _sent = self.write_to_client(vid, set);
        let add = GcPrivateShopSetItem {
            subheader: SUBHEADER_GC_PRIVATE_SHOP_ADD_ITEM,
            item: row,
        }
        .encode();
        self.private_tell_viewers(vid, &add);
        Ok(Some(PrivateShopCheckedIn {
            owner: pid,
            ware: db::private_shop::ShopItemRow {
                item: db_item_row(pid, &item),
                pos,
                gold,
                cheque,
                checkin: now,
            },
        }))
    }

    /// `CInputMain::PrivateShopItemCheckout` for the character under `vid`
    /// (`G/input_main.cpp:4781-4961`), settled at once with the DB handoff and
    /// the two syncs (`D/ClientManagerPrivateShop.cpp:1887-1955` with the game
    /// checkout, `G/private_shop_manager.cpp:449-520`, and the shop-side remove,
    /// `G/private_shop.cpp:524-551`): `CanHandleItem`, the trade windows, the
    /// logout cancel, the owner with its panel open, the modify state, the
    /// one-second wait, the ware, and the landing cell. The picked cell stands
    /// when it names a free one, else the first free one, else the no-space line;
    /// a dragon soul stone lands in its box, anything else in the base inventory.
    /// The ware lands in the window, the viewers around the stand hear the slot's
    /// clear, and the owner's move carries the clear and the set. An empty shop
    /// with no stash is destroyed with the checkout. Answers the checkout for the
    /// store, or `None` after telling the refusal.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    ///
    /// # Panics
    ///
    /// When the owner the gates proved online is gone; the caller runs the gates
    /// first, so the owner is there.
    #[allow(clippy::too_many_lines)]
    pub fn private_checkout(
        &mut self,
        vid: Vid,
        actor: Mover,
        src_pos: u16,
        dst_pos: i32,
    ) -> Result<Result<PrivateShopCheckedOut, MovedItems>, MoveItemRefused> {
        use world::character::{ItemChange, ItemRecord, MoveRecord};
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        if self.is_stunned(vid.raw()) || self.is_dead(vid.raw()) {
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        if !self.private_can_handle_item(vid) {
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        // The checkout orders the trade windows before the logout cancel
        // (`:4797-4816`), unlike its three siblings.
        if !self.private_trade_windows_clear(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot modify a personal shop while having other trading windows open.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        if self.timed_events.remove(&vid).is_some() {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Your logout has been canceled.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        if !self.has_private_shop(vid) || !self.edits_private_shop(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You do not have an open personal shop.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        if self
            .private_shops
            .get(&vid)
            .is_none_or(|shop| shop.state != PRIVATE_SHOP_STATE_MODIFY)
        {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot manage personal shop's content while it is not in a modifying state.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        let last = self.private_state_at.get(&vid).copied().unwrap_or(0);
        if last != 0 && self.last_pulse.saturating_sub(last) < PRIVATE_SHOP_STATE_WAIT_PULSES {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Please wait a moment before editing your personal shop's content again.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        let at = usize::from(src_pos);
        let Some(ware) = self
            .private_shops
            .get(&vid)
            .and_then(|shop| shop.wares.get(at))
            .and_then(Clone::clone)
        else {
            return Ok(Err(self.private_refused(vid, pid, actor)));
        };
        let proto = self.protos.get(ware.item.vnum);
        if proto.is_none() {
            return Ok(Err(self.private_refused(vid, pid, actor)));
        }
        let is_soul = proto.is_some_and(|proto| proto.item_type == gamedata::item_kind::ITEM_DS);
        let Some(land) = self.private_checkout_cell(vid, &ware.item, is_soul, dst_pos) else {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You don't have enough space in your inventory.",
                &[],
            );
            return Ok(Err(self.private_refused(vid, pid, actor)));
        };
        // The ware lands in the window with its price cleared, as the game
        // checkout binds it back (`G/private_shop_manager.cpp:478-498`).
        let mut item = ware.item.clone();
        item.pos = land;
        let placed_id = item.id;
        let window_type = land.window_type;
        let cell = land.cell;
        {
            let owner = self
                .characters
                .find_by_vid_mut(vid)
                .expect("the owner is online");
            let _placed = owner.items_mut().set(land, &item);
        }
        if let Some(shop) = self.private_shops.get_mut(&vid) {
            shop.wares[at] = None;
        }
        self.private_state_at.insert(vid, self.last_pulse);
        let remove = GcPrivateShopRemove {
            subheader: SUBHEADER_GC_PRIVATE_SHOP_REMOVE_ITEM,
            pos: src_pos,
        }
        .encode();
        self.private_tell_viewers(vid, &remove);
        // The searchers hear the ware go, as the store's `RemoveItem` tells them
        // (`D/ClientManagerPrivateShop.cpp:1395`).
        self.private_search_update(pid, i32::from(src_pos), PRIVATE_SHOP_SEARCH_REMOVED);
        let mut done = world::character::MoveDone {
            kind: world::character::MoveKind::Moved,
            records: Vec::new(),
            changes: Vec::new(),
        };
        done.records
            .push(MoveRecord::Item(ItemRecord::Set(item.gc_item_set(land, 1))));
        done.changes.push(ItemChange::Created(item));
        let mut moved = MovedItems::new(pid, vid.raw(), done, actor, &self.protos, &self.locale);
        // The owner's clear goes out first, as `RemovePrivateShopItem` syncs it
        // (`G/char.cpp:12257-12280`).
        let mut records = vec![GcPrivateShopRemove {
            subheader: SUBHEADER_GC_PRIVATE_SHOP_REMOVE_MY_ITEM,
            pos: src_pos,
        }
        .encode()];
        records.append(&mut moved.records);
        moved.records = records;
        // No wares and no stash: the shop is destroyed with the checkout
        // (`D/ClientManagerPrivateShop.cpp:1420-1430`).
        let empty = self
            .private_shops
            .get(&vid)
            .is_some_and(|shop| shop.gold == 0 && shop.wares.iter().all(Option::is_none));
        Ok(Ok(PrivateShopCheckedOut {
            moved,
            owner: pid,
            ware: placed_id,
            window_type,
            pos: cell,
            destroy: empty,
        }))
    }

    /// The checkout's landing cell: the picked cell when it names a free one, else
    /// the first free one (`G/input_main.cpp:4859-4937`, with the special windows
    /// compiled out). A dragon soul stone lands in its box, anything else in the
    /// base inventory. `None` when neither holds the ware.
    fn private_checkout_cell(
        &self,
        vid: Vid,
        item: &world::item::Item,
        is_soul: bool,
        dst_pos: i32,
    ) -> Option<protocol::item_pos::ItemPos> {
        let owner = self.characters.find_by_vid(vid).ok()?;
        let window = if is_soul {
            EWindows::DragonSoulInventory as u8
        } else {
            EWindows::Inventory as u8
        };
        let pick = u16::try_from(dst_pos)
            .ok()
            .map(|cell| protocol::item_pos::ItemPos::new(window, cell));
        if let Some(pick) = pick {
            let free = if is_soul {
                owner.items().item_at(pick).is_none()
            } else {
                let usable = usable_inventory_cells(owner.inven_point());
                owner
                    .items()
                    .is_empty_item_grid(pick, item.size, None, usable, None)
            };
            if dst_pos >= 0 && free {
                return Some(pick);
            }
        }
        if is_soul {
            let qualified = self.ds_qualified(vid);
            let graded = self.protos.get(item.vnum).map_or(0, |proto| proto.sub_type);
            return world::character::dragon_soul_buy_cell(owner.items(), item, graded, qualified);
        }
        let usable = usable_inventory_cells(owner.inven_point());
        owner
            .items()
            .find_free_inventory_cell(usable, item.size)
            .map(|cell| protocol::item_pos::ItemPos::new(EWindows::Inventory as u8, cell))
    }

    /// `CInputMain::PrivateShopTitleChange` for the character under `vid`
    /// (`G/input_main.cpp:4961-5005`), settled at once with the DB rename and the
    /// two syncs (`D/ClientManagerPrivateShop.cpp:1957-2012` and the game rename,
    /// `G/char.cpp:12375-12390` with `G/private_shop.cpp:674-697`): the stun, the
    /// owner with its panel open, the modify state, the short title, and the
    /// one-second wait. No banword check runs, unlike the build. The title is
    /// copied at 32 bytes, as `strncpy` copies it. The owner hears
    /// `TITLE_CHANGE`, the viewers around the stand `TITLE`. Answers the rename
    /// for the store, or `None` after telling the refusal.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn private_rename(
        &mut self,
        vid: Vid,
        actor: Mover,
        title: [u8; CG_PRIVATE_SHOP_TITLE_FIELD_SIZE],
    ) -> Result<Option<PrivateShopRenamed>, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        if self.is_stunned(vid.raw()) || self.is_dead(vid.raw()) {
            return Ok(None);
        }
        if !self.has_private_shop(vid) || !self.edits_private_shop(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You do not have an open personal shop.",
                &[],
            );
            return Ok(None);
        }
        if self
            .private_shops
            .get(&vid)
            .is_none_or(|shop| shop.state != PRIVATE_SHOP_STATE_MODIFY)
        {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot manage personal shop's content while it is not in a modifying state.",
                &[],
            );
            return Ok(None);
        }
        let end = title
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(title.len());
        if end < PRIVATE_SHOP_TITLE_MIN_LEN {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"The entered name is too short.",
                &[],
            );
            return Ok(None);
        }
        let last = self.private_state_at.get(&vid).copied().unwrap_or(0);
        if last != 0 && self.last_pulse.saturating_sub(last) < PRIVATE_SHOP_STATE_WAIT_PULSES {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Please wait a moment before editing your personal shop's content again.",
                &[],
            );
            return Ok(None);
        }
        self.private_state_at.insert(vid, self.last_pulse);
        let mut renamed = [0u8; CG_PRIVATE_SHOP_TITLE_FIELD_SIZE];
        renamed[..32.min(title.len())].copy_from_slice(&title[..32.min(title.len())]);
        let title_type = self
            .private_shops
            .get(&vid)
            .map_or(0, |shop| shop.title_type);
        if let Some(shop) = self.private_shops.get_mut(&vid) {
            shop.title = renamed;
        }
        let changed = GcPrivateShopTitleChange { title: renamed }.encode();
        let _sent = self.write_to_client(vid, changed);
        let titled = GcPrivateShopTitle {
            vid: Self::private_vid(vid),
            title: renamed,
            title_type,
        }
        .encode();
        // `PacketAround` from the stand (`G/private_shop.cpp:687-696`): the owner
        // stands where the stand does, so the owner's key sees the same viewers.
        self.packet_around(
            crate::game_state::view::EntityKey::Character(vid.raw()),
            &titled,
            None,
        );
        Ok(Some(PrivateShopRenamed {
            owner: pid,
            title: renamed,
        }))
    }

    /// `CInputMain::PrivateShopSlotUnlockRequest` for the character under `vid`
    /// (`G/input_main.cpp:5047-5135`), settled at once with the DB unlock and the
    /// completion (`D/ClientManagerPrivateShop.cpp:2060-2100` and
    /// `G/input_db.cpp:2803-2822`): the stun, `CanHandleItem`, the trade windows,
    /// the logout cancel, the modify state when the panel edits, the key at the
    /// named cell, and the locked count. The key's proto value is the slots it
    /// opens, clamped to what is left. With a stand standing the shop's count
    /// grows and the viewers hear it; without one the point grows and the owner
    /// hears the increased line. The key spends either way. Answers the unlock
    /// for the store, or `None` after telling the refusal.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    #[allow(clippy::too_many_lines)]
    pub fn private_unlock(
        &mut self,
        vid: Vid,
        actor: Mover,
        pos: u16,
    ) -> Result<Option<PrivateShopUnlocked>, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        if self.is_stunned(vid.raw()) || self.is_dead(vid.raw()) {
            return Ok(None);
        }
        if !self.private_can_handle_item(vid) {
            return Ok(None);
        }
        if !self.private_trade_windows_clear(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot modify a personal shop while having other trading windows open.",
                &[],
            );
            return Ok(None);
        }
        if self.timed_events.remove(&vid).is_some() {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Your logout has been canceled.",
                &[],
            );
            return Ok(None);
        }
        // The gate reads the panel, not the ownership: an owner whose panel is
        // closed unlocks without the modify state (`:5082-5086`).
        if self.edits_private_shop(vid)
            && self
                .private_shops
                .get(&vid)
                .is_none_or(|shop| shop.state != PRIVATE_SHOP_STATE_MODIFY)
        {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot manage personal shop's content while it is not in a modifying state.",
                &[],
            );
            return Ok(None);
        }
        let at = protocol::item_pos::ItemPos::new(EWindows::Inventory as u8, pos);
        let key = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at).cloned());
        let Some(key) = key else {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot unlock a slot on your personal shop without Personal Shop Key.",
                &[],
            );
            return Ok(None);
        };
        if key.vnum != PRIVATE_SHOP_SLOT_UNLOCK_VNUM {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot unlock a slot on your personal shop without Personal Shop Key.",
                &[],
            );
            return Ok(None);
        }
        let unlocked = self.private_unlocked_slots(vid);
        if unlocked >= PRIVATE_SHOP_LOCKED_SLOT_MAX_NUM {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You cannot unlock any more slots.",
                &[],
            );
            return Ok(None);
        }
        let opens = self
            .protos
            .get(key.vnum)
            .map_or(0, |proto| proto.values[0].max(0));
        let opens = u16::try_from(opens).unwrap_or(u16::MAX);
        let room = PRIVATE_SHOP_LOCKED_SLOT_MAX_NUM - unlocked;
        let opens = opens.min(room);
        let slots = unlocked + opens;
        let standing = self.has_private_shop(vid);
        if standing {
            if let Some(shop) = self.private_shops.get_mut(&vid) {
                shop.unlocked = slots;
            }
        }
        // The key spends either way: one key, as `RemoveSpecifyItem` spends it
        // (`G/input_db.cpp:2811` and `G/input_main.cpp:5126`).
        let moved = self.private_spend_key_and_grow(vid, actor, pid, key.id, slots);
        if standing {
            let record = GcPrivateShopUnlocked { slots }.encode();
            self.private_tell_viewers(vid, &record);
        } else {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You have increased available space on your personal shop.",
                &[],
            );
        }
        Ok(Some(PrivateShopUnlocked {
            moved,
            owner: pid,
            slots,
            standing,
        }))
    }

    /// `CInputMain::PrivateShopWarpRequest` and `CHARACTER::WarpToPrivateShop` for the
    /// character under `vid` (`G/input_main.cpp:5014-5036`, `G/char.cpp:12435-12474`):
    /// the gates in legacy order, then the countdown.
    ///
    /// The stunned and the dead refuse silently; a running timed event cancels with its
    /// line; a non-owner hears `You do not have an open personal shop.`. The DB peer's
    /// shop lookup and channel/port/map match (`D/ClientManagerPrivateShop.cpp:2008-2062`)
    /// collapse into the ownership: one process holds the shop, so no peer can lose it
    /// and no `NO_SHOP` line is reachable. Then, as the warp leg does: the panel closes,
    /// the shop time zeroes (the workaround for the close the request just made), and
    /// past `IsHack` the countdown starts at 10 seconds fighting, 3 else, with the
    /// teleport line. The close and the line pack in that order; the fires tell.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn private_warp_request(
        &mut self,
        vid: Vid,
        actor: Mover,
    ) -> Result<Option<PrivateShopWarp>, MoveItemRefused> {
        self.characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        if self.is_stunned(vid.raw()) || self.is_dead(vid.raw()) {
            return Ok(None);
        }
        if self.timed_events.remove(&vid).is_some() {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Your logout has been canceled.",
                &[],
            );
            return Ok(None);
        }
        if !self.has_private_shop(vid) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You do not have an open personal shop.",
                &[],
            );
            return Ok(None);
        }
        let mut records = self.private_panel_close(vid).records;
        self.clear_shop_time(vid);
        if self.is_hack(vid, actor).is_some() {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"You can not teleport to your private shop right now.",
                &[],
            );
            return Ok(None);
        }
        let fighting = self.bodies.get(&vid).is_some_and(|body| body.fighting);
        let left = if fighting { 10 } else { 3 };
        self.timed_events.insert(
            vid,
            super::timed_exit::TimedEvent::PrivateShopWarp {
                left,
                due: self.last_pulse.saturating_add(1),
            },
        );
        let to = actor.recipient(&self.locale);
        records.push(chat_packet(
            to,
            CHAT_TYPE_INFO,
            b"You are teleporting to your private shop. Please wait.",
            &[],
        ));
        Ok(Some(PrivateShopWarp { records }))
    }

    /// `private_shop_warp_event` (`G/char.cpp:12399-12433`): each due fire counts down
    /// with `%u seconds until teleport.`; at 0 the event is gone and past `CanWarp` the
    /// owner warps to the stand's spot, else the bare cancel line tells. A shop closed
    /// mid-countdown leaves no spot, as the zeroed table leaves legacy's `WarpSet(0, 0)`
    /// with no map: the owner stays, warned.
    pub(super) fn fire_private_shop_warp(&mut self, vid: Vid, left: u8, pulse: u64) {
        let Some(mover) = self.own_mover(vid) else {
            let _gone = self.timed_events.remove(&vid);
            return;
        };
        if left > 0 {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"%u seconds until teleport.",
                &[Arg::Int(left.into())],
            );
            self.timed_events.insert(
                vid,
                super::timed_exit::TimedEvent::PrivateShopWarp {
                    left: left - 1,
                    due: pulse + u64::from(crate::save::PASSES_PER_SEC),
                },
            );
            return;
        }
        let _done = self.timed_events.remove(&vid);
        if !self.private_shop_can_warp(vid, mover) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Teleport to the private shop has been canceled.",
                &[],
            );
            return;
        }
        let (Some(shop), Some((here, map)), Some(clients)) = (
            self.private_shops.get(&vid),
            self.place_of(vid.raw()),
            self.clients.as_ref(),
        ) else {
            tracing::warn!(vid = vid.raw(), "a shop warp found no shop to warp to");
            return;
        };
        let (_, x, y) = shop.at;
        let channel = shop.channel;
        let order = crate::client_registry::ClientOrder::Warp {
            x,
            y,
            private_map_index: 0,
            channel: (channel != here).then_some(channel),
            exit: None,
            sorted_until: 0,
        };
        let raw = vid.raw();
        let ordered = clients
            .members_on_map(here, map)
            .into_iter()
            .find(|(_, entry)| entry.vid == raw)
            .is_some_and(|(id, _)| clients.order(here, id, order));
        if !ordered {
            tracing::warn!(vid = raw, "a shop warp found no client to order");
        }
    }

    /// `CanWarp` for the shop's owner (`G/char.cpp:8788-8814`): past `IsHack`, neither
    /// a viewed shop, an open panel nor a browsing search warps.
    fn private_shop_can_warp(&self, vid: Vid, mover: Mover) -> bool {
        self.is_hack(vid, mover).is_none()
            && !self.private_viewing.contains_key(&vid)
            && !self.edits_private_shop(vid)
            && !self.searches_private_shop(vid)
    }

    /// The unlocked count the next key grows: the standing shop's count, else the
    /// character's point.
    fn private_unlocked_slots(&self, vid: Vid) -> u16 {
        if let Some(shop) = self.private_shops.get(&vid) {
            return shop.unlocked;
        }
        self.characters.find_by_vid(vid).map_or(0, |character| {
            let point = character.points().map_or(0, |points| {
                points.get_point(common::point_slot::POINT_PRIVATE_SHOP_UNLOCKED_SLOT)
            });
            u16::try_from(point.max(0)).unwrap_or(u16::MAX)
        })
    }

    /// Spend the unlock key and grow the point, answering the owner's move: the
    /// key leaves as `RemoveSpecifyItem` spends it, and the point grows to `slots`
    /// through `PointChange`, as the input and the completion grow it
    /// (`G/input_main.cpp:5122` and `G/input_db.cpp:2809-2811`).
    fn private_spend_key_and_grow(
        &mut self,
        vid: Vid,
        actor: Mover,
        pid: u32,
        key: ItemId,
        slots: u16,
    ) -> MovedItems {
        use world::character::{MoveDone, MoveKind, MoveRecord};
        let protos = self.protos.clone();
        let done = {
            let Ok(character) = self.characters.find_by_vid_mut(vid) else {
                return self.private_refused(vid, pid, actor);
            };
            let usable = usable_inventory_cells(character.inven_point());
            let (items, _, slots) = character.items_points_and_quickslots_mut();
            world::character::remove_specify_item(
                items,
                slots,
                &protos,
                usable,
                (PRIVATE_SHOP_SLOT_UNLOCK_VNUM, 1),
            )
        };
        let _ = key;
        let mut done = MoveDone {
            kind: MoveKind::Removed,
            records: done.records,
            changes: done.changes,
        };
        if let Some(points) = self
            .characters
            .find_by_vid_mut(vid)
            .ok()
            .and_then(|character| character.items_and_points_mut().1)
        {
            let now = points.get_point(common::point_slot::POINT_PRIVATE_SHOP_UNLOCKED_SLOT);
            let grow = i32::from(slots) - now;
            if let Ok(records) = points.point_change(
                common::point_slot::POINT_PRIVATE_SHOP_UNLOCKED_SLOT,
                grow,
                true,
                false,
            ) {
                done.records
                    .extend(records.into_iter().map(MoveRecord::Point));
            }
        }
        let mut moved = MovedItems::new(pid, vid.raw(), done, actor, &protos, &self.locale);
        if let Ok(character) = self.characters.find_by_vid(vid) {
            moved.points = character.points().cloned();
        }
        moved
    }

    /// `time(0)`, which the checkin times and the premium expiry are.
    fn private_now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(0))
    }
}

/// Group a price by threes with dots, as `format_number` does
/// (`G/private_shop_util.h:12-24`).
fn private_grouped(price: i64) -> String {
    let digits = price.unsigned_abs().to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.bytes().enumerate() {
        if index != 0 && (digits.len() - index) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(char::from(digit));
    }
    if price < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

/// The owner's name as the entity record shows it: NUL-padded, truncated to 24.
fn private_owner_field(name: &str) -> [u8; PRIVATE_SHOP_OWNER_FIELD_SIZE] {
    let mut owner = [0u8; PRIVATE_SHOP_OWNER_FIELD_SIZE];
    let name_bytes = name.as_bytes();
    let shown = name_bytes.len().min(owner.len() - 1);
    owner[..shown].copy_from_slice(&name_bytes[..shown]);
    owner
}

/// The job arm of `FilterItem` (`G/private_shop_manager.cpp:842-868`): the picked job
/// wears the ware when no antiflag refuses it. An unpicked job, or a pick outside the
/// four arms, refuses nothing, as the empty `switch` arm does.
fn private_search_job_ok(job: i32, anti_flags: u32) -> bool {
    use gamedata::item_kind::{
        ITEM_ANTIFLAG_ASSASSIN, ITEM_ANTIFLAG_SHAMAN, ITEM_ANTIFLAG_SURA, ITEM_ANTIFLAG_WARRIOR,
    };
    if job < 0 {
        return true;
    }
    match u32::try_from(job) {
        Ok(0) => anti_flags & ITEM_ANTIFLAG_WARRIOR == 0,
        Ok(1) => anti_flags & ITEM_ANTIFLAG_ASSASSIN == 0,
        Ok(2) => anti_flags & ITEM_ANTIFLAG_SURA == 0,
        Ok(3) => anti_flags & ITEM_ANTIFLAG_SHAMAN == 0,
        _ => true,
    }
}

/// The gender arm (`:870-884`): the picked gender wears the ware when no antiflag
/// refuses it. An unpicked gender, or a pick outside the two arms, refuses nothing.
fn private_search_gender_ok(gender: i32, anti_flags: u32) -> bool {
    use gamedata::item_kind::{ITEM_ANTIFLAG_FEMALE, ITEM_ANTIFLAG_MALE};
    if gender < 0 {
        return true;
    }
    match u32::try_from(gender) {
        Ok(0) => anti_flags & ITEM_ANTIFLAG_MALE == 0,
        Ok(1) => anti_flags & ITEM_ANTIFLAG_FEMALE == 0,
        _ => true,
    }
}

/// `CItem::GetLevelLimit` (`G/item.cpp:2732-2742`): the `LIMIT_LEVEL` row's value, or
/// 0 when the proto names none.
fn private_search_level(table: &gamedata::item_proto::ItemProto) -> u32 {
    table
        .limits
        .iter()
        .find(|limit| limit.kind == gamedata::item_kind::LIMIT_LEVEL)
        .map_or(0, |limit| u32::try_from(limit.value).unwrap_or(0))
}

/// One ware's item as a store row, owned by `pid` at its old cell.
fn db_item_row(pid: u32, item: &world::item::Item) -> db::items::ItemRow {
    db::items::ItemRow {
        id: item.id,
        owner_id: Some(pid),
        account_id: None,
        window_type: item.pos.window_type,
        pos: u32::from(item.pos.cell),
        vnum: item.vnum,
        count: item.count,
        refine_element: item.refine_element,
        transmutation: item.transmutation,
        flags: item.flags,
        anti_flags: item.anti_flags,
        sockets: item.sockets,
        attributes: item.attributes.map(|attr| db::items::Attribute {
            b_type: attr.b_type,
            s_value: attr.s_value,
        }),
    }
}

#[cfg(test)]
mod tests {
    use common::item_slots::EWindows;
    use common::vid::Vid;
    use protocol::item_pos::ItemPos;
    use world::item::Item;

    use super::super::GameState;
    use super::{
        CG_PRIVATE_SHOP_TITLE_FIELD_SIZE, PRIVATE_SHOP_DEFAULT_VNUM, PRIVATE_SHOP_PAPER_BUNDLE,
    };
    use crate::game_state::fixtures::{a_world, enter_seeing_with, TestClock};
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

    /// A world where player 11 holds a sword (vnum 19) in cell 0 and `other` in
    /// cell 1, standing on map 1, where empire 1 builds.
    fn a_seller(
        other: (u32, u32, u16),
    ) -> (GameState, tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        world.host_map(
            1,
            1,
            gamedata::server_attr::SectreeGrid {
                x: 0,
                y: 0,
                columns: 10,
                rows: 10,
            },
        );
        let sword = held(5, 19, 1);
        let extra = held(other.0, other.1, other.2);
        let (inbox, _) = enter_seeing_with(
            &mut world,
            (1, 1),
            11,
            (5, 5),
            10_000,
            &[(inventory(0), sword), (inventory(1), extra)],
        );
        (world, inbox)
    }

    fn title_of(text: &[u8]) -> [u8; CG_PRIVATE_SHOP_TITLE_FIELD_SIZE] {
        let mut title = [0u8; CG_PRIVATE_SHOP_TITLE_FIELD_SIZE];
        let end = text.len().min(32);
        title[..end].copy_from_slice(&text[..end]);
        title
    }

    fn priced(
        cell: u16,
        gold: i64,
        cheque: u32,
        display: u16,
    ) -> protocol::cg_private_shop::CgPrivateShopItem {
        protocol::cg_private_shop::CgPrivateShopItem {
            pos: inventory(cell),
            gold,
            cheque,
            display_pos: display,
        }
    }

    fn drained(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Ok(record) = inbox.try_recv() {
            out.push(record);
        }
        out
    }

    /// The panel opens and closes with its bare headers, and a second open is silent.
    #[test]
    fn the_panel_opens_and_closes() {
        let (mut state, mut inbox) = a_seller((6, 19, 1));
        drained(&mut inbox);
        let opened = state.private_panel_open(SELLER, mover());
        assert_eq!(opened.records.len(), 1);
        assert_eq!(
            opened.records[0][3],
            protocol::gc_private_shop::SUBHEADER_GC_PRIVATE_SHOP_OPEN_PANEL
        );
        let reopened = state.private_panel_open(SELLER, mover());
        assert!(reopened.records.is_empty(), "a second open is silent");
        let closed = state.private_panel_close(SELLER);
        assert_eq!(closed.records.len(), 1);
        assert_eq!(
            closed.records[0][3],
            protocol::gc_private_shop::SUBHEADER_GC_PRIVATE_SHOP_CLOSE_PANEL
        );
        let _ = PID;
    }

    /// The build stands the shop, moves the ware out of the window, and packs the
    /// `LOAD` and the `ADD_ENTITY` for the store to order after its rows.
    #[test]
    fn a_build_stands_the_shop_and_moves_the_ware() {
        let (mut state, mut inbox) = a_seller((6, 19, 1));
        drained(&mut inbox);
        let built = state
            .private_build(
                SELLER,
                super::PrivateBuild {
                    title: title_of(b"Axes"),
                    poly_vnum: 0,
                    title_type: 1,
                    page_count: 1,
                },
                &[priced(0, 500, 0, 3)],
                mover(),
            )
            .unwrap()
            .unwrap();
        assert!(state.has_private_shop(SELLER));
        let shop = &state.private_shops[&SELLER];
        assert_eq!(&shop.title[..5], b"Axes\x00");
        assert_eq!(
            shop.vnum, PRIVATE_SHOP_DEFAULT_VNUM,
            "vnum 0 clamps to 30000"
        );
        assert_eq!(shop.title_type, 1);
        assert_eq!(shop.page_count, 1);
        let ware = shop.wares[3].as_ref().expect("the ware stands");
        assert_eq!(ware.item.id, 5);
        assert_eq!(ware.item.vnum, 19);
        assert_eq!(ware.gold, 500);
        assert_eq!(ware.cheque, 0);
        {
            let character = state.characters.find_by_vid(SELLER).unwrap();
            assert!(
                character.items().item_at(inventory(0)).is_none(),
                "the ware leaves the window"
            );
            assert!(
                character.items().item_at(inventory(1)).is_some(),
                "the other item stays"
            );
        }
        assert_eq!(built.shop.owner_id, PID);
        assert_eq!(
            built.shop.state,
            protocol::gc_private_shop::PRIVATE_SHOP_STATE_OPEN
        );
        assert_eq!(built.shop.vnum, PRIVATE_SHOP_DEFAULT_VNUM);
        assert_eq!(built.wares.len(), 1);
        assert_eq!(built.wares[0].item.id, 5);
        assert_eq!(built.wares[0].item.vnum, 19);
        assert_eq!(built.wares[0].pos, 3);
        assert_eq!(built.wares[0].gold, 500);
        assert_eq!(
            built.load.len(),
            protocol::gc_private_shop::GC_PRIVATE_SHOP_LOAD_RECORD_SIZE
        );
        assert_eq!(
            built.load[3],
            protocol::gc_private_shop::SUBHEADER_GC_PRIVATE_SHOP_LOAD
        );
        assert_eq!(&built.load[4..8], b"Axes");
        assert_eq!(
            built.entity.len(),
            protocol::gc_private_shop::GC_PRIVATE_SHOP_ADD_ENTITY_RECORD_SIZE
        );
        assert_eq!(
            built.entity[3],
            protocol::gc_private_shop::SUBHEADER_GC_PRIVATE_SHOP_ADD_ENTITY
        );
        assert_eq!(&built.entity[24..27], b"P11", "the owner's name");
        let heard = drained(&mut inbox);
        assert!(
            heard.is_empty(),
            "the records wait for the store: {heard:02x?}"
        );
    }

    /// A title under three bytes refuses, and a banned word refuses with its line.
    #[test]
    fn a_short_title_refuses() {
        let (mut state, mut inbox) = a_seller((6, 19, 1));
        drained(&mut inbox);
        let refused = state
            .private_build(
                SELLER,
                super::PrivateBuild {
                    title: title_of(b"Ax"),
                    poly_vnum: 0,
                    title_type: 0,
                    page_count: 1,
                },
                &[priced(0, 500, 0, 3)],
                mover(),
            )
            .unwrap();
        assert!(refused.is_err());
        assert!(!state.has_private_shop(SELLER));
        assert_eq!(title_of(b"Ax")[2], 0, "two bytes plus the NUL");
        let told = drained(&mut inbox);
        assert_eq!(told.len(), 1, "the too-short refusal chats");
        let _ = PRIVATE_SHOP_PAPER_BUNDLE;
    }

    /// A bundle on the tables refuses with its line, and a second table on the
    /// same cell is silent.
    #[test]
    fn a_bundle_and_a_repeated_cell_refuse() {
        let (mut state, mut inbox) =
            a_seller((PRIVATE_SHOP_PAPER_BUNDLE, PRIVATE_SHOP_PAPER_BUNDLE, 1));
        // The bundle must be held, not only named: cell 1 holds vnum 50200 with id 50200.
        drained(&mut inbox);
        let refused = state
            .private_build(
                SELLER,
                super::PrivateBuild {
                    title: title_of(b"Axes"),
                    poly_vnum: 0,
                    title_type: 0,
                    page_count: 1,
                },
                &[priced(1, 500, 0, 3)],
                mover(),
            )
            .unwrap();
        assert!(refused.is_err());
        assert!(!state.has_private_shop(SELLER));
        let told = drained(&mut inbox);
        assert_eq!(told.len(), 1, "the bundle refusal chats");
        drained(&mut inbox);
        let silent = state
            .private_build(
                SELLER,
                super::PrivateBuild {
                    title: title_of(b"Axes"),
                    poly_vnum: 0,
                    title_type: 0,
                    page_count: 1,
                },
                &[priced(0, 500, 0, 3), priced(0, 600, 0, 4)],
                mover(),
            )
            .unwrap();
        assert!(silent.is_err());
        assert!(drained(&mut inbox).is_empty(), "a repeated cell is silent");
    }

    /// A browse opens the window with the ware's row in its slot, and an `END` closes
    /// it with the bare header.
    #[test]
    fn a_browse_opens_the_window_and_an_end_closes_it() {
        let (mut state, mut inbox) = a_seller((6, 19, 1));
        // A buyer stands beside the seller.
        let buyer = held(7, 19, 1);
        let clock = TestClock::default();
        let _ = &clock;
        let (mut viewer_inbox, _) = enter_seeing_with(
            &mut state,
            (1, 1),
            12,
            (5, 6),
            10_000,
            &[(inventory(0), buyer)],
        );
        drained(&mut inbox);
        drained(&mut viewer_inbox);
        let built = state
            .private_build(
                SELLER,
                super::PrivateBuild {
                    title: title_of(b"Axes"),
                    poly_vnum: 0,
                    title_type: 0,
                    page_count: 1,
                },
                &[priced(0, 500, 0, 3)],
                mover(),
            )
            .unwrap()
            .unwrap();
        let _ = built;
        let opened = state.private_start(Vid::new(12), SELLER.raw(), mover());
        assert_eq!(opened.records.len(), 1, "the window");
        let window = &opened.records[0];
        assert_eq!(
            window.len(),
            protocol::gc_private_shop::GC_PRIVATE_SHOP_START_RECORD_SIZE
        );
        assert_eq!(
            window[3],
            protocol::gc_private_shop::SUBHEADER_GC_PRIVATE_SHOP_START
        );
        assert_eq!(&window[4..8], b"Axes");
        let at = 45 + 3 * protocol::gc_private_shop::GC_PRIVATE_SHOP_ITEM_DATA_WIRE_SIZE;
        assert_eq!(
            u32::from_le_bytes(window[at..at + 4].try_into().expect("four bytes")),
            19
        );
        assert_eq!(
            i64::from_le_bytes(window[at + 4..at + 12].try_into().expect("eight bytes")),
            500
        );
        let closed = state.private_end(Vid::new(12));
        assert_eq!(closed.records.len(), 1, "the END");
        assert_eq!(
            closed.records[0],
            vec![
                protocol::gc_inventory::HEADER_GC_PRIVATE_SHOP.value(),
                4,
                0,
                protocol::gc_private_shop::SUBHEADER_GC_PRIVATE_SHOP_END
            ]
        );
    }

    /// The buy moves the ware to the buyer's window, clears the shop's slot, and
    /// tells the viewers, the owner and the buyer.
    #[test]
    fn a_buy_moves_the_ware_and_tells_everyone() {
        let (mut state, mut inbox) = a_seller((6, 19, 1));
        let buyer = held(7, 19, 1);
        let (mut viewer_inbox, _) = enter_seeing_with(
            &mut state,
            (1, 1),
            12,
            (5, 6),
            10_000,
            &[(inventory(0), buyer)],
        );
        drained(&mut inbox);
        drained(&mut viewer_inbox);
        state
            .private_build(
                SELLER,
                super::PrivateBuild {
                    title: title_of(b"Axes"),
                    poly_vnum: 0,
                    title_type: 0,
                    page_count: 1,
                },
                &[priced(0, 500, 0, 3)],
                mover(),
            )
            .unwrap()
            .unwrap();
        let viewer = Vid::new(12);
        state
            .characters
            .find_by_vid_mut(viewer)
            .expect("the buyer is in the world")
            .set_gold(10_000);
        let opened = state.private_start(viewer, SELLER.raw(), mover());
        assert_eq!(opened.records.len(), 1);
        drained(&mut viewer_inbox);
        let bought = state.private_buy(viewer, 3, mover()).unwrap().unwrap();
        assert_eq!(bought.ware, 5);
        assert_eq!(bought.price, 500);
        // The buyer's move: the gold and the ware's set.
        assert_eq!(bought.bought.records.len(), 2);
        // The ware is in the buyer's window and out of the shop.
        {
            let buyer = state.characters.find_by_vid(viewer).unwrap();
            assert_eq!(buyer.gold(), 10_000 - 500);
            assert!(
                buyer.items().item(5).is_some(),
                "the ware moved to the buyer"
            );
        }
        assert!(
            state.private_shops[&SELLER].wares[3].is_none(),
            "the slot cleared"
        );
        // The viewers and the owner heard the slot clear, the owner the sale, and
        // the buyer the bought line.
        let owner_heard = drained(&mut inbox);
        assert!(
            owner_heard
                .iter()
                .any(|record| record.len() == 6 && record[3] == 14),
            "REMOVE_MY_ITEM: {owner_heard:02x?}"
        );
        assert!(
            owner_heard
                .iter()
                .any(|record| record.len() == 104 && record[3] == 5),
            "SET_SALE: {owner_heard:02x?}"
        );
        assert!(
            owner_heard
                .iter()
                .any(|record| record.len() == 16 && record[3] == 6),
            "BALANCE_UPDATE: {owner_heard:02x?}"
        );
        let viewer_heard = drained(&mut viewer_inbox);
        assert!(
            viewer_heard
                .iter()
                .any(|record| record.len() == 6 && record[3] == 13),
            "REMOVE_ITEM: {viewer_heard:02x?}"
        );
    }

    /// The modify flips the state and answers `STATE_UPDATE` as the owner's own.
    #[test]
    fn a_modify_flips_the_state() {
        let (mut state, mut inbox) = a_seller((6, 19, 1));
        drained(&mut inbox);
        state
            .private_build(
                SELLER,
                super::PrivateBuild {
                    title: title_of(b"Axes"),
                    poly_vnum: 0,
                    title_type: 0,
                    page_count: 1,
                },
                &[priced(0, 500, 0, 3)],
                mover(),
            )
            .unwrap()
            .unwrap();
        state.private_panels.insert(SELLER);
        let flipped = state.private_modify(SELLER, mover()).unwrap().unwrap();
        assert_eq!(flipped.state, super::PRIVATE_SHOP_STATE_MODIFY);
        assert_eq!(
            state.private_shops[&SELLER].state,
            super::PRIVATE_SHOP_STATE_MODIFY
        );
        let heard = drained(&mut inbox);
        assert_eq!(heard.len(), 1, "STATE_UPDATE: {heard:02x?}");
        assert_eq!(&heard[0][..4], &[175, 6, 0, 15]);
        assert_eq!(&heard[0][4..], &[3, 1]);
    }

    /// The withdraw moves the stash to the gold, answers the bare `WITHDRAW` with
    /// it, and destroys the shop when no wares are left.
    #[test]
    fn a_withdraw_empties_the_stash() {
        let (mut state, mut inbox) = a_seller((6, 19, 1));
        let buyer = held(7, 19, 1);
        let (mut viewer_inbox, _) = enter_seeing_with(
            &mut state,
            (1, 1),
            12,
            (5, 6),
            10_000,
            &[(inventory(0), buyer)],
        );
        drained(&mut inbox);
        drained(&mut viewer_inbox);
        state
            .private_build(
                SELLER,
                super::PrivateBuild {
                    title: title_of(b"Axes"),
                    poly_vnum: 0,
                    title_type: 0,
                    page_count: 1,
                },
                &[priced(0, 500, 0, 3)],
                mover(),
            )
            .unwrap()
            .unwrap();
        state.private_panels.insert(SELLER);
        let viewer = Vid::new(12);
        state
            .characters
            .find_by_vid_mut(viewer)
            .expect("the buyer is in the world")
            .set_gold(10_000);
        state.private_start(viewer, SELLER.raw(), mover());
        state.private_buy(viewer, 3, mover()).unwrap().unwrap();
        assert_eq!(state.private_shops[&SELLER].gold, 500);
        drained(&mut inbox);
        state
            .characters
            .find_by_vid_mut(SELLER)
            .expect("the seller is in the world")
            .set_gold(1000);
        let withdrawn = state.private_withdraw(SELLER, mover()).unwrap().unwrap();
        assert!(withdrawn.destroy, "no wares are left");
        assert_eq!(
            withdrawn.withdrawn.records.len(),
            2,
            "WITHDRAW and the gold"
        );
        assert_eq!(&withdrawn.withdrawn.records[0], &[175, 4, 0, 16]);
        assert_eq!(state.characters.find_by_vid(SELLER).unwrap().gold(), 1500);
        assert_eq!(state.private_shops[&SELLER].gold, 0);
        let heard = drained(&mut inbox);
        assert_eq!(heard.len(), 1, "the withdrawn line: {heard:02x?}");
    }

    /// The close moves the ware back to the window, cleans the viewers, and answers
    /// `CLOSE` first.
    #[test]
    fn a_close_returns_the_wares_and_cleans_the_viewers() {
        let (mut state, mut inbox) = a_seller((6, 19, 1));
        let buyer = held(7, 19, 1);
        let (mut viewer_inbox, _) = enter_seeing_with(
            &mut state,
            (1, 1),
            12,
            (5, 6),
            10_000,
            &[(inventory(0), buyer)],
        );
        drained(&mut inbox);
        drained(&mut viewer_inbox);
        state
            .private_build(
                SELLER,
                super::PrivateBuild {
                    title: title_of(b"Axes"),
                    poly_vnum: 0,
                    title_type: 0,
                    page_count: 1,
                },
                &[priced(0, 500, 0, 3)],
                mover(),
            )
            .unwrap()
            .unwrap();
        state.private_panels.insert(SELLER);
        let viewer = Vid::new(12);
        state.private_start(viewer, SELLER.raw(), mover());
        drained(&mut viewer_inbox);
        let closed = state.private_close(SELLER, mover()).unwrap().unwrap();
        assert_eq!(closed.wares, vec![5]);
        assert!(
            !state.private_shops.contains_key(&SELLER),
            "the shop is gone"
        );
        assert_eq!(&closed.closed.records[0], &[175, 4, 0, 9], "CLOSE first");
        assert_eq!(closed.closed.records.len(), 2, "CLOSE and the ware's set");
        {
            let owner = state.characters.find_by_vid(SELLER).unwrap();
            assert!(owner.items().item(5).is_some(), "the ware came home");
        }
        let viewer_heard = drained(&mut viewer_inbox);
        assert_eq!(viewer_heard.len(), 1, "the END: {viewer_heard:02x?}");
        assert_eq!(&viewer_heard[0], &[175, 4, 0, 12]);
        let owner_heard = drained(&mut inbox);
        assert_eq!(owner_heard.len(), 1, "the closed line: {owner_heard:02x?}");
    }

    /// A standing shop, its panel open: the four edits flip it to modify first.
    fn a_standing_shop() -> (GameState, tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) {
        let (mut state, mut inbox) = a_seller((6, 19, 1));
        drained(&mut inbox);
        state
            .private_build(
                SELLER,
                super::PrivateBuild {
                    title: title_of(b"Axes"),
                    poly_vnum: 0,
                    title_type: 0,
                    page_count: 1,
                },
                &[priced(0, 500, 0, 3)],
                mover(),
            )
            .unwrap()
            .unwrap();
        state.private_panels.insert(SELLER);
        (state, inbox)
    }

    #[test]
    fn the_price_and_the_move_sync_to_owner_and_viewers() {
        let (mut state, mut inbox) = a_standing_shop();
        let buyer = held(7, 19, 1);
        let (mut viewer_inbox, _) = enter_seeing_with(
            &mut state,
            (1, 1),
            12,
            (5, 6),
            10_000,
            &[(inventory(0), buyer)],
        );
        drained(&mut inbox);
        drained(&mut viewer_inbox);
        // Before the flip the modify-state line refuses.
        let refused = state
            .private_price_change(SELLER, mover(), 3, 700, 0)
            .unwrap();
        assert!(refused.is_none());
        let told = drained(&mut inbox);
        assert_eq!(told.len(), 1, "the modify-state refusal chats");
        state.private_modify(SELLER, mover()).unwrap().unwrap();
        state.last_pulse += super::PRIVATE_SHOP_STATE_WAIT_PULSES;
        drained(&mut inbox);
        let changed = state
            .private_price_change(SELLER, mover(), 3, 700, 0)
            .unwrap()
            .unwrap();
        assert_eq!(changed.gold, 700);
        assert_eq!(
            state.private_shops[&SELLER].wares[3].as_ref().unwrap().gold,
            700
        );
        let owner_heard = drained(&mut inbox);
        assert!(
            owner_heard
                .iter()
                .any(|record| record.len() == 18 && record[3] == 17),
            "PRICE_CHANGE: {owner_heard:02x?}"
        );
        state.last_pulse += super::PRIVATE_SHOP_STATE_WAIT_PULSES;
        state.private_start(Vid::new(12), SELLER.raw(), mover());
        drained(&mut viewer_inbox);
        drained(&mut inbox);
        let moved = state.private_move(SELLER, mover(), 3, 5).unwrap().unwrap();
        assert_eq!(moved.change_pos, 5);
        assert!(state.private_shops[&SELLER].wares[3].is_none());
        assert!(state.private_shops[&SELLER].wares[5].is_some());
        let owner_heard = drained(&mut inbox);
        assert!(
            owner_heard
                .iter()
                .any(|record| record.len() == 8 && record[3] == 18),
            "MOVE: {owner_heard:02x?}"
        );
        let viewer_heard = drained(&mut viewer_inbox);
        assert!(
            viewer_heard
                .iter()
                .any(|record| record.len() == 8 && record[3] == 18),
            "MOVE: {viewer_heard:02x?}"
        );
    }

    #[test]
    fn the_checkin_shelves_and_the_checkout_unshelves() {
        let (mut state, mut inbox) = a_standing_shop();
        state.private_modify(SELLER, mover()).unwrap().unwrap();
        state.last_pulse += super::PRIVATE_SHOP_STATE_WAIT_PULSES;
        drained(&mut inbox);
        // Cell 1 still holds the extra sword; it shelves at the picked slot.
        let checked = state
            .private_checkin(SELLER, mover(), inventory(1), 900, 0, 7)
            .unwrap()
            .unwrap();
        assert_eq!(checked.ware.pos, 7);
        assert_eq!(
            state.private_shops[&SELLER].wares[7].as_ref().unwrap().gold,
            900
        );
        let owner = state.characters.find_by_vid(SELLER).unwrap();
        assert!(
            owner.items().item_at(inventory(1)).is_none(),
            "the ware left the window"
        );
        let owner_heard = drained(&mut inbox);
        assert!(
            owner_heard
                .iter()
                .any(|record| record.len() == 75 && record[3] == 4),
            "SET_ITEM: {owner_heard:02x?}"
        );
        state.last_pulse += super::PRIVATE_SHOP_STATE_WAIT_PULSES;
        let out = state
            .private_checkout(SELLER, mover(), 7, -1)
            .unwrap()
            .unwrap();
        assert!(!out.destroy, "the first ware still stands");
        let owner = state.characters.find_by_vid(SELLER).unwrap();
        assert!(
            owner.items().item_at(inventory(0)).is_some(),
            "the ware lands in the first free cell"
        );
        assert_eq!(out.window_type, EWindows::Inventory as u8);
        assert_eq!(out.pos, 0);
        assert!(out
            .moved
            .records
            .first()
            .is_some_and(|record| record.len() == 6 && record[3] == 14));
        let owner_heard = drained(&mut inbox);
        assert!(
            owner_heard.is_empty(),
            "the clear rides the move: {owner_heard:02x?}"
        );
    }

    /// The rename retitles the shop and syncs the owner and the neighbours;
    /// the short title refuses with its line.
    #[test]
    fn a_rename_retitles_and_syncs() {
        let (mut state, mut inbox) = a_standing_shop();
        let (mut viewer_inbox, _) = enter_seeing_with(
            &mut state,
            (1, 1),
            12,
            (5, 6),
            10_000,
            &[(inventory(0), held(7, 19, 1))],
        );
        drained(&mut inbox);
        drained(&mut viewer_inbox);
        state.private_modify(SELLER, mover()).unwrap().unwrap();
        state.last_pulse += super::PRIVATE_SHOP_STATE_WAIT_PULSES;
        drained(&mut inbox);
        let renamed = state
            .private_rename(SELLER, mover(), title_of(b"Swords"))
            .unwrap()
            .unwrap();
        assert_eq!(&renamed.title[..7], b"Swords\x00");
        assert_eq!(&state.private_shops[&SELLER].title[..7], b"Swords\x00");
        let owner_heard = drained(&mut inbox);
        assert!(
            owner_heard
                .iter()
                .any(|record| record.len() == 37 && record[3] == 19),
            "TITLE_CHANGE: {owner_heard:02x?}"
        );
        let viewer_heard = drained(&mut viewer_inbox);
        assert!(
            viewer_heard
                .iter()
                .any(|record| record.len() == 42 && record[3] == 2),
            "TITLE: {viewer_heard:02x?}"
        );
        state.last_pulse += super::PRIVATE_SHOP_STATE_WAIT_PULSES;
        let refused = state
            .private_rename(SELLER, mover(), title_of(b"Ax"))
            .unwrap();
        assert!(refused.is_none());
        let told = drained(&mut inbox);
        assert_eq!(told.len(), 1, "the short-title refusal chats");
    }

    /// The unlock spends the key, grows the shop's count, and tells the viewers;
    /// without a stand the point grows and the owner hears the increased line.
    #[test]
    fn an_unlock_opens_slots_with_and_without_a_stand() {
        let (mut state, mut inbox) = a_seller((6, 19, 1));
        // Cell 2 holds the shop key.
        let key = held(8, super::PRIVATE_SHOP_SLOT_UNLOCK_VNUM, 1);
        state
            .characters
            .find_by_vid_mut(SELLER)
            .expect("the seller is in the world")
            .items_mut()
            .set(inventory(2), &key)
            .expect("cell 2 is free");
        drained(&mut inbox);
        let unlocked = state.private_unlock(SELLER, mover(), 2).unwrap().unwrap();
        assert!(!unlocked.standing);
        let owner = state.characters.find_by_vid(SELLER).unwrap();
        assert!(
            owner.items().item_at(inventory(2)).is_none(),
            "the key spent"
        );
        let point = owner.points().map_or(0, |points| {
            points.get_point(common::point_slot::POINT_PRIVATE_SHOP_UNLOCKED_SLOT)
        });
        assert_eq!(point, i32::from(unlocked.slots), "the point grew");
        assert!(unlocked.slots > 0, "the key's value opened slots");
        let told = drained(&mut inbox);
        assert!(
            told.iter()
                .any(|record| record[0] == protocol::gc_inventory::HEADER_GC_CHAT.value()),
            "the increased line: {told:02x?}"
        );
        let _ = PID;
    }

    /// Build the seller's one-ware shop for the warp tests.
    fn built_shop(state: &mut super::super::GameState) {
        state
            .private_build(
                SELLER,
                super::PrivateBuild {
                    title: title_of(b"Axes"),
                    poly_vnum: 0,
                    title_type: 1,
                    page_count: 1,
                },
                &[priced(0, 500, 0, 3)],
                mover(),
            )
            .unwrap()
            .unwrap();
    }

    /// The text of one chat record.
    fn chat_text(line: &[u8]) -> Vec<u8> {
        let size = usize::from(u16::from_le_bytes([line[1], line[2]]));
        assert_eq!(size, line.len());
        line[10..].to_vec()
    }

    /// The warp request closes the panel, zeroes the shop time, and starts the
    /// countdown with the teleport line: 3 calm, 10 fighting. The stunned, the
    /// dead and the shopless refuse, and a running event cancels instead.
    #[test]
    fn a_warp_request_closes_counts_and_arms() {
        let (mut state, mut inbox) = a_seller((6, 19, 1));
        assert!(state
            .private_warp_request(SELLER, mover())
            .unwrap()
            .is_none());
        let told = drained(&mut inbox);
        assert_eq!(told.len(), 1, "the shopless hears one line");
        assert_eq!(
            chat_text(&told[0]),
            b"You do not have an open personal shop.".as_slice()
        );
        built_shop(&mut state);
        assert!(state.edits_private_shop(SELLER));
        state.set_shop_time(SELLER);
        assert!(
            state.is_hack(SELLER, mover()).is_some(),
            "the stamped shop time refuses"
        );
        drained(&mut inbox);
        let warped = state
            .private_warp_request(SELLER, mover())
            .unwrap()
            .unwrap();
        assert_eq!(warped.records.len(), 2, "the close and the line");
        assert_eq!(
            warped.records[0][3],
            protocol::gc_private_shop::SUBHEADER_GC_PRIVATE_SHOP_CLOSE_PANEL
        );
        assert_eq!(
            chat_text(&warped.records[1]),
            b"You are teleporting to your private shop. Please wait.".as_slice()
        );
        assert!(!state.edits_private_shop(SELLER), "the panel closed");
        assert!(
            state.is_hack(SELLER, mover()).is_none(),
            "the shop time zeroed"
        );
        let event = state.timed_events.get(&SELLER).expect("the countdown runs");
        assert!(
            matches!(
                event,
                super::super::timed_exit::TimedEvent::PrivateShopWarp { left: 3, .. }
            ),
            "calm waits 3, not {event:?}"
        );
        // A line while the countdown runs cancels it instead of warping.
        let again = state.private_warp_request(SELLER, mover()).unwrap();
        assert!(again.is_none());
        assert!(!state.timed_events.contains_key(&SELLER));
        let told = drained(&mut inbox);
        assert!(
            told.iter()
                .any(|line| chat_text(line) == b"Your logout has been canceled."),
            "the cancel line"
        );
        let _ = PID;
    }

    /// The fires count 3, 2, 1 one second apart, then warp the owner to the
    /// stand's spot. A shop closed mid-countdown leaves the owner where it is.
    #[test]
    fn the_warp_fires_count_then_warp_to_the_stand() {
        use std::sync::Arc;

        use crate::client_registry::{ChannelClients, ClientEntry, ClientOrder};
        let (world, mut inbox) = a_seller((6, 19, 1));
        let clients = Arc::new(ChannelClients::new());
        let mut world = world.with_clients(Arc::clone(&clients));
        let mut lease = clients.join(ClientEntry {
            channel: 1,
            map: 1,
            name: "P11".to_string(),
            vid: 11,
            empire: 1,
            language: 0,
        });
        let mut orders = lease.take_orders().expect("the queue is home");
        built_shop(&mut world);
        let shop_at = world.private_shops[&SELLER].at;
        // The owner walks away, so the warp has somewhere to go.
        assert!(world.move_to(11, shop_at.1 + 700, shop_at.2));
        drained(&mut inbox);
        world
            .private_warp_request(SELLER, mover())
            .unwrap()
            .unwrap();
        let _start = drained(&mut inbox);
        let mut pulse = world.last_pulse() + 1;
        for left in (1..=3).rev() {
            world.run_timed_exits(pulse);
            let want = format!("{left} seconds until teleport.");
            assert!(
                drained(&mut inbox)
                    .iter()
                    .any(|line| chat_text(line) == want.as_bytes()),
                "the fire tells {left}"
            );
            pulse += u64::from(crate::save::PASSES_PER_SEC);
        }
        world.run_timed_exits(pulse);
        assert!(!world.timed_events.contains_key(&SELLER));
        let order = orders.try_recv().expect("the warp order");
        match order {
            ClientOrder::Warp { x, y, channel, .. } => {
                assert_eq!((x, y), (shop_at.1, shop_at.2), "the stand's spot");
                assert_eq!(channel, None, "the stand's channel is home");
            }
            other => panic!("a warp, not {other:?}"),
        }
        // A shop closed mid-countdown: the 0 fire stays silent and stays put.
        world
            .private_warp_request(SELLER, mover())
            .unwrap()
            .unwrap();
        world.private_shops.remove(&SELLER);
        drained(&mut inbox);
        let mut pulse = world.last_pulse() + 1;
        for _ in 0..4 {
            world.run_timed_exits(pulse);
            pulse += u64::from(crate::save::PASSES_PER_SEC);
        }
        assert!(drained(&mut inbox)
            .iter()
            .all(|line| { chat_text(line) != b"Teleport to the private shop has been canceled." }));
        assert!(orders.try_recv().is_err(), "no warp without a shop");
        let _ = PID;
    }

    /// A build on map 41 refuses for empire 1, whose market is map 1.
    #[test]
    fn a_build_outside_the_market_refuses() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let sword = held(5, 19, 1);
        let extra = held(6, 19, 1);
        let (mut inbox, _) = enter_seeing_with(
            &mut world,
            (1, 41),
            11,
            (5, 5),
            10_000,
            &[(inventory(0), sword), (inventory(1), extra)],
        );
        drained(&mut inbox);
        let refused = world
            .private_build(
                SELLER,
                super::PrivateBuild {
                    title: title_of(b"Axes"),
                    poly_vnum: 0,
                    title_type: 0,
                    page_count: 1,
                },
                &[priced(0, 500, 0, 3)],
                mover(),
            )
            .unwrap();
        assert!(refused.is_err());
        assert!(!world.has_private_shop(SELLER));
        let told = drained(&mut inbox);
        assert_eq!(told.len(), 1, "the wrong-map refusal chats");
    }
    /// A blank search with no vnum, owner or type finds nothing and sends nothing,
    /// and the wait line refuses a second search within two seconds.
    #[test]
    fn a_blank_search_sends_nothing_and_the_wait_refuses() {
        use protocol::cg_private_shop::CgPrivateShopSearch;
        use protocol::gc_private_shop::SUBHEADER_GC_PRIVATE_SHOP_SEARCH_RESULT;
        let (mut state, mut inbox) = a_seller((6, 19, 1));
        drained(&mut inbox);
        let _built = state
            .private_build(
                SELLER,
                super::PrivateBuild {
                    title: title_of(b"Axes"),
                    poly_vnum: 0,
                    title_type: 1,
                    page_count: 1,
                },
                &[priced(0, 500, 0, 3)],
                mover(),
            )
            .unwrap()
            .unwrap();
        let blank = CgPrivateShopSearch {
            vnum: 0,
            owner: [0; 25],
            item_type: -1,
            item_subtype: -1,
            job: -1,
            gender: -1,
            min_stack: 0,
            max_stack: 0,
            min_refine: 0,
            max_refine: 0,
            min_level: 0,
            max_level: 0,
            attrs: [(0, 0); 7],
            sash_absorption: 0,
            alchemy_level: 0,
            alchemy_clarity: 0,
            use_filter: false,
        };
        state.last_pulse = super::PRIVATE_SHOP_SEARCH_WAIT_PULSES + 1;
        let nothing = state.private_search(SELLER, &blank, mover()).unwrap();
        assert!(nothing.records.is_empty());
        assert!(drained(&mut inbox).is_empty());
        // The second search lands inside the wait: the line tells, not the rows.
        let by_vnum = CgPrivateShopSearch { vnum: 19, ..blank };
        let waited = state.private_search(SELLER, &by_vnum, mover()).unwrap();
        assert!(waited.records.is_empty());
        let told = drained(&mut inbox);
        assert_eq!(told.len(), 1);
        assert_eq!(
            chat_text(&told[0]),
            b"Please wait a moment before searching other personal shops again."
        );
        // Past the wait the vnum finds the ware's row.
        state.last_pulse += super::PRIVATE_SHOP_SEARCH_WAIT_PULSES;
        let found = state.private_search(SELLER, &by_vnum, mover()).unwrap();
        assert_eq!(found.records.len(), 1);
        assert_eq!(found.records[0][3], SUBHEADER_GC_PRIVATE_SHOP_SEARCH_RESULT);
        let _ = PID;
    }

    /// A sold ware tells the searchers its shop, slot and removal, and a flip tells
    /// them the whole shop and its state.
    #[test]
    fn a_sale_and_a_flip_tell_the_searchers() {
        use protocol::gc_private_shop::{
            GC_PRIVATE_SHOP_WIRE_SIZE, SUBHEADER_GC_PRIVATE_SHOP_SEARCH_UPDATE,
        };
        let (mut state, mut inbox) = a_seller((6, 19, 1));
        drained(&mut inbox);
        let _built = state
            .private_build(
                SELLER,
                super::PrivateBuild {
                    title: title_of(b"Axes"),
                    poly_vnum: 0,
                    title_type: 1,
                    page_count: 1,
                },
                &[priced(0, 500, 0, 3)],
                mover(),
            )
            .unwrap()
            .unwrap();
        state
            .private_searching
            .insert(SELLER, super::PRIVATE_SHOP_SEARCH_LOOKING);
        // A flip tells the shop and its state with the whole-shop slot.
        let flipped = state.private_modify(SELLER, mover()).unwrap().unwrap();
        assert_eq!(flipped.state, super::PRIVATE_SHOP_STATE_MODIFY);
        let told = drained(&mut inbox);
        let update = told
            .iter()
            .find(|record| {
                record.len() > GC_PRIVATE_SHOP_WIRE_SIZE
                    && record[3] == SUBHEADER_GC_PRIVATE_SHOP_SEARCH_UPDATE
            })
            .expect("the flip tells the searchers");
        assert_eq!(
            u32::from_le_bytes(update[4..8].try_into().expect("four")),
            PID
        );
        assert_eq!(
            i32::from_le_bytes(update[8..12].try_into().expect("four")),
            -1
        );
        assert_eq!(update[12], super::PRIVATE_SHOP_STATE_MODIFY);
    }
}
