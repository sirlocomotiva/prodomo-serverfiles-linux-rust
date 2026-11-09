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

use common::item_slots::EWindows;
use common::vid::Vid;
use protocol::cg_private_shop::{CgPrivateShopItem, CG_PRIVATE_SHOP_TITLE_FIELD_SIZE};
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_private_shop::{
    GcPrivateShop, GcPrivateShopAddEntity, GcPrivateShopLoad, PRIVATE_SHOP_OWNER_FIELD_SIZE,
    PRIVATE_SHOP_STATE_OPEN, SUBHEADER_GC_PRIVATE_SHOP_CLOSE_PANEL,
    SUBHEADER_GC_PRIVATE_SHOP_OPEN_PANEL,
};
use world::item::{ItemId, ITEM_ANTIFLAG_GIVE, ITEM_ANTIFLAG_MYSHOP};

use super::GameState;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};
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

/// One ware of a built shop: the held item's id, its slot and its price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivateShopWare {
    /// The held item's id.
    pub item: ItemId,
    /// The asking price in yang.
    pub gold: i64,
    /// The asking price in cheques.
    pub cheque: u32,
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
            },
        );
        self.private_panels.remove(&vid);
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
            [None; PRIVATE_SHOP_WINDOW_SLOTS];
        let mut stored = Vec::with_capacity(wares.len());
        for (slot, item, gold, cheque) in wares {
            slots[usize::from(*slot)] = Some(PrivateShopWare {
                item: item.id,
                gold: *gold,
                cheque: *cheque,
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

    /// `time(0)`, which the checkin times and the premium expiry are.
    fn private_now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(0))
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
        assert_eq!(
            shop.wares[3],
            Some(super::PrivateShopWare {
                item: 5,
                gold: 500,
                cheque: 0
            })
        );
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
}
