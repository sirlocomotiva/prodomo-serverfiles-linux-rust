//! `CG_ITEM_MOVE` for the base inventory, the custom banks and the belt.
//!
//! This is `CHARACTER::MoveItem` (`char_item.cpp:7602-7850`) as a reducer: it takes the
//! character's storage and a request, changes the storage, and answers with the records the
//! client is sent and the row changes the store has to make. It holds no socket and no
//! database handle, so the caller decides when each is written.
//!
//! # What is ported
//!
//! The three outcomes a move has once it reaches the grid: a **move** (the whole stack goes),
//! a **split** (part of a stack becomes a new item), and a **merge** (part or all of a stack
//! joins a stack of the same item). Before those, every check legacy runs in its order, with
//! the chat notices legacy sends for four of them.
//!
//! # What is refused as not ported
//!
//! A dragon-soul item or equipment cell, a switchbot slot, a source window other than the
//! inventory and the equipment, and, when the caller passes no [`Gear`], a move out of or into
//! a wear cell. Each is [`MoveRefused::NotPorted`], so a caller can tell "legacy would refuse
//! this" from "this build cannot do it yet".
//!
//! # Wear cells
//!
//! With the character's [`Gear`], a move out of or into a wear cell runs legacy's
//! `EquipItem`, `UnequipItem` and `SwapItem` paths ([`super::equip`]). Those change the points
//! and the look too, so a [`MoveDone`] carries [`MoveRecord`]s, not only item records. A path
//! that changes something and then fails ends [`MoveKind::Declined`], with what it did.
//!
//! Five legacy checks have nothing to test yet and are left out: `IsExchanging`,
//! `isLocked`, `CanHandleItem`, `IsSecured`, and the observer mode. The quickslot sync and
//! the `ITEM_SPLIT` log are the caller's.
//!
//! # Four Defects this does not reproduce
//!
//! 1. **The auto-find notice is never sent.** `wFindCell` is a `WORD`, so
//!    `wFindCell != -1` compares 65535 with -1 and is always true
//!    (`char_item.cpp:7644-7645`, `:7658-7659`). A full inventory writes 65535 into the
//!    destination, which then fails `IsValidItemPosition` silently. Here the search's answer
//!    is read, and the notice legacy wrote for it is sent.
//! 2. **An item in two banks can be refused from its own bank.** `GetEmptyInventory` places
//!    an item in the first bank with room (`char_item.cpp:1214-1225`), but the move check
//!    compares the destination with `GetItemCategory`, which is the first bank that matches
//!    (`item.cpp:3149-3158`). Five of the owner's items are in two banks, and one of them
//!    placed in its second bank can then not be moved inside it. Here a destination bank is
//!    accepted when the item belongs to it.
//! 3. **A split can land on its own stack.** The grid check passes the source cell as the
//!    exception (`:7807`), which is right for a move and wrong for a split, because the
//!    source stays where it is. Here a split is checked without the exception.
//! 4. **`ITEM_FLAG_IRREMOVABLE` is tested only for window 1** (`:7625`), so the same cell
//!    addressed through window 2 moves an irremovable item out. Here both windows are
//!    tested.
//!
//! The auto-find's base search is bounded by the usable cells rather than by
//! `INVENTORY_MAX_NUM`, the same Divergence [`CharacterItems::find_free_inventory_cell`]
//! records.

use common::item_slots::{
    EWindows, CUSTOM_INVENTORY_SLOT_END, CUSTOM_INVENTORY_SLOT_START, INVENTORY_MAX_NUM,
    ITEM_COUNT_LIMIT,
};
use protocol::gc_item_window::{GcItemSet, GcItemUpdate};
use protocol::item_pos::ItemPos;

use super::equip::{find_equip_cell, CharacterLook, Equipper, Gear, Trail, WornSystem};
use super::inventory::{
    custom_inventory_category_of, is_belt_inventory_position, is_custom_inventory_position,
    is_default_inventory_position, is_dragon_soul_equip_position, is_equip_position,
    is_switchbot_position, is_valid_item_position,
};
use super::items::{CharacterItems, CountRefused, Rejected};
use super::points::PointRecord;
use crate::item::{gc_item_clear, Item, ItemId, ItemIds, ITEM_FLAG_IRREMOVABLE};
use common::enums::EWearPositions;

/// The destination cell a client sends to ask the server to choose one.
///
/// `DestCell.cell == USHRT_MAX` (`char_item.cpp:7642`, `:7656`).
pub const AUTO_FIND_CELL: u16 = u16::MAX;

/// One `CG_ITEM_MOVE` as the client sent it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveRequest {
    /// `Cell`: where the item is.
    pub from: ItemPos,
    /// `DestCell`: where it should go, or [`AUTO_FIND_CELL`].
    pub to: ItemPos,
    /// How many to move. Zero means the whole stack.
    pub count: u16,
}

/// The facts about the character a move reads and the storage does not hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveRules {
    /// `g_bItemCountLimit`: the largest stack a merge may build.
    pub count_limit: u16,
    /// How many base-inventory cells this character has unlocked.
    pub usable_cells: u16,
    /// The worn belt's `value0`, or `None` when no belt is worn.
    pub belt_grade: Option<i32>,
}

/// The facts about an item's prototype a move reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveFacts {
    /// The custom banks the item belongs to, ascending.
    pub categories: Vec<u8>,
    /// Whether a belt cell accepts it.
    pub belt_eligible: bool,
    /// Whether it is a dragon-soul item (`ITEM_DS`).
    pub dragon_soul: bool,
}

/// Which of the three outcomes a move had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveKind {
    /// The whole stack went to the destination.
    Moved,
    /// Some or all of the stack joined the stack at the destination.
    Merged,
    /// Part of the stack became a new item at the destination.
    Split,
    /// The item went on in an empty wear cell.
    Equipped,
    /// The item went on in place of the worn one, which went to the item's cell.
    Swapped,
    /// The worn item went to the first free inventory cell (`UnequipItem`).
    Unequipped,
    /// A wear-cell path changed something and then refused; the records end with its notice.
    Declined,
    /// The item, or part of its stack, went to the ground (`DropItem`).
    Dropped,
    /// A ground item went into a free cell (`PickupItem`), after any merges.
    PickedUp,
}

/// A record the client is sent, in the order legacy sends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemRecord {
    /// `GC_ITEM_SET`: a cell now holds this, or is empty when the vnum is 0.
    Set(GcItemSet),
    /// `GC_ITEM_UPDATE`: the item in a cell changed in place.
    Update(GcItemUpdate),
}

impl ItemRecord {
    /// Append the record's wire bytes to `out`.
    pub fn encode_into(self, out: &mut Vec<u8>) {
        match self {
            Self::Set(record) => record.encode_into(out),
            Self::Update(record) => record.encode_into(out),
        }
    }
}

/// A record the client is sent by a move, in the order legacy sends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveRecord {
    /// An item record.
    Item(ItemRecord),
    /// A `GC_CHARACTER_POINT_CHANGE`.
    Point(PointRecord),
    /// A `GC_CHARACTER_UPDATE` (`UpdatePacket`), which the viewers are sent too.
    Look(CharacterLook),
    /// A `GC_SPECIAL_EFFECT` on the character, which the viewers are sent too.
    Effect(u8),
    /// A `CHAT_TYPE_INFO` line.
    Notice(&'static str),
    /// A ground item appeared or went, which the viewers are sent too.
    Ground(GroundRecord),
    /// `[LS;444;%s]`: the picked-up item's name, which the caller looks up by vnum.
    PickedUp {
        /// The item's vnum.
        vnum: u32,
    },
}

/// A record about a ground item (`CItem::EncodeInsertPacket`, `EncodeRemovePacket`,
/// `G/item.cpp:163-218`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroundRecord {
    /// `GC_ITEM_GROUND_ADD`: the item lies at `x`, `y`.
    Add {
        /// The ground item's VID.
        vid: u32,
        /// Its vnum.
        vnum: u32,
        /// Map x.
        x: i32,
        /// Map y.
        y: i32,
    },
    /// `GC_ITEM_GROUND_DEL`: the item is gone from the ground.
    Del {
        /// The ground item's VID.
        vid: u32,
    },
}

/// A change the store has to make for the move to survive a restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemChange {
    /// The item is now at this position, in stored terms.
    Moved {
        /// The item.
        id: ItemId,
        /// Where it is stored now.
        pos: ItemPos,
    },
    /// The item's stack size changed.
    Count {
        /// The item.
        id: ItemId,
        /// Its new count.
        count: u16,
    },
    /// A split made this item, stored where its `pos` says.
    Created(Item),
    /// A merge used up this item.
    Destroyed {
        /// The item.
        id: ItemId,
    },
    /// The item's sockets changed: a sash rolled its absorption.
    Sockets {
        /// The item.
        id: ItemId,
        /// Its sockets now.
        sockets: [i32; crate::item::SOCKETS],
    },
}

/// What a move did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveDone {
    /// Which outcome it was.
    pub kind: MoveKind,
    /// What the client is sent, in order.
    pub records: Vec<MoveRecord>,
    /// What the store has to change, in order.
    pub changes: Vec<ItemChange>,
}

/// A path this build does not port yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unported {
    /// A source window other than the inventory and the equipment.
    SourceWindow(u8),
    /// A destination window other than the inventory and the equipment.
    DestinationWindow(u8),
    /// Equipping or taking off an item.
    Equipment,
    /// A dragon-soul item.
    DragonSoul,
    /// A switchbot slot.
    Switchbot,
    /// Wearing this item starts a system this build does not have.
    Worn(WornSystem),
    /// `CG_ITEM_USE` on an item of this type, whose use arm is not ported.
    Use(i32),
}

/// Why a move changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveRefused {
    /// `IsValidItemPosition(Cell)` is false.
    InvalidSource,
    /// No item is anchored at the source.
    Empty,
    /// More was asked for than the stack holds.
    CountAboveStack {
        /// What the stack holds.
        held: u16,
        /// What was asked for.
        asked: u16,
    },
    /// An irremovable item outside the base inventory and the custom banks.
    Irremovable,
    /// The item's vnum has no prototype.
    UnknownVnum(u32),
    /// The server was asked to choose a bank cell and none of the item's banks has room.
    NoRoomInSpecialInventory,
    /// The server was asked to choose a base cell and none has room.
    NoRoomInInventory,
    /// `IsValidItemPosition(DestCell)` is false.
    InvalidDestination,
    /// The destination is a custom bank the item does not belong to.
    WrongBank,
    /// The destination is a belt cell and the item may not go there.
    NotForBelt,
    /// A non-dragon-soul item was sent to the dragon-soul window.
    NotDragonSoulItem,
    /// The two stacks would merge but their sockets differ.
    SocketsDiffer,
    /// `IsEmptyItemGrid` is false for the destination.
    NoRoom,
    /// A split needs a new id and no allocator is installed.
    NoItemIds,
    /// A split needs a new id and the range is spent.
    IdsExhausted,
    /// The storage refused a step the checks above had allowed.
    Storage(Rejected),
    /// The storage refused a count the checks above had allowed.
    Count(CountRefused),
    /// A path this build does not port yet.
    NotPorted(Unported),
    /// `CanEquipNow`: the item's anti-flags refuse the character's job.
    JobAntiFlag,
    /// `CanEquipNow`: the level limit.
    LevelTooLow,
    /// `CanEquipNow`: the conqueror level limit.
    ChampionTooLow,
    /// `CanEquipNow`: the strength limit.
    StrTooLow,
    /// `CanEquipNow`: the intelligence limit.
    IntTooLow,
    /// `CanEquipNow`: the dexterity limit.
    DexTooLow,
    /// `CanEquipNow`: the vitality limit.
    ConTooLow,
    /// `CanEquipNow`: the same ring is already worn.
    RingTwice,
    /// `CanUnequipNow`: the belt's inventory holds an item.
    BeltNotEmpty,
    /// `CanUnequipNow`: no inventory cell has room for the worn item.
    NoRoomToUnequip,
    /// `IsEquipable` is false.
    NotEquipable,
    /// `FindEquipCell` found no cell.
    NoEquipCell,
    /// A costume body onto a wedding armour.
    WeddingCostume,
    /// A wedding armour under a costume body.
    WeddingArmour,
    /// The item's anti-flags refuse the character's sex.
    WrongSex,
    /// The character attacked in the last 1.5 s.
    RecentlyFought,
    /// The worn costume weapon could not be taken off (`UnequipItem`, `EquipItem`).
    CostumeWeaponStuck,
    /// The worn costume weapon could not be taken off (`MoveItem`).
    CostumeWeaponStuckOnMove,
    /// A costume weapon onto a missing or mismatched weapon.
    WrongWeaponForCostume,
    /// A talent item onto a taken talent cell.
    AbilityOccupied,
    /// `SwapItem` refused.
    SwapRefused,
    /// The wear cell the client named is taken.
    WearCellTaken,
    /// `CItem::CanUsedBy`: the item's anti-flags refuse the character's job.
    NotUsableByJob,
    /// `UseItemEx`: the level limit.
    UseLevelTooLow,
    /// `UseItemEx`: an item in the belt's inventory with no belt worn.
    NoBeltWorn,
    /// `UseItemEx`: an item in a belt cell the worn belt's grade does not open.
    BeltCellLocked,
    /// `DropItem`: the item's anti-flags forbid dropping or giving it.
    Undroppable,
    /// `PickupItem`: no cell has room and no stack took all of it.
    NoRoomToPickUp,
    /// `PickupItem`: the named VID is not an item lying on the picker's map.
    NotOnGround,
    /// `PickupItem`: `DistanceValid` refused, the item lies farther than 300 away.
    TooFar,
}

impl MoveRefused {
    /// The chat line legacy sends the player for this refusal, if it sends one.
    ///
    /// Each is sent verbatim as a `CHAT_TYPE_INFO` line, the text `LC_TEXT` is given at the
    /// cited line.
    #[must_use]
    pub const fn notice(&self) -> Option<&'static str> {
        match self {
            // `char_item.cpp:7651`.
            Self::NoRoomInSpecialInventory => Some("Nu ai spatiu suficient in inventarul special."),
            // `char_item.cpp:7665`.
            Self::NoRoomInInventory => Some("Nu ai spatiu suficient in inventar."),
            // `char_item.cpp:7691`.
            Self::WrongBank => {
                Some("@@(char_item.cpp)tradus:[Special_Inventory]Nu poti plasa acest obiect aici.")
            }
            // `char_item.cpp:7698`.
            Self::NotForBelt => Some("[LS;1097]"),
            // `char_item.cpp:10097-10139`.
            Self::LevelTooLow => Some("[LS;462]"),
            Self::ChampionTooLow => {
                Some("Nivelul tau campion este prea mic pentru a putea purta acest item!")
            }
            Self::StrTooLow => Some("[LS;463]"),
            Self::IntTooLow => Some("[LS;464]"),
            Self::DexTooLow => Some("[LS;465]"),
            Self::ConTooLow => Some("[LS;466]"),
            // `char_item.cpp:10175`.
            Self::RingTwice => Some("You cannot equip this item twice"),
            // `char_item.cpp:10199`.
            Self::BeltNotEmpty => Some(
                "[1095]You can only discard the belt when there are no longer any items in its \
                 inventory.",
            ),
            // `char_item.cpp:10210`.
            Self::NoRoomToUnequip => Some("[1130]There isn't enough space in your inventory."),
            // `char_item.cpp:8371`.
            Self::WeddingCostume => {
                Some("Non puoi usare un Costume con uno Smoking o Abito da Sposa.")
            }
            // `char_item.cpp:8381`.
            Self::WeddingArmour => Some("Devi rimuovere il Costume per usarlo."),
            // `char_item.cpp:8390`.
            Self::WrongSex => Some("[LS;1005]"),
            // `char_item.cpp:8421`.
            Self::RecentlyFought => Some("[LS;451]"),
            // `char_item.cpp:8275`, `:8465`, `:8474`.
            Self::CostumeWeaponStuck => {
                Some("You cannot unequip the costume weapon because there is not enough space")
            }
            // `char_item.cpp:7740`.
            Self::CostumeWeaponStuckOnMove => Some(
                "@@(char_item.cpp)tradus:You cannot unequip the costume weapon because there is \
                 not enough space",
            ),
            // `char_item.cpp:8486`.
            Self::WrongWeaponForCostume => Some(
                "You cannot equip the costume weapon, because you have the wrong weapon equipped",
            ),
            // `char_item.cpp:7754`.
            Self::WearCellTaken => Some("[LS;1092]"),
            // `char_item.cpp:7192`.
            Self::NotUsableByJob => Some("[LS;1004]"),
            // `char_item.cpp:2734`.
            Self::UseLevelTooLow => Some("[LS;1013]"),
            // `char_item.cpp:2791`.
            Self::NoBeltWorn => Some("<Belt> You can't use this item if you have no equipped belt"),
            // `char_item.cpp:2797`.
            Self::BeltCellLocked => {
                Some("<Belt> You can't use this item if you don't upgrade your belt")
            }
            // `char_item.cpp:7483`.
            Self::Undroppable => Some("[LS;442]"),
            // `char_item.cpp:8066`.
            Self::NoRoomToPickUp => Some("[LS;445]"),
            _ => None,
        }
    }
}

impl core::fmt::Display for MoveRefused {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidSource => f.write_str("the source is not an item position"),
            Self::Empty => f.write_str("no item is at the source"),
            Self::CountAboveStack { held, asked } => {
                write!(f, "{asked} were asked for and the stack holds {held}")
            }
            Self::Irremovable => f.write_str("the item may not leave this cell"),
            Self::UnknownVnum(vnum) => write!(f, "vnum {vnum} has no prototype"),
            Self::NoRoomInSpecialInventory => f.write_str("no bank of this item has room"),
            Self::NoRoomInInventory => f.write_str("the inventory has no room"),
            Self::InvalidDestination => f.write_str("the destination is not an item position"),
            Self::WrongBank => f.write_str("the item does not belong in that bank"),
            Self::NotForBelt => f.write_str("the item may not go in the belt"),
            Self::NotDragonSoulItem => f.write_str("only a dragon-soul item goes there"),
            Self::SocketsDiffer => f.write_str("the two stacks have different sockets"),
            Self::NoRoom => f.write_str("the destination has no room"),
            Self::NoItemIds => f.write_str("no item id allocator is installed"),
            Self::IdsExhausted => f.write_str("the item id range is spent"),
            Self::Storage(reason) => write!(f, "the storage refused: {reason}"),
            Self::Count(reason) => write!(f, "the storage refused a count: {reason}"),
            Self::NotPorted(what) => write!(f, "not ported yet: {what:?}"),
            Self::JobAntiFlag => f.write_str("the item refuses this job"),
            Self::LevelTooLow => f.write_str("the level is below the item's limit"),
            Self::ChampionTooLow => f.write_str("the conqueror level is below the item's limit"),
            Self::StrTooLow => f.write_str("the strength is below the item's limit"),
            Self::IntTooLow => f.write_str("the intelligence is below the item's limit"),
            Self::DexTooLow => f.write_str("the dexterity is below the item's limit"),
            Self::ConTooLow => f.write_str("the vitality is below the item's limit"),
            Self::RingTwice => f.write_str("the same ring is already worn"),
            Self::BeltNotEmpty => f.write_str("the belt's inventory is not empty"),
            Self::NoRoomToUnequip => f.write_str("no inventory cell has room for the worn item"),
            Self::NotEquipable => f.write_str("the item is not equipable"),
            Self::NoEquipCell => f.write_str("the item has no wear cell"),
            Self::WeddingCostume => f.write_str("a costume body may not cover a wedding armour"),
            Self::WeddingArmour => f.write_str("a wedding armour may not go under a costume"),
            Self::WrongSex => f.write_str("the item refuses this sex"),
            Self::RecentlyFought => f.write_str("the character fought in the last 1.5 s"),
            Self::CostumeWeaponStuck | Self::CostumeWeaponStuckOnMove => {
                f.write_str("the costume weapon could not be taken off")
            }
            Self::WrongWeaponForCostume => f.write_str("the worn weapon does not fit the costume"),
            Self::AbilityOccupied => f.write_str("the talent cell is taken"),
            Self::SwapRefused => f.write_str("the swap was refused"),
            Self::WearCellTaken => f.write_str("the wear cell is taken"),
            Self::NotUsableByJob => f.write_str("the item refuses this job's use"),
            Self::UseLevelTooLow => f.write_str("the level is below the item's use limit"),
            Self::NoBeltWorn => f.write_str("a belt item is used with no belt worn"),
            Self::BeltCellLocked => f.write_str("the belt does not open this cell"),
            Self::Undroppable => f.write_str("the item may not be dropped"),
            Self::NoRoomToPickUp => f.write_str("no cell has room for the ground item"),
            Self::NotOnGround => f.write_str("no such item lies on this map"),
            Self::TooFar => f.write_str("the ground item is too far away"),
        }
    }
}

impl std::error::Error for MoveRefused {}

pub(crate) fn is_flat_window(window_type: u8) -> bool {
    matches!(
        EWindows::try_from(window_type),
        Ok(EWindows::Inventory | EWindows::Equipment)
    )
}

/// Run one `CG_ITEM_MOVE` against a character's storage.
///
/// `ids` is needed only by a split, and `facts` is asked once, for the moved item's vnum.
/// `gear` lets a move reach a wear cell; without it, one is [`Unported::Equipment`].
///
/// # Errors
///
/// Any [`MoveRefused`]. A refusal changes nothing, so the storage is as it was and the
/// client is sent only the notice [`MoveRefused::notice`] names.
pub fn move_item(
    items: &mut CharacterItems,
    ids: Option<&mut ItemIds>,
    request: MoveRequest,
    rules: &MoveRules,
    gear: Option<&mut Gear<'_>>,
    facts: impl FnOnce(u32) -> Option<MoveFacts>,
) -> Result<MoveDone, MoveRefused> {
    let MoveRequest {
        from,
        mut to,
        count,
    } = request;
    if !is_valid_item_position(from) {
        return Err(MoveRefused::InvalidSource);
    }
    if !is_flat_window(from.window_type) {
        return Err(MoveRefused::NotPorted(Unported::SourceWindow(
            from.window_type,
        )));
    }
    let item = items.item_at(from).cloned().ok_or(MoveRefused::Empty)?;
    if item.count < count {
        return Err(MoveRefused::CountAboveStack {
            held: item.count,
            asked: count,
        });
    }
    let outside_banks = (INVENTORY_MAX_NUM..CUSTOM_INVENTORY_SLOT_START).contains(&from.cell)
        || from.cell >= CUSTOM_INVENTORY_SLOT_END;
    if item.flags & ITEM_FLAG_IRREMOVABLE != 0 && outside_banks {
        return Err(MoveRefused::Irremovable);
    }
    let facts = facts(item.vnum).ok_or(MoveRefused::UnknownVnum(item.vnum))?;
    if from.window_type == EWindows::Inventory as u8
        && to.window_type == EWindows::Inventory as u8
        && to.cell == AUTO_FIND_CELL
    {
        to.cell = auto_find(items, &item, from, &facts, rules)?;
    }
    check_destination(to, &facts)?;
    let Some(gear) = gear.filter(|_| is_equip_position(from) || is_equip_position(to)) else {
        if is_equip_position(from) || is_equip_position(to) {
            return Err(MoveRefused::NotPorted(Unported::Equipment));
        }
        check_placement(to, &facts)?;
        let place = Place {
            item: &item,
            from,
            to,
            count,
        };
        return grid_move(items, ids, place, rules, None);
    };
    if is_dragon_soul_equip_position(from) || is_dragon_soul_equip_position(to) || facts.dragon_soul
    {
        return Err(MoveRefused::NotPorted(Unported::DragonSoul));
    }
    let mut equipper = Equipper::new(items, &mut *gear, rules.usable_cells);
    let outcome = wear_move(&mut equipper, &item, from, to);
    let trail = equipper.into_trail();
    let worn = match outcome {
        Ok(Some(kind)) => return Ok(done(kind, trail)),
        Ok(None) => from.cell - INVENTORY_MAX_NUM,
        Err(refused) => return declined(refused, trail),
    };
    let place = Place {
        item: &item,
        from,
        to,
        count,
    };
    let moved = check_placement(to, &facts)
        .and_then(|()| grid_move(items, ids, place, rules, Some((worn, gear))));
    match moved {
        Ok(mut moved) => {
            let mut records = trail.records;
            records.append(&mut moved.records);
            let mut changes = trail.changes;
            changes.append(&mut moved.changes);
            Ok(MoveDone {
                kind: moved.kind,
                records,
                changes,
            })
        }
        Err(refused) => declined(refused, trail),
    }
}

/// A finished wear-cell path.
pub(crate) fn done(kind: MoveKind, trail: Trail) -> MoveDone {
    MoveDone {
        kind,
        records: trail.records,
        changes: trail.changes,
    }
}

/// A wear-cell path that refused: nothing to answer but the refusal when it had not changed
/// anything or sent anything, else what it did, with the refusal's notice last.
pub(crate) fn declined(refused: MoveRefused, mut trail: Trail) -> Result<MoveDone, MoveRefused> {
    if trail.records.is_empty() && trail.changes.is_empty() {
        return Err(refused);
    }
    if let Some(text) = refused.notice() {
        trail.records.push(MoveRecord::Notice(text));
    }
    Ok(done(MoveKind::Declined, trail))
}

/// The wear-cell branches of `MoveItem` (`char_item.cpp:7725-7757`). `None` sends a worn item
/// on to the grid path, which takes it off.
fn wear_move(
    equipper: &mut Equipper<'_, '_>,
    item: &Item,
    from: ItemPos,
    to: ItemPos,
) -> Result<Option<MoveKind>, MoveRefused> {
    if is_equip_position(from) {
        let proto = equipper.proto(item.vnum)?;
        equipper.can_unequip_now(item, proto)?;
        if find_equip_cell(equipper.items(), proto) == Some(EWearPositions::Weapon as u16) {
            equipper.free_costume_weapon(MoveRefused::CostumeWeaponStuckOnMove)?;
            if !equipper.is_empty_grid(to, item.size, Some(from.cell)) {
                equipper.unequip_item(item)?;
                return Ok(Some(MoveKind::Unequipped));
            }
        }
    }
    if !is_equip_position(to) {
        return Ok(None);
    }
    if equipper.items().item_at(to).is_some() {
        return Err(MoveRefused::WearCellTaken);
    }
    Ok(Some(if equipper.equip_item(item)? {
        MoveKind::Swapped
    } else {
        MoveKind::Equipped
    }))
}

/// The destination the server chooses when the client sends [`AUTO_FIND_CELL`].
///
/// From the base inventory, the first of the item's banks with room
/// (`GetEmptyInventory(item, 2)`); from a bank, the first free base cell
/// (`GetEmptyInventory(item, 1)`). From anywhere else the cell stays [`AUTO_FIND_CELL`],
/// which the destination check then refuses, as legacy's does.
fn auto_find(
    items: &CharacterItems,
    item: &Item,
    from: ItemPos,
    facts: &MoveFacts,
    rules: &MoveRules,
) -> Result<u16, MoveRefused> {
    if is_default_inventory_position(from) {
        return facts
            .categories
            .iter()
            .find_map(|&category| items.find_free_custom_cell(category, item.size))
            .ok_or(MoveRefused::NoRoomInSpecialInventory);
    }
    if is_custom_inventory_position(from) {
        return items
            .find_free_inventory_cell(rules.usable_cells, item.size)
            .ok_or(MoveRefused::NoRoomInInventory);
    }
    Ok(AUTO_FIND_CELL)
}

/// The destination checks before the wear-cell branches, in legacy's order
/// (`char_item.cpp:7672-7717`).
fn check_destination(to: ItemPos, facts: &MoveFacts) -> Result<(), MoveRefused> {
    if !is_valid_item_position(to) {
        return Err(MoveRefused::InvalidDestination);
    }
    if is_custom_inventory_position(to)
        && !custom_inventory_category_of(to).is_some_and(|bank| facts.categories.contains(&bank))
    {
        return Err(MoveRefused::WrongBank);
    }
    if is_belt_inventory_position(to) && !facts.belt_eligible {
        return Err(MoveRefused::NotForBelt);
    }
    if is_switchbot_position(to) {
        return Err(MoveRefused::NotPorted(Unported::Switchbot));
    }
    Ok(())
}

/// The destination checks after the wear-cell branches (`char_item.cpp:7758-7782`).
fn check_placement(to: ItemPos, facts: &MoveFacts) -> Result<(), MoveRefused> {
    if facts.dragon_soul {
        return Err(MoveRefused::NotPorted(Unported::DragonSoul));
    }
    if to.window_type == EWindows::DragonSoulInventory as u8 {
        return Err(MoveRefused::NotDragonSoulItem);
    }
    if !is_flat_window(to.window_type) {
        return Err(MoveRefused::NotPorted(Unported::DestinationWindow(
            to.window_type,
        )));
    }
    Ok(())
}

/// What a grid move moves, and from where to where.
#[derive(Debug, Clone, Copy)]
struct Place<'a> {
    item: &'a Item,
    from: ItemPos,
    to: ItemPos,
    count: u16,
}

/// The wear cell a worn item leaves, with the gear its taking off changes.
type Worn<'a, 'g> = Option<(u16, &'a mut Gear<'g>)>;

/// The grid path of `MoveItem` (`char_item.cpp:7760-7846`): a merge, a whole move or a split.
fn grid_move(
    items: &mut CharacterItems,
    ids: Option<&mut ItemIds>,
    place: Place<'_>,
    rules: &MoveRules,
    worn: Worn<'_, '_>,
) -> Result<MoveDone, MoveRefused> {
    let Place {
        item,
        from,
        to,
        count,
    } = place;
    if let Some(target) = items
        .item_at(to)
        .filter(|target| target.id != item.id && target.stacks() && target.vnum == item.vnum)
        .cloned()
    {
        if target.sockets != item.sockets {
            return Err(MoveRefused::SocketsDiffer);
        }
        let usable_cells = rules.usable_cells;
        return merge(
            items,
            item,
            &target,
            count,
            rules.count_limit,
            (worn, usable_cells),
        );
    }
    let fits = |exception: Option<u16>| {
        items.is_empty_item_grid(
            to,
            item.size,
            exception,
            rules.usable_cells,
            rules.belt_grade,
        )
    };
    if !fits(Some(from.cell)) {
        return Err(MoveRefused::NoRoom);
    }
    if count == 0 || count >= item.count || !item.stacks() {
        return match worn {
            Some((wear, gear)) => {
                let mut trail = take_off(items, gear, rules.usable_cells, wear, item)?;
                items.set(to, item).map_err(MoveRefused::Storage)?;
                trail
                    .records
                    .push(MoveRecord::Item(ItemRecord::Set(item.gc_item_set(to, 0))));
                let pos = stored(items, item.id)?.pos;
                trail.changes.push(ItemChange::Moved { id: item.id, pos });
                Ok(done(MoveKind::Moved, trail))
            }
            None => whole_move(items, item, from, to),
        };
    }
    if !fits(None) {
        return Err(MoveRefused::NoRoom);
    }
    split(items, ids, item, to, count)
}

/// `RemoveFromCharacter` of a worn item: `Unequip`, with what it sends.
fn take_off(
    items: &mut CharacterItems,
    gear: &mut Gear<'_>,
    usable_cells: u16,
    wear: u16,
    item: &Item,
) -> Result<Trail, MoveRefused> {
    let mut equipper = Equipper::new(items, gear, usable_cells);
    equipper.take_off(wear, item)?;
    Ok(equipper.into_trail())
}

/// The stored copy of an item, which is what every record after a change describes.
fn stored(items: &CharacterItems, id: ItemId) -> Result<&Item, MoveRefused> {
    items
        .item(id)
        .ok_or(MoveRefused::Count(CountRefused::NotHeld(id)))
}

/// `ITEM_STACK` (`char_item.cpp:7786-7805`).
///
/// A count of 0 asks for the whole stack, and what moves is capped by the room left under
/// the count limit. The source is updated first and then the target, which is the order
/// legacy's two `SetCount` calls send their records in. A source that is used up is taken out
/// of the storage and cleared on the client, which is what `SetCount(0)` does through
/// `M2_DESTROY_ITEM`. When nothing moves, both records are still sent, and there is nothing
/// to store.
fn merge(
    items: &mut CharacterItems,
    item: &Item,
    target: &Item,
    count: u16,
    count_limit: u16,
    (worn, usable_cells): (Worn<'_, '_>, u16),
) -> Result<MoveDone, MoveRefused> {
    let asked = if count == 0 { item.count } else { count };
    let room = count_limit
        .min(ITEM_COUNT_LIMIT)
        .saturating_sub(target.count);
    let moved = asked.min(room);
    let left = item.count - moved;
    let total = target.count + moved;
    let _ = items
        .set_count(target.id, u32::from(total))
        .map_err(MoveRefused::Count)?;
    let mut records = Vec::with_capacity(2);
    let mut changes = Vec::with_capacity(2);
    if left == 0 {
        if let Some((wear, gear)) = worn {
            let trail = take_off(items, gear, usable_cells, wear, item)?;
            records = trail.records;
            changes = trail.changes;
        } else {
            let pos = items.release(item.id).map_err(MoveRefused::Storage)?;
            records.push(MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos))));
        }
        changes.push(ItemChange::Destroyed { id: item.id });
    } else {
        let _ = items
            .set_count(item.id, u32::from(left))
            .map_err(MoveRefused::Count)?;
        records.push(MoveRecord::Item(ItemRecord::Update(
            stored(items, item.id)?.gc_item_update(),
        )));
        if moved > 0 {
            changes.push(ItemChange::Count {
                id: item.id,
                count: left,
            });
        }
    }
    records.push(MoveRecord::Item(ItemRecord::Update(
        stored(items, target.id)?.gc_item_update(),
    )));
    if moved > 0 {
        changes.push(ItemChange::Count {
            id: target.id,
            count: total,
        });
    }
    Ok(MoveDone {
        kind: MoveKind::Merged,
        records,
        changes,
    })
}

/// `ITEM_MOVE` (`char_item.cpp:7810-7824`): `RemoveFromCharacter` clears the old cell, and
/// `SetItem(DestCell, item, false)` fills the new one with no highlight.
///
/// The clear names the cell in stored terms, because `RemoveFromCharacter` addresses the
/// item's own window. The set names the destination as the client sent it, because
/// `SetItem` copies `Cell` into the record (`char_item.cpp:571`).
fn whole_move(
    items: &mut CharacterItems,
    item: &Item,
    from: ItemPos,
    to: ItemPos,
) -> Result<MoveDone, MoveRefused> {
    let old = item.pos;
    items
        .move_item(from, to, item)
        .map_err(MoveRefused::Storage)?;
    let pos = stored(items, item.id)?.pos;
    Ok(MoveDone {
        kind: MoveKind::Moved,
        records: vec![
            MoveRecord::Item(ItemRecord::Set(gc_item_clear(old))),
            MoveRecord::Item(ItemRecord::Set(item.gc_item_set(to, 0))),
        ],
        changes: vec![ItemChange::Moved { id: item.id, pos }],
    })
}

/// `ITEM_SPLIT` (`char_item.cpp:7825-7846`).
///
/// The source keeps what is left and is updated first. The new item is `CreateItem`'s:
/// the same vnum, and so the same size and flags, the moved count, and the source's sockets
/// (`FN_copy_item_socket`), with no attributes, element or transmutation. It is placed with
/// no highlight.
fn split(
    items: &mut CharacterItems,
    ids: Option<&mut ItemIds>,
    item: &Item,
    to: ItemPos,
    count: u16,
) -> Result<MoveDone, MoveRefused> {
    let ids = ids.ok_or(MoveRefused::NoItemIds)?;
    let id = ids.allocate().map_err(|_| MoveRefused::IdsExhausted)?;
    let mut piece = Item::new(id, item.vnum);
    piece.count = count;
    piece.flags = item.flags;
    piece.anti_flags = item.anti_flags;
    piece.size = item.size;
    piece.sockets = item.sockets;
    items.set(to, &piece).map_err(MoveRefused::Storage)?;
    let left = item.count - count;
    if let Err(reason) = items.set_count(item.id, u32::from(left)) {
        // Put the storage back as it was: the new item goes, and its id is spent, which is
        // harmless because an id is never reused.
        let _ = items.release(id);
        return Err(MoveRefused::Count(reason));
    }
    let placed = stored(items, id)?.clone();
    Ok(MoveDone {
        kind: MoveKind::Split,
        records: vec![
            MoveRecord::Item(ItemRecord::Update(stored(items, item.id)?.gc_item_update())),
            MoveRecord::Item(ItemRecord::Set(piece.gc_item_set(to, 0))),
        ],
        changes: vec![
            ItemChange::Count {
                id: item.id,
                count: left,
            },
            ItemChange::Created(placed),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{ItemIdRange, ITEM_ANTIFLAG_STACK, ITEM_FLAG_STACKABLE};

    const INV: u8 = EWindows::Inventory as u8;
    const EQUIP: u8 = EWindows::Equipment as u8;

    /// The item records a move sends, in order.
    fn item_records(done: &MoveDone) -> Vec<ItemRecord> {
        done.records
            .iter()
            .filter_map(|record| match record {
                MoveRecord::Item(record) => Some(*record),
                _ => None,
            })
            .collect()
    }
    const DS: u8 = EWindows::DragonSoulInventory as u8;
    const BANK_2: u16 = CUSTOM_INVENTORY_SLOT_START + 2 * 180;
    const BANK_3: u16 = CUSTOM_INVENTORY_SLOT_START + 3 * 180;
    const BELT: u16 = 274;

    const RULES: MoveRules = MoveRules {
        count_limit: 200,
        usable_cells: 90,
        belt_grade: None,
    };

    const fn at(window_type: u8, cell: u16) -> ItemPos {
        ItemPos { window_type, cell }
    }

    fn plain(id: ItemId, vnum: u32) -> Item {
        Item::new(id, vnum)
    }

    fn stack(id: ItemId, vnum: u32, count: u16) -> Item {
        let mut item = Item::new(id, vnum);
        item.count = count;
        item.flags = ITEM_FLAG_STACKABLE;
        item.sockets = [0x0102_0304, 0, 0, 0, 0, 0x0506];
        item
    }

    fn facts(categories: &[u8]) -> MoveFacts {
        MoveFacts {
            categories: categories.to_vec(),
            belt_eligible: false,
            dragon_soul: false,
        }
    }

    fn holding(placed: &[(ItemPos, Item)]) -> CharacterItems {
        let mut items = CharacterItems::new();
        for (pos, item) in placed {
            items.set(*pos, item).expect("the fixture places");
        }
        items
    }

    fn ids() -> ItemIds {
        ItemIds::new(ItemIdRange::new(1000, 2000, 1000).expect("a valid range"))
    }

    fn run(
        items: &mut CharacterItems,
        from: ItemPos,
        to: ItemPos,
        count: u16,
        known: MoveFacts,
    ) -> Result<MoveDone, MoveRefused> {
        let mut ids = ids();
        let request = MoveRequest { from, to, count };
        move_item(items, Some(&mut ids), request, &RULES, None, |_| {
            Some(known)
        })
    }

    /// Run a move that must be refused and check it changed nothing.
    fn refused(
        items: &mut CharacterItems,
        from: ItemPos,
        to: ItemPos,
        count: u16,
        known: MoveFacts,
    ) -> MoveRefused {
        let before = items.clone();
        let reason = run(items, from, to, count, known).expect_err("the move is refused");
        assert_eq!(*items, before, "a refused move changed the storage");
        reason
    }

    #[test]
    fn a_whole_move_clears_the_old_cell_and_sets_the_new_one() {
        let sword = plain(7, 19);
        let mut items = holding(&[(at(INV, 3), sword.clone())]);
        let done = run(&mut items, at(INV, 3), at(INV, 40), 0, facts(&[])).expect("moves");
        assert_eq!(done.kind, MoveKind::Moved);
        assert_eq!(
            item_records(&done),
            vec![
                ItemRecord::Set(gc_item_clear(at(INV, 3))),
                ItemRecord::Set(sword.gc_item_set(at(INV, 40), 0)),
            ]
        );
        assert_eq!(
            done.changes,
            vec![ItemChange::Moved {
                id: 7,
                pos: at(INV, 40)
            }]
        );
        assert_eq!(items.item_at(at(INV, 40)).map(|i| i.id), Some(7));
        assert!(items.item_at(at(INV, 3)).is_none());
    }

    #[test]
    fn a_two_cell_item_steps_down_onto_its_own_lower_cell() {
        let mut tall = plain(7, 19);
        tall.size = 2;
        let mut items = holding(&[(at(INV, 3), tall)]);
        let done = run(&mut items, at(INV, 3), at(INV, 8), 0, facts(&[])).expect("moves");
        assert_eq!(done.kind, MoveKind::Moved);
        assert_eq!(items.anchor_cell(at(INV, 13)), Some(8));
        assert_eq!(items.anchor_cell(at(INV, 3)), None);
    }

    #[test]
    fn a_move_through_window_2_is_cleared_in_window_1() {
        // The client may name a base cell through the equipment window; the item is stored
        // in window 1 and the clear addresses it there, as `RemoveFromCharacter` does.
        let sword = plain(7, 19);
        let mut items = holding(&[(at(INV, 3), sword)]);
        let done = run(&mut items, at(EQUIP, 3), at(INV, 4), 0, facts(&[])).expect("moves");
        assert_eq!(
            item_records(&done)[0],
            ItemRecord::Set(gc_item_clear(at(INV, 3)))
        );
    }

    #[test]
    fn a_move_into_a_locked_page_or_onto_another_item_is_refused() {
        let mut items = holding(&[(at(INV, 3), plain(7, 19)), (at(INV, 4), plain(8, 19))]);
        assert_eq!(
            refused(&mut items, at(INV, 3), at(INV, 90), 0, facts(&[])),
            MoveRefused::NoRoom
        );
        assert_eq!(
            refused(&mut items, at(INV, 3), at(INV, 4), 0, facts(&[])),
            MoveRefused::NoRoom
        );
    }

    #[test]
    fn the_source_checks_come_first_and_change_nothing() {
        let mut items = holding(&[(at(INV, 3), stack(7, 27001, 5))]);
        assert_eq!(
            refused(&mut items, at(INV, 1370), at(INV, 4), 0, facts(&[])),
            MoveRefused::InvalidSource
        );
        assert_eq!(
            refused(&mut items, at(INV, 9), at(INV, 4), 0, facts(&[])),
            MoveRefused::Empty
        );
        assert_eq!(
            refused(&mut items, at(INV, 3), at(INV, 4), 6, facts(&[])),
            MoveRefused::CountAboveStack { held: 5, asked: 6 }
        );
        assert_eq!(
            refused(&mut items, at(DS, 3), at(INV, 4), 0, facts(&[])),
            MoveRefused::NotPorted(Unported::SourceWindow(DS))
        );
        assert_eq!(
            refused(&mut items, at(INV, 3), at(INV, 1370), 0, facts(&[])),
            MoveRefused::InvalidDestination
        );
        let mut ids = ids();
        let request = MoveRequest {
            from: at(INV, 3),
            to: at(INV, 4),
            count: 0,
        };
        assert_eq!(
            move_item(&mut items, Some(&mut ids), request, &RULES, None, |_| None),
            Err(MoveRefused::UnknownVnum(27001))
        );
    }

    #[test]
    fn an_irremovable_item_stays_in_the_belt_through_either_window() {
        let mut potion = plain(7, 27001);
        potion.flags = ITEM_FLAG_IRREMOVABLE;
        let mut items = holding(&[(at(INV, BELT), potion)]);
        assert_eq!(
            refused(&mut items, at(INV, BELT), at(INV, 4), 0, facts(&[])),
            MoveRefused::Irremovable
        );
        // Legacy tests window 1 only; window 2 names the same cell and is refused too.
        assert_eq!(
            refused(&mut items, at(EQUIP, BELT), at(INV, 4), 0, facts(&[])),
            MoveRefused::Irremovable
        );
    }

    #[test]
    fn an_irremovable_item_moves_inside_the_base_inventory() {
        let mut quest = plain(7, 50001);
        quest.flags = ITEM_FLAG_IRREMOVABLE;
        let mut items = holding(&[(at(INV, 3), quest)]);
        assert!(run(&mut items, at(INV, 3), at(INV, 4), 0, facts(&[])).is_ok());
    }

    #[test]
    fn an_irremovable_item_in_the_first_equipment_cell_is_refused_before_the_equipment() {
        // `INVENTORY_MAX_NUM` itself is outside the base inventory (`:7627`), so the flag
        // refuses the move before the equipment path is reached.
        let mut armour = plain(7, 11200);
        armour.flags = ITEM_FLAG_IRREMOVABLE;
        let mut items = holding(&[(at(INV, INVENTORY_MAX_NUM), armour)]);
        assert_eq!(
            refused(
                &mut items,
                at(INV, INVENTORY_MAX_NUM),
                at(INV, 4),
                0,
                facts(&[])
            ),
            MoveRefused::Irremovable
        );
    }

    #[test]
    fn the_equipment_the_dragon_soul_and_the_switchbot_are_not_ported() {
        let mut items = holding(&[(at(INV, 3), plain(7, 19)), (at(INV, 207), plain(8, 18))]);
        assert_eq!(
            refused(&mut items, at(INV, 3), at(INV, 180), 0, facts(&[])),
            MoveRefused::NotPorted(Unported::Equipment)
        );
        assert_eq!(
            refused(&mut items, at(INV, 207), at(INV, 4), 0, facts(&[])),
            MoveRefused::NotPorted(Unported::Equipment)
        );
        assert_eq!(
            refused(&mut items, at(INV, 3), at(8, 0), 0, facts(&[])),
            MoveRefused::NotPorted(Unported::Switchbot)
        );
        assert_eq!(
            refused(&mut items, at(INV, 3), at(DS, 0), 0, facts(&[])),
            MoveRefused::NotDragonSoulItem
        );
        let dragon = MoveFacts {
            dragon_soul: true,
            ..facts(&[])
        };
        assert_eq!(
            refused(&mut items, at(INV, 3), at(INV, 4), 0, dragon),
            MoveRefused::NotPorted(Unported::DragonSoul)
        );
    }

    /// A storage whose bank `bank` has no free cell.
    fn with_full_bank(items: &mut CharacterItems, bank: u8) {
        let start = CUSTOM_INVENTORY_SLOT_START + u16::from(bank) * 180;
        for (offset, cell) in (start..start + 180).enumerate() {
            let id = 5000 + u32::from(bank) * 1000 + u32::try_from(offset).expect("small");
            items.set(at(INV, cell), &plain(id, 1)).expect("fills");
        }
    }

    #[test]
    fn auto_find_from_the_base_inventory_takes_the_first_bank_with_room() {
        let book = plain(7, 50300);
        let mut items = holding(&[(at(INV, 3), book)]);
        let done = run(
            &mut items,
            at(INV, 3),
            at(INV, AUTO_FIND_CELL),
            0,
            facts(&[2, 3]),
        )
        .expect("finds bank 2");
        assert_eq!(
            done.changes,
            vec![ItemChange::Moved {
                id: 7,
                pos: at(INV, BANK_2)
            }]
        );
    }

    #[test]
    fn an_item_in_two_banks_is_accepted_in_its_second_bank() {
        // Defect 2: legacy finds bank 3 and then refuses it, because the item's first bank
        // is 2. Here the bank the search chose is accepted.
        let mut items = holding(&[(at(INV, 3), plain(7, 27987))]);
        with_full_bank(&mut items, 2);
        let done = run(
            &mut items,
            at(INV, 3),
            at(INV, AUTO_FIND_CELL),
            0,
            facts(&[2, 3]),
        )
        .expect("finds bank 3");
        assert_eq!(
            done.changes,
            vec![ItemChange::Moved {
                id: 7,
                pos: at(INV, BANK_3)
            }]
        );
        // And it can move inside bank 3 afterwards.
        assert!(run(
            &mut items,
            at(INV, BANK_3),
            at(INV, BANK_3 + 1),
            0,
            facts(&[2, 3])
        )
        .is_ok());
    }

    #[test]
    fn auto_find_with_no_bank_room_sends_the_special_inventory_notice() {
        // Defect 1: legacy's `WORD != -1` is always true, so it never sends this.
        let mut items = holding(&[(at(INV, 3), plain(7, 27987))]);
        with_full_bank(&mut items, 2);
        let reason = refused(
            &mut items,
            at(INV, 3),
            at(INV, AUTO_FIND_CELL),
            0,
            facts(&[2]),
        );
        assert_eq!(reason, MoveRefused::NoRoomInSpecialInventory);
        assert_eq!(
            reason.notice(),
            Some("Nu ai spatiu suficient in inventarul special.")
        );
        // An item in no bank has no bank with room either.
        let reason = refused(
            &mut items,
            at(INV, 3),
            at(INV, AUTO_FIND_CELL),
            0,
            facts(&[]),
        );
        assert_eq!(reason, MoveRefused::NoRoomInSpecialInventory);
    }

    #[test]
    fn auto_find_from_a_bank_takes_the_first_usable_base_cell() {
        let mut items = holding(&[
            (at(INV, BANK_2), plain(7, 27987)),
            (at(INV, 0), plain(8, 1)),
        ]);
        let done = run(
            &mut items,
            at(INV, BANK_2),
            at(INV, AUTO_FIND_CELL),
            0,
            facts(&[2]),
        )
        .expect("finds cell 1");
        assert_eq!(
            done.changes,
            vec![ItemChange::Moved {
                id: 7,
                pos: at(INV, 1)
            }]
        );
    }

    #[test]
    fn auto_find_from_a_bank_stops_at_the_usable_cells() {
        let mut items = holding(&[(at(INV, BANK_2), plain(7, 27987))]);
        for cell in 0..90 {
            items
                .set(at(INV, cell), &plain(100 + u32::from(cell), 1))
                .expect("fills");
        }
        let reason = refused(
            &mut items,
            at(INV, BANK_2),
            at(INV, AUTO_FIND_CELL),
            0,
            facts(&[2]),
        );
        assert_eq!(reason, MoveRefused::NoRoomInInventory);
        assert_eq!(reason.notice(), Some("Nu ai spatiu suficient in inventar."));
    }

    #[test]
    fn auto_find_from_the_belt_is_an_invalid_destination() {
        let mut items = holding(&[(at(INV, BELT), plain(7, 27001))]);
        let reason = refused(
            &mut items,
            at(INV, BELT),
            at(INV, AUTO_FIND_CELL),
            0,
            facts(&[]),
        );
        assert_eq!(reason, MoveRefused::InvalidDestination);
        assert_eq!(reason.notice(), None);
    }

    #[test]
    fn a_bank_the_item_is_not_in_is_refused_with_the_notice() {
        let mut items = holding(&[(at(INV, 3), plain(7, 19))]);
        let reason = refused(&mut items, at(INV, 3), at(INV, BANK_2), 0, facts(&[3]));
        assert_eq!(reason, MoveRefused::WrongBank);
        assert_eq!(
            reason.notice(),
            Some("@@(char_item.cpp)tradus:[Special_Inventory]Nu poti plasa acest obiect aici.")
        );
        assert_eq!(
            refused(&mut items, at(INV, 3), at(INV, BANK_2), 0, facts(&[])),
            MoveRefused::WrongBank
        );
    }

    #[test]
    fn the_belt_takes_an_eligible_item_in_a_cell_its_grade_opens() {
        let potion = MoveFacts {
            belt_eligible: true,
            ..facts(&[])
        };
        let mut items = holding(&[(at(INV, 3), plain(7, 27001))]);
        let reason = refused(&mut items, at(INV, 3), at(INV, BELT), 0, facts(&[]));
        assert_eq!(reason, MoveRefused::NotForBelt);
        assert_eq!(reason.notice(), Some("[LS;1097]"));
        // No belt worn: no belt cell is open.
        assert_eq!(
            refused(&mut items, at(INV, 3), at(INV, BELT), 0, potion.clone()),
            MoveRefused::NoRoom
        );
        let graded = MoveRules {
            belt_grade: Some(1),
            ..RULES
        };
        let mut ids = ids();
        let into = |cell| MoveRequest {
            from: at(INV, 3),
            to: at(INV, cell),
            count: 0,
        };
        // Grade 1 opens the first belt cell and not the second, which needs grade 2.
        assert_eq!(
            move_item(
                &mut items,
                Some(&mut ids),
                into(BELT + 1),
                &graded,
                None,
                |_| { Some(potion.clone()) }
            ),
            Err(MoveRefused::NoRoom)
        );
        let done = move_item(
            &mut items,
            Some(&mut ids),
            into(BELT),
            &graded,
            None,
            |_| Some(potion.clone()),
        )
        .expect("moves into the belt");
        assert_eq!(
            done.changes,
            vec![ItemChange::Moved {
                id: 7,
                pos: at(INV, BELT)
            }]
        );
    }

    #[test]
    fn a_partial_merge_updates_the_source_and_then_the_target() {
        let mut items = holding(&[
            (at(INV, 3), stack(7, 27001, 30)),
            (at(INV, 4), stack(8, 27001, 100)),
        ]);
        let done = run(&mut items, at(INV, 3), at(INV, 4), 12, facts(&[])).expect("merges");
        assert_eq!(done.kind, MoveKind::Merged);
        let source = items.item(7).expect("still held").clone();
        let target = items.item(8).expect("still held").clone();
        assert_eq!((source.count, target.count), (18, 112));
        assert_eq!(
            item_records(&done),
            vec![
                ItemRecord::Update(source.gc_item_update()),
                ItemRecord::Update(target.gc_item_update()),
            ]
        );
        assert_eq!(
            done.changes,
            vec![
                ItemChange::Count { id: 7, count: 18 },
                ItemChange::Count { id: 8, count: 112 },
            ]
        );
    }

    #[test]
    fn a_whole_merge_destroys_the_source_and_clears_its_cell() {
        let mut items = holding(&[
            (at(INV, 3), stack(7, 27001, 30)),
            (at(INV, 4), stack(8, 27001, 100)),
        ]);
        let done = run(&mut items, at(INV, 3), at(INV, 4), 0, facts(&[])).expect("merges");
        let target = items.item(8).expect("still held").clone();
        assert_eq!(target.count, 130);
        assert!(items.item(7).is_none());
        assert!(items.item_at(at(INV, 3)).is_none());
        assert_eq!(
            item_records(&done),
            vec![
                ItemRecord::Set(gc_item_clear(at(INV, 3))),
                ItemRecord::Update(target.gc_item_update()),
            ]
        );
        assert_eq!(
            done.changes,
            vec![
                ItemChange::Destroyed { id: 7 },
                ItemChange::Count { id: 8, count: 130 },
            ]
        );
    }

    #[test]
    fn a_merge_is_capped_by_the_count_limit() {
        let mut items = holding(&[
            (at(INV, 3), stack(7, 27001, 30)),
            (at(INV, 4), stack(8, 27001, 190)),
        ]);
        let done = run(&mut items, at(INV, 3), at(INV, 4), 0, facts(&[])).expect("merges");
        assert_eq!(
            done.changes,
            vec![
                ItemChange::Count { id: 7, count: 20 },
                ItemChange::Count { id: 8, count: 200 },
            ]
        );
    }

    #[test]
    fn a_merge_onto_a_full_stack_sends_both_records_and_stores_nothing() {
        // Legacy's two `SetCount` calls each send an update, even with nothing moved.
        let mut items = holding(&[
            (at(INV, 3), stack(7, 27001, 30)),
            (at(INV, 4), stack(8, 27001, 200)),
        ]);
        let before = items.clone();
        let done = run(&mut items, at(INV, 3), at(INV, 4), 0, facts(&[])).expect("merges");
        assert_eq!(items, before);
        assert_eq!(done.records.len(), 2);
        assert!(done.changes.is_empty());
        // A stack above the limit (a limit lowered after the fact) takes nothing, and the
        // source does not grow the way legacy's unsigned `MIN` would let it.
        let mut items = holding(&[
            (at(INV, 3), stack(7, 27001, 30)),
            (at(INV, 4), stack(8, 27001, 250)),
        ]);
        let done = run(&mut items, at(INV, 3), at(INV, 4), 0, facts(&[])).expect("merges");
        assert!(done.changes.is_empty());
        assert_eq!(items.item(7).map(|i| i.count), Some(30));
    }

    #[test]
    fn a_merge_is_capped_at_the_legacy_ceiling_whatever_the_limit() {
        let rules = MoveRules {
            count_limit: u16::MAX,
            ..RULES
        };
        let mut items = holding(&[
            (at(INV, 3), stack(7, 27001, 30)),
            (at(INV, 4), stack(8, 27001, ITEM_COUNT_LIMIT - 10)),
        ]);
        let mut ids = ids();
        let request = MoveRequest {
            from: at(INV, 3),
            to: at(INV, 4),
            count: 0,
        };
        let done = move_item(&mut items, Some(&mut ids), request, &rules, None, |_| {
            Some(facts(&[]))
        })
        .expect("merges");
        assert_eq!(
            done.changes,
            vec![
                ItemChange::Count { id: 7, count: 20 },
                ItemChange::Count {
                    id: 8,
                    count: ITEM_COUNT_LIMIT
                },
            ]
        );
    }

    #[test]
    fn a_stack_moved_onto_its_own_cell_is_a_whole_move_and_not_a_merge() {
        // Legacy's merge needs `item != item2` (`:7786`), so the stack is taken out of its
        // cell and set back into it.
        let potions = stack(7, 27001, 30);
        let mut items = holding(&[(at(INV, 3), potions.clone())]);
        let done = run(&mut items, at(INV, 3), at(INV, 3), 0, facts(&[])).expect("moves");
        assert_eq!(done.kind, MoveKind::Moved);
        assert_eq!(
            item_records(&done),
            vec![
                ItemRecord::Set(gc_item_clear(at(INV, 3))),
                ItemRecord::Set(potions.gc_item_set(at(INV, 3), 0)),
            ]
        );
        assert_eq!(items.item_at(at(INV, 3)).map(|i| i.count), Some(30));
    }

    #[test]
    fn stacks_with_different_sockets_do_not_merge() {
        let mut other = stack(8, 27001, 100);
        other.sockets[5] = 0x0605;
        let mut items = holding(&[(at(INV, 3), stack(7, 27001, 30)), (at(INV, 4), other)]);
        assert_eq!(
            refused(&mut items, at(INV, 3), at(INV, 4), 0, facts(&[])),
            MoveRefused::SocketsDiffer
        );
    }

    #[test]
    fn a_target_that_does_not_stack_is_an_occupied_cell() {
        let mut unstackable = stack(8, 27001, 100);
        unstackable.anti_flags = ITEM_ANTIFLAG_STACK;
        let mut items = holding(&[(at(INV, 3), stack(7, 27001, 30)), (at(INV, 4), unstackable)]);
        assert_eq!(
            refused(&mut items, at(INV, 3), at(INV, 4), 0, facts(&[])),
            MoveRefused::NoRoom
        );
        // Another vnum is an occupied cell too.
        let mut items = holding(&[
            (at(INV, 3), stack(7, 27001, 30)),
            (at(INV, 4), stack(8, 27002, 100)),
        ]);
        assert_eq!(
            refused(&mut items, at(INV, 3), at(INV, 4), 0, facts(&[])),
            MoveRefused::NoRoom
        );
    }

    #[test]
    fn a_split_makes_a_new_item_with_the_sockets_and_no_attributes() {
        let mut source = stack(7, 27001, 30);
        source.attributes[0] = protocol::gc_item_window::ItemAttribute::new(1, 0x0203);
        source.refine_element = 0x0405_0607;
        let mut items = holding(&[(at(INV, 3), source)]);
        let done = run(&mut items, at(INV, 3), at(INV, 40), 12, facts(&[])).expect("splits");
        assert_eq!(done.kind, MoveKind::Split);
        let left = items.item(7).expect("still held").clone();
        let piece = items.item(1000).expect("the first id").clone();
        assert_eq!(left.count, 18);
        assert_eq!(piece.count, 12);
        assert_eq!(piece.pos, at(INV, 40));
        assert_eq!(piece.sockets, left.sockets);
        assert_eq!(piece.flags, ITEM_FLAG_STACKABLE);
        assert_eq!(piece.attributes, plain(9, 1).attributes);
        assert_eq!(piece.refine_element, 0);
        assert_eq!(
            item_records(&done),
            vec![
                ItemRecord::Update(left.gc_item_update()),
                ItemRecord::Set(piece.gc_item_set(at(INV, 40), 0)),
            ]
        );
        assert_eq!(
            done.changes,
            vec![
                ItemChange::Count { id: 7, count: 18 },
                ItemChange::Created(piece),
            ]
        );
    }

    #[test]
    fn a_count_that_is_the_whole_stack_or_an_unstackable_item_moves_whole() {
        let mut items = holding(&[(at(INV, 3), stack(7, 27001, 30))]);
        let done = run(&mut items, at(INV, 3), at(INV, 40), 30, facts(&[])).expect("moves");
        assert_eq!(done.kind, MoveKind::Moved);
        let mut unstackable = stack(8, 27001, 30);
        unstackable.anti_flags = ITEM_ANTIFLAG_STACK;
        let mut items = holding(&[(at(INV, 3), unstackable)]);
        let done = run(&mut items, at(INV, 3), at(INV, 40), 12, facts(&[])).expect("moves");
        assert_eq!(done.kind, MoveKind::Moved);
        assert_eq!(items.item(8).map(|i| i.count), Some(30));
    }

    #[test]
    fn a_split_needs_an_id() {
        let mut items = holding(&[(at(INV, 3), stack(7, 27001, 30))]);
        let before = items.clone();
        let request = MoveRequest {
            from: at(INV, 3),
            to: at(INV, 40),
            count: 12,
        };
        assert_eq!(
            move_item(&mut items, None, request, &RULES, None, |_| Some(
                facts(&[])
            )),
            Err(MoveRefused::NoItemIds)
        );
        let mut spent = ItemIds::new(ItemIdRange::new(1000, 1001, 1000).expect("a valid range"));
        let _ = spent.allocate().expect("the one id");
        assert_eq!(
            move_item(&mut items, Some(&mut spent), request, &RULES, None, |_| {
                Some(facts(&[]))
            }),
            Err(MoveRefused::IdsExhausted)
        );
        assert_eq!(items, before);
    }

    #[test]
    fn a_split_may_not_land_on_its_own_stack() {
        // Defect 3: legacy checks a split with the source cell as the exception, so a
        // two-cell stack could split onto its own lower cell.
        let mut tall = stack(7, 27001, 30);
        tall.size = 2;
        let mut items = holding(&[(at(INV, 3), tall)]);
        assert_eq!(
            refused(&mut items, at(INV, 3), at(INV, 8), 12, facts(&[])),
            MoveRefused::NoRoom
        );
        // The same cell takes the whole stack.
        assert_eq!(
            run(&mut items, at(INV, 3), at(INV, 8), 0, facts(&[]))
                .expect("moves")
                .kind,
            MoveKind::Moved
        );
    }

    #[test]
    fn only_four_refusals_carry_a_notice() {
        let silent = [
            MoveRefused::InvalidSource,
            MoveRefused::Empty,
            MoveRefused::CountAboveStack { held: 1, asked: 2 },
            MoveRefused::Irremovable,
            MoveRefused::UnknownVnum(1),
            MoveRefused::InvalidDestination,
            MoveRefused::NotDragonSoulItem,
            MoveRefused::SocketsDiffer,
            MoveRefused::NoRoom,
            MoveRefused::NoItemIds,
            MoveRefused::IdsExhausted,
            MoveRefused::NotPorted(Unported::Equipment),
        ];
        for reason in silent {
            assert_eq!(reason.notice(), None, "{reason}");
        }
    }

    #[test]
    fn a_record_encodes_as_its_own_wire_record() {
        let sword = plain(7, 19);
        let set = sword.gc_item_set(at(INV, 40), 0);
        let mut expected = Vec::new();
        set.encode_into(&mut expected);
        let mut out = Vec::new();
        ItemRecord::Set(set).encode_into(&mut out);
        assert_eq!(out, expected);
        assert_eq!(out[0], 21);
        let update = stack(8, 27001, 5).gc_item_update();
        let mut expected = Vec::new();
        update.encode_into(&mut expected);
        let mut out = Vec::new();
        ItemRecord::Update(update).encode_into(&mut out);
        assert_eq!(out, expected);
        assert_ne!(out[0], 21);
    }
}
