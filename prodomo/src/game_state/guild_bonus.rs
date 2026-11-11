//! The guild bonuses (`__GUILD_BONUSES__`, `CGuild::GuildUpdateAffect` and
//! `CGuild::GuildRemoveAffect`, `G/guild.cpp:1456-1549`).
//!
//! A member whose guild sits in one of the four level windows holds that window's
//! affects, each an infinite `AddAffect` with no flag; the windows overlap at 5, 8
//! and 15, and the first match in table order wins, so level 5 takes the 2-5 row.
//! The update clears every bonus kind (801-807) before it adds; the removal takes
//! the first kind found, in the table's order, and stops.
//!
//! Two legacy shapes are kept. `AFFECT_GUILD_BONUS_8` is 807, the same value as
//! `AFFECT_GUILD_BONUS_7` (`G/affect.h:200-201`), so the 15-20 row lands two rows
//! of kind 807 with different points. And the removal's `break` leaves the other
//! kinds behind: a kicked member of a level-10 guild keeps 804 and 805, and no
//! later login clears them while it stays guildless, as no login runs then.
//!
//! The seams are legacy's call sites: the update runs at login (after the saved
//! affects load), at join through the same login send, and once per level an offer
//! gains; the removal runs at logout, at removal, and at disband for each member
//! online.

use common::point_slot as point;
use common::vid::Vid;

use super::affect::{affect_remove_frame, take_affects_of_kind, Affect};
use crate::game_state::GameState;
use crate::loading_phase::INFINITE_AFFECT_DURATION;

/// One bonus row: the guild-level window and its `(kind, point, value)` affects, in
/// legacy's order (`G/guild.cpp:1471-1475`).
struct GuildBonusRow {
    /// The window's lowest guild level, inclusive.
    min: u8,
    /// The window's highest guild level, inclusive.
    max: u8,
    /// The affects the window lands.
    affects: &'static [(u32, usize, i32)],
}

/// The four rows. The last row's third kind is 807, not 808: the enumerator
/// collides (`G/affect.h:200-201`).
const GUILD_BONUS_ROWS: [GuildBonusRow; 4] = [
    GuildBonusRow {
        min: 2,
        max: 5,
        affects: &[(801, point::POINT_ATTBONUS_DEVIL, 5)],
    },
    GuildBonusRow {
        min: 5,
        max: 8,
        affects: &[(802, point::POINT_ATTBONUS_UNDEAD, 5)],
    },
    GuildBonusRow {
        min: 8,
        max: 15,
        affects: &[
            (803, point::POINT_ATTBONUS_BOSS, 8),
            (804, point::POINT_ATTBONUS_METIN, 8),
            (805, point::POINT_ATTBONUS_MONSTER, 10),
        ],
    },
    GuildBonusRow {
        min: 15,
        max: 20,
        affects: &[
            (806, point::POINT_SKILL_DAMAGE_BONUS, 8),
            (807, point::POINT_NORMAL_HIT_DAMAGE_BONUS, 8),
            (807, point::POINT_ATTBONUS_HUMAN, 10),
        ],
    },
];

/// The kinds the update clears, in the removal's order: the table's kinds flattened
/// (`G/guild.cpp:1533-1540`), with 807 twice, as the enumerators spell it.
const GUILD_BONUS_KINDS: [u32; 8] = [801, 802, 803, 804, 805, 806, 807, 807];

/// The row whose window holds `level`, first match in table order.
fn guild_bonus_row(level: u8) -> Option<&'static [(u32, usize, i32)]> {
    GUILD_BONUS_ROWS
        .iter()
        .find(|row| level >= row.min && level <= row.max)
        .map(|row| row.affects)
}

impl GameState {
    /// `CGuild::GuildUpdateAffect` for the player under `vid`: with no guild row
    /// nothing happens; else every bonus kind's rows come off (their points back,
    /// their remove packets sent), and the guild level's row lands, each an
    /// infinite affect with no flag, appended beside any row of its kind.
    pub(super) fn guild_bonus_update(&mut self, vid: Vid) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let Some(id) = self.guild_of(pid) else {
            return;
        };
        let Some(level) = self.guild_level(id) else {
            return;
        };
        for kind in GUILD_BONUS_KINDS {
            self.take_guild_bonus(vid, kind);
        }
        if let Some(row) = guild_bonus_row(level) {
            for (kind, on, value) in row {
                let affect = Affect::buff(*kind, *on, *value, 0, INFINITE_AFFECT_DURATION, 0);
                self.add_pc_affect(vid, affect, false);
            }
        }
    }

    /// [`GameState::guild_bonus_update`] at `level` instead of the guild's stored one:
    /// the offer commit writes the final level before its fan loop, while legacy's
    /// level-up loop climbs `m_data.level` one level per iteration (`G/guild.cpp:1600`),
    /// so each level's update reads the level that iteration reached.
    pub(super) fn guild_bonus_update_at(&mut self, vid: Vid, level: u8) {
        for kind in GUILD_BONUS_KINDS {
            self.take_guild_bonus(vid, kind);
        }
        if let Some(row) = guild_bonus_row(level) {
            for (kind, on, value) in row {
                let affect = Affect::buff(*kind, *on, *value, 0, INFINITE_AFFECT_DURATION, 0);
                self.add_pc_affect(vid, affect, false);
            }
        }
    }

    /// [`GameState::guild_bonus_update`] with the frames answered instead of sent: the
    /// join's commit appends them to its `own` packets, which the descriptor sends after
    /// the commit, so the bonus lands after the self packets as legacy's `AddMember`
    /// sends them. The points move exactly as the sent variant moves them; every point
    /// record here is self-only (`broadcast: false`, as `compute_affect` passes).
    pub(super) fn guild_bonus_update_collected(&mut self, vid: Vid) -> Vec<Vec<u8>> {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return Vec::new();
        };
        let pid = character.player_id();
        let Some(id) = self.guild_of(pid) else {
            return Vec::new();
        };
        let Some(level) = self.guild_level(id) else {
            return Vec::new();
        };
        let mut frames = Vec::new();
        for kind in GUILD_BONUS_KINDS {
            let removed = match self.affect_events.get_mut(&vid) {
                Some(event) => take_affects_of_kind(&mut event.affects, kind),
                None => Vec::new(),
            };
            for affect in &removed {
                frames.extend(self.uncompute_collected(vid, affect));
                frames.push(affect_remove_frame(affect));
            }
        }
        if let Some(row) = guild_bonus_row(level) {
            for (kind, on, value) in row {
                let affect = Affect::buff(*kind, *on, *value, 0, INFINITE_AFFECT_DURATION, 0);
                self.start_affect_event(vid);
                if let Some(event) = self.affect_events.get_mut(&vid) {
                    event.affects.push(affect);
                }
                frames.extend(self.compute_collected(vid, &affect));
                frames.push(super::affect::affect_add_frame(&affect));
            }
        }
        frames
    }

    /// `compute_affect`'s point move with its frames answered: the affect-list add and
    /// the `point_change` records, self-only.
    fn compute_collected(&mut self, vid: Vid, affect: &Affect) -> Vec<Vec<u8>> {
        if let Some(points) = self.pc_points_mut(vid.raw()) {
            points.affect_point_added(affect.apply_on, affect.value);
        }
        self.point_frames(vid, affect, true)
    }

    /// `compute_affect`'s point take-back with its frames answered.
    fn uncompute_collected(&mut self, vid: Vid, affect: &Affect) -> Vec<Vec<u8>> {
        if let Some(points) = self.pc_points_mut(vid.raw()) {
            points.affect_point_removed(affect.apply_on, affect.value);
        }
        self.point_frames(vid, affect, false)
    }

    /// The `point_change` frames of `affect`'s move onto (`add`) or off the player
    /// under `vid`, as `change_pc_point` sends them for a non-energy, non-mount slot.
    fn point_frames(&mut self, vid: Vid, affect: &Affect, add: bool) -> Vec<Vec<u8>> {
        let amount = if add { affect.value } else { -affect.value };
        let Self { characters, .. } = self;
        let Ok(character) = characters.find_by_vid_mut(vid) else {
            return Vec::new();
        };
        let (_, Some(points)) = character.items_and_points_mut() else {
            return Vec::new();
        };
        let records = points
            .point_change(usize::from(affect.apply_on), amount, false, false)
            .unwrap_or_default();
        crate::loading_phase::point_changes(&records, vid.raw())
    }

    /// `CGuild::GuildRemoveAffect` for the player under `vid`: the first bonus kind
    /// found, in table order, comes off with its points and remove packets, and the
    /// removal stops there — the other kinds stay.
    pub(super) fn guild_bonus_remove(&mut self, vid: Vid) {
        for kind in GUILD_BONUS_KINDS {
            let held = self
                .affect_events
                .get(&vid)
                .is_some_and(|event| event.affects.iter().any(|affect| affect.kind == kind));
            if held {
                self.take_guild_bonus(vid, kind);
                return;
            }
        }
    }

    /// Take every row of bonus `kind` off the player under `vid`: each point back
    /// and each remove packet sent, as `RemoveAffect(DWORD)` takes all of one type
    /// (`G/char_affect.cpp:864-880`).
    fn take_guild_bonus(&mut self, vid: Vid, kind: u32) {
        let removed = match self.affect_events.get_mut(&vid) {
            Some(event) => take_affects_of_kind(&mut event.affects, kind),
            None => Vec::new(),
        };
        for affect in &removed {
            let _stripped = self.compute_affect(vid, affect, false);
            let frame = affect_remove_frame(affect);
            let _retired = self.write_to_client(vid, frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{guild_bonus_row, GUILD_BONUS_KINDS};
    use crate::game_state::fixtures::{a_world, enter, TestClock};
    use common::point_slot as point;
    use common::vid::Vid;

    /// The windows overlap at 5, 8 and 15, and the first match wins; level 1 and a
    /// level past 20 take nothing.
    #[test]
    fn the_first_matching_window_wins() {
        assert_eq!(guild_bonus_row(1), None);
        assert_eq!(
            guild_bonus_row(2).map(<[(u32, usize, i32)]>::to_vec),
            Some(vec![(801, point::POINT_ATTBONUS_DEVIL, 5)])
        );
        assert_eq!(
            guild_bonus_row(4).map(|row| row.first().map(|first| first.0)),
            Some(Some(801))
        );
        assert_eq!(
            guild_bonus_row(5).map(|row| row.first().map(|first| first.0)),
            Some(Some(801))
        );
        assert_eq!(
            guild_bonus_row(6).map(|row| row.first().map(|first| first.0)),
            Some(Some(802))
        );
        assert_eq!(
            guild_bonus_row(8).map(|row| row.first().map(|first| first.0)),
            Some(Some(802))
        );
        assert_eq!(
            guild_bonus_row(15).map(|row| row.iter().map(|landed| landed.0).collect::<Vec<_>>()),
            Some(vec![803, 804, 805])
        );
        assert_eq!(
            guild_bonus_row(20).map(|row| row.iter().map(|landed| landed.0).collect::<Vec<_>>()),
            Some(vec![806, 807, 807])
        );
        assert_eq!(guild_bonus_row(21), None);
    }

    /// The removal order holds 807 twice, as the colliding enumerators spell it.
    #[test]
    fn the_removal_order_keeps_the_colliding_kind_twice() {
        assert_eq!(GUILD_BONUS_KINDS, [801, 802, 803, 804, 805, 806, 807, 807]);
    }

    /// The update lands the level's row and clears the rest; the removal takes the
    /// first kind found and leaves the others.
    #[test]
    fn an_update_lands_the_rows_and_a_removal_takes_the_first_kind() {
        let clock = TestClock::default();
        let mut state = a_world(&clock);
        let _inbox = enter(&mut state, 7, (5, 5), 10_000);
        let vid = Vid::new(7);
        state
            .guilds
            .install_named_test_guild(3, 7, 10, 0, b"Bonus".to_vec());
        state.guild_bonus_update(vid);
        let kinds = state
            .affect_events
            .get(&vid)
            .map(|event| {
                event
                    .affects
                    .iter()
                    .map(|affect| affect.kind)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        assert_eq!(kinds, [803, 804, 805]);
        state.guild_bonus_remove(vid);
        let kinds = state
            .affect_events
            .get(&vid)
            .map(|event| {
                event
                    .affects
                    .iter()
                    .map(|affect| affect.kind)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        assert_eq!(kinds, [804, 805]);
    }
}
