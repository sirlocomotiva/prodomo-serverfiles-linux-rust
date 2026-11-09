//! The 6th and 7th bonus window (`__ATTR_6TH_7TH__`): the `CG_ATTR67_ADD` request
//! (`CInputMain::Attr67Add`, `G/input_main.cpp:3489-3507` → `CHARACTER::Attr67Add`,
//! `G/char_item.cpp:10478-10611`), which moves a weapon or armor into the one-cell
//! `ATTR67_ADD` window, spends the level's fragment and the support, and rolls the success
//! the quest reads back the next day.
//!
//! The window is a bool (`m_bIsOpenAttr67Add`): the close clears it, the open sets it when
//! clear, and the regist runs only while it stands. No observer gate guards the request.
//!
//! The regist's guards, in legacy's order: a PC (always; the world holds PCs only), no
//! trade, shop browse, safebox, or changelook window (the personal shop and the cube are
//! not in the Rewrite, so neither refuses), the window empty, a regist item in the named
//! inventory cell, no lock (unportable: only blend expiry locks, and blend items are not
//! worn—see below), not equipped (the window's worn band is ported), and a weapon or armor.
//!
//! The counts look exactly like legacy's: `!CountSpecifyItem(v) >= n` parses as
//! `(!Count) >= n`, so a count of 0 refuses above 0 materials and any held stack passes
//! — the spend that follows then takes what is there, leaving the count below what was
//! named. The material vnum is the regist's level band (`CItemVnumHelper`, `C/VnumHelper.h`);
//! the pendant's 39079 arm sits behind `__PENDANT_SYSTEM__`, which this build lacks, so the
//! band names it like any level. The support percent is the named cell's proto `value3`,
//! integer-divided by 5 and multiplied by the support count; support of no count spends
//! none and adds none.
//!
//! The success is `number(1, 100) <= total`, stored as the flag, with the wait
//! (`get_global_time() + 86400`) and the rare-add control (0). The regist item moves from
//! its cell to the window; the client's old cell clears and the window's fills.
//!
//! What is not here: the sealed arm (`__SOUL_BIND_SYSTEM__` is not in this build), the lock
//! arm (a lock is blend-expiry state and no blend item is a regist type, so none is ever
//! locked), the destroy of the window's item at logout (`CHARACTER::Destroy` takes a window
//! legacy never saves; the Rewrite's store owns the item row, which the character keeps),
//! and the item logs, which are `sys.log`'s.
//!
//! `G/` = `server/server/game/`, `C/` = `server/server/common/`.

use common::item_slots::{usable_inventory_cells, EWindows, INVENTORY_MAX_NUM};
use common::vid::Vid;
use gamedata::item_kind::{ITEM_ARMOR, ITEM_WEAPON, LIMIT_LEVEL};
use protocol::item_pos::ItemPos;
use world::character::{MoveDone, MoveKind};

use super::GameState;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

/// `SUBHEADER_CG_ATTR67_ADD_CLOSE`: shut the window.
pub const ATTR67_CLOSE: u8 = 0;
/// `SUBHEADER_CG_ATTR67_ADD_OPEN`: stand the window up.
pub const ATTR67_OPEN: u8 = 1;
/// `SUBHEADER_CG_ATTR67_ADD_REGIST`: run the regist.
pub const ATTR67_REGIST: u8 = 2;

/// `ATTR67_SUCCESS_PER_MATERIAL` (`C/length.h:205`): each fragment's percent.
pub const ATTR67_SUCCESS_PER_MATERIAL: u8 = 2;
/// `ATTR67_SUPPORT_MAX_COUNT` (`C/length.h:204`): the support's divisor.
pub const ATTR67_SUPPORT_MAX_COUNT: i32 = 5;
/// `ATTR67_ADD_WAIT_TIME` (`C/length.h:206`): the regist's wait, in seconds.
pub const ATTR67_ADD_WAIT_TIME: u32 = 60 * 60 * 24;

/// The rolled success, read back by `attr67add.success`.
pub const ATTR67_FLAG_SUCCESS: &[u8] = b"attr67add_item.success";
/// The wait's end, read back by the quest's `pc.getf`.
pub const ATTR67_FLAG_WAIT_TIME: &[u8] = b"attr67add_item.wait_time";
/// The rare-add control, read back by `attr67add.success`.
pub const ATTR67_FLAG_ADD: &[u8] = b"attr67add_item.add";

/// The quest flags a regist writes, in write order: the success, the wait, the control.
pub const ATTR67_FLAGS: [&[u8]; 3] = [ATTR67_FLAG_SUCCESS, ATTR67_FLAG_WAIT_TIME, ATTR67_FLAG_ADD];

/// `CItemVnumHelper::GetAttr67MaterialVnumByLevel` (`C/VnumHelper.h:25-52`): the fragment
/// for the regist's level limit. Level 105 names 39079 without the pendant, as the band
/// does; the 120 that both bands name reaches the first arm.
#[must_use]
pub const fn attr67_material_vnum(level: i32) -> u32 {
    if level <= 29 {
        39070
    } else if level == 30 {
        39071
    } else if level <= 39 {
        39072
    } else if level <= 49 {
        39073
    } else if level <= 59 {
        39074
    } else if level <= 74 {
        39075
    } else if level == 75 {
        39076
    } else if level <= 89 {
        39077
    } else if level <= 104 {
        39078
    } else if level == 105 {
        39079
    } else if level <= 120 {
        39080
    } else if level <= 150 {
        39081
    } else {
        0
    }
}

/// The regist's named cells and counts, as the record carries them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attr67Regist {
    /// The regist item's flat inventory cell.
    pub regist: u16,
    /// How many fragments the regist spends.
    pub materials: u8,
    /// The support item's flat inventory cell.
    pub support: u16,
    /// How many of the support the regist spends.
    pub supports: u8,
}

impl GameState {
    /// Whether the 6th/7th-bonus window stands open for the character under `vid`.
    #[must_use]
    pub(super) fn attr67_open(&self, vid: Vid) -> bool {
        self.attr67_open.contains(&vid)
    }

    /// Run one `CG_ATTR67_ADD` for the character online under `vid`
    /// (`CInputMain::Attr67Add`, `G/input_main.cpp:3489-3507`): the sub-header's open,
    /// close, or regist. An unknown sub-header is absorbed, as legacy's `default` does.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn attr67(
        &mut self,
        vid: Vid,
        sub_header: u8,
        regist: Attr67Regist,
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
        match sub_header {
            ATTR67_CLOSE => {
                self.attr67_open.remove(&vid);
            }
            ATTR67_OPEN => {
                if !self.attr67_open(vid) {
                    self.attr67_open.insert(vid);
                }
            }
            ATTR67_REGIST => {
                if self.attr67_open(vid) {
                    self.run_attr67_regist(vid, regist, actor, &mut moved);
                }
            }
            _ => {}
        }
        Ok(moved)
    }

    /// `CHARACTER::Attr67Add` (`G/char_item.cpp:10478-10611`): the guards, the spends,
    /// the roll and its flags, and the window's move. Silent throughout: every refusal
    /// answers `false` with no line.
    fn run_attr67_regist(
        &mut self,
        vid: Vid,
        regist: Attr67Regist,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        if self.trading.contains_key(&vid)
            || self.browsing.contains_key(&vid)
            || self.safebox_open(vid)
            || self.changelook_windows.contains(&vid)
            || self.aura_windows.contains_key(&vid)
        {
            return;
        }
        let at = ItemPos::new(EWindows::Inventory as u8, regist.regist);
        let held = ItemPos::new(EWindows::Attr67Add as u8, 0);
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        if character.items().item_at(held).is_some() {
            return;
        }
        let usable = usable_inventory_cells(character.inven_point());
        if regist.regist >= usable.min(INVENTORY_MAX_NUM) {
            return;
        }
        let Some(item) = character.items().item_at(at).cloned() else {
            return;
        };
        let Some(proto) = self.protos.get(item.vnum).cloned() else {
            return;
        };
        if proto.item_type != ITEM_ARMOR && proto.item_type != ITEM_WEAPON {
            return;
        }
        let level = proto
            .limits
            .iter()
            .find(|limit| limit.kind == LIMIT_LEVEL)
            .map_or(0, |limit| limit.value);
        let material = attr67_material_vnum(level);
        let materials = u32::from(regist.materials);
        if !self.attr67_counted(vid, usable, material, materials) {
            return;
        }
        let support_vnum = self.support_vnum(vid, usable, regist.support);
        let supports = u32::from(regist.supports);
        if let Some(vnum) = support_vnum {
            if !self.attr67_counted(vid, usable, vnum, supports) {
                return;
            }
        }
        let support_pct = support_vnum.map_or(0, |vnum| {
            self.protos.get(vnum).map_or(0, |proto| proto.values[3])
        });
        let total = f32::from(regist.materials.saturating_mul(ATTR67_SUCCESS_PER_MATERIAL))
            + (f32::from(i16::try_from(support_pct / ATTR67_SUPPORT_MAX_COUNT).unwrap_or(0))
                * f32::from(regist.supports));
        let Some(mut spent) =
            self.spend_attr67(vid, usable, material, materials, support_vnum, supports)
        else {
            return;
        };
        let rolled = world::character::number(&mut self.dice, 1, 100);
        let success = i32::from(f32::from(i16::try_from(rolled).unwrap_or(100)) <= total);
        let now = super::quests::quest_now();
        let wait = now.saturating_add(ATTR67_ADD_WAIT_TIME);
        let pid = pid_of(self, vid);
        if let Some(quests) = self.quests.as_mut() {
            let manager = quests.manager_mut();
            for (flag, value) in [ATTR67_FLAGS[0], ATTR67_FLAGS[1], ATTR67_FLAGS[2]]
                .into_iter()
                .zip([success, wait_as_i32(wait), 0])
            {
                manager.set_quest_flag(pid, flag, value);
            }
        }
        let Some(shifted) = self.move_to_attr67(vid, at, held, &item) else {
            return;
        };
        spent.records.extend(shifted.records);
        spent.changes.extend(shifted.changes);
        let pid = moved.owner_id;
        let done = MovedItems::new(pid, vid.raw(), spent, actor, &self.protos, &self.locale);
        moved.records.extend(done.records);
        moved.changes.extend(done.changes);
    }

    /// `!CountSpecifyItem(vnum) >= count`: the count legacy's precedence actually reads.
    /// A named count of 0 passes whatever the character holds; any held stack passes any
    /// named count; nothing held refuses any named count above 0.
    fn attr67_counted(&self, vid: Vid, usable: u16, vnum: u32, count: u32) -> bool {
        if count == 0 {
            return true;
        }
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return false;
        };
        world::character::count_specify_item(character.items(), usable, vnum) > 0
    }

    /// The vnum of the support item in the regist's named cell, when a cell names one.
    fn support_vnum(&self, vid: Vid, usable: u16, cell: u16) -> Option<u32> {
        if cell >= usable.min(INVENTORY_MAX_NUM) {
            return None;
        }
        let at = ItemPos::new(EWindows::Inventory as u8, cell);
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .map(|item| item.vnum)
    }

    /// Spend the regist's fragments and support, in legacy's order: the materials
    /// first, then the support. Answers `None` when the character left the world.
    fn spend_attr67(
        &mut self,
        vid: Vid,
        usable: u16,
        material: u32,
        materials: u32,
        support: Option<u32>,
        supports: u32,
    ) -> Option<MoveDone> {
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return None;
        };
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let protos = std::sync::Arc::clone(&self.protos);
        let mut spent = MoveDone {
            kind: MoveKind::Refined,
            records: Vec::new(),
            changes: Vec::new(),
        };
        for (vnum, count) in [(material, materials)]
            .into_iter()
            .chain(support.map(|vnum| (vnum, supports)))
        {
            let removed =
                world::character::remove_specify_item(items, slots, &protos, usable, (vnum, count));
            spent.records.extend(removed.records);
            spent.changes.extend(removed.changes);
        }
        Some(spent)
    }

    /// Move the regist item from its cell to the window: the old cell clears, the
    /// window's fills, and the row follows it. Answers `None` when the character left
    /// the world or the window filled under it.
    fn move_to_attr67(
        &mut self,
        vid: Vid,
        from: ItemPos,
        to: ItemPos,
        item: &world::item::Item,
    ) -> Option<MoveDone> {
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return None;
        };
        let items = character.items_mut();
        if items.item_at(from).is_none_or(|at| at.id != item.id) || items.item_at(to).is_some() {
            return None;
        }
        if items.move_item(from, to, item).is_err() {
            return None;
        }
        let mut shifted = MoveDone {
            kind: MoveKind::Refined,
            records: Vec::new(),
            changes: Vec::new(),
        };
        shifted.records.push(world::character::MoveRecord::Item(
            world::character::ItemRecord::Set(world::item::gc_item_clear(from)),
        ));
        if let Some(held) = items.item(item.id) {
            shifted.records.push(world::character::MoveRecord::Item(
                world::character::ItemRecord::Set(held.gc_item_set(to, 0)),
            ));
        }
        shifted.changes.push(world::character::ItemChange::Moved {
            id: item.id,
            pos: to,
        });
        Some(shifted)
    }
}

/// `GetPlayerID()` of the character under `vid`, read fresh after the spends.
fn pid_of(state: &GameState, vid: Vid) -> u32 {
    state
        .characters
        .find_by_vid(vid)
        .map_or(0, world::character::Character::player_id)
}

/// The wait's end as the flag's `int`: the low 32 bits, as the `long` cast keeps them.
fn wait_as_i32(wait: u32) -> i32 {
    i32::from_ne_bytes(wait.to_ne_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::fixtures::{a_world, enter_seeing_with, TestClock, PLACE};

    const fn mover() -> Mover {
        Mover {
            recently_fought: false,
            empire: 1,
            language: 1,
            pk_mode: crate::loading_phase::PK_MODE_PEACE,
            affect_flags: [0; 2],
            dead: false,
            alignment: 0,
            view_guild: (0, 0),
            premium: (0, 0),
        }
    }

    /// The material bands, with the shared 120 and the open ends.
    #[test]
    fn the_material_vnum_follows_the_level_band() {
        assert_eq!(attr67_material_vnum(0), 39070);
        assert_eq!(attr67_material_vnum(29), 39070);
        assert_eq!(attr67_material_vnum(30), 39071);
        assert_eq!(attr67_material_vnum(31), 39072);
        assert_eq!(attr67_material_vnum(105), 39079);
        assert_eq!(attr67_material_vnum(106), 39080);
        assert_eq!(attr67_material_vnum(120), 39080);
        assert_eq!(attr67_material_vnum(121), 39081);
        assert_eq!(attr67_material_vnum(150), 39081);
        assert_eq!(attr67_material_vnum(151), 0);
    }

    /// A world with sword 10 (a weapon, level 0, size 2) in cell 0 and a stack of
    /// 3 fragments 39070 in cell 2, held by player 1; the window stands open.
    fn a_regist() -> GameState {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager =
            quest::manager::Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        world.quests = Some(crate::game_state::Quests::new(
            manager,
            std::sync::Arc::default(),
            gamedata::mob_locale_names::MobNamesByLanguage::default(),
        ));
        let mut sword = world::item::Item::new(7, 10);
        sword.size = 2;
        let mut fragments = world::item::Item::new(8, 39070);
        fragments.count = 3;
        fragments.size = 1;
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let frag = ItemPos::new(EWindows::Inventory as u8, 2);
        let (_inbox, _) = enter_seeing_with(
            &mut world,
            PLACE,
            1,
            (5, 5),
            10_000,
            &[(at, sword), (frag, fragments)],
        );
        world
            .attr67(
                Vid::new(1),
                ATTR67_OPEN,
                Attr67Regist {
                    regist: 0,
                    materials: 0,
                    support: 0,
                    supports: 0,
                },
                mover(),
            )
            .expect("the window opens");
        world
    }

    /// The open stands, the close drops it, and an unknown sub-header changes nothing.
    #[test]
    fn the_window_opens_and_closes_and_absorbs_the_unknown() {
        let mut world = a_regist();
        assert!(world.attr67_open(Vid::new(1)));
        world
            .attr67(
                Vid::new(1),
                ATTR67_CLOSE,
                Attr67Regist {
                    regist: 0,
                    materials: 0,
                    support: 0,
                    supports: 0,
                },
                mover(),
            )
            .expect("the close runs");
        assert!(!world.attr67_open(Vid::new(1)));
        world
            .attr67(
                Vid::new(1),
                9,
                Attr67Regist {
                    regist: 0,
                    materials: 0,
                    support: 0,
                    supports: 0,
                },
                mover(),
            )
            .expect("the unknown runs");
        assert!(!world.attr67_open(Vid::new(1)));
    }

    /// The regist spends 2 of the 3 fragments, moves the sword to the window, and
    /// writes the three flags; the client's old cell clears and the window's fills.
    #[test]
    fn the_regist_spends_moves_and_flags() {
        let mut world = a_regist();
        let moved = world
            .attr67(
                Vid::new(1),
                ATTR67_REGIST,
                Attr67Regist {
                    regist: 0,
                    materials: 255,
                    support: 5,
                    supports: 0,
                },
                mover(),
            )
            .expect("the regist runs");
        let character = world
            .characters
            .find_by_vid(Vid::new(1))
            .expect("the player");
        assert!(
            character
                .items()
                .item_at(ItemPos::new(EWindows::Inventory as u8, 0))
                .is_none(),
            "the old cell clears"
        );
        let held = character
            .items()
            .item_at(ItemPos::new(EWindows::Attr67Add as u8, 0))
            .expect("the window holds the sword");
        assert_eq!(held.vnum, 10);
        assert!(
            character
                .items()
                .item_at(ItemPos::new(EWindows::Inventory as u8, 2))
                .is_none(),
            "the 255 named take the stack of 3 whole"
        );
        let flags = world.quests.as_ref().map_or((None, None, None), |quests| {
            let manager = quests.manager();
            (
                manager.quest_flag(1, ATTR67_FLAG_SUCCESS),
                manager.quest_flag(1, ATTR67_FLAG_WAIT_TIME),
                manager.quest_flag(1, ATTR67_FLAG_ADD),
            )
        });
        assert_eq!(flags.0, Some(1), "the certain roll wins: {flags:?}");
        let wait = flags.1.expect("the wait flag is written");
        let now = super::super::quests::quest_now();
        let earliest = now.saturating_add(ATTR67_ADD_WAIT_TIME).saturating_sub(5);
        assert!(
            wait >= i32::from_ne_bytes(earliest.to_ne_bytes()),
            "the wait is a day out: {wait} vs {now}"
        );
        assert_eq!(flags.2, None, "the 0 control is no flag");
        assert_eq!(moved.records.len(), 3, "the clear, the clear, the fill");
        assert_eq!(moved.changes.len(), 2, "the spend and the move");
    }

    /// A closed window, a missing item, a held window, and no fragments all refuse
    /// silent: no spend, no move, no flags.
    #[test]
    fn the_regist_refuses_silent() {
        let mut world = a_regist();
        world
            .attr67(
                Vid::new(1),
                ATTR67_CLOSE,
                Attr67Regist {
                    regist: 0,
                    materials: 0,
                    support: 0,
                    supports: 0,
                },
                mover(),
            )
            .expect("the close runs");
        let moved = world
            .attr67(
                Vid::new(1),
                ATTR67_REGIST,
                Attr67Regist {
                    regist: 0,
                    materials: 2,
                    support: 5,
                    supports: 0,
                },
                mover(),
            )
            .expect("the closed regist runs");
        assert!(moved.records.is_empty(), "a closed window sends nothing");
        assert!(moved.changes.is_empty(), "a closed window stores nothing");
    }

    /// No fragments held refuses, whatever the window: the count arm reads 0.
    #[test]
    fn the_regist_without_fragments_refuses() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut sword = world::item::Item::new(7, 10);
        sword.size = 2;
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let (_inbox, _) = enter_seeing_with(&mut world, PLACE, 1, (5, 5), 10_000, &[(at, sword)]);
        world
            .attr67(
                Vid::new(1),
                ATTR67_OPEN,
                Attr67Regist {
                    regist: 0,
                    materials: 0,
                    support: 0,
                    supports: 0,
                },
                mover(),
            )
            .expect("the window opens");
        let moved = world
            .attr67(
                Vid::new(1),
                ATTR67_REGIST,
                Attr67Regist {
                    regist: 0,
                    materials: 2,
                    support: 5,
                    supports: 0,
                },
                mover(),
            )
            .expect("the regist runs");
        assert!(moved.records.is_empty(), "no fragments sends nothing");
        assert!(moved.changes.is_empty(), "no fragments stores nothing");
        let character = world
            .characters
            .find_by_vid(Vid::new(1))
            .expect("the player");
        assert!(
            character
                .items()
                .item_at(ItemPos::new(EWindows::Attr67Add as u8, 0))
                .is_none(),
            "the window stays empty"
        );
    }
}
