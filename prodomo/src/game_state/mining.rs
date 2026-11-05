//! Mining: `pc.mining` (`pc_mining`, `G/questlua_pc.cpp:1881-1887`, which runs
//! `CHARACTER::mining`, `G/char.cpp:5207-5245`), the mining event (`mining_event`,
//! `G/mining.cpp:355-408`) and its cancel (`mining_cancel`, `G/char.cpp:5197-5205`), which
//! every `OnMove` runs (`:6148-6179`).
//!
//! A call while the character digs cancels the dig with `[LS;815]`. Otherwise the NPC the
//! script runs for must stand on the character's map within `DISTANCE_APPROX` 1000 and be an
//! ore vein, or nothing happens; a weapon cell without a pick answers `[LS;816]`. The dig
//! draws `number(5, 15)` strokes, sends `GC_DIG_MOTION` around the character, and its event
//! fires 2 seconds a stroke later. The event looks for the pick again and for the vein, then
//! rolls `GetOrePct` (20, the mining skill's bonus and the pick's grade bonus): a hit drops
//! the vein's raw ore beside the character, owned by it for 15 seconds (`OreDrop`,
//! `:121-149`). Either way the pick trains (`PracticePick`, `:316-349`).
//!
//! # Not ported
//!
//! - The multi-farm block (`ENABLE_MULTI_FARM_BLOCK`, `G/mining.cpp:375-378`), whose
//!   registry is `sys.custom.multi_farm`: without it every character digs.
//! - `SendMoneyLog(MONEY_LOG_DROP, ...)`, the log database this build does not have.

use common::vid::Vid;
use db::items::RowChange;
use gamedata::item_kind::ITEM_PICK;
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_nested::GcDigMotion;
use protocol::item_pos::ItemPos;
use world::character::{number, WEAPON_CELL};
use world::item::Item;

use super::view::EntityKey;
use super::GameState;
use crate::chat_line::Arg;
use crate::item_move::{Mover, StoreOrder};
use crate::save::PASSES_PER_SEC;

/// `mining::info` (`G/mining.cpp:22-42`): each vein's vnum, its raw ore and the ore's
/// refined vnum.
const ORES: [(u32, u32, u32); 19] = [
    (20_047, 50_601, 50_621),
    (20_048, 50_602, 50_622),
    (20_049, 50_603, 50_623),
    (20_050, 50_604, 50_624),
    (20_051, 50_605, 50_625),
    (20_052, 50_606, 50_626),
    (20_053, 50_607, 50_627),
    (20_054, 50_608, 50_628),
    (20_055, 50_609, 50_629),
    (20_056, 50_610, 50_630),
    (20_057, 50_611, 50_631),
    (20_058, 50_612, 50_632),
    (20_059, 50_613, 50_633),
    (30_301, 50_614, 50_634),
    (30_302, 50_615, 50_635),
    (30_303, 50_616, 50_636),
    (30_304, 50_617, 50_637),
    (30_305, 50_618, 50_638),
    (30_306, 50_619, 50_639),
];

/// `fraction_info` (`G/mining.cpp:44-55`): the percent of each band and its count range.
const FRACTIONS: [(i32, i32, i32); 9] = [
    (20, 1, 10),
    (30, 11, 20),
    (20, 21, 30),
    (15, 31, 40),
    (5, 41, 50),
    (4, 51, 60),
    (3, 61, 70),
    (2, 71, 80),
    (1, 81, 90),
];

/// `PickGradeAddPct` (`G/mining.cpp:57-60`), by the pick's refine level.
const PICK_GRADE_ADD_PCT: [i32; 10] = [3, 5, 8, 11, 15, 20, 26, 32, 40, 50];

/// `SkillLevelAddPct` (`G/mining.cpp:62-77`), by the mining skill's level.
const SKILL_LEVEL_ADD_PCT: [i32; 41] = [
    0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8,
    8, 9, 9, 9, 9, 10, 10, 10, 11,
];

/// `SKILL_MINING` (`G/skill.h:145`).
const SKILL_MINING: usize = 124;
/// `GetOrePct`'s base percent (`G/mining.cpp:153`).
const DEFAULT_ORE_PCT: i32 = 20;
/// The seconds of one stroke (`PASSES_PER_SEC(2 * count)`, `G/mining.cpp:416`).
const STROKE_SECS: u64 = 2;
/// `SetOwnership(ch, 15)` of the dropped ore (`G/mining.cpp:146`).
const ORE_OWNERSHIP_SECS: i32 = 15;
/// The reach of the dig (`@fixme128`, `G/char.cpp:5218-5219`).
const DIG_REACH: i32 = 1000;

/// One character's mining event (`mining_event_info`, `G/mining.cpp:164-174`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MiningEvent {
    /// `vid_load`: the vein's VID.
    load: u32,
    /// The Pulse the event fires on.
    due: u64,
}

/// `GetRawOreFromLoad` (`G/mining.cpp:79-87`): the raw ore of the vein `load`, 0 for anything
/// else.
fn raw_ore_of(load: u32) -> u32 {
    ORES.iter()
        .find(|(vein, _, _)| *vein == load)
        .map_or(0, |(_, raw, _)| *raw)
}

/// `GetRefineFromRawOre` (`G/mining.cpp:89-97`): the refined vnum of the raw ore `raw`.
pub(super) fn refined_ore_of(raw: u32) -> Option<u32> {
    ORES.iter()
        .find(|(_, ore, _)| *ore == raw)
        .map(|(_, _, refined)| *refined)
}

/// `GetFractionCount` (`G/mining.cpp:99-113`): a band by `number(1, 100)`, then a count in
/// it.
fn fraction_count(dice: &mut dyn world::character::Dice) -> i32 {
    let mut r = number(dice, 1, 100);
    for (pct, low, high) in FRACTIONS {
        if r <= pct {
            return number(dice, low, high);
        }
        r -= pct;
    }
    0
}

/// The pick in the weapon cell: its id, its training socket and the proto values
/// `PracticePick` reads.
struct Pick {
    id: world::item::ItemId,
    trained: i32,
    values: [i32; 6],
    refined_vnum: u32,
    refine_level: i32,
}

impl GameState {
    /// `CHARACTER::mining` (`G/char.cpp:5207-5245`) of the character under `vid` on the NPC
    /// under `npc`.
    pub(super) fn mining(&mut self, vid: Vid, npc: u32, mover: Mover) {
        if self.mining_events.contains_key(&vid) {
            self.cancel_mining(vid);
            return;
        }
        let Some(&(channel, map, _)) = self.npc_of.get(&npc) else {
            return;
        };
        let Some(load) = self.npc_by_vid(npc) else {
            return;
        };
        let (load_x, load_y, race) = (load.x, load.y, u32::from(load.race));
        let Some(spot) = self.spot_of(vid.raw()) else {
            return;
        };
        let near = self.place_of(vid.raw()) == Some((channel, map))
            && crate::sync_position::distance_approx(spot.x - load_x, spot.y - load_y) <= DIG_REACH;
        if !near || raw_ore_of(race) == 0 {
            return;
        }
        if self.worn_pick(vid).is_none() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;816]", &[]);
            return;
        }
        let count = number(&mut self.dice, 5, 15);
        let mut motion = Vec::with_capacity(protocol::gc_nested::GC_DIG_MOTION_WIRE_SIZE);
        GcDigMotion {
            header: GcDigMotion::header(),
            vid: vid.raw(),
            target_vid: npc,
            count: u8::try_from(count).unwrap_or(5),
        }
        .encode_into(&mut motion);
        self.packet_around(EntityKey::Character(vid.raw()), &motion, None);
        let strokes = u64::try_from(count).unwrap_or(5);
        let due = self.last_pulse + STROKE_SECS * strokes * u64::from(PASSES_PER_SEC);
        let _dug = self
            .mining_events
            .insert(vid, MiningEvent { load: npc, due });
    }

    /// `mining_cancel` (`G/char.cpp:5197-5205`): a running dig ends with `[LS;815]`. Every
    /// `OnMove` runs it.
    pub(super) fn cancel_mining(&mut self, vid: Vid) {
        if self.mining_events.remove(&vid).is_none() {
            return;
        }
        if let Some(mover) = self.own_mover(vid) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;815]", &[]);
        }
    }

    /// The mining events due at `pulse` (`mining_event`, `G/mining.cpp:355-408`). The rows
    /// of a trained pick go to the descriptor, which writes them.
    pub(super) fn run_mining_events(&mut self, pulse: u64) {
        let due: Vec<(Vid, u32)> = self
            .mining_events
            .iter()
            .filter_map(|(vid, event)| (pulse >= event.due).then_some((*vid, event.load)))
            .collect();
        for (vid, load) in due {
            // `mining_take()`: the event ends before anything is judged.
            let _taken = self.mining_events.remove(&vid);
            let (Some(mover), Ok(character)) =
                (self.own_mover(vid), self.characters.find_by_vid(vid))
            else {
                continue;
            };
            let pid = character.player_id();
            let changes = self.dig(vid, pid, load, mover);
            if let Some((channel, map)) = self.place_of(vid.raw()) {
                let _ordered =
                    self.order_the_item_rows(vid, channel, map, changes, StoreOrder::default());
            }
        }
    }

    /// The event's judgement: the pick, the vein, the roll, the ore and the training.
    fn dig(&mut self, vid: Vid, pid: u32, load: u32, mover: Mover) -> Vec<RowChange> {
        let Some(pick) = self.worn_pick(vid) else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"You can't mine it because you don't have a pickaxe.",
                &[],
            );
            return Vec::new();
        };
        let Some(race) = self.npc_by_vid(load).map(|load| u32::from(load.race)) else {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Can't dig anymore.", &[]);
            return Vec::new();
        };
        if number(&mut self.dice, 1, 100) <= self.ore_pct(vid, &pick) {
            self.ore_drop(vid, pid, race);
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Mining was successful.", &[]);
        } else {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Mining failed.", &[]);
        }
        self.practice_pick(vid, pid, &pick, mover)
    }

    /// `GetOrePct` (`G/mining.cpp:151-162`): 20, the mining skill's bonus and the pick's
    /// grade bonus, each read clamped to its table.
    fn ore_pct(&self, vid: Vid, pick: &Pick) -> i32 {
        let skill = self
            .pc_skills
            .get(&vid.raw())
            .and_then(|state| state.levels.get(SKILL_MINING))
            .map_or(0, |&level| usize::from(level.min(40)));
        let grade = usize::try_from(pick.refine_level.clamp(0, 9)).unwrap_or(0);
        DEFAULT_ORE_PCT + SKILL_LEVEL_ADD_PCT[skill] + PICK_GRADE_ADD_PCT[grade]
    }

    /// `OreDrop` (`G/mining.cpp:121-149`): the raw ore of `load`, `GetFractionCount` of them
    /// (the first draw only proves a count exists; the item takes a second), on the ground
    /// within 200 of the character, owned by it for 15 seconds.
    fn ore_drop(&mut self, vid: Vid, pid: u32, load: u32) {
        let raw = raw_ore_of(load);
        if fraction_count(&mut self.dice) == 0 {
            return;
        }
        let count = fraction_count(&mut self.dice);
        let Some(item) = self.make_drop_hit(raw, count, -1) else {
            return;
        };
        let (Some((channel, map)), Some(spot)) =
            (self.place_of(vid.raw()), self.spot_of(vid.raw()))
        else {
            return;
        };
        let dx = number(&mut self.dice, -200, 200);
        let dy = number(&mut self.dice, -200, 200);
        let Some(ground) =
            self.lay_drop_hit(channel, map, (spot.x + dx, spot.y + dy, spot.z), item)
        else {
            return;
        };
        let Some(body) = self.bodies.get(&vid) else {
            return;
        };
        let name = crate::loading_phase::name_field(&body.card.name);
        self.set_ground_ownership(ground, Some((pid, name)), ORE_OWNERSHIP_SECS);
    }

    /// `PracticePick` (`G/mining.cpp:316-349`): a pick that can still be refined trains one
    /// step on a `number(1, value1)` of 1 and says so; one at its top only says so.
    fn practice_pick(&mut self, vid: Vid, pid: u32, pick: &Pick, mover: Mover) -> Vec<RowChange> {
        if pick.refined_vnum == 0 || number(&mut self.dice, 1, pick.values[1]) != 1 {
            return Vec::new();
        }
        let top = pick.values[2];
        if pick.trained >= top {
            self.tell_pick_topped(vid, mover);
            return Vec::new();
        }
        let trained = pick.trained.wrapping_add(1);
        let changes = self.write_held_socket(vid, pid, (pick.id, 0, trained), mover);
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"Pickaxe proficiency increased! (%d/%d)",
            &[Arg::Int(i64::from(trained)), Arg::Int(i64::from(top))],
        );
        if trained >= top {
            self.tell_pick_topped(vid, mover);
        }
        changes
    }

    /// The two lines of a pick at its training top (`G/mining.cpp:333-334`, `:341-342`).
    fn tell_pick_topped(&self, vid: Vid, mover: Mover) {
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"The pickaxe has reached its maximum training level.",
            &[],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"You can upgrade to the next level pickaxe through the woodcutter.",
            &[],
        );
    }

    /// The pick in the weapon cell of the character under `vid`, if one is there.
    fn worn_pick(&self, vid: Vid) -> Option<Pick> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let item: &Item = character.items().item_at(ItemPos::new(
            common::item_slots::EWindows::Inventory as u8,
            WEAPON_CELL,
        ))?;
        let proto = self.protos.get(item.vnum)?;
        (proto.item_type == ITEM_PICK).then(|| Pick {
            id: item.id,
            trained: item.sockets[0],
            values: proto.values,
            refined_vnum: proto.refined_vnum,
            refine_level: super::use_to_item::refine_level(proto),
        })
    }
}

#[cfg(test)]
mod tests {
    use world::character::FixedDraw;

    use super::{fraction_count, raw_ore_of, refined_ore_of};

    #[test]
    fn a_vein_yields_its_raw_ore_and_the_ore_refines_to_its_own() {
        assert_eq!(raw_ore_of(20_047), 50_601);
        assert_eq!(raw_ore_of(30_306), 50_619);
        assert_eq!(raw_ore_of(20_060), 0, "past the veins");
        assert_eq!(refined_ore_of(50_613), Some(50_633));
        assert_eq!(refined_ore_of(50_620), None);
    }

    /// The band draw walks the percents down: 20 is the first band's last, 21 the second's
    /// first, and 100 the last band's.
    #[test]
    fn the_fraction_band_is_the_one_the_percent_falls_in() {
        // `FixedDraw::new(d)` answers `d - 1` to every raw draw, so `number(1, 100)` is `d`.
        for (draw, band) in [(20, (1, 10)), (21, (11, 20)), (100, (81, 90))] {
            let mut dice = FixedDraw::new(draw);
            let count = fraction_count(&mut dice);
            assert!(
                (band.0..=band.1).contains(&count),
                "{draw}: {count} in {band:?}"
            );
        }
    }
}
