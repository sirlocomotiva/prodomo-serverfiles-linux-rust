//! The direct pick-up (`RENEWAL_PICKUP_AFFECT`, `prodomodefines.h:151`): item 70002 grants
//! `AFFECT_PICKUP_ENABLE` for seven hours, `/pickup_affect` switches it off and on, and while
//! it is on a kill's drop owned by the character goes straight into its inventory.
//!
//! - 70002 (`G/char_item.cpp:2852-2872`): with the pick-up on, `Deja ai auto-pick-up-ul.`;
//!   with it off, ` Pick-up Already Active.`; else the 25200-second realtime affect and
//!   `Auto Pick-up-ul a fost activat cu succes!`. The item is not used up.
//! - `/pickup_affect` (`do_pickup_affect`, `G/cmd_general.cpp:279-300`): the on row, else the
//!   off row, else nothing; that row goes, and with seconds left the other row takes them,
//!   else `Auto Pick-up-ul a fost dezactivat.`.
//! - The kill (`G/char_battle.cpp:874`, `:969`): a drop whose owner holds the on row is given
//!   by `AutoGiveItemNew` (`G/char_item.cpp:10725-10829`, [`world::character::auto_give_item_new`])
//!   instead of laid: `Ai primit: x%d%s.` for more than one, `Ai primit: %s.` for one, and what
//!   no cell takes lies at the owner's feet for 300 seconds. The owner's descriptor stores the
//!   rows.
//!
//! The lines carry legacy's `@@(file)tradus:` keys, which the client translates. One Defect is
//! not reproduced: when the stacks take part of a drop and no cell takes the rest, legacy's
//! line names the whole count; here it names what the stacks took.

use common::item_slots::usable_inventory_cells;
use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;
use world::character::{auto_give_item_new, GivenNew, MoveRules};
use world::item::Item;

use super::affect::{Affect, AFFECT_PICKUP_DEACTIVE};
use super::GameState;
use crate::chat_line::Arg;
use crate::item_move::{MovedItems, Mover, StoreOrder};
use crate::loading_phase::AFFECT_PICKUP_ENABLE;

/// The seconds 70002 grants: `60 * 60 * 7`.
pub(super) const PICKUP_SECONDS: i32 = 60 * 60 * 7;

/// The seconds a direct give's unplaced drop stays owned: `SetOwnership(this, 300)`.
const UNPLACED_OWNERSHIP_SECS: u16 = 300;

impl GameState {
    /// The first affect of `kind` the player under `vid` holds (`FindAffect`).
    fn find_pc_affect(&self, vid: Vid, kind: u32) -> Option<Affect> {
        self.affect_events
            .get(&vid)?
            .affects
            .iter()
            .find(|affect| affect.kind == kind)
            .copied()
    }

    /// `CanPickupDirectly` (`G/char_item.cpp:10831-10834`): the on row is held.
    pub(super) fn can_pickup_directly(&self, vid: Vid) -> bool {
        self.find_pc_affect(vid, AFFECT_PICKUP_ENABLE).is_some()
    }

    /// 70002's arm of `UseItemEx`, run in place of the item's quest.
    pub(super) fn use_pickup_item(&mut self, vid: Vid, mover: Mover) {
        if self.can_pickup_directly(vid) {
            let line = b"@@(char_item.cpp)tradus:Deja ai auto-pick-up-ul.";
            self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
            return;
        }
        if self.find_pc_affect(vid, AFFECT_PICKUP_DEACTIVE).is_some() {
            let line = b"@@(char_item.cpp)tradus: Pick-up Already Active.";
            self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
            return;
        }
        let affect = Affect::buff(AFFECT_PICKUP_ENABLE, 0, 0, 0, PICKUP_SECONDS, 0);
        self.add_pc_affect(vid, affect, false);
        let line = b"@@(char_item.cpp)tradus:Auto Pick-up-ul a fost activat cu succes!";
        self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
    }

    /// `/pickup_affect`: the held row goes, and its seconds left move to the other row.
    pub(super) fn run_pickup_affect(&mut self, vid: Vid, mover: Mover) {
        let Some(held) = self
            .find_pc_affect(vid, AFFECT_PICKUP_ENABLE)
            .or_else(|| self.find_pc_affect(vid, AFFECT_PICKUP_DEACTIVE))
        else {
            return;
        };
        let left = held.remaining.wrapping_sub(super::affect::wall_now());
        let _removed = self.remove_pc_affect(vid, held.kind);
        if left <= 0 {
            let line = b"@@(cmd_general)tradus:Auto Pick-up-ul a fost dezactivat.";
            self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
            return;
        }
        let other = if held.kind == AFFECT_PICKUP_ENABLE {
            AFFECT_PICKUP_DEACTIVE
        } else {
            AFFECT_PICKUP_ENABLE
        };
        self.add_pc_affect(vid, Affect::buff(other, 0, 0, 0, left, 0), false);
    }

    /// `AutoGiveItemNew(item)` for the drop owner `owner`: the stacks, a free cell, or its
    /// feet, with the line and the rows ordered to its descriptor.
    pub(super) fn give_drop_directly(&mut self, owner: Vid, item: Item) {
        let Some(mover) = self.own_mover(owner) else {
            return;
        };
        let Some((channel, map)) = self.place_of(owner.raw()) else {
            return;
        };
        let qualified = self.ds_qualified(owner);
        let protos = std::sync::Arc::clone(&self.protos);
        let name = protos
            .get(item.vnum)
            .map(|proto| proto.locale_name.clone())
            .unwrap_or_default();
        let real = item.count;
        let count_limit = self.item_count_limit;
        let Ok(character) = self.characters.find_by_vid_mut(owner) else {
            return;
        };
        let pid = character.player_id();
        let rules = MoveRules {
            count_limit,
            usable_cells: usable_inventory_cells(character.inven_point()),
            belt_grade: None,
            questing: false,
        };
        let (given, done) =
            auto_give_item_new(character.items_mut(), &rules, &protos, item, qualified);
        let packed = MovedItems::new(pid, owner.raw(), done, mover, &protos, &self.locale);
        for record in packed.records {
            let _sent = self.write_to_client(owner, record);
        }
        let told = match &given {
            GivenNew::Merged | GivenNew::Placed(_) => real,
            GivenNew::Unplaced(left) => real - left.count,
        };
        let placed = !matches!(given, GivenNew::Unplaced(_));
        if told > 1 {
            let line = b"Ai primit: x%d%s.";
            let args = [Arg::Int(i64::from(told)), Arg::Text(&name)];
            self.tell(owner, mover, CHAT_TYPE_INFO, line, &args);
        } else if placed {
            self.tell(
                owner,
                mover,
                CHAT_TYPE_INFO,
                b"Ai primit: %s.",
                &[Arg::Text(&name)],
            );
        }
        if let GivenNew::Unplaced(left) = given {
            self.lay_owned(owner, left, UNPLACED_OWNERSHIP_SECS);
        }
        if !packed.changes.is_empty() {
            let order = StoreOrder::default();
            let _ordered = self.order_the_item_rows(owner, channel, map, packed.changes, order);
        }
    }
}
