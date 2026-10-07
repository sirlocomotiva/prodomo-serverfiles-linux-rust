//! The safebox and the mall of an online character (`CHARACTER::ReqSafeboxLoad`,
//! `LoadSafebox`, `CloseSafebox`, `LoadMall`, `CloseMall`, `char.cpp:7051-7280`).
//!
//! The stored rows and the password live in the store (ADR-0005), so the connection task
//! checks the password and loads the rows between [`SafeboxStep::BeginOpen`] and
//! [`SafeboxStep::Open`]; this module keeps what legacy keeps on the character: the open
//! windows, the pending load and the pulse of the last load (`m_iSafeboxLoadTime`,
//! `m_iMallLoadTime`). A safebox that is open refuses a trade and a shop
//! ([`GameState::safebox_open`]), and an open trade or shop refuses a safebox that loads. For
//! [`LOAD_WAIT_PULSES`](crate::game_state::LOAD_WAIT_PULSES) after its safebox loads or closes
//! a character trades nothing (`GameState::safebox_loaded_recently`); the mall's loads do not
//! count.
//!
//! `IsHack` (`char.cpp:8234-8241`) reads the load time too, for a warp NPC (`warp_npc`).
//!
//! # Not ported
//!
//! The other checks that read the safebox's load time: `IsHack` as `do_cmd` and `do_restart`
//! call it, `CanWarp` (`:8788-8793`) and the summoning items (`char_item.cpp:7254-7261`). None of
//! their commands or items is ported.

use common::item_slots::usable_inventory_cells;
use common::vid::Vid;
use gamedata::locale_string::LocaleStrings;
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};
use protocol::gc_inventory::{
    HEADER_GC_MALL_OPEN, HEADER_GC_SAFEBOX_SIZE, HEADER_GC_SAFEBOX_WRONG_PASSWORD,
};
use protocol::gc_safebox::StoreWindow;
use protocol::gc_small::{GcHeaderAndByte, GcHeaderOnly};
use protocol::item_pos::ItemPos;
use world::character::{
    checkin, checkout, move_stored, sync_quickslots, MoveRules, Safebox, SafeboxRefused,
    StoreChange, MALL_ROWS, SAFEBOX_ROWS,
};
use world::item::Item;

use super::shop::notice;
use super::GameState;
use crate::chat_line::{chat_packet, Arg};
use crate::item_move::{belt_grade, store_record, MoveItemRefused, MovedItems, Mover};
use db::items::{AccountChange, SAFEBOX};

/// The pulses between two loads of one window (`char.cpp:7075`, `cmd_general.cpp:1034`: 10 s).
pub const LOAD_WAIT_PULSES: u64 = 250;

/// The safebox is open already (`char.cpp:7069`, `cmd_general.cpp:1030`).
pub const ALREADY_OPEN_NOTICE: &str = "[LS;527]";
/// The safebox was loaded within [`LOAD_WAIT_PULSES`] (`char.cpp:7077`).
pub const SAFEBOX_WAIT_NOTICE: &str = "[LS;828]";
/// The mall was loaded within [`LOAD_WAIT_PULSES`] (`cmd_general.cpp:1036`).
pub const MALL_WAIT_NOTICE: &str = "[LS;528]";
/// A trade or a shop is open when the safebox arrives (`input_db.cpp:1140`).
pub const OTHER_WINDOW_NOTICE: &str = "[LS;773]";
/// A trade step within [`LOAD_WAIT_PULSES`] of a safebox load or close
/// (`input_main.cpp:1377-1397`). Legacy waits `g_nPortalLimitTime` there
/// (`char_item.cpp:7166`), which is 10 seconds as well.
pub const TRADE_WAIT_NOTICE: &[u8] = b"[LS;661;%d]";
/// The seconds [`TRADE_WAIT_NOTICE`] names.
pub const TRADE_WAIT_SECONDS: i64 = 10;

/// The windows of one character.
#[derive(Debug, Default)]
pub(super) struct Storage {
    safebox: Option<Safebox>,
    mall: Option<Safebox>,
    opening: bool,
    load_time: Option<u64>,
    mall_load_time: Option<u64>,
}

/// One safebox or mall step of a character.
#[derive(Debug)]
pub enum SafeboxStep {
    /// `ReqSafeboxLoad` before the password is checked.
    BeginOpen,
    /// The password was wrong (`HEADER_GC_SAFEBOX_WRONG_PASSWORD`).
    WrongPassword {
        /// Whether the mall asked.
        mall: bool,
    },
    /// The account's safebox rows arrived (`LoadSafebox`).
    Open {
        /// The account.
        account: u32,
        /// Its stored items.
        items: Vec<Item>,
    },
    /// `do_safebox_close`.
    Close,
    /// `do_mall_password` before the password is checked.
    BeginMall,
    /// The account's mall rows arrived (`LoadMall`).
    OpenMall {
        /// The account.
        account: u32,
        /// Its stored items.
        items: Vec<Item>,
    },
    /// `do_mall_close`.
    CloseMall,
    /// `HEADER_CG_SAFEBOX_CHECKIN`.
    Checkin {
        /// The inventory cell.
        from: ItemPos,
        /// The safebox cell.
        safe_pos: u32,
    },
    /// `HEADER_CG_SAFEBOX_CHECKOUT` or `HEADER_CG_MALL_CHECKOUT`.
    Checkout {
        /// The stored cell.
        safe_pos: u32,
        /// The inventory cell.
        to: ItemPos,
        /// Whether the mall is the source.
        mall: bool,
    },
    /// `HEADER_CG_SAFEBOX_ITEM_MOVE`.
    Move {
        /// The source cell.
        from: u32,
        /// The destination cell.
        to: u32,
        /// The count to move.
        count: u16,
    },
}

/// What a safebox step asks of the connection.
#[derive(Debug)]
pub enum SafeboxAnswer {
    /// Send these records and store nothing.
    Sent(Vec<Vec<u8>>),
    /// Check the password and load the rows.
    Proceed,
    /// Store this move, then send its records.
    Moved(MovedItems),
    /// Store these account rows, then send the records.
    Rearranged {
        /// The account.
        account: u32,
        /// The row changes.
        changes: Vec<AccountChange>,
        /// The records.
        records: Vec<Vec<u8>>,
    },
}

impl GameState {
    /// Run one safebox step for the character online under `vid`.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when no character is online under `vid`.
    pub fn safebox(
        &mut self,
        vid: Vid,
        step: SafeboxStep,
        mover: Mover,
    ) -> Result<SafeboxAnswer, MoveItemRefused> {
        let owner_id = match self.characters.find_by_vid(vid) {
            Ok(character) => character.player_id(),
            Err(_) => return Err(MoveItemRefused::NoSuchCharacter { vid }),
        };
        let pulse = self.last_pulse;
        let locale = &self.locale;
        let storage = self.storages.entry(vid).or_default();
        let answer = match step {
            SafeboxStep::BeginOpen => begin(storage, pulse, false, mover, locale),
            SafeboxStep::BeginMall => begin(storage, pulse, true, mover, locale),
            SafeboxStep::WrongPassword { mall } => {
                if !mall {
                    storage.opening = false;
                }
                sent(|frame| {
                    GcHeaderOnly::new(HEADER_GC_SAFEBOX_WRONG_PASSWORD.value()).encode_into(frame);
                })
            }
            SafeboxStep::Close => close(storage, pulse, false, mover, locale),
            SafeboxStep::CloseMall => close(storage, pulse, true, mover, locale),
            SafeboxStep::Open { account, items } => {
                storage.opening = false;
                if self.trading.contains_key(&vid) || self.browsing.contains_key(&vid) {
                    SafeboxAnswer::Sent(vec![notice(OTHER_WINDOW_NOTICE, mover, locale)])
                } else {
                    open(&mut storage.safebox, StoreWindow::Safebox, account, items)
                }
            }
            SafeboxStep::OpenMall { account, items } => {
                open(&mut storage.mall, StoreWindow::Mall, account, items)
            }
            SafeboxStep::Move { from, to, count } => {
                let limit = self.item_count_limit;
                match storage.safebox.as_mut() {
                    None => SafeboxAnswer::Sent(Vec::new()),
                    Some(safebox) => match move_stored(safebox, from, to, count, limit) {
                        Ok(shifted) => {
                            let records = MovedItems::new(
                                owner_id,
                                vid.raw(),
                                shifted.done,
                                mover,
                                &self.protos,
                                locale,
                            )
                            .records;
                            rearranged(safebox.account(), shifted.changes, records)
                        }
                        Err(refused) => refusal(&refused, mover, locale),
                    },
                }
            }
            SafeboxStep::Checkin { from, safe_pos } => {
                // `input_main.cpp:2278`: a character whose script waits stores nothing, silently.
                if self.quest_running(vid) {
                    return Ok(SafeboxAnswer::Sent(Vec::new()));
                }
                return Ok(self.carry(vid, None, from, safe_pos, mover));
            }
            SafeboxStep::Checkout { safe_pos, to, mall } => {
                return Ok(self.carry(vid, Some(mall), to, safe_pos, mover));
            }
        };
        Ok(answer)
    }

    /// Whether the character under `vid` has its safebox open (`IsOpenSafebox`).
    pub(super) fn safebox_open(&self, vid: Vid) -> bool {
        self.storages
            .get(&vid)
            .is_some_and(|storage| storage.safebox.is_some())
    }

    /// Whether the safebox of the character under `vid` loaded or closed within
    /// [`LOAD_WAIT_PULSES`] (`m_iSafeboxLoadTime`), which holds back its trades. The mall's
    /// loads do not count, and a character whose safebox never loaded does not wait.
    pub(super) fn safebox_loaded_recently(&self, vid: Vid) -> bool {
        self.storages
            .get(&vid)
            .and_then(|storage| storage.load_time)
            .is_some_and(|at| self.last_pulse.saturating_sub(at) < LOAD_WAIT_PULSES)
    }

    /// [`TRADE_WAIT_NOTICE`] for `mover`.
    pub(super) fn trade_wait_line(&self, mover: Mover) -> Vec<u8> {
        let seconds = [Arg::Int(TRADE_WAIT_SECONDS)];
        let recipient = mover.recipient(&self.locale);
        chat_packet(recipient, CHAT_TYPE_INFO, TRADE_WAIT_NOTICE, &seconds)
    }

    /// Forget the windows of the character under `vid`, as its departure does.
    pub(super) fn close_storage(&mut self, vid: Vid) {
        let _storage = self.storages.remove(&vid);
    }

    /// A checkin (`mall` is `None`) or a checkout between `cell` and `safe_pos`.
    fn carry(
        &mut self,
        vid: Vid,
        mall: Option<bool>,
        cell: ItemPos,
        safe_pos: u32,
        mover: Mover,
    ) -> SafeboxAnswer {
        let questing = self.quest_running(vid);
        let Some(storage) = self.storages.get_mut(&vid) else {
            return SafeboxAnswer::Sent(Vec::new());
        };
        let window = if mall == Some(true) {
            storage.mall.as_mut()
        } else {
            storage.safebox.as_mut()
        };
        let (Some(safebox), Ok(character)) = (window, self.characters.find_by_vid_mut(vid)) else {
            return SafeboxAnswer::Sent(Vec::new());
        };
        let rules = MoveRules {
            count_limit: self.item_count_limit,
            usable_cells: usable_inventory_cells(character.inven_point()),
            belt_grade: belt_grade(character.items(), &self.protos),
            questing,
        };
        let done = if mall.is_none() {
            checkin(
                safebox,
                character.items_mut(),
                cell,
                safe_pos,
                rules,
                &self.protos,
            )
        } else {
            let items = character.items_mut();
            checkout(
                safebox,
                items,
                safe_pos,
                cell,
                rules,
                &self.protos,
                &mut self.dice,
            )
        };
        let mut done = match done {
            Ok(done) => done,
            Err(refused) => return refusal(&refused, mover, &self.locale),
        };
        let (items, slots) = character.items_and_quickslots_mut();
        sync_quickslots(&mut done, slots, items);
        let quickslots = slots.clone();
        let owner_id = character.player_id();
        let raw = character.vid().raw();
        let mut carried = MovedItems::new(owner_id, raw, done, mover, &self.protos, &self.locale);
        carried.quickslots = Some(quickslots);
        SafeboxAnswer::Moved(carried)
    }
}

/// `ReqSafeboxLoad` (`char.cpp:7051-7100`) or the mall's half of `do_mall_password`.
fn begin(
    storage: &mut Storage,
    pulse: u64,
    mall: bool,
    mover: Mover,
    locale: &LocaleStrings,
) -> SafeboxAnswer {
    let (open, loaded, wait) = if mall {
        (
            storage.mall.is_some(),
            storage.mall_load_time,
            MALL_WAIT_NOTICE,
        )
    } else {
        (
            storage.safebox.is_some(),
            storage.load_time,
            SAFEBOX_WAIT_NOTICE,
        )
    };
    if open {
        return SafeboxAnswer::Sent(vec![notice(ALREADY_OPEN_NOTICE, mover, locale)]);
    }
    if loaded.is_some_and(|at| pulse.saturating_sub(at) < LOAD_WAIT_PULSES) {
        return SafeboxAnswer::Sent(vec![notice(wait, mover, locale)]);
    }
    if mall {
        storage.mall_load_time = Some(pulse);
    } else {
        if storage.opening {
            tracing::debug!("a safebox load is pending already");
            return SafeboxAnswer::Sent(Vec::new());
        }
        storage.load_time = Some(pulse);
        storage.opening = true;
    }
    SafeboxAnswer::Proceed
}

/// `CloseSafebox` or `CloseMall`, with the client's `CloseSafebox` or `CloseMall` command.
fn close(
    storage: &mut Storage,
    pulse: u64,
    mall: bool,
    mover: Mover,
    locale: &LocaleStrings,
) -> SafeboxAnswer {
    let (window, command): (&mut Option<Safebox>, &[u8]) = if mall {
        (&mut storage.mall, b"CloseMall")
    } else {
        (&mut storage.safebox, b"CloseSafebox")
    };
    if window.take().is_none() {
        return SafeboxAnswer::Sent(Vec::new());
    }
    if mall {
        storage.mall_load_time = Some(pulse);
    } else {
        storage.load_time = Some(pulse);
        storage.opening = false;
    }
    let line = chat_packet(mover.recipient(locale), CHAT_TYPE_COMMAND, command, &[]);
    SafeboxAnswer::Sent(vec![line])
}

/// `LoadSafebox` or `LoadMall`: the size record and a SET for each stored item.
fn open(
    slot: &mut Option<Safebox>,
    window: StoreWindow,
    account: u32,
    items: Vec<Item>,
) -> SafeboxAnswer {
    if slot.is_some() {
        return SafeboxAnswer::Sent(Vec::new());
    }
    let (rows, header) = match window {
        StoreWindow::Safebox => (SAFEBOX_ROWS, HEADER_GC_SAFEBOX_SIZE),
        StoreWindow::Mall => (MALL_ROWS, HEADER_GC_MALL_OPEN),
    };
    let (safebox, skipped) = Safebox::open(window, rows, account, items);
    for item in &skipped {
        tracing::warn!(id = item.id, pos = ?item.pos, "a stored item has no free cell; skipped");
    }
    let mut size = Vec::new();
    GcHeaderAndByte::new(header.value(), rows).encode_into(&mut size);
    let mut records = vec![size];
    records.extend(safebox.records().into_iter().map(store_record));
    *slot = Some(safebox);
    SafeboxAnswer::Sent(records)
}

/// A move inside the safebox as account rows and records.
fn rearranged(account: u32, changes: Vec<StoreChange>, records: Vec<Vec<u8>>) -> SafeboxAnswer {
    let changes = changes
        .into_iter()
        .map(|change| match change {
            StoreChange::Moved { id, pos } => AccountChange::Moved {
                id,
                window_type: SAFEBOX,
                pos,
            },
            StoreChange::Count { id, count } => AccountChange::Count { id, count },
            StoreChange::Destroyed { id } => AccountChange::Destroyed { id },
        })
        .collect();
    SafeboxAnswer::Rearranged {
        account,
        changes,
        records,
    }
}

/// The line a refusal shows, or nothing for a silent one.
fn refusal(refused: &SafeboxRefused, mover: Mover, locale: &LocaleStrings) -> SafeboxAnswer {
    if let Some(text) = refused.notice() {
        return SafeboxAnswer::Sent(vec![notice(text, mover, locale)]);
    }
    tracing::debug!(%refused, "a safebox step was refused");
    SafeboxAnswer::Sent(Vec::new())
}

/// One record as its own frame.
fn sent(encode: impl FnOnce(&mut Vec<u8>)) -> SafeboxAnswer {
    let mut frame = Vec::new();
    encode(&mut frame);
    SafeboxAnswer::Sent(vec![frame])
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::item_slots::EWindows;
    use db::items::{RowChange, MALL};
    use gamedata::item_proto::ItemProtos;
    use protocol::gc_chat::CHAT_TYPE_INFO;
    use protocol::gc_inventory::{HEADER_GC_MALL_SET, HEADER_GC_SAFEBOX_SET};
    use protocol::gc_safebox::GcStoreItemSet;
    use world::character::{MoveKind, Side};
    use world::item::ITEM_FLAG_STACKABLE;

    use crate::client_registry::ClientOutbox;

    const HOLDER: Vid = Vid::new(7);
    const ACCOUNT: u32 = 3;
    /// The small red potion, stackable.
    const POTION: u32 = 27_001;
    /// A pulse well past the start, so a wait is measured from a load.
    const NOW: u64 = 10_000;

    fn a_holder(items: &[(ItemPos, Item)]) -> GameState {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy");
        let protos = ItemProtos::load(&root.join("gamedata/proto")).unwrap();
        let mut state = GameState::new(protos);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        state
            .enter_world_with_items(HOLDER, 70, "Keeper", items, ClientOutbox::new(tx))
            .unwrap();
        state.last_pulse = NOW;
        state
    }

    const fn of(empire: u8) -> Mover {
        Mover {
            recently_fought: false,
            empire,
            language: 1,
            pk_mode: crate::loading_phase::PK_MODE_PEACE,
        }
    }

    fn step(state: &mut GameState, step: SafeboxStep) -> SafeboxAnswer {
        state.safebox(HOLDER, step, of(1)).unwrap()
    }

    fn sent(answer: SafeboxAnswer) -> Vec<Vec<u8>> {
        match answer {
            SafeboxAnswer::Sent(records) => records,
            other => panic!("nothing was only sent: {other:?}"),
        }
    }

    fn moved(answer: SafeboxAnswer) -> MovedItems {
        match answer {
            SafeboxAnswer::Moved(moved) => moved,
            other => panic!("nothing moved: {other:?}"),
        }
    }

    fn line(state: &GameState, chat_type: u8, text: &str) -> Vec<u8> {
        chat_packet(
            of(1).recipient(&state.locale),
            chat_type,
            text.as_bytes(),
            &[],
        )
    }

    fn info(state: &GameState, text: &str) -> Vec<Vec<u8>> {
        vec![line(state, CHAT_TYPE_INFO, text)]
    }

    fn inventory(cell: u16) -> ItemPos {
        ItemPos::new(EWindows::Inventory as u8, cell)
    }

    fn potions(id: u32, count: u16) -> Item {
        let mut item = Item::new(id, POTION);
        item.set_size(1).unwrap();
        item.flags = ITEM_FLAG_STACKABLE;
        item.count = count;
        item
    }

    /// `potions` stored in the given window and cell, as the store loads them.
    fn stored(id: u32, count: u16, window: u8, pos: u16) -> Item {
        let mut item = potions(id, count);
        item.pos = ItemPos::new(window, pos);
        item
    }

    fn open_empty(state: &mut GameState) {
        assert!(matches!(
            step(state, SafeboxStep::BeginOpen),
            SafeboxAnswer::Proceed
        ));
        let account = ACCOUNT;
        let _records = sent(step(
            state,
            SafeboxStep::Open {
                account,
                items: vec![],
            },
        ));
        assert!(state.safebox_open(HOLDER));
    }

    fn wrong_password() -> Vec<Vec<u8>> {
        vec![vec![HEADER_GC_SAFEBOX_WRONG_PASSWORD.value()]]
    }

    #[test]
    fn a_character_that_is_not_online_is_refused() {
        let mut state = a_holder(&[]);
        let nobody = Vid::new(99);
        assert!(matches!(
            state.safebox(nobody, SafeboxStep::BeginOpen, of(1)),
            Err(MoveItemRefused::NoSuchCharacter { vid }) if vid == nobody
        ));
    }

    #[test]
    fn a_character_that_never_loaded_opens_at_once() {
        let mut state = a_holder(&[]);
        state.last_pulse = 0;
        assert!(matches!(
            step(&mut state, SafeboxStep::BeginOpen),
            SafeboxAnswer::Proceed
        ));
        let mut state = a_holder(&[]);
        state.last_pulse = 0;
        assert!(matches!(
            step(&mut state, SafeboxStep::BeginMall),
            SafeboxAnswer::Proceed
        ));
    }

    #[test]
    fn one_load_is_pending_at_a_time_and_the_next_waits_ten_seconds() {
        let mut state = a_holder(&[]);
        assert!(matches!(
            step(&mut state, SafeboxStep::BeginOpen),
            SafeboxAnswer::Proceed
        ));
        state.last_pulse = NOW + LOAD_WAIT_PULSES;
        assert!(
            sent(step(&mut state, SafeboxStep::BeginOpen)).is_empty(),
            "a pending load answers nothing"
        );
        let mall = false;
        let answer = step(&mut state, SafeboxStep::WrongPassword { mall });
        assert_eq!(sent(answer), wrong_password());
        state.last_pulse = NOW + LOAD_WAIT_PULSES - 1;
        let expected = info(&state, SAFEBOX_WAIT_NOTICE);
        assert_eq!(sent(step(&mut state, SafeboxStep::BeginOpen)), expected);
        state.last_pulse = NOW + LOAD_WAIT_PULSES;
        assert!(matches!(
            step(&mut state, SafeboxStep::BeginOpen),
            SafeboxAnswer::Proceed
        ));
    }

    #[test]
    fn an_open_safebox_sends_its_size_and_items_and_refuses_a_second_open() {
        let mut state = a_holder(&[]);
        let items = vec![
            stored(41, 5, SAFEBOX, 4),
            stored(42, 7, SAFEBOX, 0),
            stored(43, 1, SAFEBOX, 45),
        ];
        assert!(matches!(
            step(&mut state, SafeboxStep::BeginOpen),
            SafeboxAnswer::Proceed
        ));
        let account = ACCOUNT;
        let records = sent(step(&mut state, SafeboxStep::Open { account, items }));
        assert_eq!(
            records.len(),
            3,
            "the size and the two items in its 45 cells"
        );
        assert_eq!(
            records[0],
            vec![HEADER_GC_SAFEBOX_SIZE.value(), SAFEBOX_ROWS]
        );
        let cells: Vec<(u8, u16, u32, u16, u8)> = records[1..]
            .iter()
            .map(|record| {
                assert_eq!(record[0], HEADER_GC_SAFEBOX_SET.value());
                let set = GcStoreItemSet::decode(record).unwrap();
                let item = set.item;
                (
                    item.cell.window_type,
                    item.cell.cell,
                    item.vnum,
                    item.count,
                    item.highlight,
                )
            })
            .collect();
        assert_eq!(cells, vec![(3, 0, POTION, 7, 0), (3, 4, POTION, 5, 0)]);
        assert!(state.safebox_open(HOLDER));
        let expected = info(&state, ALREADY_OPEN_NOTICE);
        assert_eq!(sent(step(&mut state, SafeboxStep::BeginOpen)), expected);
        let again = SafeboxStep::Open {
            account,
            items: vec![],
        };
        assert!(
            sent(step(&mut state, again)).is_empty(),
            "a second load is dropped"
        );
    }

    #[test]
    fn a_safebox_that_arrives_during_a_trade_is_not_opened() {
        let mut state = a_holder(&[]);
        assert!(matches!(
            step(&mut state, SafeboxStep::BeginOpen),
            SafeboxAnswer::Proceed
        ));
        let _trade = state.trading.insert(HOLDER, (1, Side::Starter));
        let account = ACCOUNT;
        let answer = step(
            &mut state,
            SafeboxStep::Open {
                account,
                items: vec![],
            },
        );
        assert_eq!(sent(answer), info(&state, OTHER_WINDOW_NOTICE));
        assert!(!state.safebox_open(HOLDER));
        let _trade = state.trading.remove(&HOLDER);
        state.last_pulse = NOW + LOAD_WAIT_PULSES;
        assert!(
            matches!(
                step(&mut state, SafeboxStep::BeginOpen),
                SafeboxAnswer::Proceed
            ),
            "the refused load is no longer pending"
        );
    }

    #[test]
    fn a_close_sends_the_command_and_restarts_the_wait() {
        let mut state = a_holder(&[]);
        assert!(sent(step(&mut state, SafeboxStep::Close)).is_empty());
        open_empty(&mut state);
        state.last_pulse = NOW + 2 * LOAD_WAIT_PULSES;
        let expected = vec![line(&state, CHAT_TYPE_COMMAND, "CloseSafebox")];
        assert_eq!(sent(step(&mut state, SafeboxStep::Close)), expected);
        assert!(!state.safebox_open(HOLDER));
        let expected = info(&state, SAFEBOX_WAIT_NOTICE);
        assert_eq!(sent(step(&mut state, SafeboxStep::BeginOpen)), expected);
        assert!(sent(step(&mut state, SafeboxStep::Close)).is_empty());
    }

    #[test]
    fn the_mall_opens_27_rows_and_is_not_the_safebox() {
        let mut state = a_holder(&[]);
        assert!(matches!(
            step(&mut state, SafeboxStep::BeginMall),
            SafeboxAnswer::Proceed
        ));
        let expected = info(&state, MALL_WAIT_NOTICE);
        assert_eq!(sent(step(&mut state, SafeboxStep::BeginMall)), expected);
        let mall = true;
        let answer = step(&mut state, SafeboxStep::WrongPassword { mall });
        assert_eq!(sent(answer), wrong_password());
        let account = ACCOUNT;
        let items = vec![stored(51, 2, MALL, 130)];
        let records = sent(step(&mut state, SafeboxStep::OpenMall { account, items }));
        assert_eq!(records[0], vec![HEADER_GC_MALL_OPEN.value(), MALL_ROWS]);
        assert_eq!(records.len(), 2);
        assert_eq!(records[1][0], HEADER_GC_MALL_SET.value());
        let set = GcStoreItemSet::decode(&records[1]).unwrap();
        assert_eq!((set.item.cell.window_type, set.item.cell.cell), (MALL, 130));
        assert!(
            !state.safebox_open(HOLDER),
            "IsOpenSafebox is the safebox only"
        );
        let expected = info(&state, ALREADY_OPEN_NOTICE);
        assert_eq!(sent(step(&mut state, SafeboxStep::BeginMall)), expected);
        // A close long after the load starts the mall's wait again (`do_mall_close`,
        // `G/cmd_general.cpp:1050-1058`), and not the safebox's.
        state.last_pulse = NOW + 2 * LOAD_WAIT_PULSES;
        let expected = vec![line(&state, CHAT_TYPE_COMMAND, "CloseMall")];
        assert_eq!(sent(step(&mut state, SafeboxStep::CloseMall)), expected);
        assert!(sent(step(&mut state, SafeboxStep::CloseMall)).is_empty());
        let expected = info(&state, MALL_WAIT_NOTICE);
        assert_eq!(sent(step(&mut state, SafeboxStep::BeginMall)), expected);
        assert!(matches!(
            step(&mut state, SafeboxStep::BeginOpen),
            SafeboxAnswer::Proceed
        ));
    }

    #[test]
    fn a_step_without_its_window_answers_nothing() {
        let mut state = a_holder(&[(inventory(0), potions(61, 3))]);
        let checkin = SafeboxStep::Checkin {
            from: inventory(0),
            safe_pos: 0,
        };
        assert!(sent(step(&mut state, checkin)).is_empty());
        let (from, to, count) = (0, 1, 0);
        assert!(sent(step(&mut state, SafeboxStep::Move { from, to, count })).is_empty());
        open_empty(&mut state);
        let from_mall = SafeboxStep::Checkout {
            safe_pos: 0,
            to: inventory(5),
            mall: true,
        };
        assert!(
            sent(step(&mut state, from_mall)).is_empty(),
            "the mall is not open"
        );
    }

    /// `SafeboxCheckin` stores nothing, silently, while a script of the character waits
    /// (`G/input_main.cpp:2278`), and stores again once it ends.
    #[test]
    fn a_character_whose_script_waits_stores_nothing() {
        let mut state = a_holder(&[(inventory(2), potions(61, 3))]);
        open_empty(&mut state);
        state.start_a_quest(HOLDER);
        let checkin = SafeboxStep::Checkin {
            from: inventory(2),
            safe_pos: 6,
        };
        assert!(sent(step(&mut state, checkin)).is_empty());
        state.end_the_quest(HOLDER);
        let checkin = SafeboxStep::Checkin {
            from: inventory(2),
            safe_pos: 6,
        };
        assert_eq!(moved(step(&mut state, checkin)).kind, MoveKind::Stored);
    }

    #[test]
    fn a_checkin_and_a_checkout_carry_the_item_and_its_row() {
        let mut state = a_holder(&[(inventory(2), potions(61, 3))]);
        open_empty(&mut state);
        let checkin = SafeboxStep::Checkin {
            from: inventory(2),
            safe_pos: 6,
        };
        let stored = moved(step(&mut state, checkin));
        assert_eq!(stored.kind, MoveKind::Stored);
        assert_eq!(stored.owner_id, 70);
        let (id, account) = (61, ACCOUNT);
        assert_eq!(
            stored.changes,
            vec![RowChange::Stored {
                id,
                account,
                pos: 6
            }]
        );
        assert!(stored.quickslots.is_some());
        let set: Vec<&Vec<u8>> = stored
            .records
            .iter()
            .filter(|record| record[0] == HEADER_GC_SAFEBOX_SET.value())
            .collect();
        assert_eq!(set.len(), 1, "one SAFEBOX_SET: {:?}", stored.records);
        let checkout = SafeboxStep::Checkout {
            safe_pos: 6,
            to: inventory(9),
            mall: false,
        };
        let retrieved = moved(step(&mut state, checkout));
        assert_eq!(retrieved.kind, MoveKind::Retrieved);
        let window_type = EWindows::Inventory as u8;
        let expected = RowChange::Retrieved {
            id,
            account,
            window_type,
            pos: 9,
        };
        assert_eq!(retrieved.changes, vec![expected]);
        let character = state.characters.find_by_vid(HOLDER).unwrap();
        assert_eq!(
            character.items().item_at(inventory(9)).map(|item| item.id),
            Some(61)
        );
    }

    #[test]
    fn a_refused_checkin_shows_its_notice_and_moves_nothing() {
        let items = [
            (inventory(0), potions(61, 3)),
            (inventory(1), potions(62, 4)),
        ];
        let mut state = a_holder(&items);
        open_empty(&mut state);
        let first = SafeboxStep::Checkin {
            from: inventory(0),
            safe_pos: 0,
        };
        let _stored = moved(step(&mut state, first));
        let onto = SafeboxStep::Checkin {
            from: inventory(1),
            safe_pos: 0,
        };
        let expected = info(&state, "[LS;666]");
        assert_eq!(sent(step(&mut state, onto)), expected);
        let character = state.characters.find_by_vid(HOLDER).unwrap();
        assert!(character.items().item_at(inventory(1)).is_some());
    }

    #[test]
    fn a_move_inside_the_safebox_rearranges_account_rows() {
        let mut state = a_holder(&[]);
        assert!(matches!(
            step(&mut state, SafeboxStep::BeginOpen),
            SafeboxAnswer::Proceed
        ));
        let account = ACCOUNT;
        let items = vec![stored(71, 5, SAFEBOX, 0)];
        let _records = sent(step(&mut state, SafeboxStep::Open { account, items }));
        let (from, to, count) = (0, 7, 0);
        match step(&mut state, SafeboxStep::Move { from, to, count }) {
            SafeboxAnswer::Rearranged {
                account,
                changes,
                records,
            } => {
                assert_eq!(account, ACCOUNT);
                let window_type = SAFEBOX;
                let moved = AccountChange::Moved {
                    id: 71,
                    window_type,
                    pos: 7,
                };
                assert_eq!(changes, vec![moved]);
                assert!(!records.is_empty());
            }
            other => panic!("nothing was rearranged: {other:?}"),
        }
    }

    #[test]
    fn a_move_onto_taken_cells_sends_nothing() {
        let mut state = a_holder(&[]);
        assert!(matches!(
            step(&mut state, SafeboxStep::BeginOpen),
            SafeboxAnswer::Proceed
        ));
        let account = ACCOUNT;
        let mut other = stored(72, 5, SAFEBOX, 1);
        other.vnum = POTION + 1;
        let items = vec![stored(71, 5, SAFEBOX, 0), other];
        let _records = sent(step(&mut state, SafeboxStep::Open { account, items }));
        // Onto another item, and onto the item's own cell (`G/safebox.cpp:226-227`).
        for (from, to) in [(0, 1), (1, 1)] {
            let count = 0;
            let answer = step(&mut state, SafeboxStep::Move { from, to, count });
            assert!(sent(answer).is_empty(), "{from} -> {to}");
        }
    }

    #[test]
    fn leaving_the_world_forgets_both_windows() {
        let mut state = a_holder(&[]);
        open_empty(&mut state);
        let _kept = state.leave_world(HOLDER);
        assert!(!state.safebox_open(HOLDER));
        assert!(state.storages.is_empty());
    }
}
