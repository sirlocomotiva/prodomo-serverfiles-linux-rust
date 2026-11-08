//! The cube window (`ENABLE_CUBE_RENEWAL_WORLDARD`): `/cube open` (`do_cube`,
//! `G/cmd_general.cpp:3256-3280`), which stands the window on the clicked NPC, and
//! `CG_CUBE_RENEWAL` (`CInputMain::CubeRenewalSend`, `G/input_main.cpp:3464-3488`),
//! which crafts and closes.
//!
//! The open (`Cube_open`, `G/cuberenewal.cpp:317-359`) reads the quest NPC: the NPC the
//! last `CG_ON_CLICK` named (the manager's `npc_vid`, which `click` records). Without
//! one it is silent; on an NPC no recipe names it is silent too (`FN_check_valid_npc`).
//! A trade, a shop (either side), an open safebox, an open cube or an open aura window
//! chats `Cannot open crafting window`, and an NPC at `DISTANCE_APPROX` 1000 or past is
//! silent, as legacy's log is. The answer is four sends: `CLEAR_DATES_RECEIVE`, one
//! `DATES_RECEIVE` per recipe of the NPC's list (in file order), `DATES_LOADING`, then
//! `OPEN_RECEIVE`.
//!
//! Each date send carries its recipe: the reward vnum and count, the stackable byte (the
//! proto's `STACKABLE` flag or no `ANTI_STACK`), the first five materials (zeroed past
//! the end, so a six-material recipe's sixth never rides — kept, `:541-556`), the gold,
//! the percent, and the category truncated to its 100 bytes (legacy's `memcpy` overflows
//! past 100; the Rewrite truncates, a Divergence). The `npc_vnum` and `index` words are
//! never written: every date on the wire carries 0 for both, a Defect kept.
//!
//! The make (`Cube_Make`, `:368-516`) needs the window open, else it chats
//! `To create an item you have to have the crafting window open`. The `index` names the
//! place in the NPC's list; a miss is silent. Short materials chat
//! `You don't have the necessary materials`; short gold chats
//! `You don't have the necessary amount of gold` and neither spends.
//!
//! A landed make spends an improve item first: the base-inventory cell
//! `index_item_improve` names (`GetInventoryItem`, so a worn cell or anything at or past
//! its end misses), unless it is -1. A held stack of 40 or fewer lends its whole count
//! when the sum stays at or under 100, else what 100 leaves when the recipe is below it;
//! a stack past 40 lends nothing. The lent count leaves the stack before the rolls, even
//! when every roll fails. Each of the `count_item` crafts then draws `number(1, 100)`
//! against the percent plus the lent count, and the wins are totalled.
//!
//! The reward is `CreateItem(reward, count * wins)`: a vnum no proto has, no ids, or an
//! unported kind refuses the whole make silently, as legacy's `if (item)` does — after
//! the spends, because legacy spends before it creates (`:478-495`). A made item the
//! inventory (or the dragon soul box, for a dragon soul stone) has no room for chats
//! `You don't have enough space in your inventory` and refuses, with the spends kept.
//! The materials and the gold leave first (`RemoveSpecifyItem` over the unlocked base
//! inventory, `ChangeGold` under `ENABLE_REMOVE_LIMIT_GOLD`), and no win chats `Failed`
//! with nothing granted. A win lands the reward as `AutoGiveItemNew` does: a stackable
//! reward merges onto its stacks first, and anything left goes to the first free cell of
//! its box or the unlocked base inventory.

use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_nested::{GcCubeRenewal, GcCubeRenewalDate, GC_CUBE_CATEGORY_SIZE};
use protocol::item_pos::ItemPos;
use world::character::{
    auto_give_item_new, number, remove_specify_item, GivenNew, ItemChange, ItemRecord, MoveDone,
    MoveKind, MoveRecord,
};
use world::item::{ITEM_ANTIFLAG_STACK, ITEM_FLAG_STACKABLE};

use super::GameState;
use crate::item_move::{gold_delta, MoveItemRefused, MovedItems, Mover};

/// `CUBE_RENEWAL_SUB_HEADER_OPEN_RECEIVE` (`G/packet.h:2888`).
pub const CUBE_OPEN_RECEIVE: u8 = 0;
/// `CUBE_RENEWAL_SUB_HEADER_CLEAR_DATES_RECEIVE` (`G/packet.h:2889`).
pub const CUBE_CLEAR_DATES_RECEIVE: u8 = 1;
/// `CUBE_RENEWAL_SUB_HEADER_DATES_RECEIVE` (`G/packet.h:2890`).
pub const CUBE_DATES_RECEIVE: u8 = 2;
/// `CUBE_RENEWAL_SUB_HEADER_DATES_LOADING` (`G/packet.h:2891`).
pub const CUBE_DATES_LOADING: u8 = 3;
/// `CUBE_RENEWAL_SUB_HEADER_MAKE_ITEM` (`G/input_main.cpp:3470`).
pub const CUBE_MAKE_ITEM: u8 = 4;
/// `CUBE_RENEWAL_SUB_HEADER_CLOSE` (`G/input_main.cpp:3481`).
pub const CUBE_CLOSE: u8 = 5;

/// `CUBE_MAX_DISTANCE` (`G/cuberenewal.h:7`): an NPC at or past it is out of range.
pub const CUBE_MAX_DISTANCE: i32 = 1000;

/// The largest stack an improve lend reads (`G/cuberenewal.cpp:424`).
pub const CUBE_IMPROVE_STACK_LIMIT: u16 = 40;

/// The improve cell that lends nothing (`index_item_improve != -1`, `:417`).
pub const CUBE_NO_IMPROVE: u32 = u32::MAX;

impl GameState {
    /// `/cube open`: stand the window on the clicked NPC, answering the four sends.
    /// Anything but `open` is silent, as legacy's `default: return` is.
    pub(super) fn cube_command(&mut self, vid: Vid, rest: &[u8]) {
        let (first, _) = crate::command::one_argument(rest);
        if !first.eq_ignore_ascii_case(b"open") {
            return;
        }
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let npc = self
            .quests
            .as_ref()
            .map_or(0, |quests| quests.manager().npc_vid(pid));
        if npc == 0 {
            return;
        }
        self.open_cube(vid, npc);
    }

    /// One `CG_CUBE_RENEWAL`: the make or the close. An unknown sub-header is absorbed,
    /// as legacy's `switch` is. A quest waiting on the character holds the request, as
    /// the sash's does.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when no character is online under `vid`.
    pub fn cube_renewal(
        &mut self,
        vid: Vid,
        sub_header: u8,
        (index, count, improve): (u32, u32, u32),
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        let mut moved = MovedItems::new(
            pid,
            vid.raw(),
            MoveDone {
                kind: MoveKind::Refined,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        );
        if self.quest_running(vid) {
            return Ok(moved);
        }
        match sub_header {
            CUBE_MAKE_ITEM => self.make_cube(vid, index, count, improve, actor, &mut moved),
            CUBE_CLOSE => {
                self.cubes.remove(&vid);
            }
            _ => {}
        }
        Ok(moved)
    }

    /// `Cube_open` on the NPC under `npc`: the gates, then the four sends.
    fn open_cube(&mut self, vid: Vid, npc: u32) {
        let (Some(mover), Some(place)) = (self.own_mover(vid), self.kept_place(vid)) else {
            return;
        };
        let (channel, map) = self.place_of(vid.raw()).unwrap_or((1, 1));
        let kept = self.npcs.get(&(channel, map)).and_then(|map| {
            map.npcs
                .iter()
                .find(|npc_stands| npc_stands.vid == npc)
                .map(|npc_stands| (npc_stands.x, npc_stands.y, npc_stands.vnum))
        });
        let Some((x, y, vnum)) = kept else {
            return;
        };
        if !self.cube_recipes.has_npc(vnum) {
            return;
        }
        if self.trading.contains_key(&vid)
            || self.browsing.contains_key(&vid)
            || self.storages.contains_key(&vid)
            || self.cubes.contains_key(&vid)
            || self.aura_windows.contains_key(&vid)
        {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Cannot open crafting window",
                &[],
            );
            return;
        }
        if crate::sync_position::distance_approx(x - place.1, y - place.2) >= CUBE_MAX_DISTANCE {
            return;
        }
        self.cubes.insert(vid, npc);
        for sub in [
            CUBE_CLEAR_DATES_RECEIVE,
            CUBE_DATES_RECEIVE,
            CUBE_DATES_LOADING,
            CUBE_OPEN_RECEIVE,
        ] {
            self.tell_cube_dates(vid, sub, vnum);
        }
    }

    /// One `SendDateCubeRenewalPackets` send (`G/cuberenewal.cpp:519-585`): the bare
    /// record for every sub-header but `DATES_RECEIVE`, which sends one date per recipe
    /// of the NPC's list.
    fn tell_cube_dates(&self, vid: Vid, sub_header: u8, npc: u32) {
        if sub_header != CUBE_DATES_RECEIVE {
            let _sent = self.write_to_client(vid, self.cube_send(sub_header, None));
            return;
        }
        let recipes = self.cube_recipes.clone();
        for recipe in recipes.by_npc.get(&npc).cloned().unwrap_or_default() {
            let _sent = self.write_to_client(vid, self.cube_send(sub_header, Some(&recipe)));
        }
    }

    /// One `TPacketGCCubeRenewalReceive`: the sub-header and the date, zeroed without a
    /// recipe. `npc_vnum` and `index` are never written (a Defect kept); the category is
    /// truncated to its 100 bytes (a Divergence from legacy's overflowing `memcpy`).
    fn cube_send(&self, sub_header: u8, recipe: Option<&gamedata::cube::CubeRecipe>) -> Vec<u8> {
        let mut date = GcCubeRenewalDate {
            npc_vnum: 0,
            index: 0,
            vnum_reward: 0,
            count_reward: 0,
            item_reward_stackable: 0,
            vnum_material: [0; 5],
            count_material: [0; 5],
            gold: 0,
            percent: 0,
            category: [0; GC_CUBE_CATEGORY_SIZE],
        };
        if let Some(recipe) = recipe {
            let stackable = self.protos.get(recipe.reward.vnum).is_some_and(|proto| {
                proto.flags & ITEM_FLAG_STACKABLE != 0
                    || proto.anti_flags & ITEM_ANTIFLAG_STACK == 0
            });
            date.vnum_reward = recipe.reward.vnum;
            date.count_reward = i32::try_from(recipe.reward.count).unwrap_or(i32::MAX);
            date.item_reward_stackable = u8::from(stackable);
            for (slot, material) in recipe.material.iter().take(5).enumerate() {
                date.vnum_material[slot] = material.vnum;
                date.count_material[slot] = i32::try_from(material.count).unwrap_or(i32::MAX);
            }
            date.gold = recipe.gold;
            date.percent = recipe.percent;
            let take = recipe.category.len().min(GC_CUBE_CATEGORY_SIZE);
            date.category[..take].copy_from_slice(&recipe.category[..take]);
        }
        let mut record = Vec::new();
        GcCubeRenewal {
            header: GcCubeRenewal::header(),
            subheader: sub_header,
            date,
        }
        .encode_into(&mut record);
        record
    }

    /// `Cube_Make`: the gates in legacy's order, then the rolls, the spends and the grant.
    fn make_cube(
        &mut self,
        vid: Vid,
        index: u32,
        count: u32,
        improve: u32,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        let Some(npc) = self.cubes.get(&vid).copied() else {
            moved.records.push(self.cube_line(
                actor,
                b"To create an item you have to have the crafting window open",
            ));
            return;
        };
        let (channel, map) = self.place_of(vid.raw()).unwrap_or((1, 1));
        let vnum = self
            .npcs
            .get(&(channel, map))
            .and_then(|map| map.npcs.iter().find(|npc_stands| npc_stands.vid == npc))
            .map(|npc_stands| npc_stands.vnum);
        let Some(vnum) = vnum else {
            return;
        };
        let Some(recipe) = self
            .cube_recipes
            .by_npc
            .get(&vnum)
            .and_then(|list| list.get(index as usize))
            .cloned()
        else {
            return;
        };
        for material in &recipe.material {
            let need = u64::from(material.count) * u64::from(count);
            if u64::try_from(self.count_specify_item(vid, material.vnum)).unwrap_or(0) < need {
                moved
                    .records
                    .push(self.cube_line(actor, b"You don't have the necessary materials"));
                return;
            }
        }
        if recipe.gold != 0 {
            let gold = self
                .characters
                .find_by_vid(vid)
                .map_or(0, world::character::Character::gold);
            if u128::from(gold) < u128::from(recipe.gold) * u128::from(count) {
                moved
                    .records
                    .push(self.cube_line(actor, b"You don't have the necessary amount of gold"));
                return;
            }
        }
        let mut done = self.lend_cube_improve(vid, improve, recipe.percent);
        if count == 0 {
            self.pack_cube_done(vid, actor, moved, done);
            return;
        }
        let mut wins = 0_u32;
        for _ in 0..count {
            if number(&mut self.dice, 1, 100) <= recipe.percent + done_lent(&done) {
                wins += 1;
            }
        }
        self.spend_cube_make(vid, &recipe, count, &mut done);
        if wins == 0 {
            self.pack_cube_done(vid, actor, moved, done);
            moved.records.push(self.cube_line(actor, b"Failed"));
            return;
        }
        let granted = self.grant_cube_reward(vid, &recipe, wins, &mut done);
        // The battle-pass craft hook (`G/cuberenewal.cpp:458-459`): the made
        // count counts on the made item.
        if granted {
            let total = u64::from(recipe.reward.count) * u64::from(wins);
            let made = u16::try_from(total.min(u64::from(u16::MAX))).unwrap_or(u16::MAX);
            self.pass_on_craft(vid, actor, made, recipe.reward.vnum);
        }
        self.pack_cube_done(vid, actor, moved, done);
        if !granted {
            moved
                .records
                .push(self.cube_line(actor, b"You don't have enough space in your inventory"));
        }
    }

    /// The improve lend: the named base-inventory cell's spend, with the lent count in the
    /// returned `MoveDone`'s records' place. Split out so `make_cube` stays readable.
    fn lend_cube_improve(&mut self, vid: Vid, improve: u32, percent: i32) -> CubeLent {
        let mut lent = CubeLent::default();
        if improve == CUBE_NO_IMPROVE {
            return lent;
        }
        let cell = u16::try_from(improve).unwrap_or(u16::MAX);
        let pos = ItemPos::new(common::item_slots::EWindows::Inventory as u8, cell);
        let held = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(pos))
            .map(|item| (item.id, item.count));
        let Some((id, held)) = held else {
            return lent;
        };
        if held == 0 || held > CUBE_IMPROVE_STACK_LIMIT {
            return lent;
        }
        let count = i32::from(held);
        if percent + count <= 100 {
            lent.lent = count;
        }
        if percent < 100 && percent + count > 100 {
            lent.lent = 100 - percent;
        }
        if lent.lent <= 0 {
            lent.lent = 0;
            return lent;
        }
        let keeps = u32::from(held) - u32::try_from(lent.lent).unwrap_or(u32::from(held));
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            lent.lent = 0;
            return lent;
        };
        if keeps == 0 {
            // The whole stack went: the cell clears, as `SetCount(0)` refuses.
            let pos = ItemPos::new(common::item_slots::EWindows::Inventory as u8, cell);
            if character.items_mut().release(id).is_ok() {
                lent.done.records.push(MoveRecord::Item(ItemRecord::Set(
                    world::item::gc_item_clear(pos),
                )));
            }
            lent.done.changes.push(ItemChange::Destroyed { id });
            return lent;
        }
        let Ok(set) = character.items_mut().set_count(id, keeps) else {
            lent.lent = 0;
            return lent;
        };
        if let Some(stack) = character.items().item(id) {
            lent.done
                .records
                .push(MoveRecord::Item(ItemRecord::Update(stack.gc_item_update())));
        }
        lent.done.changes.push(ItemChange::Count { id, count: set });
        lent
    }

    /// The material and gold spends of a rolled make.
    fn spend_cube_make(
        &mut self,
        vid: Vid,
        recipe: &gamedata::cube::CubeRecipe,
        count: u32,
        lent: &mut CubeLent,
    ) {
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let usable = common::item_slots::usable_inventory_cells(character.inven_point());
        for material in &recipe.material {
            let take = u64::from(material.count) * u64::from(count);
            let (items, _, slots) = character.items_points_and_quickslots_mut();
            let spent = remove_specify_item(
                items,
                slots,
                &self.protos,
                usable,
                (material.vnum, u32::try_from(take).unwrap_or(u32::MAX)),
            );
            lent.done.records.extend(spent.records);
            lent.done.changes.extend(spent.changes);
        }
        if recipe.gold != 0 {
            let spend = u128::from(recipe.gold) * u128::from(count);
            let spend = u64::try_from(spend.min(u128::from(u64::MAX))).unwrap_or(u64::MAX);
            let gold = character.gold().saturating_sub(spend);
            lent.gold = Some(gold_delta(character.gold(), gold));
            character.set_gold(gold);
        }
    }

    /// The reward half of a won make: the made item lands as `AutoGiveItemNew` does.
    /// Answers whether it landed.
    fn grant_cube_reward(
        &mut self,
        vid: Vid,
        recipe: &gamedata::cube::CubeRecipe,
        wins: u32,
        lent: &mut CubeLent,
    ) -> bool {
        let total = u64::from(recipe.reward.count) * u64::from(wins);
        let count = i32::try_from(total.min(u64::try_from(i32::MAX).unwrap_or(u64::MAX)))
            .unwrap_or(i32::MAX);
        let Some(item) = self.make_drop_hit(recipe.reward.vnum, count, 0) else {
            return true;
        };
        let qualified = self.ds_qualified(vid);
        let count_limit = self.item_count_limit;
        let protos = std::sync::Arc::clone(&self.protos);
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return true;
        };
        let rules = world::character::MoveRules {
            count_limit,
            usable_cells: common::item_slots::usable_inventory_cells(character.inven_point()),
            belt_grade: None,
            questing: false,
            secured: false,
        };
        let (given, done) =
            auto_give_item_new(character.items_mut(), &rules, &protos, item, qualified);
        if matches!(given, GivenNew::Unplaced(_)) {
            return false;
        }
        lent.done.records.extend(done.records);
        lent.done.changes.extend(done.changes);
        true
    }

    /// Pack a make's `MoveDone` into the step's records and rows.
    fn pack_cube_done(&self, vid: Vid, actor: Mover, moved: &mut MovedItems, lent: CubeLent) {
        let done = MovedItems::new(
            moved.owner_id,
            vid.raw(),
            lent.done,
            actor,
            &self.protos,
            &self.locale,
        );
        moved.records.extend(done.records);
        moved.changes.extend(done.changes);
        if let Some(gold) = lent.gold {
            moved.gold = Some(gold);
        }
    }

    /// One `ChatPacket(CHAT_TYPE_INFO, ...)` line, encoded for the mover.
    fn cube_line(&self, actor: Mover, text: &[u8]) -> Vec<u8> {
        crate::chat_line::chat_packet(actor.recipient(&self.locale), CHAT_TYPE_INFO, text, &[])
    }
}

/// What the improve lend and the spends gathered: the lent count, the move, and the gold
/// the spends left.
#[derive(Debug)]
struct CubeLent {
    /// The lent count.
    lent: i32,
    /// The gathered move.
    done: MoveDone,
    /// The gold change, when the recipe costs gold.
    gold: Option<i64>,
}

impl Default for CubeLent {
    fn default() -> Self {
        Self {
            lent: 0,
            done: MoveDone {
                kind: MoveKind::Refined,
                records: Vec::new(),
                changes: Vec::new(),
            },
            gold: None,
        }
    }
}

/// The lent count a `CubeLent` carries.
fn done_lent(lent: &CubeLent) -> i32 {
    lent.lent
}
