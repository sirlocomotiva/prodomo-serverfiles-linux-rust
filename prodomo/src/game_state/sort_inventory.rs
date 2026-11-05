//! `/sort_inventory` (`do_sort_inventory`, `G/cmd.cpp:41-44`, `GM_PLAYER`):
//! `CHARACTER::SortInventoryItems` (`G/char.cpp:10790-10866`).
//!
//! [`GameState::run_sort_inventory`] runs the gates in legacy's order, then
//! [`world::character::sort_inventory`], whose module doc describes the sort itself:
//!
//! - a dead character, or one with a trade, a shop or a safebox open, hears `Please close
//!   other windows before sorting your inventory`. The private shop (`GetMyShop`) and the cube
//!   (`IsCubeOpen`) are not ported, so neither refuses here;
//! - within 15 seconds of its last sort, the character hears `Please wait %d seconds to sort
//!   your inventory again` with the seconds left (`get_global_time()`);
//! - a character with a quest running is refused silently;
//! - a running exit countdown (`m_pkTimedEvent`) is cancelled with `Canceled.`.
//!
//! After the sort, what no cell took lies at the character's feet, the 15 seconds start, and
//! the character hears `Your inventory items have been sorted`. The descriptor stores the
//! moved, merged and destroyed rows the answer carries.

use common::item_slots::usable_inventory_cells;
use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;
use world::character::{sort_inventory, MoveRules};

use super::gm_lines::GmRows;
use super::GameState;
use crate::chat_line::Arg;
use crate::item_move::{MovedItems, Mover};

/// The seconds between two sorts: `SetSortInventoryPulse(get_global_time() + 15)`.
pub(super) const SORT_INVENTORY_SECS: u32 = 15;

impl GameState {
    /// `/sort_inventory` for `vid`. Answers the rows the sort changed, or `None` when a gate
    /// refused it.
    pub(super) fn run_sort_inventory(&mut self, vid: Vid, mover: Mover) -> Option<GmRows> {
        let busy = self.is_dead(vid.raw())
            || self.trading.contains_key(&vid)
            || self.browsing.contains_key(&vid)
            || self.safebox_open(vid);
        if busy {
            let line = b"Please close other windows before sorting your inventory";
            self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
            return None;
        }
        let pid = self.characters.find_by_vid(vid).ok()?.player_id();
        let now = Self::global_time();
        if let Some(&(owner, until)) = self.sort_inventory_until.get(&vid) {
            if owner == pid && until > now {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"Please wait %d seconds to sort your inventory again",
                    &[Arg::Int(i64::from(until - now))],
                );
                return None;
            }
        }
        if self.quest_running(vid) {
            return None;
        }
        if self.timed_events.remove(&vid).is_some() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Canceled.", &[]);
        }
        let rules = MoveRules {
            count_limit: self.item_count_limit,
            usable_cells: 0,
            belt_grade: None,
            questing: false,
        };
        let protos = std::sync::Arc::clone(&self.protos);
        let character = self.characters.find_by_vid_mut(vid).ok()?;
        let rules = MoveRules {
            usable_cells: usable_inventory_cells(character.inven_point()),
            ..rules
        };
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let sorted = sort_inventory(items, slots, &rules, &protos);
        let packed = MovedItems::new(
            pid,
            vid.raw(),
            sorted.done,
            mover,
            &self.protos,
            &self.locale,
        );
        for record in packed.records {
            let _sent = self.write_to_client(vid, record);
        }
        for (item, secs) in sorted.unplaced {
            self.lay_owned(vid, item, secs);
        }
        let until = now.saturating_add(SORT_INVENTORY_SECS);
        self.sort_inventory_until.insert(vid, (pid, until));
        let line = b"Your inventory items have been sorted";
        self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
        Some(GmRows {
            pid,
            changes: packed.changes,
            cleared_skills: Vec::new(),
        })
    }

    /// `GetSortInventoryPulse()` of the character under `vid`: the second its last sort
    /// holds warps until, or 0 when it has not sorted. A `ClientOrder::Warp` carries it.
    pub(super) fn sort_lock(&self, vid: Vid) -> u32 {
        self.sort_inventory_until
            .get(&vid)
            .map_or(0, |&(_, until)| until)
    }
}
