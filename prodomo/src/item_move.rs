//! `CG_ITEM_MOVE` (13) between the descriptor, the world, and the store.
//!
//! The move itself is [`world::character::move_item`], which changes a character's storage and
//! answers with the records to send and the row changes to make. This module holds what the
//! game thread needs around it that the world does not: the prototype facts the move reads,
//! the worn belt's grade, the stored form of each row change, and the chat line a refusal
//! sends.
//!
//! # The order the descriptor keeps
//!
//! The world changes first, on the game thread. The descriptor then writes the rows in one
//! transaction and sends the records only once the rows are stored, so a client is never told
//! of a move the store did not take. A write that fails closes the descriptor, which takes the
//! character out of the world; the next login loads what the store holds. Legacy sends the
//! records at once and saves the items later (`CHARACTER::MoveItem`, then `ITEM_MANAGER`'s
//! delayed save), so the order is the Rewrite's, and a client cannot tell the two apart
//! because it waits for neither.
//!
//! Each descriptor stores its own steps in turn, but a drop and the pick-up of what it laid are
//! two descriptors' steps on one item. The world hands them out in its order, and the pick-up
//! waits for the drop's rows before it writes its own
//! ([`crate::item_move::StoreOrder`]): the item's row leaves the dropper before it reaches the
//! picker, and a picker whose dropper stored nothing is closed with its step unwritten, the store
//! keeping the item where it was.

use common::enums::EWearPositions;
use common::item_slots::{EWindows, INVENTORY_MAX_NUM};
use common::vid::Vid;
use db::items::{Attribute, ItemRow, RowChange};
use gamedata::belt_inventory::can_move_into_belt_inventory;
use gamedata::item_custom_category::{is_custom_category, CATEGORY_NUM};
use gamedata::item_proto::ItemProtos;
use gamedata::item_proto_value::type_value;
use gamedata::locale_string::LocaleStrings;
use protocol::gc_actors::{GcCharacterGoldChange, GcCharacterUpdate};
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_vid::{GcHeaderAndDword, GcSpecialEffect};
use protocol::item_pos::ItemPos;
use tokio::sync::watch;
use world::character::{
    CharacterItems, CharacterLook, ItemChange, MoveDone, MoveFacts, MoveKind, MoveRecord,
    MoveRefused, Points, Quickslots, StoreRecord,
};
use world::item::Item;

use crate::chat_line::{chat_packet, Arg, Recipient};
use crate::game_loop_messages::RelayScope;
use crate::item_load::stored_row_position;
use crate::loading_phase::point_changes;

/// The flat cell of the worn belt: `INVENTORY_MAX_NUM + WEAR_BELT`, 180 + 27.
pub const BELT_WEAR_CELL: u16 = INVENTORY_MAX_NUM + EWearPositions::Belt as u16;

/// What the move reads of the moving character: what the descriptor knows of it, and what the
/// world adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mover {
    /// Whether the character attacked, or was selected, in the last 1.5 s.
    pub recently_fought: bool,
    /// The character's empire, which a chat line carries.
    pub empire: u8,
    /// The descriptor's language: the one a chat line is looked up in, and the one
    /// `UpdatePacket` sends (`char.cpp:1321`).
    pub language: u8,
    /// `m_bPKMode`, which `UpdatePacket` sends (`G/char.cpp:1315`). The descriptor does not
    /// hold it: the world sets it from the body's card before an item step runs.
    pub pk_mode: u8,
    /// The two affect flag words `UpdatePacket` sends (`dwAffectFlag`). The descriptor does not
    /// hold them: the world sets them from the character's affects before an item step runs.
    pub affect_flags: [u32; 2],
}

impl Mover {
    /// The descriptor a chat line to the mover goes to.
    #[must_use]
    pub const fn recipient(self, strings: &LocaleStrings) -> Recipient<'_> {
        Recipient {
            strings,
            language: self.language,
            empire: self.empire,
        }
    }
}

/// What a move the world made has left for the descriptor to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedItems {
    /// Which outcome the move had, for the log line.
    pub kind: MoveKind,
    /// The store `player.id` of the character whose items moved.
    pub owner_id: u32,
    /// The records the client is sent, each its own frame, in the order legacy sends them.
    pub records: Vec<Vec<u8>>,
    /// Where in `records` a drop's or a pick-up's ground record stood. The world puts there
    /// what the item's view sends the mover, which may be nothing (`AddToGround`,
    /// `G/item.cpp:549-584`; `RemoveFromGround`, `:533-547`), and sends the others theirs itself.
    pub ground_at: Option<usize>,
    /// The records others are sent too, in order, each with whom it reaches: the look, the
    /// effects and the broadcast points reach the characters that see the mover
    /// (`PacketAround`).
    pub around: Vec<(RelayScope, Vec<u8>)>,
    /// The row changes, in the order they are applied.
    pub changes: Vec<RowChange>,
    /// The mover's points after the move, which the descriptor saves.
    pub points: Option<Points>,
    /// The mover's quickslots after the step's syncs, which the descriptor saves.
    pub quickslots: Option<Quickslots>,
    /// How much a step that paid or was paid changed the mover's gold, which the row changes'
    /// transaction adds to the stored gold with them.
    pub gold: Option<i64>,
    /// Where the step's rows stand among another descriptor's on the same item.
    pub store_order: StoreOrder,
}

/// Where a step's store stands among another descriptor's on the same item: a drop tells the
/// pick-ups of what it laid when its rows are stored, and a pick-up waits for that before it
/// writes its own. Drops never wait, so no two steps wait on each other.
#[derive(Debug, Clone, Default)]
pub struct StoreOrder {
    /// Told once the step's rows are stored. Dropped untold, it tells the waiters that the
    /// rows never will be.
    tell: Option<watch::Sender<bool>>,
    /// Told once the step this one follows has stored its rows.
    wait: Option<watch::Receiver<bool>>,
}

impl StoreOrder {
    /// A drop's order, and what the item it laid hands each pick-up to wait on.
    #[must_use]
    pub fn laid() -> (Self, watch::Receiver<bool>) {
        let (tell, wait) = watch::channel(false);
        let order = Self {
            tell: Some(tell),
            wait: None,
        };
        (order, wait)
    }

    /// The order of a step that stores after the one `wait` hears from.
    #[must_use]
    pub fn after(wait: watch::Receiver<bool>) -> Self {
        Self {
            tell: None,
            wait: Some(wait),
        }
    }

    /// Waits for the step's turn to store: true once the step it follows has stored its rows,
    /// or at once with none to follow; false when that step ended without storing.
    pub async fn turn(&mut self) -> bool {
        match self.wait.as_mut() {
            Some(wait) => wait.wait_for(|stored| *stored).await.is_ok(),
            None => true,
        }
    }

    /// Tells the steps that follow this one that its rows are stored.
    pub fn stored(&self) {
        if let Some(tell) = &self.tell {
            let _was_stored = tell.send_replace(true);
        }
    }
}

/// Two orders are equal when they tell and wait on the same channels.
impl PartialEq for StoreOrder {
    fn eq(&self, other: &Self) -> bool {
        let tells = match (&self.tell, &other.tell) {
            (Some(one), Some(two)) => one.same_channel(two),
            (one, two) => one.is_none() && two.is_none(),
        };
        let waits = match (&self.wait, &other.wait) {
            (Some(one), Some(two)) => one.same_channel(two),
            (one, two) => one.is_none() && two.is_none(),
        };
        tells && waits
    }
}

impl Eq for StoreOrder {}

/// The change from `before` to `after` that a Transfer adds to the stored gold.
///
/// Held gold never passes `GOLD_MAX_MAX`, which is below `i64::MAX`, so the change always
/// fits; were it not to, the saturated change is one the store refuses.
#[must_use]
pub fn gold_delta(before: u64, after: u64) -> i64 {
    let change = i128::from(after) - i128::from(before);
    i64::try_from(change).unwrap_or(if change < 0 { i64::MIN } else { i64::MAX })
}

impl MovedItems {
    /// The descriptor's half of a move the world made for `owner_id`, whose VID is `vid`.
    ///
    /// Each notice is looked up in the mover's language in `strings`.
    #[must_use]
    pub fn new(
        owner_id: u32,
        vid: u32,
        done: MoveDone,
        mover: Mover,
        protos: &ItemProtos,
        strings: &LocaleStrings,
    ) -> Self {
        let to = mover.recipient(strings);
        let mut records = Vec::with_capacity(done.records.len());
        let mut ground_at = None;
        let mut around = Vec::new();
        for record in done.records {
            let mut frame = Vec::new();
            let shared = match record {
                MoveRecord::Item(record) => {
                    record.encode_into(&mut frame);
                    None
                }
                MoveRecord::Point(record) => {
                    frame = point_changes(&[record], vid).concat();
                    record.broadcast.then_some(RelayScope::ViewExceptSelf)
                }
                MoveRecord::Look(look) => {
                    character_update(vid, &look, mover).encode_into(&mut frame);
                    Some(RelayScope::ViewExceptSelf)
                }
                MoveRecord::Effect(effect_type) => {
                    GcSpecialEffect { effect_type, vid }.encode_into(&mut frame);
                    Some(RelayScope::ViewExceptSelf)
                }
                MoveRecord::Notice(text) => {
                    frame = notice(text, to);
                    None
                }
                // The view sends it (`MovedItems::ground_at`).
                MoveRecord::Ground(_) => {
                    ground_at = Some(records.len());
                    continue;
                }
                MoveRecord::PickedUp { vnum } => {
                    frame = picked_up_notice(protos, vnum, to);
                    None
                }
                MoveRecord::Quickslot(record) => {
                    frame = crate::quickslot::encode(&[record]).concat();
                    None
                }
                // The world runs every sync before it answers; one left here sends nothing.
                MoveRecord::QuickslotSync(_) => continue,
                MoveRecord::Gold { amount, value } => {
                    GcCharacterGoldChange::new(vid, amount, value).encode_into(&mut frame);
                    None
                }
                MoveRecord::Store(record) => {
                    frame = store_record(record);
                    None
                }
            };
            if let Some(scope) = shared {
                around.push((scope, frame.clone()));
            }
            records.push(frame);
        }
        Self {
            kind: done.kind,
            owner_id,
            records,
            ground_at,
            around,
            changes: row_changes(owner_id, &done.changes),
            points: None,
            quickslots: None,
            gold: None,
            store_order: StoreOrder::default(),
        }
    }

    /// Puts `own`, the records a ground item's view sent the mover, where the ground record
    /// stood, so they keep legacy's order among the item records.
    pub fn place_ground(&mut self, own: Vec<Vec<u8>>) {
        if let Some(at) = self.ground_at {
            self.records.splice(at..at, own);
        }
    }
}

/// `CHARACTER::UpdatePacket` (`G/char.cpp:1277-1340`) for the look a move left, with the
/// mover's PK mode (`:1315`) and the descriptor's language (`:1321`).
///
/// The state flags, the guild, the alignment, the mount and the premium come from systems this
/// build does not have, and are 0. The affect words come from the mover.
pub(crate) fn character_update(vid: u32, look: &CharacterLook, mover: Mover) -> GcCharacterUpdate {
    GcCharacterUpdate {
        dw_vid: vid,
        aw_part: look.parts,
        b_moving_speed: look.moving_speed,
        b_attack_speed: look.attack_speed,
        b_state_flag: 0,
        dw_affect_flag: mover.affect_flags,
        dw_guild_id: 0,
        s_alignment: 0,
        dw_level: look.level,
        dw_conqueror_level: look.conqueror_level,
        b_pk_mode: mover.pk_mode,
        dw_mount_vnum: 0,
        b_refine_element_type: look.refine_element_type,
        dw_new_is_guild_name: 0,
        by_premium: 0,
        i_premium_time: 0,
        b_language: mover.language,
    }
}

/// Why the world made no move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveItemRefused {
    /// No character is online under that VID.
    ///
    /// A descriptor only sends a move once its character has joined the world, so this is
    /// a descriptor that outlived its character, not a state a player can reach.
    NoSuchCharacter {
        /// The VID the move named.
        vid: Vid,
    },
    /// The move was refused, and the storage is as it was.
    Refused(MoveRefused),
}

impl std::fmt::Display for MoveItemRefused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchCharacter { vid } => write!(formatter, "no character is online as {vid}"),
            Self::Refused(reason) => write!(formatter, "the move was refused: {reason}"),
        }
    }
}

impl std::error::Error for MoveItemRefused {}

/// The prototype facts a move reads for `vnum`, or `None` when it has no prototype.
///
/// The banks are walked in ascending order, the order `GetEmptyInventory` searches them in;
/// `IsDragonSoul` is `GetType() == ITEM_DS` (`G/item.h`).
#[must_use]
pub fn move_facts(protos: &ItemProtos, vnum: u32) -> Option<MoveFacts> {
    let proto = protos.get(vnum)?;
    Some(MoveFacts {
        categories: (0..CATEGORY_NUM)
            .filter(|category| is_custom_category(proto, *category))
            .collect(),
        belt_eligible: can_move_into_belt_inventory(proto),
        dragon_soul: type_value(b"ITEM_DS") == Some(proto.item_type),
        chains_quickslots: world::character::chains_when_used_up(proto),
    })
}

/// The worn belt's `value0`, or `None` when no belt is worn.
///
/// `GetWear(WEAR_BELT)->GetValue(0)` (`G/char_item.cpp:766-770`), and `GetValue` reads the
/// prototype's `alValues`. An item whose vnum has no prototype cannot be worn in legacy,
/// because `CreateItem` refuses it, so it answers as no belt.
#[must_use]
pub fn belt_grade(items: &CharacterItems, protos: &ItemProtos) -> Option<i32> {
    let belt = items.item_at(ItemPos::new(EWindows::Inventory as u8, BELT_WEAR_CELL))?;
    protos.get(belt.vnum).map(|proto| proto.values[0])
}

/// The store's form of the world's changes, for the character whose store id is `owner_id`.
#[must_use]
pub fn row_changes(owner_id: u32, changes: &[ItemChange]) -> Vec<RowChange> {
    changes
        .iter()
        .map(|change| match change {
            ItemChange::Moved { id, pos } => {
                let (window_type, pos) = stored_row_position(*pos);
                RowChange::Moved {
                    id: *id,
                    window_type,
                    pos,
                }
            }
            ItemChange::Count { id, count } => RowChange::Count {
                id: *id,
                count: *count,
            },
            ItemChange::Created(item) => RowChange::Created(item_row(owner_id, item)),
            ItemChange::Destroyed { id } => RowChange::Destroyed { id: *id },
            ItemChange::Sockets { id, sockets } => RowChange::Sockets {
                id: *id,
                sockets: *sockets,
            },
            ItemChange::Stored { id, account, pos } => RowChange::Stored {
                id: *id,
                account: *account,
                pos: *pos,
            },
            ItemChange::Retrieved { id, account, pos } => {
                let (window_type, pos) = stored_row_position(*pos);
                RowChange::Retrieved {
                    id: *id,
                    account: *account,
                    window_type,
                    pos,
                }
            }
            ItemChange::Given { id, to, pos } => {
                let (window_type, pos) = stored_row_position(*pos);
                RowChange::Given {
                    id: *id,
                    to: *to,
                    window_type,
                    pos,
                }
            }
        })
        .collect()
}

/// A safebox or mall record's wire bytes.
#[must_use]
pub fn store_record(record: StoreRecord) -> Vec<u8> {
    let mut frame = Vec::new();
    match record {
        StoreRecord::Set(set) => set.encode_into(&mut frame),
        StoreRecord::Del { window, pos } => {
            GcHeaderAndDword::new(window.del_header(), pos).encode_into(&mut frame);
        }
    }
    frame
}

/// The row a split's new item is stored as.
fn item_row(owner_id: u32, item: &Item) -> ItemRow {
    let (window_type, pos) = stored_row_position(item.pos);
    ItemRow {
        id: item.id,
        owner_id: Some(owner_id),
        account_id: None,
        window_type,
        pos,
        vnum: item.vnum,
        count: item.count,
        refine_element: item.refine_element,
        transmutation: item.transmutation,
        flags: item.flags,
        anti_flags: item.anti_flags,
        sockets: item.sockets,
        attributes: std::array::from_fn(|index| Attribute {
            b_type: item.attributes[index].b_type,
            s_value: item.attributes[index].s_value,
        }),
    }
}

/// The `CHAT_TYPE_INFO` line a refusal sends, encoded, or `None` when legacy sends nothing.
///
/// `CHARACTER::ChatPacket` (`G/char.cpp:5140-5187`), which [`chat_packet`] ports: the text is
/// looked up in the descriptor's language, and none of [`MoveRefused::notice`]'s texts holds a
/// `%`.
#[must_use]
pub fn refusal_notice(refused: &MoveRefused, to: Recipient<'_>) -> Option<Vec<u8>> {
    refused.notice().map(|text| notice(text, to))
}

/// `g_ItemDropTimeLimitValue`, which `G/questmanager.cpp:1605` makes 1000 ms from an
/// `item_drop_limit_time` of 1. Only `legacy/gamedata/locale/europe/quest/questlib_extra.lua:139`
/// sets that flag, and legacy never loads the file (ledger 227), so the owner's value is the stored
/// event flag the snapshot lacks; this stays 1000 ms until the owner decides.
pub const DROP_LIMIT: std::time::Duration = std::time::Duration::from_millis(1000);

/// What `DropItem` sends inside [`DROP_LIMIT`] (`G/char_item.cpp:7460`).
pub const DROP_LIMIT_NOTICE: &str = "@@(char_item.cpp)tradus:[#Unk]You cannot drop Yang yet";

/// Whether a drop `elapsed` after the last one that passed this check may go on
/// (`G/char_item.cpp:7456-7465`). `None` is a character that has not dropped yet.
#[must_use]
pub fn drop_allowed(elapsed: Option<std::time::Duration>) -> bool {
    elapsed.is_none_or(|elapsed| elapsed >= DROP_LIMIT)
}

/// The [`DROP_LIMIT_NOTICE`] line.
#[must_use]
pub fn drop_limit_notice(to: Recipient<'_>) -> Vec<u8> {
    notice(DROP_LIMIT_NOTICE, to)
}

/// `ChatPacket(CHAT_TYPE_INFO, "[LS;444;%s]", item->GetName())` (`G/char_item.cpp:8048`,
/// `:8080`).
///
/// `GetName` is `LC_LOCALE_ITEM_TEXT(vnum, LOCALE_DEFAULT)`, the item name table this build
/// does not load, so the name is the prototype's locale name. An item with no prototype cannot
/// be picked up, so the empty name is never sent.
fn picked_up_notice(protos: &ItemProtos, vnum: u32, to: Recipient<'_>) -> Vec<u8> {
    let name = protos
        .get(vnum)
        .map_or(&[][..], |proto| proto.locale_name.as_slice());
    chat_packet(to, CHAT_TYPE_INFO, PICKED_UP_NOTICE, &[Arg::Text(name)])
}

/// The format of the pick-up line.
const PICKED_UP_NOTICE: &[u8] = b"[LS;444;%s]";

/// `ChatPacket(CHAT_TYPE_INFO, LC_TEXT(text))`: a `CHAT_TYPE_INFO` line with no argument.
fn notice(text: &str, to: Recipient<'_>) -> Vec<u8> {
    chat_packet(to, CHAT_TYPE_INFO, text.as_bytes(), &[])
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_drop_waits_a_full_second_after_the_last_one() {
        use std::time::Duration;
        assert!(super::drop_allowed(None));
        assert!(!super::drop_allowed(Some(Duration::ZERO)));
        assert!(!super::drop_allowed(Some(Duration::from_millis(999))));
        assert!(super::drop_allowed(Some(Duration::from_millis(1000))));
    }

    /// Orders are equal when they tell and wait on the same channels; a step with no order
    /// takes its turn at once.
    #[tokio::test]
    async fn store_orders_compare_by_channel_and_an_unordered_step_never_waits() {
        use super::StoreOrder;
        let (laid, wait) = StoreOrder::laid();
        let (other, other_wait) = StoreOrder::laid();
        assert_eq!(StoreOrder::default(), StoreOrder::default());
        assert_eq!(laid, laid.clone());
        assert_ne!(laid, other);
        assert_ne!(laid, StoreOrder::default());
        assert_eq!(StoreOrder::after(wait.clone()), StoreOrder::after(wait));
        assert_ne!(StoreOrder::after(other_wait), StoreOrder::default());
        assert!(StoreOrder::default().turn().await);
        assert!(!*other.tell.as_ref().expect("a drop tells").borrow());
        other.stored();
        assert!(*other.tell.as_ref().expect("a drop tells").borrow());
    }

    use super::*;
    use common::item_slots::{BELT_INVENTORY_SLOT_START, CUSTOM_INVENTORY_SLOT_START};
    use protocol::gc_inventory::HEADER_GC_CHAT;
    use world::character::{ItemRecord, Unported};

    fn owners() -> ItemProtos {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        ItemProtos::load(&dir).expect("the owner's item protos load")
    }

    const INVENTORY: u8 = EWindows::Inventory as u8;

    #[test]
    fn the_worn_belt_is_flat_cell_207() {
        // `WEAR_BELT` is the 28th wear position (`length.h:240`), after nothing gated.
        assert_eq!(BELT_WEAR_CELL, 207);
    }

    #[test]
    fn the_facts_come_from_the_prototype() {
        let protos = owners();
        // Hand-checked in the owner's data: 27001 is a red potion (`ITEM_USE`, `USE_POTION`),
        // which a belt cell accepts, and 11 is a sword, which it does not.
        let potion = move_facts(&protos, 27001).expect("27001 is in the owner's data");
        assert!(potion.belt_eligible);
        assert!(!potion.dragon_soul);
        let sword = move_facts(&protos, 11).expect("11 is in the owner's data");
        assert!(!sword.belt_eligible);
        assert!(!sword.dragon_soul);
        assert_eq!(move_facts(&protos, 0), None);
        // An item in two banks lists both, ascending (ledger 199.2 names 27987).
        let two_banks = move_facts(&protos, 27987).expect("27987 is in the owner's data");
        assert_eq!(two_banks.categories, vec![2, 3]);
        // A dragon-soul stone is `ITEM_DS`.
        let stone = protos
            .rows()
            .iter()
            .find(|proto| Some(proto.item_type) == type_value(b"ITEM_DS"))
            .expect("the owner's data has a dragon-soul item");
        assert!(move_facts(&protos, stone.vnum).is_some_and(|facts| facts.dragon_soul));
    }

    #[test]
    fn the_belt_grade_is_the_worn_belts_value0_and_nothing_else_counts() {
        let protos = owners();
        let belt_proto = protos
            .rows()
            .iter()
            .find(|proto| proto.size == 1 && proto.values[0] != 0)
            .expect("the owner's data has a one-cell item with a value0");
        let mut items = CharacterItems::default();
        assert_eq!(belt_grade(&items, &protos), None);
        let mut belt = Item::new(5, belt_proto.vnum);
        belt.set_size(1).expect("one cell");
        // An item next to the belt cell is not the belt.
        items
            .set(ItemPos::new(INVENTORY, BELT_WEAR_CELL + 1), &belt)
            .expect("the cell after the belt is free");
        assert_eq!(belt_grade(&items, &protos), None);
        let _ = items.release(5).expect("just placed");
        items
            .set(ItemPos::new(INVENTORY, BELT_WEAR_CELL), &belt)
            .expect("the belt cell is free");
        assert_eq!(belt_grade(&items, &protos), Some(belt_proto.values[0]));
    }

    #[test]
    fn each_change_is_stored_in_the_stores_terms() {
        let mut piece = Item::new(0x0102_0304, 27001);
        piece.count = 0x0203;
        piece.flags = 0x0405_0607;
        piece.anti_flags = 0x0809_0A0B;
        piece.refine_element = 0x0C0D_0E0F;
        piece.transmutation = 0x1011_1213;
        piece.sockets = [1, 2, 3, 4, 5, 6];
        piece.pos = ItemPos::new(INVENTORY, BELT_INVENTORY_SLOT_START + 3);
        piece.attributes[6] = protocol::gc_item_window::ItemAttribute::new(7, 0x0708);
        let changes = [
            ItemChange::Moved {
                id: 1,
                pos: ItemPos::new(INVENTORY, BELT_INVENTORY_SLOT_START + 15),
            },
            ItemChange::Moved {
                id: 2,
                pos: ItemPos::new(INVENTORY, CUSTOM_INVENTORY_SLOT_START),
            },
            ItemChange::Count {
                id: 3,
                count: 0x0506,
            },
            ItemChange::Created(piece.clone()),
            ItemChange::Destroyed { id: 4 },
        ];
        let rows = row_changes(0x0A0B_0C0D, &changes);
        assert_eq!(rows.len(), 5);
        assert_eq!(
            rows[0],
            RowChange::Moved {
                id: 1,
                window_type: EWindows::BeltInventory as u8,
                pos: 15
            }
        );
        assert_eq!(
            rows[1],
            RowChange::Moved {
                id: 2,
                window_type: INVENTORY,
                pos: u32::from(CUSTOM_INVENTORY_SLOT_START)
            }
        );
        assert_eq!(
            rows[2],
            RowChange::Count {
                id: 3,
                count: 0x0506
            }
        );
        let RowChange::Created(row) = &rows[3] else {
            panic!("the split's item is a created row");
        };
        assert_eq!(row.id, piece.id);
        assert_eq!(row.owner_id, Some(0x0A0B_0C0D));
        assert_eq!(
            (row.window_type, row.pos),
            (EWindows::BeltInventory as u8, 3)
        );
        assert_eq!(row.vnum, 27001);
        assert_eq!(row.count, 0x0203);
        assert_eq!(row.flags, piece.flags);
        assert_eq!(row.anti_flags, piece.anti_flags);
        assert_eq!(row.refine_element, piece.refine_element);
        assert_eq!(row.transmutation, piece.transmutation);
        assert_eq!(row.sockets, piece.sockets);
        assert_eq!(
            (row.attributes[6].b_type, row.attributes[6].s_value),
            (7, 0x0708)
        );
        assert_eq!(
            (row.attributes[0].b_type, row.attributes[0].s_value),
            (0, 0)
        );
        assert_eq!(rows[4], RowChange::Destroyed { id: 4 });
    }

    #[test]
    fn the_records_are_one_frame_each_in_order() {
        let mut item = Item::new(9, 27001);
        item.set_size(1).expect("one cell");
        let from = ItemPos::new(INVENTORY, 1);
        let to = ItemPos::new(INVENTORY, 2);
        let done = MoveDone {
            kind: MoveKind::Moved,
            records: vec![
                MoveRecord::Item(ItemRecord::Set(world::item::gc_item_clear(from))),
                MoveRecord::Item(ItemRecord::Set(item.gc_item_set(to, 0))),
            ],
            changes: vec![ItemChange::Moved { id: 9, pos: to }],
        };
        let actor = Mover {
            recently_fought: false,
            empire: 1,
            language: 1,
            pk_mode: crate::loading_phase::PK_MODE_PEACE,
            affect_flags: [0; 2],
        };
        let strings = LocaleStrings::default();
        let moved = MovedItems::new(7, 7, done.clone(), actor, &owners(), &strings);
        assert_eq!(moved.kind, MoveKind::Moved);
        assert_eq!(moved.owner_id, 7);
        assert_eq!(moved.records.len(), 2);
        assert!(moved.around.is_empty(), "an item record is the mover's own");
        for (frame, record) in moved.records.iter().zip(done.records) {
            let MoveRecord::Item(record) = record else {
                panic!("only item records were made");
            };
            let mut expected = Vec::new();
            record.encode_into(&mut expected);
            assert_eq!(frame, &expected);
            assert_eq!(frame.len(), 72, "an item set is the measured 72 bytes");
        }
        assert_eq!(
            moved.changes,
            vec![RowChange::Moved {
                id: 9,
                window_type: INVENTORY,
                pos: 2
            }]
        );
    }

    /// A look is `UpdatePacket` with the mover's PK mode and language (`G/char.cpp:1315`,
    /// `:1321`), to the mover and to its view.
    #[test]
    fn a_look_carries_the_movers_pk_mode_and_language() {
        let look = CharacterLook {
            parts: [1, 2, 3, 4, 5, 6],
            moving_speed: 98,
            attack_speed: 100,
            level: 9,
            conqueror_level: 0,
            refine_element_type: 0,
        };
        let done = MoveDone {
            kind: MoveKind::Moved,
            records: vec![MoveRecord::Look(look)],
            changes: Vec::new(),
        };
        let actor = Mover {
            recently_fought: false,
            empire: 1,
            language: 4,
            pk_mode: crate::loading_phase::PK_MODE_PROTECT,
            affect_flags: [0; 2],
        };
        let strings = LocaleStrings::default();
        let moved = MovedItems::new(7, 0x0102_0304, done, actor, &owners(), &strings);
        let frame = &moved.records[0];
        assert_eq!(frame.len(), 55);
        assert_eq!(frame[0], 19, "GC_CHARACTER_UPDATE");
        assert_eq!(&frame[1..5], &0x0102_0304u32.to_le_bytes());
        assert_eq!(frame[42], 3, "bPKMode");
        assert_eq!(frame[54], 4, "bLanguage");
        assert_eq!(
            moved.around,
            vec![(RelayScope::ViewExceptSelf, frame.clone())]
        );
    }

    /// A ground record leaves only its place: the world fills it with what the item's view
    /// sends the mover, and nobody else is sent anything from here.
    #[test]
    fn a_ground_record_leaves_its_place_for_the_view_and_no_relay() {
        let done = MoveDone {
            kind: MoveKind::Dropped,
            records: vec![
                MoveRecord::Notice("first"),
                MoveRecord::Ground(world::character::GroundRecord::Del { vid: 3 }),
                MoveRecord::Notice("last"),
            ],
            changes: Vec::new(),
        };
        let actor = Mover {
            recently_fought: false,
            empire: 1,
            language: 1,
            pk_mode: crate::loading_phase::PK_MODE_PEACE,
            affect_flags: [0; 2],
        };
        let strings = LocaleStrings::default();
        let moved = MovedItems::new(7, 7, done, actor, &owners(), &strings);
        assert_eq!(moved.records.len(), 2);
        assert!(moved.records[0].ends_with(b"first") && moved.records[1].ends_with(b"last"));
        assert_eq!(moved.ground_at, Some(1));
        assert!(moved.around.is_empty());
        let mut placed = moved.clone();
        placed.place_ground(vec![b"own".to_vec(), b"two".to_vec()]);
        let (first, last) = (moved.records[0].clone(), moved.records[1].clone());
        assert_eq!(
            placed.records,
            vec![first, b"own".to_vec(), b"two".to_vec(), last]
        );
        let mut none = MovedItems::new(
            7,
            7,
            MoveDone {
                kind: MoveKind::Moved,
                records: vec![MoveRecord::Notice("only")],
                changes: Vec::new(),
            },
            actor,
            &owners(),
            &strings,
        );
        assert_eq!(none.ground_at, None);
        let only = none.records.clone();
        none.place_ground(vec![b"own".to_vec()]);
        assert_eq!(
            none.records, only,
            "a step with no ground record places nothing"
        );
    }

    #[test]
    fn a_refusal_with_a_notice_sends_one_info_line_and_the_rest_send_nothing() {
        let strings = LocaleStrings::default();
        let to = |empire| Recipient {
            strings: &strings,
            language: 1,
            empire,
        };
        let line =
            refusal_notice(&MoveRefused::NotForBelt, to(3)).expect("the belt refusal speaks");
        // header 4, size 10 + 9, type 1 (INFO), id 0, empire 3, bCanFormat 1, "[LS;1097]".
        let mut expected = vec![HEADER_GC_CHAT.value(), 19, 0, 1, 0, 0, 0, 0, 3, 1];
        expected.extend_from_slice(b"[LS;1097]");
        assert_eq!(line, expected);
        let room = refusal_notice(&MoveRefused::NoRoomInInventory, to(1)).expect("it speaks");
        assert_eq!(&room[10..], b"Nu ai spatiu suficient in inventar.");
        assert_eq!(refusal_notice(&MoveRefused::NoRoom, to(1)), None);
        assert_eq!(
            refusal_notice(&MoveRefused::NotPorted(Unported::Equipment), to(1)),
            None
        );
    }
}
