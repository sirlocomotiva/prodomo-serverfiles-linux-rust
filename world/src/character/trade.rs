//! A trade between two players: `CExchange` (`G/exchange.cpp`), and the arms of
//! `CInputMain::Exchange` (`G/input_main.cpp:1367-1527`) that act on one.
//!
//! A [`Trade`] holds both sides' offers: up to [`EXCHANGE_ITEM_MAX_NUM`] items, each on a
//! display cell of the 6 × 4 window (`__NEW_EXCHANGE_WINDOW__`), one amount of gold, and whether
//! the side accepts. Offering, taking back and accepting answer with what each side is sent, in
//! the order legacy sends it. Finding the other player, the distance, the other windows a
//! character has open, and when a trade is cancelled are the caller's.
//!
//! # The settlement
//!
//! When both sides accept, [`settle`] runs legacy's four checks and `CExchange::Done` for both
//! sides on copies of the two characters' storage, quickslots and gold. It answers with the
//! copies, each side's records and row changes, and how far each side's gold moves; or, when a
//! check fails, with the line each side is told. Nothing of either character changes until the
//! caller has stored the whole trade in one transaction (ADR-0003) and put the copies in place.
//!
//! `Done` runs first for the side whose accept completed the trade, as `Accept` runs it: that
//! side's items go to the other character, each to the first free cell of a custom bank it
//! belongs to and then of the unlocked base inventory, and its gold follows; then the other
//! side's do the same, into cells the first half may have freed. Each item leaves its giver with
//! the quickslots that named its cell deleted and the cell cleared, and reaches its receiver
//! highlighted, a sash rolling its absorption as `AddToCharacter` does.
//!
//! # Divergences
//!
//! - **An item with no cell.** `Done` skips an item it finds no cell for (`continue`,
//!   `G/exchange.cpp:534-539`), and the trade goes on without it. `CheckSpace` is meant to rule
//!   that out, but it fills the receiver's cells as they are before any item has left, and the
//!   second half of `Done` fills them after, so a first-fit placement can come out differently.
//!   The Rewrite places every item for real on the copies, and an item with no cell refuses the
//!   whole trade with the lines `CheckSpace` sends for that direction.
//! - **Gold at the cap.** `ChangeGold` silently refuses gold that would reach [`GOLD_MAX_MAX`]
//!   (`G/char.cpp:3823-3850`), after the giver's gold has been taken, so the gold is lost. The
//!   Rewrite checks both receivers before anything changes, and a trade that would take one to
//!   the cap is refused, that side told [`YANG_LIMIT_NOTICE`].
//!
//! # Not ported
//!
//! A dragon soul stone, an item outside the `INVENTORY` window and an item with no prototype
//! cannot be offered: the offer is refused silently, after every legacy check, so no trade
//! moves one (legacy gives a stone a cell of the dragon soul inventory). The item lock, the
//! quest check at the accept (`@fixme150`), the DB-cache check, `SetExchangeTime` (the portal
//! guard) and the item and gold logs are not ported either.

use common::item_slots::EWindows;
use gamedata::item_custom_category::{is_custom_category, CATEGORY_NUM};
use gamedata::item_kind::ITEM_DS;
use gamedata::item_proto::{ItemProto, ItemProtos};
use protocol::gc_exchange::{
    GcExchange, EXCHANGE_SUBHEADER_GC_ACCEPT, EXCHANGE_SUBHEADER_GC_ALREADY,
    EXCHANGE_SUBHEADER_GC_END, EXCHANGE_SUBHEADER_GC_GOLD_ADD, EXCHANGE_SUBHEADER_GC_ITEM_ADD,
    EXCHANGE_SUBHEADER_GC_ITEM_DEL, EXCHANGE_SUBHEADER_GC_LESS_GOLD, EXCHANGE_SUBHEADER_GC_START,
};
use protocol::item_pos::ItemPos;

use super::dice::Dice;
use super::equip::roll_sash;
use super::inventory::{is_equip_position, NPOS};
use super::item_move::{ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord};
use super::items::CharacterItems;
use super::quickslot::{sync_quickslots, QuickslotSync, Quickslots, SyncTo};
use super::shop::GOLD_MAX_MAX;
use crate::item::{gc_item_clear, Item, ItemId, ITEM_ANTIFLAG_GIVE};

/// `EXCHANGE_ITEM_MAX_NUM` under `__NEW_EXCHANGE_WINDOW__` (`G/exchange.h:9`): the items one
/// side may offer.
pub const EXCHANGE_ITEM_MAX_NUM: u8 = 24;

/// `EXCHANGE_MAX_DISTANCE` (`G/exchange.h:13`): the `DISTANCE_APPROX` at which a trade cannot
/// start, and at which one is cancelled.
pub const EXCHANGE_MAX_DISTANCE: i32 = 1000;

/// `LC_TEXT("Item cannot be handed over.")`: the item's anti-flags forbid giving it
/// (`G/exchange.cpp:204`).
pub const GIVE_REFUSED_NOTICE: &str = "Item cannot be handed over.";

/// The side whose item moved or whose gold is short (`G/exchange.cpp:637`).
pub const OUT_OF_PLACE_NOTICE: &str = "You are out of money or an item is out of place.";

/// The other side of [`OUT_OF_PLACE_NOTICE`] (`G/exchange.cpp:638`).
pub const PARTNER_OUT_OF_PLACE_NOTICE: &str =
    "The opponent is out of money or the item is out of place.";

/// The side whose items the other has no room for (`G/exchange.cpp:645`).
pub const PARTNER_FULL_NOTICE: &str = "There are no empty spaces in the opponent's inventory.";

/// The side with no room for the other's items (`G/exchange.cpp:646`).
pub const FULL_NOTICE: &str = "There are no empty spaces in your inventory.";

/// `LC_TEXT("You have reached the yang limit.")` (`G/input_main.cpp:1441`): a trade cannot
/// start, or settle, with gold at [`GOLD_MAX_MAX`].
pub const YANG_LIMIT_NOTICE: &str = "You have reached the yang limit.";

/// The line each side is sent when the trade went through, formatted with the other
/// character's name (`G/exchange.cpp:684-685`).
pub const COMPLETED_NOTICE: &str = "The exchange with %s has been completed.";

/// A trade cannot start while the starter has another window open (`G/input_main.cpp:1458`).
pub const OTHER_TRANSACTION_NOTICE: &str =
    "You cannot open a personal shop while other transactions are in progress.";

/// The player asked has another window open (`G/exchange.cpp:100`).
pub const PARTNER_BUSY_NOTICE: &str =
    "You cannot trade because the other player is in the middle of another transaction.";

/// `CGrid(6, 4)`: the display window's width.
const GRID_WIDTH: usize = 6;

/// Its height.
const GRID_HEIGHT: usize = 4;

/// Its cells.
const GRID_CELLS: usize = GRID_WIDTH * GRID_HEIGHT;

/// [`EXCHANGE_ITEM_MAX_NUM`] as a length.
const SLOTS: usize = 24;

/// One of the two characters of a trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// The character that asked for the trade: `ExchangeStart`'s `this`.
    Starter,
    /// The character it asked: the `victim`.
    Asked,
}

impl Side {
    /// The other side.
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Starter => Self::Asked,
            Self::Asked => Self::Starter,
        }
    }

    /// This side's index in the pairs a trade answers with.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Starter => 0,
            Self::Asked => 1,
        }
    }
}

/// What a side is sent about a trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeRecord {
    /// A `GC_EXCHANGE`.
    Exchange(GcExchange),
    /// A `CHAT_TYPE_INFO` line.
    Notice(&'static str),
}

/// What each side of a trade is sent by one step, in the order legacy sends it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Said {
    records: [Vec<TradeRecord>; 2],
}

impl Said {
    /// What `side` is sent.
    #[must_use]
    pub fn records_to(&self, side: Side) -> &[TradeRecord] {
        &self.records[side.index()]
    }

    /// Take what `side` is sent, leaving nothing.
    pub fn take(&mut self, side: Side) -> Vec<TradeRecord> {
        core::mem::take(&mut self.records[side.index()])
    }

    fn exchange(&mut self, side: Side, record: GcExchange) {
        self.records[side.index()].push(TradeRecord::Exchange(record));
    }
}

/// An item one side offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offered {
    /// The item.
    pub id: ItemId,
    /// Where its owner holds it: `m_aItemPos`.
    pub pos: ItemPos,
    /// Its cell in the display window: `m_abItemDisplayPos`.
    pub display: u8,
    /// How many display rows it covers: its size.
    pub size: u8,
}

/// One side's half of a trade: a `CExchange`'s items, grid, gold and accept flag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Offer {
    slots: [Option<Offered>; SLOTS],
    grid: [bool; GRID_CELLS],
    gold: u64,
    accepted: bool,
}

impl Offer {
    /// `CGrid::IsEmpty(display, 1, size)`: the item's rows fit under the window's bottom and
    /// none of them is taken.
    fn is_clear(&self, display: u8, size: u8) -> bool {
        let (cell, rows) = (usize::from(display), usize::from(size));
        cell / GRID_WIDTH + rows <= GRID_HEIGHT
            && (0..rows).all(|row| self.grid.get(cell + row * GRID_WIDTH) == Some(&false))
    }

    /// `CGrid::Put` and `CGrid::Get`.
    fn mark(&mut self, display: u8, size: u8, taken: bool) {
        for row in 0..usize::from(size) {
            if let Some(cell) = self.grid.get_mut(usize::from(display) + row * GRID_WIDTH) {
                *cell = taken;
            }
        }
    }

    fn offered(&self) -> impl Iterator<Item = Offered> + '_ {
        self.slots.iter().flatten().copied()
    }

    /// `CExchange::Check`: the side still holds every item where it offered it, and the gold.
    fn is_still_held(&self, trader: &Trader) -> bool {
        // `GetItem` finds nothing at a position `IsValidItemPosition` refuses, and neither does
        // `item_at`.
        trader.gold >= self.gold
            && self.offered().all(|offered| {
                trader
                    .items
                    .item_at(offered.pos)
                    .is_some_and(|item| item.id == offered.id)
            })
    }
}

/// What an accept did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accepted {
    /// The side accepts already, and nothing is sent.
    Unchanged,
    /// The side accepts and the other does not yet; both are told.
    Waiting(Said),
    /// Both sides accept. The caller settles the trade with [`settle`], closed by this side.
    Both,
}

/// A trade between two characters: a `CExchange` and its company.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trade {
    offers: [Offer; 2],
}

impl Trade {
    /// `ExchangeStart` once every check has passed (`G/exchange.cpp:139-152`): both windows
    /// open, the asked side told first, each with the other's VID.
    #[must_use]
    pub fn start(starter_vid: u32, asked_vid: u32) -> (Self, Said) {
        let mut said = Said::default();
        let start =
            |vid: u32| GcExchange::new(EXCHANGE_SUBHEADER_GC_START, false, u64::from(vid), NPOS, 0);
        said.exchange(Side::Asked, start(starter_vid));
        said.exchange(Side::Starter, start(asked_vid));
        (Self::default(), said)
    }

    /// The `ALREADY` a starter is sent when the player it asked is trading already
    /// (`G/exchange.cpp:129-133`).
    #[must_use]
    pub fn already_record() -> GcExchange {
        GcExchange::new(EXCHANGE_SUBHEADER_GC_ALREADY, false, 0, NPOS, 0)
    }

    /// The `END` each side is sent when the trade closes, by `Cancel` (`G/exchange.cpp:704-724`).
    #[must_use]
    pub fn end_record() -> GcExchange {
        GcExchange::new(EXCHANGE_SUBHEADER_GC_END, false, 0, NPOS, 0)
    }

    /// Whether `side` accepts: `GetAcceptStatus`.
    #[must_use]
    pub fn is_accepted(&self, side: Side) -> bool {
        self.offers[side.index()].accepted
    }

    /// The gold `side` offers.
    #[must_use]
    pub fn gold(&self, side: Side) -> u64 {
        self.offers[side.index()].gold
    }

    /// The items `side` offers, in slot order.
    pub fn offered(&self, side: Side) -> impl Iterator<Item = Offered> + '_ {
        self.offers[side.index()].offered()
    }

    /// `CExchange::AddItem` (`G/exchange.cpp:187-261`): `side` offers the item it holds `at`,
    /// shown on `display`.
    ///
    /// A position that is not one, a worn item or an empty cell is refused silently, as are an
    /// item already offered and a display cell the item does not fit. An item whose anti-flags
    /// forbid giving it is refused with [`GIVE_REFUSED_NOTICE`]. Otherwise both sides stop
    /// accepting, the item is marked offered in `items`, and both are told.
    pub fn add_item(
        &mut self,
        side: Side,
        items: &mut CharacterItems,
        at: ItemPos,
        display: u8,
        protos: &ItemProtos,
    ) -> Said {
        let mut said = Said::default();
        // `GetItem` finds nothing at a position `IsValidItemPosition` refuses, and neither does
        // `item_at`.
        if is_equip_position(at) {
            return said;
        }
        let Some(item) = items.item_at(at) else {
            return said;
        };
        if item.anti_flags & ITEM_ANTIFLAG_GIVE != 0 {
            said.records[side.index()].push(TradeRecord::Notice(GIVE_REFUSED_NOTICE));
            return said;
        }
        if items.is_exchanging(item.id) || !self.offers[side.index()].is_clear(display, item.size) {
            return said;
        }
        let ported = at.window_type == EWindows::Inventory as u8
            && protos
                .get(item.vnum)
                .is_some_and(|proto| proto.item_type != ITEM_DS);
        if !ported {
            return said;
        }
        let offered = Offered {
            id: item.id,
            pos: at,
            display,
            size: item.size,
        };
        let records = [true, false].map(|is_me| item_add(item, at, display, is_me));
        self.unaccept(side, &mut said);
        self.unaccept(side.other(), &mut said);
        let offer = &mut self.offers[side.index()];
        let Some(slot) = offer.slots.iter_mut().find(|slot| slot.is_none()) else {
            return said;
        };
        *slot = Some(offered);
        offer.mark(display, offered.size, true);
        let _marked = items.set_exchanging(offered.id, true);
        let [own, other] = records;
        said.exchange(side, own);
        said.exchange(side.other(), other);
        said
    }

    /// `CExchange::RemoveItem` (`G/exchange.cpp:263-287`): `side` takes back the item in
    /// `slot`. An empty slot, or one past the last, is refused silently.
    ///
    /// Both sides are told before they stop accepting, the other side with where the item is
    /// held.
    pub fn remove_item(&mut self, side: Side, items: &mut CharacterItems, slot: u8) -> Said {
        let mut said = Said::default();
        let offer = &mut self.offers[side.index()];
        let Some(held) = offer.slots.get_mut(usize::from(slot)) else {
            return said;
        };
        let Some(offered) = held.take() else {
            return said;
        };
        offer.mark(offered.display, offered.size, false);
        let _cleared = items.set_exchanging(offered.id, false);
        let del = |is_me: bool, pos: ItemPos| {
            GcExchange::new(
                EXCHANGE_SUBHEADER_GC_ITEM_DEL,
                is_me,
                u64::from(slot),
                pos,
                0,
            )
        };
        said.exchange(side, del(true, NPOS));
        said.exchange(side.other(), del(false, offered.pos));
        self.unaccept(side, &mut said);
        self.unaccept(side.other(), &mut said);
        said
    }

    /// `CExchange::AddGold` (`G/exchange.cpp:289-313`): `side`, holding `held`, offers `amount`.
    ///
    /// No gold is refused silently; more than the side holds is answered with `LESS_GOLD`. Gold
    /// is offered once: a second amount is refused silently, which is legacy's Quirk and kept.
    pub fn add_gold(&mut self, side: Side, held: u64, amount: u64) -> Said {
        let mut said = Said::default();
        if amount == 0 {
            return said;
        }
        if held < amount {
            let less = GcExchange::new(EXCHANGE_SUBHEADER_GC_LESS_GOLD, false, 0, NPOS, 0);
            said.exchange(side, less);
            return said;
        }
        if self.offers[side.index()].gold > 0 {
            return said;
        }
        self.unaccept(side, &mut said);
        self.unaccept(side.other(), &mut said);
        self.offers[side.index()].gold = amount;
        let add =
            |is_me: bool| GcExchange::new(EXCHANGE_SUBHEADER_GC_GOLD_ADD, is_me, amount, NPOS, 0);
        said.exchange(side, add(true));
        said.exchange(side.other(), add(false));
        said
    }

    /// `CExchange::Accept(true)` (`G/exchange.cpp:598-702`).
    pub fn accept(&mut self, side: Side) -> Accepted {
        if self.offers[side.index()].accepted {
            return Accepted::Unchanged;
        }
        self.offers[side.index()].accepted = true;
        if self.offers[side.other().index()].accepted {
            return Accepted::Both;
        }
        let mut said = Said::default();
        accept_records(side, true, &mut said);
        Accepted::Waiting(said)
    }

    /// The half of `Cancel` that frees `side`'s offered items (`G/exchange.cpp:709-713`).
    pub fn withdraw(&self, side: Side, items: &mut CharacterItems) {
        for offered in self.offered(side) {
            let _cleared = items.set_exchanging(offered.id, false);
        }
    }

    /// `Accept(false)`: nothing when the side does not accept, both told otherwise.
    fn unaccept(&mut self, side: Side, said: &mut Said) {
        let offer = &mut self.offers[side.index()];
        if offer.accepted {
            offer.accepted = false;
            accept_records(side, false, said);
        }
    }
}

/// The `ACCEPT` records: the side with `is_me` set, then the other.
fn accept_records(side: Side, accepted: bool, said: &mut Said) {
    let record = |is_me: bool| {
        GcExchange::new(
            EXCHANGE_SUBHEADER_GC_ACCEPT,
            is_me,
            u64::from(accepted),
            NPOS,
            0,
        )
    };
    said.exchange(side, record(true));
    said.exchange(side.other(), record(false));
}

/// `ITEM_ADD` with the item (`exchange_packet`, `G/exchange.cpp:25-74`).
fn item_add(item: &Item, at: ItemPos, display: u8, is_me: bool) -> GcExchange {
    let shown = ItemPos::new(EWindows::ReservedWindow as u8, u16::from(display));
    let mut record = GcExchange::new(
        EXCHANGE_SUBHEADER_GC_ITEM_ADD,
        is_me,
        u64::from(item.vnum),
        shown,
        u32::from(item.count),
    );
    record.arg4 = at;
    record.sockets = item.sockets;
    record.attrs = item.attributes;
    record.refine_element = item.refine_element;
    record.transmutation = item.transmutation;
    record
}

/// One character as a settlement reads and changes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trader {
    /// Its store id, which a given row names.
    pub player_id: u32,
    /// Its storage.
    pub items: CharacterItems,
    /// Its quickslots.
    pub quickslots: Quickslots,
    /// Its gold.
    pub gold: u64,
    /// The base inventory cells it has unlocked: `Inventory_Size()`.
    pub usable_cells: u16,
}

/// A trade that went through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    /// Both characters as the trade leaves them, by [`Side::index`].
    pub traders: [Trader; 2],
    /// What each side is sent and what its rows change, by [`Side::index`]. The records end
    /// with the gold; [`COMPLETED_NOTICE`] and the `END` follow them.
    pub done: [MoveDone; 2],
    /// What the trade adds to each side's gold, negative for gold taken, by [`Side::index`].
    pub gold: [i64; 2],
}

/// A trade a settlement refused: the line each side is told before both windows close, by
/// [`Side::index`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unsettled {
    /// The lines.
    pub notices: [Option<&'static str>; 2],
}

impl Unsettled {
    /// The line `side` is told, if any.
    #[must_use]
    pub const fn notice(&self, side: Side) -> Option<&'static str> {
        self.notices[side.index()]
    }

    fn told(giver: Side, giver_line: &'static str, receiver_line: Option<&'static str>) -> Self {
        let mut notices = [None; 2];
        notices[giver.index()] = Some(giver_line);
        notices[giver.other().index()] = receiver_line;
        Self { notices }
    }
}

/// A character and what the settlement did to it.
struct Party {
    trader: Trader,
    done: MoveDone,
}

/// `CExchange::Accept` once both sides accept (`G/exchange.cpp:606-701`), closed by `closer`,
/// the side whose accept completed it, on `traders`, copies of both characters by
/// [`Side::index`].
///
/// # Errors
///
/// [`Unsettled`] with the lines of the first check that fails, in legacy's order for the closer
/// and then the other side: an item moved or the gold short ([`OUT_OF_PLACE_NOTICE`]), then no
/// room for the side's items ([`PARTNER_FULL_NOTICE`]); then gold that would take a receiver to
/// [`GOLD_MAX_MAX`], the other side first, as `Done` pays it; then an item `Done` finds no cell
/// for.
pub fn settle(
    trade: &Trade,
    closer: Side,
    traders: [Trader; 2],
    protos: &ItemProtos,
    dice: &mut dyn Dice,
) -> Result<Settled, Unsettled> {
    let partner = closer.other();
    for (giver, receiver) in [(closer, partner), (partner, closer)] {
        let offer = &trade.offers[giver.index()];
        let giving = &traders[giver.index()];
        if !offer.is_still_held(giving) {
            let partner_line = Some(PARTNER_OUT_OF_PLACE_NOTICE);
            return Err(Unsettled::told(giver, OUT_OF_PLACE_NOTICE, partner_line));
        }
        if !has_room(offer, giving, &traders[receiver.index()], protos) {
            return Err(Unsettled::told(
                giver,
                PARTNER_FULL_NOTICE,
                Some(FULL_NOTICE),
            ));
        }
    }
    let (paid, repaid) = (trade.gold(closer), trade.gold(partner));
    let partner_peak = traders[partner.index()].gold.saturating_add(paid);
    if paid > 0 && partner_peak >= GOLD_MAX_MAX {
        return Err(Unsettled::told(partner, YANG_LIMIT_NOTICE, None));
    }
    let closer_last = traders[closer.index()]
        .gold
        .saturating_sub(paid)
        .saturating_add(repaid);
    if repaid > 0 && closer_last >= GOLD_MAX_MAX {
        return Err(Unsettled::told(closer, YANG_LIMIT_NOTICE, None));
    }
    let mut parties = traders.map(|trader| Party {
        trader,
        done: MoveDone {
            kind: MoveKind::Traded,
            records: Vec::new(),
            changes: Vec::new(),
        },
    });
    for giver in [closer, partner] {
        let offer = &trade.offers[giver.index()];
        if hand_over(offer, giver, &mut parties, protos, dice).is_none() {
            return Err(Unsettled::told(
                giver,
                PARTNER_FULL_NOTICE,
                Some(FULL_NOTICE),
            ));
        }
    }
    for party in &mut parties {
        sync_quickslots(
            &mut party.done,
            &mut party.trader.quickslots,
            &party.trader.items,
        );
    }
    let gold = [Side::Starter, Side::Asked]
        .map(|side| signed(trade.gold(side.other())) - signed(trade.gold(side)));
    let [starter, asked] = parties;
    Ok(Settled {
        traders: [starter.trader, asked.trader],
        done: [starter.done, asked.done],
        gold,
    })
}

/// `CExchange::CheckSpace` (`G/exchange.cpp:340-510`): every item `giver` offers finds a cell
/// in a copy of the receiver's storage, as `Done` would place it.
fn has_room(offer: &Offer, giver: &Trader, receiver: &Trader, protos: &ItemProtos) -> bool {
    let mut room = receiver.items.clone();
    offer.offered().all(|offered| {
        let Some(item) = giver.items.item(offered.id) else {
            return false;
        };
        let Some(proto) = protos.get(item.vnum) else {
            return false;
        };
        empty_cell(&room, proto, item.size, receiver.usable_cells)
            .is_some_and(|pos| room.set(pos, item).is_ok())
    })
}

/// `GetEmptyInventory(item)`: the first free cell of a custom bank the item belongs to, then
/// of the unlocked base inventory.
fn empty_cell(
    items: &CharacterItems,
    proto: &ItemProto,
    size: u8,
    usable_cells: u16,
) -> Option<ItemPos> {
    (0..CATEGORY_NUM)
        .filter(|bank| is_custom_category(proto, *bank))
        .find_map(|bank| items.find_free_custom_cell(bank, size))
        .or_else(|| items.find_free_inventory_cell(usable_cells, size))
        .map(|cell| ItemPos::new(EWindows::Inventory as u8, cell))
}

/// `CExchange::Done` for `giver` (`G/exchange.cpp:512-596`): each offered item in slot order
/// goes to the other party, then the gold. `None` when an item finds no cell.
fn hand_over(
    offer: &Offer,
    giver: Side,
    parties: &mut [Party; 2],
    protos: &ItemProtos,
    dice: &mut dyn Dice,
) -> Option<()> {
    let [starter, asked] = parties;
    let (from, to) = match giver {
        Side::Starter => (starter, asked),
        Side::Asked => (asked, starter),
    };
    for offered in offer.offered() {
        let mut item = from.trader.items.item(offered.id)?.clone();
        let proto = protos.get(item.vnum)?;
        let pos = empty_cell(&to.trader.items, proto, item.size, to.trader.usable_cells)?;
        let left = from.trader.items.release(item.id).ok()?;
        from.done
            .records
            .push(MoveRecord::QuickslotSync(QuickslotSync {
                from: left.cell,
                to: SyncTo::Delete,
            }));
        from.done
            .records
            .push(MoveRecord::Item(ItemRecord::Set(gc_item_clear(left))));
        let rolled = roll_sash(&mut item, proto, dice);
        to.trader.items.set(pos, &item).ok()?;
        to.done
            .records
            .push(MoveRecord::Item(ItemRecord::Set(item.gc_item_set(pos, 1))));
        if rolled {
            from.done.changes.push(ItemChange::Sockets {
                id: item.id,
                sockets: item.sockets,
            });
        }
        from.done.changes.push(ItemChange::Given {
            id: item.id,
            to: to.trader.player_id,
            pos,
        });
    }
    if offer.gold > 0 {
        from.trader.gold = from.trader.gold.saturating_sub(offer.gold);
        to.trader.gold = to.trader.gold.saturating_add(offer.gold);
        from.done.records.push(MoveRecord::Gold {
            amount: 0,
            value: from.trader.gold,
        });
        to.done.records.push(MoveRecord::Gold {
            amount: signed(offer.gold),
            value: to.trader.gold,
        });
    }
    Some(())
}

/// Gold as the signed amount a record and the store take. A settlement's checks hold every
/// amount it moves below [`GOLD_MAX_MAX`], so this never saturates.
fn signed(gold: u64) -> i64 {
    i64::try_from(gold).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::dice::Dice;
    use crate::character::quickslot::{Quickslot, QuickslotRecord};
    use common::enums::EQuickSlotType;
    use common::item_slots::{
        CUSTOM_INVENTORY_MAX_NUM, CUSTOM_INVENTORY_SLOT_START, INVENTORY_MAX_NUM,
    };
    use gamedata::item_kind::{COSTUME_SASH, ITEM_COSTUME};
    use protocol::gc_item_window::ItemAttribute;

    const INV: u8 = EWindows::Inventory as u8;
    const SWORD: u32 = 19;
    const ARMOUR: u32 = 11_200;
    const TALL: u32 = 11_400;
    const STONE: u32 = 110_000;
    const SASH: u32 = 85_004;
    const BANKED: u32 = 70_800;
    const NO_PROTO: u32 = 4;

    struct Fixed(u32);

    impl Dice for Fixed {
        fn random31(&mut self) -> u32 {
            self.0
        }
    }

    fn inv(cell: u16) -> ItemPos {
        ItemPos::new(INV, cell)
    }

    fn protos() -> ItemProtos {
        let mut armour = ItemProto::for_category_rule(ARMOUR, 2, 0);
        armour.size = 2;
        let mut tall = ItemProto::for_category_rule(TALL, 2, 0);
        tall.size = 3;
        let mut sash = ItemProto::for_category_rule(SASH, ITEM_COSTUME, COSTUME_SASH);
        sash.values[0] = 4;
        ItemProtos::from_rows(vec![
            ItemProto::for_category_rule(SWORD, 1, 0),
            armour,
            tall,
            ItemProto::for_category_rule(STONE, ITEM_DS, 0),
            sash,
            ItemProto::for_category_rule(BANKED, 0, 0),
        ])
    }

    fn item(id: u32, vnum: u32, size: u8) -> Item {
        let mut item = Item::new(id, vnum);
        item.size = size;
        item
    }

    fn holding(placed: &[(u16, Item)]) -> CharacterItems {
        let mut items = CharacterItems::new();
        for (cell, item) in placed {
            items.set(inv(*cell), item).expect("the fixture places");
        }
        items
    }

    fn trader(player_id: u32, items: CharacterItems, gold: u64) -> Trader {
        Trader {
            player_id,
            items,
            quickslots: Quickslots::default(),
            gold,
            usable_cells: 90,
        }
    }

    fn exchange(sub: u8, is_me: bool, arg1: u64, arg2: ItemPos) -> TradeRecord {
        TradeRecord::Exchange(GcExchange::new(sub, is_me, arg1, arg2, 0))
    }

    fn accept_record(is_me: bool, accepted: bool) -> TradeRecord {
        exchange(
            EXCHANGE_SUBHEADER_GC_ACCEPT,
            is_me,
            u64::from(accepted),
            NPOS,
        )
    }

    fn offering(items: &mut CharacterItems, side: Side, at: u16, display: u8) -> Trade {
        let (mut trade, _said) = Trade::start(10, 20);
        let _said = trade.add_item(side, items, inv(at), display, &protos());
        trade
    }

    #[test]
    fn a_trade_opens_with_each_side_told_the_others_vid() {
        let (trade, said) = Trade::start(10, 20);
        let start = |vid| exchange(EXCHANGE_SUBHEADER_GC_START, false, vid, NPOS);
        assert_eq!(said.records_to(Side::Asked), [start(10)]);
        assert_eq!(said.records_to(Side::Starter), [start(20)]);
        assert!(!trade.is_accepted(Side::Starter) && !trade.is_accepted(Side::Asked));
        let end = Trade::end_record();
        assert_eq!(
            (end.sub_header, end.is_me, end.arg1, end.arg2),
            (5, 0, 0, NPOS)
        );
        let already = Trade::already_record();
        assert_eq!(
            (already.sub_header, already.is_me, already.arg2),
            (6, 0, NPOS)
        );
        assert_eq!(usize::from(EXCHANGE_ITEM_MAX_NUM), SLOTS);
        assert_eq!(GRID_CELLS, SLOTS);
    }

    #[test]
    fn the_display_window_is_six_cells_wide_and_four_tall() {
        let offer = Offer::default();
        for (display, size, fits) in [
            (0, 4, true),
            (6, 3, true),
            (12, 3, false),
            (12, 2, true),
            (18, 2, false),
            (18, 1, true),
            (23, 1, true),
            (24, 1, false),
            (255, 1, false),
            // An item of size 0 covers no cell, and `IsEmpty` still refuses a display cell
            // whose row is past the bottom.
            (24, 0, true),
            (29, 0, true),
            (30, 0, false),
        ] {
            assert_eq!(offer.is_clear(display, size), fits, "{display} x {size}");
        }
        let mut offer = Offer::default();
        offer.mark(1, 3, true);
        assert!(!offer.is_clear(13, 1) && !offer.is_clear(1, 1) && !offer.is_clear(7, 1));
        assert!(offer.is_clear(0, 3) && offer.is_clear(2, 3) && offer.is_clear(19, 1));
        offer.mark(1, 3, false);
        assert!(offer.is_clear(1, 3));
    }

    #[test]
    fn an_offered_item_is_shown_to_both_sides_with_its_data() {
        let mut sword = item(7, SWORD, 1);
        sword.count = 3;
        sword.sockets = [1, 2, 3, 4, 5, 6];
        sword.attributes[0] = ItemAttribute::new(9, 17);
        sword.refine_element = 21;
        sword.transmutation = 22;
        let mut items = holding(&[(4, sword.clone())]);
        let (mut trade, _said) = Trade::start(10, 20);
        let said = trade.add_item(Side::Starter, &mut items, inv(4), 7, &protos());
        let expected = |is_me| TradeRecord::Exchange(item_add(&sword, inv(4), 7, is_me));
        assert_eq!(said.records_to(Side::Starter), [expected(true)]);
        assert_eq!(said.records_to(Side::Asked), [expected(false)]);
        let TradeRecord::Exchange(record) = expected(true) else {
            unreachable!("an exchange record");
        };
        assert_eq!(
            (record.sub_header, record.is_me, record.arg1),
            (1, 1, u64::from(SWORD))
        );
        assert_eq!(
            (record.arg2, record.arg3, record.arg4),
            (ItemPos::new(0, 7), 3, inv(4))
        );
        assert_eq!(
            (record.sockets, record.attrs),
            (sword.sockets, sword.attributes)
        );
        assert_eq!((record.refine_element, record.transmutation), (21, 22));
        assert!(items.is_exchanging(7));
        let offered: Vec<Offered> = trade.offered(Side::Starter).collect();
        let shown = Offered {
            id: 7,
            pos: inv(4),
            display: 7,
            size: 1,
        };
        assert_eq!(offered, [shown]);
        assert_eq!(trade.offered(Side::Asked).count(), 0);
    }

    #[test]
    fn an_item_that_cannot_be_given_is_refused_with_a_line() {
        let mut bound = item(7, SWORD, 1);
        bound.anti_flags = ITEM_ANTIFLAG_GIVE;
        let mut items = holding(&[(4, bound)]);
        let (mut trade, _said) = Trade::start(10, 20);
        let said = trade.add_item(Side::Starter, &mut items, inv(4), 0, &protos());
        let line = TradeRecord::Notice(GIVE_REFUSED_NOTICE);
        assert_eq!(said.records_to(Side::Starter), [line]);
        assert!(said.records_to(Side::Asked).is_empty());
        assert!(!items.is_exchanging(7) && trade.offered(Side::Starter).count() == 0);
    }

    #[test]
    fn every_other_refused_offer_is_silent_and_changes_nothing() {
        let mut items = holding(&[
            (0, item(1, SWORD, 1)),
            (1, item(2, ARMOUR, 2)),
            (2, item(3, STONE, 1)),
            (3, item(4, NO_PROTO, 1)),
        ]);
        let ds = ItemPos::new(EWindows::DragonSoulInventory as u8, 0);
        items.set(ds, &item(5, SWORD, 1)).expect("places");
        let worn = inv(INVENTORY_MAX_NUM + 4);
        items.set(worn, &item(6, SWORD, 1)).expect("places");
        let (mut trade, _said) = Trade::start(10, 20);
        let protos = protos();
        let _said = trade.add_item(Side::Starter, &mut items, inv(0), 0, &protos);
        let before = trade.clone();
        for (at, display) in [
            (ItemPos::new(99, 0), 1),
            (inv(u16::MAX), 1),
            (worn, 1),
            (inv(40), 1),
            (inv(0), 1),
            (inv(1), 0),
            (inv(1), 18),
            (inv(2), 1),
            (inv(3), 1),
            (ds, 1),
        ] {
            let said = trade.add_item(Side::Starter, &mut items, at, display, &protos);
            assert_eq!(said, Said::default(), "{at:?} on {display}");
        }
        assert_eq!(trade, before);
        assert!([2, 3, 4, 5, 6].iter().all(|id| !items.is_exchanging(*id)));
        let said = trade.add_item(Side::Starter, &mut items, inv(1), 1, &protos);
        assert_eq!(
            said.records_to(Side::Starter).len(),
            1,
            "the control is offered"
        );
    }

    #[test]
    fn an_offer_stops_both_sides_accepting_first() {
        let mut items = holding(&[
            (0, item(1, SWORD, 1)),
            (1, item(2, SWORD, 1)),
            (2, item(3, SWORD, 1)),
        ]);
        let mut trade = offering(&mut items, Side::Starter, 0, 0);
        let Accepted::Waiting(said) = trade.accept(Side::Starter) else {
            panic!("the other side does not accept yet");
        };
        assert_eq!(said.records_to(Side::Starter), [accept_record(true, true)]);
        assert_eq!(said.records_to(Side::Asked), [accept_record(false, true)]);
        assert_eq!(trade.accept(Side::Starter), Accepted::Unchanged);
        let said = trade.add_item(Side::Starter, &mut items, inv(1), 1, &protos());
        let own = said.records_to(Side::Starter);
        assert_eq!(own.first(), Some(&accept_record(true, false)));
        assert!(matches!(own.get(1), Some(TradeRecord::Exchange(r)) if r.sub_header == 1));
        let other = said.records_to(Side::Asked);
        assert_eq!(other.first(), Some(&accept_record(false, false)));
        assert_eq!(other.len(), 2);
        assert!(!trade.is_accepted(Side::Starter));
        assert!(matches!(trade.accept(Side::Asked), Accepted::Waiting(_)));
        let said = trade.add_item(Side::Starter, &mut items, inv(2), 2, &protos());
        assert_eq!(
            said.records_to(Side::Asked).first(),
            Some(&accept_record(true, false))
        );
        assert_eq!(
            said.records_to(Side::Starter).first(),
            Some(&accept_record(false, false))
        );
        assert!(!trade.is_accepted(Side::Asked));
        assert_eq!(trade.accept(Side::Starter), Accepted::Waiting(accepting()));
        assert_eq!(trade.accept(Side::Asked), Accepted::Both);
    }

    fn accepting() -> Said {
        let mut said = Said::default();
        accept_records(Side::Starter, true, &mut said);
        said
    }

    #[test]
    fn an_item_taken_back_is_told_before_the_accepts_drop() {
        let mut items = holding(&[(4, item(7, ARMOUR, 2))]);
        let mut trade = offering(&mut items, Side::Asked, 4, 0);
        assert!(matches!(trade.accept(Side::Asked), Accepted::Waiting(_)));
        for slot in [1, 23, 24, 255] {
            let said = trade.remove_item(Side::Asked, &mut items, slot);
            assert_eq!(said, Said::default(), "slot {slot}");
        }
        let said = trade.remove_item(Side::Asked, &mut items, 0);
        let del = |is_me, pos| exchange(EXCHANGE_SUBHEADER_GC_ITEM_DEL, is_me, 0, pos);
        let own = [del(true, NPOS), accept_record(true, false)];
        assert_eq!(said.records_to(Side::Asked), own);
        let other = [del(false, inv(4)), accept_record(false, false)];
        assert_eq!(said.records_to(Side::Starter), other);
        assert!(!items.is_exchanging(7));
        assert_eq!(trade.offered(Side::Asked).count(), 0);
        assert!(trade.offers[Side::Asked.index()].is_clear(0, 4));
        assert_eq!(
            trade.remove_item(Side::Asked, &mut items, 0),
            Said::default()
        );
        // A second offer takes the second slot, and taking it back names that slot.
        items
            .set(inv(10), &item(9, SWORD, 1))
            .expect("the cell is free");
        let _said = trade.add_item(Side::Asked, &mut items, inv(4), 0, &protos());
        let _said = trade.add_item(Side::Asked, &mut items, inv(10), 5, &protos());
        let said = trade.remove_item(Side::Asked, &mut items, 1);
        let del = |is_me, pos| exchange(EXCHANGE_SUBHEADER_GC_ITEM_DEL, is_me, 1, pos);
        assert_eq!(said.records_to(Side::Asked), [del(true, NPOS)]);
        assert_eq!(said.records_to(Side::Starter), [del(false, inv(10))]);
        assert!(items.is_exchanging(7) && !items.is_exchanging(9));
    }

    #[test]
    fn gold_is_offered_once_and_never_more_than_held() {
        let (mut trade, _said) = Trade::start(10, 20);
        assert_eq!(trade.add_gold(Side::Starter, 100, 0), Said::default());
        let said = trade.add_gold(Side::Starter, 100, 101);
        let less = exchange(EXCHANGE_SUBHEADER_GC_LESS_GOLD, false, 0, NPOS);
        assert_eq!(said.records_to(Side::Starter), [less]);
        assert!(said.records_to(Side::Asked).is_empty());
        assert!(matches!(trade.accept(Side::Asked), Accepted::Waiting(_)));
        let said = trade.add_gold(Side::Starter, 100, 100);
        let add = |is_me| exchange(EXCHANGE_SUBHEADER_GC_GOLD_ADD, is_me, 100, NPOS);
        assert_eq!(
            said.records_to(Side::Starter),
            [accept_record(false, false), add(true)]
        );
        assert_eq!(
            said.records_to(Side::Asked),
            [accept_record(true, false), add(false)]
        );
        assert_eq!(trade.add_gold(Side::Starter, 100, 20), Said::default());
        assert_eq!(
            (trade.gold(Side::Starter), trade.gold(Side::Asked)),
            (100, 0)
        );
    }

    #[test]
    fn a_withdrawn_offer_frees_its_items() {
        let mut items = holding(&[(0, item(1, SWORD, 1)), (1, item(2, SWORD, 1))]);
        let mut trade = offering(&mut items, Side::Starter, 0, 0);
        let _said = trade.add_item(Side::Starter, &mut items, inv(1), 1, &protos());
        assert!(items.is_exchanging(1) && items.is_exchanging(2));
        trade.withdraw(Side::Asked, &mut items);
        assert!(items.is_exchanging(1) && items.is_exchanging(2));
        trade.withdraw(Side::Starter, &mut items);
        assert!(!items.is_exchanging(1) && !items.is_exchanging(2));
    }

    fn both_ways() -> (Trade, [Trader; 2]) {
        let mut starter_items = holding(&[(3, item(7, SWORD, 1))]);
        let mut asked_items = holding(&[(0, item(8, ARMOUR, 2))]);
        let (mut trade, _said) = Trade::start(10, 20);
        let protos = protos();
        let _said = trade.add_item(Side::Starter, &mut starter_items, inv(3), 0, &protos);
        let _said = trade.add_gold(Side::Starter, 1000, 300);
        let _said = trade.add_item(Side::Asked, &mut asked_items, inv(0), 1, &protos);
        let _said = trade.add_gold(Side::Asked, 500, 100);
        let mut starter = trader(1, starter_items, 1000);
        let named = Quickslot {
            kind: EQuickSlotType::Item as u8,
            pos: 3,
        };
        let _set = starter.quickslots.set(5, named, &mut Vec::new());
        (trade, [starter, trader(2, asked_items, 500)])
    }

    #[test]
    fn a_settled_trade_moves_the_items_and_gold_both_ways() {
        let (trade, traders) = both_ways();
        let settled =
            settle(&trade, Side::Asked, traders, &protos(), &mut Fixed(0)).expect("settles");
        let [starter, asked] = &settled.done;
        let set = |item: Item, pos| MoveRecord::Item(ItemRecord::Set(item.gc_item_set(pos, 1)));
        let clear = |pos| MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos)));
        let gold = |amount, value| MoveRecord::Gold { amount, value };
        let mut sword = item(7, SWORD, 1);
        sword.pos = inv(0);
        let mut armour = item(8, ARMOUR, 2);
        armour.pos = inv(0);
        let asked_records = [
            clear(inv(0)),
            gold(0, 400),
            set(sword, inv(0)),
            gold(300, 700),
        ];
        assert_eq!(asked.records, asked_records);
        let deleted = MoveRecord::Quickslot(QuickslotRecord::Del { slot: 5 });
        let starter_records = [
            set(armour, inv(0)),
            gold(100, 1100),
            deleted,
            clear(inv(3)),
            gold(0, 800),
        ];
        assert_eq!(starter.records, starter_records);
        let given = |id, to| ItemChange::Given {
            id,
            to,
            pos: inv(0),
        };
        assert_eq!(asked.changes, [given(8, 1)]);
        assert_eq!(starter.changes, [given(7, 2)]);
        assert_eq!(
            (starter.kind, asked.kind),
            (MoveKind::Traded, MoveKind::Traded)
        );
        assert_eq!(settled.gold, [-200, 200]);
        let [starter, asked] = &settled.traders;
        assert_eq!((starter.gold, asked.gold), (800, 700));
        assert_eq!(starter.items.item_at(inv(0)).map(|item| item.id), Some(8));
        assert_eq!(asked.items.item_at(inv(0)).map(|item| item.id), Some(7));
        assert!(starter.items.item(7).is_none() && asked.items.item(8).is_none());
        assert!(!starter.items.is_exchanging(8) && !asked.items.is_exchanging(7));
        assert_eq!(starter.quickslots.get(5), Some(Quickslot::default()));
    }

    #[test]
    fn the_closer_hands_over_first() {
        let (trade, traders) = both_ways();
        let settled =
            settle(&trade, Side::Starter, traders, &protos(), &mut Fixed(0)).expect("settles");
        let [starter, asked] = &settled.traders;
        assert_eq!(
            asked.items.cell_of(7),
            Some(inv(1)),
            "beside the armour it still holds"
        );
        assert_eq!(starter.items.cell_of(8), Some(inv(0)));
        let gold = |amount, value| MoveRecord::Gold { amount, value };
        assert_eq!(settled.done[1].records.get(1), Some(&gold(300, 800)));
        assert_eq!(settled.done[1].records.last(), Some(&gold(0, 700)));
        assert_eq!(settled.done[0].records.last(), Some(&gold(100, 800)));
    }

    #[test]
    fn a_banked_item_goes_to_its_bank_and_a_sash_rolls_as_it_arrives() {
        let mut items = holding(&[(0, item(7, SASH, 1)), (1, item(9, BANKED, 1))]);
        let mut trade = offering(&mut items, Side::Starter, 0, 0);
        let _said = trade.add_item(Side::Starter, &mut items, inv(1), 1, &protos());
        let traders = [trader(1, items, 0), trader(2, CharacterItems::new(), 0)];
        let settled =
            settle(&trade, Side::Starter, traders, &protos(), &mut Fixed(3)).expect("settles");
        let received = settled.traders[1].items.item(7).expect("received");
        assert_eq!(received.sockets[0], 14, "number(11, 19) with the draw 3");
        let sockets = received.sockets;
        let [rolled, given, banked] = settled.done[0].changes.as_slice() else {
            panic!("three changes: {:?}", settled.done[0].changes);
        };
        assert_eq!(*rolled, ItemChange::Sockets { id: 7, sockets });
        let costumes = inv(CUSTOM_INVENTORY_SLOT_START + 5 * CUSTOM_INVENTORY_MAX_NUM);
        assert!(matches!(given, ItemChange::Given { id: 7, to: 2, pos } if *pos == costumes));
        let bank = inv(CUSTOM_INVENTORY_SLOT_START);
        assert!(matches!(banked, ItemChange::Given { id: 9, to: 2, pos } if *pos == bank));
        assert_eq!(settled.traders[1].items.cell_of(7), Some(costumes));
    }

    #[test]
    fn a_moved_item_or_short_gold_refuses_with_the_out_of_place_lines() {
        let (trade, [starter, asked]) = both_ways();
        let mut moved = asked.clone();
        let _gone = moved.items.release(8).expect("held");
        let refused = settle(
            &trade,
            Side::Asked,
            [starter.clone(), moved],
            &protos(),
            &mut Fixed(0),
        );
        let told = Unsettled::told(
            Side::Asked,
            OUT_OF_PLACE_NOTICE,
            Some(PARTNER_OUT_OF_PLACE_NOTICE),
        );
        assert_eq!(refused, Err(told));
        // Another item in the offered item's cell is not the item offered.
        let mut replaced = asked.clone();
        let _gone = replaced.items.release(8).expect("held");
        replaced
            .items
            .set(inv(0), &item(9, ARMOUR, 2))
            .expect("the cell is free");
        let refused = settle(
            &trade,
            Side::Asked,
            [starter.clone(), replaced],
            &protos(),
            &mut Fixed(0),
        );
        assert_eq!(refused, Err(told));
        // An offered position that is not one holds nothing.
        let mut nowhere = trade.clone();
        let offered = nowhere.offers[Side::Asked.index()].slots[0]
            .as_mut()
            .expect("offered");
        offered.pos = inv(u16::MAX);
        let refused = settle(
            &nowhere,
            Side::Asked,
            [starter.clone(), asked.clone()],
            &protos(),
            &mut Fixed(0),
        );
        assert_eq!(refused, Err(told));
        let mut short = starter.clone();
        short.gold = 299;
        let refused = settle(
            &trade,
            Side::Asked,
            [short, asked.clone()],
            &protos(),
            &mut Fixed(0),
        );
        let told = Unsettled::told(
            Side::Starter,
            OUT_OF_PLACE_NOTICE,
            Some(PARTNER_OUT_OF_PLACE_NOTICE),
        );
        assert_eq!(refused, Err(told));
        assert_eq!(told.notice(Side::Asked), Some(PARTNER_OUT_OF_PLACE_NOTICE));
        // When both sides fail, the side whose accept completed the trade is checked first.
        let mut moved = asked.clone();
        let _gone = moved.items.release(8).expect("held");
        let mut short = starter.clone();
        short.gold = 299;
        let refused = settle(
            &trade,
            Side::Asked,
            [short, moved],
            &protos(),
            &mut Fixed(0),
        );
        let closer_told = Unsettled::told(
            Side::Asked,
            OUT_OF_PLACE_NOTICE,
            Some(PARTNER_OUT_OF_PLACE_NOTICE),
        );
        assert_eq!(refused, Err(closer_told));
        let mut exact = starter;
        exact.gold = 300;
        assert!(settle(
            &trade,
            Side::Asked,
            [exact, asked],
            &protos(),
            &mut Fixed(0)
        )
        .is_ok());
    }

    #[test]
    fn no_room_refuses_with_the_full_lines() {
        let (trade, [starter, mut asked]) = both_ways();
        asked.usable_cells = 1;
        let refused = settle(
            &trade,
            Side::Asked,
            [starter.clone(), asked.clone()],
            &protos(),
            &mut Fixed(0),
        );
        let told = Unsettled::told(Side::Starter, PARTNER_FULL_NOTICE, Some(FULL_NOTICE));
        assert_eq!(refused, Err(told));
        // Two items that each fit the one free cell do not fit it together, and the closer's
        // lack of room is found before the other side's short gold.
        let mut items = holding(&[(0, item(1, SWORD, 1)), (1, item(2, SWORD, 1))]);
        let (mut pair, _said) = Trade::start(10, 20);
        let _said = pair.add_item(Side::Starter, &mut items, inv(0), 0, &protos());
        let _said = pair.add_item(Side::Starter, &mut items, inv(1), 1, &protos());
        let _said = pair.add_gold(Side::Asked, 500, 100);
        let mut receiver = trader(2, CharacterItems::new(), 99);
        receiver.usable_cells = 1;
        let refused = settle(
            &pair,
            Side::Starter,
            [trader(1, items, 0), receiver],
            &protos(),
            &mut Fixed(0),
        );
        let closer_full = Unsettled::told(Side::Starter, PARTNER_FULL_NOTICE, Some(FULL_NOTICE));
        assert_eq!(refused, Err(closer_full));
        asked.usable_cells = 2;
        assert!(settle(
            &trade,
            Side::Asked,
            [starter, asked],
            &protos(),
            &mut Fixed(0)
        )
        .is_ok());
    }

    #[test]
    fn gold_that_would_reach_the_cap_refuses_before_anything_moves() {
        let (trade, [starter, asked]) = both_ways();
        let run = |starter_gold: u64, asked_gold: u64| {
            let mut pair = [starter.clone(), asked.clone()];
            pair[0].gold = starter_gold;
            pair[1].gold = asked_gold;
            settle(&trade, Side::Asked, pair, &protos(), &mut Fixed(0))
        };
        // The asked side closes: the starter receives its 100 first, then pays its 300.
        let starter_capped = Unsettled::told(Side::Starter, YANG_LIMIT_NOTICE, None);
        assert_eq!(run(GOLD_MAX_MAX - 100, 500), Err(starter_capped));
        assert_eq!(starter_capped.notice(Side::Asked), None);
        assert!(run(GOLD_MAX_MAX - 101, 500).is_ok());
        // The asked side pays its 100, then receives the 300.
        let asked_capped = Unsettled::told(Side::Asked, YANG_LIMIT_NOTICE, None);
        assert_eq!(run(1000, GOLD_MAX_MAX - 200), Err(asked_capped));
        assert!(run(1000, GOLD_MAX_MAX - 201).is_ok());
        assert_eq!(
            run(GOLD_MAX_MAX - 100, GOLD_MAX_MAX - 200),
            Err(starter_capped)
        );
    }

    #[test]
    fn gold_not_offered_is_never_checked_against_the_cap() {
        let mut items = holding(&[(0, item(7, SWORD, 1))]);
        let trade = offering(&mut items, Side::Starter, 0, 0);
        let full = [
            trader(1, items, GOLD_MAX_MAX),
            trader(2, CharacterItems::new(), GOLD_MAX_MAX),
        ];
        let settled =
            settle(&trade, Side::Starter, full, &protos(), &mut Fixed(0)).expect("settles");
        assert_eq!(settled.gold, [0, 0]);
        assert!(settled.done.iter().all(|done| !done
            .records
            .iter()
            .any(|record| matches!(record, MoveRecord::Gold { .. }))));
    }

    /// The closer holds 45 unlocked cells, all taken but 7, 10, 12, 15 and 20, and offers the
    /// item in cell 5. The other side offers a two-cell item, then a three-cell one. Before
    /// cell 5 is freed they fit at 7 and 10; after, the first takes 5 and 10 and the second
    /// has nowhere to go.
    fn crowded(usable_cells: u16) -> (Trade, [Trader; 2]) {
        let free = [5, 7, 10, 12, 15, 20];
        let fillers: Vec<(u16, Item)> = (0..45)
            .filter(|cell| !free.contains(cell))
            .map(|cell| (cell, item(100 + u32::from(cell), SWORD, 1)))
            .collect();
        let mut closer_items = holding(&fillers);
        closer_items
            .set(inv(5), &item(7, SWORD, 1))
            .expect("places");
        let mut other_items = holding(&[(0, item(8, ARMOUR, 2)), (1, item(9, TALL, 3))]);
        let mut trade = offering(&mut closer_items, Side::Starter, 5, 0);
        let protos = protos();
        let _said = trade.add_item(Side::Asked, &mut other_items, inv(0), 0, &protos);
        let _said = trade.add_item(Side::Asked, &mut other_items, inv(1), 1, &protos);
        let mut closer = trader(1, closer_items, 0);
        closer.usable_cells = usable_cells;
        (trade, [closer, trader(2, other_items, 0)])
    }

    #[test]
    fn an_item_with_no_cell_once_cells_are_freed_refuses_the_whole_trade() {
        let (trade, traders) = crowded(45);
        let refused = settle(&trade, Side::Starter, traders, &protos(), &mut Fixed(0));
        let told = Unsettled::told(Side::Asked, PARTNER_FULL_NOTICE, Some(FULL_NOTICE));
        assert_eq!(refused, Err(told));
        let (trade, traders) = crowded(90);
        let settled =
            settle(&trade, Side::Starter, traders, &protos(), &mut Fixed(0)).expect("settles");
        let closer = &settled.traders[0];
        assert_eq!(closer.items.cell_of(8), Some(inv(5)));
        assert_eq!(closer.items.cell_of(9), Some(inv(45)), "the next page");
    }

    #[test]
    fn a_side_is_the_other_of_its_other() {
        for side in [Side::Starter, Side::Asked] {
            assert_eq!(side.other().other(), side);
            assert_ne!(side.other().index(), side.index());
        }
        assert_eq!(Side::Starter.index(), 0);
    }

    #[test]
    fn signed_gold_saturates_only_past_the_signed_range() {
        assert_eq!(signed(GOLD_MAX_MAX), 1_200_000_000_000_000_000);
        assert_eq!(signed(u64::MAX), i64::MAX);
    }
}
