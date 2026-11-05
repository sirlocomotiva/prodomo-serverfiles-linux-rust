//! The target's drop list: `CInputMain::TargetInfoLoad` (`G/input_main.cpp:66-123`, behind
//! `__SEND_TARGET_INFO__`) and `ITEM_MANAGER::CreateDropItemVector`
//! (`G/item_manager.cpp:915-1095`).
//!
//! # What legacy does
//!
//! The target window asks with `CG_TARGET_INFO_LOAD` and a VID. `CreateDropItemVector` lists
//! what the target may drop for the asking character, building each entry with
//! `CreateItem(vnum, count, 0, true, pct)`:
//!
//! 1. a player or a polymorphed character lists nothing;
//! 2. the common-drop loop walks the rank's rows inside the asker's level band but never adds
//!    one, because the item it tests is still `NULL` there (`:944`), so no common drop is
//!    listed;
//! 3. every row of the race's drop group (`drop_item_group.txt` and the `drop` groups);
//! 4. one row of the race's `kill` group, drawn by `GetOne` (`number(1, total)` against the
//!    running part percents), with no kill-per-drop gate;
//! 5. every row of the race's `limit` group when its level limit, a `DWORD`, is at most the
//!    asker's level;
//! 6. every row of the race's `thiefgloves` group, with no glove check;
//! 7. the proto's `DROP_ITEM` when the etc table names it, with a count of 1.
//!
//! `ENABLE_STONE_DROP_ON_METIN` is not defined, so the stone arm is dead (C15). Only a monster
//! or a stone is answered, and the list is checked first, so the `GetOne` draw runs for any
//! target. One entry is sent as it is, a stone (`ITEM_METIN`) included; several are sorted by
//! `CustomSort` (weapon, armour, use, belt, costume, books, stone, material, the rest), then
//! by sub-type, and each is sent as a `GC_TARGET_INFO` with the target's VID and race, the
//! shown vnum (the mask's, when there is one), the count and the rarity, skipping the stones.
//!
//! The count is `CreateItem`'s: gold (`ITEM_ELK`) keeps the row's count up to `INT_MAX`, a
//! stackable item is held between 1 and the stack limit (a `MAKECOUNT` item asking for 1 takes
//! its `VALUE1`), and anything else is 1; the record keeps its low 16 bits. The rarity is the
//! row's percent ×10000 (a `kill` row's rare percent); the etc row asks with the default -1, so
//! its rarity is `0xFFFFFFFF`.
//!
//! # Divergences
//!
//! - **No items are made.** Legacy creates a real item for each entry, each with a new ID and,
//!   for a timed item, a running expiry event, and never destroys them (a leak per click, a
//!   Defect not reproduced). The Rewrite computes what each item would show.
//! - **A rarity of 0.** `CreateItem` sets the rarity only for a nonzero percent, and the item's
//!   constructor never initialises it, so a row with percent 0 sends whatever the memory held (a
//!   Defect not reproduced). The Rewrite sends 0.
//! - **Ties in the sort.** `std::sort` leaves entries of the same kind and sub-type in an order
//!   only libstdc++'s introsort decides once there are more than 16. The Rewrite sorts stably,
//!   keeping the table's order.
//! - **Where the target is found.** As for `CG_TARGET`, only on the asker's Channel and map
//!   (see [`super::target`]).

use common::vid::Vid;
use gamedata::drop::{DropTables, MobGroup, MobItem};
use gamedata::item_kind::{
    ITEM_ARMOR, ITEM_BELT, ITEM_COSTUME, ITEM_ELK, ITEM_MATERIAL, ITEM_METIN, ITEM_SKILLBOOK,
    ITEM_SKILLFORGET, ITEM_USE, ITEM_WEAPON,
};
use gamedata::item_proto::ItemProtos;
use gamedata::mob_proto::{CHAR_TYPE_MONSTER, CHAR_TYPE_STONE};
use protocol::gc_actors::GcTargetInfo;
use world::character::{number, ITEM_FLAG_MAKECOUNT};
use world::item::ITEM_FLAG_STACKABLE;

use super::GameState;

/// One listed drop: what the item `CreateItem` would have made shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Listed {
    /// `GetVnum()`: the mask's vnum when the item has one.
    vnum: u32,
    /// `GetCount()`, cut to the record's `WORD`.
    count: u16,
    /// `GetRarity()`.
    rarity: u32,
    /// `CustomSort()`, then `GetSubType()`.
    order: (u8, i32),
    /// `GetType() == ITEM_METIN`.
    stone: bool,
}

/// What the item tables `CreateItem` reads hold.
struct Makers<'a> {
    /// The protos.
    protos: &'a ItemProtos,
    /// The mask table: `new_vnum -> ori_vnum`.
    masks: &'a [(u32, u32)],
    /// `g_bItemCountLimit`.
    count_limit: u16,
}

/// `CItem::CustomSort` (`G/item.h:269-290`).
fn custom_sort(item_type: i32) -> u8 {
    match item_type {
        ITEM_WEAPON => 1,
        ITEM_ARMOR => 2,
        ITEM_USE => 3,
        ITEM_BELT => 4,
        ITEM_COSTUME => 5,
        ITEM_SKILLBOOK | ITEM_SKILLFORGET => 6,
        ITEM_METIN => 7,
        ITEM_MATERIAL => 8,
        _ => 9,
    }
}

/// An `int` read as the `DWORD` legacy passes it as.
fn as_dword(value: i32) -> u32 {
    u32::from_le_bytes(value.to_le_bytes())
}

impl Makers<'_> {
    /// What `CreateItem(vnum, count, 0, true, pct)` shows, or nothing when it makes no item
    /// (vnum 0, or no proto).
    fn listed(&self, vnum: u32, count: i32, pct: i32) -> Option<Listed> {
        if vnum == 0 {
            return None;
        }
        let proto = self.protos.get(vnum)?;
        let shown = self
            .masks
            .iter()
            .find(|&&(new, _)| new == vnum)
            .map_or(vnum, |&(_, ori)| ori);
        let limit = u32::from(self.count_limit);
        let int_max = as_dword(i32::MAX);
        let held = if proto.item_type == ITEM_ELK {
            as_dword(count).min(int_max)
        } else if proto.flags & ITEM_FLAG_STACKABLE != 0 {
            // `MINMAX(1, count, g_bItemCountLimit)` on the `DWORD` count, then `SetCount`'s
            // `MIN` against the limit, which a `MAKECOUNT` value meets.
            let mut held = as_dword(count).clamp(1, limit);
            if held <= 1 && proto.flags & ITEM_FLAG_MAKECOUNT != 0 {
                held = as_dword(proto.values[1]);
            }
            held.min(limit)
        } else {
            1
        };
        let [low, high, ..] = held.to_le_bytes();
        Some(Listed {
            vnum: shown,
            count: u16::from_le_bytes([low, high]),
            rarity: if pct == 0 { 0 } else { as_dword(pct) },
            order: (custom_sort(proto.item_type), proto.sub_type),
            stone: proto.item_type == ITEM_METIN,
        })
    }
}

/// `GetOneIndex` (`G/item_manager.h:273-278`): the first row whose running part percent
/// reaches `pick`.
fn kill_row(group: &MobGroup, pick: i32) -> Option<&MobItem> {
    let mut reached = 0;
    group.items.iter().find(|row| {
        reached += row.part_pct;
        pick <= reached
    })
}

/// The race's drop list for an asker of `level`, in `CreateDropItemVector`'s order, with the
/// `kill` row `pick` chose (see [`kill_row`]).
fn drop_list(
    tables: &DropTables,
    makers: &Makers<'_>,
    (race, drop_vnum): (u32, u32),
    level: u8,
    pick: Option<i32>,
) -> Vec<Listed> {
    let mut list = Vec::new();
    if let Some(group) = tables.groups.groups.get(&race) {
        list.extend(
            group
                .items
                .iter()
                .filter_map(|row| makers.listed(row.vnum, row.count, as_i32(row.pct))),
        );
    }
    if let (Some(group), Some(pick)) = (tables.mob.kill.get(&race), pick) {
        if let Some(row) = kill_row(group, pick) {
            list.extend(makers.listed(row.vnum, row.count, row.rare_pct));
        }
    }
    if let Some(group) = tables.mob.limit.get(&race) {
        if as_dword(group.level_limit) <= u32::from(level) {
            list.extend(
                group
                    .items
                    .iter()
                    .filter_map(|row| makers.listed(row.vnum, row.count, as_i32(row.pct))),
            );
        }
    }
    if let Some(group) = tables.mob.gloves.get(&race) {
        list.extend(
            group
                .items
                .iter()
                .filter_map(|row| makers.listed(row.vnum, row.count, as_i32(row.pct))),
        );
    }
    if drop_vnum != 0 && tables.etc.probs.contains_key(&drop_vnum) {
        list.extend(makers.listed(drop_vnum, 1, -1));
    }
    list
}

/// A percent ×10000 as the `int` the group vectors hand `CreateItem`.
fn as_i32(pct: u32) -> i32 {
    i32::from_le_bytes(pct.to_le_bytes())
}

/// The `GC_TARGET_INFO` records for `list`, of the target `vid` of race `race`: one entry as
/// it is, several sorted with the stones skipped.
fn info_records(vid: u32, race: u32, mut list: Vec<Listed>) -> Vec<Vec<u8>> {
    if list.len() > 1 {
        list.sort_by_key(|listed| listed.order);
        list.retain(|listed| !listed.stone);
    }
    list.iter()
        .map(|listed| {
            let mut record = Vec::new();
            GcTargetInfo::new(vid, race, listed.vnum, listed.count, listed.rarity)
                .encode_into(&mut record);
            record
        })
        .collect()
}

impl GameState {
    /// `CInputMain::TargetInfoLoad` for the player under `vid`, which asked about `wanted`:
    /// the drop list of a monster or stone standing where the player stands, one
    /// `GC_TARGET_INFO` per entry.
    pub(super) fn target_info(&mut self, vid: Vid, wanted: u32) {
        let Some(target) = self.targeted(vid, wanted) else {
            return;
        };
        // A player lists nothing (`IsPC`); no NPC is polymorphed.
        let Some(npc) = self.npc_by_vid(target) else {
            return;
        };
        let (race, drop_vnum, char_type) =
            (u32::from(npc.race), npc.fight.drop_item_vnum, npc.char_type);
        let Some(level) = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(world::character::Character::points)
            .map(world::character::Points::level)
        else {
            return;
        };
        // `GetOne` draws whenever the race has a nonempty `kill` group.
        let total = self
            .drops
            .mob
            .kill
            .get(&race)
            .map_or(0, |group| group.items.iter().map(|row| row.part_pct).sum());
        let pick = (total > 0).then(|| number(&mut self.dice, 1, total));
        let makers = Makers {
            protos: &self.protos,
            masks: self.item_masks.pairs(),
            count_limit: self.item_count_limit,
        };
        let list = drop_list(&self.drops, &makers, (race, drop_vnum), level, pick);
        if char_type != CHAR_TYPE_MONSTER && char_type != CHAR_TYPE_STONE {
            return;
        }
        for record in info_records(target, race, list) {
            let _sent = self.write_to_client(vid, record);
        }
    }
}

#[cfg(test)]
mod tests {
    use gamedata::drop::{DropGroup, DropItem, LevelGroup};
    use gamedata::item_proto::ItemProto;

    use super::*;

    fn proto(vnum: u32, item_type: i32, sub_type: i32) -> ItemProto {
        ItemProto::for_category_rule(vnum, item_type, sub_type)
    }

    fn stackable(mut proto: ItemProto) -> ItemProto {
        proto.flags |= ITEM_FLAG_STACKABLE;
        proto
    }

    fn row(vnum: u32, pct: u32, count: i32) -> DropItem {
        DropItem { vnum, pct, count }
    }

    /// The `vnum`, count and rarity of each record.
    fn shown(records: &[Vec<u8>]) -> Vec<(u32, u16, u32)> {
        records
            .iter()
            .map(|record| {
                let info = GcTargetInfo::decode(record).expect("a target info record");
                (info.dw_vnum, info.count, info.rarity)
            })
            .collect()
    }

    #[test]
    fn a_count_is_what_create_item_would_hold() {
        let mut makecount = stackable(proto(3, ITEM_USE, 0));
        makecount.flags |= ITEM_FLAG_MAKECOUNT;
        makecount.values[1] = 0x0001_2345;
        let protos = ItemProtos::from_rows(vec![
            proto(1, ITEM_ELK, 0),
            stackable(proto(2, ITEM_USE, 0)),
            makecount,
            proto(4, ITEM_WEAPON, 0),
        ]);
        let makers = Makers {
            protos: &protos,
            masks: &[(4, 40)],
            count_limit: 200,
        };
        let count = |vnum, count| makers.listed(vnum, count, 1).map(|listed| listed.count);
        // Gold keeps its count; the record holds the low 16 bits of 0x12345.
        assert_eq!(count(1, 0x0001_2345), Some(0x2345));
        // A stack is held within 1..=200; a negative `int` is a huge `DWORD`.
        assert_eq!(count(2, 0), Some(1));
        assert_eq!(count(2, 250), Some(200));
        assert_eq!(count(2, -1), Some(200));
        // `MAKECOUNT` takes `VALUE1` for 1, then the limit holds it.
        assert_eq!(count(3, 1), Some(200));
        assert_eq!(count(3, 7), Some(7));
        // Anything else is 1, and a mask shows its own vnum.
        assert_eq!(count(4, 9), Some(1));
        assert_eq!(makers.listed(4, 1, 1).map(|listed| listed.vnum), Some(40));
        // No proto and vnum 0 make no item.
        assert_eq!(makers.listed(5, 1, 1), None);
        assert_eq!(makers.listed(0, 1, 1), None);
        // The rarity: the percent, the default -1 as a `DWORD`, and 0 for 0.
        assert_eq!(makers.listed(2, 1, 2500).map(|l| l.rarity), Some(2500));
        assert_eq!(makers.listed(2, 1, -1).map(|l| l.rarity), Some(u32::MAX));
        assert_eq!(makers.listed(2, 1, 0).map(|l| l.rarity), Some(0));
    }

    #[test]
    fn the_list_runs_the_groups_in_order_and_sorts_several_without_stones() {
        let protos = ItemProtos::from_rows(vec![
            proto(10, ITEM_MATERIAL, 0),
            proto(11, ITEM_WEAPON, 3),
            proto(12, ITEM_WEAPON, 1),
            proto(13, ITEM_METIN, 0),
            proto(14, ITEM_SKILLFORGET, 0),
            proto(15, 6, 0),
            proto(16, ITEM_ARMOR, 0),
            proto(17, ITEM_USE, 0),
        ]);
        let makers = Makers {
            protos: &protos,
            masks: &[],
            count_limit: 200,
        };
        let mut tables = DropTables::default();
        tables.groups.groups.insert(
            101,
            DropGroup {
                vnum: 0,
                items: vec![row(10, 1000, 1), row(99, 5, 1), row(11, 2000, 1)],
            },
        );
        tables.mob.kill.insert(
            101,
            MobGroup {
                kill_drop: 1,
                items: vec![
                    MobItem {
                        vnum: 12,
                        count: 1,
                        part_pct: 3,
                        rare_pct: 7,
                    },
                    MobItem {
                        vnum: 13,
                        count: 1,
                        part_pct: 5,
                        rare_pct: 0,
                    },
                ],
            },
        );
        tables.mob.limit.insert(
            101,
            LevelGroup {
                level_limit: 30,
                items: vec![row(14, 3000, 1)],
            },
        );
        tables.mob.gloves.insert(
            101,
            DropGroup {
                vnum: 0,
                items: vec![row(15, 4000, 1)],
            },
        );
        tables.etc.probs.insert(16, 100);
        // Level 29 misses the limit group; the kill pick 3 is the first row.
        let low = drop_list(&tables, &makers, (101, 16), 29, Some(3));
        let vnums: Vec<u32> = low.iter().map(|listed| listed.vnum).collect();
        assert_eq!(vnums, [10, 11, 12, 15, 16]);
        // Level 30 meets it; pick 4 is the stone; no pick lists no kill row.
        let high = drop_list(&tables, &makers, (101, 16), 30, Some(4));
        let vnums: Vec<u32> = high.iter().map(|listed| listed.vnum).collect();
        assert_eq!(vnums, [10, 11, 13, 14, 15, 16]);
        let none = drop_list(&tables, &makers, (101, 0), 30, None);
        assert_eq!(none.len(), 4);
        // Several sort by kind, then sub-type, and drop the stone; the etc rarity is -1.
        assert_eq!(
            shown(&info_records(7, 101, high)),
            [
                (11, 1, 2000),
                (16, 1, u32::MAX),
                (14, 1, 3000),
                (10, 1, 1000),
                (15, 1, 4000),
            ]
        );
        let sorted = shown(&info_records(7, 101, low));
        assert_eq!(sorted[..2], [(12, 1, 7), (11, 1, 2000)]);
        // One entry is sent even when it is a stone.
        let stone = makers.listed(13, 1, 0).expect("the stone");
        assert_eq!(shown(&info_records(7, 101, vec![stone])), [(13, 1, 0)]);
        // A limit stored negative is a huge `DWORD`, so no level meets it.
        tables
            .mob
            .limit
            .get_mut(&101)
            .expect("the limit group")
            .level_limit = -1;
        assert_eq!(drop_list(&tables, &makers, (101, 0), 255, None).len(), 3);
    }
}
