//! Unlocking inventory rows (`ENABLE_EXTEND_INVEN_SYSTEM`,
//! `G/input_main.cpp:1069-1075` → `CHARACTER::Update_Inven`, `G/char.cpp:11763-11788`).
//!
//! The 1-byte `ENVANTER_BLACK` record (0xe2) is a pure unlock knock: no payload, refused to
//! observers (`G/input_main.cpp:3711-3714`). The unlock needs the price in keys, counted as
//! the two key vnums together (`INVENTORY_OPEN_KEY_VNUM` 72319 and
//! `INVENTORY_OPEN_KEY_VNUM2` 72320), and spends them one at a time starting from the first
//! (`INVENTORY_START_DELETE_VNUM`). A shortfall chats `ExInvenItemUseMsg` with the missing
//! count (`CHAT_TYPE_COMMAND`); a full row set stays silent.
//!
//! The price list is triples of 2 up: `needkeys[Inven_Point()]` is
//! `INVENTORY_NEED_KEY_START + Inven_Point() / 3`. The maximum, 18
//! ([`INVENTORY_LOCK_COVER_COUNT`]), names a price, but the full-row guard fires first, so
//! it is never paid.
//!
//! `G/` = `server/server/game/`, `C/` = `server/server/common/`.

use common::item_slots::{
    usable_inventory_cells, INVENTORY_LOCK_COVER_COUNT, INVENTORY_NEED_KEY_INCREASE,
    INVENTORY_NEED_KEY_START, INVENTORY_OPEN_KEY_VNUM, INVENTORY_OPEN_KEY_VNUM2,
    INVENTORY_START_DELETE_VNUM,
};
use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_COMMAND;

use super::GameState;
use crate::chat_line::Arg;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

/// The price of the unlock at `inven_point`: 2, then one more every three rows
/// (`char.cpp:11766-11770`).
#[must_use]
pub fn unlock_price(inven_point: u16) -> u32 {
    INVENTORY_NEED_KEY_START + u32::from(inven_point) / u32::from(INVENTORY_NEED_KEY_INCREASE)
}

impl GameState {
    /// One `ENVANTER_BLACK` for the character under `vid`: the next inventory row unlocked
    /// for its price in keys, or the shortfall chatted. Answers the rows the spend changed,
    /// as an item step does, so the descriptor stores the spent keys with the unlock.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn extend_inven(&mut self, vid: Vid, actor: Mover) -> Result<MovedItems, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        let moved = MovedItems::new(
            pid,
            vid.raw(),
            world::character::MoveDone {
                kind: world::character::MoveKind::Removed,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        );
        Ok(self.update_inven(vid, actor, moved))
    }

    /// `CHARACTER::Update_Inven` (`G/char.cpp:11763-11788`).
    fn update_inven(&mut self, vid: Vid, actor: Mover, mut moved: MovedItems) -> MovedItems {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return moved;
        };
        let inven = character.inven_point();
        if i32::from(inven) >= INVENTORY_LOCK_COVER_COUNT {
            return moved;
        }
        let price = unlock_price(inven);
        let keys = self.count_specify_item(vid, INVENTORY_OPEN_KEY_VNUM)
            + self.count_specify_item(vid, INVENTORY_OPEN_KEY_VNUM2);
        if i64::from(keys) < i64::from(price) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_COMMAND,
                b"ExInvenItemUseMsg %d",
                &[Arg::Int(i64::from(price) - i64::from(keys))],
            );
            return moved;
        }
        let mut rest = price;
        let mut spends = INVENTORY_START_DELETE_VNUM;
        while rest > 0 {
            if self.count_specify_item(vid, spends) <= 0 {
                spends = if INVENTORY_START_DELETE_VNUM == INVENTORY_OPEN_KEY_VNUM {
                    INVENTORY_OPEN_KEY_VNUM2
                } else {
                    INVENTORY_OPEN_KEY_VNUM
                };
            }
            self.spend_inven_key(vid, actor, spends, &mut moved);
            rest -= 1;
        }
        self.change_pc_point(vid, common::point_slot::POINT_INVEN, 1, false);
        self.sync_inven_stat(vid, &mut moved);
        moved
    }

    /// One `RemoveSpecifyItem(vnum, 1)` for the player under `vid`, its records and changes
    /// folded into `moved`.
    fn spend_inven_key(&mut self, vid: Vid, actor: Mover, vnum: u32, moved: &mut MovedItems) {
        let protos = self.protos.clone();
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let usable = usable_inventory_cells(character.inven_point());
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let done = world::character::remove_specify_item(items, slots, &protos, usable, (vnum, 1));
        let packed = MovedItems::new(
            moved.owner_id,
            vid.raw(),
            world::character::MoveDone {
                kind: world::character::MoveKind::Removed,
                records: done.records,
                changes: done.changes,
            },
            actor,
            &protos,
            &self.locale,
        );
        moved.records.extend(packed.records);
        moved.changes.extend(packed.changes);
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        moved.points = character.points().cloned();
        moved.quickslots = Some(character.quickslots().clone());
    }

    /// The model's `Inven_Point` follows the points' after the rise, and the stored
    /// points follow both: the scans, the world grants and the save read three numbers,
    /// and all three are the unlock now.
    fn sync_inven_stat(&mut self, vid: Vid, moved: &mut MovedItems) {
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let rise = {
            let Some(points) = character.items_points_and_quickslots_mut().1 else {
                return;
            };
            u16::try_from(points.inven_point()).unwrap_or(u16::MAX)
        };
        character.set_inven_point(rise);
        moved.points = character.points().cloned();
        moved.quickslots = Some(character.quickslots().clone());
    }
}
