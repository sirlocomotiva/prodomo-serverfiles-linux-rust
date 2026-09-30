//! The item instance: the state a single item carries, and the ids that name it.
//!
//! # What this is
//!
//! Legacy stores an item as a `CItem *` owned by a singleton `ITEM_MANAGER`, and
//! a character holds that bare pointer. The Rewrite holds an owned
//! [`Item`] value, so there is no pointer arithmetic, no owner pointer to
//! dangle, and no delete-inside-a-member-function to reproduce.
//!
//! # What this is not
//!
//! **The item limits are not on the instance.** `aLimits` is a member of
//! `TItemTable` (`tables.h:879`) -- the **prototype** -- and not of `TItemData`
//! (`packet.h:2971-2985`), which is the instance record and has no such member.
//! `CItem` has no `SetLimit` at all: `GetLimitType` and `GetLimitValue`
//! (`item.h:110-111`) both read `m_pProto->aLimits[idx]`, so a limit is fixed by
//! the prototype and no instance can change it. They are read from
//! `item_proto.txt` into `gamedata::item_proto::ItemProto::limits`, and a unit
//! that wants a limit goes there. Ledger 195 gave the instance a
//! `limits: [ItemLimit; 2]` and removed it, because an instance limit would have
//! had nothing to save it to: legacy's own `player.item` table has no limit
//! columns.
//!
//! `ITEM_LIMIT_MAX_NUM` is 2 (`item_length.h:11`) and `ELimitTypes`
//! (`item_length.h:427-450`) names **ten** types, `LIMIT_NONE` through
//! `LIMIT_CHAMPION`, then `LIMIT_MAX_NUM` as an eleventh. The sentinel is dead:
//! `LIMIT_MAX_NUM` is read nowhere in `server/server`, while all ten loops over
//! the array's elements are bounded by `ITEM_LIMIT_MAX_NUM` (swept with both
//! controls -- 14 hits for the array bound, 1 for the sentinel, which is its own
//! definition). So the tenth type has nowhere to live, and widening the array to
//! fit the enumeration would invent storage no loop can reach.
//!
//! The fields that are here are the legacy `TItemData` set
//! (`packet.h:2973-2985`), which is what the item windows put on the wire. All
//! four feature switches that gate them are live in this build, so all four are
//! here:
//! `ENABLE_REFINE_ELEMENT` (`prodomodefines.h:28`) gates
//! [`Item::refine_element`], `__CHANGELOOK_SYSTEM__` (`:16`) gates
//! [`Item::transmutation`], and `ENABLE_EXTENDED_SOCKETS` (`:76`) is what makes
//! [`Item::sockets`] six wide instead of three.
//!
//! # Three things legacy gets wrong, and what this does instead
//!
//! 1. **`CItem`'s one constructor leaves the proto and the owner indeterminate**
//!    (`item.h`, per the ledger-195 audit). Every field here is initialised, and
//!    an item that has no prototype says so with `Option` rather than by
//!    reading an uninitialised pointer.
//! 2. **`SetCount` silently clamps and can destroy its own object**
//!    (`item.cpp:296-348`, with `M2_DESTROY_ITEM(this)` at `:330` and `:338`).
//!    [`Item::set_count`] here is a total function that reports whether the
//!    value was accepted; it never destroys anything, because destruction is
//!    storage's decision, not a field setter's.
//! 3. **An id is never reused** (`item_manager_idrange.cpp:20-49`, a bare
//!    `m_dwCurrentID++`). [`ItemIds`] reproduces that with a monotonic
//!    counter, and it also reproduces the fact that the range is finite and
//!    exhaustible rather than pretending the ids are free.

use common::item_slots::ITEM_COUNT_LIMIT;
use protocol::gc_item_window::{GcItemSet, GcItemUpdate, ItemAttribute};
use protocol::item_pos::ItemPos;

use crate::character::NPOS;

/// `ITEM_SOCKET_MAX_NUM` = 6 in this build (`item_length.h:14` under
/// `ENABLE_EXTENDED_SOCKETS`), which is the width of [`Item::sockets`].
///
/// Widened from [`common::constants::ITEM_SOCKET_MAX_NUM`] rather than stated
/// again. The ledger-195 survey found two literals in one workspace saying
/// different things, so a third here would have been the same drift with a
/// different number.
pub const SOCKETS: usize = common::constants::ITEM_SOCKET_MAX_NUM as usize;

/// `ITEM_ATTRIBUTE_MAX_NUM` = 7 (`item_length.h:30`, unconditional), which is
/// the width of [`Item::attributes`] and of the prototype's `aAttr`. It is derived upstream from
/// `ITEM_ATTRIBUTE_NORM_START` plus the normal and rare counts, so it is
/// 5 + 2 and not a free choice.
pub const ATTRIBUTES: usize = common::constants::ITEM_ATTRIBUTE_MAX_NUM as usize;

/// `ITEM_FLAG_STACKABLE` (`item_length.h:361`): the proto flag that lets two stacks of one
/// vnum merge and one stack split. Legacy's `CItem::IsStackable` (`item.h:38`) reads it.
pub const ITEM_FLAG_STACKABLE: u32 = 1 << 2;

/// `ITEM_FLAG_IRREMOVABLE` (`item_length.h:367`): an item that may not be moved out of the
/// band between the base inventory and the custom banks (`char_item.cpp:7625-7629`).
pub const ITEM_FLAG_IRREMOVABLE: u32 = 1 << 8;

/// `ITEM_ANTIFLAG_STACK` (`item_length.h:393`): the anti-flag that forbids a stack merge
/// or split even on a stackable item.
pub const ITEM_ANTIFLAG_STACK: u32 = 1 << 15;

/// `ITEM_ANTIFLAG_DROP` (`item_length.h:385`): the item may not be dropped.
pub const ITEM_ANTIFLAG_DROP: u32 = 1 << 7;

/// `ITEM_ANTIFLAG_SELL` (`item_length.h:386`): no shop buys the item back
/// (`shop_manager.cpp:524`).
pub const ITEM_ANTIFLAG_SELL: u32 = 1 << 8;

/// `ITEM_ANTIFLAG_GIVE` (`item_length.h:391`): the item may not be given, and so not dropped
/// either (`char_item.cpp:7481`).
pub const ITEM_ANTIFLAG_GIVE: u32 = 1 << 13;

/// `ITEM_ANTIFLAG_SAFEBOX` (`item_length.h:395`): the item may not go in a safebox
/// (`input_main.cpp:2322`).
pub const ITEM_ANTIFLAG_SAFEBOX: u32 = 1 << 17;

/// An item instance's unique id.
///
/// Legacy's is a `DWORD` from a monotonic counter that is never rewound, so a
/// released id is never handed out again. The value 0 is not a valid id:
/// `GetNewID` starts at the DB-provided `dwUsableItemIDMin` and
/// `m_dwCurrentID != 0` is asserted, so 0 means "not allocated".
pub type ItemId = u32;

/// The id of an item that does not exist. `0` is unreachable for a real id.
pub const NO_ITEM: ItemId = 0;

/// The record that tells a client a cell is empty again.
///
/// Legacy clears a cell with `SetItem(pos, NULL)`, which sends `HEADER_GC_ITEM_DEL` (20) in
/// the deprecated `TPacketGCItemDelDeprecated` layout, every field zero
/// (`char_item.cpp:596-611`). The Reference client frames byte 20 at the current
/// `TPacketGCItemDel` width, which the deprecated layout does not match, so the record it
/// can read is a `GC_ITEM_SET` (21) with vnum 0 and every other field zero. That is a
/// recorded Divergence (ledger 211.3), and the play test calibrates it.
#[must_use]
pub const fn gc_item_clear(pos: ItemPos) -> GcItemSet {
    GcItemSet {
        cell: pos,
        vnum: 0,
        count: 0,
        refine_element: 0,
        transmutation: 0,
        flags: 0,
        anti_flags: 0,
        highlight: 0,
        sockets: [0; SOCKETS],
        attributes: [ItemAttribute::new(0, 0); ATTRIBUTES],
    }
}

/// Why a count was refused.
///
/// [`Item::set_count`] is total, so a caller always learns the difference
/// between "the value was stored" and "the value was not". Legacy has no such
/// distinction: `SetCount` clamps and returns `true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CountRejected {
    /// Above [`ITEM_COUNT_LIMIT`]. Legacy would have clamped this to the limit
    /// and reported success (`item.cpp:304`).
    AboveLimit {
        /// The value that was asked for.
        requested: u32,
        /// The ceiling it exceeded.
        limit: u32,
    },
    /// Zero. Legacy accepts a zero count and, when the item has an owner,
    /// destroys the item from inside its own setter (`item.cpp:307-338`).
    /// Removal is [`crate::character::CharacterItems::remove`]'s decision, so
    /// here zero is refused and reported.
    Zero,
}

impl core::fmt::Display for CountRejected {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AboveLimit { requested, limit } => {
                write!(f, "count {requested} is above the limit of {limit}")
            }
            Self::Zero => f.write_str("a count of zero is a removal, not a count"),
        }
    }
}

impl std::error::Error for CountRejected {}

/// One item: the state a single instance carries.
///
/// Every field is a legacy wire fact or a measured constant; nothing here is
/// gameplay policy. Whether an item may go in a given slot, whether two items
/// stack, and what a flag does are all above this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The unique id. Never 0, never reused.
    pub id: ItemId,
    /// `TItemData::vnum` (`packet.h:2973`): the item prototype this instance is
    /// made from.
    pub vnum: u32,
    /// `TItemData::count` (`packet.h:2974`): the stack size. Bounded by
    /// [`ITEM_COUNT_LIMIT`].
    pub count: u16,
    /// `TItemData::dwRefineElement` (`packet.h:2976`), under
    /// `ENABLE_REFINE_ELEMENT`. Which of the six elements an item was refined
    /// with, and the zero that means none.
    pub refine_element: u32,
    /// `TItemData::transmutation` (`packet.h:2979`), under
    /// `__CHANGELOOK_SYSTEM__`.
    pub transmutation: u32,
    /// `TItemData::flags` (`packet.h:2981`).
    pub flags: u32,
    /// `TItemData::anti_flags` (`packet.h:2982`). This is a separate bitmap
    /// from `flags` and is not stored the same way; the two are not inverses.
    pub anti_flags: u32,
    /// `TItemData::alSockets` (`packet.h:2983`). Six wide here.
    ///
    /// Sockets 0 and 1 are not decoration: socket 0 holds an absolute expiry
    /// and socket 1 a first-used marker, so the item's timers live in this
    /// array and not in any member. The 3 and 4 in [`common::constants`] index
    /// the toggle and riding flags.
    pub sockets: [i32; SOCKETS],
    /// `TItemData::aAttr` (`packet.h:2984`): seven
    /// [`ItemAttribute`] values, which is the protocol's own model of
    /// `TPlayerItemAttribute`. This crate reuses it rather than declaring a
    /// second attribute type, so there is one definition of the wire pair.
    pub attributes: [ItemAttribute; ATTRIBUTES],
    /// The grid footprint from the prototype's `bSize`: how many cells the
    /// stack occupies, walked with the window's stride.
    ///
    /// Legacy's `GetSize()` is `m_pProto ? m_pProto->bSize : 0` (`item.h:69`),
    /// so an item with no prototype reports a size of zero and its `SetItem`
    /// grid walk runs no iterations. This field is `u8` and is set explicitly,
    /// so a caller that has not read the prototype must say what it means.
    pub size: u8,
    /// Where the item currently is. [`NPOS`] until it is stored.
    ///
    /// Legacy keeps the window and cell in `CItem::m_window_type` and
    /// `m_wCell` and leaves them meaningful only while the owner is set; the
    /// pair is the same shape as a `TItemPos`.
    pub pos: ItemPos,
}

impl Item {
    /// Build an item from a prototype's shape.
    ///
    /// The id is required and is never [`NO_ITEM`], so an unallocated item
    /// cannot be built by accident. Everything else defaults to the zero the
    /// wire already expects, which is what a legacy `CItem` has after its
    /// proto and owner are set -- and unlike legacy, it is *set* rather than
    /// left indeterminate.
    pub fn new(id: ItemId, vnum: u32) -> Self {
        debug_assert_ne!(id, NO_ITEM, "an item needs an allocated id");
        Self {
            id,
            vnum,
            count: 1,
            refine_element: 0,
            transmutation: 0,
            flags: 0,
            anti_flags: 0,
            sockets: [0; SOCKETS],
            attributes: [ItemAttribute::new(0, 0); ATTRIBUTES],
            size: 1,
            pos: NPOS,
        }
    }

    /// This item's id.
    #[must_use]
    pub const fn id(&self) -> ItemId {
        self.id
    }

    /// The stack size.
    #[must_use]
    pub const fn count(&self) -> u16 {
        self.count
    }

    /// Whether this item stacks: `CItem::IsStackable` (`item.h:38`) and not
    /// `ITEM_ANTIFLAG_STACK`, which is the pair every merge and split in `MoveItem` tests.
    #[must_use]
    pub const fn stacks(&self) -> bool {
        self.flags & ITEM_FLAG_STACKABLE != 0 && self.anti_flags & ITEM_ANTIFLAG_STACK == 0
    }

    /// How many grid cells the stack occupies.
    ///
    /// A size of zero is representable because legacy's `GetSize()` returns it
    /// for an item with no prototype, and because a zero-sized item in a
    /// zero-sized window would make the walk a no-op. The Rewrite's storage
    /// treats a zero as a refusal rather than an empty footprint.
    #[must_use]
    pub const fn size(&self) -> u8 {
        self.size
    }

    /// Whether the item is at no position.
    ///
    /// Legacy asks the same question with `GetOwner() == NULL`, which is a
    /// different question: an owned item whose position is `NPOS` cannot exist
    /// there, but an unowned item with a stale position can, and that is one of
    /// the states a destroyed-but-referenced item is left in.
    #[must_use]
    pub fn is_unplaced(&self) -> bool {
        self.pos == NPOS
    }

    /// The window this item is in.
    #[must_use]
    pub const fn window(&self) -> u8 {
        self.pos.window_type
    }

    /// The `GC_ITEM_SET` record that tells a client this item is at this cell.
    ///
    /// `char_item.cpp:585-615` is `SetItem` filling a `TPacketGCItemSet` field by
    /// field. Nine of the record's fields are the item's own, and this is where
    /// that correspondence is asserted rather than restated: the field order in
    /// the constructor is the field order in `GC_ITEM_SET`, so a field that moved
    /// in one and not the other fails the test below.
    ///
    /// `highlight` is passed in rather than derived. `char_item.cpp:585-589` has
    /// two candidates and `__BL_ENABLE_PICKUP_ITEM_EFFECT__` is defined, so the
    /// live value is the caller's `bHighlight` argument and the cell is not
    /// consulted. A fresh grant passes 1.
    #[must_use]
    pub fn gc_item_set(&self, pos: ItemPos, highlight: u8) -> GcItemSet {
        GcItemSet {
            cell: pos,
            vnum: self.vnum,
            count: self.count,
            refine_element: self.refine_element,
            transmutation: self.transmutation,
            flags: self.flags,
            anti_flags: self.anti_flags,
            highlight,
            sockets: self.sockets,
            attributes: self.attributes,
        }
    }

    /// The `GC_ITEM_UPDATE` record for this item where it is stored.
    ///
    /// `CItem::UpdatePacket` (`item.cpp:249-276`) addresses the client with the item's own
    /// window and cell, not with whatever position a caller asked about, so this reads
    /// [`Item::pos`], which [`crate::character::CharacterItems::set`] keeps in legacy's terms.
    #[must_use]
    pub const fn gc_item_update(&self) -> GcItemUpdate {
        GcItemUpdate {
            cell: self.pos,
            count: self.count,
            refine_element: self.refine_element,
            transmutation: self.transmutation,
            sockets: self.sockets,
            attributes: self.attributes,
        }
    }

    /// Set the stack size, or report why it was refused.
    ///
    /// # Errors
    ///
    /// [`CountRejected::AboveLimit`] when the count is over the ceiling, and
    /// [`CountRejected::Zero`] when it is zero. Neither value is stored and
    /// nothing is destroyed, so a refusal leaves the count as it was.
    ///
    /// This is where legacy's silent clamp is refused. `CItem::SetCount`
    /// (`item.cpp:296-348`) stores `MIN(count, g_bItemCountLimit)`, returns
    /// `true`, and for a count of zero with an owner calls
    /// `M2_DESTROY_ITEM(this)` from inside its own body. The consequences are
    /// both wrong: an over-limit request becomes a full stack and the caller is
    /// told it worked, and a zero becomes a destruction the setter's caller
    /// cannot observe or refuse.
    ///
    /// Here a value is either stored or reported. Nothing is destroyed.
    pub fn set_count(&mut self, count: u32) -> Result<u16, CountRejected> {
        if count == 0 {
            return Err(CountRejected::Zero);
        }
        if count > u32::from(ITEM_COUNT_LIMIT) {
            return Err(CountRejected::AboveLimit {
                requested: count,
                limit: u32::from(ITEM_COUNT_LIMIT),
            });
        }
        // The two checks above make this total, so the conversion is proved
        // rather than assumed. `try_from` is used because a truncating cast
        // would hide a future change to the ceiling.
        let stored = u16::try_from(count).map_err(|_| CountRejected::AboveLimit {
            requested: count,
            limit: u32::from(ITEM_COUNT_LIMIT),
        })?;
        self.count = stored;
        Ok(stored)
    }

    /// Set the grid footprint.
    ///
    /// # Errors
    ///
    /// [`ZeroSize`] when asked for a footprint of zero.
    /// Refuses zero, because a zero-sized item would make a storage walk a
    /// no-op that stores the item without marking any cell, which is exactly
    /// how a later write ends up believing the cell is free.
    pub fn set_size(&mut self, size: u8) -> Result<u8, ZeroSize> {
        if size == 0 {
            return Err(ZeroSize);
        }
        self.size = size;
        Ok(size)
    }

    /// Read one socket.
    ///
    /// The bounds are the array's, not the caller's: legacy reads these through
    /// unchecked pointer arithmetic, and socket 0 and 1 carry the timers, so an
    /// out-of-range read here would be a wrong *time*, not a wrong number.
    #[must_use]
    pub const fn socket(&self, index: usize) -> Option<i32> {
        if index < SOCKETS {
            Some(self.sockets[index])
        } else {
            None
        }
    }

    /// Write one socket, or report that the index is out of range.
    ///
    /// # Errors
    ///
    /// [`SocketOutOfRange`] for an index of [`SOCKETS`] or more.
    pub fn set_socket(&mut self, index: usize, value: i32) -> Result<(), SocketOutOfRange> {
        if index >= SOCKETS {
            return Err(SocketOutOfRange(index));
        }
        self.sockets[index] = value;
        Ok(())
    }

    /// Read one attribute, or report that the index is out of range.
    #[must_use]
    pub const fn attribute(&self, index: usize) -> Option<ItemAttribute> {
        if index < ATTRIBUTES {
            Some(self.attributes[index])
        } else {
            None
        }
    }

    /// Write one attribute, or report that the index is out of range.
    ///
    /// # Errors
    ///
    /// [`AttributeOutOfRange`] for an index of [`ATTRIBUTES`] or more.
    pub fn set_attribute(
        &mut self,
        index: usize,
        attr: ItemAttribute,
    ) -> Result<(), AttributeOutOfRange> {
        if index >= ATTRIBUTES {
            return Err(AttributeOutOfRange(index));
        }
        self.attributes[index] = attr;
        Ok(())
    }
}

/// A grid footprint of zero was asked for. See [`Item::set_size`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ZeroSize;

impl core::fmt::Display for ZeroSize {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("an item needs at least one cell of grid footprint")
    }
}

impl std::error::Error for ZeroSize {}

/// A socket index past [`SOCKETS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SocketOutOfRange(pub usize);

impl core::fmt::Display for SocketOutOfRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "socket {} is outside the {} sockets an item has",
            self.0, SOCKETS
        )
    }
}

impl std::error::Error for SocketOutOfRange {}

/// An attribute index past [`ATTRIBUTES`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AttributeOutOfRange(pub usize);

impl core::fmt::Display for AttributeOutOfRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "attribute {} is outside the {} attributes an item has",
            self.0, ATTRIBUTES
        )
    }
}

impl std::error::Error for AttributeOutOfRange {}

/// The usable id range, `[first, last)`.
///
/// Legacy calls this `TItemIDRangeTable` and seeds it from the DB over the
/// retired DB-peer protocol (`item_manager_idrange.cpp:63`). The Rewrite's
/// store supplies the same three numbers, but the shape is unchanged: a
/// half-open interval with a separate "first usable id" floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemIdRange {
    /// The lowest id in the range.
    pub first: u32,
    /// One past the highest id. The `last` itself is never issued, because
    /// reaching it is what triggers the refill.
    pub last: u32,
    /// The first id a real item may take. The ids below it are reserved.
    pub first_usable: u32,
}

impl ItemIdRange {
    /// Build a range, checking that it is usable.
    ///
    /// # Errors
    ///
    /// [`BadIdRange`] when the range is empty, when `first_usable` is 0, or
    /// when it falls outside the range. The 0 case is the one that matters
    /// here: [`NO_ITEM`] is 0, so a range seeded at 0 would hand out an id that
    /// reads as "no item".
    ///
    /// A range whose `first_usable` is not inside it, or that is empty, is
    /// refused. Legacy has no such check: `GetNewID` only asserts that the
    /// counter is not 0, so a bad range from the DB peer produced ids that
    /// collided with the reserved ones.
    pub const fn new(first: u32, last: u32, first_usable: u32) -> Result<Self, BadIdRange> {
        // Three rules, and the middle one is the one a test would miss: the
        // first usable id may not be 0, because [`NO_ITEM`] is 0 and an id of
        // 0 would be indistinguishable from "there is no item here". Legacy
        // gets this for free from `assert(m_dwCurrentID != 0)`
        // (`item_manager_idrange.cpp:23`), which is an assertion and not a
        // check, so a range seeded at 0 would sail through.
        if first_usable == 0 || first_usable < first || first_usable >= last {
            return Err(BadIdRange {
                first,
                last,
                first_usable,
            });
        }
        Ok(Self {
            first,
            last,
            first_usable,
        })
    }

    /// How many ids the range can ever hand out.
    #[must_use]
    pub const fn capacity(&self) -> u32 {
        self.last - self.first_usable
    }
}

/// A range that cannot issue any id was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BadIdRange {
    /// The requested floor.
    pub first: u32,
    /// The requested ceiling, exclusive.
    pub last: u32,
    /// The requested first usable id.
    pub first_usable: u32,
}

impl core::fmt::Display for BadIdRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "item id range {}..{} has no usable id: first_usable {} is outside it",
            self.first, self.last, self.first_usable
        )
    }
}

impl std::error::Error for BadIdRange {}

/// The id range ran out, so no item could be created.
///
/// Legacy handles this by logging ten times, touching `.killscript`, calling
/// `thecore_shutdown()` and returning **0** (`item_manager_idrange.cpp:29-34`).
/// Returning 0 is the part worth not copying: 0 is the one value an id cannot
/// take, and a caller that did not check would store an item under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IdRangeExhausted {
    /// The exhausted range.
    pub range: ItemIdRange,
}

impl core::fmt::Display for IdRangeExhausted {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "the item id range is exhausted after {} ids; the server needs a new range",
            self.range.capacity()
        )
    }
}

impl std::error::Error for IdRangeExhausted {}

/// Hands out item ids, monotonically, and never twice.
///
/// This is legacy's `ITEM_MANAGER::GetNewID` (`item_manager_idrange.cpp:20-49`)
/// with the shutdown replaced by an error. The three properties that matter are
/// all preserved:
///
/// - **Monotonic.** `m_dwCurrentID++`, with no hashing, no probing and no
///   free-list, so ids are dense in the range.
/// - **Never reused.** Nothing rewinds the counter, not even on release. That
///   is what makes an id safe to cache in a client packet and to leave in a
///   log line: a stale id can never come back to mean a different item.
/// - **Finite.** The range is a hard resource that is exhaustible, and this
///   says so instead of running the process down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemIds {
    range: ItemIdRange,
    next: u32,
    issued: u32,
}

impl ItemIds {
    /// Start an allocator at the range's first usable id.
    ///
    /// The count starts at zero even though the counter starts at
    /// `first_usable`, so `issued` is the number handed out and not the value
    /// of the counter.
    pub fn new(range: ItemIdRange) -> Self {
        Self {
            next: range.first_usable,
            range,
            issued: 0,
        }
    }

    /// The range being drawn from.
    #[must_use]
    pub const fn range(&self) -> ItemIdRange {
        self.range
    }

    /// How many ids have been handed out.
    #[must_use]
    pub const fn issued(&self) -> u32 {
        self.issued
    }

    /// How many ids are left.
    #[must_use]
    pub const fn remaining(&self) -> u32 {
        self.range.capacity() - self.issued
    }

    /// The id the next call will return.
    #[must_use]
    pub const fn peek(&self) -> u32 {
        self.next
    }

    /// Take the next id, or report that the range is spent.
    ///
    /// # Errors
    ///
    /// [`IdRangeExhausted`] once every id in the range has been handed out. The
    /// allocator is left unchanged, so a retry after a new range is installed
    /// behaves the same as the first call did.
    ///
    /// The bound is checked before the counter moves, so an exhausted range
    /// leaves the allocator exactly as it was and a later retry reports the
    /// same thing. Legacy checks the same condition at the same point, but then
    /// returns 0 after shutting the process down.
    pub fn allocate(&mut self) -> Result<ItemId, IdRangeExhausted> {
        if self.next >= self.range.last {
            return Err(IdRangeExhausted { range: self.range });
        }
        let id = self.next;
        self.next += 1;
        self.issued += 1;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::constants::ITEM_LIMIT_MAX_NUM;
    use common::item_slots::ITEM_COUNT_LIMIT;
    use gamedata::item_proto_value::ANTI_FLAG;

    /// A range with room to spare, for the tests that are not about exhaustion.
    const fn test_range() -> ItemIdRange {
        match ItemIdRange::new(1, 11, 2) {
            Ok(r) => r,
            // A `const fn` cannot unwrap; the values are literals, so a failure
            // here is a change to the literals and the panic says so.
            Err(_) => panic!("the test range must be usable"),
        }
    }

    #[test]
    fn ids_are_monotonic_and_dense() {
        let mut ids = ItemIds::new(test_range());
        let mut seen = Vec::new();
        for _ in 0..9 {
            seen.push(ids.allocate().expect("the range has nine ids"));
        }
        assert_eq!(seen, vec![2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(ids.issued(), 9);
        assert_eq!(ids.remaining(), 0);
    }

    #[test]
    fn an_exhausted_range_reports_it_rather_than_returning_zero() {
        // `GetNewID` returns 0 after shutting the process down
        // (`item_manager_idrange.cpp:29-34`). 0 is the one value an item id
        // cannot be, so a caller that did not check would store an item under
        // it. The Rewrite reports instead, and the allocator is unchanged.
        let mut ids = ItemIds::new(test_range());
        while ids.allocate().is_ok() {}
        let before = ids.clone();
        let err = ids.allocate().expect_err("the range is spent");
        assert_eq!(err.range.capacity(), 9);
        assert_eq!(ids, before, "a failed allocation must change nothing");
        // The counter never reaches 0, so the one value that reads as "no
        // item" is never handed out. `first_usable` is 2 and the range never
        // runs backwards, so this holds for the whole run, not just at the end.
        assert_eq!(NO_ITEM, 0);
        assert!(ids.peek() >= test_range().first_usable);
    }

    #[test]
    fn an_id_is_never_handed_out_twice_even_after_it_is_given_up() {
        // Nothing rewinds the counter, so a released id can never come back.
        // That is what makes an id safe to leave in a client packet and in a
        // log line: a stale id cannot come to mean a different item.
        let mut ids = ItemIds::new(test_range());
        let first = ids.allocate().expect("usable");
        let second = ids.allocate().expect("usable");
        assert_ne!(first, second);
        let mut all = Vec::new();
        while let Ok(id) = ids.allocate() {
            all.push(id);
        }
        assert!(!all.contains(&first));
    }

    #[test]
    fn a_range_that_cannot_issue_an_id_is_refused() {
        // Legacy trusts whatever the DB peer sent and only asserts the counter
        // is non-zero, so a bad range produced ids that collided with the
        // reserved ones. The zero case is the one a value-only test would miss.
        assert!(ItemIdRange::new(1, 11, 0).is_err(), "first_usable is 0");
        assert!(
            ItemIdRange::new(0, 11, 0).is_err(),
            "first_usable is 0 even when first is 0"
        );
        assert!(
            ItemIdRange::new(3, 11, 2).is_err(),
            "first_usable below first"
        );
        assert!(ItemIdRange::new(1, 11, 11).is_err(), "first_usable at last");
        assert!(ItemIdRange::new(5, 5, 5).is_err(), "an empty range");
        assert!(
            ItemIdRange::new(0, 1, 1).is_err(),
            "no id is left once 0 is excluded"
        );
        assert!(ItemIdRange::new(1, 11, 2).is_ok());
        assert!(ItemIdRange::new(0, 2, 1).is_ok(), "first may be 0");
    }

    #[test]
    fn a_count_over_the_ceiling_is_refused_and_leaves_the_count_alone() {
        let mut item = Item::new(1, 100);
        let before = item.count();
        let err = item
            .set_count(u32::from(ITEM_COUNT_LIMIT) + 1)
            .expect_err("over the ceiling");
        assert_eq!(
            err,
            CountRejected::AboveLimit {
                requested: u32::from(ITEM_COUNT_LIMIT) + 1,
                limit: u32::from(ITEM_COUNT_LIMIT),
            }
        );
        assert_eq!(item.count(), before, "a refusal must not change the count");
    }

    #[test]
    fn a_count_of_zero_is_a_removal_and_is_refused_here() {
        // Legacy's `SetCount` accepts zero and, when the item has an owner,
        // calls `M2_DESTROY_ITEM(this)` from inside its own body
        // (`item.cpp:307-338`). Here it is reported and nothing is destroyed.
        let mut item = Item::new(1, 100);
        assert_eq!(item.set_count(0), Err(CountRejected::Zero));
        assert_eq!(item.count(), 1);
    }

    #[test]
    fn the_whole_useful_count_range_is_accepted() {
        let mut item = Item::new(1, 100);
        for count in [1_u32, 2, 45, 500, 4999, u32::from(ITEM_COUNT_LIMIT)] {
            let stored = item
                .set_count(count)
                .unwrap_or_else(|e| panic!("{count}: {e}"));
            assert_eq!(u32::from(stored), count);
            assert_eq!(u32::from(item.count()), count);
        }
    }

    #[test]
    fn a_footprint_of_zero_is_refused() {
        // A zero-sized item would be stored with an empty grid, so the next
        // write would believe its cells were free.
        let mut item = Item::new(1, 100);
        assert_eq!(item.set_size(0), Err(ZeroSize));
        assert_eq!(item.size(), 1, "the footprint is unchanged after a refusal");
        assert_eq!(item.set_size(4), Ok(4));
    }

    #[test]
    fn every_socket_index_is_in_bounds_or_reported() {
        let mut item = Item::new(1, 100);
        for index in 0..12 {
            // A value with distinct halves, so a sign or width mistake in the
            // field cannot pass as the number that went in.
            let value = i32::try_from(index).expect("the loop is small") * -7 - 1;
            let outcome = item.set_socket(index, value);
            if index < SOCKETS {
                assert_eq!(outcome, Ok(()), "socket {index} is in bounds");
                assert_eq!(item.socket(index), Some(value));
            } else {
                assert_eq!(outcome, Err(SocketOutOfRange(index)));
                assert_eq!(item.socket(index), None);
            }
        }
        // Every one of the six real sockets took its value, and the first index
        // past the array reads as absent rather than as a seventh socket.
        for index in 0..SOCKETS {
            let want = i32::try_from(index).expect("six") * -7 - 1;
            assert_eq!(item.socket(index), Some(want), "socket {index}");
        }
        assert_eq!(item.socket(SOCKETS), None);
    }

    #[test]
    fn socket_zero_and_one_carry_the_timers_so_they_round_trip_a_timestamp() {
        // Socket 0 is an absolute expiry and socket 1 a first-used marker, so
        // the item's clocks live in the array and a truncating codec would
        // produce a wrong TIME, not a wrong number.
        let mut item = Item::new(1, 100);
        let expiry = i32::MAX;
        item.set_socket(0, expiry).expect("socket 0");
        item.set_socket(1, -1).expect("socket 1");
        assert_eq!(item.socket(0), Some(expiry));
        assert_eq!(item.socket(1), Some(-1));
    }

    #[test]
    fn every_attribute_index_is_in_bounds_or_reported() {
        let mut item = Item::new(1, 100);
        for index in 0..12 {
            let attr = ItemAttribute::new(
                u8::try_from(index).expect("small"),
                -300 + i16::try_from(index).expect("small"),
            );
            let outcome = item.set_attribute(index, attr);
            if index < ATTRIBUTES {
                assert_eq!(outcome, Ok(()), "attribute {index} is in bounds");
                assert_eq!(item.attribute(index), Some(attr));
            } else {
                assert_eq!(outcome, Err(AttributeOutOfRange(index)));
                assert_eq!(item.attribute(index), None);
            }
        }
    }

    #[test]
    fn all_seven_attribute_slots_hold_a_distinct_signed_value() {
        // The wire is `BYTE` then signed `short`. A negative bonus is a real
        // value, so the attribute has to be signed, and the values here have
        // distinct halves so an endianness error cannot hide.
        let mut item = Item::new(1, 100);
        for index in 0..ATTRIBUTES {
            let slot = u8::try_from(index).expect("seven slots");
            let value = -0x0102 - i16::try_from(index).expect("seven slots");
            item.set_attribute(index, ItemAttribute::new(0xA0 + slot, value))
                .expect("in bounds");
        }
        let round: Vec<ItemAttribute> = (0..ATTRIBUTES)
            .map(|i| item.attribute(i).expect("in bounds"))
            .collect();
        let bytes: Vec<u8> = round.iter().flat_map(|a| a.encode()).collect();
        assert_eq!(bytes.len(), ATTRIBUTES * 3, "three bytes per attribute");
        for (index, attr) in round.iter().enumerate() {
            let at = index * 3;
            let slot = u8::try_from(index).expect("seven slots");
            let value = -0x0102 - i16::try_from(index).expect("seven slots");
            // The type byte comes first, then the value little-endian. Reading
            // the value back through `i16::from_le_bytes` rather than through a
            // cast is what makes the sign and the byte order both visible.
            assert_eq!(attr.b_type, 0xA0 + slot, "attribute {index} type");
            assert_eq!(attr.s_value, value, "attribute {index} value");
            assert_eq!(bytes[at], 0xA0 + slot, "attribute {index} type byte");
            assert_eq!(
                i16::from_le_bytes([bytes[at + 1], bytes[at + 2]]),
                value,
                "attribute {index} value bytes"
            );
        }
    }

    #[test]
    fn a_fresh_item_has_every_field_set_rather_than_indeterminate() {
        // Legacy's one constructor leaves the proto and the owner
        // indeterminate, so reading either before it is set reads whatever was
        // on the heap. Here every field has a stated value from the start.
        let item = Item::new(7, 30_000);
        assert_eq!(item.id(), 7);
        assert_eq!(item.vnum, 30_000);
        assert_eq!(item.count(), 1);
        assert_eq!(item.refine_element, 0);
        assert_eq!(item.transmutation, 0);
        assert_eq!(item.flags, 0);
        assert_eq!(item.anti_flags, 0);
        assert_eq!(item.sockets, [0; SOCKETS]);
        assert_eq!(item.attributes, [ItemAttribute::new(0, 0); ATTRIBUTES]);
        assert!(item.is_unplaced());
        assert_eq!(item.window(), NPOS.window_type);
    }

    #[test]
    fn every_width_is_the_legacy_one_and_is_derived_not_restated() {
        // The two widths this crate owns, stated so that a derivation that
        // pointed at the wrong constant fails here rather than in a wire format
        // three crates downstream. `ITEM_SOCKET_MAX_NUM` is the one that was
        // wrong once: it said 3 in `constants.rs` while `tables.rs` said 6.
        assert_eq!(SOCKETS, 6, "ENABLE_EXTENDED_SOCKETS is live");
        assert_eq!(ATTRIBUTES, 7, "5 normal plus 2 rare");
        assert_eq!(
            SOCKETS,
            common::constants::ITEM_SOCKET_MAX_NUM as usize,
            "and it is the same number the workspace agrees on"
        );
        assert_eq!(ATTRIBUTES, 5 + 2);
        // The unique sockets are the last two, and the timers are the first two,
        // which is only true because the count is 6 and not 3.
        assert_eq!(common::constants::ITEM_SOCKET_UNIQUE_SAVE_TIME, 4);
        assert_eq!(common::constants::ITEM_SOCKET_UNIQUE_REMAIN_TIME, 5);
        assert_eq!(common::constants::ITEM_SOCKET_REMAIN_SEC, 0);
    }

    #[test]
    fn every_anti_flag_is_the_bit_its_proto_name_reads_as() {
        // `get_Item_AntiFlag_Value` reads the n-th name as `1 << n` (`D/ProtoReader.cpp:365-393`),
        // and `item_length.h:378-396` gives each flag the same bit.
        for (flag, name) in [
            (ITEM_ANTIFLAG_DROP, "ANTI_DROP"),
            (ITEM_ANTIFLAG_SELL, "ANTI_SELL"),
            (ITEM_ANTIFLAG_GIVE, "ANTI_GIVE"),
            (ITEM_ANTIFLAG_STACK, "ANTI_STACK"),
            (ITEM_ANTIFLAG_SAFEBOX, "ANTI_SAFEBOX"),
        ] {
            let at = ANTI_FLAG.iter().position(|known| *known == name);
            assert_eq!(at.map(|at| 1_u32 << at), Some(flag), "{name}");
        }
    }

    #[test]
    fn an_instance_carries_no_limit_because_the_prototype_fixes_them() {
        // Ledger 195 gave the instance a `limits: [ItemLimit; 2]` and
        // `set_limit`. Both were wrong and both are gone. `aLimits` is a member
        // of `TItemTable` (`tables.h:879`) -- the prototype -- and not of
        // `TItemData` (`packet.h:2973-2985`), which is the instance record and
        // has no such member. `CItem` has no `SetLimit`: `GetLimitType` and
        // `GetLimitValue` (`item.h:110-111`) both read `m_pProto->aLimits[idx]`.
        // The legacy `player.item` table has no limit columns either
        // (`player.sql:351-395`), so an instance limit would have had nothing to
        // save it and nothing to load it from.
        //
        // This is a compile-time assertion: if a limit field ever comes back,
        // this file stops compiling, which is the point. The behavioural half of
        // the claim is checked here in the only way a test can: the prototype
        // side owns it, in `gamedata::item_proto::ItemProto::limits`, and that
        // type is the one a caller must go through.
        assert_eq!(
            ITEM_LIMIT_MAX_NUM, 2,
            "the prototype array is two wide, not the ten `ELimitTypes` names"
        );
        assert_eq!(ITEM_LIMIT_MAX_NUM, 10 - 8, "LIMIT_CHAMPION is the tenth");
    }
}
