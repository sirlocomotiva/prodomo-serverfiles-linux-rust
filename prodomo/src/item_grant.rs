//! The item grant as a transport-free reducer.
//!
//! This sits between an Operator (or a GM chat command) and the world. It owns no
//! socket, no store handle and no thread: it takes the pieces it needs by
//! reference, applies the grant, and hands back a record to send and a row to
//! write. The caller decides what to do with either, which is the same contract
//! [`crate::lifecycle`] and [`crate::account_player`] use.
//!
//! # The order is the contract
//!
//! Legacy's `ACMD(do_item)` (`game/cmd_gm.cpp:467-519`) checks in this order, and
//! every step is a place a mistake becomes observable:
//!
//! 1. `m_pPC->IsEmptyItemGrid` / `GetEmptyInventoryEx` -- the free-cell search
//!    ([`world::character::grant`]).
//! 2. `pItem = ITEM_MANAGER::instance().CreateItem(vnum, 0)` with
//!    `bTryMagic = true` (`:499`).
//! 3. `pItem->SetCount(count)` (`:501`).
//! 4. `pPC->AddToCharacter(pItem)` (`:503`) -- the **second** placement attempt, on
//!    the item that now knows its own size.
//! 5. `d->GetCharacter()->Save()` on the *character* (`:507`).
//! 6. `M2_DESTROY_ITEM(pItem)` on failure (`:511`), then
//!    `ChatPacket(CHAT_TYPE_INFO, "Not enough inventory space.")` (`:513`).
//!
//! # Four decisions that are not transcriptions
//!
//! * **No magic.** Step 2 passes `bTryMagic = true`, so a legacy GM grant can hand
//!   out a randomly enchanted or socketed item. That needs the proto percentage
//!   columns and an RNG, and an Operator command that silently rolls item stats is
//!   worse than one that does not. A grant is a plain item. Divergence 199.2.
//! * **A locked page is never offered** (ledger 198.3 Defect). The search is bounded
//!   by the character's usable cell count, not by the array length.
//! * **Written through immediately.** Legacy step 5 saves the *character* and lets
//!   the DB server's own cache absorb the write, so an item is not durable for up to
//!   five minutes and is lost outright on a crash in that window. The Rewrite
//!   commits the row in the same transaction as the placement (ADR-0003).
//! * **The refusal is logged and answered.** Legacy logs in `PickupItem` and not in
//!   `do_item`; the Rewrite logs in both, because a silent refusal is how a lost
//!   item goes unnoticed.

#![warn(missing_docs)]

use gamedata::item_custom_category::{is_custom_category, CATEGORY_NUM};
use gamedata::item_proto::ItemProtos;
use world::character::{
    grant as place, CharacterManager, GrantRefused as PlacementRefused, Rejected,
};
use world::item::{Item, ItemIds};

use db::items::ItemRow;

/// The message legacy's `do_item` refusal sends, byte for byte.
///
/// `cmd_gm.cpp:513` passes this literal to `ChatPacket`, so a client that is told
/// "not enough space" sees exactly this text. The Rewrite adds a log line
/// alongside it; it does not change the text.
pub const NO_ROOM_MESSAGE: &str = "Not enough inventory space.";

/// What a grant was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRequest {
    /// The target character's name, matched case-insensitively because names are
    /// unique regardless of case.
    pub target: String,
    /// The item prototype vnum.
    pub vnum: u32,
    /// The stack count. `None` means 1.
    ///
    /// A count is clamped rather than refused, because `SetCount` in legacy stores
    /// `MIN(count, g_bItemCountLimit)` (`item.cpp:296-348`) and a GM who types a
    /// number expects a stack, not an error. Ledger 195 removed `Item`'s silent
    /// clamp from the *setter*; this is the caller deciding on purpose, which is the
    /// difference between a clamp and a surprise.
    ///
    /// The ceiling is the global `ITEM_MAX_COUNT` (`common/item_length.h:19`), 5000.
    /// There is no per-prototype maximum in this fork; see 200.4.
    pub count: Option<u16>,
}

/// Why a grant did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantRefusal {
    /// No character has that name.
    NoSuchCharacter {
        /// The name that was asked for.
        name: String,
    },
    /// The vnum is not in `item_proto.txt`.
    ///
    /// Legacy would create the item and it would have no prototype, so
    /// `GetSize()` is 0 and `AddToCharacter` refuses it (`item.cpp:69`). The Rewrite
    /// refuses at the boundary and names the vnum, because a silent failure here is
    /// indistinguishable from a full inventory.
    UnknownVnum {
        /// The vnum that was asked for.
        vnum: u32,
    },
    /// Nothing was free. The caller sends [`NO_ROOM_MESSAGE`].
    NoRoom {
        /// The item's footprint in cells.
        size: u8,
    },
    /// Every item id in the configured range has been handed out.
    ///
    /// Legacy's `dwMaxItemID` counter wraps instead (`item.h`), which re-issues ids
    /// that live items still hold. The Rewrite's [`ItemIds`] refuses, and this is
    /// where that surfaces.
    IdsExhausted,
    /// The world has no allocator yet.
    ///
    /// Refused before anything is placed, so no id is burned and nothing changes.
    /// A caller that retries after [`GameCommand::InstallItemIdRange`] gets a fresh
    /// answer. The ready gate stays closed until the install lands, so a client
    /// cannot reach a world in this state; the variant exists because a grant can be
    /// asked for from a path that does not go through the gate.
    ///
    /// [`GameCommand::InstallItemIdRange`]: crate::game_loop_messages::GameCommand::InstallItemIdRange
    NoAllocator,
    /// The allocator handed back an id this character already owns.
    ///
    /// Not a gameplay outcome and not reachable through one: the world has a single
    /// [`ItemIds`] for its lifetime, and its ids are monotonic and never reused, so a
    /// repeat means the caller built a second allocator over a populated world. It is
    /// a named variant because a probe found it: a test that made a fresh allocator
    /// for a second grant got [`GrantRefusal::NoRoom`] and the real cause was three
    /// fields away. Flattening it to "no room" would have sent whoever debugged it
    /// looking at the inventory instead of the allocator.
    IdAlreadyOwned {
        /// The id the allocator repeated.
        id: u32,
    },
}

impl std::fmt::Display for GrantRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchCharacter { name } => write!(formatter, "no character named {name}"),
            Self::UnknownVnum { vnum } => write!(formatter, "no item prototype with vnum {vnum}"),
            Self::NoRoom { size } => {
                write!(formatter, "{NO_ROOM_MESSAGE} (the item is {size} cells)")
            }
            Self::IdsExhausted => formatter.write_str("the item id range is exhausted"),
            Self::NoAllocator => formatter.write_str("the world has no item id allocator yet"),
            Self::IdAlreadyOwned { id } => {
                write!(formatter, "item id {id} is already held by this character")
            }
        }
    }
}

/// A grant that happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantOutcome {
    /// The `GC_ITEM_SET` record to send, exactly 72 bytes once encoded.
    pub record: protocol::gc_item_window::GcItemSet,
    /// The row to write, already carrying the id, owner, window and cell.
    ///
    /// The cell here is the **absolute** one, because the store keys a row by
    /// `(owner_id, window_type, pos)`, and the client record carries the same pair.
    pub row: ItemRow,
    /// The stack count actually stored, after the clamp.
    pub count: u16,
    /// The custom-inventory bank the item went into, or `None` for the base
    /// inventory.
    pub bank: Option<u8>,
}

/// Apply a grant to `character`.
///
/// `protos` supplies the size and the custom-inventory category, `ids` supplies the
/// next id, and the store row is built here so the caller never has to reconstruct
/// the id or the cell.
///
/// # Errors
///
/// [`GrantRefusal`] for a name that matches nobody, a vnum with no prototype, a full
/// inventory, or a spent id range. The character is left unchanged in every case,
/// including after an id has been taken: the id is a monotonic counter, and giving
/// one back would be the only way to reuse it.
///
/// # Panics
///
/// Never. A prototype whose `bSize` is 0 cannot be granted, and arrives as
/// [`GrantRefusal::NoRoom`] rather than as a panic.
pub fn grant_item(
    world: &mut CharacterManager,
    protos: &ItemProtos,
    ids: &mut ItemIds,
    request: &GrantRequest,
    inven_point: u16,
) -> Result<GrantOutcome, GrantRefusal> {
    let Some(proto) = protos.get(request.vnum) else {
        return Err(GrantRefusal::UnknownVnum { vnum: request.vnum });
    };
    let Some(character) = world.find_player_mut(&request.target) else {
        return Err(GrantRefusal::NoSuchCharacter {
            name: request.target.clone(),
        });
    };
    let owner_id = character.player_id();
    // The category list is walked in ascending order, and the search takes the first
    // bank with room rather than the first bank the item matches. See ledger 199.2.
    let banks: Vec<u8> = (0..CATEGORY_NUM)
        .filter(|category| is_custom_category(proto, *category))
        .collect();

    let id = ids.allocate().map_err(|_| GrantRefusal::IdsExhausted)?;
    let mut item = Item::new(id, request.vnum);
    item.set_size(size_from_proto(proto))
        .map_err(|_| GrantRefusal::NoRoom { size: 0 })?;

    // This fork has no per-prototype stack maximum: `command grep -ral dwStackMax`
    // over `server/server/` and `legacy/` returns nothing, while the same search for
    // `bSize` returns `common/tables.h` and `common/length.h`, so the tree is being
    // searched. `ITEM_MAX_COUNT` (`common/item_length.h:19`) is the only ceiling, and
    // it is a global one. See 200.4.
    let count = request
        .count
        .unwrap_or(1)
        .clamp(1, common::item_slots::ITEM_COUNT_LIMIT);
    item.set_count(u32::from(count))
        .map_err(|_| GrantRefusal::NoRoom { size: item.size() })?;
    // `CreateItem` ends in `CItem::SetProto`, which copies `dwFlags` into `m_lFlag`
    // (`G/item.cpp:221-226`), and `GetAntiFlag` reads the prototype (`G/item.h:78`), so
    // both travel from the prototype and not from anything the grant chose.
    item.flags = proto.flags;
    item.anti_flags = proto.anti_flags;

    let placed =
        place(character.items_mut(), &mut item, &banks, inven_point).map_err(
            |error| match error {
                PlacementRefused::NoRoom { size } => GrantRefusal::NoRoom { size },
                // The search and the placement share `footprint_is_clear`, so a
                // placement refusal is never a geometry problem. The one cause a
                // caller can actually hit is a repeated item id, and that gets its own
                // answer: reporting it as "no room" would send an operator to look at
                // the inventory when the allocator is what is wrong.
                PlacementRefused::SearchDisagreedWithPlacement {
                    reason: Rejected::AlreadyOwned { id, at },
                    ..
                } => {
                    tracing::warn!(
                        id,
                        held_at = ?at,
                        "item placement refused: the allocator repeated an id the character already holds"
                    );
                    GrantRefusal::IdAlreadyOwned { id }
                }
                // Any other disagreement is a bug in the shared footprint rule, and is
                // logged as one rather than dressed up as a gameplay refusal.
                PlacementRefused::SearchDisagreedWithPlacement { cell, reason } => {
                    tracing::error!(
                        cell,
                        ?reason,
                        "item placement refused a cell the free-cell search reported free"
                    );
                    GrantRefusal::NoRoom { size: item.size() }
                }
            },
        )?;

    let row = ItemRow {
        id,
        owner_id: Some(owner_id),
        account_id: None,
        window_type: placed.pos.window_type,
        pos: u32::from(placed.pos.cell),
        vnum: item.vnum,
        count: item.count,
        refine_element: item.refine_element,
        transmutation: item.transmutation,
        flags: item.flags,
        anti_flags: item.anti_flags,
        sockets: item.sockets,
        // A fixed 7-slot array, so it is indexed rather than collected: an array
        // that could not be built from an iterator is the compiler's way of saying the
        // two attribute widths are not the same, and they must be.
        attributes: std::array::from_fn(|index| db::items::Attribute {
            b_type: item.attributes[index].b_type,
            s_value: item.attributes[index].s_value,
        }),
    };
    // `highlight` is 1 because legacy evaluates `SetItem` with the caller's
    // `bHighlight`, and a freshly created item is the highlighted one
    // (`cmd_gm.cpp:503`).
    Ok(GrantOutcome {
        record: item.gc_item_set(placed.pos, 1),
        row,
        count,
        bank: placed.bank,
    })
}

/// `bSize` is signed in the file, and zero or negative means an item with no
/// footprint, which is exactly the case [`world::character::grant`] refuses.
fn size_from_proto(proto: &gamedata::item_proto::ItemProto) -> u8 {
    u8::try_from(proto.size).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::item_slots::{EWindows, CUSTOM_INVENTORY_SLOT_START};
    use world::character::CharacterManager;

    /// The owner's `item_proto.txt` and `item_names.txt`, read fresh through the real
    /// reader, so these tests run against real data instead of a fixture that could
    /// drift away from it.
    fn owners() -> ItemProtos {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        let read = |name: &str| {
            let path = dir.join(name);
            std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
        };
        gamedata::item_proto::parse(&read("item_proto.txt"), &read("item_names.txt"))
            .expect("the owner's item proto loads")
    }

    /// A real vnum whose prototype is exactly `size` cells.
    ///
    /// Searched rather than hard-coded, because a vnum written into a test is a vnum
    /// that silently stops existing when the owner's data changes.
    fn a_vnum_of_size(protos: &ItemProtos, size: i32) -> u32 {
        protos
            .rows()
            .iter()
            .find(|proto| proto.size == size)
            .map_or_else(
                || panic!("no prototype of size {size} in the owner's data"),
                |proto| proto.vnum,
            )
    }

    /// A world with one player named `Shaman`.
    fn world_with_shaman() -> CharacterManager {
        let mut world = CharacterManager::new();
        world
            .create_player(7, "Shaman")
            .expect("the first player is free");
        world
    }

    /// Ids drawn from a range that starts at 1, because id 0 is `NO_ITEM`.
    fn fresh_ids() -> ItemIds {
        ItemIds::new(
            world::item::ItemIdRange::new(1, 1_000_000, 1).expect("a range that can issue an id"),
        )
    }

    fn request(vnum: u32, count: Option<u16>) -> GrantRequest {
        GrantRequest {
            target: "Shaman".to_owned(),
            vnum,
            count,
        }
    }

    #[test]
    fn a_grant_reaches_a_character_and_names_the_cell() {
        let protos = owners();
        let mut world = world_with_shaman();
        let mut ids = fresh_ids();
        let vnum = a_vnum_of_size(&protos, 1);

        let outcome = grant_item(&mut world, &protos, &mut ids, &request(vnum, None), 0)
            .expect("an empty inventory has room");
        assert_eq!(outcome.record.cell.window_type, EWindows::Inventory as u8);
        assert_eq!(outcome.record.cell.cell, 0);
        assert_eq!(outcome.record.vnum, vnum);
        assert_eq!(outcome.count, 1, "no count means one");
        assert_eq!(outcome.bank, None);
    }

    #[test]
    fn the_store_row_and_the_client_record_agree_on_the_id_window_and_cell() {
        // The store keys a row by `(owner_id, window_type, pos)` and the client record
        // carries the same window and cell. If the two ever disagreed the client would
        // draw an item the database has not heard of, or the reverse, and neither would
        // be an error. This is the test that fails first if they do.
        let protos = owners();
        let mut world = world_with_shaman();
        let mut ids = fresh_ids();
        let vnum = a_vnum_of_size(&protos, 1);

        let outcome = grant_item(&mut world, &protos, &mut ids, &request(vnum, None), 0)
            .expect("an empty inventory has room");
        assert_eq!(
            outcome.row.id,
            ids.issued(),
            "the id came from the allocator"
        );
        assert_eq!(outcome.row.owner_id, Some(7), "the row names the character");
        assert_eq!(outcome.row.window_type, outcome.record.cell.window_type);
        assert_eq!(outcome.row.pos, u32::from(outcome.record.cell.cell));
        assert_eq!(outcome.row.vnum, outcome.record.vnum);
        assert_eq!(outcome.row.count, outcome.record.count);
        assert_eq!(outcome.row.sockets, outcome.record.sockets);
        assert_eq!(outcome.record.highlight, 1, "a fresh grant is highlighted");
    }

    #[test]
    fn the_flags_and_anti_flags_are_the_prototypes() {
        let protos = owners();
        let mut world = world_with_shaman();
        let mut ids = fresh_ids();
        let proto = protos
            .rows()
            .iter()
            .find(|proto| proto.size == 1 && proto.flags != 0 && proto.anti_flags != 0)
            .expect("the owner's data has a one-cell prototype with both flag words set");
        let outcome = grant_item(&mut world, &protos, &mut ids, &request(proto.vnum, None), 0)
            .expect("an empty inventory has room");
        assert_eq!(outcome.record.flags, proto.flags);
        assert_eq!(outcome.record.anti_flags, proto.anti_flags);
        assert_eq!(
            outcome.row.flags, proto.flags,
            "the row stores what the client saw"
        );
        assert_eq!(outcome.row.anti_flags, proto.anti_flags);
    }

    #[test]
    fn the_encoded_record_is_the_measured_seventy_two_bytes() {
        let protos = owners();
        let mut world = world_with_shaman();
        let mut ids = fresh_ids();
        let vnum = a_vnum_of_size(&protos, 1);
        let outcome = grant_item(&mut world, &protos, &mut ids, &request(vnum, Some(3)), 0)
            .expect("an empty inventory has room");
        let bytes = outcome.record.encode();
        assert_eq!(bytes.len(), 72);
        assert_eq!(bytes[0], 21, "`HEADER_GC_ITEM_SET`");
        assert_eq!(outcome.count, 3, "the count reached the record");
    }

    #[test]
    fn a_count_is_clamped_to_the_global_ceiling_and_never_stored_as_zero() {
        let protos = owners();
        let mut world = world_with_shaman();

        // One allocator for the world, because the world has one. A second allocator
        // would hand back an id the character already holds, which is
        // `IdAlreadyOwned` and not a count question.
        let mut ids = fresh_ids();
        let vnum = a_vnum_of_size(&protos, 1);

        // Over the ceiling: clamped to `ITEM_MAX_COUNT`, not refused and not stored.
        let over = grant_item(
            &mut world,
            &protos,
            &mut ids,
            &request(vnum, Some(60_000)),
            0,
        )
        .expect("a clamp is not a refusal");
        assert_eq!(over.count, common::item_slots::ITEM_COUNT_LIMIT);
        assert_eq!(over.row.count, common::item_slots::ITEM_COUNT_LIMIT);

        // Zero is raised to one rather than stored: a zero count means "destroy this"
        // everywhere else in the codebase, so storing it here would create an item that
        // the first save deletes.
        let zero = grant_item(&mut world, &protos, &mut ids, &request(vnum, Some(0)), 0)
            .expect("a zero count is raised, not refused");
        assert_eq!(zero.count, 1);
        assert_eq!(zero.row.count, 1);
    }

    #[test]
    fn an_unknown_vnum_is_refused_and_names_the_vnum() {
        let protos = owners();
        let mut world = world_with_shaman();
        let mut ids = fresh_ids();
        // `u32::MAX` cannot be a vnum: the file's vnums are far below it, so this is a
        // real absence rather than a guess.
        let refused = grant_item(&mut world, &protos, &mut ids, &request(u32::MAX, None), 0)
            .expect_err("no prototype has that vnum");
        assert_eq!(refused, GrantRefusal::UnknownVnum { vnum: u32::MAX });
    }

    #[test]
    fn an_unknown_name_is_refused_before_anything_is_allocated() {
        let protos = owners();
        let mut world = world_with_shaman();
        let mut ids = fresh_ids();
        let vnum = a_vnum_of_size(&protos, 1);
        let mut missing = request(vnum, None);
        missing.target = "Nobody".to_owned();

        let refused = grant_item(&mut world, &protos, &mut ids, &missing, 0)
            .expect_err("no character has that name");
        assert_eq!(
            refused,
            GrantRefusal::NoSuchCharacter {
                name: "Nobody".to_owned()
            }
        );
        assert_eq!(ids.issued(), 0, "a refused grant takes no id");
    }

    #[test]
    fn the_name_lookup_ignores_case_because_names_are_unique_regardless_of_it() {
        let protos = owners();
        let mut world = world_with_shaman();
        let mut ids = fresh_ids();
        let vnum = a_vnum_of_size(&protos, 1);
        let mut shouty = request(vnum, None);
        shouty.target = "SHAMAN".to_owned();

        let outcome = grant_item(&mut world, &protos, &mut ids, &shouty, 0)
            .expect("the name matches regardless of case");
        assert_eq!(outcome.row.owner_id, Some(7));
    }

    #[test]
    fn a_full_inventory_refuses_with_the_legacy_wording() {
        let protos = owners();
        let mut world = world_with_shaman();
        let mut ids = fresh_ids();
        let vnum = a_vnum_of_size(&protos, 1);

        // Fill the paid-for base inventory through grants themselves, so the fixture
        // cannot disagree with the placement code. `usable_inventory_cells(0)` is 90,
        // which is the point: this is not 180, and writing 180 here is what made the
        // first run of this test fail.
        for _ in 0..90 {
            grant_item(&mut world, &protos, &mut ids, &request(vnum, None), 0)
                .expect("each grant takes a cell");
        }
        let refused = grant_item(&mut world, &protos, &mut ids, &request(vnum, None), 0)
            .expect_err("nothing is free");
        assert_eq!(refused, GrantRefusal::NoRoom { size: 1 });
        assert_eq!(
            refused.to_string(),
            "Not enough inventory space. (the item is 1 cells)",
            "the legacy sentence is a prefix, with the Rewrite's detail after it"
        );
    }

    #[test]
    fn a_locked_page_is_never_offered_by_the_reducer_either() {
        let protos = owners();
        let mut world = world_with_shaman();
        let mut ids = fresh_ids();
        let vnum = a_vnum_of_size(&protos, 1);

        let _ = vnum;
        // A base-inventory item has no custom bank, so this is the only place the bound
        // can be tested without a category rule. `usable_inventory_cells(0)` is 90, so
        // 90 grants fit and the 91st does not.
        let base = a_vnum_with_no_custom_bank(&protos);
        for _ in 0..90 {
            grant_item(&mut world, &protos, &mut ids, &request(base, None), 0)
                .expect("each grant takes a paid-for cell");
        }
        grant_item(&mut world, &protos, &mut ids, &request(base, None), 0)
            .expect_err("the 91st cell is inside a page the player has not paid for");
        // The positive control, on the same world and the same allocator: one more
        // `inven_point` opens cell 90, so the refusal above is the bound and not a
        // broken search.
        let paid = grant_item(&mut world, &protos, &mut ids, &request(base, None), 1)
            .expect("one more `inven_point` opens cell 90");
        assert_eq!(paid.record.cell.cell, 90);
        let _ = vnum;
    }

    #[test]
    fn an_item_in_a_custom_bank_lands_in_that_bank_and_not_in_the_base_inventory() {
        let protos = owners();
        let mut world = world_with_shaman();
        let mut ids = fresh_ids();
        let custom = a_vnum_with_a_custom_bank(&protos);
        let outcome = grant_item(&mut world, &protos, &mut ids, &request(custom, None), 0)
            .expect("a fresh character has room everywhere");
        assert!(outcome.bank.is_some(), "the item matched a custom bank");
        assert!(
            outcome.record.cell.cell >= CUSTOM_INVENTORY_SLOT_START,
            "the item landed at cell {} which is below the custom start",
            outcome.record.cell.cell
        );
    }

    #[test]
    fn a_multi_cell_item_moves_the_grid_and_occupies_every_cell_it_walks() {
        let protos = owners();
        let mut world = world_with_shaman();
        let mut ids = fresh_ids();
        // A 2-cell item must not be placeable on top of the 1-cell item at cell 0.
        let one = a_vnum_of_size(&protos, 1);
        let two = a_vnum_of_size(&protos, 2);
        let first = grant_item(&mut world, &protos, &mut ids, &request(one, None), 0)
            .expect("cell 0 is free");
        let second = grant_item(&mut world, &protos, &mut ids, &request(two, None), 0)
            .expect("cells 1 and 6 are free");
        assert_eq!(first.record.cell.cell, 0);
        assert_ne!(
            second.record.cell.cell, 0,
            "a 2-cell item cannot take a cell the 1-cell item's grid covers"
        );
    }

    #[test]
    fn every_refusal_leaves_the_world_exactly_as_it_was() {
        let protos = owners();
        let mut world = world_with_shaman();
        let mut ids = fresh_ids();
        let one = a_vnum_of_size(&protos, 1);

        // Fill the paid-for inventory, then record the world's state.
        for _ in 0..90 {
            grant_item(&mut world, &protos, &mut ids, &request(one, None), 0)
                .expect("each grant takes a cell");
        }
        let items_before = world
            .find_player_by_name("Shaman")
            .expect("Shaman is online")
            .items()
            .len();
        assert_eq!(items_before, 90);

        // Every refusal path.
        let mut bad = request(one, None);
        bad.target = "Nobody".to_owned();
        assert!(grant_item(&mut world, &protos, &mut ids, &bad, 0).is_err());
        assert!(grant_item(&mut world, &protos, &mut ids, &request(u32::MAX, None), 0).is_err());
        for _ in 0..20 {
            assert!(grant_item(&mut world, &protos, &mut ids, &request(one, None), 0).is_err());
        }

        assert_eq!(
            world
                .find_player_by_name("Shaman")
                .expect("Shaman is online")
                .items()
                .len(),
            items_before,
            "a refusal adds nothing to the world"
        );
        // A refused **placement** does consume an id, because the id is taken before
        // the cell is chosen. That is recorded rather than papered over: what matters
        // is that the allocator never hands the same id out twice, so a retry cannot
        // collide. (A refused **target** takes no id, because the name is resolved
        // first, which the unknown-name test pins.)
        assert_eq!(
            ids.issued(),
            110,
            "20 refused placements each burned one id, and none was reissued"
        );
        // The one item per id invariant is enforced by `CharacterItems::set`, and the
        // repeated-id test below is what proves it: two grants of the same id cannot
        // both land, so the count here and the count there agree.
    }

    #[test]
    fn a_repeated_id_is_reported_as_a_repeated_id_and_not_as_a_full_inventory() {
        // Found by a probe rather than by reading. A second allocator over a populated
        // world hands back id 1 again, the character already owns it, and the first
        // version of this reducer flattened that to "no room". The remedy for each is
        // different, so they are different answers.
        let protos = owners();
        let mut world = world_with_shaman();
        let one = a_vnum_of_size(&protos, 1);

        let mut first = fresh_ids();
        grant_item(&mut world, &protos, &mut first, &request(one, None), 0)
            .expect("the first grant works");

        // A second allocator restarts at id 1, which the character now holds.
        let mut second = fresh_ids();
        let refused = grant_item(&mut world, &protos, &mut second, &request(one, None), 0)
            .expect_err("the id is already owned");
        assert_eq!(refused, GrantRefusal::IdAlreadyOwned { id: 1 });
        assert_ne!(refused, GrantRefusal::NoRoom { size: 1 });
    }

    /// A real vnum that belongs to **no** custom bank, so a grant of it must fall
    /// through to the base inventory.
    fn a_vnum_with_no_custom_bank(protos: &ItemProtos) -> u32 {
        protos
            .rows()
            .iter()
            .filter(|proto| (0..CATEGORY_NUM).all(|category| !is_custom_category(proto, category)))
            .map(|proto| proto.vnum)
            .next()
            .expect("the owner's data has an item in no custom bank")
    }

    /// A real vnum that belongs to at least one custom bank.
    fn a_vnum_with_a_custom_bank(protos: &ItemProtos) -> u32 {
        protos
            .rows()
            .iter()
            .find(|proto| (0..CATEGORY_NUM).any(|category| is_custom_category(proto, category)))
            .map(|proto| proto.vnum)
            .expect("the owner's data has an item in a custom bank")
    }
}
