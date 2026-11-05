//! The dragon soul lines: `/dragon_soul`, `/ds_list` (`G/cmd_gm.cpp:5024-5059`) and
//! `/ds_change_attr` (`G/cmd_general.cpp:3864-3931`).

use common::item_slots::{
    DRAGON_SOUL_EQUIP_SLOT_END, DRAGON_SOUL_EQUIP_SLOT_START, DRAGON_SOUL_INVENTORY_MAX_NUM,
};
use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_small::GcDragonSoulChangeAttrResult;
use protocol::item_pos::ItemPos;
use world::character::{
    count_specify_item, force_worn_attribute, number, remove_specify_item, set_item_socket, Dice,
    MoveDone, MoveKind, MoveRecord, APPLY_SKILL,
};
use world::item::Item;

use super::GameState;
use crate::chat_line::Arg;
use crate::item_move::{gold_delta, MoveItemRefused, MovedItems, Mover};
use crate::loading_phase::{
    AFFECT_DRAGON_SOUL_DECK_0, AFFECT_DRAGON_SOUL_QUALIFIED, INFINITE_AFFECT_DURATION,
};

impl GameState {
    /// Share the dragon soul table boot read. Without it `/ds_change_attr` redraws
    /// nothing: every stone's type has no applies.
    #[must_use]
    pub fn with_dragon_soul_table(
        mut self,
        table: std::sync::Arc<gamedata::dragon_soul_table::DragonSoulTable>,
    ) -> Self {
        self.dragon_soul_table = table;
        self
    }
}

/// `DRAGON_SOUL_DECK_MAX_NUM` (`length.h:263`): the two decks.
const DECKS: i32 = 2;

/// `DS_SLOT_MAX` (`item_length.h:181`): the six stones a deck holds.
const DECK_SLOTS: u16 = 6;

/// `ITEM_SOCKET_DRAGON_SOUL_ACTIVE_IDX` (`item_length.h:64`): the socket an active stone's
/// `ActivateDragonSoul` writes 1 to.
const ACTIVE_SOCKET: usize = 2;

/// `ITEM_SOCKET_REMAIN_SEC` (`common/src/constants.rs`): socket 0, which
/// `IsTimeLeftDragonSoul` reads for a wear-timed stone.
const REMAIN_SOCKET: usize = 0;
/// `CHANGE_ATTR_FLAME_VNUM` (`G/DragonSoul.h:7`): the flame the redraw burns.
const FLAME_VNUM: u32 = 30524;

/// `CHANGE_ATTR_NEED_FLAME` (`G/DragonSoul.h:8`): the flames a stone's step costs.
const NEED_FLAME: [u32; 5] = [1, 3, 9, 27, 81];

/// `CHANGE_ATTR_NEED_YANG` (`G/DragonSoul.h:9`): the gold a stone's step is checked against.
const NEED_YANG: [u64; 5] = [50_000, 150_000, 450_000, 13_500_000, 4_050_000];

/// The gold `do_ds_change_attr` spends: `ChangeGold(-5000000)` (`G/cmd_general.cpp:3922`),
/// whatever the step's check named.
const SPENT_GOLD: u64 = 5_000_000;

/// `DRAGON_SOUL_ADDITIONAL_ATTR_START_IDX` (`G/dragon_soul_table.h:11`): the first
/// additional attribute's index.
const ADDITIONAL_START: usize = 3;

/// The six attributes the line clears before the redraw (`G/cmd_general.cpp:3912-3915`).
const CLEARED_ATTRIBUTES: usize = 6;

/// `IsTimeLeftDragonSoul` (`G/DragonSoul.cpp:1195-1210`): a wear-timed stone reads its socket 0,
/// any other stone is always in time.
fn time_left(stone: &Item, wear_timed: bool) -> bool {
    if wear_timed {
        stone.sockets[REMAIN_SOCKET] > 0
    } else {
        true
    }
}

/// `IsActiveDragonSoul` (`G/DragonSoul.cpp:1212-1215`): socket 2 is nonzero.
fn is_active(stone: &Item) -> bool {
    stone.sockets[ACTIVE_SOCKET] != 0
}

/// `(short)(ceil(value * factor - 0.01))` (`G/DragonSoul.cpp:267,292`): the redrawn
/// attribute's value. The casts are the C++ expression's own: `float` product, `ceil`,
/// then the `short` conversion.
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn weighed(value: i32, factor: f32) -> i16 {
    f32::ceil(value as f32 * factor - 0.01) as i16
}

/// `fnumber(from, to)` (`libthecore/utils.cpp:377-380`): the draw over `RAND_MAX` times
/// the width, plus the low bound. The draw is the dice's 31-bit number; `RAND_MAX` is
/// `0x7FFF_FFFF`, so the quotient never exceeds 1. The `f32` quotient is legacy's own
/// `(float)draw / (float)RAND_MAX`.
#[allow(clippy::cast_possible_truncation)]
fn fnumber(dice: &mut dyn Dice, from: f32, to: f32) -> f32 {
    (f64::from(dice.random31() & 0x7fff_ffff) / f64::from(0x7fff_ffffu32)) as f32 * (to - from)
        + from
}

impl GameState {
    /// Runs one `/dragon_soul` line (`do_dragon_soul`, `G/cmd_gm.cpp:5024-5047`).
    ///
    /// Any caller at any pose. The first word's first byte names the arm: `a` reads the next
    /// word as the deck (`str_to_number(int)`, silent unless nonempty) and activates it,
    /// `d` deactivates every deck, and anything else runs nothing.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when the VID has no character in the world.
    pub(super) fn run_dragon_soul(
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
        let (word, back) = crate::command::one_argument(rest);
        let mut done = MoveDone {
            kind: MoveKind::Socketed,
            records: Vec::new(),
            changes: Vec::new(),
        };
        match word.first() {
            Some(b'a') => {
                let (deck_word, _) = crate::command::one_argument(back);
                if deck_word.is_empty() {
                    return Ok(self.finish_dragon_soul(vid, owner, mover, done));
                }
                let deck = super::skill_admin::str_to_wrapping_int(&deck_word);
                self.activate_deck(vid, deck, &mut done);
            }
            Some(b'd') => self.deactivate_all(vid, &mut done),
            _ => {}
        }
        Ok(self.finish_dragon_soul(vid, owner, mover, done))
    }

    /// The moved step of a `/dragon_soul` line: the socket updates ride, the points the
    /// stones changed ride with them, and the chat lines the run told are already in the
    /// outbox.
    fn finish_dragon_soul(
        &mut self,
        vid: Vid,
        owner: u32,
        caller: Option<Mover>,
        done: MoveDone,
    ) -> MovedItems {
        let Some(caller) = caller else {
            // No body, no descriptor language: the stones still moved, with a Mover that
            // carries nothing, as `run_use_item_line`'s miss does for its silence.
            let silent = Mover {
                recently_fought: false,
                empire: 0,
                language: 0,
                pk_mode: 0,
                affect_flags: [0; 2],
                dead: false,
                alignment: 0,
                view_guild: (0, 0),
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

    /// `DragonSoul_ActivateDeck` (`G/char_dragonsoul.cpp:87-113`): a deck outside 0..2 runs
    /// nothing, the active deck answers at once, else every deck deactivates first (so the
    /// stones' updates and uncomputes precede the not-activated line), and an unqualified
    /// caller hears `The dragonsoul is not activated.` (`LC_TEXT`, so the client's own
    /// tongue). Else the deck's affect lands and its stones activate.
    fn activate_deck(&mut self, vid: Vid, deck: i32, done: &mut MoveDone) {
        if !(0..DECKS).contains(&deck) {
            return;
        }
        if self
            .bodies
            .get(&vid)
            .is_some_and(|body| body.active_deck == deck)
        {
            return;
        }
        self.deactivate_all(vid, done);
        if !self.ds_qualified(vid) {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"The dragonsoul is not activated.",
                    &[],
                );
            }
            return;
        }
        self.add_deck_affect(vid, u32::try_from(deck).unwrap_or(0));
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.active_deck = deck;
        }
        let start = DRAGON_SOUL_EQUIP_SLOT_START + DECK_SLOTS * u16::try_from(deck).unwrap_or(0);
        let end = start + DECK_SLOTS;
        for cell in start..end {
            self.activate_stone(vid, cell, deck, done);
        }
    }

    /// `DragonSoul_DeactivateAll` (`G/char_dragonsoul.cpp:115-127`): every deck stone
    /// deactivates, the deck goes to -1, and both deck affects go.
    fn deactivate_all(&mut self, vid: Vid, done: &mut MoveDone) {
        for cell in DRAGON_SOUL_EQUIP_SLOT_START..DRAGON_SOUL_EQUIP_SLOT_END {
            self.deactivate_stone(vid, cell, done);
        }
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.active_deck = -1;
        }
        self.remove_deck_affects(vid);
    }

    /// Whether the character holds the qualification (`DragonSoul_IsQualified`,
    /// `G/char_dragonsoul.cpp:68-73`): the 540 row.
    pub(super) fn ds_qualified(&self, vid: Vid) -> bool {
        self.affect_events.get(&vid).is_some_and(|event| {
            event
                .affects
                .iter()
                .any(|affect| affect.kind == AFFECT_DRAGON_SOUL_QUALIFIED)
        })
    }

    /// `AddAffect(541 + deck, APPLY_NONE, 0, 0, INFINITE, 0, false)`: the deck's row appends
    /// (no `bOverride`), computes nothing, and its add packet joins the outbox.
    fn add_deck_affect(&mut self, vid: Vid, deck: u32) {
        self.add_pc_affect(
            vid,
            super::affect::Affect {
                kind: AFFECT_DRAGON_SOUL_DECK_0 + deck,
                apply_on: 0,
                value: 0,
                flag: 0,
                remaining: INFINITE_AFFECT_DURATION,
                sp_cost: 0,
            },
            false,
        );
    }

    /// `RemoveAffect(541)` and `RemoveAffect(542)`: each deck row uncomputes (nothing), its
    /// remove packet goes, and the flags update once at the end, as `toggle_invisible`'s
    /// removal arm does.
    fn remove_deck_affects(&mut self, vid: Vid) {
        let flags_before = self.affect_flags_of(vid);
        for kind in [AFFECT_DRAGON_SOUL_DECK_0, AFFECT_DRAGON_SOUL_DECK_0 + 1] {
            let removed = match self.affect_events.get_mut(&vid) {
                Some(event) => super::affect::take_affects_of_kind(&mut event.affects, kind),
                None => Vec::new(),
            };
            for affect in &removed {
                self.compute_affect(vid, affect, false);
                let frame = super::affect::affect_remove_frame(affect);
                let _retired = self.write_to_client(vid, frame);
            }
        }
        let flags_after = self.affect_flags_of(vid);
        if flags_after != flags_before {
            self.send_affect_update(vid, flags_after);
        }
    }

    /// `DSManager::ActivateDragonSoul` (`G/DragonSoul.cpp:1217-1247`) of the stone in the
    /// flat deck `cell`: a stone in the active deck's span with time left and socket 2 clear
    /// applies its points, writes socket 2 to 1 (the update rides, the row is saved), and
    /// its wear timer starts — which the Rewrite has no machinery for yet, so the wait is
    /// on the timer, not the line. Anything else answers without touching the stone.
    fn activate_stone(&mut self, vid: Vid, cell: u16, deck: i32, done: &mut MoveDone) {
        let deck = u16::try_from(deck).unwrap_or(0);
        let start = DRAGON_SOUL_EQUIP_SLOT_START + DECK_SLOTS * deck;
        if cell < start || cell >= start + DECK_SLOTS {
            return;
        }
        let at =
            protocol::item_pos::ItemPos::new(common::item_slots::EWindows::Inventory as u8, cell);
        let Some(stone) = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at).cloned())
        else {
            return;
        };
        let Some(proto) = self.protos.get(stone.vnum) else {
            return;
        };
        let wear_timed = proto.timer_based_on_wear.is_some();
        if !time_left(&stone, wear_timed) || is_active(&stone) {
            return;
        }
        let applies = world::character::item_applies(&stone, proto, &self.protos);
        // `ModifyPoints(true)` (`G/item.cpp:781-1010`): each apply through `ApplyPoint`, with
        // the current maxima's share kept, not a raw point change.
        if let Some(points) = self.pc_points_mut(vid.raw()) {
            let mut records = Vec::new();
            for (kind, value) in &applies {
                records.extend(points.apply_point(*kind, *value));
            }
            self.send_point_records(vid, &records);
        }
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let step = set_item_socket(character.items_mut(), stone.id, (ACTIVE_SOCKET, 1));
        done.records.extend(step.records);
        done.changes.extend(step.changes);
    }

    /// `DSManager::DeactivateDragonSoul(item, true)` (`G/DragonSoul.cpp:1249-1279`): a stone
    /// with socket 2 set writes it back to 0 (the update rides, the row is saved) and
    /// unapplies its points. The skip flag is always true here, so no refresh runs.
    fn deactivate_stone(&mut self, vid: Vid, cell: u16, done: &mut MoveDone) {
        let at =
            protocol::item_pos::ItemPos::new(common::item_slots::EWindows::Inventory as u8, cell);
        let Some(stone) = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at).cloned())
        else {
            return;
        };
        if !is_active(&stone) {
            return;
        }
        let applies = self
            .protos
            .get(stone.vnum)
            .map(|proto| world::character::item_applies(&stone, proto, &self.protos))
            .unwrap_or_default();
        for (kind, value) in &applies {
            // `ModifyPoints(false)` (`G/item.cpp:781-1010`): each apply unapplied through
            // `ApplyPoint`, except the skill type, which legacy XORs with `0x00800000`
            // rather than negating.
            let back = if *kind == APPLY_SKILL {
                *value ^ 0x0080_0000
            } else {
                -*value
            };
            if let Some(points) = self.pc_points_mut(vid.raw()) {
                let records = points.apply_point(*kind, back);
                self.send_point_records(vid, &records);
            }
        }
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let step = set_item_socket(character.items_mut(), stone.id, (ACTIVE_SOCKET, 0));
        done.records.extend(step.records);
        done.changes.extend(step.changes);
    }

    /// Runs one `/ds_list` line (`do_ds_list`, `G/cmd_gm.cpp:5049-5059`): the caller hears
    /// `cell : %d, name : %s, id : %d` for each held dragon soul stone, in cell order. The
    /// name is the proto's `szLocaleName`, which is the item-names file's row in the
    /// caller's language; nothing rides and nothing is stored.
    pub(super) fn run_ds_list(&mut self, vid: Vid) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let stones: Vec<(u16, u32, u32)> = self
            .characters
            .find_by_vid(vid)
            .map(|character| {
                character
                    .items()
                    .occupied()
                    .into_iter()
                    .filter(|(pos, _)| {
                        pos.window_type == common::item_slots::EWindows::DragonSoulInventory as u8
                    })
                    .filter_map(|(pos, id)| {
                        character
                            .items()
                            .item(id)
                            .map(|item| (pos.cell, item.vnum, item.id))
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (cell, vnum, id) in stones {
            let name = self
                .protos
                .get(vnum)
                .map(|proto| proto.locale_name.clone())
                .unwrap_or_default();
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"cell : %d, name : %s, id : %d",
                &[
                    Arg::Int(i64::from(cell)),
                    Arg::Text(&name),
                    Arg::Int(i64::from(id)),
                ],
            );
        }
    }

    /// Runs one `/ds_change_attr` line (`do_ds_change_attr`, `G/cmd_general.cpp:3864-3931`).
    ///
    /// The word is the soul-inventory cell (`str_to_number(DWORD)`, silent past
    /// `DRAGON_SOUL_INVENTORY_MAX_NUM`). A trade, a browsed shop, an open safebox, the
    /// cube (no ported path opens it) or a personal shop (none exists) answers the
    /// cannot-open line. An empty cell, an exchanging stone, a non-soul item, or an
    /// active stone runs nothing past the equipped line; too little gold or too few
    /// flames answers its line. Else the six attributes clear, the table redraws them
    /// ([`Self::change_attributes`]), the flames and 5,000,000 gold go, and the result
    /// rides to the view.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when the VID has no character in the world.
    pub(super) fn run_ds_change_attr(
        &mut self,
        vid: Vid,
        rest: &[u8],
        mover: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let owner = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?
            .player_id();
        let mut done = MoveDone {
            kind: MoveKind::Attributed,
            records: Vec::new(),
            changes: Vec::new(),
        };
        let (word, _) = crate::command::one_argument(rest);
        if word.is_empty() {
            return Ok(self.finish_change_attr(vid, owner, mover, done));
        }
        if self.trading.contains_key(&vid)
            || self.browsing.contains_key(&vid)
            || self.safebox_open(vid)
        {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"@@(cmd_general)tradus:dragonsoul_plus_cannot_open",
                &[],
            );
            return Ok(self.finish_change_attr(vid, owner, mover, done));
        }
        let Some((id, vnum)) = self.change_attr_stone(vid, mover, &word) else {
            return Ok(self.finish_change_attr(vid, owner, mover, done));
        };
        let step = (vnum / 100) % 10;
        let Some(need_flame) = NEED_FLAME.get(usize::try_from(step).unwrap_or(usize::MAX)) else {
            return Ok(self.finish_change_attr(vid, owner, mover, done));
        };
        let need_flame = *need_flame;
        if !self.change_attr_funded(vid, mover, step, need_flame) {
            return Ok(self.finish_change_attr(vid, owner, mover, done));
        }
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            let (items, _, _) = character.items_points_and_quickslots_mut();
            for index in 0..CLEARED_ATTRIBUTES {
                let cleared = force_worn_attribute(items, id, (index, 0, 0));
                done.records.extend(cleared.records);
                done.changes.extend(cleared.changes);
            }
        }
        let redrawn = self.change_attributes(vid, id, vnum, &mut done);
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            if redrawn {
                b"@@(cmd_general)tradus:dragonsoul_plus_change_success".as_slice()
            } else {
                b"@@(cmd_general)tradus:dragonsoul_plus_change_fail".as_slice()
            },
            &[],
        );
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            let usable = common::item_slots::usable_inventory_cells(character.inven_point());
            let protos = self.protos.clone();
            let (items, _, slots) = character.items_points_and_quickslots_mut();
            let spent =
                remove_specify_item(items, slots, &protos, usable, (FLAME_VNUM, need_flame));
            done.records.extend(spent.records);
            done.changes.extend(spent.changes);
        }
        // `ChangeGold(-5000000)`: the record shows no amount for a loss, and the store
        // takes the delta the Transfer adds.
        let held = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::gold);
        let left = held - SPENT_GOLD;
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            character.set_gold(left);
        }
        done.records.push(MoveRecord::Gold {
            amount: 0,
            value: left,
        });
        let mut packed = self.finish_change_attr(vid, owner, mover, done);
        packed.gold = Some(gold_delta(held, left));
        let mut result = Vec::new();
        GcDragonSoulChangeAttrResult {
            result: u8::from(redrawn),
        }
        .encode_into(&mut result);
        self.packet_around(super::view::EntityKey::Character(vid.raw()), &result, None);
        Ok(packed)
    }

    /// The stone the word names: the soul-inventory cell's id and vnum, or `None` for an
    /// empty cell, an exchanging stone, a non-soul item, or a past-end cell. An active
    /// stone tells the equipped line and answers `None`.
    fn change_attr_stone(&self, vid: Vid, mover: Mover, word: &[u8]) -> Option<(u32, u32)> {
        let cell = super::skill_admin::str_to_dword(word);
        // `dwPos < 0` is dead on a `DWORD`; the range check is the upper bound alone.
        if u64::from(cell) >= u64::from(DRAGON_SOUL_INVENTORY_MAX_NUM) {
            return None;
        }
        let pos = ItemPos::new(
            common::item_slots::EWindows::DragonSoulInventory as u8,
            u16::try_from(cell).unwrap_or(u16::MAX),
        );
        let (id, vnum) = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| {
                let stone = character.items().item_at(pos)?;
                // `IsEquipped` is dead here: a deck stone lives in the deck cells, never in
                // the soul inventory the line reads.
                if character.items().is_exchanging(stone.id) {
                    return None;
                }
                Some((stone.id, stone.vnum))
            })?;
        let is_soul = self
            .protos
            .get(vnum)
            .is_some_and(|proto| proto.item_type == gamedata::item_kind::ITEM_DS);
        if !is_soul {
            return None;
        }
        if self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item(id))
            .is_some_and(is_active)
        {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"@@(cmd_general)tradus:dragonsoul_plus_cannot_change_equiped",
                &[],
            );
            return None;
        }
        Some((id, vnum))
    }

    /// The gold and flame gates: the held gold reaches the step's `NEED_YANG` and the
    /// flames reach `need_flame`, else the caller hears the short line. Answers whether
    /// the redraw is funded.
    fn change_attr_funded(&self, vid: Vid, mover: Mover, step: u32, need_flame: u32) -> bool {
        let index = usize::try_from(step).unwrap_or(usize::MAX) % NEED_YANG.len();
        let need_yang = NEED_YANG[index];
        let held = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::gold);
        if held < need_yang {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"@@(cmd_general)tradus:dragonsoul_plus_not_enough_yangs%i",
                &[Arg::Int(i64::try_from(need_yang).unwrap_or(i64::MAX))],
            );
            return false;
        }
        let usable = self.characters.find_by_vid(vid).map_or(0, |character| {
            common::item_slots::usable_inventory_cells(character.inven_point())
        });
        let flames = self.characters.find_by_vid(vid).map_or(0, |character| {
            count_specify_item(character.items(), usable, FLAME_VNUM)
        });
        if flames < i32::try_from(need_flame).unwrap_or(i32::MAX) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"@@(cmd_general)tradus:dragonsoul_plus_not_enough_items",
                &[],
            );
            return false;
        }
        true
    }

    /// `DSManager::ChangeAttributes` (`G/DragonSoul.cpp:221-300`): the stone's attributes
    /// redrawn from the table, into `done`. The basic applies land in order, capped at the
    /// rows the type has; then `number(add_min, add_max)` capped at three distinct
    /// additionals, drawn by weight. Each value is `ceil(value * weight / 100 - 0.01)` as
    /// an `i16`. Answers whether the redraw ran: a stone whose type, numbers or weight
    /// the table lacks, or whose distinct draw fails, keeps its cleared attributes, and
    /// nothing is written.
    fn change_attributes(&mut self, vid: Vid, id: u32, vnum: u32, done: &mut MoveDone) -> bool {
        let ds_type = u8::try_from(vnum / 10_000).unwrap_or(u8::MAX);
        let grade = u8::try_from((vnum / 1000) % 10).unwrap_or(u8::MAX);
        let step = u8::try_from((vnum / 100) % 10).unwrap_or(u8::MAX);
        let strength = u8::try_from((vnum / 10) % 10).unwrap_or(u8::MAX);
        let table = self.dragon_soul_table.clone();
        let (Some(plain), Some(additionals), Some((basis, add_min, add_max)), Some(weight)) = (
            table.basic_applies(ds_type).map(<[_]>::to_vec),
            table.additional_applies(ds_type).map(<[_]>::to_vec),
            table.apply_numbers(grade),
            table.weight(grade, step, strength),
        ) else {
            return false;
        };
        let factor = weight / 100.0;
        let mut pairs: Vec<(usize, u8, i16)> = Vec::new();
        for (index, apply) in plain
            .iter()
            .enumerate()
            .take(usize::try_from(basis.max(0)).unwrap_or(usize::MAX))
        {
            pairs.push((
                index,
                u8::try_from(apply.apply_type).unwrap_or(u8::MAX),
                weighed(apply.apply_value, factor),
            ));
        }
        let draws = number(&mut self.dice, add_min, add_max).min(3);
        if draws > 0 {
            let probs: Vec<f32> = additionals.iter().map(|apply| apply.prob).collect();
            let count = usize::try_from(draws).unwrap_or(0);
            let Some(picked) = self.distinct_draws(&probs, count) else {
                return false;
            };
            for (slot, which) in picked.iter().enumerate() {
                let Some(apply) = additionals.get(*which) else {
                    return false;
                };
                pairs.push((
                    ADDITIONAL_START + slot,
                    u8::try_from(apply.apply_type).unwrap_or(u8::MAX),
                    weighed(apply.apply_value, factor),
                ));
            }
        }
        // Every read passed, so the pairs land: each through `SetForceAttribute`, with
        // its update and its saved row.
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            let (items, _, _) = character.items_points_and_quickslots_mut();
            for (index, b_type, s_value) in pairs {
                let forced = force_worn_attribute(items, id, (index, b_type, s_value));
                done.records.extend(forced.records);
                done.changes.extend(forced.changes);
            }
        }
        true
    }

    /// `MakeDistinctRandomNumberSet` (`G/DragonSoul.cpp:35-66`): `count` distinct indexes
    /// into `probs`, each drawn by weight from the weights left. A draw is
    /// `fnumber(0, range)`, and the first weight whose running sum reaches it wins; a
    /// draw no sum reaches (a zero range) picks nothing, and the set fails. `None`
    /// when `count` outnumbers the weights.
    fn distinct_draws(&mut self, probs: &[f32], count: usize) -> Option<Vec<usize>> {
        if probs.len() < count {
            return None;
        }
        let mut left: Vec<(usize, f32)> = probs.iter().copied().enumerate().collect();
        let mut picked = Vec::with_capacity(count);
        for _ in 0..count {
            let range: f32 = left.iter().map(|(_, prob)| *prob).sum();
            let rolled = fnumber(&mut self.dice, 0.0, range);
            let mut sum = 0.0;
            let mut won = None;
            for (slot, (index, prob)) in left.iter().enumerate() {
                sum += *prob;
                if sum >= rolled {
                    won = Some((slot, *index));
                    break;
                }
            }
            let (slot, index) = won?;
            left.remove(slot);
            picked.push(index);
        }
        Some(picked)
    }

    /// The moved step of a `/ds_change_attr` line: the updates ride, the gold delta and
    /// the points ride with them, and the chat lines the run told are already in the
    /// outbox.
    fn finish_change_attr(
        &mut self,
        vid: Vid,
        owner: u32,
        mover: Mover,
        done: MoveDone,
    ) -> MovedItems {
        let mut step = MovedItems::new(owner, vid.raw(), done, mover, &self.protos, &self.locale);
        step.points = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points().cloned());
        step
    }
}

#[cfg(test)]
mod tests {
    use common::item_slots::EWindows;
    use common::vid::Vid;
    use protocol::gc_inventory::HEADER_GC_CHAT;
    use protocol::gc_nested::GcAffectAdd;
    use protocol::gc_vid::HEADER_GC_AFFECT_REMOVE;
    use protocol::item_pos::ItemPos;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::item::Item;

    use super::*;
    use crate::game_state::fixtures::{a_world, enter_seeing_with, TestClock, PLACE};
    use crate::loading_phase::AFFECT_DRAGON_SOUL_QUALIFIED;

    /// The owner's first dragon soul stone: vnum 110000, a wear-timed `ITEM_DS`.
    const STONE: u32 = 110_000;

    /// A stone of `id` and `vnum`, with `sockets` and no attributes.
    fn a_stone(id: u32, vnum: u32, sockets: [i32; 6]) -> Item {
        let mut stone = Item::new(id, vnum);
        for (slot, value) in sockets.iter().enumerate() {
            stone.set_socket(slot, *value).expect("six sockets");
        }
        stone
    }

    /// The flat cell of deck 0's first stone: 244.
    fn deck_cell() -> ItemPos {
        ItemPos::new(EWindows::Inventory as u8, DRAGON_SOUL_EQUIP_SLOT_START)
    }

    /// A world with one entered player holding `items`, and its inbox drained of the enter.
    fn entered(items: &[(ItemPos, Item)]) -> (GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let (mut inbox, _) = enter_seeing_with(&mut world, PLACE, 1, (5, 5), 10_000, items);
        while inbox.try_recv().is_ok() {}
        (world, inbox)
    }

    /// The qualification row, as `DragonSoul_GiveQualification` lays it.
    fn qualify(world: &mut GameState, vid: Vid) {
        world.add_pc_affect(
            vid,
            super::super::affect::Affect {
                kind: AFFECT_DRAGON_SOUL_QUALIFIED,
                apply_on: 0,
                value: 0,
                flag: 0,
                remaining: INFINITE_AFFECT_DURATION,
                sp_cost: 0,
            },
            false,
        );
    }

    /// The affect-add kinds the inbox heard, in order.
    fn added(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<u32> {
        std::iter::from_fn(|| inbox.try_recv().ok())
            .filter(|frame| frame.first() == Some(&GcAffectAdd::header()))
            .map(|frame| u32::from_le_bytes(frame[1..5].try_into().expect("four bytes")))
            .collect()
    }

    /// An unqualified caller's deck line deactivates first, then hears the not-activated
    /// line; a qualified caller's lands the deck affect and writes socket 2, and `d`
    /// takes it back with the deck affect's remove.
    #[test]
    fn the_deck_line_qualifies_then_lands_and_the_d_arm_takes_it_back() {
        let vid = Vid::new(1);
        // A wear-timed stone with an hour left, in deck 0's first cell.
        let stone = a_stone(7, STONE, [3600, 0, 0, 0, 0, 0]);
        let (mut world, mut inbox) = entered(&[(deck_cell(), stone)]);
        assert_eq!(world.bodies[&vid].active_deck, -1, "no deck at enter");
        // Unqualified: the not-activated line, no deck, socket 2 untouched.
        let moved = world.run_dragon_soul(vid, b"a 0").expect("the line runs");
        assert!(
            moved.records.is_empty(),
            "no socket update without the deck"
        );
        assert_eq!(
            world.bodies[&vid].active_deck, -1,
            "no deck lands unqualified"
        );
        let heard: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert!(
            heard.iter().any(|frame| frame
                .windows(b"The dragonsoul is not activated.".len())
                .any(|window| window == b"The dragonsoul is not activated.")),
            "the not-activated line: {heard:02x?}"
        );
        while inbox.try_recv().is_ok() {}
        // Qualified: the 541 add, deck 0, socket 2 set with its update riding.
        qualify(&mut world, vid);
        while inbox.try_recv().is_ok() {}
        let moved = world.run_dragon_soul(vid, b"a 0").expect("the line runs");
        assert_eq!(world.bodies[&vid].active_deck, 0, "deck 0 is active");
        assert_eq!(
            added(&mut inbox),
            [AFFECT_DRAGON_SOUL_DECK_0],
            "the deck add"
        );
        assert_eq!(moved.records.len(), 1, "the stone's update rides");
        assert_eq!(moved.changes.len(), 1, "the stone's row is saved");
        let held = world
            .characters
            .find_by_vid(vid)
            .expect("the player")
            .items()
            .item_at(deck_cell())
            .expect("the stone is still worn");
        assert_eq!(held.sockets[ACTIVE_SOCKET], 1, "socket 2 is set");
        // The deck again: nothing answers twice.
        while inbox.try_recv().is_ok() {}
        let moved = world.run_dragon_soul(vid, b"a 0").expect("the line runs");
        assert!(moved.records.is_empty(), "the active deck answers at once");
        assert!(inbox.try_recv().is_err(), "nothing is told twice");
        // `d`: socket 2 clears with its update, the deck goes to -1, the 541 remove.
        let moved = world.run_dragon_soul(vid, b"d").expect("the line runs");
        assert_eq!(world.bodies[&vid].active_deck, -1, "no deck after d");
        assert_eq!(moved.records.len(), 1, "the clear's update rides");
        let held = world
            .characters
            .find_by_vid(vid)
            .expect("the player")
            .items()
            .item_at(deck_cell())
            .expect("the stone is still worn");
        assert_eq!(held.sockets[ACTIVE_SOCKET], 0, "socket 2 is clear");
        let removed: Vec<u32> = std::iter::from_fn(|| inbox.try_recv().ok())
            .filter(|frame| frame.first() == Some(&HEADER_GC_AFFECT_REMOVE))
            .map(|frame| u32::from_le_bytes(frame[1..5].try_into().expect("four bytes")))
            .collect();
        assert_eq!(removed, [AFFECT_DRAGON_SOUL_DECK_0], "the deck remove");
    }

    /// A deck outside 0..2, an empty deck word, a wordless line and any other first byte
    /// run nothing: no deck, no socket, no line.
    #[test]
    fn the_deck_line_refuses_a_bad_deck_and_any_other_first_byte_silently() {
        let vid = Vid::new(1);
        let stone = a_stone(7, STONE, [3600, 0, 0, 0, 0, 0]);
        let (mut world, mut inbox) = entered(&[(deck_cell(), stone)]);
        qualify(&mut world, vid);
        while inbox.try_recv().is_ok() {}
        for rest in [&b"a 2"[..], b"a -1", b"a", b"", b"x 0", b"A 0"] {
            let moved = world.run_dragon_soul(vid, rest).expect("the line runs");
            assert!(moved.records.is_empty(), "no update for {rest:?}");
            assert!(moved.changes.is_empty(), "no row for {rest:?}");
        }
        assert_eq!(world.bodies[&vid].active_deck, -1, "no deck lands");
        assert!(inbox.try_recv().is_err(), "nothing is told");
        let held = world
            .characters
            .find_by_vid(vid)
            .expect("the player")
            .items()
            .item_at(deck_cell())
            .expect("the stone is still worn");
        assert_eq!(held.sockets[ACTIVE_SOCKET], 0, "socket 2 stays clear");
    }

    /// A wear-timed stone with no time left stays down: the deck lands, but socket 2
    /// stays clear with no update and no row.
    #[test]
    fn a_stone_with_no_time_left_stays_down_while_the_deck_lands() {
        let vid = Vid::new(1);
        let stone = a_stone(7, STONE, [0, 0, 0, 0, 0, 0]);
        let (mut world, mut inbox) = entered(&[(deck_cell(), stone)]);
        qualify(&mut world, vid);
        while inbox.try_recv().is_ok() {}
        let moved = world.run_dragon_soul(vid, b"a 0").expect("the line runs");
        assert_eq!(world.bodies[&vid].active_deck, 0, "the deck lands");
        assert_eq!(
            added(&mut inbox),
            [AFFECT_DRAGON_SOUL_DECK_0],
            "the deck add"
        );
        assert!(moved.records.is_empty(), "no update for a spent stone");
        assert!(moved.changes.is_empty(), "no row for a spent stone");
        let held = world
            .characters
            .find_by_vid(vid)
            .expect("the player")
            .items()
            .item_at(deck_cell())
            .expect("the stone is still worn");
        assert_eq!(held.sockets[ACTIVE_SOCKET], 0, "socket 2 stays clear");
    }

    /// Deck 1's stones answer deck 1, and a switch deactivates deck 0's stone before deck
    /// 1's lands: both updates ride in that order.
    #[test]
    fn the_second_deck_lands_and_a_switch_deactivates_before_it_lands() {
        let vid = Vid::new(1);
        let first = a_stone(7, STONE, [3600, 0, 0, 0, 0, 0]);
        let second_at = ItemPos::new(
            EWindows::Inventory as u8,
            DRAGON_SOUL_EQUIP_SLOT_START + DECK_SLOTS,
        );
        let second = a_stone(8, STONE, [3600, 0, 0, 0, 0, 0]);
        let (mut world, mut inbox) = entered(&[(deck_cell(), first), (second_at, second)]);
        qualify(&mut world, vid);
        while inbox.try_recv().is_ok() {}
        world.run_dragon_soul(vid, b"a 0").expect("deck 0 runs");
        while inbox.try_recv().is_ok() {}
        let moved = world.run_dragon_soul(vid, b"a 1").expect("deck 1 runs");
        assert_eq!(world.bodies[&vid].active_deck, 1, "deck 1 is active");
        assert_eq!(
            added(&mut inbox),
            [AFFECT_DRAGON_SOUL_DECK_0 + 1],
            "the deck-1 add"
        );
        assert_eq!(moved.records.len(), 2, "both stones' updates ride");
        let items = &world
            .characters
            .find_by_vid(vid)
            .expect("the player")
            .items()
            .clone();
        assert_eq!(
            items.item_at(deck_cell()).expect("deck 0's stone").sockets[ACTIVE_SOCKET],
            0,
            "deck 0's stone goes down first"
        );
        assert_eq!(
            items.item_at(second_at).expect("deck 1's stone").sockets[ACTIVE_SOCKET],
            1,
            "deck 1's stone lands"
        );
    }

    /// `/ds_list` names each held stone in cell order, with the item-names row in the
    /// caller's language; with no stone it stays silent.
    #[test]
    fn the_list_names_each_held_stone_and_stays_silent_with_none() {
        let vid = Vid::new(1);
        let ds_at = |cell: u16| ItemPos::new(EWindows::DragonSoulInventory as u8, cell);
        let first = a_stone(7, STONE, [0, 0, 0, 0, 0, 0]);
        let second = a_stone(8, STONE, [0, 0, 0, 0, 0, 0]);
        let (mut world, mut inbox) = entered(&[(ds_at(3), first), (ds_at(1), second)]);
        world.run_ds_list(vid);
        let heard: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert_eq!(heard.len(), 2, "one line per stone: {heard:02x?}");
        // `occupied` walks the array in order, so cell 1's line precedes cell 3's.
        let text = |frame: &[u8]| frame[10..].to_vec();
        assert_eq!(
            text(&heard[0]),
            b"cell : 1, name : Diamant Dragon Brut, id : 8".to_vec(),
            "cell 1 first"
        );
        assert_eq!(
            text(&heard[1]),
            b"cell : 3, name : Diamant Dragon Brut, id : 7".to_vec(),
            "then cell 3"
        );
        let (mut world, mut inbox) = entered(&[]);
        world.run_ds_list(vid);
        // The revive update may follow the enter; the list itself tells nothing.
        let heard: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok())
            .filter(|frame| frame.first() == Some(&HEADER_GC_CHAT.value()))
            .collect();
        assert!(heard.is_empty(), "no stone, no line: {heard:02x?}");
    }

    /// A world with the owner's soul table loaded, one entered player holding `items`,
    /// `gold`, and its inbox drained of the enter.
    fn entered_with_table(
        items: &[(ItemPos, Item)],
        gold: u64,
    ) -> (GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        let table = gamedata::dragon_soul_table::DragonSoulTable::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../legacy/gamedata/locale/europe"),
        )
        .expect("the owner's soul table loads");
        let mut world = a_world(&clock).with_dragon_soul_table(std::sync::Arc::new(table));
        let (mut inbox, _) = enter_seeing_with(&mut world, PLACE, 1, (5, 5), 10_000, items);
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .expect("the player is in the world")
            .set_gold(gold);
        while inbox.try_recv().is_ok() {}
        (world, inbox)
    }

    /// A stack of `count` flames (`30524`) of `id`.
    fn a_flame(id: u32, count: u16) -> Item {
        let mut flame = Item::new(id, FLAME_VNUM);
        flame.count = count;
        flame
    }

    /// The inbox's frames, drained.
    fn heard(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// The chat texts of `frames`, in order.
    fn chats(frames: &[Vec<u8>]) -> Vec<Vec<u8>> {
        frames
            .iter()
            .filter(|frame| frame.first() == Some(&HEADER_GC_CHAT.value()))
            .map(|frame| frame[10..].to_vec())
            .collect()
    }

    /// The change-attribute result bytes of `frames`, in order.
    fn results(frames: &[Vec<u8>]) -> Vec<u8> {
        frames
            .iter()
            .filter(|frame| {
                frame.first()
                    == Some(&protocol::gc_inventory::HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT.value())
            })
            .map(|frame| frame[1])
            .collect()
    }

    /// A funded redraw runs: the stone's attribute 0 lands its weighed basic apply, the
    /// flame and the 5 million are spent with the updates riding and the rows saved, the
    /// caller hears success, and the watchers hear result 1.
    #[test]
    fn the_redraw_lands_and_spends_and_the_watchers_hear_its_result() {
        let vid = Vid::new(1);
        let ds_at = |cell: u16| ItemPos::new(EWindows::DragonSoulInventory as u8, cell);
        let stone = a_stone(7, STONE, [0, 0, 0, 0, 0, 0]);
        let flame_at = ItemPos::new(EWindows::Inventory as u8, 0);
        let (mut world, mut inbox) =
            entered_with_table(&[(ds_at(1), stone), (flame_at, a_flame(9, 3))], 6_000_000);
        let mover = world.own_mover(vid).expect("the entrant moves");
        let step = world
            .run_ds_change_attr(vid, b"1", mover)
            .expect("the line runs");
        assert!(!step.records.is_empty(), "the updates ride");
        assert!(!step.changes.is_empty(), "the rows save");
        // Diamond's first basic is `INT 10`; grade 0, step 0, strength 0 weighs 1, so
        // attribute 0 is `ceil(10 * 0.01 - 0.01)` as an `i16`: 1.
        let held = world
            .characters
            .find_by_vid(vid)
            .expect("the player")
            .items()
            .item_at(ds_at(1))
            .expect("the stone stays")
            .clone();
        assert_eq!(
            (held.attributes[0].b_type, held.attributes[0].s_value),
            (4, 1),
            "the weighed basic lands"
        );
        assert!(
            held.attributes[1..]
                .iter()
                .all(|attr| attr.b_type == 0 && attr.s_value == 0),
            "grade 0 draws no additional: {:?}",
            held.attributes
        );
        let player = world.characters.find_by_vid(vid).expect("the player");
        assert_eq!(player.gold(), 1_000_000, "the 5 million are spent");
        let usable = common::item_slots::usable_inventory_cells(player.inven_point());
        assert_eq!(
            count_specify_item(player.items(), usable, FLAME_VNUM),
            2,
            "step 0 costs one flame"
        );
        let frames = heard(&mut inbox);
        assert_eq!(
            chats(&frames),
            [b"@@(cmd_general)tradus:dragonsoul_plus_change_success".to_vec()],
            "the caller hears success"
        );
        assert_eq!(results(&frames), [1], "the watchers hear result 1");
    }

    /// A wordless line, a past-end cell, an empty cell, and a non-soul item run
    /// nothing: no spend, no line, no result.
    #[test]
    fn the_line_is_silent_past_the_cells_and_on_any_other_item() {
        let vid = Vid::new(1);
        let ds_at = |cell: u16| ItemPos::new(EWindows::DragonSoulInventory as u8, cell);
        // A +0 stick (17), which is no soul stone, in cell 2.
        let stick = Item::new(11, 17);
        let flame_at = ItemPos::new(EWindows::Inventory as u8, 0);
        let (mut world, mut inbox) = entered_with_table(
            &[
                (ds_at(1), a_stone(7, STONE, [0, 0, 0, 0, 0, 0])),
                (ds_at(2), stick),
                (flame_at, a_flame(9, 1)),
            ],
            6_000_000,
        );
        let mover = world.own_mover(vid).expect("the entrant moves");
        for rest in [&b""[..], b"9999", b"5", b"2"] {
            let step = world
                .run_ds_change_attr(vid, rest, mover)
                .expect("the line runs");
            assert!(step.records.is_empty(), "no update for {rest:?}");
            assert!(step.changes.is_empty(), "no row for {rest:?}");
            assert!(step.gold.is_none(), "no spend for {rest:?}");
        }
        assert!(
            heard(&mut inbox).is_empty(),
            "no line and no result for any refused word"
        );
        let player = world.characters.find_by_vid(vid).expect("the player");
        assert_eq!(player.gold(), 6_000_000, "no gold spent");
        let usable = common::item_slots::usable_inventory_cells(player.inven_point());
        assert_eq!(
            count_specify_item(player.items(), usable, FLAME_VNUM),
            1,
            "no flame spent"
        );
    }

    /// An active stone tells the equipped line; short gold and short flames tell their
    /// own lines. All three spend nothing and send no result.
    #[test]
    fn the_gates_tell_their_lines_and_spend_nothing() {
        let vid = Vid::new(1);
        let ds_at = |cell: u16| ItemPos::new(EWindows::DragonSoulInventory as u8, cell);
        let flame_at = ItemPos::new(EWindows::Inventory as u8, 0);
        // An active stone: socket 2 set.
        let (mut world, mut inbox) = entered_with_table(
            &[
                (ds_at(1), a_stone(7, STONE, [0, 0, 1, 0, 0, 0])),
                (flame_at, a_flame(9, 1)),
            ],
            6_000_000,
        );
        let mover = world.own_mover(vid).expect("the entrant moves");
        let step = world
            .run_ds_change_attr(vid, b"1", mover)
            .expect("the line runs");
        assert!(step.records.is_empty(), "no update for an active stone");
        let frames = heard(&mut inbox);
        assert_eq!(
            chats(&frames),
            [b"@@(cmd_general)tradus:dragonsoul_plus_cannot_change_equiped".to_vec()],
            "the active stone hears the equipped line"
        );
        assert!(results(&frames).is_empty(), "no result for an active stone");
        // Short gold: step 0 wants 50,000 held.
        let (mut world, mut inbox) = entered_with_table(
            &[
                (ds_at(1), a_stone(7, STONE, [0, 0, 0, 0, 0, 0])),
                (flame_at, a_flame(9, 1)),
            ],
            40_000,
        );
        let mover = world.own_mover(vid).expect("the entrant moves");
        world
            .run_ds_change_attr(vid, b"1", mover)
            .expect("the line runs");
        let frames = heard(&mut inbox);
        assert!(
            chats(&frames).iter().any(|line| line
                .starts_with(b"@@(cmd_general)tradus:dragonsoul_plus_not_enough_yangs".as_slice())),
            "short gold hears its line: {:?}",
            chats(&frames)
        );
        assert!(results(&frames).is_empty(), "no result for short gold");
        // Short flames: step 0 wants one.
        let (mut world, mut inbox) = entered_with_table(
            &[(ds_at(1), a_stone(7, STONE, [0, 0, 0, 0, 0, 0]))],
            6_000_000,
        );
        let mover = world.own_mover(vid).expect("the entrant moves");
        world
            .run_ds_change_attr(vid, b"1", mover)
            .expect("the line runs");
        let frames = heard(&mut inbox);
        assert_eq!(
            chats(&frames),
            [b"@@(cmd_general)tradus:dragonsoul_plus_not_enough_items".to_vec()],
            "short flames hear their line"
        );
        assert!(results(&frames).is_empty(), "no result for short flames");
        let player = world.characters.find_by_vid(vid).expect("the player");
        assert_eq!(player.gold(), 6_000_000, "no gold spent");
    }
}
