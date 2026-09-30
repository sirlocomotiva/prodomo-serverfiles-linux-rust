//! Trades between players: `CInputMain::Exchange` (`G/input_main.cpp:1367-1527`),
//! `CHARACTER::ExchangeStart` (`G/exchange.cpp:77-152`), and the cancels legacy runs from
//! elsewhere.
//!
//! The world keeps each open trade with the two characters in it, as legacy's `CHARACTER`
//! keeps `m_pkExchange`. The offers, the accepts and the settlement are
//! [`world::character::Trade`]'s and [`world::character::settle`]'s; this is the manager around
//! them, which finds the other player, measures the distance, checks the other windows and
//! sends each side what it is told. Records to the side that did not send the step go through
//! its client's outbox; the sender's are answered.
//!
//! # The order
//!
//! First the safebox wait (`G/input_main.cpp:1377-1397`): a `START` whose VID names another
//! player on the map whose safebox loaded or closed within
//! [`LOAD_WAIT_PULSES`](crate::game_state::LOAD_WAIT_PULSES) tells that player the wait line
//! and the sender nothing, and any step of a character whose own safebox did is refused with
//! that line. The mall's loads do not count.
//!
//! A `START` is ignored while the character trades already, and when the VID names nobody on
//! its map. Then gold at `GOLD_MAX_MAX` is refused with the yang-limit line, and a browsed shop
//! or an open safebox with the other-transaction line; then `ExchangeStart` ignores the
//! character itself and an NPC, refuses a player who browses a shop or has its safebox open
//! with the busy line, ignores one `EXCHANGE_MAX_DISTANCE` or farther away, and answers
//! `ALREADY` when that player trades already. Otherwise both windows open.
//!
//! An offer (`ITEM_ADD`, `ITEM_DEL`, `ELK_ADD`) runs only while the other side does not accept.
//! The accept that completes the trade settles it: the store writes both sides' rows in one
//! transaction (ADR-0003), then each side is sent its moves, the completed line and `END`; a
//! settlement that fails tells each side its line and ends the trade.
//!
//! A trade ends for both sides on `CANCEL`, when either character leaves the world
//! (`CHARACTER::Destroy`), and when the two stand `EXCHANGE_MAX_DISTANCE` or farther apart.
//!
//! # Divergences
//!
//! - **The distance.** Legacy measures it in `StateMove` every 16 Pulses while the character
//!   that holds the exchange moves (`G/char.cpp:6962-6973`), so a trade survives the other side
//!   walking away as long as this one stands still. The Rewrite measures every trade every 16
//!   Pulses, and a side that is no longer on the trade's map ends it too.
//! - **A player on another map.** `CHARACTER_MANAGER::Find` looks a VID up across every map of
//!   the process and `DISTANCE_APPROX` ignores the map, so a client naming a player on another
//!   map at the same coordinates starts a trade with it. Only a modified client names one: a
//!   Defect. The Rewrite looks on the character's own Channel and map.
//! - **The partner of every step.** `CInputMain::Exchange` looks up `arg1` for every subheader,
//!   although only `START` names a character (a Defect): the step is ignored when that
//!   character waits after a safebox load, which tells it the wait line, or is dead. The
//!   Rewrite checks the wait of the player a `START` asks alone. Nobody dies yet, so no step is
//!   ignored for a dead character.
//! - **A safebox never loaded.** Legacy's load time starts at 0 (`G/char.cpp:278`), so every
//!   trade step waits for the first 10 seconds after the process starts. A character whose
//!   safebox never loaded does not wait in the Rewrite.
//! - **A failed store.** The world settles first and the store writes after, as every item move
//!   does. When the write fails the character whose accept completed the trade is disconnected,
//!   and its partner keeps the world's side of the trade until it relogs.
//!
//! # Not ported
//!
//! The quest check on `START` (`GiveItemToPC`), the spectator check, `IsSecured`, the
//! exchange-block mode, the personal shop, the cube and the aura window are not in the Rewrite,
//! so none refuses a trade. An open safebox refuses as an open shop does (`exchange.cpp:92-98`).
//! The mall does not: `IsOpenSafebox` is the safebox only.

use common::item_slots::usable_inventory_cells;
use common::vid::Vid;
use gamedata::item_proto::ItemProtos;
use gamedata::locale_string::LocaleStrings;
use protocol::cg_exchange::CgExchange;
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::item_pos::ItemPos;
use world::character::{
    settle, Accepted, Character, MoveDone, Said, Settled, Side, Trade, TradeRecord, Trader,
    COMPLETED_NOTICE, EXCHANGE_MAX_DISTANCE, GOLD_MAX_MAX, OTHER_TRANSACTION_NOTICE,
    PARTNER_BUSY_NOTICE, YANG_LIMIT_NOTICE,
};

use super::shop::notice;
use super::GameState;
use crate::chat_line::{chat_packet, Arg};
use crate::client_registry::{ChannelClients, ClientOutbox, PositionTable};
use crate::game_loop_messages::GroundPlace;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};
use crate::sync_position::distance_approx;

/// `EXCHANGE_SUBHEADER_CG_START` (`G/packet.h:716`): arg1 is the VID asked.
pub const EXCHANGE_SUBHEADER_CG_START: u8 = 0;

/// `EXCHANGE_SUBHEADER_CG_ITEM_ADD`: `Pos` is the item, arg2 its display cell.
pub const EXCHANGE_SUBHEADER_CG_ITEM_ADD: u8 = 1;

/// `EXCHANGE_SUBHEADER_CG_ITEM_DEL`: arg1 is the slot.
pub const EXCHANGE_SUBHEADER_CG_ITEM_DEL: u8 = 2;

/// `EXCHANGE_SUBHEADER_CG_ELK_ADD`: arg1 is the gold.
pub const EXCHANGE_SUBHEADER_CG_ELK_ADD: u8 = 3;

/// `EXCHANGE_SUBHEADER_CG_ACCEPT`.
pub const EXCHANGE_SUBHEADER_CG_ACCEPT: u8 = 4;

/// `EXCHANGE_SUBHEADER_CG_CANCEL`.
pub const EXCHANGE_SUBHEADER_CG_CANCEL: u8 = 5;

/// `StateMove` measures the distance when the Pulse's low four bits are clear
/// (`G/char.cpp:6962`).
const DISTANCE_PULSE_MASK: u64 = 15;

/// One open trade: the two characters and what each has offered.
#[derive(Debug, Clone)]
pub(super) struct Deal {
    trade: Trade,
    /// The characters, by [`Side::index`].
    vids: [Vid; 2],
    /// Who each side's lines are written for, by [`Side::index`].
    movers: [Mover; 2],
    /// The Channel the trade started on.
    channel: u8,
    /// The map it started on.
    map: i32,
}

impl Deal {
    /// The other side's character and who its lines are written for.
    const fn partner(&self, side: Side) -> (Vid, Mover) {
        let other = side.other().index();
        (self.vids[other], self.movers[other])
    }
}

/// One trade request of a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeStep {
    /// `START`: ask the player under `target` to trade.
    Start {
        /// The VID asked.
        target: u32,
    },
    /// `ITEM_ADD`: offer the item held `at`, shown on display cell `display`.
    AddItem {
        /// Where the character holds the item.
        at: ItemPos,
        /// The display cell.
        display: u8,
    },
    /// `ITEM_DEL`: take back the item in offer slot `slot`.
    RemoveItem {
        /// The offer slot.
        slot: u8,
    },
    /// `ELK_ADD`: offer `amount` gold.
    AddGold {
        /// The gold.
        amount: u64,
    },
    /// `ACCEPT`.
    Accept,
    /// `CANCEL`.
    Cancel,
}

impl TradeStep {
    /// The step a `CG_EXCHANGE` asks for, as `CInputMain::Exchange` reads it: `Find` takes
    /// arg1 as a `DWORD` and `RemoveItem` as a `BYTE`, so both keep its low bytes. `None` for a
    /// subheader the switch has no arm for, which legacy ignores.
    #[must_use]
    pub fn requested(record: CgExchange) -> Option<Self> {
        let [b0, b1, b2, b3, ..] = record.arg1.to_le_bytes();
        match record.sub_header {
            EXCHANGE_SUBHEADER_CG_START => Some(Self::Start {
                target: u32::from_le_bytes([b0, b1, b2, b3]),
            }),
            EXCHANGE_SUBHEADER_CG_ITEM_ADD => Some(Self::AddItem {
                at: record.pos,
                display: record.arg2,
            }),
            EXCHANGE_SUBHEADER_CG_ITEM_DEL => Some(Self::RemoveItem { slot: b0 }),
            EXCHANGE_SUBHEADER_CG_ELK_ADD => Some(Self::AddGold {
                amount: record.arg1,
            }),
            EXCHANGE_SUBHEADER_CG_ACCEPT => Some(Self::Accept),
            EXCHANGE_SUBHEADER_CG_CANCEL => Some(Self::Cancel),
            _ => None,
        }
    }
}

/// Why a trade step changed nothing, or ended the trade unsettled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeDeclined {
    /// The character's safebox loaded or closed within
    /// [`LOAD_WAIT_PULSES`](crate::game_state::LOAD_WAIT_PULSES).
    SafeboxWait,
    /// The player a `START` asks had its safebox load or close within
    /// [`LOAD_WAIT_PULSES`](crate::game_state::LOAD_WAIT_PULSES); that player is told why, and
    /// the sender nothing.
    PartnerSafeboxWait,
    /// The character trades already.
    Trading,
    /// No character on the character's map has the VID asked.
    NoSuchTarget {
        /// The VID asked.
        target: u32,
    },
    /// The character holds `GOLD_MAX_MAX` gold or more.
    YangLimit,
    /// The character browses a shop.
    OtherTransaction,
    /// The character asked itself.
    Itself,
    /// The VID asked is an NPC's.
    NotAPlayer,
    /// The player asked browses a shop.
    PartnerBusy,
    /// The player asked stands `EXCHANGE_MAX_DISTANCE` or farther away.
    OutOfReach {
        /// `DISTANCE_APPROX` to the player.
        distance: i32,
    },
    /// The player asked trades already.
    PartnerTrading,
    /// The character does not trade.
    NotTrading,
    /// The other side accepts, so the offers are fixed.
    PartnerAccepts,
    /// Both sides accepted and a settlement check failed; the trade ended.
    Unsettled,
}

/// Both sides of a trade that went through, for the store to write in one transaction.
#[derive(Debug, Clone)]
pub struct TradeSettled {
    /// The side whose accept completed the trade: the one that sent the step.
    pub own: MovedItems,
    /// The other side.
    pub partner: MovedItems,
    /// Where the other side's records go once the store has written the trade.
    pub partner_outbox: Option<ClientOutbox>,
}

/// What a trade step did.
#[derive(Debug, Clone)]
pub enum TradeAnswer {
    /// The step ran; the records are the sender's, and the other side has been sent its own.
    Sent(Vec<Vec<u8>>),
    /// Nothing changed, or the trade ended unsettled; the records are how legacy answers it.
    Declined {
        /// Why.
        reason: TradeDeclined,
        /// The records the sender is sent.
        records: Vec<Vec<u8>>,
    },
    /// Both sides accepted and the world settled the trade.
    Settled(Box<TradeSettled>),
}

impl TradeAnswer {
    /// A decline legacy answers with nothing.
    const fn silent(reason: TradeDeclined) -> Self {
        Self::Declined {
            reason,
            records: Vec::new(),
        }
    }
}

/// What a `START` found under the VID asked.
enum Found {
    /// A player online on the map, standing at (`x`, `y`).
    Player {
        /// Where it stands.
        x: i32,
        /// Where it stands.
        y: i32,
        /// Who its lines are written for.
        mover: Mover,
    },
    /// An NPC standing on the map.
    Npc,
}

impl GameState {
    /// Share the process's position table, which a trade finds and measures the other player
    /// in. Without it no player can be asked to trade.
    #[must_use]
    pub fn with_positions(mut self, positions: std::sync::Arc<PositionTable>) -> Self {
        self.positions = Some(positions);
        self
    }

    /// Run one trade step for the character online under `vid`, standing at `place`.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when no character is online under `vid`.
    pub fn trade(
        &mut self,
        vid: Vid,
        step: TradeStep,
        place: GroundPlace,
        mover: Mover,
    ) -> Result<TradeAnswer, MoveItemRefused> {
        if self.characters.find_by_vid(vid).is_err() {
            return Err(MoveItemRefused::NoSuchCharacter { vid });
        }
        if let Some(waits) = self.safebox_wait(vid, step, place, mover) {
            return Ok(waits);
        }
        Ok(match step {
            TradeStep::Start { target } => self.start_trade(vid, target, place, mover),
            TradeStep::AddItem { at, display } => {
                self.offer(vid, |trade, side, character, protos| {
                    trade.add_item(side, character.items_mut(), at, display, protos)
                })
            }
            TradeStep::RemoveItem { slot } => self.offer(vid, |trade, side, character, _| {
                trade.remove_item(side, character.items_mut(), slot)
            }),
            TradeStep::AddGold { amount } => self.offer(vid, |trade, side, character, _| {
                trade.add_gold(side, character.gold(), amount)
            }),
            TradeStep::Accept => self.accept_trade(vid),
            TradeStep::Cancel => self.cancel_step(vid),
        })
    }

    /// End the trade of the character under `vid`, if it trades, as its departure does.
    pub(super) fn cancel_trade(&mut self, vid: Vid) {
        let Some(&(key, _)) = self.trading.get(&vid) else {
            return;
        };
        if let Some(deal) = self.close_deal(key) {
            self.write_ended(&deal, [None; 2]);
        }
    }

    /// End every trade whose two sides stand `EXCHANGE_MAX_DISTANCE` or farther apart, or no
    /// longer on its map, on a Pulse `StateMove` measures on.
    pub(super) fn cancel_distant_trades(&mut self, pulse: u64) {
        if pulse & DISTANCE_PULSE_MASK != 0 {
            return;
        }
        let (Some(positions), Some(clients)) = (self.positions.as_deref(), self.clients.as_deref())
        else {
            return;
        };
        let apart: Vec<u32> = self
            .trades
            .iter()
            .filter(|(_, deal)| is_apart(deal, positions, clients))
            .map(|(key, _)| *key)
            .collect();
        for key in apart {
            if let Some(deal) = self.close_deal(key) {
                self.write_ended(&deal, [None; 2]);
            }
        }
    }

    /// The safebox wait of `CInputMain::Exchange` (`G/input_main.cpp:1377-1397`): the other
    /// player a `START` asks, then the sender.
    fn safebox_wait(
        &self,
        vid: Vid,
        step: TradeStep,
        place: GroundPlace,
        mover: Mover,
    ) -> Option<TradeAnswer> {
        if let TradeStep::Start { target } = step {
            let partner = Vid::new(target);
            let asked = match self.find_target(target, place) {
                Some(Found::Player { mover: asked, .. }) if partner != vid => Some(asked),
                _ => None,
            };
            if let Some(asked) = asked.filter(|_| self.safebox_loaded_recently(partner)) {
                let _sent = self.write_to_client(partner, self.trade_wait_line(asked));
                return Some(TradeAnswer::silent(TradeDeclined::PartnerSafeboxWait));
            }
        }
        self.safebox_loaded_recently(vid)
            .then(|| TradeAnswer::Declined {
                reason: TradeDeclined::SafeboxWait,
                records: vec![self.trade_wait_line(mover)],
            })
    }

    /// The `START` arm of `CInputMain::Exchange`, then `ExchangeStart`.
    fn start_trade(
        &mut self,
        vid: Vid,
        target: u32,
        place: GroundPlace,
        mover: Mover,
    ) -> TradeAnswer {
        if self.trading.contains_key(&vid) {
            return TradeAnswer::silent(TradeDeclined::Trading);
        }
        let Some(found) = self.find_target(target, place) else {
            return TradeAnswer::silent(TradeDeclined::NoSuchTarget { target });
        };
        let gold = self.characters.find_by_vid(vid).map_or(0, Character::gold);
        if gold >= GOLD_MAX_MAX {
            return self.told(TradeDeclined::YangLimit, YANG_LIMIT_NOTICE, mover);
        }
        if self.browsing.contains_key(&vid) || self.safebox_open(vid) {
            return self.told(
                TradeDeclined::OtherTransaction,
                OTHER_TRANSACTION_NOTICE,
                mover,
            );
        }
        if target == vid.raw() {
            return TradeAnswer::silent(TradeDeclined::Itself);
        }
        let Found::Player { x, y, mover: asked } = found else {
            return TradeAnswer::silent(TradeDeclined::NotAPlayer);
        };
        let partner = Vid::new(target);
        if self.browsing.contains_key(&partner) || self.safebox_open(partner) {
            return self.told(TradeDeclined::PartnerBusy, PARTNER_BUSY_NOTICE, mover);
        }
        let distance = distance_approx(place.x.saturating_sub(x), place.y.saturating_sub(y));
        if distance >= EXCHANGE_MAX_DISTANCE {
            return TradeAnswer::silent(TradeDeclined::OutOfReach { distance });
        }
        if self.trading.contains_key(&partner) {
            return TradeAnswer::Declined {
                reason: TradeDeclined::PartnerTrading,
                records: vec![Trade::already_record().encode()],
            };
        }
        let (trade, said) = Trade::start(vid.raw(), target);
        let deal = Deal {
            trade,
            vids: [vid, partner],
            movers: [mover, asked],
            channel: place.channel,
            map: place.map,
        };
        let key = vid.raw();
        let _replaced = self.trades.insert(key, deal);
        let _starter = self.trading.insert(vid, (key, Side::Starter));
        let _asked = self.trading.insert(partner, (key, Side::Asked));
        TradeAnswer::Sent(self.tell_both(Side::Starter, mover, (partner, asked), said))
    }

    /// A decline answered with one line to the sender.
    fn told(&self, reason: TradeDeclined, text: &str, mover: Mover) -> TradeAnswer {
        TradeAnswer::Declined {
            reason,
            records: vec![notice(text, mover, &self.locale)],
        }
    }

    /// What the VID asked names on the character's own Channel and map: an NPC standing
    /// there, or a player online there.
    fn find_target(&self, target: u32, place: GroundPlace) -> Option<Found> {
        let is_npc = self
            .npcs
            .get(&(place.channel, place.map))
            .is_some_and(|map| map.npcs.iter().any(|npc| npc.vid == target));
        if is_npc {
            return Some(Found::Npc);
        }
        let clients = self.clients.as_deref()?;
        let positions = self.positions.as_deref()?;
        let (_, tracked) = positions.find_on_map(clients, place.channel, place.map, target)?;
        let entry = clients
            .on_map(place.channel, place.map)
            .into_iter()
            .find(|entry| entry.vid == target)?;
        self.characters.find_by_vid(Vid::new(target)).ok()?;
        Some(Found::Player {
            x: tracked.x,
            y: tracked.y,
            mover: Mover {
                recently_fought: false,
                empire: entry.empire,
                language: entry.language,
            },
        })
    }

    /// An offer by the character under `vid`, while the other side does not accept.
    fn offer(
        &mut self,
        vid: Vid,
        step: impl FnOnce(&mut Trade, Side, &mut Character, &ItemProtos) -> Said,
    ) -> TradeAnswer {
        let Some(&(key, side)) = self.trading.get(&vid) else {
            return TradeAnswer::silent(TradeDeclined::NotTrading);
        };
        let (Some(deal), Ok(character)) = (
            self.trades.get_mut(&key),
            self.characters.find_by_vid_mut(vid),
        ) else {
            return TradeAnswer::silent(TradeDeclined::NotTrading);
        };
        if deal.trade.is_accepted(side.other()) {
            return TradeAnswer::silent(TradeDeclined::PartnerAccepts);
        }
        let said = step(&mut deal.trade, side, character, &self.protos);
        let own = deal.movers[side.index()];
        let partner = deal.partner(side);
        TradeAnswer::Sent(self.tell_both(side, own, partner, said))
    }

    /// `Accept(true)` by the character under `vid`.
    fn accept_trade(&mut self, vid: Vid) -> TradeAnswer {
        let Some(&(key, side)) = self.trading.get(&vid) else {
            return TradeAnswer::silent(TradeDeclined::NotTrading);
        };
        let Some(deal) = self.trades.get_mut(&key) else {
            return TradeAnswer::silent(TradeDeclined::NotTrading);
        };
        match deal.trade.accept(side) {
            Accepted::Unchanged => TradeAnswer::Sent(Vec::new()),
            Accepted::Waiting(said) => {
                let own = deal.movers[side.index()];
                let partner = deal.partner(side);
                TradeAnswer::Sent(self.tell_both(side, own, partner, said))
            }
            Accepted::Both => self.settle_trade(key, side),
        }
    }

    /// `Cancel` by the character under `vid`.
    fn cancel_step(&mut self, vid: Vid) -> TradeAnswer {
        let Some(&(key, side)) = self.trading.get(&vid) else {
            return TradeAnswer::silent(TradeDeclined::NotTrading);
        };
        let Some(deal) = self.close_deal(key) else {
            return TradeAnswer::silent(TradeDeclined::NotTrading);
        };
        TradeAnswer::Sent(self.answer_ended(&deal, side, [None; 2]))
    }

    /// The rest of `Accept` once both sides accept, the accept of `closer` completing it.
    fn settle_trade(&mut self, key: u32, closer: Side) -> TradeAnswer {
        let Some(deal) = self.close_deal(key) else {
            return TradeAnswer::silent(TradeDeclined::NotTrading);
        };
        let [starter, asked] = deal.vids.map(|vid| self.trader(vid));
        let (Some(starter), Some(asked)) = (starter, asked) else {
            return TradeAnswer::Declined {
                reason: TradeDeclined::NotTrading,
                records: self.answer_ended(&deal, closer, [None; 2]),
            };
        };
        let traders = [starter, asked];
        match settle(&deal.trade, closer, traders, &self.protos, &mut self.dice) {
            Ok(settled) => TradeAnswer::Settled(Box::new(self.put_back(&deal, closer, settled))),
            Err(unsettled) => TradeAnswer::Declined {
                reason: TradeDeclined::Unsettled,
                records: self.answer_ended(&deal, closer, unsettled.notices),
            },
        }
    }

    /// The character under `vid` as a settlement reads it.
    fn trader(&self, vid: Vid) -> Option<Trader> {
        let character = self.characters.find_by_vid(vid).ok()?;
        Some(Trader {
            player_id: character.player_id(),
            items: character.items().clone(),
            quickslots: character.quickslots().clone(),
            gold: character.gold(),
            usable_cells: usable_inventory_cells(character.inven_point()),
        })
    }

    /// Put both settled characters in place and build what each side is sent and stores.
    fn put_back(&mut self, deal: &Deal, closer: Side, settled: Settled) -> TradeSettled {
        let names = deal.vids.map(|vid| {
            self.characters
                .find_by_vid(vid)
                .map(|character| character.name().as_bytes().to_vec())
                .unwrap_or_default()
        });
        let Settled {
            traders: [starter, asked],
            done: [starter_done, asked_done],
            gold: [starter_gold, asked_gold],
        } = settled;
        let starter = self.hand_back(
            deal,
            Side::Starter,
            starter,
            starter_done,
            starter_gold,
            &names[1],
        );
        let asked = self.hand_back(deal, Side::Asked, asked, asked_done, asked_gold, &names[0]);
        let (own, partner) = match closer {
            Side::Starter => (starter, asked),
            Side::Asked => (asked, starter),
        };
        let partner_vid = deal.vids[closer.other().index()];
        TradeSettled {
            own,
            partner,
            partner_outbox: self.outboxes.get(&partner_vid).cloned(),
        }
    }

    /// Put one settled character in place: what `Done` sent it, the completed line naming
    /// `other`, and `END` (`G/exchange.cpp:674-693`).
    fn hand_back(
        &mut self,
        deal: &Deal,
        side: Side,
        trader: Trader,
        done: MoveDone,
        gold: i64,
        other: &[u8],
    ) -> MovedItems {
        let vid = deal.vids[side.index()];
        let mover = deal.movers[side.index()];
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            *character.items_mut() = trader.items;
            character.set_quickslots(trader.quickslots.clone());
            character.set_gold(trader.gold);
        }
        let to = mover.recipient(&self.locale);
        let completed = chat_packet(
            to,
            CHAT_TYPE_INFO,
            COMPLETED_NOTICE.as_bytes(),
            &[Arg::Text(other)],
        );
        let mut step = MovedItems::new(
            trader.player_id,
            vid.raw(),
            done,
            mover,
            &self.protos,
            &self.locale,
        );
        step.records.push(completed);
        step.records.push(Trade::end_record().encode());
        step.quickslots = Some(trader.quickslots);
        step.gold = Some(gold);
        step
    }

    /// Take the trade under `key` out of the world, if it is open.
    fn close_deal(&mut self, key: u32) -> Option<Deal> {
        let deal = self.trades.remove(&key)?;
        for vid in deal.vids {
            let _left = self.trading.remove(&vid);
        }
        Some(deal)
    }

    /// `Cancel` for both sides of a closed trade: each side's offered items are free again, and
    /// each is sent its line, if any, then `END`. Answers both sides' records.
    fn end_deal(&mut self, deal: &Deal, notices: [Option<&'static str>; 2]) -> [Vec<Vec<u8>>; 2] {
        [Side::Starter, Side::Asked].map(|side| {
            let at = side.index();
            if let Ok(character) = self.characters.find_by_vid_mut(deal.vids[at]) {
                deal.trade.withdraw(side, character.items_mut());
            }
            let mut records = Vec::with_capacity(2);
            if let Some(text) = notices[at] {
                records.push(notice(text, deal.movers[at], &self.locale));
            }
            records.push(Trade::end_record().encode());
            records
        })
    }

    /// End a closed trade, writing the other side's records and answering `first`'s.
    fn answer_ended(
        &mut self,
        deal: &Deal,
        first: Side,
        notices: [Option<&'static str>; 2],
    ) -> Vec<Vec<u8>> {
        let mut ended = self.end_deal(deal, notices);
        let other = first.other().index();
        for record in core::mem::take(&mut ended[other]) {
            let _sent = self.write_to_client(deal.vids[other], record);
        }
        core::mem::take(&mut ended[first.index()])
    }

    /// End a closed trade, writing both sides' records.
    fn write_ended(&mut self, deal: &Deal, notices: [Option<&'static str>; 2]) {
        let ended = self.end_deal(deal, notices);
        for (vid, records) in deal.vids.into_iter().zip(ended) {
            for record in records {
                let _sent = self.write_to_client(vid, record);
            }
        }
    }

    /// Write the other side's records of `said` to its client and answer `side`'s.
    fn tell_both(
        &self,
        side: Side,
        mover: Mover,
        partner: (Vid, Mover),
        mut said: Said,
    ) -> Vec<Vec<u8>> {
        let (partner_vid, partner_mover) = partner;
        for record in encoded(said.take(side.other()), partner_mover, &self.locale) {
            let _sent = self.write_to_client(partner_vid, record);
        }
        encoded(said.take(side), mover, &self.locale)
    }
}

/// Each record as its frame: a `GC_EXCHANGE`, or a `CHAT_TYPE_INFO` line for `mover`.
fn encoded(records: Vec<TradeRecord>, mover: Mover, locale: &LocaleStrings) -> Vec<Vec<u8>> {
    records
        .into_iter()
        .map(|record| match record {
            TradeRecord::Exchange(record) => record.encode(),
            TradeRecord::Notice(text) => notice(text, mover, locale),
        })
        .collect()
}

/// Whether a side of `deal` is no longer on its map, or the two stand
/// `EXCHANGE_MAX_DISTANCE` or farther apart.
fn is_apart(deal: &Deal, positions: &PositionTable, clients: &ChannelClients) -> bool {
    let [one, other] = deal
        .vids
        .map(|vid| positions.find_on_map(clients, deal.channel, deal.map, vid.raw()));
    let (Some((_, one)), Some((_, other))) = (one, other) else {
        return true;
    };
    distance_approx(one.x.saturating_sub(other.x), one.y.saturating_sub(other.y))
        >= EXCHANGE_MAX_DISTANCE
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use common::item_slots::EWindows;
    use db::items::RowChange;
    use gamedata::npc_shop::{shops_from_dump, NpcShops};
    use protocol::gc_actors::GcCharacterGoldChange;
    use protocol::gc_exchange::{
        GcExchange, EXCHANGE_SUBHEADER_GC_ACCEPT, EXCHANGE_SUBHEADER_GC_ALREADY,
        EXCHANGE_SUBHEADER_GC_END, EXCHANGE_SUBHEADER_GC_GOLD_ADD, EXCHANGE_SUBHEADER_GC_ITEM_ADD,
        EXCHANGE_SUBHEADER_GC_ITEM_DEL, EXCHANGE_SUBHEADER_GC_LESS_GOLD,
        EXCHANGE_SUBHEADER_GC_START,
    };
    use protocol::gc_item_window::HEADER_GC_ITEM_SET;
    use tokio::sync::oneshot;
    use world::character::{
        MoveKind, Quickslot, Quickslots, FULL_NOTICE, GIVE_REFUSED_NOTICE, NPOS,
        OUT_OF_PLACE_NOTICE, PARTNER_FULL_NOTICE, PARTNER_OUT_OF_PLACE_NOTICE,
    };
    use world::item::{Item, ITEM_ANTIFLAG_GIVE, ITEM_FLAG_STACKABLE};
    use world::npc::{MapNpcs, Npc};

    use super::*;
    use crate::client_registry::{ClientEntry, Lease};
    use crate::game_loop::PulseProcessor;
    use crate::game_loop_messages::GameCommand;
    use crate::game_state::{
        SafeboxAnswer, SafeboxStep, LOAD_WAIT_PULSES, OTHER_WINDOW_NOTICE, TRADE_WAIT_SECONDS,
    };
    use crate::game_state::{ShopAnswer, ShopStep};
    use crate::sync_position::SyncPositionVictimKind;

    const ALPHA: Vid = Vid::new(7);
    const YANKEE: Vid = Vid::new(9);
    /// A third player on the map.
    const ZULU: Vid = Vid::new(13);
    /// A player on map 42.
    const FARAWAY: Vid = Vid::new(11);
    /// 9007, whose shop sells weapons.
    const KEEPER: u32 = 0x8000_0001;
    /// Where everybody stands.
    const SPOT: i32 = 10_000;
    /// The small red potion, stackable.
    const POTION: u32 = 27_001;
    /// Alpha's potions.
    const ALPHAS: u32 = 5_000_001;
    /// Yankee's potions, which cannot be given.
    const YANKEES: u32 = 5_000_002;
    /// Each player's index in [`PEOPLE`].
    const A: usize = 0;
    const Y: usize = 1;
    const Z: usize = 2;
    /// Everybody online on Channel 1: VID, map, Name and empire.
    const PEOPLE: [(Vid, i32, &str, u8); 4] = [
        (ALPHA, 41, "Alpha", 1),
        (YANKEE, 41, "Yankee", 2),
        (ZULU, 41, "Zulu", 1),
        (FARAWAY, 42, "Faraway", 1),
    ];

    /// The world and the lease of each of [`PEOPLE`], which its records reach.
    struct Square {
        state: GameState,
        positions: Arc<PositionTable>,
        leases: Vec<Lease>,
    }

    impl Square {
        fn step(&mut self, who: usize, step: TradeStep) -> TradeAnswer {
            let (vid, _, _, empire) = PEOPLE[who];
            self.state.trade(vid, step, at(0), of(empire)).unwrap()
        }

        fn heard(&mut self, who: usize) -> Vec<Vec<u8>> {
            std::iter::from_fn(|| self.leases[who].try_next()).collect()
        }

        /// Move `who` `dx` east of the spot.
        fn stand(&self, who: usize, dx: i32) {
            self.stand_at(who, dx, 0);
        }

        /// Move `who` `dx` along x and `dy` along y from the spot.
        fn stand_at(&self, who: usize, dx: i32, dy: i32) {
            let id = self.leases[who].id();
            let vid = PEOPLE[who].0.raw();
            let kind = SyncPositionVictimKind::Player;
            self.positions.track(id, kind, vid, SPOT + dx, SPOT + dy);
        }

        fn character(&mut self, who: usize) -> &mut Character {
            self.state
                .characters
                .find_by_vid_mut(PEOPLE[who].0)
                .unwrap()
        }

        fn is_offered(&mut self, who: usize, id: u32) -> bool {
            self.character(who).items().is_exchanging(id)
        }

        fn browse(&mut self, who: usize) {
            let (vid, _, _, empire) = PEOPLE[who];
            let click = ShopStep::Click { target: KEEPER };
            let answer = self.state.shop(vid, click, at(0), of(empire)).unwrap();
            assert!(matches!(answer, ShopAnswer::Sent(_)), "{answer:?}");
        }

        /// Open `who`'s safebox, or its mall, with nothing in it.
        fn open_store(&mut self, who: usize, mall: bool) {
            let (vid, _, _, empire) = PEOPLE[who];
            let account = 3;
            let items = Vec::new();
            let steps = if mall {
                [
                    SafeboxStep::BeginMall,
                    SafeboxStep::OpenMall { account, items },
                ]
            } else {
                [SafeboxStep::BeginOpen, SafeboxStep::Open { account, items }]
            };
            for step in steps {
                let _answer = self.state.safebox(vid, step, of(empire)).unwrap();
            }
        }

        fn close_store(&mut self, who: usize, mall: bool) {
            let (vid, _, _, empire) = PEOPLE[who];
            let step = if mall {
                SafeboxStep::CloseMall
            } else {
                SafeboxStep::Close
            };
            let _answer = self.state.safebox(vid, step, of(empire)).unwrap();
        }

        fn leave_shop(&mut self, who: usize) {
            let (vid, _, _, empire) = PEOPLE[who];
            let _ended = self.state.shop(vid, ShopStep::End, at(0), of(empire));
        }

        /// Let every load so far stop holding back a trade.
        fn wait_out(&mut self) {
            self.state.last_pulse += LOAD_WAIT_PULSES;
        }

        /// Alpha asks Yankee, and both windows open.
        fn started(&mut self) {
            let answer = self.step(A, start(YANKEE));
            assert_eq!(
                sent(answer),
                vec![exchange(EXCHANGE_SUBHEADER_GC_START, false, 9)]
            );
            assert_eq!(
                self.heard(Y),
                vec![exchange(EXCHANGE_SUBHEADER_GC_START, false, 7)]
            );
        }

        fn is_idle(&self) -> bool {
            self.state.trades.is_empty() && self.state.trading.is_empty()
        }
    }

    /// Everybody at the spot beside a keeper on map 41, Alpha holding `alpha` and 1000 gold,
    /// Yankee `yankee` and 400.
    fn a_square(alpha: &[(ItemPos, Item)], yankee: &[(ItemPos, Item)]) -> Square {
        a_square_speaking(alpha, yankee, 1, LocaleStrings::default())
    }

    /// [`a_square`] with Yankee's client in `yankee_language`, and every language's `strings`.
    fn a_square_speaking(
        alpha: &[(ItemPos, Item)],
        yankee: &[(ItemPos, Item)],
        yankee_language: u8,
        strings: LocaleStrings,
    ) -> Square {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy");
        let protos = ItemProtos::load(&root.join("gamedata/proto")).unwrap();
        let dump = std::fs::read(root.join("sql/gamedata/player.sql")).unwrap();
        let shops = NpcShops::lay_out(&shops_from_dump(&dump).unwrap(), &protos);
        let clients = Arc::new(ChannelClients::new());
        let positions = Arc::new(PositionTable::new());
        let mut state = GameState::new(protos)
            .with_npc_shops(Arc::new(shops))
            .with_clients(Arc::clone(&clients))
            .with_positions(Arc::clone(&positions))
            .with_locale_strings(Arc::new(strings));
        let npcs = MapNpcs {
            npcs: vec![keeper()],
            positions: Vec::new(),
        };
        let _npcs = state.npcs.insert((1, 41), Arc::new(npcs));
        let mut leases = Vec::new();
        for (at, (vid, map, name, empire)) in PEOPLE.into_iter().enumerate() {
            let lease = clients.join(ClientEntry {
                channel: 1,
                map,
                name: name.to_owned(),
                vid: vid.raw(),
                empire,
                language: if at == Y { yankee_language } else { 1 },
            });
            let kind = SyncPositionVictimKind::Player;
            positions.track(lease.id(), kind, vid.raw(), SPOT, SPOT);
            let items = [alpha, yankee].get(at).copied().unwrap_or_default();
            state
                .enter_world_with_items(vid, vid.raw() * 10, name, items, lease.outbox())
                .unwrap();
            leases.push(lease);
        }
        let mut square = Square {
            state,
            positions,
            leases,
        };
        square.character(A).set_gold(1000);
        square.character(Y).set_gold(400);
        square
    }

    fn keeper() -> Npc {
        Npc {
            vid: KEEPER,
            vnum: 9007,
            race: 9007,
            char_type: 1,
            on_click: 1,
            x: SPOT,
            y: SPOT,
            z: 0,
            rotation: 0,
            empire: 1,
            moving_speed: 0,
            attack_speed: 0,
            name: Vec::new(),
        }
    }

    /// `dx` east of the spot on Channel 1's map 41.
    const fn at(dx: i32) -> GroundPlace {
        GroundPlace {
            channel: 1,
            map: 41,
            x: SPOT + dx,
            y: SPOT,
        }
    }

    const fn of(empire: u8) -> Mover {
        Mover {
            recently_fought: false,
            empire,
            language: 1,
        }
    }

    const fn start(target: Vid) -> TradeStep {
        TradeStep::Start {
            target: target.raw(),
        }
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

    fn exchange(sub_header: u8, is_me: bool, arg1: u64) -> Vec<u8> {
        GcExchange::new(sub_header, is_me, arg1, NPOS, 0).encode()
    }

    fn end() -> Vec<u8> {
        exchange(EXCHANGE_SUBHEADER_GC_END, false, 0)
    }

    /// A `CHAT_TYPE_INFO` line of `text` to a player of `empire`.
    fn line(empire: u8, text: &str) -> Vec<u8> {
        notice(text, of(empire), &LocaleStrings::default())
    }

    /// The safebox-wait line, `[LS;661;10]`, to a player of `empire`.
    fn wait_line(empire: u8) -> Vec<u8> {
        let locale = LocaleStrings::default();
        let seconds = [Arg::Int(TRADE_WAIT_SECONDS)];
        chat_packet(
            of(empire).recipient(&locale),
            CHAT_TYPE_INFO,
            b"[LS;661;%d]",
            &seconds,
        )
    }

    fn gold_change(vid: Vid, value: u64, amount: i64) -> Vec<u8> {
        let mut frame = Vec::new();
        GcCharacterGoldChange::new(vid.raw(), amount, value).encode_into(&mut frame);
        frame
    }

    fn sent(answer: TradeAnswer) -> Vec<Vec<u8>> {
        match answer {
            TradeAnswer::Sent(records) => records,
            other => panic!("the step did not run: {other:?}"),
        }
    }

    fn declined(answer: TradeAnswer) -> (TradeDeclined, Vec<Vec<u8>>) {
        match answer {
            TradeAnswer::Declined { reason, records } => (reason, records),
            other => panic!("the step was not declined: {other:?}"),
        }
    }

    fn silent(reason: TradeDeclined) -> (TradeDeclined, Vec<Vec<u8>>) {
        (reason, Vec::new())
    }

    #[test]
    fn each_exchange_record_asks_for_its_step() {
        let pos = ItemPos::new(1, 0x0102);
        let record = |sub_header, arg1| CgExchange {
            sub_header,
            arg1,
            arg2: 0x0b,
            pos,
        };
        let wide = 0x0102_0304_0506_0708;
        let asked = |sub_header| TradeStep::requested(record(sub_header, wide));
        assert_eq!(
            asked(0),
            Some(TradeStep::Start {
                target: 0x0506_0708
            })
        );
        let add = TradeStep::AddItem {
            at: pos,
            display: 0x0b,
        };
        assert_eq!(asked(1), Some(add));
        assert_eq!(asked(2), Some(TradeStep::RemoveItem { slot: 8 }));
        assert_eq!(asked(3), Some(TradeStep::AddGold { amount: wide }));
        assert_eq!(asked(4), Some(TradeStep::Accept));
        assert_eq!(asked(5), Some(TradeStep::Cancel));
        assert_eq!(asked(6), None);
        assert_eq!(asked(0xff), None);
    }

    #[test]
    fn a_start_opens_both_windows() {
        let mut square = a_square(&[], &[]);
        square.started();
        assert_eq!(square.state.trading[&ALPHA], (7, Side::Starter));
        assert_eq!(square.state.trading[&YANKEE], (7, Side::Asked));
        assert_eq!(square.state.trades.len(), 1);
        assert!(
            square.heard(A).is_empty(),
            "the starter's records are answered"
        );
        assert!(square.heard(Z).is_empty());
    }

    #[test]
    fn a_start_is_refused_in_legacys_order() {
        let mut square = a_square(&[], &[]);
        let nobody = TradeStep::Start { target: 12_345 };
        square.character(A).set_gold(GOLD_MAX_MAX);
        square.browse(A);
        let unknown = square.step(A, nobody);
        let target = 12_345;
        assert_eq!(
            declined(unknown),
            silent(TradeDeclined::NoSuchTarget { target })
        );
        let elsewhere = square.step(A, start(FARAWAY));
        let target = FARAWAY.raw();
        assert_eq!(
            declined(elsewhere),
            silent(TradeDeclined::NoSuchTarget { target }),
            "a player on another map is not looked up"
        );
        let clients = square
            .state
            .clients
            .clone()
            .expect("the square has clients");
        let entering = clients.join(ClientEntry {
            channel: 1,
            map: 41,
            name: "Entering".to_owned(),
            vid: 15,
            empire: 1,
            language: 1,
        });
        let kind = SyncPositionVictimKind::Player;
        square.positions.track(entering.id(), kind, 15, SPOT, SPOT);
        let unentered = square.step(A, start(Vid::new(15)));
        let target = 15;
        assert_eq!(
            declined(unentered),
            silent(TradeDeclined::NoSuchTarget { target }),
            "a player whose character has not entered the world is not looked up"
        );
        let rich = square.step(A, start(YANKEE));
        let told = vec![line(1, YANG_LIMIT_NOTICE)];
        assert_eq!(declined(rich), (TradeDeclined::YangLimit, told));
        square.character(A).set_gold(GOLD_MAX_MAX - 1);
        let busy = square.step(A, start(ALPHA));
        let told = vec![line(1, OTHER_TRANSACTION_NOTICE)];
        assert_eq!(declined(busy), (TradeDeclined::OtherTransaction, told));
        square.leave_shop(A);
        let itself = square.step(A, start(ALPHA));
        assert_eq!(declined(itself), silent(TradeDeclined::Itself));
        let npc = square.step(A, TradeStep::Start { target: KEEPER });
        assert_eq!(declined(npc), silent(TradeDeclined::NotAPlayer));
        square.browse(Y);
        square.stand(Y, 1041);
        let partner_busy = square.step(A, start(YANKEE));
        let told = vec![line(1, PARTNER_BUSY_NOTICE)];
        assert_eq!(declined(partner_busy), (TradeDeclined::PartnerBusy, told));
        square.leave_shop(Y);
        assert_eq!(distance_approx(1041, 0), EXCHANGE_MAX_DISTANCE);
        let far = square.step(A, start(YANKEE));
        let distance = EXCHANGE_MAX_DISTANCE;
        assert_eq!(
            declined(far),
            silent(TradeDeclined::OutOfReach { distance })
        );
        square.stand_at(Y, 0, 1041);
        let far = square.step(A, start(YANKEE));
        assert_eq!(
            declined(far),
            silent(TradeDeclined::OutOfReach { distance }),
            "the distance along y"
        );
        square.stand(Y, 1040);
        let _zulu = sent(square.step(Z, start(YANKEE)));
        square.stand(Y, 1041);
        let far = square.step(A, start(YANKEE));
        assert_eq!(
            declined(far),
            silent(TradeDeclined::OutOfReach { distance }),
            "the distance comes before the other trade"
        );
        square.stand(Y, 0);
        let already = square.step(A, start(YANKEE));
        let told = vec![exchange(EXCHANGE_SUBHEADER_GC_ALREADY, false, 0)];
        assert_eq!(declined(already), (TradeDeclined::PartnerTrading, told));
        let again = square.step(Z, nobody);
        assert_eq!(declined(again), silent(TradeDeclined::Trading));
        assert!(!square.state.trading.contains_key(&ALPHA));
        assert!(square.heard(A).is_empty());
    }

    #[test]
    fn an_open_safebox_on_either_side_refuses_a_start_and_the_mall_does_not() {
        let mut square = a_square(&[], &[]);
        square.open_store(A, false);
        square.wait_out();
        let busy = square.step(A, start(YANKEE));
        let told = vec![line(1, OTHER_TRANSACTION_NOTICE)];
        assert_eq!(declined(busy), (TradeDeclined::OtherTransaction, told));
        square.close_store(A, false);
        square.open_store(Y, false);
        square.wait_out();
        let partner_busy = square.step(A, start(YANKEE));
        let told = vec![line(1, PARTNER_BUSY_NOTICE)];
        assert_eq!(declined(partner_busy), (TradeDeclined::PartnerBusy, told));
        square.close_store(Y, false);
        square.wait_out();
        // Both malls load on the Pulse the trade starts: their loads hold nothing back.
        square.open_store(A, true);
        square.open_store(Y, true);
        square.started();
    }

    #[test]
    fn every_step_waits_ten_seconds_after_the_senders_safebox_loads_or_closes() {
        let mut square = a_square(&[], &[]);
        square.state.last_pulse = 1_000;
        square.open_store(A, false);
        square.close_store(A, false);
        square.state.last_pulse = 1_000 + LOAD_WAIT_PULSES - 1;
        for target in [YANKEE, ALPHA] {
            let held = square.step(A, start(target));
            assert_eq!(
                declined(held),
                (TradeDeclined::SafeboxWait, vec![wait_line(1)])
            );
        }
        assert!(square.heard(A).is_empty() && square.heard(Y).is_empty());
        assert!(square.state.trades.is_empty());
        square.state.last_pulse = 1_000 + LOAD_WAIT_PULSES;
        square.started();
    }

    #[test]
    fn a_start_is_ignored_while_the_player_asked_waits_after_its_safebox_loads() {
        let mut square = a_square(&[], &[]);
        square.state.last_pulse = 1_000;
        square.open_store(Y, false);
        square.close_store(Y, false);
        let _closed = square.heard(Y);
        square.state.last_pulse = 1_000 + LOAD_WAIT_PULSES - 1;
        let held = square.step(A, start(YANKEE));
        assert_eq!(declined(held), silent(TradeDeclined::PartnerSafeboxWait));
        assert_eq!(square.heard(Y), vec![wait_line(2)]);
        assert!(square.heard(A).is_empty());
        assert!(square.state.trades.is_empty());
        // Yankee's own steps wait as well; Alpha's do not.
        let own = square.step(Y, start(ALPHA));
        assert_eq!(
            declined(own),
            (TradeDeclined::SafeboxWait, vec![wait_line(2)])
        );
        square.state.last_pulse = 1_000 + LOAD_WAIT_PULSES;
        square.started();
    }

    #[test]
    fn a_safebox_that_loads_during_a_trade_holds_back_its_accept_and_its_cancel() {
        let mut square = a_square(&[], &[]);
        square.state.last_pulse = 1_000;
        square.started();
        let (vid, _, _, empire) = PEOPLE[A];
        let begun = square
            .state
            .safebox(vid, SafeboxStep::BeginOpen, of(empire));
        assert!(matches!(begun, Ok(SafeboxAnswer::Proceed)), "{begun:?}");
        for step in [TradeStep::Accept, TradeStep::Cancel] {
            let held = square.step(A, step);
            assert_eq!(
                declined(held),
                (TradeDeclined::SafeboxWait, vec![wait_line(1)])
            );
        }
        assert!(square.state.trading.contains_key(&ALPHA));
        assert!(square.heard(Y).is_empty());
        // Yankee's steps name no character: they do not wait on Alpha's safebox.
        let accepted = sent(square.step(Y, TradeStep::Accept));
        assert_eq!(
            accepted,
            vec![exchange(EXCHANGE_SUBHEADER_GC_ACCEPT, true, 1)]
        );
        let _heard = square.heard(A);
        square.state.last_pulse = 1_000 + LOAD_WAIT_PULSES;
        let cancelled = sent(square.step(A, TradeStep::Cancel));
        assert_eq!(cancelled, vec![end()]);
        assert!(square.is_idle());
    }

    #[test]
    fn a_mall_that_loads_as_the_trade_starts_holds_nothing_back() {
        // `CInputMain::Exchange` waits on `GetSafeboxLoadTime` alone (`G/input_main.cpp:1379`,
        // `:1393`), and neither safebox ever loaded here.
        let mut square = a_square(&[], &[]);
        square.open_store(A, true);
        square.open_store(Y, true);
        square.started();
    }

    #[test]
    fn a_safebox_that_arrives_while_its_character_browses_a_shop_is_not_opened() {
        let mut square = a_square(&[], &[]);
        let (vid, _, _, empire) = PEOPLE[A];
        let begun = square
            .state
            .safebox(vid, SafeboxStep::BeginOpen, of(empire));
        assert!(matches!(begun, Ok(SafeboxAnswer::Proceed)), "{begun:?}");
        square.browse(A);
        let (account, items) = (3, Vec::new());
        let open = SafeboxStep::Open { account, items };
        let answer = square.state.safebox(vid, open, of(empire)).unwrap();
        let told = vec![line(1, OTHER_WINDOW_NOTICE)];
        assert!(
            matches!(&answer, SafeboxAnswer::Sent(records) if *records == told),
            "{answer:?}"
        );
        assert!(!square.state.safebox_open(ALPHA));
    }

    #[test]
    fn each_offer_is_shown_to_both_sides() {
        let mut bound = potions(YANKEES, 3);
        bound.anti_flags = ITEM_ANTIFLAG_GIVE;
        let mut square = a_square(
            &[(inventory(3), potions(ALPHAS, 5))],
            &[(inventory(0), bound)],
        );
        square.started();
        let add = TradeStep::AddItem {
            at: inventory(3),
            display: 2,
        };
        let own = sent(square.step(A, add));
        let theirs = square.heard(Y);
        for (records, is_me) in [(own, 1), (theirs, 0)] {
            assert_eq!(records.len(), 1);
            let record = GcExchange::decode(&records[0]).unwrap();
            let shown = ItemPos::new(EWindows::ReservedWindow as u8, 2);
            assert_eq!(
                (record.sub_header, record.is_me, record.arg1, record.arg2),
                (
                    EXCHANGE_SUBHEADER_GC_ITEM_ADD,
                    is_me,
                    u64::from(POTION),
                    shown
                )
            );
            assert_eq!((record.arg3, record.arg4), (5, inventory(3)));
        }
        assert!(square.is_offered(A, ALPHAS));
        let gold = sent(square.step(A, TradeStep::AddGold { amount: 300 }));
        assert_eq!(
            gold,
            vec![exchange(EXCHANGE_SUBHEADER_GC_GOLD_ADD, true, 300)]
        );
        let heard = square.heard(Y);
        assert_eq!(
            heard,
            vec![exchange(EXCHANGE_SUBHEADER_GC_GOLD_ADD, false, 300)]
        );
        let removed = sent(square.step(A, TradeStep::RemoveItem { slot: 0 }));
        assert_eq!(
            removed,
            vec![exchange(EXCHANGE_SUBHEADER_GC_ITEM_DEL, true, 0)]
        );
        let held = GcExchange::new(EXCHANGE_SUBHEADER_GC_ITEM_DEL, false, 0, inventory(3), 0);
        assert_eq!(square.heard(Y), vec![held.encode()]);
        assert!(!square.is_offered(A, ALPHAS));
        let short = sent(square.step(Y, TradeStep::AddGold { amount: 401 }));
        assert_eq!(
            short,
            vec![exchange(EXCHANGE_SUBHEADER_GC_LESS_GOLD, false, 0)]
        );
        assert!(square.heard(A).is_empty());
        let bound = TradeStep::AddItem {
            at: inventory(0),
            display: 0,
        };
        let refused = square.state.trade(YANKEE, bound, at(0), of(1)).unwrap();
        assert_eq!(
            sent(refused),
            vec![line(2, GIVE_REFUSED_NOTICE)],
            "a line in a trade is written for the mover it started with"
        );
    }

    #[test]
    fn an_offer_waits_while_the_other_side_accepts() {
        let mut square = a_square(&[(inventory(3), potions(ALPHAS, 5))], &[]);
        square.started();
        let accepted = sent(square.step(Y, TradeStep::Accept));
        assert_eq!(
            accepted,
            vec![exchange(EXCHANGE_SUBHEADER_GC_ACCEPT, true, 1)]
        );
        let heard = square.heard(A);
        assert_eq!(
            heard,
            vec![exchange(EXCHANGE_SUBHEADER_GC_ACCEPT, false, 1)]
        );
        assert!(sent(square.step(Y, TradeStep::Accept)).is_empty());
        assert!(square.heard(A).is_empty());
        let gold = square.step(A, TradeStep::AddGold { amount: 300 });
        assert_eq!(declined(gold), silent(TradeDeclined::PartnerAccepts));
        let add = TradeStep::AddItem {
            at: inventory(3),
            display: 0,
        };
        assert_eq!(
            declined(square.step(A, add)),
            silent(TradeDeclined::PartnerAccepts)
        );
        assert!(!square.is_offered(A, ALPHAS));
        assert!(square.heard(Y).is_empty());
        let own = sent(square.step(Y, TradeStep::AddGold { amount: 100 }));
        let told = |is_me| {
            vec![
                exchange(EXCHANGE_SUBHEADER_GC_ACCEPT, is_me, 0),
                exchange(EXCHANGE_SUBHEADER_GC_GOLD_ADD, is_me, 100),
            ]
        };
        assert_eq!(
            own,
            told(true),
            "the side's own offer takes its accept back"
        );
        assert_eq!(square.heard(A), told(false));
        assert_eq!(sent(square.step(A, add)).len(), 1);
        for step in [
            TradeStep::AddGold { amount: 1 },
            TradeStep::RemoveItem { slot: 0 },
            TradeStep::Accept,
            TradeStep::Cancel,
        ] {
            let answer = square.step(Z, step);
            assert_eq!(declined(answer), silent(TradeDeclined::NotTrading));
        }
    }

    #[test]
    fn both_accepts_settle_the_trade_for_both_sides() {
        let mut square = a_square(&[(inventory(3), potions(ALPHAS, 5))], &[]);
        let mut slots = Quickslots::default();
        assert!(slots.set(0, Quickslot { kind: 1, pos: 3 }, &mut Vec::new()));
        square.character(A).set_quickslots(slots);
        square.started();
        let add = TradeStep::AddItem {
            at: inventory(3),
            display: 0,
        };
        let _item = sent(square.step(A, add));
        let _gold = sent(square.step(A, TradeStep::AddGold { amount: 300 }));
        let _paid = sent(square.step(Y, TradeStep::AddGold { amount: 200 }));
        let _accepted = sent(square.step(A, TradeStep::Accept));
        let _alpha = square.heard(A);
        let _yankee = square.heard(Y);
        let TradeAnswer::Settled(settled) = square.step(Y, TradeStep::Accept) else {
            panic!("the trade did not settle");
        };
        assert!(
            square.heard(A).is_empty(),
            "the other side waits for the store"
        );
        assert!(square.heard(Y).is_empty());
        assert!(square.is_idle());
        let TradeSettled {
            own,
            partner,
            partner_outbox,
        } = *settled;
        assert_eq!(
            (own.kind, own.owner_id, own.gold),
            (MoveKind::Traded, 90, Some(100))
        );
        assert_eq!((partner.owner_id, partner.gold), (70, Some(-100)));
        assert!(own.changes.is_empty());
        let given = RowChange::Given {
            id: ALPHAS,
            to: 90,
            window_type: EWindows::Inventory as u8,
            pos: 0,
        };
        assert_eq!(partner.changes, vec![given]);
        assert_eq!(own.records.len(), 5);
        assert_eq!(own.records[0], gold_change(YANKEE, 200, 0));
        assert_eq!(own.records[1][0], HEADER_GC_ITEM_SET);
        assert_eq!(own.records[2], gold_change(YANKEE, 500, 300));
        let strings = LocaleStrings::default();
        let completed = |empire: u8, other: &[u8]| {
            let to = of(empire).recipient(&strings);
            let text = COMPLETED_NOTICE.as_bytes();
            chat_packet(to, CHAT_TYPE_INFO, text, &[Arg::Text(other)])
        };
        assert_eq!(own.records[3..], [completed(2, b"Alpha"), end()]);
        assert_eq!(
            partner.records.len(),
            6,
            "the quickslot's delete comes before the clear"
        );
        assert_eq!(partner.records[0], gold_change(ALPHA, 1200, 200));
        assert_eq!(partner.records[2][0], HEADER_GC_ITEM_SET);
        let tail = &partner.records[partner.records.len() - 3..];
        let told = [gold_change(ALPHA, 900, 0), completed(1, b"Yankee"), end()];
        assert_eq!(tail, told);
        assert_eq!(own.quickslots, Some(Quickslots::default()));
        assert_eq!(partner.quickslots, Some(Quickslots::default()));
        assert_eq!(square.character(A).quickslots(), &Quickslots::default());
        assert_eq!(square.character(A).gold(), 900);
        assert_eq!(square.character(Y).gold(), 500);
        assert!(square.character(A).items().item_at(inventory(3)).is_none());
        let received = square.character(Y).items().item_at(inventory(0)).cloned();
        assert_eq!(
            received.map(|item| (item.id, item.count)),
            Some((ALPHAS, 5))
        );
        assert!(!square.is_offered(Y, ALPHAS));
        let outbox = partner_outbox.unwrap();
        assert!(outbox.send(b"stored".to_vec()));
        assert_eq!(
            square.heard(A),
            vec![b"stored".to_vec()],
            "the outbox is Alpha's"
        );
    }

    #[test]
    fn a_settlement_that_fails_tells_each_side_and_ends_the_trade() {
        let mut square = a_square(&[(inventory(3), potions(ALPHAS, 5))], &[]);
        square.started();
        let add = TradeStep::AddItem {
            at: inventory(3),
            display: 0,
        };
        let _item = sent(square.step(A, add));
        let _gold = sent(square.step(A, TradeStep::AddGold { amount: 300 }));
        let _accepted = sent(square.step(A, TradeStep::Accept));
        let _told = square.heard(Y);
        square.character(A).set_gold(299);
        let answer = square.step(Y, TradeStep::Accept);
        let told = vec![line(2, PARTNER_OUT_OF_PLACE_NOTICE), end()];
        assert_eq!(declined(answer), (TradeDeclined::Unsettled, told));
        assert_eq!(square.heard(A), vec![line(1, OUT_OF_PLACE_NOTICE), end()]);
        assert!(square.is_idle());
        assert!(!square.is_offered(A, ALPHAS));
        assert_eq!(square.character(A).gold(), 299);
        assert_eq!(square.character(Y).gold(), 400);
        assert!(square.character(A).items().item_at(inventory(3)).is_some());
        assert_eq!(sent(square.step(Y, start(ALPHA))).len(), 1);
    }

    #[test]
    fn a_receiver_has_room_only_in_its_unlocked_cells() {
        let crowd: Vec<(ItemPos, Item)> = (0..90)
            .map(|cell| (inventory(cell), potions(6_000_000 + u32::from(cell), 1)))
            .collect();
        let mut square = a_square(&[(inventory(3), potions(ALPHAS, 5))], &crowd);
        square.started();
        let add = TradeStep::AddItem {
            at: inventory(3),
            display: 0,
        };
        let _item = sent(square.step(A, add));
        let _accepted = sent(square.step(A, TradeStep::Accept));
        let _told = square.heard(Y);
        let answer = square.step(Y, TradeStep::Accept);
        let told = vec![line(2, FULL_NOTICE), end()];
        assert_eq!(
            declined(answer),
            (TradeDeclined::Unsettled, told),
            "a fresh character's cells from 90 on are locked"
        );
        assert_eq!(square.heard(A), vec![line(1, PARTNER_FULL_NOTICE), end()]);
        assert!(square.character(A).items().item_at(inventory(3)).is_some());
    }

    #[test]
    fn a_line_to_the_asked_side_is_in_its_clients_language() {
        let strings = || {
            let table = format!("\"{FULL_NOTICE}\";\"Kein Platz im Inventar.\";\n");
            let german = gamedata::locale_string::LanguageTable::parse(table.as_bytes());
            LocaleStrings::default().with_table(5, german)
        };
        let crowd: Vec<(ItemPos, Item)> = (0..90)
            .map(|cell| (inventory(cell), potions(6_000_000 + u32::from(cell), 1)))
            .collect();
        let alpha = [(inventory(3), potions(ALPHAS, 5))];
        let mut square = a_square_speaking(&alpha, &crowd, 5, strings());
        square.started();
        let add = TradeStep::AddItem {
            at: inventory(3),
            display: 0,
        };
        let _item = sent(square.step(A, add));
        let _told = square.heard(Y);
        let _accepted = sent(square.step(Y, TradeStep::Accept));
        let _told = square.heard(A);
        let answer = square.step(A, TradeStep::Accept);
        let told = vec![line(1, PARTNER_FULL_NOTICE), end()];
        assert_eq!(declined(answer), (TradeDeclined::Unsettled, told));
        let german = Mover {
            language: 5,
            ..of(2)
        };
        let full = notice(FULL_NOTICE, german, &strings());
        let text = b"Kein Platz";
        assert!(
            full.windows(text.len()).any(|part| part == text),
            "the table has the line"
        );
        assert_eq!(square.heard(Y), vec![full, end()]);
    }

    #[test]
    fn a_cancel_ends_the_trade_for_both_sides() {
        let mut square = a_square(&[(inventory(3), potions(ALPHAS, 5))], &[]);
        square.started();
        let add = TradeStep::AddItem {
            at: inventory(3),
            display: 0,
        };
        let _item = sent(square.step(A, add));
        let _told = square.heard(Y);
        assert_eq!(sent(square.step(Y, TradeStep::Cancel)), vec![end()]);
        assert_eq!(square.heard(A), vec![end()]);
        assert!(square.is_idle());
        assert!(!square.is_offered(A, ALPHAS));
        let again = square.step(Y, TradeStep::Cancel);
        assert_eq!(declined(again), silent(TradeDeclined::NotTrading));
    }

    #[test]
    fn leaving_the_world_ends_the_other_sides_trade() {
        let mut square = a_square(&[], &[(inventory(0), potions(YANKEES, 2))]);
        square.started();
        let add = TradeStep::AddItem {
            at: inventory(0),
            display: 0,
        };
        let _item = sent(square.step(Y, add));
        let _told = square.heard(A);
        let _kept = square.state.leave_world(ALPHA);
        assert_eq!(square.heard(Y), vec![end()]);
        assert!(square.is_idle());
        assert!(!square.is_offered(Y, YANKEES));
        assert_eq!(sent(square.step(Y, start(ZULU))).len(), 1);
    }

    #[test]
    fn a_trade_ends_on_a_sixteenth_pulse_once_its_sides_stand_apart() {
        let mut square = a_square(&[], &[]);
        square.started();
        square.stand(Y, 1040);
        square.state.process_pulse(16);
        assert_eq!(square.state.trades.len(), 1, "999 apart is in reach");
        square.stand(Y, 1041);
        square.state.process_pulse(24);
        assert_eq!(square.state.trades.len(), 1, "a Pulse off the sixteenth");
        assert!(square.heard(A).is_empty());
        square.state.process_pulse(32);
        assert!(square.is_idle());
        assert_eq!(square.heard(A), vec![end()]);
        assert_eq!(square.heard(Y), vec![end()]);
        square.stand(Y, 0);
        square.started();
        square.stand_at(Y, 0, 1041);
        square.state.process_pulse(48);
        assert!(square.is_idle(), "the distance along y");
        assert_eq!(square.heard(A), vec![end()]);
        assert_eq!(square.heard(Y), vec![end()]);
        square.stand(Y, 0);
        square.started();
        square.positions.forget(square.leases[Y].id());
        square.state.process_pulse(63);
        assert_eq!(square.state.trades.len(), 1);
        square.state.process_pulse(64);
        assert!(square.is_idle(), "a side off the map is apart");
        assert_eq!(square.heard(A), vec![end()]);
    }

    #[test]
    fn the_game_thread_runs_a_trade_step_and_answers_it() {
        let mut square = a_square(&[], &[]);
        let (reply, answer) = oneshot::channel();
        square.state.apply(GameCommand::Trade {
            vid: ALPHA,
            step: start(YANKEE),
            place: at(0),
            mover: of(1),
            reply,
        });
        let started = answer.blocking_recv().unwrap().unwrap();
        assert_eq!(
            sent(started),
            vec![exchange(EXCHANGE_SUBHEADER_GC_START, false, 9)]
        );
        let (reply, answer) = oneshot::channel();
        drop(answer);
        square.state.apply(GameCommand::Trade {
            vid: YANKEE,
            step: TradeStep::Cancel,
            place: at(0),
            mover: of(2),
            reply,
        });
        assert!(
            square.is_idle(),
            "the step runs though nobody is left to answer"
        );
        let nobody = Vid::new(99);
        let refused = square.state.trade(nobody, TradeStep::Accept, at(0), of(1));
        assert_eq!(
            refused.err(),
            Some(MoveItemRefused::NoSuchCharacter { vid: nobody })
        );
    }
}
