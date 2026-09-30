//! Buying from an NPC's shop and selling to one: `CShop::Buy` (`G/shop.cpp:610-810`) for a
//! shop no player keeps, and `CShopManager::Sell` (`G/shop_manager.cpp:456-594`).
//!
//! These change one character's storage and gold and answer with the records and row
//! changes, as a move does. Which shop is open, how far its keeper stands and what a stranger
//! pays are the caller's: the shop is not a character's.
//!
//! # What a buy does
//!
//! The price is checked, then the item is made as `CreateItem` makes it with no magic
//! (`G/item_manager.cpp:160-455`): a stackable item takes the slot's count held between 1 and
//! the stack limit, anything else takes 1, and the proto's `bGainSocketPct` opens that many
//! sockets, at most six. It goes whole to the first free cell of the unlocked base inventory
//! (`GetEmptyInventory(BYTE)`, `G/char_item.cpp:1258-1268`), never onto a stack. The gold
//! record comes first, then the cell's.
//!
//! # What a sale does
//!
//! A count of 0 or more than the stack sells the whole item. The price is the proto's
//! `dwShopBuyPrice` for each one, or so many for one gold under `ITEM_FLAG_COUNT_PER_1GOLD`,
//! then a fifth of that, less the 3 % tax. The records are the tax line, the cell's, then the
//! gold's.
//!
//! # Divergences
//!
//! - **Gold past the cap.** Legacy takes the item and then calls `ChangeGold`, which refuses
//!   gold that would reach [`GOLD_MAX_MAX`] and pays nothing, so the item is lost. That is a
//!   Defect, not reproduced: [`sell_item`] refuses before anything changes.
//! - **What `CreateItem` does by name.** The paths in [`UnportedCreation`] fill sockets,
//!   attributes or timers the Rewrite does not keep yet. A buy that would take one is refused
//!   with [`ShopRefused::NotPorted`] rather than making a different item. No owner shop sells
//!   one. The mask table (`ori_to_new_table.txt`) is not read either; no owner shop item is in
//!   it.
//! - **The item id.** Legacy numbers the item before it looks for a cell, and a full inventory
//!   burns the number. The Rewrite numbers it once it has a cell. No client sees an id.
//! - **The tax in 64 bits.** Legacy keeps the tax in a `DWORD`, so a sale worth more than
//!   about 143 billion before the tax pays too much. The Rewrite keeps it in 64 bits. No owner
//!   item comes near it.

use common::item_slots::EWindows;
use gamedata::item_kind::{
    COSTUME_AURA, ITEM_BLEND, ITEM_COSTUME, ITEM_DS, ITEM_ELK, ITEM_UNIQUE, LIMIT_REAL_TIME,
    LIMIT_TIMER_BASED_ON_WEAR,
};
use gamedata::item_proto::{ItemProto, ItemProtos};
use gamedata::npc_shop::{ShopSlot, ITEM_FLAG_COUNT_PER_1GOLD};
use protocol::gc_shop::{
    SHOP_SUBHEADER_GC_INVENTORY_FULL, SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY,
    SHOP_SUBHEADER_GC_SOLD_OUT,
};
use protocol::item_pos::ItemPos;

use super::inventory::is_equip_position;
use super::item_move::{ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord, MoveRules};
use super::items::{CharacterItems, CountRefused, Rejected};
use super::quickslot::{QuickslotSync, SyncTo};
use crate::item::{gc_item_clear, Item, ItemIds, ITEM_ANTIFLAG_SELL, ITEM_FLAG_STACKABLE, SOCKETS};

/// `GOLD_MAX_MAX` (`common/length.h:100`): the gold `ChangeGold` never lets a character reach.
pub const GOLD_MAX_MAX: u64 = 1_200_000_000_000_000_000;

/// `[LS;881;%d]` with the sale tax of 3 (`G/shop_manager.cpp:571`).
pub const SALE_TAX_NOTICE: &str = "[LS;881;3]";

/// `[LS;1059]`: a worn item is not for sale (`G/shop_manager.cpp:515`).
pub const WORN_NOTICE: &str = "[LS;1059]";

/// `iVal`, the sale tax in percent (`G/shop_manager.cpp:548`).
const SALE_TAX_PERCENT: u64 = 3;

/// The auto potions and the skill books `CreateItem` fills a socket of by vnum
/// (`G/item_manager.cpp:222-247`, `:333-384`, `G/unique_item.h:44-104`).
const SOCKETED_VNUMS: [u32; 15] = [
    72_723, 72_724, 72_725, 72_726, 72_727, 72_728, 72_729, 72_730, 76_004, 76_005, 76_021, 76_022,
    50_300, 70_037, 70_055,
];

/// A path of `CreateItem` a bought item would take that the Rewrite does not port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnportedCreation {
    /// `ITEM_ELK`: gold as an item, which is neither numbered nor counted.
    Elk,
    /// `ITEM_UNIQUE`: the remaining time in socket 2 and the expiry event.
    Unique,
    /// `ITEM_DS`: `DragonSoulItemInitialize` and the dragon soul inventory.
    DragonSoul,
    /// `ITEM_BLEND`: the blend table's bonus.
    Blend,
    /// `COSTUME_AURA`: the aura level in a socket.
    Aura,
    /// A `LIMIT_REAL_TIME` or `LIMIT_TIMER_BASED_ON_WEAR` limit: an expiry in socket 0.
    Timed,
    /// `sAddonType`: `ApplyAddon`'s attributes.
    Addon,
    /// `bAlterToMagicItemPct` of 100: `AlterToMagicItem`'s attributes.
    Magic,
    /// An auto potion or a skill book, whose socket `CreateItem` fills by vnum.
    Vnum(u32),
}

/// Why a buy or a sale changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShopRefused {
    /// The slot's price is 0, or the buyer holds less gold than it.
    NotEnoughMoney,
    /// `CreateItem` answered nothing: the vnum has no proto.
    SoldOut,
    /// The item would take a path of `CreateItem` the Rewrite does not port.
    NotPorted(UnportedCreation),
    /// No free cell of the unlocked base inventory is tall enough.
    InventoryFull,
    /// The item ids are used up.
    IdsExhausted,
    /// The cell to sell from is not a position, or holds nothing.
    Empty,
    /// The item to sell has no proto.
    UnknownVnum(u32),
    /// The item to sell is worn.
    Worn,
    /// The item's anti-flags forbid selling it.
    Unsellable,
    /// The sale would take the seller's gold to [`GOLD_MAX_MAX`].
    GoldOverflow {
        /// The seller's gold.
        gold: u64,
        /// The sale's price.
        price: u64,
    },
    /// The storage refused a change it was asked for.
    Storage(Rejected),
    /// The storage refused a count it was asked for.
    Count(CountRefused),
}

impl ShopRefused {
    /// The `GC_SHOP` subheader a refused buy answers with, if it answers with one.
    ///
    /// `CShopManager::Buy` sends `CShop::Buy`'s answer unless it is `SHOP_SUBHEADER_GC_OK`
    /// (`G/shop_manager.cpp:444-453`). A refusal `CreateItem` cannot give is sold out, as a
    /// `NULL` from it is.
    #[must_use]
    pub const fn subheader(&self) -> Option<u8> {
        match self {
            Self::NotEnoughMoney => Some(SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY),
            Self::SoldOut | Self::NotPorted(_) | Self::IdsExhausted => {
                Some(SHOP_SUBHEADER_GC_SOLD_OUT)
            }
            Self::InventoryFull => Some(SHOP_SUBHEADER_GC_INVENTORY_FULL),
            _ => None,
        }
    }

    /// The chat line a refused sale sends, if it sends one.
    #[must_use]
    pub const fn notice(&self) -> Option<&'static str> {
        match self {
            Self::Worn => Some(WORN_NOTICE),
            _ => None,
        }
    }
}

impl core::fmt::Display for ShopRefused {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotEnoughMoney => write!(f, "not enough gold"),
            Self::SoldOut => write!(f, "the item has no proto"),
            Self::NotPorted(path) => write!(f, "making the item takes an unported path: {path:?}"),
            Self::InventoryFull => write!(f, "no free cell"),
            Self::IdsExhausted => write!(f, "the item ids are used up"),
            Self::Empty => write!(f, "no item in that cell"),
            Self::UnknownVnum(vnum) => write!(f, "item vnum {vnum} has no proto"),
            Self::Worn => write!(f, "the item is worn"),
            Self::Unsellable => write!(f, "the item may not be sold"),
            Self::GoldOverflow { gold, price } => {
                write!(f, "{gold} gold and a price of {price} reach the gold cap")
            }
            Self::Storage(rejected) => write!(f, "the storage refused: {rejected:?}"),
            Self::Count(refused) => write!(f, "the storage refused the count: {refused:?}"),
        }
    }
}

impl std::error::Error for ShopRefused {}

/// The first path of `CreateItem` making `vnum` from `proto` would take that is not ported.
#[must_use]
pub fn unported_creation(vnum: u32, proto: &ItemProto) -> Option<UnportedCreation> {
    let [magic_pct, ..] = proto.alter_to_magic_item_pct.to_le_bytes();
    let [addon_low, addon_high, ..] = proto.addon_type.to_le_bytes();
    let timed = proto
        .limits
        .iter()
        .any(|limit| limit.kind == LIMIT_REAL_TIME || limit.kind == LIMIT_TIMER_BASED_ON_WEAR);
    let path = match proto.item_type {
        ITEM_ELK => UnportedCreation::Elk,
        ITEM_UNIQUE => UnportedCreation::Unique,
        ITEM_DS => UnportedCreation::DragonSoul,
        ITEM_BLEND => UnportedCreation::Blend,
        ITEM_COSTUME if proto.sub_type == COSTUME_AURA => UnportedCreation::Aura,
        _ if timed => UnportedCreation::Timed,
        _ if addon_low != 0 || addon_high != 0 => UnportedCreation::Addon,
        _ if magic_pct == 100 => UnportedCreation::Magic,
        _ if SOCKETED_VNUMS.contains(&vnum) => UnportedCreation::Vnum(vnum),
        _ => return None,
    };
    Some(path)
}

/// `CShop::Buy` for an NPC shop: `offer` goes to the buyer for `offer.price` gold.
///
/// `gold` is what the buyer holds. The answer is the move and the gold left.
///
/// # Errors
///
/// [`ShopRefused::NotEnoughMoney`], [`ShopRefused::SoldOut`], [`ShopRefused::NotPorted`],
/// [`ShopRefused::InventoryFull`] or [`ShopRefused::IdsExhausted`], in the order legacy
/// checks them. Nothing changes on any of them.
pub fn buy_item(
    items: &mut CharacterItems,
    ids: &mut ItemIds,
    gold: u64,
    offer: ShopSlot,
    protos: &ItemProtos,
    rules: &MoveRules,
) -> Result<(MoveDone, u64), ShopRefused> {
    if offer.price == 0 || gold < offer.price {
        return Err(ShopRefused::NotEnoughMoney);
    }
    let proto = protos.get(offer.vnum).ok_or(ShopRefused::SoldOut)?;
    if let Some(path) = unported_creation(offer.vnum, proto) {
        return Err(ShopRefused::NotPorted(path));
    }
    // `bSize` is a `BYTE` assigned from the file's `int`, so it is the low byte.
    let [size, ..] = proto.size.to_le_bytes();
    if size == 0 {
        return Err(ShopRefused::SoldOut);
    }
    let cell = items
        .find_free_inventory_cell(rules.usable_cells, size)
        .ok_or(ShopRefused::InventoryFull)?;
    let count = if proto.flags & ITEM_FLAG_STACKABLE == 0 {
        1
    } else {
        offer.count.max(1).min(rules.count_limit)
    };
    let id = ids.allocate().map_err(|_| ShopRefused::IdsExhausted)?;
    let mut item = Item::new(id, offer.vnum);
    item.set_count(u32::from(count))
        .map_err(|rejected| ShopRefused::Count(CountRefused::Count(rejected)))?;
    item.size = size;
    item.flags = proto.flags;
    item.anti_flags = proto.anti_flags;
    let [socket_pct, ..] = proto.gain_socket_pct.to_le_bytes();
    for socket in item
        .sockets
        .iter_mut()
        .take(usize::from(socket_pct).min(SOCKETS))
    {
        *socket = 1;
    }
    let pos = ItemPos::new(EWindows::Inventory as u8, cell);
    item.pos = pos;
    items.set(pos, &item).map_err(ShopRefused::Storage)?;
    let left = gold - offer.price;
    let done = MoveDone {
        kind: MoveKind::Bought,
        records: vec![
            MoveRecord::Gold {
                amount: 0,
                value: left,
            },
            // `AddToCharacter` highlights an item its new owner never held.
            MoveRecord::Item(ItemRecord::Set(item.gc_item_set(pos, 1))),
        ],
        changes: vec![ItemChange::Created(item)],
    };
    Ok((done, left))
}

/// The gold a sale of `count` of an item made from `proto` pays, after the tax
/// (`G/shop_manager.cpp:532-553`).
#[must_use]
pub fn sale_price(proto: &ItemProto, count: u16) -> u64 {
    let count = u64::from(count);
    let each = u64::from(proto.shop_buy_price);
    let price = if proto.flags & ITEM_FLAG_COUNT_PER_1GOLD == 0 {
        each * count
    } else if each == 0 {
        count
    } else {
        count / each
    } / 5;
    price - price * SALE_TAX_PERCENT / 100
}

/// `CShopManager::Sell`: `count` of the item in inventory cell `cell` goes to the shop.
///
/// `gold` is what the seller holds. The answer is the move and the gold held after it.
///
/// # Errors
///
/// [`ShopRefused::Empty`], [`ShopRefused::Worn`], [`ShopRefused::UnknownVnum`],
/// [`ShopRefused::Unsellable`] or [`ShopRefused::GoldOverflow`]. Nothing changes on any of
/// them.
pub fn sell_item(
    items: &mut CharacterItems,
    gold: u64,
    cell: u16,
    count: u16,
    protos: &ItemProtos,
) -> Result<(MoveDone, u64), ShopRefused> {
    // `GetItem` finds nothing at a cell `IsValidItemPosition` refuses, and neither does
    // `item_at`.
    let at = ItemPos::new(EWindows::Inventory as u8, cell);
    let item = items.item_at(at).cloned().ok_or(ShopRefused::Empty)?;
    if is_equip_position(at) {
        return Err(ShopRefused::Worn);
    }
    let proto = protos
        .get(item.vnum)
        .ok_or(ShopRefused::UnknownVnum(item.vnum))?;
    if proto.anti_flags & ITEM_ANTIFLAG_SELL != 0 {
        return Err(ShopRefused::Unsellable);
    }
    let count = if count == 0 || count > item.count {
        item.count
    } else {
        count
    };
    let price = sale_price(proto, count);
    let held = gold
        .checked_add(price)
        .filter(|held| *held < GOLD_MAX_MAX)
        .ok_or(ShopRefused::GoldOverflow { gold, price })?;
    let mut records = vec![MoveRecord::Notice(SALE_TAX_NOTICE)];
    let change = if count == item.count {
        let pos = items.release(item.id).map_err(ShopRefused::Storage)?;
        // `RemoveItem` syncs the slots before `RemoveFromCharacter` clears the cell.
        records.push(MoveRecord::QuickslotSync(QuickslotSync {
            from: cell,
            to: SyncTo::Delete,
        }));
        records.push(MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos))));
        ItemChange::Destroyed { id: item.id }
    } else {
        let left = item.count - count;
        items
            .set_count(item.id, u32::from(left))
            .map_err(ShopRefused::Count)?;
        let kept = items.item(item.id).ok_or(ShopRefused::Empty)?;
        records.push(MoveRecord::Item(ItemRecord::Update(kept.gc_item_update())));
        ItemChange::Count {
            id: item.id,
            count: left,
        }
    };
    records.push(MoveRecord::Gold {
        amount: i64::try_from(price).unwrap_or(i64::MAX),
        value: held,
    });
    let done = MoveDone {
        kind: MoveKind::Sold,
        records,
        changes: vec![change],
    };
    Ok((done, held))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::ItemIdRange;
    use common::item_slots::INVENTORY_AND_EQUIP_SLOT_MAX;
    use gamedata::item_kind::{ITEM_WEAPON, LIMIT_LEVEL};
    use gamedata::item_proto::ItemValue;

    const INV: u8 = EWindows::Inventory as u8;
    const SWORD: u32 = 19;
    const ARROW: u32 = 8_000;
    const TALL: u32 = 3_100;
    const BOUND: u32 = 80_003;
    const PER_GOLD: u32 = 27_990;

    const RULES: MoveRules = MoveRules {
        count_limit: 200,
        usable_cells: 90,
        belt_grade: None,
        questing: false,
    };

    fn inv(cell: u16) -> ItemPos {
        ItemPos::new(INV, cell)
    }

    fn protos() -> ItemProtos {
        let mut sword = ItemProto::for_category_rule(SWORD, ITEM_WEAPON, 0);
        sword.size = 2;
        sword.gain_socket_pct = 3;
        sword.shop_buy_price = 1_000;
        sword.flags = 1 << 5;
        sword.anti_flags = 1 << 7;
        let mut arrow = ItemProto::for_category_rule(ARROW, ITEM_WEAPON, 0);
        arrow.flags = ITEM_FLAG_STACKABLE;
        arrow.shop_buy_price = 10;
        let mut tall = ItemProto::for_category_rule(TALL, ITEM_WEAPON, 0);
        tall.size = 3;
        tall.gain_socket_pct = 300;
        let mut bound = ItemProto::for_category_rule(BOUND, 3, 0);
        bound.anti_flags = ITEM_ANTIFLAG_SELL;
        let mut per_gold = ItemProto::for_category_rule(PER_GOLD, 3, 0);
        per_gold.flags = ITEM_FLAG_STACKABLE | ITEM_FLAG_COUNT_PER_1GOLD;
        per_gold.shop_buy_price = 4;
        ItemProtos::from_rows(vec![sword, arrow, tall, bound, per_gold])
    }

    fn ids() -> ItemIds {
        ItemIds::new(ItemIdRange::new(1000, 2000, 1000).expect("a valid range"))
    }

    fn offer(vnum: u32, count: u16, price: u64) -> ShopSlot {
        ShopSlot { vnum, count, price }
    }

    fn holding(placed: &[(u16, Item)]) -> CharacterItems {
        let mut items = CharacterItems::new();
        for (cell, item) in placed {
            items.set(inv(*cell), item).expect("the fixture places");
        }
        items
    }

    fn arrows(id: u32, count: u16) -> Item {
        let mut item = Item::new(id, ARROW);
        item.count = count;
        item.flags = ITEM_FLAG_STACKABLE;
        item
    }

    fn buy(
        items: &mut CharacterItems,
        gold: u64,
        offer: ShopSlot,
    ) -> Result<(MoveDone, u64), ShopRefused> {
        buy_item(items, &mut ids(), gold, offer, &protos(), &RULES)
    }

    #[test]
    fn a_bought_item_takes_the_first_free_cell_and_the_gold() {
        let mut items = holding(&[(0, Item::new(7, TALL))]);
        let (done, left) = buy(&mut items, 5_000, offer(SWORD, 1, 1_200)).expect("buys");
        assert_eq!(left, 3_800);
        assert_eq!(done.kind, MoveKind::Bought);
        let bought = items
            .item_at(inv(1))
            .cloned()
            .expect("in the next free column");
        assert_eq!(
            (bought.id, bought.vnum, bought.count, bought.size),
            (1000, SWORD, 1, 2)
        );
        assert_eq!((bought.flags, bought.anti_flags), (1 << 5, 1 << 7));
        assert_eq!(bought.sockets, [1, 1, 1, 0, 0, 0]);
        assert_eq!(
            done.records,
            vec![
                MoveRecord::Gold {
                    amount: 0,
                    value: 3_800
                },
                MoveRecord::Item(ItemRecord::Set(bought.gc_item_set(inv(1), 1))),
            ]
        );
        assert_eq!(done.changes, vec![ItemChange::Created(bought)]);
    }

    #[test]
    fn a_bought_stack_is_new_and_held_to_the_stack_limit() {
        let mut items = holding(&[(0, arrows(7, 5))]);
        let mut ids = ids();
        let protos = protos();
        let mut buy = |items: &mut CharacterItems, gold, offer| {
            buy_item(items, &mut ids, gold, offer, &protos, &RULES)
        };
        let (_, left) = buy(&mut items, 100, offer(ARROW, 500, 100)).expect("buys");
        assert_eq!(left, 0, "all of the gold is enough");
        assert_eq!(items.item_at(inv(0)).map(Item::count), Some(5), "no merge");
        assert_eq!(items.item_at(inv(1)).map(Item::count), Some(200));
        let (_, _) = buy(&mut items, 1, offer(ARROW, 0, 1)).expect("buys");
        assert_eq!(
            items.item_at(inv(2)).map(Item::count),
            Some(1),
            "at least one"
        );
        let (_, _) = buy(&mut items, 1, offer(SWORD, 9, 1)).expect("buys");
        assert_eq!(items.item_at(inv(3)).map(Item::count), Some(1), "no stack");
    }

    #[test]
    fn a_socket_count_is_a_byte_and_at_most_six() {
        let mut items = CharacterItems::new();
        let (done, _) = buy(&mut items, 1, offer(TALL, 1, 1)).expect("buys");
        let [ItemChange::Created(item)] = done.changes.as_slice() else {
            panic!("one item made");
        };
        assert_eq!(item.sockets, [1; SOCKETS], "300 is 44 as a byte, then six");
    }

    #[test]
    fn a_price_of_nothing_or_more_than_the_gold_refuses() {
        let mut items = CharacterItems::new();
        for (gold, price) in [(100, 0), (99, 100), (0, 1)] {
            assert_eq!(
                buy(&mut items, gold, offer(SWORD, 1, price)),
                Err(ShopRefused::NotEnoughMoney)
            );
        }
        assert!(
            buy(&mut items, 100, offer(SWORD, 1, 100)).is_ok(),
            "exactly enough"
        );
    }

    #[test]
    fn a_refused_buy_changes_nothing_and_says_why() {
        let full: Vec<(u16, Item)> = (0..90)
            .map(|cell| (cell, arrows(u32::from(cell) + 1, 1)))
            .collect();
        let mut items = holding(&full);
        assert_eq!(
            buy(&mut items, 100, offer(SWORD, 1, 10)),
            Err(ShopRefused::InventoryFull)
        );
        assert_eq!(
            buy(&mut items, 100, offer(4_242, 1, 10)),
            Err(ShopRefused::SoldOut)
        );
        let mut exhausted = ItemIds::new(ItemIdRange::new(1000, 1001, 1000).expect("one id"));
        let _ = exhausted.allocate().expect("the one id");
        let mut empty = CharacterItems::new();
        assert_eq!(
            buy_item(
                &mut empty,
                &mut exhausted,
                100,
                offer(SWORD, 1, 10),
                &protos(),
                &RULES
            ),
            Err(ShopRefused::IdsExhausted)
        );
        assert!(empty.item_at(inv(0)).is_none());
        for (refused, answer) in [
            (ShopRefused::NotEnoughMoney, Some(5)),
            (ShopRefused::SoldOut, Some(9)),
            (ShopRefused::NotPorted(UnportedCreation::Elk), Some(9)),
            (ShopRefused::IdsExhausted, Some(9)),
            (ShopRefused::InventoryFull, Some(7)),
            (ShopRefused::Worn, None),
            (ShopRefused::Empty, None),
        ] {
            assert_eq!(refused.subheader(), answer, "{refused}");
        }
        assert_eq!(ShopRefused::Worn.notice(), Some("[LS;1059]"));
        assert_eq!(ShopRefused::Unsellable.notice(), None);
    }

    #[test]
    fn each_unported_creation_path_is_named() {
        let with = |item_type, sub_type, change: fn(&mut ItemProto)| {
            let mut proto = ItemProto::for_category_rule(1, item_type, sub_type);
            change(&mut proto);
            unported_creation(1, &proto)
        };
        let none = |_: &mut ItemProto| {};
        assert_eq!(with(ITEM_WEAPON, 0, none), None);
        assert_eq!(with(ITEM_ELK, 0, none), Some(UnportedCreation::Elk));
        assert_eq!(with(ITEM_UNIQUE, 0, none), Some(UnportedCreation::Unique));
        assert_eq!(with(ITEM_DS, 0, none), Some(UnportedCreation::DragonSoul));
        assert_eq!(with(ITEM_BLEND, 0, none), Some(UnportedCreation::Blend));
        assert_eq!(
            with(ITEM_COSTUME, COSTUME_AURA, none),
            Some(UnportedCreation::Aura)
        );
        assert_eq!(
            with(ITEM_COSTUME, 1, none),
            None,
            "a hair costume is ported"
        );
        let timed = |kind| {
            move |proto: &mut ItemProto| {
                proto.limits[1] = ItemValue { kind, value: 0 };
            }
        };
        let mut proto = ItemProto::for_category_rule(1, ITEM_WEAPON, 0);
        for (kind, path) in [
            (LIMIT_REAL_TIME, Some(UnportedCreation::Timed)),
            (LIMIT_TIMER_BASED_ON_WEAR, Some(UnportedCreation::Timed)),
            (LIMIT_LEVEL, None),
        ] {
            timed(kind)(&mut proto);
            assert_eq!(unported_creation(1, &proto), path, "{kind}");
        }
        assert_eq!(
            with(ITEM_WEAPON, 0, |p| p.addon_type = -1),
            Some(UnportedCreation::Addon)
        );
        assert_eq!(
            with(ITEM_WEAPON, 0, |p| p.addon_type = 0x0100),
            Some(UnportedCreation::Addon),
            "the high byte alone"
        );
        assert_eq!(
            with(ITEM_WEAPON, 0, |p| p.addon_type = 0x1_0000),
            None,
            "a short"
        );
        assert_eq!(
            with(ITEM_WEAPON, 0, |p| p.alter_to_magic_item_pct = 100),
            Some(UnportedCreation::Magic)
        );
        assert_eq!(
            with(ITEM_WEAPON, 0, |p| p.alter_to_magic_item_pct = 99),
            None
        );
        let plain = ItemProto::for_category_rule(1, ITEM_WEAPON, 0);
        // Listed here rather than read from `SOCKETED_VNUMS`, so that a vnum dropped from the
        // table is caught.
        let socketed = [
            72_723, 72_724, 72_725, 72_726, 72_727, 72_728, 72_729, 72_730, 76_004, 76_005, 76_021,
            76_022, 50_300, 70_037, 70_055,
        ];
        assert_eq!(socketed.len(), SOCKETED_VNUMS.len());
        for vnum in socketed {
            assert_eq!(
                unported_creation(vnum, &plain),
                Some(UnportedCreation::Vnum(vnum))
            );
        }
        assert_eq!(unported_creation(72_722, &plain), None);
        assert_eq!(unported_creation(72_731, &plain), None);
    }

    #[test]
    fn an_unported_item_is_not_bought() {
        let mut elk = ItemProto::for_category_rule(50, ITEM_ELK, 0);
        elk.size = 1;
        let protos = ItemProtos::from_rows(vec![elk]);
        let mut items = CharacterItems::new();
        assert_eq!(
            buy_item(&mut items, &mut ids(), 10, offer(50, 1, 1), &protos, &RULES),
            Err(ShopRefused::NotPorted(UnportedCreation::Elk))
        );
        assert!(items.item_at(inv(0)).is_none());
    }

    #[test]
    fn a_sale_is_a_fifth_of_the_price_less_the_tax() {
        let protos = protos();
        let sword = protos.get(SWORD).expect("a sword");
        assert_eq!(sale_price(sword, 1), 194, "1000 / 5 = 200, less 6");
        assert_eq!(sale_price(sword, 3), 582, "3000 / 5 = 600, less 18");
        let per_gold = protos.get(PER_GOLD).expect("per gold");
        assert_eq!(
            sale_price(per_gold, 200),
            10,
            "50 for 200, 10 for a fifth, less 0"
        );
        let mut free = per_gold.clone();
        free.shop_buy_price = 0;
        assert_eq!(
            sale_price(&free, 100),
            20,
            "a count per gold of 0 pays the count"
        );
        let arrow = protos.get(ARROW).expect("an arrow");
        assert_eq!(sale_price(arrow, 0), 0);
        assert_eq!(sale_price(arrow, 4), 8, "40 / 5 = 8, less 0");
    }

    #[test]
    fn a_whole_sale_empties_the_cell_and_pays() {
        let mut sword = Item::new(7, SWORD);
        sword.size = 2;
        let mut items = holding(&[(4, sword)]);
        let (done, gold) = sell_item(&mut items, 6, 4, 0, &protos()).expect("sells");
        assert_eq!(gold, 200);
        assert_eq!(done.kind, MoveKind::Sold);
        assert_eq!(
            done.records,
            vec![
                MoveRecord::Notice(SALE_TAX_NOTICE),
                MoveRecord::QuickslotSync(QuickslotSync {
                    from: 4,
                    to: SyncTo::Delete
                }),
                MoveRecord::Item(ItemRecord::Set(gc_item_clear(inv(4)))),
                MoveRecord::Gold {
                    amount: 194,
                    value: 200
                },
            ]
        );
        assert_eq!(done.changes, vec![ItemChange::Destroyed { id: 7 }]);
        assert!(items.item_at(inv(4)).is_none());
        assert!(items.item_at(inv(9)).is_none(), "its whole footprint");
    }

    #[test]
    fn a_part_sale_keeps_the_rest_and_a_large_count_sells_all() {
        let mut items = holding(&[(2, arrows(7, 50))]);
        let (done, gold) = sell_item(&mut items, 0, 2, 20, &protos()).expect("sells");
        assert_eq!(gold, 39, "200 / 5 = 40, less 1");
        let kept = items.item(7).cloned().expect("kept");
        assert_eq!(kept.count, 30);
        assert_eq!(
            done.records,
            vec![
                MoveRecord::Notice(SALE_TAX_NOTICE),
                MoveRecord::Item(ItemRecord::Update(kept.gc_item_update())),
                MoveRecord::Gold {
                    amount: 39,
                    value: 39
                },
            ]
        );
        assert_eq!(done.changes, vec![ItemChange::Count { id: 7, count: 30 }]);
        let (done, gold) = sell_item(&mut items, 39, 2, 31, &protos()).expect("sells");
        assert_eq!(gold, 39 + 59, "the other 30: 300 / 5 = 60, less 1");
        assert_eq!(done.changes, vec![ItemChange::Destroyed { id: 7 }]);
    }

    #[test]
    fn a_sale_that_cannot_be_made_changes_nothing() {
        let mut worn = Item::new(8, SWORD);
        worn.size = 1;
        let mut bound = Item::new(9, BOUND);
        bound.size = 1;
        let mut items = holding(&[(0, arrows(7, 5)), (1, bound), (5, Item::new(10, 4_242))]);
        items.set(inv(180), &worn).expect("worn");
        let protos = protos();
        for cell in [3, INVENTORY_AND_EQUIP_SLOT_MAX, u16::MAX] {
            assert_eq!(
                sell_item(&mut items, 0, cell, 0, &protos),
                Err(ShopRefused::Empty),
                "cell {cell}"
            );
        }
        assert_eq!(
            sell_item(&mut items, 0, 180, 0, &protos),
            Err(ShopRefused::Worn)
        );
        assert_eq!(
            sell_item(&mut items, 0, 1, 0, &protos),
            Err(ShopRefused::Unsellable)
        );
        assert_eq!(
            sell_item(&mut items, 0, 5, 0, &protos),
            Err(ShopRefused::UnknownVnum(4_242))
        );
        for gold in [GOLD_MAX_MAX - 10, GOLD_MAX_MAX - 9, u64::MAX] {
            assert_eq!(
                sell_item(&mut items, gold, 0, 0, &protos),
                Err(ShopRefused::GoldOverflow { gold, price: 10 }),
                "five arrows pay 10, and the cap itself is refused"
            );
        }
        assert_eq!(items.item_at(inv(0)).map(Item::count), Some(5));
        let (_, gold) = sell_item(&mut items, GOLD_MAX_MAX - 11, 0, 0, &protos).expect("sells");
        assert_eq!(gold, GOLD_MAX_MAX - 1, "one below the cap is allowed");
    }
}
