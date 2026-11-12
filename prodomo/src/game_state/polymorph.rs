//! `/polymorph` (`do_polymorph`, `G/cmd_gm.cpp:3049-3068`): the caller shows as the
//! named mob vnum, or unmorphs on an empty or job vnum. And the polymorph ball's use
//! (`ItemProcess_Polymorph`'s ball arm, `G/char_item.cpp:9749-9814`): the socket-0 vnum
//! morphs the caller through the timed affect, or refuses with its line, the ball kept.
//!
//! `SetPolymorph` (`G/char.cpp:7680-7724`) sets the race and the maintain flag
//! (`world::character::Points::set_polymorph`), stops the ride, reencodes with the spawn
//! state bit and `AFF_SPAWN`, runs the four attribute changes with amount 0, resets the
//! combo, and recomputes the battle points: the morphed arm (`G/char.cpp:2688-2707`)
//! while a morph is on, the plain arm when it clears. The combo has no port yet, so its
//! reset is a gap; the ride is never on, so stopping it is a no-op.
//!
//! The spawn state bit is held only for the reencode (`REMOVE_BIT`, `:7705`), but
//! `AFF_SPAWN` is never reset (D5), so every later insert and update of the player carries
//! it, as each later insert carries the shown race (`GetRaceNum`).

//!
//! The ball's use is a `MoveKind::Morphed` world step: the guards run the gates in
//! legacy's order — the morph, the ride, the socket-0 vnum against the mob table, and
//! the level bar — then the step consumes the ball, and [`GameState::land_polymorph_ball`]
//! lays the `POINT_POLYMORPH` row (whose change morphs through the point arm) and the
//! attack-bonus row, strips the two blade auras, and tells `USE_POLYMORPH_BALL` to the
//! log. The land takes the ball, socket and skill captured before the step, because the
//! step already spent the ball.
//!
//! The 50322 book's use is a `MoveKind::Booked` world step: the step changes nothing
//! (its count never falls), and [`GameState::land_polymorph_book`] runs
//! `CPolymorphUtils::PolymorphCharacter` (`G/polymorph.cpp:36-115`): the master's
//! duration, the chance from the levels, the practice in socket 2 and the skill, the
//! morph row and the typed bonus row, and socket 1's stamp down. `GiveBook` and
//! `BookUpgrade` stay unported behind their two `unused` quest names.
//!
//! Everything here runs on the game thread only (ADR-0002).

use common::point_slot as point;
use common::vid::Vid;
use world::character::{number, polymorph_power, PolyAttack, SKILL_POLYMORPH};

use super::view::EntityKey;
use super::GameState;
use crate::command::two_arguments;
use crate::game_state::skill_admin::str_to_dword;
use crate::game_state::whisper::str_to_int;
use crate::loading_phase::{
    AFFECT_POLYMORPH, AFF_GEOMGYEONG, AFF_GWIGUM, AFF_POLYMORPH, AFF_SPAWN,
};
use protocol::gc_chat::CHAT_TYPE_INFO;

/// The 50322 practice book's vnum (`POLYMORPH_BOOK_ID`, `G/polymorph.h:8`).
const POLYMORPH_BOOK: u32 = 50_322;

/// The reversal-of-pietrification scroll (`case USE_SPECIAL`'s 71092 arm,
/// `G/char_item.cpp:5217-5235`).
const REVERSAL_SCROLL: u32 = 71_092;

/// The vnums whose book morphs run (`m_mapSPDType`, `G/polymorph.cpp:11-14`): the dog
/// 101 and 1901. The attack and defence tables stay empty, so no other vnum wins a bonus.
const BOOK_SPD: [u32; 2] = [101, 1901];

/// `LC_TEXT("Transformation failed")` (`G/polymorph.cpp:72,79`): the key is in no locale
/// file (the main table and all 12 country files were swept for both book lines), so the
/// locale lookup answers the key itself.
const BOOK_FAILED: &[u8] = b"Transformation failed";

/// `LC_TEXT("I ended up training in disguise.")` (`:119`): the key is likewise absent,
/// so the lookup answers the key.
const BOOK_SPENT: &[u8] = b"I ended up training in disguise.";

impl GameState {
    /// `do_polymorph` (`G/cmd_gm.cpp:3049-3068`): two words (the vnum and the maintain
    /// flag); an empty line is silent.
    pub(super) fn run_polymorph(&mut self, vid: Vid, rest: &[u8]) {
        let (first, second) = two_arguments(rest);
        if first.is_empty() {
            return;
        }
        let vnum = str_to_dword(&first);
        let maintain = if second.is_empty() {
            false
        } else {
            str_to_int(&second) > 0
        };
        self.set_polymorph(vid, vnum, maintain);
    }

    /// `SetPolymorph` (`G/char.cpp:7680-7724`) on the player under `vid`: the race and
    /// the flag, the stopped ride, the spawn reencode, the four zero changes, and the
    /// battle compute. An unchanged vnum sends nothing.
    pub(super) fn set_polymorph(&mut self, vid: Vid, vnum: u32, maintain: bool) {
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        if !points.set_polymorph(vnum, maintain) {
            return;
        }
        // `StopRiding` (`:7697-7698`).
        self.stop_riding(vid);
        self.reencode_with_spawn(vid);
        if !maintain {
            for kind in [
                point::POINT_ST,
                point::POINT_DX,
                point::POINT_IQ,
                point::POINT_HT,
            ] {
                self.change_pc_point(vid, kind, 0, false);
            }
        }
        // `SetValidComboInterval(0)` and `SetComboSequence(0)` (`:7720-7721`): the combo
        // has no port yet, so its reset is a gap.
        let records = self.compute_for_morph(vid);
        self.send_point_records(vid, &records);
    }

    /// The battle compute `SetPolymorph` ends on: the morphed arm over the vnum's row
    /// while a morph is on (`G/char.cpp:2688-2707`), the plain arm when it clears.
    /// An unknown vnum zeroes the grades, as `CMobManager::Get` missing does.
    fn compute_for_morph(&mut self, vid: Vid) -> Vec<world::character::PointRecord> {
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return Vec::new();
        };
        if !points.is_polymorphed() {
            return points.compute_battle_points();
        }
        let vnum = points.polymorph_race();
        let row = self
            .regens
            .respawner()
            .and_then(|respawner| respawner.mob(vnum))
            .map(|mob| (mob.str, mob.con, mob.def));
        let power = self.polymorph_power(vid);
        let group = self
            .bodies
            .get(&vid)
            .map_or(0, |body| body.card.skill_group);
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return Vec::new();
        };
        points.compute_polymorph_battle_points(row, power, group)
    }

    /// The morphed attacker's row for `CalcMeleeDamage`'s and `CalcArrowDamage`'s
    /// polymorph arms: `Some` exactly while the player under `vid` is morphed without
    /// keeping its stats, with the power and the vnum's `(dex, damage_min,
    /// damage_max)`, or `None` for an unknown vnum, as `CMobManager::Get` missing.
    pub(super) fn poly_attack_of(&self, vid: Vid) -> Option<PolyAttack> {
        let points = self.characters.find_by_vid(vid).ok()?.points()?;
        if !points.is_polymorphed() || points.poly_maintain_stat() {
            return None;
        }
        let vnum = points.polymorph_race();
        let row = self
            .regens
            .respawner()
            .and_then(|respawner| respawner.mob(vnum))
            .map(|mob| (mob.dex, mob.damage_range[0], mob.damage_range[1]));
        Some(PolyAttack {
            power: self.polymorph_power(vid),
            row,
        })
    }

    /// The plain point [`GameState::poly_victim_dx`] falls back to: 0 with no
    /// points, as the rating's victim arm would read for a missing victim.
    pub(super) fn victim_plain_dx(&self, vid: Vid) -> i32 {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, |points| points.get_point(point::POINT_DX))
    }

    /// The victim's morphed dexterity for `CalcAttackRating`'s victim arm
    /// (`G/battle.cpp:281`): `GetPolymorphPoint(POINT_DX)` of the player under `vid`,
    /// `Some` exactly while morphed without keeping its stats and the row is known,
    /// else the plain point. A `None` row (an unknown vnum) reads the plain point,
    /// as the missed `CMobManager::Get` does (`G/char.cpp:3673-3701`).
    pub(super) fn poly_victim_dx(&self, vid: Vid) -> Option<i32> {
        let poly = self.poly_attack_of(vid)?;
        let dex = poly.row.map(|row| row.0)?;
        let points = self.characters.find_by_vid(vid).ok()?.points()?;
        Some(points.polymorph_point(Some(dex), poly.power))
    }

    /// `GetPolymorphPower` (`G/char.cpp:7669-7678`): the `poly` event flag on the test
    /// server when it is set, else the skill-129 climb through the power table.
    fn polymorph_power(&self, vid: Vid) -> i32 {
        if self.test_server {
            if let Some(quests) = &self.quests {
                let flag = quests.manager().event_flag(b"poly");
                if flag != 0 {
                    return flag;
                }
            }
        }
        polymorph_power(self.pc_skill_level(vid.raw(), SKILL_POLYMORPH))
    }

    /// The reencode `SetPolymorph` runs (`G/char.cpp:7700-7705`): `AFF_SPAWN` lands on the
    /// caller's base words for good, the spawn state bit is held, the view around the
    /// caller re-shows it (`ViewReencode`), and the bit is dropped again.
    fn reencode_with_spawn(&mut self, vid: Vid) {
        let Some(body) = self.bodies.get_mut(&vid) else {
            return;
        };
        let word = usize::try_from((AFF_SPAWN - 1) / 32).unwrap_or(0);
        body.base_affect_flags[word] |= 1 << ((AFF_SPAWN - 1) % 32);
        body.add_state.spawn = true;
        let mut effects = Vec::new();
        if let (Some(place), Some(spot)) = (self.place_of(vid.raw()), self.spot_of(vid.raw())) {
            if let Some(index) = self.maps.get_mut(&place) {
                // A same-spot `show` is `ViewReencode` (`G/entity_view.cpp:23-45`).
                let _shown = index.show(
                    EntityKey::Character(vid.raw()),
                    (spot.x, spot.y, spot.z),
                    self.view_radius,
                    &mut effects,
                );
            }
        }
        self.deliver(&effects, None);
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.add_state.spawn = false;
        }
    }

    /// The polymorph ball at `at` for the character online under `vid`, or `None` when the
    /// step is no ball use: no item, no proto, no `ITEM_POLYMORPH` kind, or a vnum outside
    /// the ball five. Answers the ball's vnum, its socket-0 vnum, and the caller's skill-129
    /// level.
    pub(super) fn ball_at(
        &self,
        vid: Vid,
        at: protocol::item_pos::ItemPos,
    ) -> Option<(u32, u32, u8)> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let item = character.items().item_at(at)?.clone();
        let proto = self.protos.get(item.vnum)?;
        if proto.item_type != gamedata::item_kind::ITEM_POLYMORPH {
            return None;
        }
        if !matches!(item.vnum, 70_104..=70_107 | 71_093) {
            return None;
        }
        let skill = self.pc_skill_level(vid.raw(), SKILL_POLYMORPH);
        let socket = u32::from_ne_bytes(item.sockets[0].to_ne_bytes());
        Some((item.vnum, socket, skill))
    }

    /// Whether the caller may use the ball (`ItemProcess_Polymorph`,
    /// `G/char_item.cpp:9749-9796`): morphed refuses `[LS;458]`, riding refuses
    /// `[LS;1053]`, a zero or unknown socket-0 vnum refuses `[LS;460]`, and a mob at or
    /// past the caller's level plus its bar refuses `[LS;461]` — each tells, each keeps
    /// the ball. Anything else passes.
    pub(super) fn ball_use_allowed(
        &mut self,
        vid: Vid,
        at: protocol::item_pos::ItemPos,
        mover: crate::item_move::Mover,
    ) -> bool {
        let Some((_ball, socket, _skill)) = self.ball_at(vid, at) else {
            return true;
        };
        let points = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points().cloned());
        let Some(points) = points else {
            return true;
        };
        let Some(mine) = self.own_mover(vid).map(|own| crate::item_move::Mover {
            recently_fought: mover.recently_fought,
            ..own
        }) else {
            return true;
        };
        if points.is_polymorphed() {
            self.tell(vid, mine, CHAT_TYPE_INFO, b"[LS;458]", &[]);
            return false;
        }
        if self.is_riding(vid.raw()) {
            self.tell(vid, mine, CHAT_TYPE_INFO, b"[LS;1053]", &[]);
            return false;
        }
        let row = (socket != 0)
            .then(|| {
                self.regens
                    .respawner()
                    .and_then(|respawner| respawner.mob(socket))
            })
            .flatten();
        let Some(row) = row else {
            self.tell(vid, mine, CHAT_TYPE_INFO, b"[LS;460]", &[]);
            return false;
        };
        // `MAX(0, 20 - GetLevel() * 3 / 10)` (`:9790`): the bar past the caller's level the
        // mob must sit under.
        let bar = (20 - i32::from(points.level()) * 3 / 10).max(0);
        if i32::from(row.level) >= i32::from(points.level()) + bar {
            self.tell(vid, mine, CHAT_TYPE_INFO, b"[LS;461]", &[]);
            return false;
        }
        true
    }

    /// Lands the ball's use (`G/char_item.cpp:9797-9814`): the timed `AFFECT_POLYMORPH`
    /// pair — `POINT_POLYMORPH` with the socket-0 vnum (its change morphs through the
    /// point arm), and `POINT_ATT_BONUS` with `(2 + skill / 40) * 100` — strips the two
    /// blade auras, and tells the use to the log. The duration is 5 minutes unskilled,
    /// else `(5 + (5 + skill / 40 * 25))` minutes (`:9800`).
    /// The 50322 book at `at` for the character online under `vid`, or `None` when the
    /// step is no book use. Answers sockets 0-2 and the caller's skill-129 level.
    pub(super) fn poly_book_at(
        &self,
        vid: Vid,
        at: protocol::item_pos::ItemPos,
    ) -> Option<(u32, u32, u32, u8)> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let item = character.items().item_at(at)?.clone();
        if item.vnum != POLYMORPH_BOOK {
            return None;
        }
        let proto = self.protos.get(item.vnum)?;
        if proto.item_type != gamedata::item_kind::ITEM_POLYMORPH {
            return None;
        }
        let sockets = [
            u32::from_ne_bytes(item.sockets[0].to_ne_bytes()),
            u32::from_ne_bytes(item.sockets[1].to_ne_bytes()),
            u32::from_ne_bytes(item.sockets[2].to_ne_bytes()),
        ];
        let skill = self.pc_skill_level(vid.raw(), SKILL_POLYMORPH);
        Some((sockets[0], sockets[1], sockets[2], skill))
    }

    /// Whether the item at `at` is the 71092 reversal scroll (`case USE_SPECIAL`'s
    /// 71092 arm, `G/char_item.cpp:5217-5235`).
    pub(super) fn reversal_at(&self, vid: Vid, at: protocol::item_pos::ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .is_some_and(|item| item.vnum == REVERSAL_SCROLL)
    }

    /// Uses the 71092 reversal scroll (`G/char_item.cpp:5217-5235`): with a target set
    /// the target's morph goes, else the caller's own; an unmorphed answer changes
    /// nothing. Each clear is `SetPolymorph(0)` then `RemoveAffect(AFFECT_POLYMORPH)`,
    /// in legacy's order — the scroll is kept (the arm neither counts nor removes)
    /// and nothing is said either way, so the answer is the bare used step.
    pub(super) fn use_reversal_item(
        &mut self,
        vid: Vid,
        mover: crate::item_move::Mover,
    ) -> Result<crate::item_move::MovedItems, crate::item_move::MoveItemRefused> {
        let target = self.targets.of(vid.raw());
        let cleared = target.unwrap_or(vid.raw());
        let morphed = self
            .characters
            .find_by_vid(Vid::new(cleared))
            .ok()
            .and_then(|character| character.points())
            .is_some_and(|points| points.polymorph_race() > 0);
        if morphed {
            self.set_polymorph(Vid::new(cleared), 0, false);
            self.remove_affects_of(Vid::new(cleared), AFFECT_POLYMORPH);
        }
        self.nothing_used(vid, mover)
    }

    /// Lands the book's use (`CPolymorphUtils::PolymorphCharacter`,
    /// `G/polymorph.cpp:36-115`, with the `UpdateBookPracticeGrade` tail, `:117-129`).
    /// The caller's skill mastery names the duration (10/15/20/25 ticks; anything else
    /// keeps the book silently), the chance is the caller's level minus the mob's plus
    /// socket 2 plus 29 plus the skill, and a miss tells the failed line; a hit lays
    /// the morph row (override) and, for the dog or 1901, the move-speed bonus row of
    /// skill plus socket 2 (`duration - 1`, no override), then stamps socket 1 down —
    /// or tells the spent line when it already reads 0. The log tells the use first,
    /// as the ball arm's does. `moved` gains socket 1's stamp change and update.
    pub(super) fn land_polymorph_book(
        &mut self,
        vid: Vid,
        at: protocol::item_pos::ItemPos,
        held: (u32, u32, u32),
        skill: u8,
        moved: &mut crate::item_move::MovedItems,
    ) {
        use world::character::set_item_socket;

        let (socket0, socket1, socket2) = held;

        let Some(mine) = self.own_mover(vid) else {
            return;
        };
        tracing::info!(socket0, "USE_POLYMORPH_BOOK");
        let duration = match crate::game_state::pc_skill::master_of(skill) {
            0 => 10,
            1 => 15,
            2 => 20,
            3 => 25,
            _ => return,
        };
        let level = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, |points| i32::from(points.level()));
        let mob = (socket0 != 0)
            .then(|| {
                self.regens
                    .respawner()
                    .and_then(|respawner| respawner.mob(socket0))
            })
            .flatten();
        let Some(mob) = mob else {
            return;
        };
        let practice = i32::try_from(socket2).unwrap_or(i32::MAX);
        let chance = level - i32::from(mob.level) + practice + (29 + i32::from(skill));
        let hit = chance > 0 && number(&mut self.dice, 1, 100) <= chance;
        if !hit {
            let line = self.locale.find(BOOK_FAILED, mine.language);
            self.tell(vid, mine, CHAT_TYPE_INFO, line, &[]);
            return;
        }
        let vnum = i32::try_from(socket0).unwrap_or(i32::MAX);
        self.add_pc_affect(
            vid,
            super::affect::Affect::buff(
                AFFECT_POLYMORPH,
                point::POINT_POLYMORPH,
                vnum,
                AFF_POLYMORPH,
                duration,
                0,
            ),
            true,
        );
        if BOOK_SPD.contains(&socket0) {
            let bonus = i32::from(skill).saturating_add(practice);
            self.add_pc_affect(
                vid,
                super::affect::Affect::buff(
                    AFFECT_POLYMORPH,
                    point::POINT_MOV_SPEED,
                    bonus,
                    AFF_POLYMORPH,
                    duration - 1,
                    0,
                ),
                false,
            );
        }
        // `UpdateBookPracticeGrade` (`:117-129`): socket 1 stamps down, or the spent
        // line tells when it already reads 0. The stamp's update and store change join
        // the step's, as `SetSocket` sends and stores.
        if socket1 > 0 {
            let Ok(character) = self.characters.find_by_vid_mut(vid) else {
                return;
            };
            let Some(item) = character.items().item_at(at).cloned() else {
                return;
            };
            let pid = character.player_id();
            let written = set_item_socket(
                character.items_mut(),
                item.id,
                (1, i32::try_from(socket1 - 1).unwrap_or(i32::MAX)),
            );
            let stamped = crate::item_move::MovedItems::new(
                pid,
                vid.raw(),
                written,
                mine,
                &self.protos,
                &self.locale,
            );
            moved.records.extend(stamped.records);
            moved.changes.extend(stamped.changes);
        } else {
            let line = self.locale.find(BOOK_SPENT, mine.language);
            self.tell(vid, mine, CHAT_TYPE_INFO, line, &[]);
        }
    }

    /// Lands the ball's use (`G/char_item.cpp:9797-9814`): the timed `AFFECT_POLYMORPH`
    /// pair — `POINT_POLYMORPH` with the socket-0 vnum (its change morphs through the
    /// point arm), and `POINT_ATT_BONUS` with `(2 + skill / 40) * 100` — strips the two
    /// blade auras, and tells the use to the log. The duration is 5 minutes unskilled,
    /// else `(5 + (5 + skill / 40 * 25))` minutes (`:9800`).
    pub(super) fn land_polymorph_ball(&mut self, vid: Vid, ball: u32, socket: u32, skill: u8) {
        let minutes: i64 = if skill == 0 {
            5
        } else {
            5 + (5 + i64::from(skill) / 40 * 25)
        };
        let seconds = i32::try_from(minutes.saturating_mul(60)).unwrap_or(i32::MAX);
        let bonus = (2 + i64::from(skill) / 40) * 100;
        let bonus = i32::try_from(bonus).unwrap_or(i32::MAX);
        let vnum = i32::try_from(socket).unwrap_or(i32::MAX);
        // The second add passes `false` (`:9807`): `FindAffect` would find the first row
        // and override it, so only `true` then `false` keeps both rows.
        for (on, value, over) in [
            (point::POINT_POLYMORPH, vnum, true),
            (point::POINT_ATT_BONUS, bonus, false),
        ] {
            self.add_pc_affect(
                vid,
                super::affect::Affect::buff(AFFECT_POLYMORPH, on, value, AFF_POLYMORPH, seconds, 0),
                over,
            );
        }
        // The blade auras (`:9809-9812`): each held flag strips its skill's row.
        for (flag, skill) in [(AFF_GEOMGYEONG, 4), (AFF_GWIGUM, 63)] {
            let word = usize::try_from((flag - 1) / 32).unwrap_or(0);
            if word < 2 && self.affect_flags_of(vid)[word] & (1 << ((flag - 1) % 32)) != 0 {
                let _stripped = self.remove_pc_affect(vid, skill);
            }
        }
        tracing::info!(ball, socket, seconds, "USE_POLYMORPH_BALL");
    }
}

#[cfg(test)]
mod tests {
    use protocol::gc_actors::GcCharacterAdd;
    use tokio::sync::mpsc::UnboundedReceiver;

    use super::super::fixtures::{enter, enter_seeing, open_cells, stand_field, TestClock, PLACE};
    use super::*;
    use crate::loading_phase::{ADD_CHARACTER_STATE_DEAD, ADD_CHARACTER_STATE_SPAWN};

    /// The caller's VID.
    const ME: u32 = 1;

    /// Where it stands.
    const SPOT: (i32, i32) = (3200, 3200);

    /// Near it: the witness sees every reencode.
    const NEAR: (i32, i32) = (3100, 3200);

    /// A dog's vnum: the race the witness reads.
    const DOG: u32 = 101;

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// A world with the caller at [`SPOT`] and a witness [`NEAR`], both drained.
    fn a_morph() -> (
        TestClock,
        GameState,
        UnboundedReceiver<Vec<u8>>,
        UnboundedReceiver<Vec<u8>>,
    ) {
        let clock = TestClock::default();
        clock.set(1_000);
        let mut world = stand_field(&clock, Vec::new(), open_cells(), true);
        let mut mine = enter(&mut world, ME, SPOT, 12_345);
        let mut witness = enter(&mut world, 2, NEAR, 12_345);
        drained(&mut mine);
        drained(&mut witness);
        (clock, world, mine, witness)
    }

    /// The caller's `GC_CHARACTER_ADD` among `heard`.
    fn alpha_add(heard: &[Vec<u8>]) -> GcCharacterAdd {
        heard
            .iter()
            .filter_map(|record| GcCharacterAdd::decode(record).ok())
            .find(|add| add.dw_vid == ME)
            .expect("the caller's own add")
    }

    /// `do_polymorph`'s own words: empty is silent, two words name the vnum and the
    /// maintain flag, and a non-positive second word does not maintain.
    #[test]
    fn the_line_is_silent_when_empty_and_names_the_vnum() {
        let (_clock, mut world, mut mine, mut witness) = a_morph();
        world.run_polymorph(Vid::new(ME), b"");
        assert!(drained(&mut mine).is_empty(), "an empty line is silent");
        assert!(
            drained(&mut witness).is_empty(),
            "the witness hears nothing"
        );
        assert!(!world.points_of(ME).expect("points").is_polymorphed());
        world.run_polymorph(Vid::new(ME), b"101 1");
        let points = world.points_of(ME).expect("points");
        assert!(points.is_polymorphed());
        assert_eq!(points.polymorph_race(), DOG);
        world.run_polymorph(Vid::new(ME), b"0");
        assert!(!world.points_of(ME).expect("points").is_polymorphed());
        world.run_polymorph(Vid::new(ME), b"101 0");
        let points = world.points_of(ME).expect("points");
        assert!(points.is_polymorphed());
        assert_eq!(points.polymorph_race(), DOG);
    }

    /// `SetPolymorph`'s vnum gate: a job vnum unmorphs, and an unchanged vnum sends
    /// nothing (`G/char.cpp:7683-7690`).
    #[test]
    fn a_job_vnum_unmorphs_and_an_unchanged_vnum_is_silent() {
        let (_clock, mut world, mut mine, mut witness) = a_morph();
        world.run_polymorph(Vid::new(ME), b"101");
        drained(&mut mine);
        drained(&mut witness);
        world.run_polymorph(Vid::new(ME), b"2");
        assert!(!world.points_of(ME).expect("points").is_polymorphed());
        assert!(!drained(&mut mine).is_empty(), "the clear reencodes");
        drained(&mut witness);
        world.run_polymorph(Vid::new(ME), b"3");
        assert!(drained(&mut mine).is_empty(), "still clear, so silent");
        assert!(
            drained(&mut witness).is_empty(),
            "the witness hears nothing"
        );
    }

    /// `poly_attack_of`: nothing while clear, nothing while maintaining, and the row
    /// with the power otherwise — or no row for an unknown vnum.
    #[test]
    fn the_attack_row_is_some_only_for_a_non_maintain_morph() {
        use gamedata::mob_group::MobGroups;
        use gamedata::mob_locale_names::MobLocaleNames;
        use gamedata::mob_proto::{MobProto, MobProtos};
        use gamedata::records::MobTableRecord;
        use std::sync::Arc;
        use world::npc::{NpcVids, Respawner, FIRST_NPC_VID};

        let (_clock, mut world, mut mine, mut witness) = a_morph();
        let row = MobProto {
            line: 2,
            table: MobTableRecord {
                vnum: DOG,
                dex: 6,
                damage_range: [24, 126],
                ..MobTableRecord::default()
            },
        };
        world.install_respawner(
            Respawner::new(
                Arc::new(MobProtos::from_rows(vec![row])),
                Arc::new(MobGroups::default()),
                Arc::new(MobLocaleNames::parse(b"VNUM\tNAME\n").unwrap()),
                NpcVids::starting_at(FIRST_NPC_VID + 100),
            ),
            open_cells(),
        );
        let vid = Vid::new(ME);
        assert!(world.poly_attack_of(vid).is_none(), "clear reads none");
        world.run_polymorph(vid, b"101");
        let poly = world.poly_attack_of(vid).expect("the morph reads its row");
        assert_eq!(poly.row, Some((6, 24, 126)));
        assert_eq!(poly.power, 0, "unskilled skill 129 reads the zeroed slot");
        // An unchanged vnum leaves the maintain flag alone (`:7689-7690`), so the
        // maintain morph clears first.
        world.run_polymorph(vid, b"0");
        world.run_polymorph(vid, b"101 1");
        assert!(
            world.poly_attack_of(vid).is_none(),
            "a maintain morph reads none"
        );
        world.run_polymorph(vid, b"404");
        let poly = world
            .poly_attack_of(vid)
            .expect("an unknown vnum still reads some");
        assert!(poly.row.is_none(), "but its row is none");
        drained(&mut mine);
        drained(&mut witness);
    }

    /// `poly_victim_dx`: nothing while clear or maintaining, the plain point for an
    /// unknown vnum, and the row's scaled dexterity plus the caller's own otherwise.
    #[test]
    fn the_victim_row_reads_the_morphed_dexterity() {
        use gamedata::mob_group::MobGroups;
        use gamedata::mob_locale_names::MobLocaleNames;
        use gamedata::mob_proto::{MobProto, MobProtos};
        use gamedata::records::MobTableRecord;
        use std::sync::Arc;
        use world::npc::{NpcVids, Respawner, FIRST_NPC_VID};

        let (_clock, mut world, mut mine, mut witness) = a_morph();
        let row = MobProto {
            line: 2,
            table: MobTableRecord {
                vnum: DOG,
                dex: 6,
                ..MobTableRecord::default()
            },
        };
        world.install_respawner(
            Respawner::new(
                Arc::new(MobProtos::from_rows(vec![row])),
                Arc::new(MobGroups::default()),
                Arc::new(MobLocaleNames::parse(b"VNUM\tNAME\n").unwrap()),
                NpcVids::starting_at(FIRST_NPC_VID + 100),
            ),
            open_cells(),
        );
        let vid = Vid::new(ME);
        assert!(world.poly_victim_dx(vid).is_none(), "clear reads none");
        world.run_polymorph(vid, b"101");
        // Unskilled skill 129 reads power 0, so the row adds nothing to the
        // fixture's own 3.
        assert_eq!(world.poly_victim_dx(vid), Some(3));
        world.run_polymorph(vid, b"0");
        world.run_polymorph(vid, b"101 1");
        assert!(
            world.poly_victim_dx(vid).is_none(),
            "a maintain morph reads none"
        );
        world.run_polymorph(vid, b"404");
        assert!(
            world.poly_victim_dx(vid).is_none(),
            "an unknown vnum reads none"
        );
        drained(&mut mine);
        drained(&mut witness);
    }

    /// The spawn reencode: the caller's own add carries the vnum's low 16 bits, the
    /// spawn state flag, and `AFF_SPAWN` on its affect words (`G/char.cpp:7700-7705`).
    #[test]
    fn the_spawn_add_carries_the_vnum_race_and_the_spawn_words() {
        let (_clock, mut world, _mine, mut witness) = a_morph();
        world.run_polymorph(Vid::new(ME), b"101");
        let heard = drained(&mut witness);
        let add = alpha_add(&heard);
        assert_eq!(add.w_race_num, u16::try_from(DOG).unwrap());
        assert_eq!(add.b_state_flag, ADD_CHARACTER_STATE_SPAWN);
        let slot = usize::try_from((AFF_SPAWN - 1) / 32).unwrap();
        assert_eq!(
            add.dw_affect_flag[slot] & (1 << ((AFF_SPAWN - 1) % 32)),
            1 << ((AFF_SPAWN - 1) % 32)
        );
    }

    /// The witness's own insert stays plain: a re-show that is not the caller carries
    /// no spawn state flag.
    #[test]
    fn a_re_shown_witness_carries_no_spawn_flag() {
        let (_clock, mut world, mut mine, _witness) = a_morph();
        world.run_polymorph(Vid::new(ME), b"101");
        let heard = drained(&mut mine);
        let other = heard
            .iter()
            .filter_map(|record| GcCharacterAdd::decode(record).ok())
            .find(|add| add.dw_vid == 2)
            .expect("the witness's re-show to the caller");
        assert_eq!(other.b_state_flag & ADD_CHARACTER_STATE_SPAWN, 0);
    }

    /// Whether `add` carries `AFF_SPAWN` on its affect words.
    fn has_spawn_affect(add: &GcCharacterAdd) -> bool {
        let slot = usize::try_from((AFF_SPAWN - 1) / 32).unwrap();
        add.dw_affect_flag[slot] & (1 << ((AFF_SPAWN - 1) % 32)) != 0
    }

    /// After the reencode, a late viewer's insert shows the morph's race with no spawn
    /// state bit (`REMOVE_BIT`, `G/char.cpp:7705`) and `AFF_SPAWN` still set, which nothing
    /// resets (D5): not even the clear, after which the late insert shows the job again.
    #[test]
    fn a_late_viewer_sees_the_race_and_the_kept_spawn_affect() {
        let (_clock, mut world, _mine, _witness) = a_morph();
        world.run_polymorph(Vid::new(ME), b"101");
        let (_late, seen) = enter_seeing(&mut world, PLACE, 3, (3300, 3200), 12_345);
        let add = alpha_add(&seen);
        assert_eq!(add.w_race_num, u16::try_from(DOG).unwrap());
        assert_eq!(add.b_state_flag, 0, "the spawn bit was dropped");
        assert!(has_spawn_affect(&add), "AFF_SPAWN stays");

        world.run_polymorph(Vid::new(ME), b"0");
        let (_later, seen) = enter_seeing(&mut world, PLACE, 4, (3200, 3300), 12_345);
        let add = alpha_add(&seen);
        let job = world.points_of(ME).expect("points").race();
        assert_eq!(add.w_race_num, u16::from(job), "the clear shows the job");
        assert!(has_spawn_affect(&add), "AFF_SPAWN outlives the clear");
    }

    /// The caller's mover: empire 1, at peace, with no flags.
    const fn mover() -> crate::item_move::Mover {
        crate::item_move::Mover {
            recently_fought: false,
            empire: 1,
            language: 1,
            pk_mode: crate::loading_phase::PK_MODE_PEACE,
            affect_flags: [0; 2],
            dead: false,
            killer_mode: false,
            alignment: 0,
            view_guild: (0, 0),
            premium: (0, 0),
        }
    }

    /// A world with a 70104 ball holding socket 0 `socket` in cell 0, and the dog (level
    /// `dog`) in the mob table, the caller entered and drained.
    fn a_ball(socket: u32, dog: u8) -> (GameState, UnboundedReceiver<Vec<u8>>, Vid) {
        use std::sync::Arc;

        use common::item_slots::EWindows;
        use gamedata::item_proto::{ItemProto, ItemProtos};
        use gamedata::mob_group::MobGroups;
        use gamedata::mob_locale_names::MobLocaleNames;
        use gamedata::mob_proto::{MobProto, MobProtos, CHAR_TYPE_MONSTER};
        use gamedata::records::MobTableRecord;
        use protocol::item_pos::ItemPos;
        use world::item::Item;
        use world::npc::{NpcVids, Respawner, FIRST_NPC_VID};

        use super::super::fixtures::{a_world, enter_seeing_with};

        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let proto = ItemProto::for_category_rule(70_104, gamedata::item_kind::ITEM_POLYMORPH, 0);
        world.protos = Arc::new(ItemProtos::from_rows(vec![proto]));
        let row = MobProto {
            line: 2,
            table: MobTableRecord {
                vnum: DOG,
                mob_type: CHAR_TYPE_MONSTER,
                level: dog,
                max_hp: 100,
                ..MobTableRecord::default()
            },
        };
        world.install_respawner(
            Respawner::new(
                Arc::new(MobProtos::from_rows(vec![row])),
                Arc::new(MobGroups::default()),
                Arc::new(MobLocaleNames::parse(b"VNUM\tNAME\n").unwrap()),
                NpcVids::starting_at(FIRST_NPC_VID + 100),
            ),
            open_cells(),
        );
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let mut ball = Item::new(7, 70_104);
        ball.sockets[0] = i32::try_from(socket).unwrap_or(i32::MAX);
        let (inbox, _) = enter_seeing_with(&mut world, PLACE, ME, SPOT, 12_345, &[(at, ball)]);
        let vid = Vid::new(ME);
        (world, inbox, vid)
    }

    /// The 71092 scroll's use clears the caller's own morph (`G/char_item.cpp:5228-5233`):
    /// the race clears, the timed `AFFECT_POLYMORPH` rows go with their remove packets,
    /// and the scroll stays in its cell — the arm consumes nothing and says nothing.
    #[test]
    fn a_used_scroll_clears_the_callers_own_morph_and_stays() {
        use common::item_slots::EWindows;
        use protocol::item_pos::ItemPos;

        let (mut world, mut mine, vid) = a_ball(DOG, 1);
        drained(&mut mine);
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let account = crate::game_loop_messages::QuestAccount::default();
        // The ball's own use morphs through the timed rows, as live, spending the
        // ball; the scroll is granted into the freed cell 0.
        let ball_moved = world
            .use_item(vid, at, mover(), &account)
            .expect("the ball lands");
        assert_eq!(ball_moved.kind, world::character::MoveKind::Morphed);
        drained(&mut mine);
        let mut scroll = world::item::Item::new(9, REVERSAL_SCROLL);
        {
            let (items, _) = world
                .characters
                .find_by_vid_mut(vid)
                .expect("caller")
                .items_and_points_mut();
            world::character::grant(items, &mut scroll, &[], 45).expect("a free cell");
        }
        let scroll_at = scroll.pos;
        let moved = world
            .use_item(vid, scroll_at, mover(), &account)
            .expect("the use lands");
        assert_eq!(moved.kind, world::character::MoveKind::Used);
        let points = world.points_of(ME).expect("points");
        assert!(!points.is_polymorphed());
        assert!(
            world
                .characters
                .find_by_vid(vid)
                .expect("caller")
                .items()
                .item_at(scroll_at)
                .is_some_and(|item| item.vnum == REVERSAL_SCROLL),
            "the scroll stays in its cell"
        );
        assert!(
            world
                .affect_events
                .get(&vid)
                .is_none_or(|event| event.affects.is_empty()),
            "the morph rows go"
        );
    }

    /// The 71092 scroll's use with a target set clears the TARGET's morph
    /// (`G/char_item.cpp:5220-5226`): the caller's own morph, when it has one, stays, and
    /// the target's timed rows go with it.
    #[test]
    fn a_used_scroll_clears_the_targets_morph_and_spares_the_caller() {
        use common::item_slots::EWindows;
        use protocol::item_pos::ItemPos;

        let (mut world, mut mine, vid) = a_ball(DOG, 1);
        drained(&mut mine);
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let account = crate::game_loop_messages::QuestAccount::default();
        let ball_moved = world
            .use_item(vid, at, mover(), &account)
            .expect("the ball lands");
        assert_eq!(ball_moved.kind, world::character::MoveKind::Morphed);
        drained(&mut mine);
        let mut scroll = world::item::Item::new(9, REVERSAL_SCROLL);
        {
            let (items, _) = world
                .characters
                .find_by_vid_mut(vid)
                .expect("caller")
                .items_and_points_mut();
            world::character::grant(items, &mut scroll, &[], 45).expect("a free cell");
        }
        let scroll_at = scroll.pos;
        // A morphed twin on the same map, and the caller's target on it.
        let (mut twin_inbox, _) = super::super::fixtures::enter_seeing_with(
            &mut world,
            PLACE,
            ME + 1,
            (3200, 3400),
            12_345,
            &[],
        );
        drained(&mut twin_inbox);
        world.set_polymorph(Vid::new(ME + 1), DOG, false);
        drained(&mut mine);
        drained(&mut twin_inbox);
        world.target(vid, ME + 1);
        drained(&mut mine);
        let moved = world
            .use_item(vid, scroll_at, mover(), &account)
            .expect("the use lands");
        assert_eq!(moved.kind, world::character::MoveKind::Used);
        assert!(
            world.points_of(ME).expect("caller").is_polymorphed(),
            "the caller's morph stays"
        );
        let twin = world.points_of(ME + 1).expect("twin");
        assert!(!twin.is_polymorphed(), "the target's morph goes");
    }

    /// The ball's use through the world step: the caller morphs into the socket-0 vnum
    /// with the spawn words, the ball leaves the inventory, and the two timed rows
    /// land (`POINT_POLYMORPH` with the vnum, `POINT_ATT_BONUS` with 200 unskilled).
    #[test]
    fn a_used_ball_morphs_and_spends_itself() {
        use common::item_slots::EWindows;
        use protocol::item_pos::ItemPos;

        let (mut world, mut mine, vid) = a_ball(DOG, 1);
        drained(&mut mine);
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let account = crate::game_loop_messages::QuestAccount::default();
        let step = mover();
        let moved = world
            .use_item(vid, at, step, &account)
            .expect("the use lands");
        assert_eq!(moved.kind, world::character::MoveKind::Morphed);
        let points = world.points_of(ME).expect("points");
        assert!(points.is_polymorphed());
        assert_eq!(points.polymorph_race(), DOG);
        assert!(
            world
                .characters
                .find_by_vid(vid)
                .expect("caller")
                .items()
                .item_at(at)
                .is_none(),
            "the spent ball leaves the cell"
        );
        let rows: Vec<(u32, usize, i32)> = world
            .affect_events
            .get(&vid)
            .map(|event| {
                event
                    .affects
                    .iter()
                    .map(|affect| (affect.kind, usize::from(affect.apply_on), affect.value))
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            rows.contains(&(AFFECT_POLYMORPH, point::POINT_POLYMORPH, 101)),
            "the morph row lands: {rows:?}"
        );
        assert!(
            rows.contains(&(AFFECT_POLYMORPH, point::POINT_ATT_BONUS, 200)),
            "the bonus row lands: {rows:?}"
        );
        let heard = drained(&mut mine);
        let add = alpha_add(&heard);
        assert_eq!(add.w_race_num, u16::try_from(DOG).unwrap());
        assert_eq!(add.b_state_flag, ADD_CHARACTER_STATE_SPAWN);
    }

    /// The ball's refusals, each with its line, the ball kept: morphed `[LS;458]`, a
    /// zero socket `[LS;460]`, an unknown socket `[LS;460]`, and a mob past the level
    /// bar `[LS;461]`.
    #[test]
    fn a_gated_ball_refuses_with_its_line_and_keeps_the_ball() {
        use common::item_slots::EWindows;
        use protocol::gc_chat::GcChat;
        use protocol::item_pos::ItemPos;

        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let account = crate::game_loop_messages::QuestAccount::default();
        let step = mover();
        for (socket, dog, morph_first, line) in [
            (DOG, 1, true, b"[LS;458]"),
            (0, 1, false, b"[LS;460]"),
            (7, 1, false, b"[LS;460]"),
            (DOG, 30, false, b"[LS;461]"),
        ] {
            let (mut world, mut mine, vid) = a_ball(socket, dog);
            if morph_first {
                world.set_polymorph(vid, DOG, false);
            }
            drained(&mut mine);
            let moved = world
                .use_item(vid, at, step, &account)
                .expect("a refusal answers");
            assert_eq!(moved.kind, world::character::MoveKind::Used);
            assert!(
                world
                    .characters
                    .find_by_vid(vid)
                    .expect("caller")
                    .items()
                    .item_at(at)
                    .is_some(),
                "the refused ball stays"
            );
            let heard = drained(&mut mine);
            assert!(
                heard
                    .iter()
                    .filter_map(|record| GcChat::decode(record).ok())
                    .any(|chat| chat.text == line),
                "the line tells: {heard:02x?}"
            );
            assert!(
                !world.points_of(ME).expect("points").is_polymorphed() || morph_first,
                "no morph lands"
            );
        }
    }

    /// A world with a 50322 book holding `sockets` in cell 0, the dog (level 1) in the
    /// mob table, and the caller's skill 129 at `skill`, entered and drained.
    fn a_book(sockets: [u32; 3], skill: u8) -> (GameState, UnboundedReceiver<Vec<u8>>, Vid) {
        use std::sync::Arc;

        use common::item_slots::EWindows;
        use gamedata::item_proto::{ItemProto, ItemProtos};
        use gamedata::mob_group::MobGroups;
        use gamedata::mob_locale_names::MobLocaleNames;
        use gamedata::mob_proto::{MobProto, MobProtos, CHAR_TYPE_MONSTER};
        use gamedata::records::MobTableRecord;
        use protocol::item_pos::ItemPos;
        use world::item::Item;
        use world::npc::{NpcVids, Respawner, FIRST_NPC_VID};

        use super::super::fixtures::{a_world, enter_seeing_with};

        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let proto =
            ItemProto::for_category_rule(POLYMORPH_BOOK, gamedata::item_kind::ITEM_POLYMORPH, 0);
        world.protos = Arc::new(ItemProtos::from_rows(vec![proto]));
        let row = MobProto {
            line: 2,
            table: MobTableRecord {
                vnum: DOG,
                mob_type: CHAR_TYPE_MONSTER,
                level: 1,
                max_hp: 100,
                ..MobTableRecord::default()
            },
        };
        world.install_respawner(
            Respawner::new(
                Arc::new(MobProtos::from_rows(vec![row])),
                Arc::new(MobGroups::default()),
                Arc::new(MobLocaleNames::parse(b"VNUM\tNAME\n").unwrap()),
                NpcVids::starting_at(FIRST_NPC_VID + 100),
            ),
            open_cells(),
        );
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let mut book = Item::new(7, POLYMORPH_BOOK);
        for (slot, value) in sockets.iter().enumerate() {
            book.sockets[slot] = i32::try_from(*value).unwrap_or(i32::MAX);
        }
        let (inbox, _) = enter_seeing_with(&mut world, PLACE, ME, SPOT, 12_345, &[(at, book)]);
        world.set_pc_skill_level(ME, SKILL_POLYMORPH, skill);
        let vid = Vid::new(ME);
        (world, inbox, vid)
    }

    /// The book's hit through the world step: the caller morphs into socket 0 with the
    /// spawn words, the book stays with socket 1 stamped down, and the two rows land
    /// (the morph row of 10 ticks, the dog's move-speed bonus of 9). Seed 0 draws at
    /// or under the chance (10 - 1 + 0 + 29 + 0 = 38).
    #[test]
    fn a_used_book_morphs_and_stamps_its_practice_down() {
        use common::item_slots::EWindows;
        use protocol::item_pos::ItemPos;
        use world::character::Pcg32;

        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let account = crate::game_loop_messages::QuestAccount::default();
        let step = mover();
        let (mut world, mut mine, vid) = a_book([DOG, 3, 0], 0);
        world.dice = Pcg32::new(0, 0);
        drained(&mut mine);
        let moved = world
            .use_item(vid, at, step, &account)
            .expect("the use lands");
        assert_eq!(moved.kind, world::character::MoveKind::Booked);
        assert!(
            world.points_of(ME).expect("points").is_polymorphed(),
            "seed 0 wins the chance"
        );
        let points = world.points_of(ME).expect("points");
        assert_eq!(points.polymorph_race(), DOG);
        let book = world
            .characters
            .find_by_vid(vid)
            .expect("caller")
            .items()
            .item_at(at)
            .cloned()
            .expect("the book stays");
        assert_eq!(book.sockets[1], 2, "socket 1 stamps down");
        let rows: Vec<(u32, usize, i32, i32)> = world
            .affect_events
            .get(&vid)
            .map(|event| {
                event
                    .affects
                    .iter()
                    .map(|affect| {
                        (
                            affect.kind,
                            usize::from(affect.apply_on),
                            affect.value,
                            affect.remaining,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            rows.contains(&(AFFECT_POLYMORPH, point::POINT_POLYMORPH, 101, 10)),
            "the morph row lands: {rows:?}"
        );
        assert!(
            rows.contains(&(AFFECT_POLYMORPH, point::POINT_MOV_SPEED, 0, 9)),
            "the dog's run row lands: {rows:?}"
        );
    }

    /// The book's miss through the world step: the failed line tells, no morph lands,
    /// and the book stays unstamped. Seed 1 draws past the chance
    /// (10 - 1 + 0 + 29 + 0 = 38).
    #[test]
    fn a_missed_book_tells_the_failed_line_and_stamps_nothing() {
        use common::item_slots::EWindows;
        use protocol::gc_chat::GcChat;
        use protocol::item_pos::ItemPos;
        use world::character::Pcg32;

        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let account = crate::game_loop_messages::QuestAccount::default();
        let step = mover();
        let (mut world, mut mine, vid) = a_book([DOG, 3, 0], 0);
        world.dice = Pcg32::new(1, 0);
        drained(&mut mine);
        let moved = world
            .use_item(vid, at, step, &account)
            .expect("the use lands");
        assert_eq!(moved.kind, world::character::MoveKind::Booked);
        assert!(
            !world.points_of(ME).expect("points").is_polymorphed(),
            "seed 1 misses the chance"
        );
        let heard = drained(&mut mine);
        assert!(
            heard
                .iter()
                .filter_map(|record| GcChat::decode(record).ok())
                .any(|chat| chat.text == BOOK_FAILED),
            "the failed line tells: {heard:02x?}"
        );
        let book = world
            .characters
            .find_by_vid(vid)
            .expect("caller")
            .items()
            .item_at(at)
            .cloned()
            .expect("the book stays");
        assert_eq!(book.sockets[1], 3, "a miss stamps nothing");
    }

    /// `/polymorph` runs at `POS_DEAD`: a dead caller's reencode carries the dead bit
    /// beside the spawn bit, as `m_bAddChrState` holds both.
    #[test]
    fn a_dead_callers_reencode_keeps_the_dead_bit() {
        let (_clock, mut world, _mine, mut witness) = a_morph();
        world.pc_combat.entry(ME).or_default().dead = true;
        world.run_polymorph(Vid::new(ME), b"101");
        let add = alpha_add(&drained(&mut witness));
        assert_eq!(
            add.b_state_flag,
            ADD_CHARACTER_STATE_DEAD | ADD_CHARACTER_STATE_SPAWN
        );
    }
}
