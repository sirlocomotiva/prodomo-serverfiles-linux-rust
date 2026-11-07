//! The Okey card game (`/cards`, `do_cards`, `G/cmd_general.cpp:3812-3864`, `GM_PLAYER`):
//! `CHARACTER::Cards_open` and friends (`G/char_cards.cpp:29-322`).
//!
//! Each character holds a [`Cards`] hand: 5 hand slots, 3 field slots, the cards left, the
//! field's points and the game's points, plus the 24 cards already drawn this deck. Nothing
//! persists: the store keeps no card state, and `Cards_clean_list` zeroes both lists.
//!
//! [`GameState::run_cards_line`] runs the arms in legacy's order. The first word's first byte,
//! lowercased, names the arm (`LOWER`, `C/utils.h:14`); the `o`, `d`, `a` and `r` arms read the
//! second word as a DWORD (`str_to_number`, `C/utils.h:52-59`) only when its first byte is a
//! digit (`isdigit`), else they run nothing. Anything else runs nothing.
//!
//! - `o` opens: with a trade, a shop, a safebox or the cube open, the caller hears `Please
//!   close all windows first.` With cards left the list is not cleaned and the deck is not
//!   refilled; otherwise 30,000 gold and one card set (79506) buy a fresh deck of 24. The
//!   caller hears `cards open <safemode>`, then the two info lines. The private shop
//!   (`GetMyShop`) and the cube (`IsCubeOpen`) are not ported, so neither refuses here.
//! - `p` pulls a card: a full hand hears `You don't have enough space in your hands`, an
//!   empty deck hears `You don't have enough cards`, and five cards across hand and field
//!   hear `You don't have enough space on the table`. Else a card of type 1-3 and value 1-8
//!   is drawn (`number`, `C/utils.cpp:355-375`), a repeat is redrawn, and the two info lines
//!   follow. The draw's deck-exhausted recursion (`RandomizeCards` with all 24 drawn) is
//!   unreachable: the pullout refuses once the 24 are out.
//! - `e` ends: the points buy a box — 400 or more the golden (71194), 250-399 the silver
//!   (71195), 1-249 the bronze (71196), 0 nothing — through `AutoGiveItem`, the score is
//!   logged to the Okey table, the lists clean, and the two info lines follow.
//! - `d` destroys the hand slot the second word names: past slot 4, or an empty slot, runs
//!   nothing. Else the slot clears and the two info lines follow.
//! - `a` accepts the hand slot the second word names onto the field: past slot 4, an empty
//!   slot, or a full field runs nothing but the two info lines. When the field fills, a win
//!   sends the reward line and resets the field, else the field restores to the hand; the two
//!   info lines follow either way.
//! - `r` restores the field slot the second word names to the hand: past slot 2, or an empty
//!   slot, runs nothing. Else the card moves back and the two info lines follow. A restore
//!   with a full hand writes past it (`GetEmptySpaceInHand` answers -1): a legacy Defect the
//!   Rewrite refuses instead (ledger 411), since the write lands outside the hand.
//!
//! The two info lines (`SendUpdatedInformations`) are `cards info` with the 5 hand pairs, the
//! cards left and the points, and `cards finfo` with the 3 field pairs and the field points.
//! The reward line (`SendReward`) is `cards reward` with the 3 field pairs and the field
//! points.
//!
//! A win (`CheckReward`) pays the field's points and adds them to the game's: three of a kind
//! 150, a suited run 100, three of a value the lowest times 10 plus 10, any run the lowest
//! times 10. A loss restores the field to the hand. `CheckReward`'s restore runs again in
//! `CardsAccept`'s loss arm, but the field is already empty, so the second restore moves
//! nothing.
//!
//! The two ranks (`GetGlobalRank`, `GetRundRank`, `G/char_cards.cpp:336-403`) read the Okey
//! table the end logged: the global rank sums each Name's points, the round rank lists each
//! game, best first, ten rows. An empty table pushes an empty string. The text is
//! `[ENTER]1. Place - <name> - <points> points.` per row, cut to the 4096-byte buffer the way
//! `snprintf` cuts it.

use std::collections::HashMap;

use common::item_slots::usable_inventory_cells;
use common::vid::Vid;
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};
use world::character::{
    count_specify_item, number, remove_specify_item, AutoGiven, MoveDone, MoveKind, MoveRecord,
};

use super::GameState;
use crate::chat_line::Arg;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

/// The hand slots (`MAX_CARDS_IN_HAND`, `G/char.h:55`).
const HAND: usize = 5;
/// The field slots (`MAX_CARDS_IN_FIELD`, `G/char.h:56`).
const FIELD: usize = 3;
/// The drawn cards a deck remembers (`randomized_cards[24]`, `G/char.h:2464`).
const DECK: usize = 24;
/// The gold a fresh deck costs (`Cards_open`).
const DECK_GOLD: u64 = 30_000;
/// The card set a fresh deck spends (`Cards_open`).
const CARD_SET: u32 = 79506;
/// A fresh deck's cards (`Cards_open`).
const FRESH_DECK: u32 = 24;
/// The golden box: 400 points or more (`CardsEnd`).
const GOLDEN_BOX: u32 = 71194;
/// The silver box: 250-399 points (`CardsEnd`).
const SILVER_BOX: u32 = 71195;
/// The bronze box: 1-249 points (`CardsEnd`).
const BRONZE_BOX: u32 = 71196;
/// The golden box's floor (`CardsEnd`).
const GOLDEN_POINTS: u32 = 400;
/// The silver box's floor (`CardsEnd`).
const SILVER_POINTS: u32 = 250;
/// Three of a kind's points (`CheckReward`).
const KIND_POINTS: u32 = 150;
/// A suited run's points (`CheckReward`).
const SUITED_RUN_POINTS: u32 = 100;
/// The rank rows (`LIMIT 10`).
const RANK_ROWS: usize = 10;
/// The rank buffer (`char szBuf[4096+1]`).
const RANK_BUFFER: usize = 4096;

/// One card: its type and value. A type of 0 is no card.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Card {
    /// The card's type, 1-3.
    kind: u32,
    /// The card's value, 1-8.
    value: u32,
}

/// A character's cards (`CARDS_INFO` and `randomized_cards`, `G/char.h:2428-2464`).
#[derive(Debug, Clone, Default)]
pub(super) struct Cards {
    /// The hand's cards.
    hand: [Card; HAND],
    /// The field's cards.
    field: [Card; FIELD],
    /// The cards left in the deck.
    left: u32,
    /// The field's points.
    field_points: u32,
    /// The game's points.
    points: u32,
    /// The cards drawn this deck.
    drawn: [Card; DECK],
}

/// One Okey game the end logged (`log.okay_event`): who scored and how much.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OkayGame {
    /// The scorer's PID.
    pid: u32,
    /// The scorer's Name.
    name: Vec<u8>,
    /// The game's points.
    points: u32,
}

/// The Okey table: every ended game, in end order.
#[derive(Debug, Default)]
pub(super) struct OkayTable {
    /// The ended games.
    games: Vec<OkayGame>,
}

/// The card tables: each character's hand and the ended games.
#[derive(Debug, Default)]
pub(super) struct CardTables {
    /// Each character's Okey cards by PID.
    pub(super) hands: HashMap<u32, Cards>,
    /// The ended games the ranks read.
    pub(super) okay: OkayTable,
}

impl Cards {
    /// The first empty hand slot, if one waits (`GetEmptySpaceInHand`).
    fn empty_hand(&self) -> Option<usize> {
        self.hand.iter().position(|card| card.kind == 0)
    }

    /// The first empty field slot, if one waits (`GetEmptySpaceInField`).
    fn empty_field(&self) -> Option<usize> {
        self.field.iter().position(|card| card.kind == 0)
    }

    /// The cards across hand and field (`GetAllCardsCount`).
    fn dealt(&self) -> usize {
        self.hand
            .iter()
            .chain(self.field.iter())
            .filter(|card| card.kind != 0)
            .count()
    }

    /// Whether the field's three types match (`TypesAreSame`).
    fn types_same(&self) -> bool {
        self.field[0].kind == self.field[1].kind && self.field[1].kind == self.field[2].kind
    }

    /// Whether the field's three values match (`ValuesAreSame`).
    fn values_same(&self) -> bool {
        self.field[0].value == self.field[1].value && self.field[1].value == self.field[2].value
    }

    /// Whether the field's three values run (`CardsMatch`): the six orders of a run of three.
    fn values_run(&self) -> bool {
        let [a, b, c] = [
            self.field[0].value,
            self.field[1].value,
            self.field[2].value,
        ];
        (a + 1 == b && b + 1 == c)
            || (a + 1 == c && c + 1 == b)
            || (b + 1 == a && a + 1 == c)
            || (a + 1 == b && c + 1 == a)
            || (b + 1 == a && c + 1 == b)
            || (b + 1 == c && c + 1 == a)
    }

    /// The field's lowest value (`GetLowestCard`).
    fn lowest(&self) -> u32 {
        self.field[0]
            .value
            .min(self.field[1].value)
            .min(self.field[2].value)
    }

    /// `CheckReward`: the win's points land on the field and the game, and the field
    /// restores to the hand on a loss. Answers whether the field won.
    fn check_reward(&mut self) -> bool {
        if self.types_same() && self.values_same() {
            self.field_points = KIND_POINTS;
            self.points += KIND_POINTS;
            true
        } else if self.types_same() && self.values_run() {
            self.field_points = SUITED_RUN_POINTS;
            self.points += SUITED_RUN_POINTS;
            true
        } else if self.values_same() {
            let points = self.lowest() * 10 + 10;
            self.field_points = points;
            self.points += points;
            true
        } else if self.values_run() {
            let points = self.lowest() * 10;
            self.field_points = points;
            self.points += points;
            true
        } else {
            self.restore_field();
            false
        }
    }

    /// `RestoreField`: each field card moves to the first empty hand slot.
    fn restore_field(&mut self) {
        for slot in 0..FIELD {
            if self.field[slot].kind == 0 {
                continue;
            }
            if let Some(empty) = self.empty_hand() {
                self.hand[empty] = self.field[slot];
                self.field[slot] = Card::default();
            }
        }
    }

    /// `ResetField`: the field clears.
    fn reset_field(&mut self) {
        self.field = [Card::default(); FIELD];
    }

    /// `Cards_clean_list`: both lists zero.
    fn reset_all(&mut self) {
        *self = Self::default();
    }
}

impl OkayTable {
    /// The global rank's text (`GetGlobalRank`): each Name's summed points, best first, ten
    /// rows. Empty with no game.
    pub(super) fn global_rank(&self) -> Vec<u8> {
        let mut sums: HashMap<u32, (Vec<u8>, u32)> = HashMap::new();
        for game in &self.games {
            sums.entry(game.pid)
                .and_modify(|(_, sum)| *sum += game.points)
                .or_insert((game.name.clone(), game.points));
        }
        let mut rows: Vec<(Vec<u8>, u32)> = sums.into_values().collect();
        rows.sort_by(|(_, a), (_, b)| b.cmp(a));
        rank_text(&rows)
    }

    /// Both rank texts, global then round, for a quest step's snapshot.
    pub(super) fn ranks(&self) -> (Vec<u8>, Vec<u8>) {
        (self.global_rank(), self.round_rank())
    }

    /// The round rank's text (`GetRundRank`): each game, best first, ten rows. Empty with no
    /// game.
    pub(super) fn round_rank(&self) -> Vec<u8> {
        let mut rows: Vec<(Vec<u8>, u32)> = self
            .games
            .iter()
            .map(|game| (game.name.clone(), game.points))
            .collect();
        rows.sort_by(|(_, a), (_, b)| b.cmp(a));
        rank_text(&rows)
    }
}

/// The rank text of `rows`: `[ENTER]<pos>. Place - <name> - <points> points.` per row, the
/// first ten, cut the way the 4096-byte `snprintf` buffer cuts them.
fn rank_text(rows: &[(Vec<u8>, u32)]) -> Vec<u8> {
    let mut text = Vec::new();
    for (at, (name, points)) in rows.iter().take(RANK_ROWS).enumerate() {
        // Each `snprintf` writes into the room the NUL still allows: at most
        // `buflen - len - 1` bytes, and nothing once the buffer is full.
        for piece in [
            b"[ENTER]".as_slice(),
            format!("{}. Place - ", at + 1).as_bytes(),
            name,
            format!(" - {points} points.").as_bytes(),
        ] {
            let room = RANK_BUFFER.saturating_sub(text.len() + 1);
            text.extend_from_slice(&piece[..piece.len().min(room)]);
        }
    }
    text
}

impl GameState {
    /// The caller's cards, made when it has none.
    fn cards_of(&mut self, pid: u32) -> &mut Cards {
        self.card_tables.hands.entry(pid).or_default()
    }

    /// The two info lines (`SendUpdatedInformations`).
    fn tell_cards(&self, vid: Vid, mover: Mover, cards: &Cards) {
        let hand = cards.hand;
        let field = cards.field;
        self.tell(
            vid,
            mover,
            CHAT_TYPE_COMMAND,
            b"cards info %d %d %d %d %d %d %d %d %d %d %d %d",
            &[
                Arg::Int(i64::from(hand[0].kind)),
                Arg::Int(i64::from(hand[0].value)),
                Arg::Int(i64::from(hand[1].kind)),
                Arg::Int(i64::from(hand[1].value)),
                Arg::Int(i64::from(hand[2].kind)),
                Arg::Int(i64::from(hand[2].value)),
                Arg::Int(i64::from(hand[3].kind)),
                Arg::Int(i64::from(hand[3].value)),
                Arg::Int(i64::from(hand[4].kind)),
                Arg::Int(i64::from(hand[4].value)),
                Arg::Int(i64::from(cards.left)),
                Arg::Int(i64::from(cards.points)),
            ],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_COMMAND,
            b"cards finfo %d %d %d %d %d %d %d",
            &[
                Arg::Int(i64::from(field[0].kind)),
                Arg::Int(i64::from(field[0].value)),
                Arg::Int(i64::from(field[1].kind)),
                Arg::Int(i64::from(field[1].value)),
                Arg::Int(i64::from(field[2].kind)),
                Arg::Int(i64::from(field[2].value)),
                Arg::Int(i64::from(cards.field_points)),
            ],
        );
    }

    /// The reward line (`SendReward`).
    fn tell_reward(&self, vid: Vid, mover: Mover, cards: &Cards) {
        let field = cards.field;
        self.tell(
            vid,
            mover,
            CHAT_TYPE_COMMAND,
            b"cards reward %d %d %d %d %d %d %d",
            &[
                Arg::Int(i64::from(field[0].kind)),
                Arg::Int(i64::from(field[0].value)),
                Arg::Int(i64::from(field[1].kind)),
                Arg::Int(i64::from(field[1].value)),
                Arg::Int(i64::from(field[2].kind)),
                Arg::Int(i64::from(field[2].value)),
                Arg::Int(i64::from(cards.field_points)),
            ],
        );
    }

    /// Draw a card the deck has not drawn (`RandomizeCards` with `CardWasRandomized`): a
    /// type of 1-3 and a value of 1-8, a repeat redrawn. The deck holds 24 distinct cards,
    /// so a draw always lands while cards are left.
    fn draw_card(&mut self, pid: u32) -> Card {
        loop {
            let kind = number(&mut self.dice, 1, 3);
            let value = number(&mut self.dice, 1, 8);
            let card = Card {
                kind: u32::try_from(kind).unwrap_or(u32::MAX),
                value: u32::try_from(value).unwrap_or(u32::MAX),
            };
            let cards = self.cards_of(pid);
            if cards.drawn.iter().any(|drawn| *drawn == card) {
                continue;
            }
            if let Some(empty) = cards.drawn.iter().position(|drawn| drawn.kind == 0) {
                cards.drawn[empty] = card;
            }
            return card;
        }
    }
    /// One `/cards` line (`do_cards`, `G/cmd_general.cpp:3812-3864`).
    ///
    /// Any caller at any pose. The first word's first byte, lowercased, names the arm;
    /// the `o`, `d`, `a` and `r` arms read the second word as a DWORD only when its
    /// first byte is a digit, else they run nothing. The open's gold spend and card-set
    /// spend ride the answer, and the end's box give rides too; the chat lines are told
    /// at once.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when the VID has no character in the world.
    pub(super) fn run_cards_line(
        &mut self,
        vid: Vid,
        rest: &[u8],
    ) -> Result<MovedItems, MoveItemRefused> {
        let owner = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?
            .player_id();
        let mover = self.own_mover(vid);
        let (first, back) = crate::command::one_argument(rest);
        let (second, _) = crate::command::one_argument(back);
        let mut done = MoveDone {
            kind: MoveKind::Given,
            records: Vec::new(),
            changes: Vec::new(),
        };
        let held = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::gold);
        match first.first().map(u8::to_ascii_lowercase) {
            Some(b'o') => self.cards_open(vid, owner, &second, &mut done),
            Some(b'p') => self.cards_pullout(vid, owner),
            Some(b'e') => self.cards_end(vid, owner, &mut done),
            Some(b'd') => self.cards_destroy(vid, owner, &second),
            Some(b'a') => self.cards_accept(vid, owner, &second),
            Some(b'r') => self.cards_restore(vid, owner, &second),
            _ => {}
        }
        let mut step = self.finish_cards(vid, owner, mover, done);
        let left = self
            .characters
            .find_by_vid(vid)
            .map_or(held, world::character::Character::gold);
        if left != held {
            step.gold = Some(crate::item_move::gold_delta(held, left));
        }
        Ok(step)
    }

    /// The DWORD the `o`, `d`, `a` and `r` arms read: `Some` only when the word's first
    /// byte is a digit (`isdigit(*arg2)`).
    fn cards_arg(second: &[u8]) -> Option<u32> {
        if !second.first().is_some_and(u8::is_ascii_digit) {
            return None;
        }
        Some(super::skill_admin::str_to_dword(second))
    }

    /// The moved step of a `/cards` line: the spends and the box give ride, the points
    /// the spends did not change ride with them, and the chat lines the run told are
    /// already in the outbox.
    fn finish_cards(
        &mut self,
        vid: Vid,
        owner: u32,
        caller: Option<Mover>,
        done: MoveDone,
    ) -> MovedItems {
        let Some(caller) = caller else {
            let silent = Mover {
                recently_fought: false,
                empire: 0,
                language: 0,
                pk_mode: 0,
                affect_flags: [0; 2],
                dead: false,
                alignment: 0,
                view_guild: (0, 0),
                premium: (0, 0),
            };
            let mut step =
                MovedItems::new(owner, vid.raw(), done, silent, &self.protos, &self.locale);
            step.points = None;
            return step;
        };
        let mut step = MovedItems::new(owner, vid.raw(), done, caller, &self.protos, &self.locale);
        step.points = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points().cloned());
        step
    }
    /// `Cards_open` (`G/char_cards.cpp:29-56`): with a trade, a shop, a browsing or a
    /// safebox open, the caller hears `Please close all windows first.` With cards left
    /// the deck stands; otherwise 30,000 gold and one card set buy a fresh deck of 24.
    /// The caller hears `cards open <safemode>`, then the two info lines.
    ///
    /// The gold spend is `ChangeGold`, not the `PointChange(POINT_GOLD)` the source
    /// calls: under `ENABLE_REMOVE_LIMIT_GOLD` that arm is compiled out, so legacy sends
    /// the point record while keeping the gold (a Defect, as the drop's was). The spend
    /// rides the answer as a gold record with a `gold` delta.
    fn cards_open(&mut self, vid: Vid, pid: u32, second: &[u8], done: &mut MoveDone) {
        let Some(safemode) = Self::cards_arg(second) else {
            return;
        };
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        if self.trading.contains_key(&vid)
            || self.browsing.contains_key(&vid)
            || self.safebox_open(vid)
        {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Please close all windows first.",
                &[],
            );
            return;
        }
        if self.cards_of(pid).left == 0 {
            let held = self
                .characters
                .find_by_vid(vid)
                .map_or(0, world::character::Character::gold);
            if held < DECK_GOLD {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"You don't have enough money",
                    &[],
                );
                return;
            }
            let usable = self.characters.find_by_vid(vid).map_or(0, |character| {
                usable_inventory_cells(character.inven_point())
            });
            let has = self.characters.find_by_vid(vid).map_or(0, |character| {
                count_specify_item(character.items(), usable, CARD_SET)
            });
            if has < 1 {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"You don't have any Okey Card Set(s)",
                    &[],
                );
                return;
            }
            let left = held - DECK_GOLD;
            if let Ok(character) = self.characters.find_by_vid_mut(vid) {
                character.set_gold(left);
            }
            done.records.push(MoveRecord::Gold {
                amount: 0,
                value: left,
            });
            let removed = self.characters.find_by_vid_mut(vid).map(|character| {
                let usable = usable_inventory_cells(character.inven_point());
                let (items, _, slots) = character.items_points_and_quickslots_mut();
                let protos = std::sync::Arc::clone(&self.protos);
                remove_specify_item(items, slots, &protos, usable, (CARD_SET, 1))
            });
            if let Ok(removed) = removed {
                done.records.extend(removed.records);
                done.changes.extend(removed.changes);
            }
            // `Cards_clean_list` zeroes both lists before the refill.
            self.card_tables.hands.entry(pid).or_default().reset_all();
            self.card_tables.hands.entry(pid).or_default().left = FRESH_DECK;
        }
        self.tell(
            vid,
            mover,
            CHAT_TYPE_COMMAND,
            b"cards open %d",
            &[Arg::Int(i64::from(safemode))],
        );
        let cards = self.cards_of(pid).clone();
        self.tell_cards(vid, mover, &cards);
    }

    /// `Cards_pullout` (`G/char_cards.cpp:73-94`): a full hand, an empty deck, or five
    /// cards across hand and field are refused with their lines. Else a fresh card lands
    /// in the hand and the two info lines follow.
    fn cards_pullout(&mut self, vid: Vid, pid: u32) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let cards = self.cards_of(pid).clone();
        if cards.empty_hand().is_none() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"You don't have enough space in your hands",
                &[],
            );
            return;
        }
        if cards.left < 1 {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"You don't have enough cards",
                &[],
            );
            return;
        }
        if cards.dealt() >= HAND {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"You don't have enough space on the table",
                &[],
            );
            return;
        }
        let card = self.draw_card(pid);
        let cards = self.cards_of(pid);
        if let Some(empty) = cards.empty_hand() {
            cards.hand[empty] = card;
            cards.left -= 1;
        }
        let cards = self.cards_of(pid).clone();
        self.tell_cards(vid, mover, &cards);
    }
    /// `CardsEnd` (`G/char_cards.cpp:323-334`): the points buy a box through
    /// `AutoGiveItem` — golden at 400, silver at 250, bronze above 0, nothing at 0 — the
    /// score logs to the Okey table, the lists clean, and the two info lines follow.
    fn cards_end(&mut self, vid: Vid, pid: u32, done: &mut MoveDone) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let points = self.cards_of(pid).points;
        let prize = if points >= GOLDEN_POINTS {
            Some(GOLDEN_BOX)
        } else if points >= SILVER_POINTS {
            Some(SILVER_BOX)
        } else if points > 0 {
            Some(BRONZE_BOX)
        } else {
            None
        };
        if let Some(vnum) = prize {
            let questing = self.quest_running(vid);
            if let Some((given, mut made)) = self.auto_give(vid, (vnum, 1), -1, questing) {
                made.records
                    .retain(|record| !matches!(record, MoveRecord::PickedUp { .. }));
                if let AutoGiven::Unplaced(item) = given {
                    self.lay_auto_given(vid, item);
                }
                done.records.extend(made.records);
                done.changes.extend(made.changes);
            }
        }
        let name = self
            .characters
            .find_by_vid(vid)
            .map(|character| character.name().as_bytes().to_vec())
            .unwrap_or_default();
        self.card_tables
            .okay
            .games
            .push(OkayGame { pid, name, points });
        self.card_tables.hands.entry(pid).or_default().reset_all();
        let cards = self.cards_of(pid).clone();
        self.tell_cards(vid, mover, &cards);
    }

    /// `CardsDestroy` (`G/char_cards.cpp:152-162`): past slot 4, or an empty slot, runs
    /// nothing. Else the slot clears and the two info lines follow.
    fn cards_destroy(&mut self, vid: Vid, pid: u32, second: &[u8]) {
        let (Some(mover), Some(slot)) = (self.own_mover(vid), Self::cards_arg(second)) else {
            return;
        };
        let Ok(slot) = usize::try_from(slot) else {
            return;
        };
        if slot + 1 > HAND {
            return;
        }
        if self.cards_of(pid).hand[slot].kind == 0 {
            return;
        }
        self.cards_of(pid).hand[slot] = Card::default();
        let cards = self.cards_of(pid).clone();
        self.tell_cards(vid, mover, &cards);
    }

    /// `CardsAccept` (`G/char_cards.cpp:164-190`): past slot 4 or an empty slot runs
    /// nothing; a full field runs nothing but the two info lines. Else the card moves
    /// onto the field, and when the field fills a win sends the reward line and resets
    /// the field while a loss restores it to the hand; the two info lines follow.
    fn cards_accept(&mut self, vid: Vid, pid: u32, second: &[u8]) {
        let (Some(mover), Some(slot)) = (self.own_mover(vid), Self::cards_arg(second)) else {
            return;
        };
        let Ok(slot) = usize::try_from(slot) else {
            return;
        };
        if slot + 1 > HAND {
            return;
        }
        if self.cards_of(pid).hand[slot].kind == 0 {
            return;
        }
        if self.cards_of(pid).empty_field().is_some() {
            let card = self.cards_of(pid).hand[slot];
            if let Some(empty) = self.cards_of(pid).empty_field() {
                self.cards_of(pid).field[empty] = card;
                self.cards_of(pid).hand[slot] = Card::default();
            }
        }
        if self.cards_of(pid).empty_field().is_none() {
            if self.cards_of(pid).check_reward() {
                let cards = self.cards_of(pid).clone();
                self.tell_reward(vid, mover, &cards);
                self.cards_of(pid).reset_field();
            } else {
                self.cards_of(pid).restore_field();
            }
        }
        let cards = self.cards_of(pid).clone();
        self.tell_cards(vid, mover, &cards);
    }

    /// `CardsRestore` (`G/char_cards.cpp:192-206`): past slot 2, or an empty slot, runs
    /// nothing. Else the card moves back to the hand and the two info lines follow. A
    /// full hand refuses instead of writing past it (ledger 411).
    fn cards_restore(&mut self, vid: Vid, pid: u32, second: &[u8]) {
        let (Some(mover), Some(slot)) = (self.own_mover(vid), Self::cards_arg(second)) else {
            return;
        };
        let Ok(slot) = usize::try_from(slot) else {
            return;
        };
        if slot + 1 > FIELD {
            return;
        }
        if self.cards_of(pid).field[slot].kind == 0 {
            return;
        }
        if self.cards_of(pid).empty_hand().is_none() {
            return;
        }
        let card = self.cards_of(pid).field[slot];
        if let Some(empty) = self.cards_of(pid).empty_hand() {
            self.cards_of(pid).hand[empty] = card;
            self.cards_of(pid).field[slot] = Card::default();
        }
        let cards = self.cards_of(pid).clone();
        self.tell_cards(vid, mover, &cards);
    }
}

#[cfg(test)]
mod tests {
    use common::item_slots::EWindows;
    use common::vid::Vid;
    use protocol::item_pos::ItemPos;
    use world::item::Item;

    use super::*;
    use crate::game_state::fixtures::{a_world, enter_seeing_with, TestClock, PLACE};

    /// A world with one entered player holding `items`, its inbox drained of the enter,
    /// and 100,000 gold.
    fn entered(
        items: &[(ItemPos, Item)],
    ) -> (GameState, tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        world
            .install_item_ids(world::item::ItemIdRange::new(1, 1_000_000, 1).expect("a range"))
            .expect("the first install");
        let (mut inbox, _) = enter_seeing_with(&mut world, PLACE, 1, (5, 5), 10_000, items);
        while inbox.try_recv().is_ok() {}
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .expect("the player is in")
            .set_gold(100_000);
        (world, inbox)
    }

    /// A card set (79506) of `id` in `cell`.
    fn card_set(id: u32, cell: u16) -> (ItemPos, Item) {
        (
            ItemPos::new(EWindows::Inventory as u8, cell),
            Item::new(id, CARD_SET),
        )
    }

    /// The inbox's chat texts, in order.
    fn chats(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok())
            .filter(|frame| frame.first() == Some(&protocol::gc_inventory::HEADER_GC_CHAT.value()))
            .map(|frame| frame[10..].to_vec())
            .collect()
    }

    #[test]
    fn the_open_buys_a_deck_and_the_second_open_keeps_it() {
        let (mut world, mut inbox) = entered(&[card_set(10, 0)]);
        let step = world
            .run_cards_line(Vid::new(1), b"open 1")
            .expect("the line runs");
        assert_eq!(world.cards_of(1).left, FRESH_DECK);
        assert_eq!(
            world
                .characters
                .find_by_vid(Vid::new(1))
                .expect("in")
                .gold(),
            70_000
        );
        assert!(
            step.records.iter().any(|record| record[0] == 225),
            "the gold change rides"
        );
        assert_eq!(step.gold, Some(-30_000));
        let lines = chats(&mut inbox);
        assert_eq!(lines[0], b"cards open 1");
        assert_eq!(
            lines[1], b"cards info 0 0 0 0 0 0 0 0 0 0 24 0",
            "the fresh hand is empty"
        );
        assert_eq!(lines[2], b"cards finfo 0 0 0 0 0 0 0");
        // The second open spends nothing: cards are left.
        let step = world
            .run_cards_line(Vid::new(1), b"open 0")
            .expect("the line runs");
        assert_eq!(
            world
                .characters
                .find_by_vid(Vid::new(1))
                .expect("in")
                .gold(),
            70_000
        );
        assert_eq!(step.gold, None);
        let lines = chats(&mut inbox);
        assert_eq!(lines[0], b"cards open 0");
    }

    #[test]
    fn the_open_refuses_without_gold_or_a_set() {
        let (mut world, mut inbox) = entered(&[]);
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .expect("in")
            .set_gold(29_999);
        world
            .run_cards_line(Vid::new(1), b"open 1")
            .expect("the line runs");
        assert_eq!(
            chats(&mut inbox),
            [b"You don't have enough money".as_slice()]
        );
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .expect("in")
            .set_gold(100_000);
        world
            .run_cards_line(Vid::new(1), b"open 1")
            .expect("the line runs");
        assert_eq!(
            chats(&mut inbox),
            [b"You don't have any Okey Card Set(s)".as_slice()]
        );
    }

    #[test]
    fn the_pullout_deals_five_and_refuses_the_sixth() {
        let (mut world, mut inbox) = entered(&[card_set(10, 0)]);
        world.run_cards_line(Vid::new(1), b"open 0").expect("open");
        while inbox.try_recv().is_ok() {}
        for _ in 0..HAND {
            world
                .run_cards_line(Vid::new(1), b"pullout")
                .expect("a pull lands");
        }
        assert_eq!(world.cards_of(1).dealt(), HAND);
        assert_eq!(
            world.cards_of(1).left,
            FRESH_DECK - u32::try_from(HAND).unwrap()
        );
        world
            .run_cards_line(Vid::new(1), b"pullout")
            .expect("the line runs");
        assert_eq!(
            chats(&mut inbox).last().expect("a line"),
            b"You don't have enough space in your hands"
        );
    }

    #[test]
    fn the_accept_pays_three_of_a_kind_and_the_end_logs_it() {
        let (mut world, mut inbox) = entered(&[card_set(10, 0)]);
        world.run_cards_line(Vid::new(1), b"open 0").expect("open");
        {
            let cards = world.cards_of(1);
            cards.hand[0] = Card { kind: 2, value: 5 };
            cards.hand[1] = Card { kind: 2, value: 5 };
            cards.hand[2] = Card { kind: 2, value: 5 };
        }
        for slot in [0, 1, 2] {
            world
                .run_cards_line(Vid::new(1), format!("accept {slot}").as_bytes())
                .expect("an accept lands");
        }
        assert_eq!(world.cards_of(1).points, KIND_POINTS);
        assert!(world.cards_of(1).field.iter().all(|card| card.kind == 0));
        let lines = chats(&mut inbox);
        assert!(
            lines
                .iter()
                .any(|line| line == b"cards reward 2 5 2 5 2 5 150"),
            "the reward names the field: {lines:?}"
        );
        while inbox.try_recv().is_ok() {}
        let step = world
            .run_cards_line(Vid::new(1), b"end")
            .expect("the end runs");
        assert!(step
            .changes
            .iter()
            .any(|change| format!("{change:?}").contains("71196")));
        assert_eq!(world.cards_of(1).points, 0);
        assert_eq!(world.card_tables.okay.games.len(), 1);
        assert_eq!(
            world.card_tables.okay.global_rank(),
            b"[ENTER]1. Place - P1 - 150 points."
        );
        assert_eq!(
            world.card_tables.okay.round_rank(),
            b"[ENTER]1. Place - P1 - 150 points."
        );
    }

    #[test]
    fn the_rank_text_stops_at_the_snprintf_cut() {
        let name = vec![b'n'; 5000];
        let rows = vec![(name, 1)];
        let text = rank_text(&rows);
        // `[ENTER]1. Place - ` is 17 bytes; the buffer holds 4095, so the name
        // fills the rest and the points never land.
        assert_eq!(text.len(), RANK_BUFFER - 1);
        assert!(text.ends_with(b"n"));
    }

    #[test]
    fn the_ranks_sum_and_order_the_games() {
        let mut table = OkayTable::default();
        assert!(table.global_rank().is_empty());
        table.games.push(OkayGame {
            pid: 1,
            name: b"Alpha".to_vec(),
            points: 100,
        });
        table.games.push(OkayGame {
            pid: 2,
            name: b"Beta".to_vec(),
            points: 400,
        });
        table.games.push(OkayGame {
            pid: 1,
            name: b"Alpha".to_vec(),
            points: 350,
        });
        assert_eq!(
            table.global_rank(),
            b"[ENTER]1. Place - Alpha - 450 points.[ENTER]2. Place - Beta - 400 points."
        );
        assert_eq!(
            table.round_rank(),
            b"[ENTER]1. Place - Beta - 400 points.[ENTER]2. Place - Alpha - 350 points.\
             [ENTER]3. Place - Alpha - 100 points."
        );
    }
}
