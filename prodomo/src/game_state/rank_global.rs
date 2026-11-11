//! The global rank (`ENABLE_GLOBAL_RANK`, `G/RankGlobal.cpp`): nine counters per character,
//! the board the client is shown, the season's end and its prizes.
//!
//! Legacy split it in two. The DB process held the counters (`rank_vec`), added each
//! `HEADER_DG_RANKGLOBAL_ADD_POINT`, and every `RANKGLOBAL_DB_SAVE_TIME` (20 minutes) sent them
//! to every core (`D/ClientManager.cpp:5341-5380`); each core sorted what it was sent and shows
//! that snapshot until the next one (`RankGlobal_sort`, `G/RankGlobal.cpp:147-175`). So a kill
//! counts at once and shows at the next send. Here one board holds both: [`RankBoard::rows`] are
//! the DB process's counters and [`RankBoard::shown`] the cores' snapshot, taken every 20
//! minutes.
//!
//! Legacy's first snapshot came 20 seconds after a core booted, when it asked the DB process
//! for one (`G/main.cpp:210-236`). Here the board is installed before any client is accepted
//! and is sorted then: a recorded Divergence (ledger 402), since only the retired DB peer made
//! the first 20 seconds empty.
//!
//! - [`GameState::rank_login`] is `RankGlobal_send_players` and `RankGlobal_send_items` at the
//!   end of `Entergame` (`G/input_login.cpp:946-949`).
//! - [`GameState::run_rank_line`] is `/open_global_rank` and `/rank_end_season`
//!   (`G/cmd_general.cpp:4118-4134`).
//! - [`GameState::add_rank_point`] is `RankGlobal_add_point` with the DB process's half
//!   (`G/RankGlobal.cpp:106-128`, `D/ClientManager.cpp:5473-5509`).
//! - A character's row is saved with it ([`GameState::rank_row`]); a season's end is stored by
//!   the GM's descriptor in one transaction ([`SeasonEnd`]); a claim of prizes is stored with
//!   the items the claim gave (`quests.rs`, [`super::quests::Giving::ItemsAndPrizes`]).
//!
//! Ties in a counter keep the board's order, which is the order the rows were loaded in
//! (`pid`) and then the order characters first scored. Legacy's `std::sort` leaves the order of
//! ties unspecified.

use std::collections::HashMap;

use common::vid::Vid;
use db::rank_global::{RankBoot, StoredRank, StoredReward, StoredWinner, COUNTERS};
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};
use tracing::warn;

use crate::chat_line::Arg;
use crate::game_loop_messages::RankLine;
use crate::game_state::GameState;
use crate::item_move::Mover;
use crate::save::PASSES_PER_SEC;

/// `RANKGLOBAL_DB_SAVE_TIME` (`C/prodomodefines.h:172`), 20 minutes, in Pulses.
const REFRESH_PULSES: u64 = 60 * 20 * PASSES_PER_SEC as u64;

/// The event flag that numbers the season (`G/RankGlobal.cpp:47`, `:97`).
const SEASON_FLAG: &[u8] = b"RankGlobalSeason";

/// The places a counter lists, and the places a season's end awards.
const PLACES: usize = 10;

/// One counter of one character, as the snapshot holds it (`stRankGlobal_player_sort`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Placed {
    kind: u8,
    pid: u32,
    name: Vec<u8>,
    count: i32,
    level: i32,
    empire: u8,
}

/// The board: the counters, the snapshot the client is shown, the prizes and the unclaimed
/// awards.
#[derive(Debug, Default)]
pub(super) struct RankBoard {
    /// The generation a character's row is saved under; a season's end raises it.
    generation: i64,
    /// The counters, in board order.
    rows: Vec<StoredRank>,
    /// The prizes, as the Operator listed them.
    rewards: Vec<StoredReward>,
    /// The snapshot: each counter's non-zero entries, best first, counter by counter.
    shown: Vec<Placed>,
    /// The unclaimed prizes, (vnum, count) in award order, by `pid`.
    unclaimed: HashMap<u32, Vec<(u32, u16)>>,
    /// The Pulse the next snapshot is taken at.
    next_refresh: u64,
}

/// What a quest step does to the global rank, which the world applies after it.
#[derive(Debug, Default)]
pub(super) struct RankStep {
    /// Every `pc.global_rank_add_point` (counter, count), in order, for
    /// [`GameState::add_rank_point`].
    pub(super) points: Vec<(u8, i32)>,
    /// The reader's prizes, as `pc.global_rank_get_reward` finds them.
    pub(super) prizes: Prizes,
}

/// The reader's prizes during a quest step.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) enum Prizes {
    /// The step's descriptor stores no claim, so nothing is given.
    #[default]
    Closed,
    /// The prizes the reader won and has not claimed, (vnum, count) in award order.
    Open(Vec<(u32, u16)>),
    /// The step claimed them; its descriptor stores the claim with the items.
    Claimed,
}

/// A season's end for the GM's descriptor to store (`db::rank_global::end_season`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeasonEnd {
    /// The season that begins.
    pub season: i32,
    /// The generation that begins.
    pub generation: i64,
    /// Every prize awarded.
    pub winners: Vec<StoredWinner>,
}

impl RankBoard {
    /// The prizes `pid` won and has not claimed.
    pub(super) fn prizes_of(&self, pid: u32) -> Vec<(u32, u16)> {
        self.unclaimed.get(&pid).cloned().unwrap_or_default()
    }

    /// `pid` claimed its prizes.
    pub(super) fn forget_prizes(&mut self, pid: u32) {
        self.unclaimed.remove(&pid);
    }
}

/// `RankGlobal_sort` (`G/RankGlobal.cpp:147-175`): counter by counter, every row whose count
/// in it is not zero, best first.
fn sorted(rows: &[StoredRank]) -> Vec<Placed> {
    let mut placed = Vec::new();
    for kind in 0..COUNTERS {
        let start = placed.len();
        placed.extend(
            rows.iter()
                .filter(|row| row.pid != 0 && row.counts[kind] != 0)
                .map(|row| Placed {
                    kind: u8::try_from(kind).unwrap_or(u8::MAX),
                    pid: row.pid,
                    name: row.name.clone(),
                    count: row.counts[kind],
                    level: row.level,
                    empire: row.empire,
                }),
        );
        placed[start..].sort_by(|a, b| b.count.cmp(&a.count));
    }
    placed
}

/// Each counter's entries with their places, from 1: (place, entry).
fn places(placed: &[Placed]) -> impl Iterator<Item = (usize, &Placed)> {
    let mut place = 0;
    let mut kind = None;
    placed.iter().map(move |entry| {
        if kind == Some(entry.kind) {
            place += 1;
        } else {
            kind = Some(entry.kind);
            place = 1;
        }
        (place, entry)
    })
}

impl GameState {
    /// Install the board boot read: the season flag, the counters sorted for the first
    /// snapshot, and the next one 20 minutes on.
    pub(super) fn install_rank(&mut self, boot: RankBoot) {
        let mut unclaimed: HashMap<u32, Vec<(u32, u16)>> = HashMap::new();
        for (pid, vnum, count) in boot.unclaimed {
            unclaimed.entry(pid).or_default().push((vnum, count));
        }
        self.rank = RankBoard {
            generation: boot.generation,
            shown: sorted(&boot.rows),
            rows: boot.rows,
            rewards: boot.rewards,
            unclaimed,
            next_refresh: self.last_pulse.saturating_add(REFRESH_PULSES),
        };
        // Legacy writes the flag only at a season's end, so a boot at season 0 leaves it
        // unset and `/geteventflag` does not list it.
        if boot.season != 0 {
            self.set_event_flag(SEASON_FLAG, boot.season);
        }
    }

    /// The DB process's 20-minute send (`D/ClientManager.cpp:5374-5379`): a new snapshot.
    pub(super) fn run_rank_refresh(&mut self, pulse: u64) {
        if pulse < self.rank.next_refresh {
            return;
        }
        self.rank.shown = sorted(&self.rank.rows);
        self.rank.next_refresh = pulse.saturating_add(REFRESH_PULSES);
    }

    /// The character's row and the generation to save it under, when it ever scored.
    pub(super) fn rank_row(&self, pid: u32) -> Option<(i64, StoredRank)> {
        let row = self.rank.rows.iter().find(|row| row.pid == pid)?;
        Some((self.rank.generation, row.clone()))
    }

    /// The lines `Entergame` ends with (`G/input_login.cpp:946-949`): the board, then the
    /// prizes.
    pub(super) fn rank_login(&self, vid: Vid) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        self.send_rank_board(vid, mover);
        self.send_rank_prizes(vid, mover);
    }

    /// `RankGlobal_send_players` (`G/RankGlobal.cpp:38-73`): the clears, the season, each
    /// counter's first ten places, the character's own place in each counter it is in, and
    /// the refresh.
    fn send_rank_board(&self, vid: Vid, mover: Mover) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let season = self
            .quests
            .as_ref()
            .map_or(0, |quests| quests.manager().event_flag(SEASON_FLAG));
        self.tell(
            vid,
            mover,
            CHAT_TYPE_COMMAND,
            b"GlobalRankData clear_rank",
            &[],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_COMMAND,
            b"GlobalRankMyChar clear",
            &[],
        );
        let season = [Arg::Int(season.into())];
        self.tell(
            vid,
            mover,
            CHAT_TYPE_COMMAND,
            b"GlobalRankData ref_season|%d",
            &season,
        );
        for (place, entry) in places(&self.rank.shown) {
            let (kind, name) = (Arg::Int(entry.kind.into()), Arg::Text(&entry.name));
            let (count, level) = (Arg::Int(entry.count.into()), Arg::Int(entry.level.into()));
            let empire = Arg::Int(entry.empire.into());
            if place <= PLACES {
                let args = [kind, name, count, level, empire];
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_COMMAND,
                    b"GlobalRankData %d|%s|%d|%d|%d",
                    &args,
                );
            }
            if entry.pid == pid {
                let place = Arg::Int(i64::try_from(place).unwrap_or(i64::MAX));
                let args = [kind, name, place, count, level, empire];
                let format = b"GlobalRankMyChar %d|%s|%d|%d|%d|%d";
                self.tell(vid, mover, CHAT_TYPE_COMMAND, format, &args);
            }
        }
        self.tell(vid, mover, CHAT_TYPE_COMMAND, b"GlobalRankRefresh", &[]);
    }

    /// `RankGlobal_send_items` (`G/RankGlobal.cpp:75-88`): nothing without prizes, else the
    /// clear and one line per prize.
    fn send_rank_prizes(&self, vid: Vid, mover: Mover) {
        if self.rank.rewards.is_empty() {
            return;
        }
        let clear = b"GlobalRankDataItem clear_items";
        self.tell(vid, mover, CHAT_TYPE_COMMAND, clear, &[]);
        for reward in &self.rank.rewards {
            let args = [
                Arg::Int(reward.kind.into()),
                Arg::Int(reward.pos.into()),
                Arg::Int(reward.vnum.into()),
                Arg::Int(reward.count.into()),
            ];
            let format = b"GlobalRankDataItem %d|%d|%d|%d";
            self.tell(vid, mover, CHAT_TYPE_COMMAND, format, &args);
        }
    }

    /// `RankGlobal_add_point` (`G/RankGlobal.cpp:106-128`) and the DB process's half
    /// (`D/ClientManager.cpp:5473-5509`): a counter past the ninth is refused; a character
    /// with a row adds to it and takes its level and empire anew, keeping the Name it first
    /// scored with; one without gets a row.
    pub(super) fn add_rank_point(&mut self, vid: Vid, kind: u8, count: i32) {
        let slot = usize::from(kind);
        if slot >= COUNTERS {
            warn!(kind, "error; RankGlobal_add_point bad type.");
            return;
        }
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let level = character
            .points()
            .map_or(1, |points| i32::from(points.level()));
        let empire = self.bodies.get(&vid).map_or(0, |body| body.card.empire);
        if pid == 0 {
            return;
        }
        if let Some(row) = self.rank.rows.iter_mut().find(|row| row.pid == pid) {
            row.counts[slot] = row.counts[slot].wrapping_add(count);
            row.level = level;
            row.empire = empire;
            return;
        }
        let mut counts = [0; COUNTERS];
        counts[slot] = count;
        let name = character.name().as_bytes().to_vec();
        self.rank.rows.push(StoredRank {
            pid,
            name,
            level,
            empire,
            counts,
        });
    }

    /// `/open_global_rank` and `/rank_end_season` (`G/cmd_general.cpp:4118-4134`). Answers the
    /// season's end the GM's descriptor stores, or `None`.
    pub(super) fn run_rank_line(
        &mut self,
        vid: Vid,
        line: RankLine,
        mover: Mover,
    ) -> Option<SeasonEnd> {
        match line {
            RankLine::Open => {
                self.send_rank_board(vid, mover);
                None
            }
            RankLine::EndSeason => {
                let ended = self.end_rank_season();
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"RankGlobal - Season is over.",
                    &[],
                );
                Some(ended)
            }
        }
    }

    /// `RankGlobal_end_season`, both halves (`G/RankGlobal.cpp:90-104`,
    /// `D/ClientManager.cpp:5382-5470`): the season flag rises; each counter's first ten places
    /// win every prize listed for that counter and place; the counters empty; the snapshot
    /// empties until the next one, which is 20 minutes on.
    fn end_rank_season(&mut self) -> SeasonEnd {
        let season = self
            .quests
            .as_ref()
            .map_or(0, |quests| quests.manager().event_flag(SEASON_FLAG))
            .wrapping_add(1);
        self.set_event_flag(SEASON_FLAG, season);
        let mut winners = Vec::new();
        for (place, entry) in places(&sorted(&self.rank.rows)) {
            if place > PLACES {
                continue;
            }
            let position = i32::try_from(place).unwrap_or(i32::MAX);
            for reward in &self.rank.rewards {
                if reward.kind == entry.kind && reward.pos == position {
                    winners.push(StoredWinner {
                        pid: entry.pid,
                        name: entry.name.clone(),
                        position,
                        kind: entry.kind,
                        vnum: reward.vnum,
                        count: reward.count,
                    });
                }
            }
        }
        for winner in &winners {
            self.rank
                .unclaimed
                .entry(winner.pid)
                .or_default()
                .push((winner.vnum, winner.count));
        }
        self.rank.rows.clear();
        self.rank.shown.clear();
        self.rank.generation += 1;
        self.rank.next_refresh = self.last_pulse.saturating_add(REFRESH_PULSES);
        SeasonEnd {
            season,
            generation: self.rank.generation,
            winners,
        }
    }
}

#[cfg(test)]
mod tests {
    use common::vid::Vid;
    use gamedata::mob_locale_names::MobNamesByLanguage;

    use super::{places, sorted, RankBoot, RankLine};
    use crate::game_state::fixtures::{a_world, enter, TestClock};
    use crate::game_state::GameState;

    const VID: u32 = 0x6001;

    /// A world with the quest runtime, which holds the season flag.
    fn world(clock: &TestClock) -> GameState {
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager =
            quest::manager::Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        let quests = crate::game_state::quests::Quests::new(
            manager,
            std::sync::Arc::default(),
            MobNamesByLanguage::default(),
        );
        a_world(clock).with_quests(quests)
    }

    fn row(pid: u32, name: &[u8], counts: [i32; 9]) -> db::rank_global::StoredRank {
        db::rank_global::StoredRank {
            pid,
            name: name.to_vec(),
            level: 154,
            empire: 1,
            counts,
        }
    }

    fn boot() -> RankBoot {
        let low = row(1, b"Low", [0, 2, 0, 0, 0, 0, 0, 0, 0]);
        let high = row(2, b"High", [5, 0, 0, 0, 0, 0, 0, 0, 0]);
        RankBoot {
            season: 3,
            generation: 1,
            rows: [low, high].to_vec(),
            rewards: [db::rank_global::StoredReward {
                kind: 0,
                pos: 1,
                vnum: 19,
                count: 3,
            }]
            .to_vec(),
            unclaimed: [(9, 19, 3)].to_vec(),
        }
    }

    fn mover() -> crate::item_move::Mover {
        crate::item_move::Mover {
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
        assert_eq!(line[0], 4, "HEADER_GC_CHAT: {line:02x?}");
        line[10..].to_vec()
    }

    fn heard(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// `sorted` lists counter by counter, zeros skipped, best first; `places` numbers each
    /// counter from 1.
    #[test]
    fn the_sort_lists_each_counters_nonzero_best_first() {
        let shown = sorted(&boot().rows);
        assert_eq!(shown.len(), 2);
        assert_eq!((shown[0].kind, shown[0].count), (0, 5));
        assert_eq!((shown[1].kind, shown[1].count), (1, 2));
        assert_eq!(
            places(&shown).map(|(place, _)| place).collect::<Vec<_>>(),
            [1, 1]
        );
        let tied = [
            row(1, b"A", [5, 0, 0, 0, 0, 0, 0, 0, 0]),
            row(2, b"B", [5, 0, 0, 0, 0, 0, 0, 0, 0]),
        ];
        let shown = sorted(&tied);
        assert_eq!(
            shown.iter().map(|entry| entry.pid).collect::<Vec<_>>(),
            [1, 2]
        );
        let pair = [
            row(1, b"A", [2, 0, 0, 0, 0, 0, 0, 0, 0]),
            row(2, b"B", [9, 0, 0, 0, 0, 0, 0, 0, 0]),
        ];
        let shown = sorted(&pair);
        assert_eq!(
            shown
                .iter()
                .map(|entry| (entry.pid, entry.count))
                .collect::<Vec<_>>(),
            [(2, 9), (1, 2)]
        );
    }

    /// The login sends the clears, the season, each counter's first ten, the character's
    /// own place, and the refresh; then the prizes.
    #[test]
    fn the_login_sends_the_board_and_the_prizes() {
        let clock = TestClock::default();
        let mut world = world(&clock);
        let mut inbox = enter(&mut world, VID, (10, 10), 100);
        let _shown = heard(&mut inbox);
        world.install_rank(boot());
        world.rank_login(Vid::new(VID));
        let texts: Vec<Vec<u8>> = heard(&mut inbox).iter().map(|line| text_of(line)).collect();
        assert_eq!(
            texts,
            [
                b"GlobalRankData clear_rank".to_vec(),
                b"GlobalRankMyChar clear".to_vec(),
                b"GlobalRankData ref_season|3".to_vec(),
                b"GlobalRankData 0|High|5|154|1".to_vec(),
                b"GlobalRankData 1|Low|2|154|1".to_vec(),
                b"GlobalRankRefresh".to_vec(),
                b"GlobalRankDataItem clear_items".to_vec(),
                b"GlobalRankDataItem 0|1|19|3".to_vec(),
            ]
        );
    }
    /// A counter with eleven scorers lists the first ten; the eleventh still reads its own
    /// place.
    #[test]
    fn the_board_lists_ten_places_and_the_eleventh_reads_its_own() {
        let clock = TestClock::default();
        let mut world = world(&clock);
        let mut inbox = enter(&mut world, VID, (10, 10), 100);
        let _shown = heard(&mut inbox);
        let rows: Vec<db::rank_global::StoredRank> = (1..=11i32)
            .map(|count| {
                row(
                    u32::try_from(count).expect("a small pid"),
                    format!("P{count}").as_bytes(),
                    [count, 0, 0, 0, 0, 0, 0, 0, 0],
                )
            })
            .collect();
        world.install_rank(RankBoot { rows, ..boot() });
        world.rank_login(Vid::new(VID));
        let texts: Vec<Vec<u8>> = heard(&mut inbox).iter().map(|line| text_of(line)).collect();
        let entries: Vec<&Vec<u8>> = texts
            .iter()
            .filter(|text| text.starts_with(b"GlobalRankData 0|"))
            .collect();
        assert_eq!(entries.len(), 10);
        assert_eq!(entries[0], b"GlobalRankData 0|P11|11|154|1");
        assert_eq!(entries[9], b"GlobalRankData 0|P2|2|154|1");
    }

    /// A counter past the ninth is refused; a kill counts at once, shows at the next
    /// refresh; `/open_global_rank` resends the board.
    #[test]
    fn a_kill_counts_at_once_and_shows_at_the_refresh() {
        let clock = TestClock::default();
        let mut world = world(&clock);
        let mut inbox = enter(&mut world, VID, (10, 10), 100);
        let _shown = heard(&mut inbox);
        world.install_rank(boot());
        world.add_rank_point(Vid::new(VID), 9, 7);
        assert!(
            world.rank_row(VID).is_none(),
            "the ninth counter is refused"
        );
        world.add_rank_point(Vid::new(VID), 2, 7);
        let (_, saved) = world.rank_row(VID).expect("the kill counted at once");
        assert_eq!(saved.counts[2], 7);
        world.run_rank_line(Vid::new(VID), RankLine::Open, mover());
        assert!(
            !heard(&mut inbox)
                .iter()
                .any(|line| text_of(line).starts_with(b"GlobalRankData 2")),
            "shows the old snapshot"
        );
        world.run_rank_refresh(u64::MAX);
        world.run_rank_line(Vid::new(VID), RankLine::Open, mover());
        let texts: Vec<Vec<u8>> = heard(&mut inbox).iter().map(|line| text_of(line)).collect();
        assert!(texts
            .iter()
            .any(|text| text.starts_with(b"GlobalRankMyChar 2|P24577|1|7")));
    }

    /// `/rank_end_season` raises the season, awards each counter's first ten the prizes
    /// listed for their place, empties the counters, and bumps the generation; the login
    /// then sends no entries.
    #[test]
    fn the_seasons_end_awards_and_empties() {
        let clock = TestClock::default();
        let mut world = world(&clock);
        let mut inbox = enter(&mut world, VID, (10, 10), 100);
        let _shown = heard(&mut inbox);
        world.install_rank(boot());
        let ended = world
            .run_rank_line(Vid::new(VID), RankLine::EndSeason, mover())
            .expect("an end stores");
        assert_eq!((ended.season, ended.generation), (4, 2));
        assert_eq!(ended.winners.len(), 1);
        assert_eq!(
            (
                ended.winners[0].pid,
                ended.winners[0].vnum,
                ended.winners[0].count
            ),
            (2, 19, 3)
        );
        assert_eq!(world.rank.prizes_of(2), [(19, 3)]);
        assert!(world.rank_row(2).is_none());
        let texts: Vec<Vec<u8>> = heard(&mut inbox).iter().map(|line| text_of(line)).collect();
        assert_eq!(
            texts.last().map(Vec::as_slice),
            Some(b"RankGlobal - Season is over.".as_slice())
        );
        world.rank_login(Vid::new(VID));
        let texts: Vec<Vec<u8>> = heard(&mut inbox).iter().map(|line| text_of(line)).collect();
        assert_eq!(texts[2], b"GlobalRankData ref_season|4");
        assert!(!texts
            .iter()
            .any(|text| text.starts_with(b"GlobalRankData 0")));
    }
}
