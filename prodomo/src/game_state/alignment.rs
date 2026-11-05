//! The alignment bands (`CHARACTER::UpdateAlignment`, `G/char_battle.cpp:3477-3565`): the
//! real alignment moves by an amount, clamped to ±200000, and the eight `ALIGNMENT_BONUS`
//! Legacy keeps `m_iAlignment` and `m_iRealAlignment` as two values; the shown one leaves the
//! real one only while the hide-alignment unique (70048) holds it at 0 through
//! `ShowAlignment(false)` (`G/char_battle.cpp:3457-3475`, `G/char_item.cpp:8287-8288,8556-8557`,
//! `G/item.cpp:2177-2178,2236-2237`). No ported path grants that unique, so this build keeps one
//! value and the client-visible one always follows it.
//!
//! # What legacy does
//!
//! `UpdateAlignment(iAmount)` takes the /10 the shown value last had, clamps the real value,
//! and — since shown and real agree — copies it to the shown one and sends `UpdatePacket`
//! when the /10 moved. Then it tests
//! the eight bonus affects in its own order — 570, 573, 572, 571, 574, 575, 576, 577 — and
//! removes each one the new value no longer keeps. Finally the band that holds the value
//! adds its two affects with `bOverride`: band 1 (10000-39999) gives 570
//! `POINT_ATTBONUS_HUMAN` +3 and 572 `POINT_MAX_HP` +500, band 2 (40000-79999) gives 571 +6
//! and 573 +1000, band 3 (80000-119999) gives 574 +9 and 575 +1500, and band 4 (120000 and
//! up) gives 576 +12 and 577 +2000, each for `ALIGNMENT_TIME` (one year in ticks).
//!
//! The removal conditions are the exact equalities: 570 and 572 stay only while the real
//! alignment is exactly 10000, 571 and 573 only at 40000, 574 and 575 only at 80000, and
//! 576 and 577 while it is at least 120000. Any other value in a band therefore removes and
//! re-adds its two affects on every update.
//!
//! # What this build does
//!
//! The rule is shared with the quest session, which changes the alignment while a script
//! runs: [`alignment_bonuses`], [`stale_alignment_bonus`] and [`clamped_alignment`] are
//! pure, and the session applies the same order to its borrowed affects and points.
//!
//! Everything here runs on the game thread only (ADR-0002).

use common::point_slot as point;
use common::vid::Vid;

use super::affect::{affect_remove_frame, take_affects_of_kind, Affect};
use super::GameState;

/// `ALIGNMENT_CLAMP` (`G/char_battle.cpp:3484`): both ends of the stored alignment.
pub(super) const ALIGNMENT_CLAMP: i32 = 200_000;
/// `NAME_ALIGNMENT_1` (`common/length.h:1068`): the first band's lower bound.
pub(super) const NAME_ALIGNMENT_1: i32 = 10_000;
/// `NAME_ALIGNMENT_2` (`common/length.h:1069`): the second band's lower bound.
pub(super) const NAME_ALIGNMENT_2: i32 = 40_000;
/// `NAME_ALIGNMENT_3` (`common/length.h:1070`): the third band's lower bound.
pub(super) const NAME_ALIGNMENT_3: i32 = 80_000;
/// `NAME_ALIGNMENT_4` (`common/length.h:1071`): the fourth band's lower bound.
pub(super) const NAME_ALIGNMENT_4: i32 = 120_000;
/// `ALIGNMENT_TIME` (`common/length.h:1073`): one year in ticks, the band affects' duration.
pub(super) const ALIGNMENT_TIME: i32 = 60 * 60 * 24 * 365;
/// `ALIGNMENT_BONUS_1` (`G/affect.h:108`): band 1's human-attack bonus.
pub(super) const ALIGNMENT_BONUS_1: u32 = 570;
/// `ALIGNMENT_BONUS_2` (`G/affect.h:109`): band 2's human-attack bonus.
pub(super) const ALIGNMENT_BONUS_2: u32 = 571;
/// `ALIGNMENT_BONUS_3` (`G/affect.h:110`): band 1's max-HP bonus.
pub(super) const ALIGNMENT_BONUS_3: u32 = 572;
/// `ALIGNMENT_BONUS_4` (`G/affect.h:111`): band 2's max-HP bonus.
pub(super) const ALIGNMENT_BONUS_4: u32 = 573;
/// `ALIGNMENT_BONUS_5` (`G/affect.h:112`): band 3's human-attack bonus.
pub(super) const ALIGNMENT_BONUS_5: u32 = 574;
/// `ALIGNMENT_BONUS_6` (`G/affect.h:113`): band 3's max-HP bonus.
pub(super) const ALIGNMENT_BONUS_6: u32 = 575;
/// `ALIGNMENT_BONUS_7` (`G/affect.h:114`): band 4's human-attack bonus.
pub(super) const ALIGNMENT_BONUS_7: u32 = 576;
/// `ALIGNMENT_BONUS_8` (`G/affect.h:115`): band 4's max-HP bonus.
pub(super) const ALIGNMENT_BONUS_8: u32 = 577;

/// The order `UpdateAlignment` tests the bonus affects in (`G/char_battle.cpp:3495-3541`):
/// 570, 573, 572, 571, 574, 575, 576, 577.
pub(super) const ALIGNMENT_REMOVAL_ORDER: [u32; 8] = [
    ALIGNMENT_BONUS_1,
    ALIGNMENT_BONUS_4,
    ALIGNMENT_BONUS_3,
    ALIGNMENT_BONUS_2,
    ALIGNMENT_BONUS_5,
    ALIGNMENT_BONUS_6,
    ALIGNMENT_BONUS_7,
    ALIGNMENT_BONUS_8,
];

/// `MINMAX(-200000, m_iRealAlignment + iAmount, 200000)` in `i64`, so an amount that would
/// overflow an `int` in legacy clamps instead (the x86 result there is undefined).
pub(super) fn clamped_alignment(alignment: i32, amount: i32) -> i32 {
    let value = i64::from(alignment) + i64::from(amount);
    let low = -i64::from(ALIGNMENT_CLAMP);
    let high = i64::from(ALIGNMENT_CLAMP);
    if value < low {
        -ALIGNMENT_CLAMP
    } else if value > high {
        ALIGNMENT_CLAMP
    } else {
        i32::try_from(value).unwrap_or_else(|_| unreachable!("{value} is within ±200000"))
    }
}

/// Whether the bonus affect `kind` no longer belongs at `alignment`, in the exact
/// conditions `UpdateAlignment` uses (`G/char_battle.cpp:3495-3541`).
pub(super) fn stale_alignment_bonus(kind: u32, alignment: i32) -> bool {
    match kind {
        ALIGNMENT_BONUS_1 | ALIGNMENT_BONUS_3 => alignment != NAME_ALIGNMENT_1,
        ALIGNMENT_BONUS_2 | ALIGNMENT_BONUS_4 => alignment != NAME_ALIGNMENT_2,
        ALIGNMENT_BONUS_5 | ALIGNMENT_BONUS_6 => alignment != NAME_ALIGNMENT_3,
        ALIGNMENT_BONUS_7 | ALIGNMENT_BONUS_8 => alignment < NAME_ALIGNMENT_4,
        _ => false,
    }
}

/// The band's two affects, in legacy's add order: the human-attack bonus then the max-HP
/// bonus, each as `(kind, point slot, value)` (`G/char_battle.cpp:3543-3565`).
pub(super) fn alignment_bonuses(alignment: i32) -> Option<[(u32, usize, i32); 2]> {
    if (NAME_ALIGNMENT_1..NAME_ALIGNMENT_2).contains(&alignment) {
        Some([
            (ALIGNMENT_BONUS_1, point::POINT_ATTBONUS_HUMAN, 3),
            (ALIGNMENT_BONUS_3, point::POINT_MAX_HP, 500),
        ])
    } else if (NAME_ALIGNMENT_2..NAME_ALIGNMENT_3).contains(&alignment) {
        Some([
            (ALIGNMENT_BONUS_2, point::POINT_ATTBONUS_HUMAN, 6),
            (ALIGNMENT_BONUS_4, point::POINT_MAX_HP, 1000),
        ])
    } else if (NAME_ALIGNMENT_3..NAME_ALIGNMENT_4).contains(&alignment) {
        Some([
            (ALIGNMENT_BONUS_5, point::POINT_ATTBONUS_HUMAN, 9),
            (ALIGNMENT_BONUS_6, point::POINT_MAX_HP, 1500),
        ])
    } else if alignment >= NAME_ALIGNMENT_4 {
        Some([
            (ALIGNMENT_BONUS_7, point::POINT_ATTBONUS_HUMAN, 12),
            (ALIGNMENT_BONUS_8, point::POINT_MAX_HP, 2000),
        ])
    } else {
        None
    }
}

/// One band affect as `AddAffect` builds it: the point, the value, no flag, and the year
/// long `ALIGNMENT_TIME`.
pub(super) fn alignment_bonus(kind: u32, slot: usize, value: i32) -> Affect {
    Affect {
        kind,
        apply_on: u8::try_from(slot).unwrap_or(u8::MAX),
        value,
        flag: 0,
        remaining: ALIGNMENT_TIME,
        sp_cost: 0,
    }
}

impl GameState {
    /// `CHARACTER::UpdateAlignment` (`G/char_battle.cpp:3477-3565`) on the player under
    /// `vid`: clamp the real alignment, send the update when its /10 moved (before the
    /// band changes, with the flags the character still holds), remove each bonus the new
    /// value no longer keeps in legacy's order, then add the band's two affects.
    pub(super) fn update_alignment(&mut self, vid: Vid, amount: i32) {
        let Some(body) = self.bodies.get(&vid) else {
            return;
        };
        let before = body.card.alignment / 10;
        let after = clamped_alignment(body.card.alignment, amount);
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.card.alignment = after;
        }
        if before != after / 10 {
            let flags = self.affect_flags_of(vid);
            self.send_affect_update(vid, flags);
        }
        for kind in ALIGNMENT_REMOVAL_ORDER {
            let held = self
                .affect_events
                .get(&vid)
                .is_some_and(|event| event.affects.iter().any(|affect| affect.kind == kind));
            if held && stale_alignment_bonus(kind, after) {
                self.remove_alignment_bonus(vid, kind);
            }
        }
        if let Some(bonuses) = alignment_bonuses(after) {
            for (kind, slot, value) in bonuses {
                self.add_pc_affect(vid, alignment_bonus(kind, slot, value), true);
            }
        }
    }

    /// `CHARACTER::RemoveAffect` for one alignment bonus: the point it added goes back and
    /// its remove packet goes to the character. Legacy recomputes every point here; the
    /// Rewrite changes the one slot, as it does for every other affect, and sends no
    /// `GC_CHARACTER_UPDATE` (a bonus affect has no flag).
    fn remove_alignment_bonus(&mut self, vid: Vid, kind: u32) {
        let removed = match self.affect_events.get_mut(&vid) {
            Some(event) => take_affects_of_kind(&mut event.affects, kind),
            None => Vec::new(),
        };
        for affect in &removed {
            self.change_pc_point(vid, usize::from(affect.apply_on), -affect.value, false);
            let frame = affect_remove_frame(affect);
            let _retired = self.write_to_client(vid, frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        alignment_bonuses, clamped_alignment, stale_alignment_bonus, ALIGNMENT_BONUS_1,
        ALIGNMENT_BONUS_2, ALIGNMENT_BONUS_3, ALIGNMENT_BONUS_4, ALIGNMENT_BONUS_5,
        ALIGNMENT_BONUS_6, ALIGNMENT_BONUS_7, ALIGNMENT_BONUS_8, ALIGNMENT_CLAMP, ALIGNMENT_TIME,
        NAME_ALIGNMENT_1, NAME_ALIGNMENT_2, NAME_ALIGNMENT_3, NAME_ALIGNMENT_4,
    };
    use crate::game_state::fixtures::{a_world, enter, TestClock};
    use crate::game_state::GameState;
    use common::point_slot as point;
    use common::vid::Vid;
    use protocol::gc_actors::GcCharacterUpdate;

    /// The four bands, their edges, and no band below 10000.
    #[test]
    fn the_bands_are_the_four_legacy_windows() {
        assert_eq!(alignment_bonuses(-200_000), None);
        assert_eq!(alignment_bonuses(9_999), None);
        assert_eq!(
            alignment_bonuses(NAME_ALIGNMENT_1),
            Some([
                (ALIGNMENT_BONUS_1, point::POINT_ATTBONUS_HUMAN, 3),
                (ALIGNMENT_BONUS_3, point::POINT_MAX_HP, 500),
            ])
        );
        assert_eq!(
            alignment_bonuses(NAME_ALIGNMENT_2 - 1),
            alignment_bonuses(NAME_ALIGNMENT_1)
        );
        assert_eq!(
            alignment_bonuses(NAME_ALIGNMENT_2),
            Some([
                (ALIGNMENT_BONUS_2, point::POINT_ATTBONUS_HUMAN, 6),
                (ALIGNMENT_BONUS_4, point::POINT_MAX_HP, 1000),
            ])
        );
        assert_eq!(
            alignment_bonuses(NAME_ALIGNMENT_3),
            Some([
                (ALIGNMENT_BONUS_5, point::POINT_ATTBONUS_HUMAN, 9),
                (ALIGNMENT_BONUS_6, point::POINT_MAX_HP, 1500),
            ])
        );
        assert_eq!(
            alignment_bonuses(NAME_ALIGNMENT_4),
            Some([
                (ALIGNMENT_BONUS_7, point::POINT_ATTBONUS_HUMAN, 12),
                (ALIGNMENT_BONUS_8, point::POINT_MAX_HP, 2000),
            ])
        );
        assert_eq!(
            alignment_bonuses(ALIGNMENT_CLAMP),
            alignment_bonuses(NAME_ALIGNMENT_4)
        );
    }

    /// The exact equalities of `UpdateAlignment`'s eight remove tests.
    #[test]
    fn each_bonus_stays_only_at_the_value_legacy_names() {
        assert!(stale_alignment_bonus(ALIGNMENT_BONUS_1, 9_999));
        assert!(!stale_alignment_bonus(ALIGNMENT_BONUS_1, 10_000));
        assert!(stale_alignment_bonus(ALIGNMENT_BONUS_1, 10_001));
        assert!(!stale_alignment_bonus(ALIGNMENT_BONUS_2, 40_000));
        assert!(!stale_alignment_bonus(ALIGNMENT_BONUS_3, 10_000));
        assert!(!stale_alignment_bonus(ALIGNMENT_BONUS_4, 40_000));
        assert!(!stale_alignment_bonus(ALIGNMENT_BONUS_5, 80_000));
        assert!(!stale_alignment_bonus(ALIGNMENT_BONUS_6, 80_000));
        assert!(!stale_alignment_bonus(ALIGNMENT_BONUS_7, 120_000));
        assert!(!stale_alignment_bonus(ALIGNMENT_BONUS_8, 200_000));
        assert!(stale_alignment_bonus(ALIGNMENT_BONUS_7, 119_999));
        assert!(stale_alignment_bonus(ALIGNMENT_BONUS_8, 119_999));
        assert!(
            !stale_alignment_bonus(999, 0),
            "an unknown kind is not ours"
        );
    }

    /// The clamp holds both ends, lands exactly on them, and still holds an amount that
    /// would overflow an `int`.
    #[test]
    fn the_clamp_holds_the_alignment_at_two_hundred_thousand() {
        assert_eq!(clamped_alignment(0, 1), 1);
        assert_eq!(clamped_alignment(199_999, 1), ALIGNMENT_CLAMP);
        assert_eq!(clamped_alignment(199_999, 2), ALIGNMENT_CLAMP);
        assert_eq!(clamped_alignment(199_999, 7), ALIGNMENT_CLAMP);
        assert_eq!(clamped_alignment(-199_999, -1), -ALIGNMENT_CLAMP);
        assert_eq!(clamped_alignment(-199_999, -2), -ALIGNMENT_CLAMP);
        assert_eq!(clamped_alignment(-199_999, -7), -ALIGNMENT_CLAMP);
        assert_eq!(clamped_alignment(0, i32::MAX), ALIGNMENT_CLAMP);
        assert_eq!(clamped_alignment(0, i32::MIN), -ALIGNMENT_CLAMP);
    }

    /// A crossing sends the update with the new /10 before the band's affects, then the
    /// two add packets; a move to another band takes the old pair back and adds the new.
    #[test]
    fn a_crossing_sends_the_update_before_the_band_changes() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, 1, (5, 5), 10_000);
        let vid = Vid::new(1);
        let att_before = points_of(&world, vid, point::POINT_ATTBONUS_HUMAN);
        let hp_before = points_of(&world, vid, point::POINT_MAX_HP);
        if let Some(body) = world.bodies.get_mut(&vid) {
            body.card.alignment = 9_995;
        }
        while inbox.try_recv().is_ok() {}

        world.update_alignment(vid, 10);
        assert_eq!(world.bodies[&vid].card.alignment, 10_005);
        assert_eq!(
            points_of(&world, vid, point::POINT_ATTBONUS_HUMAN),
            att_before + 3
        );
        assert_eq!(points_of(&world, vid, point::POINT_MAX_HP), hp_before + 500);
        assert_eq!(
            affect_kinds(&world, vid),
            [ALIGNMENT_BONUS_1, ALIGNMENT_BONUS_3]
        );
        let heard: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        let update_at = update_position(&heard, 1_000);
        let update = GcCharacterUpdate::decode(&heard[update_at]).expect("a character update");
        assert_eq!(update.s_alignment, 1_000);
        let adds: Vec<usize> = heard
            .iter()
            .enumerate()
            .filter(|(_, frame)| frame.first() == Some(&0x7e))
            .map(|(at, _)| at)
            .collect();
        assert_eq!(adds.len(), 2);
        assert!(adds.iter().all(|at| *at > update_at));
        let added: Vec<u32> = adds
            .iter()
            .map(|at| u32::from_le_bytes(heard[*at][1..5].try_into().expect("four bytes")))
            .collect();
        assert_eq!(added, [ALIGNMENT_BONUS_1, ALIGNMENT_BONUS_3]);
        for at in &adds {
            let duration = i32::from_le_bytes(heard[*at][14..18].try_into().expect("four bytes"));
            assert_eq!(duration, ALIGNMENT_TIME, "the band's year-long duration");
        }

        // 40000 is band 2: the pair 570/572 no longer stays at 10000, so it goes before
        // 571/573 arrive.
        while inbox.try_recv().is_ok() {}
        if let Some(body) = world.bodies.get_mut(&vid) {
            body.card.alignment = 40_000;
        }
        world.update_alignment(vid, 0);
        assert_eq!(
            affect_kinds(&world, vid),
            [ALIGNMENT_BONUS_2, ALIGNMENT_BONUS_4]
        );
        assert_eq!(
            points_of(&world, vid, point::POINT_ATTBONUS_HUMAN),
            att_before + 6
        );
        assert_eq!(
            points_of(&world, vid, point::POINT_MAX_HP),
            hp_before + 1000
        );
        let heard: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        let frames: Vec<(u8, u32)> = heard
            .iter()
            .filter(|frame| matches!(frame.first(), Some(0x7e | 0x7f)))
            .map(|frame| {
                let kind = u32::from_le_bytes(frame[1..5].try_into().expect("four bytes"));
                (frame[0], kind)
            })
            .collect();
        assert_eq!(
            frames,
            [
                (0x7f, ALIGNMENT_BONUS_1),
                (0x7f, ALIGNMENT_BONUS_3),
                (0x7e, ALIGNMENT_BONUS_2),
                (0x7e, ALIGNMENT_BONUS_4),
            ]
        );
    }

    /// The kinds of the affects `vid` holds, in the event's order.
    fn affect_kinds(world: &GameState, vid: Vid) -> Vec<u32> {
        world
            .affect_events
            .get(&vid)
            .map_or_else(Vec::new, |event| {
                event.affects.iter().map(|affect| affect.kind).collect()
            })
    }

    /// The quest session's four GRAND names on the live world: `pc.get_real_alignment` reads
    /// the value an earlier `pc.change_alignment` in the same step left, the step's amounts are
    /// applied after it exactly as [`GameState::update_alignment`] applies them, and the two
    /// skill-book calls read and remove the affect the world holds.
    #[test]
    fn the_session_alignment_settles_after_the_step_and_the_book_affect_is_removed() {
        use crate::game_state::GroundPlace;
        use protocol::gc_vid::HEADER_GC_AFFECT_REMOVE;

        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, 1, (5, 5), 10_000);
        let vid = Vid::new(1);
        world.start_a_quest(vid);
        let pid = world
            .characters()
            .find_by_vid(vid)
            .expect("the entrant is in the world")
            .player_id();
        let place = GroundPlace {
            channel: 1,
            map: 1,
            x: 5,
            y: 5,
        };
        let mover = world.own_mover(vid).expect("the entrant has a body");
        while inbox.try_recv().is_ok() {}

        let read = world
            .run_quests(place, mover, 10, pid, |_manager, game| {
                let before = game.real_alignment();
                game.change_alignment(12_000);
                let after = game.real_alignment();
                game.change_alignment(-250);
                (before, after, game.real_alignment())
            })
            .expect("the quests are loaded");
        assert_eq!(
            read.0,
            (0, 12_000, 11_750),
            "the script reads its own changes"
        );
        assert_eq!(read.1 .4, [12_000, -250], "both amounts are answered");
        assert_eq!(
            world.bodies[&vid].card.alignment, 11_750,
            "the step settled"
        );

        // The crossing update comes before the band's affects, and the second amount
        // leaves band 1 at the exact-equality test and re-enters it, so legacy removes
        // and re-adds its pair.
        let heard: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        let first_update = update_position(&heard, 1_200);
        let second_update = update_position(&heard, 1_175);
        let adds: Vec<usize> = heard
            .iter()
            .enumerate()
            .filter(|(_, frame)| frame.first() == Some(&0x7e))
            .map(|(at, _)| at)
            .collect();
        assert_eq!(adds.len(), 4, "band 1's pair, twice");
        assert!(adds[..2]
            .iter()
            .all(|at| (first_update + 1..second_update).contains(at)));
        assert!(adds[2..].iter().all(|at| *at > second_update));
        assert_eq!(
            affect_kinds(&world, vid),
            [ALIGNMENT_BONUS_1, ALIGNMENT_BONUS_3]
        );

        // `pc.is_skill_book_no_delay` reads the affect and `pc.remove_skill_book_no_delay`
        // takes it: the remove frame is queued for the character.
        world.affect_events.insert(
            vid,
            super::super::affect::AffectEvent {
                due: 0,
                sequence: 0,
                affects: vec![super::super::affect::Affect {
                    kind: super::super::pc_skill::AFFECT_SKILL_NO_BOOK_DELAY,
                    apply_on: 0,
                    value: 0,
                    flag: 0,
                    remaining: 100,
                    sp_cost: 0,
                }],
            },
        );
        let read = world
            .run_quests(place, mover, 10, pid, |_manager, game| {
                let held = game.has_skill_book_no_delay();
                game.remove_skill_book_no_delay();
                (held, game.has_skill_book_no_delay())
            })
            .expect("the quests are loaded");
        assert_eq!(read.0, (true, false));
        assert!(affect_kinds(&world, vid).is_empty());
        assert!(read
            .1
             .0
            .iter()
            .any(|frame| frame.first() == Some(&HEADER_GC_AFFECT_REMOVE)));
    }

    /// The GRAND book 50525's `use` script end-to-end through the live locale: the no-delay
    /// book affect opens both rest gates and is then removed, the script reads the real
    /// alignment for its threshold, and the failed read moves the world alignment by its
    /// amount. All four names run, the book leaves the inventory, and the alignment update
    /// follows the step.
    #[test]
    fn the_grand_book_runs_the_alignment_names_through_the_live_script() {
        use super::super::quests::QuestStep;
        use protocol::gc_vid::HEADER_GC_AFFECT_REMOVE;

        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let (mut inbox, vid, cell, place, mover) = grand_reader(&mut world);
        while inbox.try_recv().is_ok() {}

        let used = world.use_item(vid, cell, mover).expect("the use runs");
        assert_eq!(used.kind, world::character::MoveKind::QuestUsed);
        assert!(world.quest_running(vid), "the first page asks");

        // "Yes" to the warning.
        let warning = world
            .quest(vid, &QuestStep::Answer(0), place, mover)
            .expect("the answer runs");
        assert!(!warning.0.is_empty());
        assert!(world.quest_running(vid), "the skill list asks");

        // The ward: the script reads the real alignment, then the no-delay affect skips
        // the rest gate and is removed.
        let choice = world
            .quest(vid, &QuestStep::Answer(0), place, mover)
            .expect("the choice runs");
        assert!(
            choice
                .0
                .iter()
                .any(|frame| frame.first() == Some(&HEADER_GC_AFFECT_REMOVE)),
            "the no-delay affect is taken"
        );
        assert!(
            affect_kinds(&world, vid).is_empty(),
            "the world no longer holds it"
        );
        assert!(world.quest_running(vid), "the confirmation asks");

        // "Yes" to the read: no skill row is hosted, so it fails and the script moves the
        // alignment, then takes the book.
        let done = world
            .quest(vid, &QuestStep::Answer(0), place, mover)
            .expect("the read runs");
        assert!(!world.quest_running(vid), "the script ends");
        assert!(done.1.is_empty(), "the read did not climb");
        assert_eq!(
            world.bodies[&vid].card.alignment, -3_530,
            "the script's amount settled after the step"
        );
        assert!(affect_kinds(&world, vid).is_empty(), "no band affects");
        let updates: Vec<i16> = std::iter::from_fn(|| inbox.try_recv().ok())
            .filter_map(|frame| GcCharacterUpdate::decode(&frame).ok())
            .map(|update| update.s_alignment)
            .collect();
        assert_eq!(updates, [-353], "the crossing update follows the step");
        assert!(
            world
                .characters()
                .find_by_vid(vid)
                .expect("the reader is in the world")
                .items()
                .item_at(cell)
                .is_none(),
            "the book left"
        );
    }

    /// A reader holding the GRAND book 50525 in the live locale: an apprenticeship, a
    /// GRAND-level ward, a rest stamp an hour out that only the seeded no-delay book
    /// affect opens, and dice seed 0, whose first draw moves a failed read by 353.
    /// Answers the inbox, the reader, the book's cell, its place and its mover.
    fn grand_reader(
        world: &mut GameState,
    ) -> (
        tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        Vid,
        protocol::item_pos::ItemPos,
        crate::game_state::GroundPlace,
        crate::game_state::Mover,
    ) {
        use super::super::quests::Quests;
        use crate::game_state::fixtures::{enter_seeing_with, PLACE};
        use common::item_slots::EWindows;
        use gamedata::mob_locale_names::MobNamesByLanguage;
        use protocol::item_pos::ItemPos;
        use quest::manager::Manager;
        use std::sync::Arc;
        use world::character::Pcg32;
        use world::item::Item;

        let vid = Vid::new(1);
        let cell = ItemPos::new(EWindows::Inventory as u8, 0);
        let mut book = Item::new(11, 50_525);
        book.set_size(1).expect("a positive footprint");
        let (inbox, _shown) = enter_seeing_with(world, PLACE, 1, (5, 5), 10_000, &[(cell, book)]);

        // The real locale's quests, an apprenticeship, a GRAND-level ward, and a rest stamp
        // in the future that only the no-delay book affect opens.
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager = Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        world.quests = Some(Quests::new(
            manager,
            Arc::default(),
            MobNamesByLanguage::default(),
        ));
        world
            .bodies
            .get_mut(&vid)
            .expect("the body stands")
            .card
            .skill_group = 1;
        world.set_pc_skill_level(vid.raw(), 221, 30);
        let pid = world
            .characters()
            .find_by_vid(vid)
            .expect("the reader is in the world")
            .player_id();
        let now = u32::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("after the epoch")
                .as_secs(),
        )
        .expect("a 32-bit time");
        world
            .quests
            .as_mut()
            .expect("the quests are loaded")
            .manager_mut()
            .load_flags(
                pid,
                [(
                    b"new7and8th_skills.next_time".to_vec(),
                    i32::try_from(now + 3_600).expect("a 32-bit time"),
                )],
            );
        world.affect_events.insert(
            vid,
            super::super::affect::AffectEvent {
                due: 0,
                sequence: 0,
                affects: vec![super::super::affect::Affect {
                    kind: super::super::pc_skill::AFFECT_SKILL_NO_BOOK_DELAY,
                    apply_on: 0,
                    value: 0,
                    flag: 0,
                    remaining: 100,
                    sp_cost: 0,
                }],
            },
        );
        // Seed 0's first 31-bit draw is 1918936004; 168 divides it with remainder 20, so
        // the failed read moves the alignment by 333 + 20 = 353 points.
        world.dice = Pcg32::new(0, 0);

        let mover = world.own_mover(vid).expect("the body stands");
        let place = crate::game_state::GroundPlace {
            channel: PLACE.0,
            map: PLACE.1,
            x: 5,
            y: 5,
        };
        (inbox, vid, cell, place, mover)
    }

    /// The points `vid` has in `slot`.
    fn points_of(world: &GameState, vid: Vid, slot: usize) -> i32 {
        world
            .characters()
            .find_by_vid(vid)
            .expect("the entrant is in the world")
            .points()
            .expect("the entrant has points")
            .get_point(slot)
    }

    /// The position of the `GC_CHARACTER_UPDATE` whose alignment is `alignment`.
    fn update_position(heard: &[Vec<u8>], alignment: i16) -> usize {
        heard
            .iter()
            .position(|frame| {
                frame.first() == Some(&19)
                    && GcCharacterUpdate::decode(frame)
                        .is_ok_and(|update| update.s_alignment == alignment)
            })
            .expect("the crossing's update")
    }
}
