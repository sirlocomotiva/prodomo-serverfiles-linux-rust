//! The giftbox and treasure-key uses: `case ITEM_GIFTBOX` (`G/char_item.cpp:3180-3258`)
//! and `case ITEM_TREASURE_KEY` (`:3099-3177`).
//!
//! The battle-pass `CHEST_OPEN` mission registers first, then the dragon-soul gate
//! refuses a 51500-52000 box to an unqualified caller with `[LS;1094]`. The box's
//! vnum names its special group; with no group the use talks `[LS;56]` and keeps
//! the box. The draw is the group's (`GetMultiIndex`, already ported as
//! [`SpecialItemGroup::multi_index`]): a `Pct` bag rolls `number(1, 100)` per row,
//! any other bag draws once over the totals. Each drawn row runs its arm — the
//! seven keyword arms or the item arm — and only when every arm lands is one box
//! spent and the row's line told. A failed draw talks the failure line instead:
//! the box is kept, but the gifts the earlier rows already gave stay given and
//! sent, as legacy's early `return false` sends them.
//!
//! The key's use runs the same draw on the box it names: a missing or held box is
//! silent, a non-box talks `[LS;1248]`, a value mismatch or a failed draw talks
//! `[LS;46]`, and the key and the box go only when every arm lands. The key has
//! no battle-pass hook and no dragon-soul gate.
//!
//! # The arms
//!
//! - `GOLD` (1): nothing. `PointChange(POINT_GOLD)` is an empty case under
//!   `ENABLE_REMOVE_LIMIT_GOLD` (`G/char.cpp:4406-4423`), so legacy grants no
//!   gold; the line still prints. No group can even hold the row: the reader
//!   refuses `gold` as an unknown item.
//! - `EXP` (2): `gain_pc_exp`.
//! - `MOB` (3): the count is the mob vnum; it spawns at the opener's spot plus
//!   `number(-500, 500)` each way, facing -1 with spawn motion, aggressive. A
//!   spawn that stands nothing fails the use.
//! - `SLOW` (4): the 300 s slow with value minus the count.
//! - `DRAIN_HP` (5): `max_hp * count / 100`, capped at `hp - 1`, off the HP.
//! - `POISON` (6): `poison_pc` with no attacker.
//! - `MOB_GROUP` (7): the count names the mob group, stood in the rect the four
//!   `number(300, 500)` rolls draw around the opener, aggressive.
//! - default: `AutoGiveItem(vnum, count, rare)`; a give that makes nothing fails
//!   the use. Vnum 0 lands here too, as legacy's `switch` has no `NONE` case, so
//!   `CreateItem(0)` returns null and fails it.
//!
//! # Not ported
//!
//! - The `ENABLE_NEWSTUFF` box time limit (`:3182-3191`): its value stays 0, as
//!   only the never-loaded `questlib_extra` sets `box_use_limit_time`.
//!
//! `G/` = `server/server/game/`.

use common::point_slot as point;
use common::vid::Vid;
use gamedata::battle_pass::MISSION_TYPE_CHEST_OPEN;
use gamedata::item_kind::ITEM_GIFTBOX;
use gamedata::mob_proto::AIFLAG_AGGRESSIVE;
use protocol::gc_chat::{CHAT_TYPE_INFO, CHAT_TYPE_TALKING};
use protocol::item_pos::ItemPos;
use world::character::{number, AutoGiven, MoveDone};

use super::affect::Affect;
use super::GameState;
use crate::chat_line::{chat_packet, Arg};
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

/// The dragon-soul boxes (`:3204`): a caller without the qualification opens none.
const DRAGON_SOUL_BOX_MIN: u32 = 51_500;
/// The dragon-soul boxes (`:3204`): a caller without the qualification opens none.
const DRAGON_SOUL_BOX_MAX: u32 = 52_000;
/// The clam shell (`case 27987`, `G/char_item.cpp:4294-4344`): an `ITEM_GIFTBOX`
/// proto with a hardcoded draw in the `USE_SPECIAL` switch.
const CLAM_SHELL: u32 = 27_987;
/// The shell's water-stone piece (`:4313`).
const SHELL_WATER_STONE: u32 = 27_990;
/// The shell's white, blue and blood pearls (`:4326-4340`).
const SHELL_PEARLS: [u32; 3] = [27_992, 27_993, 27_994];
/// The shell's roll ceiling for the water stone (`:4311`): 50.
const SHELL_STONE_CEILING: i32 = 50;
/// The shell's dead-air ceiling (`prob_table_gb2312[0]`, `:4321`): 95.
const SHELL_DEAD_CEILING: i32 = 95;
/// The shell's white-pearl ceiling (`[1]`): 97.
const SHELL_WHITE_CEILING: i32 = 97;
/// The shell's blue-pearl ceiling (`[2]`): 99.
const SHELL_BLUE_CEILING: i32 = 99;

/// The slow's seconds (`:9628`).
const SLOW_SECS: i32 = 300;

impl GameState {
    /// Whether the item at `at` is a giftbox (`ITEM_GIFTBOX`).
    pub(super) fn giftbox_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .and_then(|item| self.protos.get(item.vnum))
            .is_some_and(|proto| proto.item_type == ITEM_GIFTBOX)
    }

    /// Whether the item at `at` is the 27987 clam shell (`case 27987`,
    /// `G/char_item.cpp:4294-4344`).
    pub(super) fn clam_shell_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .is_some_and(|item| item.vnum == CLAM_SHELL)
    }

    /// Crack one clam shell (`case 27987`, `G/char_item.cpp:4294-4344`): the
    /// `CHEST_OPEN` mission registers first, then the use checks run and one
    /// shell spends. The roll is `number(1, 100)`: 50 or less tells `[LS;221]`
    /// and gives the 27990 water-stone piece; 95 or less tells `[LS;232]` and
    /// gives nothing; 97 or less tells `[LS;243]` and gives the 27992 white
    /// pearl; 99 or less tells `[LS;253]` and gives the 27993 blue pearl; 100
    /// tells `[LS;264]` and gives the 27994 blood pearl. The give rides
    /// `AutoGiveItem`, whose extras fall to the feet when the pack is full.
    pub(super) fn use_clam_shell(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        self.register_pass_item(vid, actor, MISSION_TYPE_CHEST_OPEN, 1, CLAM_SHELL);
        let roll = number(&mut self.dice, 1, 100);
        let (line, gift) = if roll <= SHELL_STONE_CEILING {
            (b"[LS;221]".as_slice(), Some(SHELL_WATER_STONE))
        } else if roll <= SHELL_DEAD_CEILING {
            (b"[LS;232]".as_slice(), None)
        } else if roll <= SHELL_WHITE_CEILING {
            (b"[LS;243]".as_slice(), Some(SHELL_PEARLS[0]))
        } else if roll <= SHELL_BLUE_CEILING {
            (b"[LS;253]".as_slice(), Some(SHELL_PEARLS[1]))
        } else {
            (b"[LS;264]".as_slice(), Some(SHELL_PEARLS[2]))
        };
        let mut moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, _protos| {
            world::character::consume_giftbox(items, at, rules, gear)
        })?;
        let questing = self.quest_running(vid);
        if let Some(gift) = gift {
            let mut gifts = MoveDone {
                kind: world::character::MoveKind::Used,
                records: Vec::new(),
                changes: Vec::new(),
            };
            let Some((outcome, done)) = self.auto_give(vid, (gift, 1), 0, questing) else {
                return Err(MoveItemRefused::NoSuchCharacter { vid });
            };
            match outcome {
                AutoGiven::NoProto | AutoGiven::NotMade => {
                    return Err(MoveItemRefused::NoSuchCharacter { vid });
                }
                AutoGiven::Merged(_) | AutoGiven::Placed { .. } => {}
                AutoGiven::Unplaced(item) => self.lay_auto_given(vid, item),
            }
            gifts.records.extend(done.records);
            gifts.changes.extend(done.changes);
            let packed = MovedItems::new(
                moved.owner_id,
                vid.raw(),
                gifts,
                actor,
                &self.protos,
                &self.locale,
            );
            let mut records = packed.records;
            records.append(&mut moved.records);
            self.push_group_row_lines(actor.recipient(&self.locale), gift, 1, &mut records);
            moved.records = records;
            let mut changes = packed.changes;
            changes.append(&mut moved.changes);
            moved.changes = changes;
        }
        self.tell(vid, actor, CHAT_TYPE_INFO, line, &[]);
        Ok(moved)
    }

    /// Run one giftbox use: the mission, the gate, the draw, the arms, the spend,
    /// and the lines.
    pub(super) fn use_giftbox(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        if self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .is_some_and(world::character::Points::is_secured)
        {
            return Err(MoveItemRefused::Refused(
                world::character::MoveRefused::Secured,
            ));
        }
        let box_vnum = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .map_or(0, |item| item.vnum);
        self.register_pass_item(vid, actor, MISSION_TYPE_CHEST_OPEN, 1, box_vnum);
        if (DRAGON_SOUL_BOX_MIN..DRAGON_SOUL_BOX_MAX).contains(&box_vnum) && !self.ds_qualified(vid)
        {
            self.tell(vid, actor, CHAT_TYPE_INFO, b"[LS;1094]", &[]);
            return Ok(self.giftbox_kept(vid, actor));
        }
        let questing = self.quest_running(vid);
        let mut gifts = MoveDone {
            kind: world::character::MoveKind::Used,
            records: Vec::new(),
            changes: Vec::new(),
        };
        let mut lines: Vec<(u32, i32)> = Vec::new();
        if !self.draw_group_rows(vid, box_vnum, questing, &mut gifts, &mut lines) {
            return Ok(self.pack_failed_draw(vid, actor, gifts, b"[LS;56]"));
        }
        let mut moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, _protos| {
            world::character::consume_giftbox(items, at, rules, gear)
        })?;
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        let packed = MovedItems::new(pid, vid.raw(), gifts, actor, &self.protos, &self.locale);
        let to = actor.recipient(&self.locale);
        let mut records = packed.records;
        records.append(&mut moved.records);
        for (vnum, count) in lines {
            self.push_group_row_lines(to, vnum, count, &mut records);
        }
        moved.records = records;
        let mut changes = packed.changes;
        changes.append(&mut moved.changes);
        moved.changes = changes;
        Ok(moved)
    }

    /// Run one treasure-key use on the box at `dest`: the checks, the box's draw,
    /// the spend of both, and the lines (`case ITEM_TREASURE_KEY`,
    /// `G/char_item.cpp:3099-3177`).
    pub(super) fn key_step(
        &mut self,
        vid: Vid,
        at: ItemPos,
        dest: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        if self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .is_some_and(world::character::Points::is_secured)
        {
            return Err(MoveItemRefused::Refused(
                world::character::MoveRefused::Secured,
            ));
        }
        // The key's checks run before the draw, whose arms have side effects.
        self.run_item_step(vid, actor, |items, _ids, rules, gear, _protos| {
            world::character::check_key_use(items, at, rules, gear)
        })?;
        let box_vnum = self.characters.find_by_vid(vid).ok().and_then(|character| {
            let box_item = character.items().item_at(dest)?;
            if character.items().is_exchanging(box_item.id) {
                return None;
            }
            if world::character::is_equip_position(dest) {
                return None;
            }
            Some(box_item.vnum)
        });
        let Some(box_vnum) = box_vnum else {
            return Ok(self.giftbox_kept(vid, actor));
        };
        let box_type = self.protos.get(box_vnum).map(|proto| proto.item_type);
        if box_type != Some(gamedata::item_kind::ITEM_TREASURE_BOX) {
            self.tell(vid, actor, CHAT_TYPE_TALKING, b"[LS;1248]", &[]);
            return Ok(self.giftbox_kept(vid, actor));
        }
        let key_value = self.key_value0(vid, at);
        let box_value = self.protos.get(box_vnum).map_or(0, |proto| proto.values[0]);
        if key_value != box_value {
            self.tell(vid, actor, CHAT_TYPE_TALKING, b"[LS;46]", &[]);
            return Ok(self.giftbox_kept(vid, actor));
        }
        let questing = self.quest_running(vid);
        let mut gifts = MoveDone {
            kind: world::character::MoveKind::Used,
            records: Vec::new(),
            changes: Vec::new(),
        };
        let mut lines: Vec<(u32, i32)> = Vec::new();
        if !self.draw_group_rows(vid, box_vnum, questing, &mut gifts, &mut lines) {
            return Ok(self.pack_failed_draw(vid, actor, gifts, b"[LS;46]"));
        }
        let mut moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, _protos| {
            world::character::consume_key_and_box(items, at, dest, rules, gear)
        })?;
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        let packed = MovedItems::new(pid, vid.raw(), gifts, actor, &self.protos, &self.locale);
        let to = actor.recipient(&self.locale);
        let mut records = packed.records;
        records.append(&mut moved.records);
        for (vnum, count) in lines {
            self.push_group_row_lines(to, vnum, count, &mut records);
        }
        moved.records = records;
        let mut changes = packed.changes;
        changes.append(&mut moved.changes);
        moved.changes = changes;
        Ok(moved)
    }

    /// The key's `value0`, or 0 with no key.
    fn key_value0(&self, vid: Vid, at: ItemPos) -> i32 {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .and_then(|key| self.protos.get(key.vnum))
            .map_or(0, |proto| proto.values[0])
    }

    /// The box's draw and its arms (`GiveItemFromSpecialItemGroup`,
    /// `G/char_item.cpp:9575-9690`): the rows land into `gifts`, their `(vnum,
    /// count)` into `lines`. A missing or empty group fails, as a failed arm
    /// does; the gifts the earlier rows already gave stay in `gifts`.
    fn draw_group_rows(
        &mut self,
        vid: Vid,
        box_vnum: u32,
        questing: bool,
        gifts: &mut MoveDone,
        lines: &mut Vec<(u32, i32)>,
    ) -> bool {
        let key = i32::try_from(box_vnum).unwrap_or(i32::MAX);
        let Some(group) = self.item_groups.special.get(&key).cloned() else {
            return false;
        };
        if group.is_empty() {
            // Legacy reads `m_vecProbs.back()` on the empty vector, a Defect.
            return false;
        }
        let rolls: Vec<i32> = if group.group_type == gamedata::special_item_group::GroupType::Pct {
            (0..group.group_size())
                .map(|_| number(&mut self.dice, 1, 100))
                .collect()
        } else {
            let back = group.totals.last().copied().unwrap_or(0).max(1);
            vec![number(&mut self.dice, 1, back)]
        };
        for index in group.multi_index(&rolls) {
            let (Some(vnum), Some(count)) = (group.vnum(index), group.count(index)) else {
                continue;
            };
            let rare = group.rare_pct(index).unwrap_or(0);
            if !self.land_group_row(vid, vnum, count, rare, questing, gifts) {
                return false;
            }
            lines.push((vnum, count));
        }
        true
    }

    /// A failed draw's answer: the gifts the earlier rows gave, then the failure
    /// line. The box is kept.
    fn pack_failed_draw(&self, vid: Vid, actor: Mover, gifts: MoveDone, line: &[u8]) -> MovedItems {
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        let mut packed = MovedItems::new(pid, vid.raw(), gifts, actor, &self.protos, &self.locale);
        let to = actor.recipient(&self.locale);
        packed.records.push(crate::chat_line::chat_packet(
            to,
            CHAT_TYPE_TALKING,
            line,
            &[],
        ));
        packed
    }

    /// The kept box: the use talked its refusal, and nothing changed.
    fn giftbox_kept(&self, vid: Vid, actor: Mover) -> MovedItems {
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        MovedItems::new(
            pid,
            vid.raw(),
            MoveDone {
                kind: world::character::MoveKind::Used,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        )
    }

    /// One drawn row's arm (`:9589-9679`): the gift lands, or the use fails.
    fn land_group_row(
        &mut self,
        vid: Vid,
        vnum: u32,
        count: i32,
        rare: i32,
        questing: bool,
        gifts: &mut MoveDone,
    ) -> bool {
        match vnum {
            // `PointChange(POINT_GOLD)` grants nothing under the gold switch; the
            // arm still succeeds.
            1 => true,
            2 => {
                self.gain_pc_exp(vid, i64::from(count));
                true
            }
            3 => {
                let Some(place) = self.place_of(vid.raw()) else {
                    return false;
                };
                let Some(spot) = self.spot_of(vid.raw()) else {
                    return false;
                };
                let dx = number(&mut self.dice, -500, 500);
                let dy = number(&mut self.dice, -500, 500);
                let at = (spot.x.saturating_add(dx), spot.y.saturating_add(dy));
                let mob = u32::try_from(count.max(0)).unwrap_or(0);
                let Some(stood) = self.spawn_event_mob(place, mob, at, -1, true) else {
                    return false;
                };
                if let Some(npc) = self.npc_mut(stood) {
                    npc.ai_flags |= AIFLAG_AGGRESSIVE;
                }
                true
            }
            4 => {
                use crate::loading_phase::{AFFECT_SLOW, AFF_SLOW};
                self.add_pc_affect(
                    vid,
                    Affect::buff(
                        AFFECT_SLOW,
                        point::POINT_MOV_SPEED,
                        -count,
                        AFF_SLOW,
                        SLOW_SECS,
                        0,
                    ),
                    true,
                );
                true
            }
            5 => {
                let (hp, max_hp) = self
                    .characters
                    .find_by_vid(vid)
                    .ok()
                    .and_then(|character| character.points())
                    .map_or((0, 0), |points| (points.hp(), points.max_hp()));
                let drop = (i64::from(max_hp) * i64::from(count) / 100)
                    .min(i64::from(hp.saturating_sub(1)))
                    .max(0);
                let drop = i32::try_from(drop).unwrap_or(i32::MAX);
                self.change_pc_point(vid, point::POINT_HP, -drop, false);
                true
            }
            6 => {
                self.poison_pc(vid, None);
                true
            }
            7 => {
                let Some(place) = self.place_of(vid.raw()) else {
                    return false;
                };
                let Some(spot) = self.spot_of(vid.raw()) else {
                    return false;
                };
                let sx = spot.x.saturating_sub(number(&mut self.dice, 300, 500));
                let sy = spot.y.saturating_sub(number(&mut self.dice, 300, 500));
                let ex = spot.x.saturating_add(number(&mut self.dice, 300, 500));
                let ey = spot.y.saturating_add(number(&mut self.dice, 300, 500));
                let group = u32::try_from(count.max(0)).unwrap_or(0);
                self.spawn_event_group_aggressive(place, group, (sx, sy, ex, ey));
                true
            }
            _ => {
                let count = u16::try_from(count.max(0)).unwrap_or(u16::MAX);
                let Some((outcome, done)) = self.auto_give(vid, (vnum, count), rare, questing)
                else {
                    return false;
                };
                match outcome {
                    AutoGiven::NoProto | AutoGiven::NotMade => return false,
                    AutoGiven::Merged(_) | AutoGiven::Placed { .. } => {}
                    AutoGiven::Unplaced(item) => self.lay_auto_given(vid, item),
                }
                gifts.records.extend(done.records);
                gifts.changes.extend(done.changes);
                true
            }
        }
    }

    /// One landed row's lines (`:3217-3247`): the keyword's, or the gift's name.
    /// Experience tells two: `[LS;1279]`, then `[LS;1290;%d]`.
    fn push_group_row_lines(
        &self,
        to: crate::chat_line::Recipient<'_>,
        vnum: u32,
        count: i32,
        records: &mut Vec<Vec<u8>>,
    ) {
        let count = i64::from(count);
        if vnum == 2 {
            records.push(chat_packet(to, CHAT_TYPE_INFO, b"[LS;1279]", &[]));
        }
        let line = match vnum {
            1 => chat_packet(to, CHAT_TYPE_INFO, b"[LS;1269;%d]", &[Arg::Int(count)]),
            2 => chat_packet(to, CHAT_TYPE_INFO, b"[LS;1290;%d]", &[Arg::Int(count)]),
            3 | 7 => chat_packet(to, CHAT_TYPE_INFO, b"[LS;1299]", &[]),
            4 => chat_packet(to, CHAT_TYPE_INFO, b"[LS;1310]", &[]),
            5 => chat_packet(to, CHAT_TYPE_INFO, b"[LS;3]", &[]),
            6 => chat_packet(to, CHAT_TYPE_INFO, b"[LS;13]", &[]),
            _ => {
                let name = self.item_get_name(vnum);
                if count > 1 {
                    chat_packet(
                        to,
                        CHAT_TYPE_INFO,
                        b"[LS;204;%s;%d]",
                        &[Arg::Text(&name), Arg::Int(count)],
                    )
                } else {
                    chat_packet(to, CHAT_TYPE_INFO, b"[LS;35;%s]", &[Arg::Text(&name)])
                }
            }
        };
        records.push(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::fixtures::{a_world, enter, enter_seeing_with, TestClock, PLACE};
    use common::item_slots::EWindows;
    use world::item::Item;

    const VID: u32 = 0x6001;
    const BOX_VNUM: u32 = 50_011;

    fn mover() -> Mover {
        Mover {
            recently_fought: false,
            empire: 1,
            language: 0,
            pk_mode: 4,
            affect_flags: [0; 2],
            dead: false,
            killer_mode: false,
            alignment: 0,
            view_guild: (0, 0),
            premium: (0, 0),
        }
    }

    fn text_of(line: &[u8]) -> Vec<u8> {
        let size = usize::from(u16::from_le_bytes([line[1], line[2]]));
        assert_eq!(size, line.len());
        assert_eq!(line[0], 4, "HEADER_GC_CHAT");
        line[10..].to_vec()
    }

    fn boxed(vnum: u32, id: u32, cell: u16) -> (ItemPos, Item) {
        let mut box_item = Item::new(id, vnum);
        box_item.set_size(1).expect("a positive footprint");
        (ItemPos::new(EWindows::Inventory as u8, cell), box_item)
    }

    fn box_at() -> ItemPos {
        ItemPos::new(EWindows::Inventory as u8, 0)
    }

    /// With no group the use talks `[LS;56]` and the box stays shut.
    #[test]
    fn without_a_group_the_box_is_kept_and_talks() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let held = vec![boxed(BOX_VNUM, 60, 0)];
        let (mut inbox, _) = enter_seeing_with(&mut world, PLACE, VID, (5, 5), 10_000, &held);
        assert!(
            !world.item_groups.special.contains_key(&50_011),
            "the default table holds no moonlight group"
        );
        let moved = world
            .use_giftbox(Vid::new(VID), box_at(), mover())
            .expect("a kept box is no refusal");
        assert!(inbox.try_recv().is_err(), "nothing tells ahead");
        assert_eq!(moved.records.len(), 1, "only the failure line sends");
        assert_eq!(text_of(&moved.records[0]), b"[LS;56]");
        assert!(moved.changes.is_empty(), "nothing saves");
        let kept = world
            .characters
            .find_by_vid(Vid::new(VID))
            .expect("online")
            .items()
            .item_at(box_at())
            .expect("the box stays");
        assert_eq!((kept.vnum, kept.count), (BOX_VNUM, 1));
    }

    /// The dragon-soul gate fires before the group lookup: an unqualified caller
    /// hears `[LS;1094]` for a 51500-52000 box that has no group either.
    #[test]
    fn a_dragon_soul_box_refuses_the_unqualified_first() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let held = vec![boxed(51_505, 60, 0)];
        let (mut inbox, _) = enter_seeing_with(&mut world, PLACE, VID, (5, 5), 10_000, &held);
        assert!(!world.ds_qualified(Vid::new(VID)), "no 540 row");
        let moved = world
            .use_giftbox(Vid::new(VID), box_at(), mover())
            .expect("a kept box is no refusal");
        assert_eq!(text_of(&inbox.try_recv().expect("the gate")), b"[LS;1094]");
        assert!(moved.records.is_empty(), "nothing else sends");
    }

    /// The `GOLD` arm grants nothing: `PointChange(POINT_GOLD)` is empty under the
    /// gold switch, and the arm still succeeds.
    #[test]
    fn a_gold_row_grants_nothing_and_succeeds() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, VID, (5, 5), 10_000);
        let before = world
            .characters
            .find_by_vid(Vid::new(VID))
            .expect("online")
            .gold();
        let mut gifts = MoveDone {
            kind: world::character::MoveKind::Used,
            records: Vec::new(),
            changes: Vec::new(),
        };
        assert!(world.land_group_row(Vid::new(VID), 1, 500, -1, false, &mut gifts));
        let after = world
            .characters
            .find_by_vid(Vid::new(VID))
            .expect("online")
            .gold();
        assert_eq!((before, after), (0, 0), "no gold moves");
        assert!(gifts.records.is_empty() && gifts.changes.is_empty());
    }

    /// The `EXP` arm banks through the live grant.
    #[test]
    fn an_exp_row_banks() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, VID, (5, 5), 10_000);
        let mut gifts = MoveDone {
            kind: world::character::MoveKind::Used,
            records: Vec::new(),
            changes: Vec::new(),
        };
        assert!(world.land_group_row(Vid::new(VID), 2, 1_000, -1, false, &mut gifts));
        let points = world
            .characters
            .find_by_vid(Vid::new(VID))
            .expect("online")
            .points()
            .expect("points");
        assert_eq!(points.exp(), 1_000);
    }

    /// The `SLOW` arm lands minus the count for 300 seconds.
    #[test]
    fn a_slow_row_lands_minus_its_count_for_300() {
        use crate::loading_phase::AFFECT_SLOW;
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, VID, (5, 5), 10_000);
        let mut gifts = MoveDone {
            kind: world::character::MoveKind::Used,
            records: Vec::new(),
            changes: Vec::new(),
        };
        assert!(world.land_group_row(Vid::new(VID), 4, 7, -1, false, &mut gifts));
        let event = world
            .affect_events
            .get(&Vid::new(VID))
            .expect("the slow lands");
        let slow = event
            .affects
            .iter()
            .find(|affect| affect.kind == AFFECT_SLOW)
            .expect("a slow row");
        assert_eq!((slow.value, slow.remaining), (-7, 300));
    }

    /// The `DRAIN_HP` arm takes `max_hp * count / 100` but never the last point.
    #[test]
    fn a_drain_row_takes_its_share_and_never_the_last_point() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, VID, (5, 5), 10_000);
        let (hp, max_hp) = world
            .characters
            .find_by_vid(Vid::new(VID))
            .expect("online")
            .points()
            .map_or((0, 0), |points| (points.hp(), points.max_hp()));
        let mut gifts = MoveDone {
            kind: world::character::MoveKind::Used,
            records: Vec::new(),
            changes: Vec::new(),
        };
        assert!(world.land_group_row(Vid::new(VID), 5, 50, -1, false, &mut gifts));
        let drop = (i64::from(max_hp) * 50 / 100).min(i64::from(hp - 1)).max(0);
        let want = hp - i32::try_from(drop).unwrap_or(i32::MAX);
        let after = world
            .characters
            .find_by_vid(Vid::new(VID))
            .expect("online")
            .points()
            .expect("points")
            .hp();
        assert_eq!(after, want.max(1));
        assert!(after >= 1, "the last point stays");
    }

    /// The `MOB` arm with nowhere to stand fails the use, as a null spawn does.
    #[test]
    fn a_mob_row_with_no_spot_fails() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut gifts = MoveDone {
            kind: world::character::MoveKind::Used,
            records: Vec::new(),
            changes: Vec::new(),
        };
        assert!(!world.land_group_row(Vid::new(VID), 3, 101, -1, false, &mut gifts));
    }

    /// The item arm fails with no proto, and vnum 0 fails as `CreateItem(0)`
    /// answers null.
    #[test]
    fn an_item_row_without_a_proto_and_vnum_zero_fail() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, VID, (5, 5), 10_000);
        let mut gifts = MoveDone {
            kind: world::character::MoveKind::Used,
            records: Vec::new(),
            changes: Vec::new(),
        };
        assert!(!world.land_group_row(Vid::new(VID), 99_999_999, 1, -1, false, &mut gifts));
        assert!(!world.land_group_row(Vid::new(VID), 0, 1, -1, false, &mut gifts));
    }

    /// The key's success: with a group the draw lands, the key and the box go,
    /// and the row tells its line.
    #[test]
    fn a_key_opens_its_matching_box() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        world
            .install_item_ids(world::item::ItemIdRange::new(1, 1_000_000, 1).unwrap())
            .expect("no ids yet");
        let held = vec![boxed(50_008, 60, 0), boxed(50_006, 61, 1)];
        let (_inbox, _) = enter_seeing_with(&mut world, PLACE, VID, (5, 5), 10_000, &held);
        let mut group = gamedata::special_item_group::SpecialItemGroup::default();
        group.add_item(19, 1, 100, 0);
        let groups = gamedata::special_item_group::SpecialItemGroups {
            special: [(50_006, group)].into_iter().collect(),
            ..Default::default()
        };
        world.item_groups = std::sync::Arc::new(groups);
        let key_at = ItemPos::new(EWindows::Inventory as u8, 0);
        let box_at = ItemPos::new(EWindows::Inventory as u8, 1);
        let moved = world
            .key_step(Vid::new(VID), key_at, box_at, mover())
            .expect("the draw lands");
        let character = world.characters.find_by_vid(Vid::new(VID)).expect("online");
        assert!(character.items().item_at(key_at).is_none(), "the key goes");
        assert!(character.items().item_at(box_at).is_none(), "the box goes");
        let sword = character
            .items()
            .item_at(ItemPos::new(EWindows::Inventory as u8, 2))
            .expect("the gift lands past both");
        assert_eq!((sword.vnum, sword.count), (19, 1));
        let texts: Vec<Vec<u8>> = moved
            .records
            .iter()
            .filter(|record| record.first() == Some(&4))
            .map(|record| record[10..].to_vec())
            .collect();
        assert!(
            texts.iter().any(|text| text.starts_with(b"[LS;444;")),
            "the pickup rides: {texts:?}"
        );
        assert!(
            texts.iter().any(|text| text.starts_with(b"[LS;35;")),
            "the row tells: {texts:?}"
        );
        assert!(!moved.changes.is_empty(), "the spend and the gift save");
    }

    /// Seed 0's shell draw lands in the water-stone band: it tells `[LS;221]`
    /// and lands the 27990 piece while one shell spends.
    #[test]
    fn a_low_roll_cracks_the_shell_into_a_water_stone() {
        use world::character::Pcg32;
        use world::item::ItemIdRange;
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        world.dice = Pcg32::new(0, 0);
        world
            .install_item_ids(ItemIdRange::new(1, 1_000_000, 100).unwrap())
            .unwrap();
        let held = vec![boxed(CLAM_SHELL, 60, 0)];
        let (mut inbox, _) = enter_seeing_with(&mut world, PLACE, VID, (5, 5), 10_000, &held);
        assert!(world.clam_shell_at(Vid::new(VID), box_at()));
        let moved = world
            .use_clam_shell(Vid::new(VID), box_at(), mover())
            .expect("the shell cracks");
        let told = inbox.try_recv().expect("the shell tells its line");
        assert_eq!(text_of(&told), b"[LS;221]");
        let texts: Vec<Vec<u8>> = moved
            .records
            .iter()
            .filter(|record| record.first() == Some(&4))
            .map(|record| record[10..].to_vec())
            .collect();
        assert!(
            texts.iter().any(|text| text.starts_with(b"[LS;35;")),
            "the gift names itself: {texts:?}"
        );
        let character = world.characters.find_by_vid(Vid::new(VID)).expect("online");
        assert!(
            character
                .items()
                .item_at(box_at())
                .is_none_or(|item| { item.vnum == SHELL_WATER_STONE && item.count == 1 }),
            "the water-stone piece lands where the shell stood"
        );
        assert!(!moved.changes.is_empty(), "the spend and the gift save");
    }

    /// Seed 1's shell draw lands in the dead-air band: it tells `[LS;232]` and
    /// gives nothing, while the shell still spends.
    #[test]
    fn a_dead_roll_cracks_the_shell_into_nothing() {
        use world::character::Pcg32;
        use world::item::ItemIdRange;
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        world.dice = Pcg32::new(1, 0);
        world
            .install_item_ids(ItemIdRange::new(1, 1_000_000, 100).unwrap())
            .unwrap();
        let held = vec![boxed(CLAM_SHELL, 60, 0)];
        let (mut inbox, _) = enter_seeing_with(&mut world, PLACE, VID, (5, 5), 10_000, &held);
        let moved = world
            .use_clam_shell(Vid::new(VID), box_at(), mover())
            .expect("the shell cracks");
        let told = inbox.try_recv().expect("the shell tells its line");
        assert_eq!(text_of(&told), b"[LS;232]");
        let texts: Vec<Vec<u8>> = moved
            .records
            .iter()
            .filter(|record| record.first() == Some(&4))
            .map(|record| record[10..].to_vec())
            .collect();
        assert!(texts.is_empty(), "no gift names itself: {texts:?}");
        let left = world
            .characters
            .find_by_vid(Vid::new(VID))
            .expect("online")
            .items()
            .item_at(box_at());
        assert!(left.is_none(), "the one shell spends");
    }

    /// Seed 22's shell draw lands past the dead-air ceiling: the white pearl
    /// lands with `[LS;243]`.
    #[test]
    fn a_high_roll_cracks_the_shell_into_a_white_pearl() {
        use world::character::Pcg32;
        use world::item::ItemIdRange;
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        world.dice = Pcg32::new(22, 0);
        world
            .install_item_ids(ItemIdRange::new(1, 1_000_000, 100).unwrap())
            .unwrap();
        let held = vec![boxed(CLAM_SHELL, 60, 0)];
        let (mut inbox, _) = enter_seeing_with(&mut world, PLACE, VID, (5, 5), 10_000, &held);
        let moved = world
            .use_clam_shell(Vid::new(VID), box_at(), mover())
            .expect("the shell cracks");
        let told = inbox.try_recv().expect("the shell tells its line");
        assert_eq!(text_of(&told), b"[LS;243]");
        let texts: Vec<Vec<u8>> = moved
            .records
            .iter()
            .filter(|record| record.first() == Some(&4))
            .map(|record| record[10..].to_vec())
            .collect();
        assert!(
            texts.iter().any(|text| text.starts_with(b"[LS;35;")),
            "the pearl names itself: {texts:?}"
        );
    }

    /// Experience tells two lines; the single-count gift names itself alone, and
    /// two already takes the plural form.
    #[test]
    fn the_lines_tell_two_for_experience_and_one_for_a_single_gift() {
        let clock = TestClock::default();
        let world = a_world(&clock);
        let to = mover().recipient(&world.locale);
        let mut records = Vec::new();
        world.push_group_row_lines(to, 2, 300, &mut records);
        assert_eq!(records.len(), 2);
        assert_eq!(text_of(&records[0]), b"[LS;1279]");
        assert_eq!(text_of(&records[1]), b"[LS;1290;300]");
        records.clear();
        world.push_group_row_lines(to, 30_053, 1, &mut records);
        assert_eq!(records.len(), 1);
        let line = text_of(&records[0]);
        assert!(line.starts_with(b"[LS;35;"), "{}", line.escape_ascii());
        assert!(line.ends_with(b"]"), "{}", line.escape_ascii());
        // The count boundary: two already takes the plural form.
        records.clear();
        world.push_group_row_lines(to, 30_053, 2, &mut records);
        assert_eq!(records.len(), 1);
        let line = text_of(&records[0]);
        assert!(line.starts_with(b"[LS;204;"), "{}", line.escape_ascii());
        assert!(line.ends_with(b";2]"), "{}", line.escape_ascii());
    }
}
